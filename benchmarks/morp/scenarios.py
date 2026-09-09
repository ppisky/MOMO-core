"""Paired native experiments with explicit probe-dependency semantics."""
from pathlib import Path

from .common import digest, read_json, require, write_new
from .runner import execute, plan, MOMO_SCENARIO_FLAGS
from .metrics import compare, interval

PAIRED_SCENARIOS = ("basic_context", "all_enabled")
# Compatibility alias for callers that explicitly request the historical
# two-arm matrix. The CLI default is CORE_SCENARIOS.
SCENARIOS = PAIRED_SCENARIOS
CORE_SCENARIOS = ("basic_context", "dmw_nsg", "all_enabled")
CAUSAL_SCENARIOS = tuple(MOMO_SCENARIO_FLAGS)


def create(cases, manifest, config, repeats, directory, selection=None, matrix="core"):
    require(config.get("backend") == "momo", "scenarios require a MOMO config")
    root = Path(directory)
    require(not root.exists(), "scenario output already exists")
    names = {"paired": PAIRED_SCENARIOS, "core": CORE_SCENARIOS,
             "causal": CAUSAL_SCENARIOS}.get(matrix)
    require(names is not None, "unknown scenario matrix")
    plans = {}
    for scenario in names:
        memory, semantic_graph, mo_state = MOMO_SCENARIO_FLAGS[scenario]
        configured = {**config, "scenario": scenario, "memory": memory,
                      "semantic_graph": semantic_graph, "mo_state": mo_state}
        plans[scenario] = plan(cases, manifest, configured, repeats, selection)
    dependencies = {name: sum(c.get("momo_preset", {}).get("dependency") == name for c in cases)
                    for name in ("context", "extracted")}
    experiment = {"schema": "morp.scenarios/2", "plans": {name: p["plan_sha256"] for name, p in plans.items()},
                  "arms": list(names),
                  "candidate_calls": sum(p["candidate_calls"] for p in plans.values()),
                  "matrix": matrix,
                  "maintenance_calls": "additional; every arm that writes DMW or NSG drains before probe",
                  "order": "rotate first arm by case/repeat index", "network_executed": False,
                  "dependency_cases": dependencies,
                  "definition": "Every arm ingests identical events under the selected history_mode. Context presets continue the active conversation; extracted presets open a fresh conversation. Only memory, semantic_graph and mo_state change; model, budgets and oracle exclusion remain paired."}
    for name, run_plan in plans.items():
        write_new(root / f"{name}.plan.json", run_plan)
    write_new(root / "experiment.json", experiment)
    return experiment


def run(directory, manifest, cases, output, allow_ai=False):
    require(allow_ai, "AI disabled; execution requires --allow-ai")
    root = Path(directory)
    experiment = read_json(root / "experiment.json")
    names = tuple(experiment["arms"])
    require(set(names) == set(experiment["plans"]) and len(names) == len(set(names)),
            "scenario arm order is incomplete or duplicated")
    plans = {name: read_json(root / f"{name}.plan.json") for name in names}
    for name, p in plans.items():
        require(experiment["plans"][name] == p["plan_sha256"], "experiment/plan mismatch")
        require(p["dataset_sha256"] == manifest["cases_sha256"], "dataset mismatch")
    require(all(plans[names[0]]["requests"] == plans[name]["requests"] for name in names[1:]), "unpaired plans")
    for index, request in enumerate(plans[names[0]]["requests"]):
        offset = index % len(names)
        order = names[offset:] + names[:offset]
        for name in order:
            execute(plans[name], cases, Path(output) / name, allow_ai=True,
                    request_keys={(request["case_id"], request["repeat"])})
    return {name: str(Path(output) / name / "predictions.jsonl") for name in names}


def comparison(left, right):
    a, b = left["experiment"], right["experiment"]
    require(a["config"].get("scenario") == SCENARIOS[0] and b["config"].get("scenario") == SCENARIOS[1],
            "expected basic_context then all_enabled")
    excluded = {"scenario", "memory", "semantic_graph", "mo_state"}
    require({k: v for k, v in a["config"].items() if k not in excluded} ==
            {k: v for k, v in b["config"].items() if k not in excluded}, "non-feature settings differ")
    quality = compare(left, right)
    left_cases = {row["case_id"]: row for row in left["cases"]}
    right_cases = {row["case_id"]: row for row in right["cases"]}
    dependency_quality = {}
    for dependency in sorted({row.get("dependency") for row in left["cases"] if row.get("dependency")}):
        selected = [identity for identity, row in left_cases.items() if row.get("dependency") == dependency]
        clusters = {}
        complete = bool(selected)
        for identity in selected:
            a_row, b_row = left_cases[identity], right_cases[identity]
            if a_row["score"] is None or b_row["score"] is None:
                complete = False
                continue
            clusters.setdefault(a_row["family"], []).append(b_row["score"] - a_row["score"])
        deltas = [sum(values) / len(values) for values in clusters.values()]
        dependency_quality[dependency] = {
            "paired_cases": sum(len(values) for values in clusters.values()),
            "family_count": len(clusters),
            "complete": complete,
            "family_mean_delta": sum(deltas) / len(deltas) if complete and deltas else None,
            "ci95_family_bootstrap": interval(deltas) if complete else None,
        }
    performance = {}
    pa = {(r["case_id"], r["repeat"]): r for r in left["performance"]["cases"]}
    pb = {(r["case_id"], r["repeat"]): r for r in right["performance"]["cases"]}
    require(pa.keys() == pb.keys(), "performance selections differ")
    families = {r["case_id"]: r["family"] for r in left["cases"]}
    for metric in ("probe_seconds", "observed_response_cost"):
        clusters = {}
        for key in pa:
            x, y = pa[key][metric], pb[key][metric]
            if metric == "observed_response_cost" and not (
                    pa[key]["response_usage_complete"] and pb[key]["response_usage_complete"]):
                continue
            if x is not None and y is not None and pa[key]["status"] == pb[key]["status"] == "ok":
                family = families[f"{key[0]}@repeat-{key[1]}"]
                clusters.setdefault(family, []).append(y - x)
        values = [sum(v) / len(v) for v in clusters.values()]
        paired = sum(len(v) for v in clusters.values())
        complete = paired == len(left["cases"])
        performance[metric] = {"paired_cases": paired, "complete": complete,
                               "family_mean_delta": sum(values) / len(values) if values and complete else None,
                               "ci95_family_bootstrap": interval(values) if complete else None}
    return {"schema": "morp.scenario-comparison/2", "quality": quality,
            "quality_by_dependency": dependency_quality, "performance": performance,
            "direction": "all_enabled-minus-basic_context", "report_sha256": [digest(left), digest(right)],
            "cost_note": "Observed response costs exclude native background/model gateway work; no total-cost advantage may be inferred.",
            "publication_status": "provisional_synthetic_not_human_calibrated"}


def causal_comparison(reports):
    """Report identifiable component contrasts without inventing one total score."""
    require(set(reports) == set(CAUSAL_SCENARIOS), "causal comparison requires all seven arms")
    excluded = {"scenario", "memory", "semantic_graph", "mo_state"}
    reference = None
    for name in CAUSAL_SCENARIOS:
        report = reports[name]
        require(report["experiment"]["config"].get("scenario") == name,
                f"report/config arm mismatch: {name}")
        invariant = {key: value for key, value in report["experiment"]["config"].items()
                     if key not in excluded}
        if reference is None:
            reference = invariant
        require(invariant == reference, "causal arms changed model, revision, budget or protocol")

    contrasts = {
        "dmw_vs_context": ("basic_context", "dmw_only"),
        "nsg_vs_context": ("basic_context", "nsg_only"),
        "dmw_nsg_vs_context": ("basic_context", "dmw_nsg"),
        "state_given_dmw": ("dmw_only", "dmw_state"),
        "state_given_nsg": ("nsg_only", "nsg_state"),
        "state_given_dmw_nsg": ("dmw_nsg", "all_enabled"),
        "all_enabled_vs_context": ("basic_context", "all_enabled"),
    }
    effects = {}
    for label, (left, right) in contrasts.items():
        effects[label] = {
            "left": left,
            "right": right,
            "quality": compare(reports[left], reports[right]),
        }
    return {
        "schema": "morp.causal-comparison/1",
        "effects": effects,
        "arm_performance": {name: reports[name].get("performance", {}) for name in CAUSAL_SCENARIOS},
        "interpretation": "Each quality effect is right-minus-left with the same model and paired cases. Read latency and cost per arm; missing native background usage keeps total cost unknown.",
        "publication_status": "provisional_synthetic_not_human_calibrated",
    }
