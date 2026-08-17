# MOMO Core Development

MOMO Core is a local-first Rust workspace. Its crates are internal modules of
the same system and may be used directly or through the local HTTP/SSE server.

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
- `momo-config` parses and serializes portable runtime configuration.
- `momo-server` exposes Core over a loopback HTTP/SSE interface.

## Storage layout

Runtime state is intentionally split between `momo.sqlite3` (SQLx/SQLite
application records) and `nsg-vectors.db` (the standalone Turso database used
for NSG vector indexes). DMW and NSG source documents remain in the filesystem.

`NsgVectorStore` isolates Turso persistence and exact cosine ranking from the
rest of Core. Its observable contract covers scope identity, vector space,
dimensions, source hashes, and stable ordering. The index is disposable: the
filesystem source documents are authoritative and MOC exports do not include
the Turso database.

## Runtime data and secrets

API keys, signing certificates, production credentials, user data, and runtime
logs must remain outside Git. Portable configuration and MOC exports reject
credential-like fields rather than preserving them.
