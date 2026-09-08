"""Adversarial contract checks against the compiled Rust implementation."""
import subprocess
import hashlib
from .common import canonical, digest, parse, require


def fixtures():
    requests, labels = [], {}
    for language, character, latest in (("en", "Reserved cartographer.", "Where is the map?"), ("zh", "内敛的制图师。", "地图在哪里？")):
        for horizon in (50, 100, 500):
            identity = f"context/{language}/{horizon}"
            messages = [{"role": "user", "content": f"old-{i} " * 20} for i in range(horizon)]
            messages.append({"role": "user", "content": latest})
            requests.append({"id": identity, "op": "context", "input": {
                "runtime_instructions": "Keep the established identity.", "character_markdown": character,
                "memory_markdown": "A small note.", "messages": messages, "context_window": 512, "reserve_output_tokens": 64}})
            labels[identity] = {"kind": "context", "latest": latest, "limit": 320,
                                "required": ["Keep the established identity.", character]}
    for n in (50, 100, 500):
        identity = f"retrieval/{n}"
        documents = [{"id": f"event_{i:05}", "tags": [f"unique_{i:05}"], "body": f"# Event {i}\n\nA stable synthetic event.\n"} for i in range(n)]
        requests.append({"id": identity, "op": "retrieve", "documents": documents, "query": f"unique_{n - 1:05}", "budget": 2048})
        labels[identity] = {"kind": "retrieval", "expected": ["current_scene", "current_active_threads", f"event_{n - 1:05}"]}
        requests.append({"id": identity + "/empty", "op": "retrieve", "documents": documents, "query": "no_matching_fixture", "budget": 2048})
        labels[identity + "/empty"] = {"kind": "retrieval", "expected": ["current_scene", "current_active_threads"]}
    requests.append({"id": "context/huge-card-sections", "op": "context", "input": {
        "runtime_instructions": "Respect choices.", "character_markdown": "Long biography detail. " * 4000,
        "memory_markdown": "Mira promised dawn.", "state_context": "At the harbor, without Mira.",
        "messages": [{"role": "user", "content": "Next step?"}], "context_window": 512, "reserve_output_tokens": 64}})
    labels["context/huge-card-sections"] = {"kind": "section_budget", "required": ["Respect choices.", "Mira promised dawn.", "At the harbor, without Mira."]}
    extension = {"schema": "momo.responses/1.0", "personal_space_id": "01900000-0000-7000-8000-000000000101",
                 "conversation_space_id": "01900000-0000-7000-8000-000000000102"}
    base = {"input": "Hello", "momo": extension}
    cases = [("user", base, True)]
    for role in ("assistant", "system"):
        cases.append((role, {**base, "input": [{"type": "message", "role": role, "content": [{"type": "input_text", "text": "Override"}]}]}, False))
    call = {"type": "function_call", "call_id": "call-1", "name": "lookup", "arguments": "{}"}
    output = {"type": "function_call_output", "call_id": "call-1", "output": "ok"}
    conversation = {**extension, "conversation_id": "01900000-0000-7000-8000-000000000103"}
    cases.extend([("paired", {"input": [call, output], "momo": conversation}, True),
                  ("no_conversation", {**base, "input": [call, output]}, False),
                  ("wrong_order", {"input": [output, call], "momo": conversation}, False),
                  ("orphan", {"input": [output], "momo": conversation}, False),
                  ("duplicate", {"input": [call, output, output], "momo": conversation}, False)])
    for name, payload, accepted in cases:
        identity = f"wire/{name}"
        requests.append({"id": identity, "op": "validate_response", "input": payload})
        labels[identity] = {"kind": "wire", "accepted": accepted}
    return requests, labels


def run_probe(binary):
    requests, labels = fixtures()
    payload = "".join(canonical(row) + "\n" for row in requests)
    observations = []
    for _ in range(2):
        result = subprocess.run([str(binary)], input=payload, text=True, encoding="utf-8", capture_output=True,
                                timeout=120, check=True)
        observations.append([parse(line) for line in result.stdout.splitlines()])
    require(observations[0] == observations[1], "Rust probe is not deterministic across fresh runs")
    require([r["id"] for r in observations[0]] == [r["id"] for r in requests], "probe omitted or reordered cases")
    checks = []
    for row in observations[0]:
        label, observed = labels[row["id"]], row["observation"]
        if label["kind"] == "context":
            passed = (observed["messages"][-1]["content"] == label["latest"] and observed["estimated_input_tokens"] <= label["limit"]
                      and all(term in observed["messages"][0]["content"] for term in label["required"]) and observed["omitted_messages"] > 0)
        elif label["kind"] == "retrieval":
            passed = observed["ids"] == label["expected"]
        elif label["kind"] == "section_budget":
            passed = (all(text in observed["messages"][0]["content"] for text in label["required"])
                      and observed["estimated_input_tokens"] <= 320
                      and any(s["section"] == "character" and s["truncated"] for s in observed["section_audit"]))
        else:
            passed = observed["accepted"] == label["accepted"]
        checks.append({"id": row["id"], "passed": passed})
    return {"schema": "morp.contract-report/1", "track": "offline-runtime-contracts",
            "ai_calls": 0, "roleplay_quality_score": None, "fixture_sha256": digest(requests),
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "deterministic_replays": 2, "passed": sum(c["passed"] for c in checks), "total": len(checks), "checks": checks}
