# Dual-Mem Wiki and NSG v1 Implementation

**Updated:** 2026-08-10

This document describes the implemented runtime contract. The normative product
specifications remain `Dual-Mem_Wiki_v1.md` and
`Narrative_Semantic_Graph_v1.md`.

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
- Retrieval combines normalized deterministic anchor matching with optional
  vector reciprocal-rank fusion, then applies importance/ID ordering, one-hop
  expansion, Zone filtering, and a hard token budget.
- Chat retrieval allocates 75 percent of the memory budget to DMW and gives the
  remaining budget to NSG.

## MO State

- `momo-memory` implements a deterministic MO State v1 compiler.
- The compiler starts with the built-in five-dimension contract and may merge a
  user override from `config/state_contract.yaml`.
- It extracts DMW and NSG signals, emits ordered state directives, trims lower
  priority dimensions to fit the state budget, and returns an audit object.
- Contract load failures degrade the state context and surface warnings instead
  of failing the entire chat path.

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
