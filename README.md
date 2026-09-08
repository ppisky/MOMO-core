# MOMO Core

[简体中文](README.zh-CN.md)

MOMO Core is a local-first Rust foundation for AI character experiences. It
provides character data, conversations, long-term memory, narrative semantics,
an autonomous state runtime, portable containers, encryption, model gateways, and a
local HTTP interface in one workspace.

## Capabilities

- independent MOMO Character Card v2 (`character.toml` + Markdown)
- Character Card v1/v2 JSON and PNG import
- Character Card v3 JSON, PNG, and full CHARX-container import; APNG is rejected
- Character Card v2/v3 JSON and CHARX export with source preservation
- typed multi-Space MOC v3 import/export with explicit one-to-one Space mapping
- typed MOMO LSB carriers for PNG and lossless WebP (no APNG or AVIF)
- Dual-Mem Wiki (DMW) long-term memory
- Narrative Semantic Graph (NSG)
- MO State v2 autonomous runtime with per-Space DMW/NSG, scene, revision, and snapshot management
- SQLite application storage and a separate Turso vector store
- native `MomoApi` orchestration plus outbound model-adapter interfaces
- governed image input through an optional visual-description adapter
- capability discovery and context budgeting
- vector-store contracts and deterministic retrieval

The crates under `crates/` are implementation modules of MOMO Core. They are
not separate products or independently published crates.io packages. The 1.0
product stability surface is the versioned native HTTP wire
(`momo.responses/1.0` and `momo.control/1.0`) plus documented portable formats;
the remaining `/v1` routes are a local administration profile.
Embedders may still use the Rust API from the same pinned workspace revision,
without an independent crates.io SemVer promise. See the
[HTTP boundary](docs/http_api_1_0.md) and [specification index](docs/spec_index.md).
`momo-server` exposes the same Core capabilities over a loopback HTTP/SSE
interface for local applications.

## Character-card format boundary

MOMO Character Card v2 is an independent format defined by this repository in
[`Character_Card_v2.md`](Character_Card_v2.md). Its “v2” does not mean the
external `chara_card_v2` JSON/PNG format. Core currently imports and exports the
MOMO format inside MOC v3. It also imports external CCv1/v2 JSON and PNG plus
CCv3 JSON, PNG, and CHARX, and exports CCv2/CCv3 JSON and CHARX. APNG is
intentionally unsupported. CHARX
assets, `x_meta`, `module.risum`, and unknown safe entries are retained with the
source container and survive MOC round trips.

Compatibility design for external formats is based specifically on
[Character Card v2](https://github.com/malfoyslastname/character-card-spec-v2/blob/8083fb388615ccbce768e97cbbd49d2b3214632c/spec_v2.md)
and [Character Card v3](https://github.com/kwaroran/character-card-spec-v3/blob/f3a86af019fbd99f788f7a1155f399655b34ab35/SPEC_V3.md).
Risu-specific extensions are identified from the
[RisuAI reference implementation](https://github.com/kwaroran/Risuai/blob/c0ed1026de4b06a1c4600b79c789fea0616c297c/src/ts/characterCards.ts), with the
[Character Foundry CHARX documentation](https://github.com/character-foundry/character-foundry/blob/322fe8d940d1b91c978b43330b80ab2e115002e4/docs/charx.md)
used only as a downstream cross-check. All external specification documents are
linked at pinned revisions rather than redistributed. See [character-card formats and compatibility](docs/character_card_compatibility.md)
for source precedence, licensing boundaries, terminology, and implementation status.
See [experience-grounded character design](docs/character_design_mechanism.en.md)
for the authoring model used by the original MORP-Bench cast.
The experimental [Dynamic Disposition Model](DYNAMIC_DISPOSITION_MODEL_v1.md)
defines how those stable tendencies could be modulated by current context and
MO State without creating another memory store.

## Workspace

- `momo-core`: orchestration and client-facing Rust APIs
- `momo-domain`: shared domain types
- `momo-storage`: SQLite application persistence and Turso vector storage
- `momo-memory`: DMW, NSG, retrieval, scene parsing, and MO State projection
- `momo-moc`: MOC containers
- `momo-crypto`: encrypted private containers
- `momo-config`: portable runtime configuration
- `momo-server`: local HTTP/SSE interface

## Data storage

Core deliberately uses two independent databases instead of putting all data
in one SQLite file:

- `momo.sqlite3`, managed through SQLx/SQLite, stores application data such as
  characters, conversations, messages, deletion state, patch reviews, and
  portable metadata;
- `nsg-vectors.db`, managed by the official `turso` Rust library, stores only
  the NSG vector index.
- `character-packages/<character_id>/source.charx` retains an imported CHARX
  source container so binary assets and application extensions can round-trip
  through MOC.

DMW and NSG YAML/Markdown documents under `spaces/<space_id>/memory` remain the
portable sources of truth. Turso vectors are validated by source hash and
vector-space identity, can be rebuilt from those documents, and are not stored
in MOC files. `NsgVectorStore` is only the internal boundary that keeps Turso
details out of the rest of Core; it is not a third database. When upgrading to
0.3.2, the legacy SQLite `nsg_vectors` table is removed and the host should
rebuild the vector cache when needed.

Version 0.4.2 aligns the outbound embeddings adapter with the documented OpenAI
request/response metadata, retains token usage, and returns useful 400/502/504
status codes from the local generation endpoint. That local endpoint remains a
MOMO orchestration API, not a drop-in OpenAI server. Raw-vector APIs remain a
low-level compatibility path. See the
[embedding interface profile](docs/vectorization_model_interface_0_4_2.md).

Version 0.5.0 includes the native `POST /v1/momo/responses` wire contract,
true upstream-to-client SSE deltas, persistent request-ID replay, Core-owned
embedding profiles and background DMW/NSG maintenance. Function tools have a
shared cross-protocol golden contract, and the codec-independent
[MOMO LSB Carrier v1](docs/momo_lsb_carrier_v1.md) includes bounded PNG codec
integration for PNG and lossless WebP. The 0.5.0 release
release also unifies error envelopes and request/stream bounds, adds
cancellation, timeout, rate-limit and logical-route metrics. See the
[0.5.0 roadmap](docs/roadmap_0_5_0.md) and
[migration guide](docs/migration_0_4_2_to_0_5_0.md) for compatibility details
and the 1.0 stability gates.

The local 1.0 candidate includes the Core image-input path: up to eight user
images go directly to an image-capable conversation model, or through the
optional logical `vision` description route when the conversation model is
text-only. Resolved input and usage are persisted for deterministic request-ID
replay. The `momo.responses/1.0` cross-repository fixtures are frozen, but this
is published as the `v1.0.0-rc.1` prerelease; broader credentialed and MORP
coverage remains the stable `v1.0.0` gate. See the
[1.0.0 release contract](docs/roadmap_1_0_0.md).

Background maintenance prompts are portable Markdown files rather than
abbreviated inline TOML. Start from [momo.example.toml](momo.example.toml) and
read the [English configuration guide](docs/maintenance_prompts.en.md) or the
[简体中文指南](docs/maintenance_prompts.zh-CN.md).

## Space identity

One Core instance root contains many independent Spaces; it is not itself one
person's Space. Public contracts use role-qualified UUIDs:
`personal_space_id`, `conversation_space_id`, memory-source `space_id`, and
`memory_write_space_id`. Retrieval may read several authorized Spaces with
independent weights, while one maintenance run writes to exactly one explicit
Space. Character cards use a globally unique `character_id`; `owner_space_id`
is only their management/export ownership, and there is no character-catalogue
UUID. See the [normative Space model](docs/space_model_1_0.md) and the
[host guide](docs/spaces_and_controls.en.md).

## Validate

[MORP-Bench](benchmarks/morp/README.en.md) adds original memory scenarios, ACGN
label-ablation experiments, reproducible scoring and source-license notes.
Run `bash scripts/test-morp.sh`; Windows PowerShell also has `./scripts/test-morp.ps1`.
The default workflow is offline; candidate and judge inference require an
explicit `--allow-ai` and are not part of CI.

Generate a model plan without contacting a provider:

```bash
bash scripts/run-morp-model.sh \
  --config benchmarks/morp/configs/baseline.example.json
```

The runner supports lightweight selection by character, dimension, scenario
family, ACGN arm, or exact case ID. Reports expose explicit 0–100 objective and
selected-dimension summaries without converting pending subjective judgements
into zero. Record a credentialed run in the
[real-results template](benchmarks/morp/RESULTS_TEMPLATE.md).
The current non-normative engineering assessment is available in the
[role-playing implementation review](docs/roleplay_implementation_review.en.md).

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

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
