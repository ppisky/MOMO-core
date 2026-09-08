"""Versioned metrics. None means unmeasured, never a passing score."""
from collections import Counter, defaultdict
import math
import random
import unicodedata

from .common import canonical, digest, finite, require, unique
from .corpus import DIMENSIONS

POLICY = {"version": "morp.scoring/1", "aggregation": "field -> case -> family -> dimension -> macro",
          "missing_prediction": 0, "invalid_prediction": 0, "leak_gate": "case_zero",
          "judge_scale": [0, 4], "judge_disagreement_limit": 1,
          "bootstrap_unit": "family", "bootstrap_samples": 2000, "seed": 20260905,
          "minimum_families_for_interval": 2, "human_calibrated": False,
          "primary_acgn_arm": "label_free", "evidence_metrics": "self-report diagnostic only",
          "invalid_judge": None, "human_adjudication": "one evidence-bound final vote"}


def norm(value):
    return unicodedata.normalize("NFKC", value).casefold().strip()


def same(actual, expected):
    # No truthiness, substring matching, fuzzy numeric coercion or "False == 0".
    if type(actual) is not type(expected):
        return False
    if isinstance(expected, str):
        return norm(actual) == norm(expected)
    return actual == expected


def retrieval_metrics(actual, expected):
    require(isinstance(actual, list) and all(isinstance(x, str) for x in actual), "evidence_ids must be strings")
    require(len(set(actual)) == len(actual), "duplicate retrieved evidence")
    relevant, retrieved = set(expected), set(actual)
    if not relevant:
        return {"precision": 1.0 if not retrieved else 0.0, "recall": None, "f1": None, "ndcg": None}
    hits = len(relevant & retrieved)
    precision = hits / len(retrieved) if retrieved else 0.0
    recall = hits / len(relevant)
    dcg = sum(1 / math.log2(i + 2) for i, item in enumerate(actual) if item in relevant)
    ideal = sum(1 / math.log2(i + 2) for i in range(min(len(actual), len(relevant))))
    return {"precision": precision, "recall": recall,
            "f1": 2 * precision * recall / (precision + recall) if precision + recall else 0.0,
            "ndcg": dcg / ideal if ideal else 0.0}


def js_divergence(left, right):
    """Base-2 JSD in [0,1], no smoothing. Empty distributions are not measured."""
    require(all(finite(x) and x >= 0 for x in left + right), "invalid distribution")
    require(len(left) == len(right) and len(left) > 0, "distribution shape mismatch")
    if not sum(left) or not sum(right):
        return None
    p, q = [x / sum(left) for x in left], [x / sum(right) for x in right]
    m = [(a + b) / 2 for a, b in zip(p, q)]
    return sum(a * math.log2(a / mid) / 2 if a else 0 for a, mid in zip(p, m)) + sum(
        b * math.log2(b / mid) / 2 if b else 0 for b, mid in zip(q, m))


def emotion_metrics(pairs, labels):
    """Local label diagnostics; not a reimplementation of EmoCharacter's EC/REC."""
    require(len(set(labels)) == len(labels) and bool(labels), "invalid labels")
    require(bool(pairs), "no emotion observations")
    require(all(a in labels and b in labels for a, b in pairs), "unknown emotion label")
    f1s = []
    for label in labels:
        tp = sum(a == label and b == label for a, b in pairs)
        fp = sum(a != label and b == label for a, b in pairs)
        fn = sum(a == label and b != label for a, b in pairs)
        if 2 * tp + fp + fn:
            f1s.append(2 * tp / (2 * tp + fp + fn))
    gold, pred = Counter(a for a, _ in pairs), Counter(b for _, b in pairs)
    return {"accuracy": sum(a == b for a, b in pairs) / len(pairs),
            "macro_f1_supported_labels": sum(f1s) / len(f1s),
            "distribution_jsd": js_divergence([gold[l] for l in labels], [pred[l] for l in labels]),
            "n": len(pairs), "profile": "local-emotion-label-diagnostic/1"}


def stance_metrics(pairs, low=1, high=5):
    require(high > low and bool(pairs), "invalid stance scale or empty observations")
    require(all(finite(a) and finite(b) and low <= a <= high and low <= b <= high for a, b in pairs), "invalid stance")
    # Signed bias and spread distinguish premature convergence from accuracy.
    gold, pred = [a for a, _ in pairs], [b for _, b in pairs]
    variance = lambda xs: sum((x - sum(xs) / len(xs)) ** 2 for x in xs) / len(xs)
    return {"normalized_mae": sum(abs(a - b) for a, b in pairs) / len(pairs) / (high - low),
            "signed_bias": (sum(pred) - sum(gold)) / len(pairs), "human_variance": variance(gold),
            "candidate_variance": variance(pred), "n": len(pairs), "profile": "local-stance-diagnostic/1"}


def interval(values):
    if len(values) < POLICY["minimum_families_for_interval"]:
        return None
    rng = random.Random(POLICY["seed"])
    samples = sorted(sum(rng.choices(values, k=len(values))) / len(values)
                     for _ in range(POLICY["bootstrap_samples"]))
    return [samples[int(.025 * len(samples))], samples[int(.975 * len(samples))]]


def grade_judges(case, prediction, votes):
    if not votes:
        return None, "unjudged"
    judges = set()
    scores = []
    human = []
    for vote in votes:
        require(vote["case_id"] == case["id"] and vote["prediction_sha256"] == digest(prediction), "judge/prediction mismatch")
        require(vote["rubric_sha256"] == digest(case["expected"]["rubric"]), "judge/rubric mismatch")
        require(isinstance(vote["judge"], str) and vote["judge"] and vote["judge"] not in judges, "duplicate/empty judge")
        judges.add(vote["judge"])
        require(type(vote["score"]) is int and 0 <= vote["score"] <= 4, "invalid judge score")
        quote = vote.get("quote", "")
        require(isinstance(quote, str) and bool(quote.strip()) and quote in prediction["answer"], "judge quote is not candidate evidence")
        require(isinstance(vote.get("reason"), str) and bool(vote["reason"].strip()), "missing judge reason")
        scores.append(vote["score"])
        if vote.get("source") == "human":
            require(isinstance(vote.get("reviewer"), str) and bool(vote["reviewer"].strip()), "human adjudication requires reviewer identity")
            human.append(vote["score"])
    require(len(human) <= 1, "conflicting human adjudications")
    if human:
        return human[0] / 4, "human_adjudicated"
    if len(scores) < 2:
        return None, "needs_second_judge"
    if max(scores) - min(scores) > POLICY["judge_disagreement_limit"]:
        return None, "needs_human_review"
    return sum(scores) / len(scores) / 4, "provisional_judge"


def score(cases, predictions, votes=()):
    by_id = unique(predictions, "case_id")
    require(set(by_id) <= {case["id"] for case in cases}, "predictions contain unknown or out-of-split cases")
    judges = defaultdict(list)
    for vote in votes:
        require(vote["case_id"] in by_id, "orphan judge vote")
        judges[vote["case_id"]].append(vote)
    details, grouped = [], defaultdict(lambda: defaultdict(list))
    for case in cases:
        prediction = by_id.get(case["id"])
        row = {"case_id": case["id"], "family": case["family"], "dimension": case["dimension"],
               "language": case["language"], "horizon": case["horizon"], "score": 0.0, "status": "missing", "leak": False}
        row["primary"] = case.get("ablation", {}).get("arm", "label_free") == "label_free"
        if prediction:
            require(prediction.get("case_sha256") == digest(case), "prediction/case content mismatch")
            if prediction.get("status") == "error":
                row["status"] = "error"
            elif prediction.get("status") == "ok":
                try:
                    require(isinstance(prediction["answer"], str) and bool(prediction["answer"].strip()), "empty answer")
                    require(isinstance(prediction["facts"], dict) and set(prediction["facts"]) == set(case["facts_schema"]), "facts schema mismatch")
                    expected = case["expected"]["facts"]
                    if expected:
                        row["facts_only_diagnostic"] = (
                            sum(same(prediction["facts"][key], value) for key, value in expected.items())
                            / len(expected)
                        )
                    evidence_valid = set(prediction["evidence_ids"]) <= {e["id"] for e in case["history"]}
                    row["evidence_status"] = "verified_event_ids" if evidence_valid else "invalid_or_unmapped_ids"
                    row["evidence_diagnostic"] = retrieval_metrics(prediction["evidence_ids"], case["expected"]["evidence_ids"])
                    require(evidence_valid, "invented or unmapped evidence ID")
                    row["leak"] = any(norm(secret) in norm(canonical(prediction)) for secret in case["expected"]["forbidden"])
                    if case["requires_judge"]:
                        try:
                            row["score"], row["status"] = grade_judges(case, prediction, judges[case["id"]])
                        except (ValueError, KeyError, TypeError):
                            row["score"], row["status"] = None, "invalid_judgement"
                    else:
                        row["score"] = sum(same(prediction["facts"][k], v) for k, v in expected.items()) / len(expected)
                        row["status"] = "scored"
                    if row["leak"]:
                        row.update(score=0.0, status="leak")
                except (ValueError, KeyError, TypeError):
                    row.update(score=0.0, status="invalid")
            else:
                raise ValueError("unknown prediction status")
        details.append(row)
        if row["primary"]:
            grouped[case["dimension"]][case["family"]].append(row["score"])
    dimensions = {}
    for dimension in DIMENSIONS:
        families = grouped[dimension]
        complete = bool(families) and all(all(x is not None for x in xs) for xs in families.values())
        means = [sum(xs) / len(xs) for xs in families.values()] if complete else []
        dimensions[dimension] = {"score": sum(means) / len(means) if means else None,
                                 "family_count": len(families), "ci95_family_bootstrap": interval(means),
                                 "complete": complete}
    scores = [x["score"] for x in dimensions.values()]
    observed = sum(row["status"] not in ("missing", "error", "invalid") for row in details)
    leaks = [row for row in details if next(c for c in cases if c["id"] == row["case_id"])["expected"]["forbidden"]]
    return {"schema": "morp.report/1", "policy": POLICY, "policy_sha256": digest(POLICY),
            "macro_score": sum(scores) / len(scores) if all(x is not None for x in scores) else None,
            "dimensions": dimensions, "coverage": {"planned": len(cases), "valid_predictions": observed,
                "fraction": observed / len(cases) if cases else 0, "statuses": dict(Counter(r["status"] for r in details))},
            "leakage": {"planned_cases": len(leaks), "observed_violations": sum(r["leak"] for r in leaks),
                        "note": "Missing/error responses are not evidence of privacy protection."},
            "publication_status": "provisional_synthetic_not_human_calibrated", "cases": details}


def compare(left, right):
    require(left["policy_sha256"] == right["policy_sha256"], "scoring policy mismatch")
    require(left["experiment"]["dataset_sha256"] == right["experiment"]["dataset_sha256"], "dataset mismatch")
    require(left["experiment"]["protocol"] == right["experiment"]["protocol"], "protocol mismatch")
    a, b = unique(left["cases"], "case_id"), unique(right["cases"], "case_id")
    require(a.keys() == b.keys(), "case selections differ")
    families = defaultdict(list)
    for identity in sorted(a):
        if not a[identity]["primary"]:
            continue
        require(a[identity]["score"] is not None and b[identity]["score"] is not None, "unjudged cases cannot be compared")
        families[a[identity]["family"]].append(b[identity]["score"] - a[identity]["score"])
    deltas = [sum(values) / len(values) for values in families.values()]
    return {"paired_family_macro_delta": sum(deltas) / len(deltas), "ci95": interval(deltas),
            "primary_dimension_macro_delta": (right["macro_score"] - left["macro_score"] if left["macro_score"] is not None and right["macro_score"] is not None else None),
            "family_count": len(deltas), "direction": "right-minus-left",
            "claim": "family-weighted diagnostic; main score uses equal dimension weights; neither is a significance guarantee"}


def ablation_report(cases, details):
    """Paired family-level effects. No label matching is used as a target."""
    metadata = {c["id"]: c for c in cases if "ablation" in c}
    pairs = defaultdict(dict)
    for row in details:
        identity, repeat = row["case_id"].rsplit("@repeat-", 1)
        if identity in metadata:
            design = metadata[identity]["ablation"]
            pairs[(design["pair"], repeat)][design["arm"]] = (row["family"], row["score"])
    output = {"pair_count": len(pairs), "scored_pairs": 0, "effects": {}, "interpretation": "score label-free minus comparison; mechanism-based blind rubric, not stereotype recognition"}
    for comparison in ("labeled", "labels_only"):
        clusters = defaultdict(list)
        missing = 0
        for arms in pairs.values():
            if any(arm not in arms or arms[arm][1] is None for arm in ("label_free", comparison)):
                missing += 1
                continue
            family, baseline = arms["label_free"]
            clusters[family].append(baseline - arms[comparison][1])
        deltas = [sum(values) / len(values) for values in clusters.values()]
        output["effects"][comparison] = {"mean_delta": sum(deltas) / len(deltas) if deltas and missing == 0 else None,
                                         "ci95_family_bootstrap": interval(deltas) if missing == 0 else None,
                                         "family_count": len(clusters), "missing_pairs": missing,
                                         "noninferiority_margin_exploratory": .05}
        if comparison == "labeled":
            output["scored_pairs"] = len(pairs) - missing
    return output
