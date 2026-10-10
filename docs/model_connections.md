# Local model connections

2026-10-09. This is a local administration profile used by MOMO App. Core owns provider protocol calls and request governance; the HTTP host owns connection persistence, while the application host owns parameter-group names and selection. No model parameters are bound to character cards or conversations.

## Administration

- `GET /v1/model-configuration`: `{revision, providers, models}`. Providers expose `has_api_key`, never the stored key. Models expose immutable `configured_<sha256>` references.
- `PUT /v1/model-configuration`: complete replacement with the expected `revision`. Stale revisions return 409. Provider fields: UUID `id`, `name`, `protocol` (`openai`, `openai_responses`, `anthropic`, `gemini`), `base_url`, optional `api_path`, optional `api_key`. Omitting the key preserves it; an empty string clears the current key. Model fields: UUID `id`/`provider_id`, `model_id`, `name`, `kind` (`chat`, `vision`, `embedding`), `context_window`, nullable positive `max_output_tokens`.
- `GET /v1/model-configuration/providers/:id/models`: authenticated discovery through the stored provider, bounded to 2 MiB/1024 entries; returns `{models:[{model_id,name}],has_more}`. It never automatically adds unknown models or infers their category. Further pages can be entered manually.

The base URL includes the API version prefix. A custom path is appended to it. HTTP(S) endpoints reject URL credentials, queries and fragments; redirects are disabled. Limits: 64 providers, 1024 models, 4096 retained connection snapshots and 32 MiB of persisted configuration. Updates write/sync a temporary file, atomically replace `model-connections.json`, then publish the new state. This is an HTTP-host file owned by `momo-server`, not an input or resource of `MomoRuntime`. The default location is `$MOMO_DATA_DIR/model-connections.json`; `MOMO_MODEL_CONNECTIONS_PATH` selects another private host path. It contains credentials, including frozen credentials needed by older request references. Removing a current provider does not erase snapshots used for retries. Do not export it as public settings or include it in logs/source control.

## Response routing

2026-10-10 follow-up requirement: Core-owned recommended requests for background purposes, explicit overrides and reset semantics are documented in [purpose request configuration](model_purpose_request_configuration.md). This is a pending design, not an implemented defaults resource. Existing routing support below must not be presented as that completed capability.

`model` may be an immutable configured reference. An unknown reference or embedding model fails before response admission writes user content. Old gateway aliases remain supported. Configured endpoints receive their actual provider model ID; internal `momo_*` gateway metadata is removed.

An optional `momo.model_routes` map extends the response request. Allowed keys are `vision`, `memory_distillation`, and `semantic_graph_governance`. Each entry contains `model` and nullable `temperature`, `top_p`, `max_output_tokens`. The host resolves “follow conversation” to the originating request's model/parameters before sending it. It freezes that selection under the request ID; editing host groups changes new requests only. These fields participate in Core's request fingerprint. Empty maps are omitted from serialization, preserving existing request fingerprints.

Core applies the Space's request-override policy to proposals. DDM uses the memory-distillation route. Configured vision routes require vision-kind definitions before admission, including ordinary text turns; the vision route may be omitted. Legacy gateway aliases still undergo capability checks when fallback is needed. Response provenance captures route references without credentials. A maintenance batch uses the last participating turn's captured configuration, so another window's selected conversation cannot redirect it. Older evidence without route metadata retains legacy aliases. Model routes do not change memory ownership, injection or automatic/manual provenance.

Embedding definitions are rejected from generation. Registration alone does not change an index; explicit `momo.embedding` selection connects a definition to the existing dedicated profile/index contract described below.

2026-10-09 setting correction: `context_window` may be omitted or null to retain an undeclared capability; zero is rejected. Internal budgeting uses a protective estimate rather than storing that estimate as a declaration. Route entries also accept optional `parameters`, governed by the originating Space's allowed-parameter policy. Empty parameter maps are omitted from serialization so existing request fingerprints remain stable. See [optional model context](optional_model_context.md).

## Protocol coverage and verification

OpenAI Chat Completions reuses the bounded Core gateway. Responses, Anthropic Messages and Gemini convert text/system/image/function messages and normalize completion/usage/stream text. Native streams require a successful terminal event; mismatch, truncation, upstream errors, argument/stream limits and cancellation fail explicitly. Function-call deltas for these three protocols are emitted after the provider completes the call, while text remains incremental. Gemini images currently require a base64 data URL. Provider-specific reasoning/signature extensions are not a general compatibility guarantee.

Specification references: [OpenAI Responses events](https://developers.openai.com/api/reference/resources/responses/streaming-events), [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming), [Gemini content generation](https://ai.google.dev/api/generate-content).

Offline tests cover all four request/stream conversions, secret redaction, CAS, persistence, frozen references and function payloads. Real local HTTP integration from MOMO App covers configuration, model discovery, selected model/sampling/output, default-assistant response and idempotent replay against a fixture provider. These are not paid provider tests or ron desktop acceptance.

## Configured NSG embeddings (2026-10-10)

`momo.embedding` is an optional, separate selection: `{ "model": "configured_…", "dimension": 1536 }` uses an embedding-kind connection; `{ "model": null, "dimension": null }` explicitly disables query embeddings. Omission retains the existing gateway contract for independent Core users. It is not a generation route and takes no conversation sampling, stream or request-parameter JSON. The current configured adapter accepts OpenAI-compatible embeddings through `openai` or `openai_responses` providers and their base URL's `/embeddings` endpoint.

The host resolver supplies credentials. Core validates the immutable reference, type, protocol and dimension (1–8192) before admitting a response. It derives a stable profile from provider ID, model ID and dimension, using existing L2 normalization and no dimension override in the upstream request. Credentials and account revision changes do not change vector compatibility; model or dimension changes do. Selected semantic-graph sources receive existing incremental index maintenance before query retrieval; unchanged documents reuse their vectors. A failed batch does not replace the prior index, and response warnings preserve the existing retrieval degradation behavior. Source assets remain unchanged. Web may choose models directly for background generation and send model-only routes; existing Core route parameter support remains available to independent consumers.

验证（2026-10-10）：Rust 1.99.0 的 Core 217 项、server 30 项单元测试通过；受影响库的严格 Clippy、格式和 locked server 构建通过。真实 Web HTTP 与隔离 Core/离线模型验证首次索引、增量复用、查询调用、错误类型、维数不匹配保留旧索引/源资产、关闭后不调用向量接口；取消能中断正在等待的向量请求。浏览器显示降级提示。没有验证付费服务商或 ron 桌面。

## Ownership correction (2026-10-09)

The first implementation loaded account storage from `MomoRuntime`; that contradicted the existing [domain/configuration boundary](domain_and_configuration_boundaries.zh-CN.md). Account persistence, revisions, redaction and discovery now live in `momo-server`. It injects a shared `ModelConnectionResolver` through `MomoApiService::with_model_connections`. Embedding users may supply their own resolver/storage, or keep `MomoApiService::new` and the existing gateway without any account file. Protocol validation remains in Core. An unresolved `configured_` reference fails rather than falling back to a different model.

Default-assistant identity/mapping, purpose aliases, request override policies, Prompt Spaces, memory/NSG/DDM orchestration were already Core capabilities. The preceding feature commit added atomic default creation and host-selected connection/parameter transport, not replacements for those definitions. The earlier Space lifecycle work added asset disposition/recovery transactions, not another memory implementation.

Verification of this ownership correction: Rust 1.99.0, `cargo test --locked -p momo_core -p momo-server --lib` (214 Core / 30 server tests), targeted all-target Clippy with `-D warnings`, locked server build and formatting passed. New tests prove embedded initialization ignores a malformed host file, resolver injection is service-scoped and validates its result, and an external host directory retains removed-model references after reopen. MOMO App's real HTTP fixture also passed with `MOMO_MODEL_CONNECTIONS_PATH` outside the Core data directory, including discovery, default-assistant streaming, parameters and replay. No paid provider or additional desktop acceptance is implied.
