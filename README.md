# MOMO Core 0.3

[简体中文](README.zh-CN.md)

MOMO Core is a local-first Rust foundation for AI character experiences. It
provides character data, conversations, long-term memory, narrative semantics,
state compilation, portable containers, encryption, model gateways, and a
local HTTP interface in one workspace.

## Capabilities

- Character Card v2
- MOC v2 import and export
- Dual-Mem Wiki (DMW) long-term memory
- Narrative Semantic Graph (NSG)
- MO State compilation
- local SQLite persistence
- OpenAI-compatible completion and streaming
- capability discovery and context budgeting
- vector-store contracts and deterministic retrieval

The crates under `crates/` are implementation modules of MOMO Core. They are
not separate products. `momo-server` exposes the same Core capabilities over a
loopback HTTP/SSE interface for local applications.

## Workspace

- `momo-core`: orchestration and client-facing Rust APIs
- `momo-domain`: shared domain types
- `momo-storage`: local persistence and vector-store contracts
- `momo-memory`: DMW, NSG, retrieval, and MO State
- `momo-moc`: MOC containers
- `momo-crypto`: encrypted private containers
- `momo-config`: portable runtime configuration
- `momo-server`: local HTTP/SSE interface

## Vector storage

`NsgVectorStore` defines the vector-storage boundary. In MOMO documentation,
Turso always means the standalone database library selected for the vector
database backend. The current source tree also contains a deterministic local
exact-ranking implementation used by the 0.3 test suite.

## Validate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

See [Core 0.3 status](docs/core_progress_0_3.md) and the
[development guide](docs/development.en.md).

## Contributing

Issues are open for reproducible problems and concrete proposals. Pull requests
are temporarily paused. Read [CONTRIBUTING.md](CONTRIBUTING.md) before reporting
an issue.

## License

Apache License 2.0. See [LICENSE](LICENSE) and
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
