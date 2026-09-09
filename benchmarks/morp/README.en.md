# MORP-Bench 1.0

[简体中文](README.md)

MORP means **MOMO Role-Playing Benchmark**. Its primary score measures whether a model or product can perform a particular character naturally and consistently.

Version 1.0 contains 64 bilingual counterfactual cases in 16 families and eight equally weighted dimensions: character fidelity, emotional continuity, relationship dynamics, user agency, world embodiment, bounded initiative, narrative coherence, and epistemic viewpoint. Each dimension has two independent families.

Memory recall, fact extraction, evidence IDs, retrieval quality, and maintenance throughput are not part of the role-play score. The former generators remain available only through explicit legacy suite names.

The native MOMO protocol stages the authored user/assistant transcript exactly and makes one candidate call for the final turn. It does not manufacture history with placeholder assistant replies or multiply candidate calls by transcript length. The OpenAI-compatible baseline also replays real message roles.

Build and plan offline:

```bash
python -m benchmarks.morp build --out target/morp-roleplay
python -m benchmarks.morp validate target/morp-roleplay
python -m benchmarks.morp plan target/morp-roleplay --config path/to/candidate.json --split eval --repeats 3 --out target/morp-roleplay.plan.json
```

Execution requires an explicit `--allow-ai`. A candidate returns its natural performed turn as plain text; an answer-only JSON object remains accepted for adapter compatibility. One identity-bound reviewer scores each successful answer from 0 to 4 with a grounded quote and reason. Codex may fill this auditable reviewer role without being represented as a human. Two independent model judges remain an optional compatibility path, not a release dependency. Aggregation is review → case → family → dimension → equal-weight eight-dimension score. The 0–100 `roleplay_score` remains null while a successful answer is unreviewed or a planned prediction is missing; terminal candidate errors retain their registered zero penalty.

Create an offline blind plan and editable reviewer rows with `judge-plan` followed by `review-template --reviewer codex:rc2`. Fill `score`, `quote`, and `reason`, set each row's `status` to `ok`, then pass that single JSONL file to `score --votes`.

Legacy diagnostic generators are available as `memory`, `acgn`, `momo`, `stress`, and `legacy-all`. Their reports are not MORP 1.0 role-playing scores and must not be mixed with the primary leaderboard.

The public cases are original development material and are not yet broadly human-calibrated. Results are suitable for regression and product comparisons, not universal claims about role-play quality.
