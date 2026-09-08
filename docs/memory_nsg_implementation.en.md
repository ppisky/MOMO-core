# Dual-Mem Wiki and NSG v2 Implementation

**Updated:** 2026-09-05

This document describes the implemented runtime profile. The normative product
specifications are [`Dual-Mem_Wiki_v2.md`](../Dual-Mem_Wiki_v2.md) and
[`Narrative_Semantic_Graph_v2.md`](../Narrative_Semantic_Graph_v2.md).

## Distillation

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
- Only low-value event memories that have remained unreferenced for 180 days
  can be forgotten. Forgetting keeps a minimal tombstone in
  `tombstones/forgotten.yaml` and records an audit event.
- Runtime fields such as `touch_at` and `archived_at` are controlled by MOMO,
  not by model patches.

## NSG semantic web

- `.nsg` files use strict node metadata, semantic tags, and four edge
  categories with a relation whitelist.
- Automatically created nodes start as `draft`.
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
  observation and scene-aware DMW distillation is scheduled after each text
  turn. The external tool executor remains owned by the host harness.

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
