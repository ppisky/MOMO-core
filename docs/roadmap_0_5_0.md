# MOMO Core 0.5.0 release contract

**Status:** released
**Updated:** 2026-08-27

MOMO 0.5 freezes a coherent pre-1.0 architecture. A checklist item is complete
only when its public contract, implementation, negative cases, and docs agree.

## Frozen boundaries

- Core owns native `MomoApi`, domain state, persistence, DMW/NSG/MO State,
  context construction, request governance, MOC, compatibility conversion, and
  LSB codecs.
- `momo-server` transports MomoApi over local HTTP/SSE. Its high-level route is
  `POST /v1/momo/responses`.
- CLI, Discord, OpenAI Chat Completions, OpenAI Responses, Anthropic Messages,
  model providers, and future visual description are adapters. They do not own
  Core conversation orchestration.
- `config.toml` is host-local adapter wiring. `momo.toml` is portable behaviour
  and contains request/vision governance. Credentials never enter either a MOC
  or a request audit.

## Frozen formats

- MOC uses `.moc`; its bytes are a tar archive compressed with Zstandard. MOC
  v2 is snapshot-only and has no v1 migrator or incremental/deletion variants.
- Import and export use tagged/enum plans. Stored source, generated external
  character compatibility, MOC compatibility profile, and LSB carrier are
  distinct operations.
- A MOC compatibility profile is one of `none`, `preserved_source`,
  `generated_ccv2_json`, `generated_ccv3_json`, or `generated_ccv3_charx`.
- LSB has exactly one typed payload (`momo_character`, `moc`, or `charx`) and
  one lossless carrier. Supported carriers are PNG and lossless WebP. APNG,
  animated WebP, JPEG, and AVIF are rejected.

## Request governance

The native request may ask to override context window, output tokens, sampling,
instructions, tools, the future visual-description prompt, and explicitly named
provider parameters. `momo.toml` independently allows, ignores, or rejects each
class. The effective context/output budget, sources, applied fields, ignored
fields, and forwarded named parameters are returned in `momo.request_audit`.

Visual input is not enabled in 0.5. The dedicated prompt and override policy are
frozen now so a future vision adapter can be added without moving this concern
into Discord, CLI, or a model-provider implementation.

## Implemented release gates

- [x] Versioned MomoApi response, error, cancellation, replay, and SSE event
  lifecycle.
- [x] Structured tool-call continuation preserves call IDs and tool roles.
- [x] Atomic first-user-message idempotency closes the crash/retry gap.
- [x] Explicit import format and exact preserved source bytes for JSON, PNG, and
  CHARX.
- [x] Typed MOC module selection, compatibility profiles, protection type, and
  conflict mode.
- [x] PNG/lossless-WebP LSB codecs with animation, size, capacity, integrity,
  and payload-type rejection.
- [x] Request-governance parser and effective-request audit.
- [x] Unsupported account sync/account crypto facade and other no-op capability
  claims removed; private MOC encryption remains.

## Completed release audit

- [x] Split Core orchestration, response HTTP/SSE transport, stream encoding,
  and HTTP errors without changing the frozen MomoApi wire contract.
- [x] Regenerate and checksum shared 0.5 fixtures after the route/event changes.
- [x] Verify both repositories with fmt, clippy `-D warnings`, full tests, and
  rustdoc; run the cross-repository contract tests using the Rust test suites.
- [x] Produce the requirement-by-requirement completion report and remove stale
  APNG, MOC migration, ambiguous config, and native/OpenAI route claims.

The remaining pre-release checks require real provider and Discord credentials;
they are deployment smoke tests, not missing product implementations. See the
[English review](review_0_5_0.en.md) or [简体中文 review](review_0_5_0.zh-CN.md).

## 1.0 gate

The next version may be 1.0 only after the remaining 0.5 audit is green and a
real image-input adapter implements the already-frozen multimodal boundary.
Audio/video/realtime, automatic provider failover, and claims of lossless
conversion for arbitrary vendor extensions are outside this gate.
