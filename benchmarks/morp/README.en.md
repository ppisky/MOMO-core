# MORP-Bench 0.2

Paired native `basic_context` / `all_enabled` experiments are available through
`scenario-plan`, `scenario-run`, and `scenario-compare`. MOMO dependency presets
ingest the same events but do not replay full history at the probe: `context`
continues the active conversation, while `extracted` opens a fresh conversation.
DMW, NSG and MO State are toggled together under the same budgets.
The reports include quality, client response latency and observed token-price
estimates. Native total cost remains unmeasured without background provider bills.
See the [scenario workflow and accounting boundaries](SCENARIOS.md) (Chinese).

[简体中文](README.md)

MORP means **MOMO Role-playing**. This directory contains an
original role-playing and memory benchmark, ACGN label ablations, source
adapters, immutable execution plans, checkpointed runs, scoring, judge
calibration, and paired comparison. It uses the Python 3.13 standard library
only. Offline commands need no API key and do not contact a model service.

The default corpus has **511 test points in 53 scenario families**: 264 memory
and interaction points, 135 points from 15 original adult ACGN characters,
72 MOMO dependency presets, 16 objective v0.2 stress projections, and 24 v0.2
role-playing projections. Expanded points are not 511
independent samples. Scenario family, character, language, horizon, arm, and
repeat are clustered during aggregation.

All original material was authored by the AI assistant during this development
cycle and has not received independent human annotation or preference
validation. Version 0.1 is a public development/regression benchmark, not a
contamination-resistant hidden leaderboard; the `eval` split is not secret.
Original code and data recipes use the repository Apache-2.0 license. External
data retains its own license.

## One-command offline validation

Run from the repository root:

```bash
bash scripts/test-morp.sh
```

The Windows PowerShell equivalent is `./scripts/test-morp.ps1`.

The script runs Python unit tests, builds the real Rust probe, generates the
dataset and native plan, then replays context, DMW, and request-contract checks
in a fresh workspace. Reports go to `target/morp-offline-...`. `ai_calls` must
be zero and `roleplay_quality_score` must be null. Simulated responses derived
from expected answers are never reported as model quality.

## Original scenario design

| Dimension | Representative scenarios | What is scored |
| --- | --- | --- |
| recall | Gift location, preferences, cross-session promises | Exact answers supported by history |
| update | Explicit correction, out-of-order old logs, unverified rumor | Latest credible state rather than the last arriving string |
| reasoning | Ownership to storage, elapsed time, a newly learned code | Cross-event reasoning and local-rule application |
| boundary | Unknown facts, unrevealed plot, private source, forgetting request | Knowledge boundaries and behavioral non-disclosure |
| world | Inventory, scene transition, participant departure | World-state and causal consistency |
| persona | User agency, identity, ACGN mechanisms | Behavior grounded in experience rather than a catchphrase |
| emotion | Loss, reunion, expression under stress | Contextual character consistency, not permanent positivity |
| social | Unsupported peer pressure, belief revision after evidence | Reasonable persistence and reasonable change |

Memory recipes support 50, 100, and 500 historical events, split into a session
every 25 events. Fixed hashes generate fictional locations, gifts, and drinks
so answers cannot be guessed from character names or common knowledge. This is
a templated stress test with limited filler diversity, not proof of coverage for
the full complexity of a real 500-turn narrative. `forget` tests output
non-disclosure, not disk erasure or model-weight forgetting.

Version 0.1.2 adds a MOMO-native preset suite with three original adult
characters grounded in experience, beliefs, needs, rules, relationships, and
behavioral tendencies. Six bilingual families cover current-scene overrides,
local protocols, agency boundaries, corrected commitments, relational
inference, and persistent relationship boundaries. The `context` and
`extracted` probe dependencies are explicit metadata and plan filters.

Expected values, evidence labels, forbidden strings, judge rubrics, and source
metadata never reach the candidate model. Full-context input uses an allowlist
and removes invisible Spaces. The native flow writes private events to a
different Space that the final probe does not read. This depends on correct host
Space selection and does not prove remote authorization that the local server
does not implement.

## ACGN: does the person survive removal of the label?

Every character in `acgn.py` has six mechanisms: experience, beliefs, needs,
rules, relationships, and behavioral tendencies. Labels—such as tsundere,
low-expression, energetic, strategic, possessive, or villain-coded—organize an
experiment; they are neither the definition of authenticity nor medical
diagnoses. Every character is an original adult. Relationship-coded labels do
not automatically create family or romantic relationships.

Each situation has three arms:

1. `label_free`: the six mechanisms only; this is the primary scoring arm.
2. `labeled`: the same mechanisms plus the label.
3. `labels_only`: name, adult status, and label only; a missing-causal-detail control.

The three arms share history, question, reference mechanism, and rubric. Judges
always see the same label-free mechanism and do not see model identity, arm, or
label. Wording, catchphrases, and label keywords are not score targets.

Trust, stress, closeness, and publicity use a 0..1 situational scale. MORP 0.1
pre-registers three combined conditions: public/low-trust,
private/high-trust, and crisis/high-trust/high-stress. They test contextual
differences but cannot isolate one knob's causal effect. A future response-curve
experiment must change one knob while holding all others fixed.

`acgn_ablation` reports `label_free - labeled` and
`label_free - labels_only`, clustered by character. A missing paired score
suppresses the full effect. Control arms do not enter the primary persona score.
The 0.05 margin is a pre-registered exploratory non-inferiority margin and does
not by itself justify an equivalence claim.

## Safe model runner

The wrapper defaults to **plan only, with zero model calls**. It creates a new
output directory, validates the data, prints the protocol and
`candidate_calls`, and stops unless `--allow-ai` is explicitly present:

```bash
bash scripts/run-morp-model.sh \
  --config benchmarks/morp/configs/baseline.example.json
```

Replace every `REPLACE_...` value, keep credentials only in the environment
variable named by `api_key_env`, and inspect the planned call count before
enabling execution:

```bash
bash scripts/run-morp-model.sh \
  --config path/to/candidate.json \
  --allow-ai
```

The Windows equivalent is `./scripts/run-morp-model.ps1`, with PowerShell-style
parameters such as `-Config` and `-AllowAI`.

The wrapper does not start MOMO Core or a model gateway. With `backend: "momo"`,
start Core against a dedicated evaluation data directory and point `base_url`
to Core's `/v1`. With `backend: "openai"`, point `base_url` to an
OpenAI-compatible `/v1`. The two protocols produce separate reports and must
not be mixed on a leaderboard.

## Lightweight selection and explicit scores

The following objective profile takes one family from recall, update,
reasoning, boundary, and world at a 10-event horizon. It contains ten bilingual
cases. An OpenAI-compatible direct run needs 10 candidate calls; a native MOMO
run needs 110, plus provider-dependent maintenance calls.

```bash
bash scripts/run-morp-model.sh \
  --config path/to/candidate.json \
  --suite memory \
  --split eval \
  --horizon 10 \
  --family gift \
  --family correction \
  --family two_hop \
  --family private \
  --family inventory
```

Without `--allow-ai`, this still stops after planning. A 10-event profile is a
smoke test and is not a standard 50/100/500 stress result. After execution it
produces an `objective_score` on a 0–100 scale without model judges.

To inspect one character, select its ID or exact Chinese name. This example
selects Akabane Rin's primary arm in three situations: three direct-model calls
or nine native-MOMO calls.

```bash
bash scripts/run-morp-model.sh \
  --config path/to/candidate.json \
  --suite acgn \
  --split all \
  --character c01 \
  --arm label_free
```

Omit `--arm` to include all three arms: nine direct calls or 27 native calls.
Repeat `--character` for more characters. `--dimension`, `--family`,
`--character`, `--arm`, `--case-id`, and `--dependency` are repeatable; different filter kinds
combine with AND. The immutable plan records the complete selection. Unknown
values and empty selections fail before execution.

Every report includes a `score_summary` on a 0–100 scale:

- `objective_score`: equal-weight macro average over selected objective dimensions;
- `facts_only_diagnostic`: objective fact matching retained even when evidence IDs cannot be
  mapped; it diagnoses extraction versus evidence-protocol failures and is not the strict score;
- `selected_score`: macro average over every selected primary dimension;
- `coverage`: percentage of valid responses, not answer quality;
- `status`: `complete`, `objective_only`, or `requires_judges`.

Persona, emotion, and social quality cannot receive a fabricated deterministic
score. `selected_score` stays null until two independent model judges agree or
one auditable human adjudication supplies a quote, reason, and reviewer.

## Build, plan, and run

For native attribution, `"history_mode":"recorded"` writes a fixed transcript
to the local conversation and maintenance queue, so the candidate model only
answers the final probe. Use `"live"` for small trajectory samples where every
candidate reply can feed later maintenance. Report the protocols separately.

```bash
python3 -m benchmarks.morp build --out target/morp-dataset
python3 -m benchmarks.morp validate target/morp-dataset
python3 -m benchmarks.morp plan target/morp-dataset \
  --config benchmarks/morp/configs/momo.example.json \
  --out target/morp-plan.json
python3 -m benchmarks.morp calibration-plan \
  --out target/morp-calibration-plan.json
```

`build --suite acgn` creates only the character experiment;
`--suite memory` creates memory/interaction cases, and `--suite momo` creates
the MOMO context/extracted presets. `--horizons 50 100 500` and
`--variants 2` control expansion. Plans default to `eval` and three repeats;
use `--split dev` for development. A plan records input, dataset, harness,
protocol, deployment-revision, and selection hashes plus the predicted
candidate-call count. Maintenance and embedding calls are additional.

Two protocols are reported separately:

- `full-context-replay/1` supplies all authorized history in one call and
  measures context use without background memory writes.
- `momo-online-sessions/1` ingests history through `/v1/momo/responses`, creates
  new conversations across sessions, and probes memory in a fresh conversation.
  ACGN probes remain in the current conversation to avoid conflating immediate
  situational expression with memory-distillation latency.

Native runs use independent random Spaces and do not delete user Spaces. Always
connect to a server with a dedicated evaluation data directory. Checkpoints and
intermediate responses are intentionally retained for audit. Put exact model,
route, maintenance, and deployment identity in `revision`; scripts cannot infer
or verify an arbitrary provider's remote revision. `base_url` ends at `/v1`.
Credentials are referenced through `api_key_env`, never stored in a plan.

After explicit operator authorization, the full manual sequence is:

```bash
python3 -m benchmarks.morp run target/morp-dataset \
  --plan target/morp-plan.json \
  --out benchmarks/results/run-a \
  --allow-ai
python3 -m benchmarks.morp judge-plan target/morp-dataset \
  --predictions benchmarks/results/run-a/predictions.jsonl \
  --out target/judge-plan.json
python3 -m benchmarks.morp judge --plan target/judge-plan.json \
  --config path/to/judge-a.json \
  --out benchmarks/results/judge-a.jsonl --allow-ai
python3 -m benchmarks.morp judge --plan target/judge-plan.json \
  --config path/to/judge-b.json \
  --out benchmarks/results/judge-b.jsonl --allow-ai
python3 -m benchmarks.morp score target/morp-dataset \
  --plan target/morp-plan.json \
  --predictions benchmarks/results/run-a/predictions.jsonl \
  --votes benchmarks/results/judge-a.jsonl benchmarks/results/judge-b.jsonl \
  --out benchmarks/results/report-a.json
```

Judges must use two independent model/deployment identities. Different
temperatures on the same endpoint, model, and revision are not independent.
Temperature zero does not guarantee byte-identical provider output; repeats
measure variation and must not be cherry-picked. Candidate and judge output is
flushed one case at a time. The same plan resumes unfinished work, while stored
failures are not silently retried. Changed data, prompts, model, parameters, or
deployment require a new plan and result directory.

## Scoring

Objective fields use exact type-aware equality after Unicode NFKC, case, and
outer-whitespace normalization. Substring matches, Boolean-number coercion,
and string-number coercion are rejected. Multi-field cases receive per-field
credit. A nonempty in-character `answer` is still required. Evidence
precision/recall/F1/nDCG is a self-reported diagnostic and is not mixed into
answer quality.

Subjective cases use anchored 0–4 judgements with an exact quote and reason.
Two judges differing by more than one point require human review. Invalid
formats or fabricated quotes remain unscored instead of penalizing the
candidate. One evidence-bound human vote may resolve the case and overrides a
model disagreement.

Aggregation is field → case → scenario family → dimension → equal-weight
dimension macro. Language, horizon, and repeat do not increase a family's
weight. Family-level confidence intervals use 2,000 deterministic bootstrap
samples and remain null with fewer than two families. Missing, invalid, and
error candidates score zero and remain in the denominator. Unjudged subjective
cases remain null. Exact forbidden-string leakage zeros the case and is counted
separately; this check cannot detect every semantic paraphrase.

Use the [real-results template](RESULTS_TEMPLATE.md) to record an actual run.
The template contains no fabricated score.

## Judge calibration

`calibration-plan` contains 16 original positive/negative anchors across eight
families: agency, emotion, state update, knowledge boundary, relationship
boundary, social pressure, evidence revision, and low expression. Quality
labels do not enter judge prompts.

```bash
python3 -m benchmarks.morp calibration-plan \
  --out target/morp-calibration-plan.json
python3 -m benchmarks.morp judge \
  --plan target/morp-calibration-plan.json \
  --config path/to/judge-a.json \
  --out benchmarks/results/calibration-a.jsonl \
  --allow-ai
python3 -m benchmarks.morp calibration-score \
  --votes benchmarks/results/calibration-a.jsonl \
  --out benchmarks/results/calibration-report.json
```

Passing these assistant-authored anchors validates the process, not agreement
with human preferences. Published model rankings still need independent human
annotation, ambiguous counterexamples, cross-family judge agreement, and blind
review.

## External benchmarks and licensing

| Source | Project treatment | Adopted idea |
| --- | --- | --- |
| RPGBench | Apache-2.0 repository; local `games.jsonl` import and initialization proxy | Game state, narrative constraints, user agency |
| EmoCharacter | No independently verified redistributable data license; original emotion scenarios instead | Emotion and cross-situation change |
| DEBATE | Research-only/non-commercial; not imported | Opinion persistence, evidence revision, convergence diagnostics |
| LongMemEval cleaned | MIT data card; local JSON import | Cross-session memory, time, update, unknown-answer behavior |
| LoCoMo | CC-BY-NC-4.0; not imported | Long dialogue, multi-hop links, relationship continuity |
| MemoryAgentBench | MIT data card; local JSONL adapter with provenance | Incremental learning, retrieval, conflict resolution |
| PersonaMem v1 | MIT data card; pinned two-file multiple-choice adapter | Implicit preference, preference change, future-information cutoff |
| BEAM | Data CC-BY-SA-4.0 and code MIT; design reference only | Length scaling, event order, preference following, summarization |

External data is never downloaded automatically, committed, or used to execute
remote installation/model scripts. Place permitted data in ignored
`benchmarks/data/`, pin bytes and upstream revision, and preserve the original
license notice. A local proxy result is not directly comparable to an upstream
paper's official score. Run `python3 -m benchmarks.morp sources` for the
machine-readable source registry.
