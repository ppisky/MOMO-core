# Dual-Mem Wiki and NSG v2 Implementation

**Updated:** 2026-09-30

This document describes the implemented runtime profile. The normative product
specifications are [`Dual-Mem_Wiki_v2.md`](../Dual-Mem_Wiki_v2.md) and
[`Narrative_Semantic_Graph_v2.md`](../Narrative_Semantic_Graph_v2.md).

## Identity and provenance boundary

The working tree implements [provenance Profile 1](memory_provenance_runtime.md).
Legacy association fields retain their original meaning. Historical evidence,
record grants, qualified state/prompt inputs, scoped scenes and portable revision
history are stored separately. Native results include Space/module/record references
and perspective metadata. Prompt revision is not assistant identity.

## Distillation

- Automatic DMW and NSG maintenance independently consume non-overlapping
  batches of pending completed user/assistant pairs per write Space, default
  12 pairs. This queue is independent of context eviction; removing six old
  turns would be a context policy, not a six-turn summary limit. Current Core
  selects history by token budget rather than a fixed six-turn eviction rule.
  Manual management drains can flush a partial batch. See the
  [code-verified maintenance guide](memory_maintenance_current_behavior.zh-CN.md).
- Every request prepends an immutable YAML Patch contract before optional user
  guidance.
- The current Unix timestamp is injected at runtime. Prompts contain no fixed
  timestamp that a model can copy.
- Unknown operation and frontmatter fields are rejected. `title` is content,
  never an operation field.
- Markdown content must use YAML literal block scalars so quotes, backslashes,
  emoji, and LaTeX cannot create quoted-scalar escape failures.
- Official and custom provider paths allow 120 seconds for structured output.
- A generated patch can be auto-approved, held for review, or rejected by
  policy. Application remains transactional.

## DMW Lifecycle

- Active documents can decay into the archive and can only be restored through
  an explicit user-authorized operation.
- Archived weights continue to decay.
- Native MO State now ages non-core events by completed interaction clocks
  per Space/conversation/character: default decay after 48 unhit turns and
  forgetting after 240 archived/unhit turns in every enrolled context.
  Importance below 0.2, weight below 0.05 and no protected references/tags are
  required for forgetting. Idle time does not count. Forgetting keeps a minimal tombstone in
  `tombstones/forgotten.yaml` and records an audit event.
- Runtime fields such as `touch_at` and `archived_at` are controlled by MOMO,
  not by model patches.
- `mo_state.memory_lifecycle` configures turn intervals, decay factor and
  enable/physical-forgetting switches. Completion stores durable activity;
  prepared file plans recover without double aging. See the
  [implemented lifecycle profile](memory_lifecycle_runtime.md).
- Explicit low-level calendar maintenance remains available for compatibility;
  native automatic/HTTP maintenance no longer uses calendar age. Manual archive
  no longer sweeps unrelated documents.

## NSG semantic web

- `.nsg` files use strict node metadata, semantic tags, and four edge
  categories with a relation whitelist.
- Automatically created nodes start as `draft`.
- Automatic Draft-to-Canon metadata changes become candidates requiring approval.
- Canon mutations become pending revision candidates unless an authorized
  manual operation explicitly changes them.
- Retrieval tokenizes multiword anchors for normalized deterministic matching with optional
  vector reciprocal-rank fusion, then applies importance/ID ordering, one-hop
  expansion, Zone filtering, and a hard token budget.
- Hybrid chat retrieval caps DMW at 60 percent before NSG runs. Any unused DMW
  share remains available to NSG, so a complete graph node is not starved by
  always-loaded current-memory documents.
- Direct DMW hits that exceed their remaining share are cropped at Markdown
  paragraph boundaries instead of being discarded as a whole.
- DMW IDs, aliases, and tags support specific-term matches inside multiword
  values, so a query does not need to repeat an entire generated title.

## MO State

- `momo-memory` keeps the deterministic v1 projector as one component of the
  implemented MO State v2 autonomous runtime.
- The compiler starts with the built-in five-dimension contract and may merge a
  user override from `config/state_contract.yaml`.
- Retrieval embeds the exact DMW metadata snapshot needed by the compiler, so
  MO State performs no second file read and cannot mix two filesystem versions.
- It extracts DMW and NSG signals, resolves explicitly declared rule conflicts,
  emits ordered state directives, enforces a hard state budget, and returns an
  audit object.
- Contract load failures degrade the state context and surface warnings instead
  of failing the entire chat path.
- A structured `current/scene.md` is parsed into a first-class scene snapshot;
  an empty legacy scene template is upgraded without overwriting user content.
- Authoritative DMW, NSG, and scene content receive independent fingerprints
  and monotonic revisions. Retrieval-only `touch_at`, indexes, and audit logs do
  not manufacture source revisions.
- The Core response runtime serializes mutation per Space, journals each state
  event in SQLite, idempotently publishes its snapshot, and atomically marks
  the projected state operation complete with the response replay record.
- In `closed_autonomous`, due DMW/NSG maintenance is recovered before the next
  observation. Completion of an eligible text response schedules a maintenance
  check; scene-aware DMW distillation still waits for its configured batch
  threshold. It is not necessarily a model call on every turn. The external
  tool executor remains owned by the host harness.

## Verification Fixture

`tests/fixtures/dmw-memory-guaranteed.moc` contains a six-message conversation
with explicit relationship, allergy, fear, and promise facts. Regenerate it
with:

```sh
cargo run -p momo_core --example create_memory_fixture -- \
  tests/fixtures/dmw-memory-guaranteed.moc
```

The fixture is intended for repeatable manual distillation checks in addition
to the Rust parser, patch, lifecycle, and retrieval tests.
