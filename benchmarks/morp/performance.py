"""Observed client timing and explicitly scoped token-price estimates."""
import math

from .common import finite, require


def validate_pricing(pricing):
    require(isinstance(pricing, dict) and set(pricing) == {
        "currency", "input_per_million", "output_per_million"}, "invalid pricing fields")
    require(isinstance(pricing["currency"], str) and bool(pricing["currency"].strip()), "currency required")
    require(all(finite(pricing[k]) and pricing[k] >= 0 for k in
                ("input_per_million", "output_per_million")), "prices must be finite and nonnegative")


def tokens(usage):
    if not isinstance(usage, dict):
        return None
    incoming = usage.get("input_tokens", usage.get("prompt_tokens"))
    outgoing = usage.get("output_tokens", usage.get("completion_tokens"))
    if any(type(v) is not int or v < 0 for v in (incoming, outgoing)):
        return None
    return incoming, outgoing


def distribution(values):
    values = sorted(v for v in values if finite(v) and v >= 0)
    return {"count": len(values), "mean": sum(values) / len(values) if values else None,
            "p50": values[math.ceil(.5 * len(values)) - 1] if values else None,
            "p95": values[math.ceil(.95 * len(values)) - 1] if values else None,
            "p99": values[math.ceil(.99 * len(values)) - 1] if values else None}


def report(rows, config, planned, votes=()):
    pricing = config.get("pricing")
    if pricing is not None:
        validate_pricing(pricing)
    calls, probe, ingestion, drains, post_drains = [], [], [], [], []
    per_case = []
    cycle_drains = []
    for row in rows:
        usage = row.get("usage", {})
        native = "per_response" in usage
        observed = usage.get("per_response", []) if native else [usage]
        calls.extend(observed)
        timings = usage.get("timings", [])
        cycle_drains.extend(usage.get("cycle_barriers", {}).values())
        probe_time = next((t["seconds"] for t in timings if t["event_id"] == "probe"), None)
        if not native and row.get("status") == "ok":
            probe_time = row.get("attempt_seconds")
        if probe_time is not None:
            probe.append(probe_time)
        history_times = [t["seconds"] for t in timings if t["event_id"] != "probe"]
        if history_times:
            ingestion.append(sum(history_times))
        drain = usage.get("drain_seconds")
        if drain is not None:
            drains.append(drain)
        if usage.get("post_probe_drain_seconds") is not None:
            post_drains.append(usage["post_probe_drain_seconds"])
        valid = [tokens(u) for u in observed]
        complete = bool(valid) and all(v is not None for v in valid)
        known_usage = [v for v in valid if v is not None]
        estimate = (sum(v[0] * pricing["input_per_million"] + v[1] * pricing["output_per_million"]
                        for v in known_usage) / 1_000_000 if known_usage and pricing else None)
        per_case.append({"case_id": row["case_id"], "repeat": row["repeat"],
                         "probe_seconds": probe_time, "observed_response_cost": estimate,
                         "response_usage_complete": complete,
                         "status": row.get("status")})
    counts = [tokens(u) for u in calls]
    known = [c for c in counts if c is not None]
    complete = len(rows) == planned and all(r.get("status") == "ok" for r in rows) and bool(counts) and len(known) == len(counts)
    costs = [r["observed_response_cost"] for r in per_case]
    native = config["backend"] == "momo"
    judge_costs = {}
    judge_unpriced = 0
    for vote in votes:
        rate, count = vote.get("pricing"), tokens(vote.get("usage"))
        if rate is None or count is None:
            judge_unpriced += 1
            continue
        validate_pricing(rate)
        currency = rate["currency"]
        judge_costs[currency] = judge_costs.get(currency, 0) + (
            count[0] * rate["input_per_million"] + count[1] * rate["output_per_million"]) / 1_000_000
    return {"schema": "morp.performance/1", "planned_cases": planned, "recorded_cases": len(rows),
            "failed_cases": sum(r.get("status") != "ok" for r in rows),
            "attempt_seconds": distribution([r.get("attempt_seconds") for r in rows]),
            "probe_seconds": distribution(probe), "ingestion_response_seconds": distribution(ingestion),
            "maintenance_barrier_seconds": distribution(drains),
            "cycle_barrier_seconds": distribution(cycle_drains),
            "post_probe_barrier_seconds": distribution(post_drains),
            "tokens": {"observed_calls": len(counts), "calls_with_usage": len(known),
                       "input": sum(c[0] for c in known), "output": sum(c[1] for c in known),
                       "complete_for_recorded_responses": complete},
            "cost": {"currency": pricing["currency"] if pricing else None,
                     "observed_response_estimate": sum(c for c in costs if c is not None) if pricing and any(c is not None for c in costs) else None,
                     "total_estimate": sum(costs) if complete and pricing and not native else None,
                     "complete": bool(complete and pricing and not native),
                     "unmeasured": (["maintenance", "embeddings", "gateway_internal_calls"] if native else []) +
                                   (["failed_or_missing_response_usage"] if not complete else []),
                     "note": "Flat input/output list-price estimate; no cache discounts. Native response usage is not total provider cost."},
            "judges": {"recorded_votes": len(votes),
                       "observed_cost_by_currency": judge_costs, "unpriced_votes": judge_unpriced,
                       "attempt_seconds": distribution([v.get("attempt_seconds") for v in votes]),
                       "usage": [v.get("usage", {}) for v in votes],
                       "note": "Judge usage is separate from candidate cost; price each judge with its own rates."},
            "cases": per_case,
            "timing_note": "Client non-streaming response latency, not TTFT. Checkpointed response timings survive resume; attempt duration measures this invocation only. Barrier time can overlap prior background work."}
