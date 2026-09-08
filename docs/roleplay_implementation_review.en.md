# MOMO Core Role-Playing Implementation Review

[简体中文](roleplay_implementation_review.zh-CN.md)

**Review date:** 2026-09-05

**Scope:** current worktree, including uncommitted MO State v2, control-protocol,
and maintenance-batch changes

**Nature:** non-normative engineering review; precedence remains defined by
[`spec_index.md`](spec_index.md)

## Conclusion

MOMO Core is already a solid role-playing runtime foundation, but the existing
tests alone do not prove that it is a mature, high-quality role-playing product.
Its strongest areas are asset compatibility, security boundaries,
deterministic memory retrieval, idempotent persistence, and Canon protection.
Its weaker areas are a complete tool loop, cross-Space state consistency,
runtime application of opening assets, and automated evaluation of actual
character behavior.

The overall engineering score is **7.9 / 10**. This measures implementation
completeness, correctness, maintainability, and documentation alignment. It is
not a score for a model's prose or role-playing ability.

Scale: 9–10 means clear boundaries with end-to-end verification; 7–8 means the
main path is reliable with explicit gaps; 5–6 means a usable baseline exists but
the documented loop is incomplete; below 5 means the area is still primarily an
interface or design.

## Module scores

| Module | Score | Assessment |
| --- | ---: | --- |
| Character Card core and external compatibility | 8.7 | MOMO v2 and CCv1/v2/v3 JSON, PNG, and CHARX recognition, limits, source preservation, and reverse-export boundaries are clear. Paths, BOM, frontmatter, APNG, archives, and unknown fields are tested. The administration API still lacks a convenient request that creates complete `opening_markdown` and author-URL content in one operation. |
| Conversations, messages, and structured controls | 8.4 | Space ownership, character switching, deletion/clearing, idempotency conflicts, and atomic response completion are strong. Message storage is still a text-only three-role model and cannot natively preserve image blocks or complete tool-call history. |
| Prompt and context assembly | 7.7 | Character, user, DMW, MO State, NSG, and history have an explicit order and hard budget while preserving the newest user turn. Governed runtime instructions no longer behave like trimmable history, and multi-Space memory retains provenance. At the original review point, all system sections were still trimmed as one unit without per-section loss audit; later progress below addresses this. |
| DMW long-term memory | 8.8 | Retrieval, direct hits, one-hop expansion, budget isolation, `touch_at` versus injection time, decay, archive, forgetting tombstones, transaction rollback, and Patch permissions have strong deterministic implementations. Real long-dialogue false-write, missed-write, and recall-quality measurements are still absent. |
| NSG narrative semantic graph | 8.6 | Draft isolation, Canon revision candidates, anchor/vector fusion, Auto-Zone, one-hop expansion, edge allowlists, and candidate deduplication are implemented and tested. Semantic quality still depends on the Governor model, with no precision/recall evidence on a conflict corpus. |
| MO State v2 | 6.7 | Per-Space locks, source fingerprints, revisions, Operation Journal, Snapshot, Scene parsing, and recovery maintenance form a credible baseline. Independent multi-source versions, a shared DMW/NSG Saga, a truly bounded Reconciler, soft-timeout execution, and durable `run_id` pause/resume remain incomplete. |
| Multimodal role playing | 7.6 | Capability discovery selects original-image passthrough or a governed description fallback, with count/size limits, persisted fallback usage, and retry-safe vision resolution. Direct images are visible only during the current turn; later history retains only an image-count marker, so long-term visual continuity still needs host or memory support. |
| Tool calls and agent loop | 5.9 | Gateway tool deltas, call IDs, and input/output mappings work. Same-conversation and exact-pair validation now prevent tool JSON from masquerading as user history. The 1.0 wire still lacks `requires_action`, durable pause points, same-run resumption, enforced step/time limits, and complete typed tool history. |
| Configuration governance and capability discovery | 8.1 | Allow/ignore/reject behavior, parameter allowlists, capability caps, vision fallback, and prompt-file boundaries are clear. Host fields that Core did not execute were removed from the official example, and portable preservation is explicitly distinct from execution. |
| Portability, security, and recovery | 9.0 | MOC v3, encryption, LSB, path-traversal protection, single-instance locking, atomic file writes, persistent idempotency, and source preservation are the project's most mature areas. |
| Testing and observability | 7.8 | Workspace tests cover many failure paths, boundaries, and mock end-to-end flows, with useful state/request audit data. Real-provider smoke tests, a fault-injection matrix, and actual role-playing behavior runs are still missing. |

## Documentation/code alignment

| Surface | Alignment | Review result |
| --- | ---: | --- |
| Character Card v2 | High | Physical format, field boundaries, security validation, and compatible imports are substantially aligned. `opening.md` has a clear format role; runtime documentation now states that Core does not automatically insert it into a response. |
| DMW v2 | High | Major constants and retrieval/lifecycle rules match the implementation. The specification mainly exceeds tests in qualitative SHOULD requirements and asynchronous policy, not contradictory behavior. |
| NSG v2 | High | Canon/Draft, retrieval, Zone, edge, and vector-cache boundaries match the code. Model-governance quality still lacks dataset-level evidence. |
| MO State v2 | Medium | The document describes the complete target architecture while the code implements a baseline. The implementation-status section now explicitly lists tool resumption, multi-source versions, and cross-system Saga behavior as unimplemented. |
| Native response runtime | Medium-high | Idempotency, persistence, streaming, vision, and maintenance order align with the implementation. Message roles, instruction precedence, card hot updates, openings, image history, and tool-continuation semantics are now documented. |
| Space model | High | Ownership and access responsibilities are clear. Documentation now matches the implementation: a Space weight divides the Space budget, while ranking occurs independently inside each Space. |
| Portable runtime configuration | Medium-high | Core executes governance, maintenance, MO State, vision, and prompt fields. Host-owned routing, default-character, weight, and concurrency fields were removed from `momo.example.toml`; safe unknown fields are preserved but not executed. |

## Corrections made during the review

1. Governed top-level `instructions` enter a protected `# Runtime Instructions`
   system section instead of silently disappearing during history trimming.
2. DMW/NSG content preserves its memory-Space label and ID in final RP context.
3. Autonomous MO State compilation, source observation, and Journal operations
   use the same managed Space.
4. Structured `message` input accepts only the `user` role; system instructions
   pass through governance, and clients cannot forge Core-owned history.
5. Tool continuations require an existing conversation and exact one-to-one
   `call_id` pairing; tool-only input no longer becomes a fake user message.
6. Character CRUD validates name, SemVer, author, URL, text size, and
   frontmatter consistently with the card format. The silently ignored HTTP
   `description` field was removed.
7. Runtime, Space, MO State, and example-configuration documents now state the
   actual execution boundaries.

## Remaining priorities

Later progress on 2026-09-05 added [MORP-Bench](../benchmarks/morp/README.en.md),
including original memory scenarios, label-free ACGN comparisons, offline Rust
contract replay, explicitly enabled candidate/judge execution, hashes and
checkpoints, paired scoring, and license filtering. Context now uses
per-section budgets with section-level trimming audit. This addresses the
original “whole-system-section trimming” and “no behavior-evaluation workflow”
findings. Model evaluation has still not been run; human calibration,
real-provider smoke testing, and the cross-system gaps below remain. The score
above is preserved as a point-in-time review and is not raised merely because
new evaluation scripts exist.

### P1: directly affects long-dialogue reliability

- Record independent DMW/NSG/Scene revisions for every read source in a
  multi-Space MO State Snapshot, not only the managed Space.
- Design next-generation typed conversation-event storage for tool calls,
  outputs, and authorized image descriptions. Until then, do not claim a fully
  resumable agent run.
- Run the role-playing behavior evaluation: constraint adherence, consistency
  over 50/100/500 events, DMW false/missed writes, NSG Canon conflicts, scene
  transitions, and private-memory leakage.
- Give an event affecting both DMW and NSG one shared Saga/Outbox operation
  identity and fault-inject between the two persistence stages.

### P2: product experience and diagnostics

- Add an explicit host operation to create a conversation and apply its
  opening, including template substitution, role, and idempotency semantics.
- Provide an optional character-revision pin. Card updates currently affect all
  later turns of bound conversations immediately.
- Run credentialed provider release smoke tests plus cancellation, timeout,
  truncated-stream, duplicate-delta, and invalid-maintenance-Patch faults.

## Verification record

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace --all-features`: passed, 203 tests.
- `cargo doc --workspace --all-features --no-deps`: passed.
- `scripts/test-morp.sh`: passed, 58 harness tests, 387 offline points,
  21/21 runtime contract checks, and zero AI calls.
- `cargo audit`: passed; 476 `Cargo.lock` dependencies scanned with no known
  vulnerabilities reported.
- `git diff --check`: passed with only existing CRLF/LF conversion warnings.
