# MOMO Core Development

MOMO Core is a local-first Rust workspace. Its crates are internal modules of
the same system and may be used directly or through the local HTTP/SSE server.

Read [the 1.0 architecture](architecture_1_0.md) before changing runtime ownership,
application interfaces, mailbox admission or cross-store commit behavior. Rust
callers use typed operations; internal JSON compatibility wrappers have been removed.

The complete local candidate check is `./scripts/test-release.ps1` on Windows
or `bash scripts/test-release.sh` on Linux/macOS. Both use the committed lockfile.
Skipping the security audit must be reported as an incomplete release gate.

## Requirements

- Rust `1.96.1`, pinned by `rust-toolchain.toml`
- a C/C++ build toolchain for native dependencies
- network access on a fresh machine to populate the Cargo cache

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

## Workspace modules

- `momo-core` assembles storage, memory, context, model access, portable data,
  and client-facing APIs.
- `momo-domain` defines shared local domain objects.
- `momo-storage` owns SQLite application persistence and the Turso-backed
  `NsgVectorStore`.
- `momo-memory` implements DMW, NSG, retrieval, lifecycle maintenance, patch
  validation, and MO State.
- `momo-moc` implements verified MOC containers.
- `momo-crypto` implements private-container encryption.
- `momo-config` parses and serializes TOML documents used by portable assets.
- `momo-server` exposes Core over a loopback HTTP/SSE interface.

## Storage layout

Runtime state is intentionally split between `momo.sqlite3` (SQLx/SQLite
application records) and `nsg-vectors.db` (the standalone Turso database used
for NSG vector indexes). Each Space also owns
`spaces/<space_id>/memory-journal.sqlite3`, a SQLite after-image journal committed
before DMW/NSG files are changed. `spaces/<space_id>/memory` is its materialized
checkpoint; `.memory-worker-cache` beside it is disposable private working state.
Only Space worker messages may mutate these workspaces. Every worker start
restores the latest journal snapshot, including snapshots already marked applied;
external file edits are not ingested. Clearing also journals pending SQL/vector
cleanup and blocks that Space until cleanup is acknowledged. Journals and working
copies are excluded from portable MOC modules. See the architecture document for
panic isolation, startup recovery and the current complete-snapshot I/O cost.
Imported CHARX source containers are retained under
`character-packages/<character_id>/source.charx`; they are compatibility
payloads used for asset-preserving CHARX and MOC round trips, not executable
content.

`NsgVectorStore` isolates Turso persistence and exact cosine ranking from the
rest of Core. Its observable contract covers scope identity, vector space,
dimensions, source hashes, and stable ordering. The index is disposable: the
filesystem source documents are authoritative and MOC exports do not include
the Turso database.

`NsgVectorStore` remains the persistence and ranking boundary rather than an
embedding-model provider. Version 0.4.1 adds a separate `EmbeddingProvider`,
an OpenAI-compatible implementation, HTTP batch generation, atomic full or
incremental NSG index rebuilds, and query-text embedding. The implemented
contract and its 0.4.2 protocol alignment are documented in
`vectorization_model_interface_0_4_2.md`.

## Offline performance investigation

On Windows, run `./scripts/profile-core.ps1`. It builds Release examples before
timing, then runs DMW retrieval/lifecycle and vector workloads sequentially.
All data is synthetic or tracked repository text; no model credentials are used.
Timings, hotpath JSON and environment details are saved under `target/`.
The first Windows measurements and their limits are recorded in
[the 2026-09-30 performance report](performance_2026_09_30.zh-CN.md).

The `hotpath` feature on `momo-memory` and `momo-storage` is opt-in. Normal builds
do not include the profiler. An all-features build includes these diagnostics
and is not the normal production profile.

Equivalent commands on other platforms (set `HOTPATH_METRICS_SERVER_OFF=1` to
disable the profiler's local metrics listener):

```bash
HOTPATH_METRICS_SERVER_OFF=1 cargo run --release -p momo-memory --example retrieval_benchmark --features hotpath --locked -- 1000 25 5
HOTPATH_METRICS_SERVER_OFF=1 cargo run --release -p momo-storage --example vector_benchmark --features hotpath --locked -- 5000 384 64 25
```

Function timings include nested work and async waiting; they are not CPU samples
and must not be summed as exclusive time. Whole-run reports include warm-up and
fixture/index construction. The examples separately print repeated warm-query
percentiles. Lifecycle timing covers file-plan preparation/application, not the
full Core/Space-worker journals, provenance eligibility or mailbox contention.
The vector benchmark uses an in-memory cache and exact ranking. Run without
`hotpath` for uninstrumented latency; use full native workloads before making
product SLA claims. SQL/HTTP tracing, lock wrappers, allocation tracing and CPU
sampling are not enabled by this initial integration.

## Runtime data and secrets

API keys, signing certificates, production credentials, user data, and runtime
logs must remain outside Git. Portable configuration and MOC exports reject
credential-like fields rather than preserving them.
