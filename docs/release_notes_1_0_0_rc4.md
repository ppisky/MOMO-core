# MOMO Core 1.0.0-rc.4

`v1.0.0-rc.4` is a runtime-boundary and retrieval-performance candidate. It
keeps the frozen `momo.responses/1.0` wire, Character Card v2, DMW v2, NSG v2,
MO State v2, and MOC v3 formats unchanged.

## Highlights

- Splits the response orchestrator into explicit execution, response,
  generation, state, and maintenance modules without changing the native
  response contract.
- Hardens memory, semantic-graph, vector, portable-import, and response replay
  boundaries found during the post-rc.3 architecture review.
- Makes `MomoCore` own a bounded, process-lifetime registry of per-Space
  `MemoryWorkspace` instances. Repeated API requests now reuse the same
  validated workspace and derived-index cache instead of reconstructing both
  for every request.
- Keeps the filesystem source of truth observable: cached indexes carry source
  path, size, and modification stamps and rebuild when an external edit or
  corrupt index is detected.
- Moves DMW/NSG directory traversal, parsing, retrieval, and MO State source
  fingerprinting onto Tokio's blocking pool so local filesystem latency does
  not occupy asynchronous request workers.
- Computes normalized query terms once per retrieval and avoids reparsing the
  complete persisted index when refreshing a selected document's activity
  timestamp.
- Repairs the synthetic retrieval benchmark so it queries the fixture's unique
  structured tag rather than a generic word shared by every document.

## Performance evidence

On the same Windows development host, release-mode repeated DMW retrieval with
activity writes changed as follows:

| Documents | rc.3-path p50 | rc.4-candidate p50 | Reduction |
| ---: | ---: | ---: | ---: |
| 5,000 | 698,706 us | 111,157 us | 84.1% |
| 10,000 | 1,504,813 us | 219,241 us | 85.4% |

These are deterministic local microbenchmarks, not a network-service SLA. The
workspace registry makes the measured warm-workspace lifetime match repeated
requests through one running Core process. Cold startup and external file
changes still validate or rebuild indexes by design.

## Compatibility and limits

- No frozen HTTP fixture, portable schema, migration, or database schema
  changes in this candidate.
- Workspace crates remain unpublished implementation modules; the change from
  an owned `MemoryWorkspace` result to `Arc<MemoryWorkspace>` is internal to the
  pinned workspace revision.
- The process-global JSON compatibility facade remains available. Moving all
  orchestration internals to typed injected services is a later refactor, not
  hidden inside this performance candidate.

## Verification

The tagged candidate passed the repository's full local validation surface on
2026-09-21: 273 Rust tests, 102 Python tests, and 21/21 offline runtime contract
probes. It was subsequently merged and published as `v1.0.0-rc.4`. Any
post-tag stabilization change must pass the complete Linux, Windows, and
security GitHub CI matrix again before another release:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `python -m unittest discover -s benchmarks/morp/tests -q`
- `cargo doc --workspace --all-features --no-deps`
- `cargo build --release --workspace`
