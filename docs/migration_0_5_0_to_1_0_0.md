# Draft migration: MOMO Core 0.5.0 to 1.0.0

**Status:** destructive upgrade notes; no compatibility layer is provided

## Rust embedding API

`MomoApiService::execute` no longer accepts a caller-supplied flattened input
string. Core validates and derives the effective input from
`MomoResponseRequest`, which prevents a host from persisting or retrieving with
text that disagrees with the request fingerprint.

```rust
// 0.5
service.execute(&request, &request_id, &input, stream).await?;

// 1.0 development API
service.execute(&request, &request_id, stream).await?;
```

`GatewayMessage::content` is now `Option<GatewayMessageContent>` rather than
`Option<String>`. Construct ordinary text with `Some(text.into())`. The
`Parts(Vec<GatewayContentPart>)` form represents outbound multimodal user
content without changing the serialized shape of existing text messages.

The workspace crates are implementation modules and now use `publish = false`.
The supported product stability surface is the native HTTP contract and the
documented MOC/character/memory formats, not independent crates.io packages.

## Image input

Structured native input may contain `input_image` either as a top-level user
item or inside a user message's content blocks. References accept HTTP(S) URLs
or `data:image/` URLs; `detail` is `auto`, `low`, or `high`. A request may
contain at most eight images, and image input cannot be mixed with function
call continuation items.

Image input is disabled until portable configuration enables it:

```toml
[vision]
enabled = true
prompt = "Describe only visible facts relevant to the conversation."
```

Core discovers `/v1/models/conversation` first. When its `momo.modalities`
contains `"image"`, Core sends the original images directly to the final
conversation request and does not use the vision prompt. The existing roleplay
and context instructions remain the authority for that call.

When the conversation model is text-only, the adapter host must expose a
logical model named `vision`, report `"image"` in that route's discovered
modalities, and accept OpenAI-compatible multimodal Chat Completions content.
Portable configuration never contains provider URLs or credentials.

In `mobot`, every `[[models]]` entry declares `modalities`; `[model_use].vision`
optionally selects the fallback model profile while the provider URL, real
model ID, and credential environment variable remain in host-local
`config.toml`. The gateway converts OpenAI Chat `image_url` blocks to
Responses `input_image` blocks or Anthropic `image` sources. Anthropic URL
sources are preserved as URLs; supported base64 data URLs are split into
`media_type` and `data`. Anthropic has no direct equivalent of OpenAI's image
`detail` hint, so that hint is not forwarded on the Anthropic path.

## Local storage

Migration `0017_response_resolved_input.sql` adds nullable
`response_operations.resolved_input_json`. Core fills it before atomically
writing the user message. Pending pre-migration text-only operations remain
readable; new or retried operations populate the field. No raw image bytes are
added to SQLite by this migration.

## Spaces replace the withdrawn request scopes

`momo-server` no longer compiles, reads, or silently selects a product-wide
default scope UUID. `MOMO_SCOPE_ID` and mobot's former `core.scope_id` are
removed rather than made mandatory: a process-level value still mixes users.

Every stateful request supplies the UUID that owns the affected resource. A
native response supplies `personal_space_id`, `conversation_space_id`, zero or
more weighted `memory_sources`, and at most one `memory_write_space_id`.
Characters are resolved globally by `character_id`; there is no character
catalogue UUID.
Response idempotency and cancellation are keyed by personal Space plus request
ID. See [`space_model_1_0.md`](space_model_1_0.md).

The withdrawn scope UUID is not reinterpreted as a character directory or
silently reused. A host may deliberately convert data into explicit Spaces
through an offline tool or MOC v3 `space_map`; otherwise the old data is
discarded.

mobot's historical `discord_sessions.json` stored only a platform session key
and a `conversation_id`, so it cannot prove which conversation Space owns the
referenced conversation. It is not a supported 1.0 migration source. The new
host rejects that schema and creates a fresh versioned session store after the
obsolete file is discarded. There is no ID-only compatibility decoder or
ownership guessing path.

## Host-owned MOC modules

The MOC v3 container validates every declared module and Space module, including unknown
safe module IDs. A host can now explicitly claim validated unknown payloads to
a selected directory during import, and explicitly contribute extension-module
directories during export. Core returns module metadata and claimed paths but
does not interpret or execute those payloads. Without an explicit claim,
temporary unknown payloads are discarded when import finishes.

## Portable maintenance prompts

The DMW distillation and NSG governance system prompts are full Markdown files.
`momo.toml` references them through `memory_distillation_file` and
`semantic_graph_governance_file`; MOC config import/export moves those files
with the TOML. Core rejects missing, unsafe, escaping, non-UTF-8, empty, or
oversized references. It does not replace a missing file with an abbreviated
compatibility prompt.

`[prompts]` itself is optional: omission selects the standard relative files
`prompts/dmw_distiller.md` and `prompts/nsg_governor.md`.

## Wire compatibility

Core and mobot keep only byte-identical `contracts/1.0` fixtures, including a
structured multimodal request, and the native request schema is
`momo.responses/1.0`. Core package metadata is set to `1.0.0`; mobot is an
independent `0.1.0` host that requires Core `>=1.0.0, <2.0.0`. Local tags may
exist, but neither repository has been pushed or published as a GitHub release.
