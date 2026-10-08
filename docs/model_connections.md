# Local model connections

2026-10-09. This is a local administration profile used by MOMO App. Core owns provider calls and connection persistence; the host owns parameter-group names and selection. No model parameters are bound to character cards or conversations.

## Administration

- `GET /v1/model-configuration`: `{revision, providers, models}`. Providers expose `has_api_key`, never the stored key. Models expose immutable `configured_<sha256>` references.
- `PUT /v1/model-configuration`: complete replacement with the expected `revision`. Stale revisions return 409. Provider fields: UUID `id`, `name`, `protocol` (`openai`, `openai_responses`, `anthropic`, `gemini`), `base_url`, optional `api_path`, optional `api_key`. Omitting the key preserves it; an empty string clears the current key. Model fields: UUID `id`/`provider_id`, `model_id`, `name`, `kind` (`chat`, `vision`, `embedding`), `context_window`, nullable positive `max_output_tokens`.
- `GET /v1/model-configuration/providers/:id/models`: authenticated discovery through the stored provider, bounded to 2 MiB/1024 entries; returns `{models:[{model_id,name}],has_more}`. It never automatically adds unknown models or infers their category. Further pages can be entered manually.

The base URL includes the API version prefix. A custom path is appended to it. HTTP(S) endpoints reject URL credentials, queries and fragments; redirects are disabled. Limits: 64 providers, 1024 models, 4096 retained connection snapshots and 32 MiB of persisted configuration. Updates write/sync a temporary file, atomically replace `model-connections.json`, then publish the new state. This file belongs to the private Core data directory and contains credentials, including frozen credentials needed by older request references. Removing a current provider does not erase snapshots used for retries. Do not export it as public settings or include it in logs/source control.

## Response routing

`model` may be an immutable configured reference. An unknown reference or embedding model fails before response admission writes user content. Old gateway aliases remain supported. Configured endpoints receive their actual provider model ID; internal `momo_*` gateway metadata is removed.

An optional `momo.model_routes` map extends the response request. Allowed keys are `vision`, `memory_distillation`, and `semantic_graph_governance`. Each entry contains `model` and nullable `temperature`, `top_p`, `max_output_tokens`. The host resolves “follow conversation” to the originating request's model/parameters before sending it. It freezes that selection under the request ID; editing host groups changes new requests only. These fields participate in Core's request fingerprint. Empty maps are omitted from serialization, preserving existing request fingerprints.

Core applies the Space's request-override policy to proposals. DDM uses the memory-distillation route. Visual description requires a vision-capable model when images actually require fallback; an ordinary text turn can keep its unused vision route following the dialogue. Response provenance captures route references without credentials. A maintenance batch uses the last participating turn's captured configuration, so another window's selected conversation cannot redirect it. Older evidence without route metadata retains legacy aliases. Model routes do not change memory ownership, injection or automatic/manual provenance.

Embedding definitions are catalog metadata and are rejected from generation; the existing dedicated embedding gateway/profile contract remains responsible for vector dimensions and index compatibility. This change does not silently switch an existing vector index to a new embedding model.

## Protocol coverage and verification

OpenAI Chat Completions reuses the bounded Core gateway. Responses, Anthropic Messages and Gemini convert text/system/image/function messages and normalize completion/usage/stream text. Native streams require a successful terminal event; mismatch, truncation, upstream errors, argument/stream limits and cancellation fail explicitly. Function-call deltas for these three protocols are emitted after the provider completes the call, while text remains incremental. Gemini images currently require a base64 data URL. Provider-specific reasoning/signature extensions are not a general compatibility guarantee.

Specification references: [OpenAI Responses events](https://developers.openai.com/api/reference/resources/responses/streaming-events), [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming), [Gemini content generation](https://ai.google.dev/api/generate-content).

Offline tests cover all four request/stream conversions, secret redaction, CAS, persistence, frozen references and function payloads. Real local HTTP integration from MOMO App covers configuration, model discovery, selected model/sampling/output, default-assistant response and idempotent replay against a fixture provider. These are not paid provider tests or ron desktop acceptance.
