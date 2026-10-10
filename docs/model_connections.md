# Local model connections

2026-10-09. This is a local administration profile used by MOMO App. Core owns provider protocol calls and request governance; the HTTP host owns connection persistence, while the application host owns parameter-group names and selection. No model parameters are bound to character cards or conversations.

## Administration

- `GET /v1/model-configuration`: `{revision, providers, models}`. Providers expose `has_api_key`, never the stored key. Models expose immutable `configured_<sha256>` references.
- `PUT /v1/model-configuration`: complete replacement with the expected `revision`. Stale revisions return 409. Provider fields: UUID `id`, `name`, `protocol` (`openai`, `openai_responses`, `anthropic`), `base_url`, optional `api_path`, optional `api_key`. Omitting the key preserves it; an empty string clears the current key. Model fields: UUID `id`/`provider_id`, `model_id`, `name`, `kind` (`chat`, `vision`, `embedding`), `context_window`, nullable positive `max_output_tokens`.
- `GET /v1/model-configuration/providers/:id/models`: authenticated discovery through the stored provider, bounded to 2 MiB/1024 entries; returns `{models:[{model_id,name}],has_more}`. It never automatically adds unknown models or infers their category. Further pages can be entered manually.

The base URL includes the API version prefix. A custom path is appended to it. HTTP(S) endpoints reject URL credentials, queries and fragments; redirects are disabled. Limits: 64 providers, 1024 models, 4096 retained connection snapshots and 32 MiB of persisted configuration. Updates write/sync a temporary file, atomically replace `model-connections.json`, then publish the new state. This is an HTTP-host file owned by `momo-server`, not an input or resource of `MomoRuntime`. The default location is `$MOMO_DATA_DIR/model-connections.json`; `MOMO_MODEL_CONNECTIONS_PATH` selects another private host path. It contains credentials, including frozen credentials needed by older request references. Removing a current provider does not erase snapshots used for retries. Do not export it as public settings or include it in logs/source control.

## Response routing

2026-10-10: Core-owned purpose defaults, validated overrides and reset semantics are implemented in [purpose request configuration](model_purpose_request_configuration.md). `POST /v1/momo/model-requests/resolve` resolves configured models without calling a provider or writing state. Hosts save only custom deltas; no model-request Space is introduced.

`model` may be an immutable configured reference. An unknown reference or embedding model fails before response admission writes user content. Old gateway aliases remain supported. Configured endpoints receive their actual provider model ID; internal `momo_*` gateway metadata is removed.

An optional `momo.model_routes` map extends the response request. Allowed keys are `vision`, `memory_distillation`, and `semantic_graph_governance`. Each entry contains `model` and nullable `temperature`, `top_p`, `max_output_tokens`, plus optional extra `parameters`. “Follow conversation” resolves only the model: conversation sampling, output, stream and extra parameters are not copied. Core overlays each purpose's custom deltas onto its recommended defaults, subject to the originating Space policy and model budget. Omitted fields inherit purpose defaults. Route fields participate in the request fingerprint; empty maps remain omitted.

Core applies the Space's request-override policy to proposals. DDM uses the memory-distillation route. Configured vision routes require vision-kind definitions before admission, including ordinary text turns; the vision route may be omitted. Legacy gateway aliases still undergo capability checks when fallback is needed. Response provenance captures route references without credentials. A maintenance batch uses the last participating turn's captured configuration, so another window's selected conversation cannot redirect it. Older evidence without route metadata retains legacy aliases. Model routes do not change memory ownership, injection or automatic/manual provenance.

Embedding definitions are rejected from generation. Registration alone does not change an index; explicit `momo.embedding` selection connects a definition to the existing dedicated profile/index contract described below.

2026-10-09 setting correction: `context_window` may be omitted or null to retain an undeclared capability; zero is rejected. Internal budgeting uses a protective estimate rather than storing that estimate as a declaration. Route entries also accept optional `parameters`, governed by the originating Space's allowed-parameter policy. Empty parameter maps are omitted from serialization so existing request fingerprints remain stable. See [optional model context](optional_model_context.md).

## Protocol coverage and verification

OpenAI Chat Completions reuses the bounded Core gateway. Responses and Anthropic Messages convert text/system/image/function messages and normalize completion/usage/stream text. Native streams require a successful terminal event; mismatch, truncation, upstream errors, argument/stream limits and cancellation fail explicitly. Function-call deltas for these two protocols are emitted after the provider completes the call, while text remains incremental. Provider-specific reasoning/signature extensions are not a general compatibility guarantee.

Specification references: [OpenAI Responses events](https://developers.openai.com/api/reference/resources/responses/streaming-events), [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming).

Offline tests cover all four request/stream conversions, secret redaction, CAS, persistence, frozen references and function payloads. Real local HTTP integration from MOMO App covers configuration, model discovery, selected model/sampling/output, default-assistant response and idempotent replay against a fixture provider. These are not paid provider tests or ron desktop acceptance.

## Configured NSG embeddings (2026-10-10)

`momo.embedding` is an optional, separate selection: `{ "model": "configured_…" }` follows the embedding model's native default dimension; an optional positive `dimension` requests the provider's `dimensions`. `{ "model": null }` disables embeddings. Omission of the whole selection retains the gateway contract for independent Core users. Embeddings always use the sibling `/embeddings` endpoint, including when the generation provider is Responses or its base URL explicitly ends in `/responses`.

The host supplies credentials. Core checks reference, embedding kind and protocol before admitting a response. `embedding.parameters` optionally accepts `dimensions` and `encoding_format` (`float` only); both dimension declarations must agree. Existing normalization/prefix processing remains Core-owned, never forwarded as provider fields. Native dimension is discovered from a real vector, validated throughout each batch, and included in the resolved vector-space identity. Native-default indexing checks one document to resolve the dimension before reusing other unchanged compatible vectors. Failed batches preserve prior indexes and graph sources. No generation sampling/output/stream parameters enter an embedding request.

`openai` is the default protocol. An explicit `api_path` ending in `/responses`, or a base URL ending in `/responses` with no custom path, selects the Responses adapter within an OpenAI-compatible connection. `openai_responses` explicitly selects it too. A complete generation endpoint is not appended again; model discovery and embeddings use sibling endpoints. There is no silent protocol switch after a failed generation. The native Google/Gemini adapter has been removed; Google-hosted models exposing these OpenAI protocols may still connect through them.

验证（2026-10-10）：Rust 1.99.0 的 Core 217 项、server 30 项单元测试通过；受影响库的严格 Clippy、格式和 locked server 构建通过。真实 Web HTTP 与隔离 Core/离线模型验证首次索引、增量复用、查询调用、错误类型、维数不匹配保留旧索引/源资产、关闭后不调用向量接口；取消能中断正在等待的向量请求。浏览器显示降级提示。没有验证付费服务商或 ron 桌面。

## Ownership correction (2026-10-09)

The first implementation loaded account storage from `MomoRuntime`; that contradicted the existing [domain/configuration boundary](domain_and_configuration_boundaries.zh-CN.md). Account persistence, revisions, redaction and discovery now live in `momo-server`. It injects a shared `ModelConnectionResolver` through `MomoApiService::with_model_connections`. Embedding users may supply their own resolver/storage, or keep `MomoApiService::new` and the existing gateway without any account file. Protocol validation remains in Core. An unresolved `configured_` reference fails rather than falling back to a different model.

Default-assistant identity/mapping, purpose aliases, request override policies, Prompt Spaces, memory/NSG/DDM orchestration were already Core capabilities. The preceding feature commit added atomic default creation and host-selected connection/parameter transport, not replacements for those definitions. The earlier Space lifecycle work added asset disposition/recovery transactions, not another memory implementation.

Verification of this ownership correction: Rust 1.99.0, `cargo test --locked -p momo_core -p momo-server --lib` (214 Core / 30 server tests), targeted all-target Clippy with `-D warnings`, locked server build and formatting passed. New tests prove embedded initialization ignores a malformed host file, resolver injection is service-scoped and validates its result, and an external host directory retains removed-model references after reopen. MOMO App's real HTTP fixture also passed with `MOMO_MODEL_CONNECTIONS_PATH` outside the Core data directory, including discovery, default-assistant streaming, parameters and replay. No paid provider or additional desktop acceptance is implied.
