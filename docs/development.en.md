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
- `momo-storage` owns SQLite persistence and `NsgVectorStore`.
- `momo-memory` implements DMW, NSG, retrieval, lifecycle maintenance, patch
  validation, and MO State.
- `momo-moc` implements verified MOC containers.
- `momo-crypto` implements private-container encryption.
- `momo-config` parses and serializes portable runtime configuration.
- `momo-server` exposes Core over a loopback HTTP/SSE interface.

## Vector storage

`NsgVectorStore` isolates vector persistence and ranking from the rest of Core.
Turso means the standalone database library selected for the vector database
backend.

The local exact-ranking implementation defines deterministic validation
semantics for owner scope, vector space, dimensions, source hashes, and stable
ordering. Another backend must preserve those observable semantics.

## Runtime data and secrets

API keys, signing certificates, production credentials, user data, and runtime
logs must remain outside Git. Portable configuration and MOC exports reject
credential-like fields rather than preserving them.
