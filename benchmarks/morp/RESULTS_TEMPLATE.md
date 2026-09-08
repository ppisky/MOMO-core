# MORP-Bench Real-Run Results

For the paired runtime experiment, record `basic_context` and `all_enabled`
separately, attach both plans/reports and `scenario-compare` output. See
[SCENARIOS.md](SCENARIOS.md). Record probe mean/P50/P95, ingestion and maintenance
barrier durations, observed response cost, unmeasured cost categories and judge
cost separately. Do not fill native total cost from response usage alone.

> Status: **NOT RUN**. This file is a reporting template. Dashes are unfilled
> fields, not zero scores, failures, or measured results.

## Run identity

| Field | Value |
| --- | --- |
| Run name | — |
| Date/time (UTC) | — |
| Operator | — |
| Candidate provider | — |
| Candidate model ID | — |
| Candidate/deployment revision | — |
| Protocol | — (`full-context-replay/1` or `momo-online-sessions/1`) |
| Dataset version | MORP-Bench 0.2 |
| Dataset SHA-256 | — |
| Plan SHA-256 | — |
| Harness SHA-256 | — |
| Scoring-policy SHA-256 | — |
| Candidate config SHA-256 | — |
| Git commit/worktree description | — |
| Result artifact path | — |

## Selection

| Field | Value |
| --- | --- |
| Suite | — (`memory`, `acgn`, or `all`) |
| Split | — (`dev`, `eval`, or `all`) |
| Horizons | — |
| Variants | — |
| Repeats | — |
| Dimensions | — |
| Families | — |
| Characters | — |
| ACGN arms | — |
| Exact case IDs | — |
| Planned cases | — |

## Execution and cost

| Metric | Planned | Actual |
| --- | ---: | ---: |
| Candidate calls | — | — |
| MOMO maintenance calls | — | — |
| Embedding calls | — | — |
| Judge A calls | — | — |
| Judge B calls | — | — |
| Human adjudications | — | — |
| Input tokens | — | — |
| Output tokens | — | — |
| Wall time | — | — |
| Provider cost | — | — |

## Score summary

All reported scores use the 0–100 presentation scale. Copy values from the
generated report; do not replace `null` with zero.

| Metric | Score | Status/notes |
| --- | ---: | --- |
| Coverage | — / 100 | Valid-return rate; not answer quality |
| Objective score | — / 100 | — |
| Selected-dimensions score | — / 100 | — |
| Full eight-dimension macro score | — / 100 | — |
| Leakage violations | — | Exact forbidden-value checks only |
| Publication status | — | Expected to remain provisional until human calibration |

## Dimension results

| Dimension | Families | Cases | Score / 100 | 95% family-bootstrap CI | Evidence/judge status |
| --- | ---: | ---: | ---: | --- | --- |
| Recall | — | — | — | — | — |
| Update | — | — | — | — | — |
| Reasoning | — | — | — | — | — |
| Boundary | — | — | — | — | — |
| World | — | — | — | — | — |
| Persona | — | — | — | — | — |
| Emotion | — | — | — | — | — |
| Social | — | — | — | — | — |

## Lightweight run registry

Only fill a row after the referenced artifacts exist. Planned call counts are
copied from that run's immutable plan, not estimated from this template.

| Profile | Protocol | Candidate revision | Cases | Candidate calls | Objective / 100 | Selected / 100 | Coverage / 100 | Artifact |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| Five-dimension objective smoke | — | — | — | — | — | — | — | — |
| Single-character `label_free` | — | — | — | — | — | — | — | — |
| Single-character three-arm ablation | — | — | — | — | — | — | — | — |
| Custom selection | — | — | — | — | — | — | — | — |

## ACGN ablation

| Comparison | Scored pairs | Mean `label_free - comparison` | 95% CI | Interpretation |
| --- | ---: | ---: | --- | --- |
| `labeled` | — | — | — | Positive favors mechanism-only input |
| `labels_only` | — | — | — | Positive favors mechanism-only input |

## Judge and human-review record

| Field | Judge A | Judge B |
| --- | --- | --- |
| Provider/model | — | — |
| Revision | — | — |
| Config SHA-256 | — | — |
| Calibration coverage | — | — |
| Calibration sanity pass | — | — |
| Invalid votes | — | — |

| Human review metric | Value |
| --- | --- |
| Reviewer protocol/version | — |
| Cases sent to review | — |
| Resolved disagreements | — |
| Unresolved disagreements | — |

## Provider smoke matrix

| Check | Result | Evidence |
| --- | --- | --- |
| Health/model discovery | NOT RUN | — |
| Text response | NOT RUN | — |
| HTTPS image URL | NOT RUN | — |
| Image data URL | NOT RUN | — |
| Streaming completion | NOT RUN | — |
| Cancellation | NOT RUN | — |
| Timeout mapping | NOT RUN | — |
| Truncated stream handling | NOT RUN | — |
| Duplicate delta handling | NOT RUN | — |
| Invalid maintenance Patch rejection | NOT RUN | — |

## Observations and limitations

- Qualitative examples reviewed: —
- Known provider errors: —
- Known benchmark limitations affecting this run: —
- Deviations from the immutable plan: — (a deviation requires a new plan)
- Human-calibration limitations: —

## Sign-off

| Role | Name | Date | Decision |
| --- | --- | --- | --- |
| Operator | — | — | — |
| Reviewer | — | — | — |
| Release owner | — | — | — |
