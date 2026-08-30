# MOMO Core 1.0.0 release contract

**Status:** approved for a local `v1.0.0` tag; GitHub publication is prohibited pending owner testing
**Updated:** 2026-08-30

MOMO 1.0 is a stability release, not a version-number-only release. The 0.5
architecture remains the ownership boundary: Core owns native orchestration and
portable policy, while provider and application integrations remain adapters.

## Image-input gate

- [x] Native structured requests accept bounded HTTP(S) and `data:image/`
  references with `auto`, `low`, or `high` detail.
- [x] Images are limited to eight per operation, allowed only as user input,
  and cannot be mixed into a function-call continuation.
- [x] Portable `vision.enabled` permits image input; its governed prompt applies
  only to the text-description fallback.
- [x] When `conversation` advertises `image`, original image blocks go directly
  to the final roleplay call without invoking the vision adapter.
- [x] For a text-only conversation model, `GatewayVisionAdapter` calls the
  optional logical `vision` route with OpenAI-compatible multimodal content.
- [x] Fallback visual descriptions enter retrieval, persistence, context
  construction, and background maintenance as explicit labelled text.
- [x] Resolved input, vision usage, and upstream request IDs persist with the
  response operation, preserving request-ID retry semantics.
- [x] Unit and workspace tests cover validation, multimodal serialization,
  governed resolution, persistence, and replay.
- [x] The `mobot` adapter host advertises explicit per-model modalities,
  supports an optional `vision` fallback route, and preserves image blocks for
  each supported provider protocol.
- [ ] Owner-run credentialed Core -> gateway -> provider smoke tests with both a
  remote HTTPS image and a bounded data URL.

The local host currently has no value for the configured provider credential
environment variables, so this external deployment check cannot be performed
by the release build process. It does not block the authorized local tag; it
does block GitHub publication until the owner has tested the candidate. Local
direct-multimodal, fallback, persistence, replay, and cross-protocol tests are
green.

## Stable-surface gate

- [x] Treat the workspace crates as implementation modules rather than
  independently published crates.io SemVer APIs; every package is marked
  `publish = false`. The native HTTP/MOC contracts remain the product stability
  surface.
- [x] Freeze the `momo.responses/1.0` native response schema and add new cross-repository golden
  fixtures without deleting the 0.5 fixtures.
- [x] Publish [draft migration notes](migration_0_5_0_to_1_0_0.md) for the
  `MomoApiService::execute` ownership change, multimodal gateway-message
  content type, and SQLite migration.
- [x] Verify formatting, strict Clippy, all-feature tests, rustdoc, fixture
  checksums, and the adapter-host contract from the same revisions.
- [x] Update workspace package versions to `1.0.0` for local release-candidate
  builds.
- [x] Expose host hand-off for unknown MOC modules: explicit validated claim on
  import and explicit extension-module contribution on export, without Core
  interpreting or executing extension payloads.
- [x] Replace abbreviated inline maintenance prompts with bounded portable
  Markdown references; ship complete DMW/NSG files and move them with the MOC
  config module.
- [x] Enforce the normative
  [identity and scope contract](identity_scope_1_0.md): no server default scope,
  explicit personal/conversation/character scopes, scoped resource ownership,
  and scope-bound request replay and cancellation.
- [x] Add cross-scope negative tests for the stateful character, conversation,
  message, and native response routes. A resource from scope A cannot be read,
  written, replayed, or cancelled from scope B.
- [x] Enforce explicit scopes in the native response path, scoped local
  character/conversation/message routes, and request-ID cancellation keys.
- [x] Keep workspace-document access inside its filesystem capability boundary
  and portable import inside its explicit destination-scope boundary; neither
  route falls back to a process-global identity.
- [x] Re-run the complete Core and mobot verification matrix after the first
  identity remediation pass. Credentialed provider smoke testing remains open.
- [x] Receive owner authorization to create a local `v1.0.0` tag. Do not push
  the commit or tag, and do not create a GitHub Release.

## Explicitly outside 1.0

Audio, video, realtime media, automatic provider failover, arbitrary vendor
extension conversion, MOC v1 migration, incremental MOC, APNG LSB carriers,
and lossy image carriers are not part of the 1.0 gate.
