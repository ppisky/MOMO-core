"""Run from the repository root: python -m benchmarks.morp --help."""
import argparse
import platform
from pathlib import Path
import sys

from .common import digest, load_dataset, read_json, read_jsonl, require, unique, write_new
from .corpus import DIMENSIONS, VERSION, make_cases
from .metrics import POLICY, ablation_report, compare, emotion_metrics, interval, score, stance_metrics
from .offline import run_probe
from .runner import execute, execute_judge, judge_plan, plan
from .upstream import SOURCES, import_locked, pin
from .acgn import CAST, make_acgn_cases
from .calibration import calibration_plan, calibration_score
from .momo_presets import make_momo_preset_cases
from .stress import make_stress_cases
from .roleplay_v02 import make_roleplay_v02_cases
from . import scenarios
from .performance import report as performance_report


def save_dataset(output, cases, provenance, version=VERSION):
    root = Path(output)
    require(not root.exists(), "dataset output already exists; choose a new version directory")
    unique(cases, "id")
    write_new(root / "cases.jsonl", cases, lines=True)
    manifest = {"schema": "morp.dataset/1", "version": version, "count": len(cases), "cases_sha256": digest(cases),
                "policy_sha256": digest(POLICY), "provenance": provenance, "human_calibrated": False}
    write_new(root / "manifest.json", manifest)
    load_dataset(root)
    return manifest


def select_cases(cases, split="eval", dimensions=(), families=(), characters=(), arms=(), case_ids=(),
                 dependencies=()):
    """Apply auditable, conjunctive plan filters without changing the dataset."""
    dimensions, families, characters = set(dimensions), set(families), set(characters)
    arms, case_ids, dependencies = set(arms), set(case_ids), set(dependencies)
    character_aliases = {identity: identity for identity, _, _, _ in CAST}
    character_aliases.update({name: identity for identity, name, _, _ in CAST})
    unknown_characters = characters - character_aliases.keys()
    require(not unknown_characters, f"unknown characters: {sorted(unknown_characters)}")
    character_ids = {character_aliases[value] for value in characters}

    available_families = {case["family"] for case in cases}
    available_ids = {case["id"] for case in cases}
    require(families <= available_families, f"unknown families: {sorted(families - available_families)}")
    require(case_ids <= available_ids, f"unknown case IDs: {sorted(case_ids - available_ids)}")

    def character_id(case):
        parts = case["id"].split("/")
        return parts[2] if len(parts) > 2 and parts[0] == "acgn" else None

    selected = [case for case in cases
                if (split == "all" or case["split"] == split)
                and (not dimensions or case["dimension"] in dimensions)
                and (not families or case["family"] in families)
                and (not character_ids or character_id(case) in character_ids)
                and (not arms or case.get("ablation", {}).get("arm") in arms)
                and (not case_ids or case["id"] in case_ids)
                and (not dependencies or case.get("momo_preset", {}).get("dependency") in dependencies)]
    require(bool(selected), "selection matched no cases; check the split and filters")
    selection = {"split": split, "dimensions": sorted(dimensions), "families": sorted(families),
                 "characters": sorted(character_ids), "arms": sorted(arms), "case_ids": sorted(case_ids),
                 "dependencies": sorted(dependencies),
                 "case_count": len(selected)}
    return selected, selection


def score_run(dataset, run_plan, rows, votes):
    manifest, all_cases = load_dataset(dataset)
    require(run_plan["dataset_sha256"] == manifest["cases_sha256"], "plan/dataset mismatch")
    require(digest({k: v for k, v in run_plan.items() if k != "plan_sha256"}) == run_plan["plan_sha256"], "plan hash mismatch")
    require(manifest["policy_sha256"] == digest(POLICY), "dataset/scorer version mismatch")
    expected = {(r["case_id"], r["repeat"]) for r in run_plan["requests"]}
    require(len(expected) == len(run_plan["requests"]), "duplicate planned request")
    received = {(r["case_id"], r["repeat"]) for r in rows}
    require(len(received) == len(rows) and received <= expected, "duplicate/unplanned predictions")
    require(all(r["plan_sha256"] == run_plan["plan_sha256"] for r in rows), "prediction/plan mismatch")
    reports = []
    for repeat in range(run_plan["repeats"]):
        chosen = {identity for identity, r in expected if r == repeat}
        cases = [c for c in all_cases if c["id"] in chosen]
        require(len(cases) == len(chosen) and bool(cases), "unknown/empty planned cases")
        report = score(cases, [r for r in rows if r["repeat"] == repeat], [v for v in votes if v["repeat"] == repeat])
        reports.append(report)
    result = reports[0]
    details = [{**row, "case_id": row["case_id"] + f"@repeat-{i}"} for i, report in enumerate(reports) for row in report["cases"]]
    case_metadata = {case["id"]: case.get("momo_preset", {}) for case in all_cases}
    for row in details:
        metadata = case_metadata[row["case_id"].rsplit("@repeat-", 1)[0]]
        if metadata:
            row["dependency"] = metadata["dependency"]
            row["difficulty"] = metadata["difficulty"]
    # Repeated runs/lengths/languages remain inside their scenario cluster.
    for dimension in result["dimensions"]:
        families = {}
        for row in details:
            if row["dimension"] == dimension and row["primary"]:
                families.setdefault(row["family"], []).append(row["score"])
        complete = bool(families) and all(x is not None for xs in families.values() for x in xs)
        means = [sum(xs) / len(xs) for xs in families.values()] if complete else []
        result["dimensions"][dimension].update(score=sum(means) / len(means) if means else None,
                                               ci95_family_bootstrap=interval(means), complete=complete)
    scores = [r["score"] for r in result["dimensions"].values()]
    result["macro_score"] = sum(scores) / len(scores) if all(x is not None for x in scores) else None
    result["cases"] = details
    result["acgn_ablation"] = ablation_report(all_cases, details)
    from collections import Counter
    statuses = Counter(r["status"] for r in details)
    valid = sum(r["status"] not in ("missing", "invalid", "error") for r in details)
    result["coverage"] = {"planned": len(details), "valid_predictions": valid, "fraction": valid / len(details), "statuses": dict(statuses)}
    result["leakage"] = {"planned_cases": sum(r["leakage"]["planned_cases"] for r in reports),
                         "observed_violations": sum(r["leak"] for r in details), "note": "Missing/error responses do not establish privacy protection."}
    result["experiment"] = {"dataset_sha256": manifest["cases_sha256"], "plan_sha256": run_plan["plan_sha256"],
                            "protocol": run_plan["protocol"], "config": run_plan["config"], "repeats": run_plan["repeats"],
                            "python": platform.python_version(), "prediction_sha256": digest(rows), "judge_sha256": digest(votes)}
    result["slices"] = {}
    for field in ("language", "horizon"):
        result["slices"][field] = {str(value): {"cases": sum(r[field] == value for r in details),
            "unscored": sum(r[field] == value and r["score"] is None for r in details),
            "case_mean_diagnostic": (sum(r["score"] for r in details if r[field] == value) / sum(r[field] == value for r in details)
                                     if all(r["score"] is not None for r in details if r[field] == value) else None)}
            for value in sorted({r[field] for r in details}, key=str)}
    for field in ("dependency", "difficulty"):
        values = sorted({row[field] for row in details if field in row})
        if values:
            result["slices"][field] = {value: {
                "cases": sum(row.get(field) == value for row in details),
                "unscored": sum(row.get(field) == value and row["score"] is None for row in details),
                "case_mean_diagnostic": (
                    sum(row["score"] for row in details if row.get(field) == value)
                    / sum(row.get(field) == value for row in details)
                    if all(row["score"] is not None for row in details if row.get(field) == value)
                    else None),
            } for value in values}
    selected_dimensions = [name for name, value in result["dimensions"].items() if value["family_count"]]
    selected_values = [result["dimensions"][name]["score"] for name in selected_dimensions]
    selected_score = (sum(selected_values) / len(selected_values)
                      if selected_values and all(value is not None for value in selected_values) else None)

    chosen_cases = {identity: next(case for case in all_cases if case["id"] == identity)
                    for identity, _ in expected}
    objective_groups = {}
    for row in details:
        identity = row["case_id"].rsplit("@repeat-", 1)[0]
        case = chosen_cases[identity]
        if case["requires_judge"] or not row["primary"]:
            continue
        objective_groups.setdefault(row["dimension"], {}).setdefault(row["family"], []).append(row["score"])
    objective_dimensions = {}
    for dimension, families in objective_groups.items():
        family_scores = [sum(values) / len(values) for values in families.values()]
        objective_dimensions[dimension] = sum(family_scores) / len(family_scores)
    objective_score = (sum(objective_dimensions.values()) / len(objective_dimensions)
                       if objective_dimensions else None)
    diagnostic_groups = {}
    for row in details:
        if row["primary"] and row.get("facts_only_diagnostic") is not None:
            diagnostic_groups.setdefault(row["dimension"], {}).setdefault(row["family"], []).append(
                row["facts_only_diagnostic"]
            )
    diagnostic_dimensions = {
        dimension: sum(sum(values) / len(values) for values in families.values()) / len(families)
        for dimension, families in diagnostic_groups.items()
    }
    facts_only_diagnostic = (
        sum(diagnostic_dimensions.values()) / len(diagnostic_dimensions)
        if diagnostic_dimensions else None
    )
    result["score_summary"] = {
        "scale": "0..100",
        "status": ("incomplete_execution" if statuses.get("missing", 0) or statuses.get("error", 0) else
                   "invalid_predictions" if statuses.get("invalid", 0) else
                   "complete" if selected_score is not None else
                   "objective_only" if objective_score is not None else "requires_judges"),
        "selected_dimensions": selected_dimensions,
        "selected_score": round(selected_score * 100, 2) if selected_score is not None else None,
        "objective_dimensions": sorted(objective_dimensions),
        "objective_score": round(objective_score * 100, 2) if objective_score is not None else None,
        "facts_only_diagnostic": (
            round(facts_only_diagnostic * 100, 2) if facts_only_diagnostic is not None else None
        ),
        "coverage": round(result["coverage"]["fraction"] * 100, 2),
        "note": "Missing/error predictions retain the registered zero penalty, but do not mean execution completed or that an answer was factually wrong. Subjective cases require two independent judges or one auditable human adjudication."
    }
    result["performance"] = performance_report(rows, run_plan["config"], len(expected), votes)
    pipeline_cases = []
    for prediction in rows:
        probe = prediction.get("usage", {}).get("audits", {}).get("probe", {})
        request_audit = probe.get("request", {})
        state_audit = probe.get("state", {})
        retrieval = request_audit.get("memory_retrieval", {})
        state_input = state_audit.get("input", {})
        pipeline_cases.append({
            "case_id": prediction["case_id"],
            "repeat": prediction["repeat"],
            "retrieval_status": retrieval.get("status"),
            "retrieval_count": retrieval.get("count"),
            "state_input_fingerprint": state_input.get("fingerprint_sha256"),
            "state_input_scope": state_input.get("scope"),
            "state_injection_status": state_audit.get("injection_status"),
            "warnings": probe.get("warnings", []),
        })
    result["pipeline_audit"] = {
        "observed_probe_audits": sum(case["retrieval_status"] is not None for case in pipeline_cases),
        "retrieval_statuses": dict(Counter(case["retrieval_status"] for case in pipeline_cases
                                            if case["retrieval_status"] is not None)),
        "state_injection_statuses": dict(Counter(case["state_injection_status"] for case in pipeline_cases
                                                  if case["state_injection_status"] is not None)),
        "cases": pipeline_cases,
        "note": "IDs and fingerprints describe actual runtime inputs. Gold event IDs are not treated as retrieved document IDs.",
    }
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description="MORP-Bench; offline by default. AI commands require --allow-ai.")
    sub = parser.add_subparsers(dest="command", required=True)
    build = sub.add_parser("build")
    build.add_argument("--out", required=True)
    build.add_argument("--horizons", type=int, nargs="+", default=[50, 100, 500])
    build.add_argument("--variants", type=int, default=2)
    build.add_argument("--suite", choices=["all", "memory", "acgn", "momo", "stress", "roleplay"], default="all")
    validate = sub.add_parser("validate")
    validate.add_argument("dataset")
    planned = sub.add_parser("plan")
    planned.add_argument("dataset")
    planned.add_argument("--config", required=True)
    planned.add_argument("--split", choices=["dev", "eval", "all"], default="eval")
    planned.add_argument("--repeats", type=int, default=3)
    planned.add_argument("--dimension", action="append", choices=DIMENSIONS, default=[])
    planned.add_argument("--family", action="append", default=[])
    planned.add_argument("--character", action="append", default=[], help="ACGN character ID such as c01, or its exact name")
    planned.add_argument("--arm", action="append", choices=["label_free", "labeled", "labels_only"], default=[])
    planned.add_argument("--case-id", action="append", default=[])
    planned.add_argument("--dependency", action="append", choices=["context", "extracted"], default=[])
    planned.add_argument("--out", required=True)
    scenario_plan = sub.add_parser("scenario-plan", parents=[planned], add_help=False,
                                   help="Create paired basic_context/all_enabled plans, offline")
    scenario_plan.add_argument("--matrix", choices=["paired", "core", "causal"], default="paired")
    scenario_run = sub.add_parser("scenario-run")
    scenario_run.add_argument("dataset")
    scenario_run.add_argument("--plans", required=True)
    scenario_run.add_argument("--out", required=True)
    scenario_run.add_argument("--allow-ai", action="store_true")
    scenario_compare = sub.add_parser("scenario-compare")
    scenario_compare.add_argument("basic")
    scenario_compare.add_argument("all_enabled")
    scenario_compare.add_argument("--out", required=True)
    scenario_analyze = sub.add_parser("scenario-analyze")
    for scenario_name in scenarios.CAUSAL_SCENARIOS:
        scenario_analyze.add_argument(f"--{scenario_name.replace('_', '-')}", required=True)
    scenario_analyze.add_argument("--out", required=True)
    run = sub.add_parser("run")
    run.add_argument("dataset")
    run.add_argument("--plan", required=True)
    run.add_argument("--out", required=True)
    run.add_argument("--allow-ai", action="store_true")
    scoring = sub.add_parser("score")
    scoring.add_argument("dataset")
    scoring.add_argument("--plan", required=True)
    scoring.add_argument("--predictions", required=True)
    scoring.add_argument("--votes", nargs="*", default=[])
    scoring.add_argument("--out", required=True)
    jp = sub.add_parser("judge-plan")
    jp.add_argument("dataset")
    jp.add_argument("--predictions", required=True)
    jp.add_argument("--out", required=True)
    judge = sub.add_parser("judge")
    judge.add_argument("--plan", required=True)
    judge.add_argument("--config", required=True)
    judge.add_argument("--out", required=True)
    judge.add_argument("--allow-ai", action="store_true")
    cmp = sub.add_parser("compare")
    cmp.add_argument("left")
    cmp.add_argument("right")
    cmp.add_argument("--out", required=True)
    offline = sub.add_parser("offline")
    offline.add_argument("--probe", required=True)
    offline.add_argument("--out", required=True)
    sub.add_parser("sources")
    locked = sub.add_parser("pin")
    locked.add_argument("source", choices=SOURCES)
    locked.add_argument("path")
    locked.add_argument("--revision", required=True)
    locked.add_argument("--license-note", required=True)
    locked.add_argument("--out", required=True)
    locked.add_argument("--context", help="PersonaMem shared_contexts JSONL companion")
    imported = sub.add_parser("import")
    imported.add_argument("lock")
    imported.add_argument("--out", required=True)
    labels = sub.add_parser("label-metrics")
    labels.add_argument("kind", choices=["emotion", "stance"])
    labels.add_argument("input", help="JSON containing pairs, and labels for emotion")
    labels.add_argument("--out", required=True)
    calibration = sub.add_parser("calibration-plan")
    calibration.add_argument("--out", required=True)
    calibrated = sub.add_parser("calibration-score")
    calibrated.add_argument("--votes", nargs="+", required=True)
    calibrated.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    if args.command == "build":
        require(len(set(args.horizons)) == len(args.horizons) and all(10 <= h <= 2000 for h in args.horizons), "unique horizons must be 10..2000")
        require(1 <= args.variants <= 20, "variants must be 1..20")
        cases = (make_cases(tuple(sorted(args.horizons)), args.variants)
                 if args.suite in ("all", "memory") else [])
        if args.suite in ("all", "acgn"):
            cases += make_acgn_cases()
        if args.suite in ("all", "momo"):
            cases += make_momo_preset_cases(tuple(sorted(args.horizons)), args.variants)
        if args.suite in ("all", "stress"):
            cases += make_stress_cases()
        if args.suite in ("all", "roleplay"):
            cases += make_roleplay_v02_cases()
        provenance = {"source": "MOMO original", "horizons": args.horizons, "variants": args.variants, "suite": args.suite}
        if args.suite == "stress":
            provenance.update(horizons=[36], variants=2, suite_revision="0.2.0")
        elif args.suite == "roleplay":
            provenance.update(horizons=[24], variants=2, suite_revision="0.2.0")
        elif args.suite == "all":
            provenance.update(component_versions={"legacy": VERSION, "stress": "0.2.0", "roleplay": "0.2.0"})
        dataset_version = "0.2.0" if args.suite in ("all", "stress", "roleplay") else VERSION
        result = save_dataset(args.out, sorted(cases, key=lambda c: c["id"]), provenance, dataset_version)
    elif args.command == "validate":
        result, _ = load_dataset(args.dataset)
    elif args.command in ("plan", "scenario-plan"):
        manifest, cases = load_dataset(args.dataset)
        cases, selection = select_cases(cases, args.split, args.dimension, args.family,
                                        args.character, args.arm, args.case_id, args.dependency)
        if args.command == "scenario-plan":
            result = scenarios.create(cases, manifest, read_json(args.config), args.repeats, args.out, selection, args.matrix)
        else:
            result = plan(cases, manifest, read_json(args.config), args.repeats, selection)
            write_new(args.out, result)
    elif args.command == "scenario-run":
        manifest, cases = load_dataset(args.dataset)
        result = scenarios.run(args.plans, manifest, cases, args.out, args.allow_ai)
    elif args.command == "scenario-compare":
        result = scenarios.comparison(read_json(args.basic), read_json(args.all_enabled))
        write_new(args.out, result)
    elif args.command == "scenario-analyze":
        reports = {name: read_json(getattr(args, name)) for name in scenarios.CAUSAL_SCENARIOS}
        result = scenarios.causal_comparison(reports)
        write_new(args.out, result)
    elif args.command == "run":
        manifest, cases = load_dataset(args.dataset)
        run_plan = read_json(args.plan)
        require(run_plan["dataset_sha256"] == manifest["cases_sha256"], "plan/dataset mismatch")
        result = {"predictions": str(execute(run_plan, cases, args.out, args.allow_ai))}
    elif args.command == "score":
        result = score_run(args.dataset, read_json(args.plan), read_jsonl(args.predictions), [v for p in args.votes for v in read_jsonl(p)])
        write_new(args.out, result)
    elif args.command == "judge-plan":
        _, cases = load_dataset(args.dataset)
        result = judge_plan(cases, read_jsonl(args.predictions))
        write_new(args.out, result)
    elif args.command == "judge":
        execute_judge(read_json(args.plan), read_json(args.config), args.out, args.allow_ai)
        result = {"votes": args.out}
    elif args.command == "compare":
        result = compare(read_json(args.left), read_json(args.right))
        write_new(args.out, result)
    elif args.command == "offline":
        result = run_probe(Path(args.probe).resolve(strict=True))
        write_new(args.out, result)
        require(result["passed"] == result["total"], "offline runtime regression failed; see report")
    elif args.command == "pin":
        result = pin(args.source, args.path, args.revision, args.license_note, args.out, args.context)
    elif args.command == "import":
        lock = read_json(args.lock)
        result = save_dataset(args.out, import_locked(lock), lock)
    elif args.command == "label-metrics":
        data = read_json(args.input)
        result = emotion_metrics(data["pairs"], data["labels"]) if args.kind == "emotion" else stance_metrics(data["pairs"], data.get("low", 1), data.get("high", 5))
        result["input_sha256"] = digest(data)
        write_new(args.out, result)
    elif args.command == "calibration-plan":
        result, _ = calibration_plan()
        write_new(args.out, result)
    elif args.command == "calibration-score":
        result = calibration_score([v for p in args.votes for v in read_jsonl(p)])
        write_new(args.out, result)
    else:
        result = SOURCES
    from .common import canonical
    # Print a compact summary; full plans, labels and reports live in files.
    if args.command in ("plan", "judge-plan", "calibration-plan"):
        result = {k: v for k, v in result.items() if k not in ("requests", "jobs")}
    if args.command == "score":
        result = {k: v for k, v in result.items() if k != "cases"}
    print(canonical(result))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(f"MORP-Bench: {error}", file=sys.stderr)
        sys.exit(2)
