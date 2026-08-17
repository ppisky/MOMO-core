# MOMO Core

[简体中文](README.zh-CN.md)

MOMO Core is a local-first Rust foundation for AI character experiences. It
provides character data, conversations, long-term memory, narrative semantics,
state compilation, portable containers, encryption, model gateways, and a
local HTTP interface in one workspace.

## Capabilities

- independent MOMO Character Card v2 (`character.toml` + Markdown)
- Character Card v1/v2 JSON and PNG import
- Character Card v3 JSON, PNG/APNG, and CHARX import
- Character Card v2/v3 JSON export with source-field preservation
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

## Character-card format boundary

MOMO Character Card v2 is an independent format defined by this repository in
[`Character_Card_v2.md`](Character_Card_v2.md). Its “v2” does not mean the
external `chara_card_v2` JSON/PNG format. Core currently imports and exports the
MOMO format inside MOC v2. It also imports external CCv1/v2 JSON and PNG plus
CCv3 JSON, PNG/APNG, and CHARX, and exports CCv2/CCv3 JSON. Unsupported external
fields are retained as source metadata and survive MOC round trips.

Compatibility design for external formats is based specifically on
[Character Card v2](https://github.com/malfoyslastname/character-card-spec-v2)
and [Character Card v3](https://github.com/kwaroran/character-card-spec-v3).
See [character-card formats and compatibility](docs/character_card_compatibility.md)
for pinned sources, terminology, and implementation status.

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
exact-ranking implementation used by the test suite.

## Scope identity

`scope_id` is the only namespace identifier used by public models, APIs,
storage, vector records, patch reviews, and MOC operations. A scope is an
opaque UUID whose meaning and access policy belong to the host application.
Core stores each memory workspace under `memory/scopes/<scope_id>`.

## Validate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

See the [development guide](docs/development.en.md) and
[character-card compatibility profile](docs/character_card_compatibility.md).

## Contributing

Issues are open for reproducible problems and concrete proposals. Pull requests
are also open; substantial changes should preferably begin with an Issue. Read
[CONTRIBUTING.md](CONTRIBUTING.md) before participating.

## License

Apache License 2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE), and
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
