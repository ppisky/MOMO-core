# MOMO Core 1.0.0 release contract

**Status:** `v1.0.0-rc.1` authorized for GitHub prerelease publication; the
existing local `v1.0.0` tag remains a historical candidate and must not be
moved or published

**Updated:** 2026-09-08

The checks and artifact hashes below describe the existing tagged candidate,
not later work made after that tag. Current contract precedence is
defined by [`spec_index.md`](spec_index.md), and the supported transport
surface is defined by [`http_api_1_0.md`](http_api_1_0.md).

MOMO 1.0 is a stability release, not a version-number-only release. Core owns
native orchestration and portable policy, while provider and application
integrations remain adapters. The unpublished 0.5 wire and ownership shapes are
not compatibility surfaces.

## Gate 1: native contract and Space boundaries

- [x] Freeze `momo.responses/1.0` with explicit `personal_space_id`,
  `conversation_space_id`, weighted `memory_sources`, and one optional
  `memory_write_space_id`.
- [x] Remove the process-wide default UUID, `MOMO_SCOPE_ID`, and the withdrawn
  character-catalogue UUID. Characters are globally addressed by
  `character_id`; optional `owner_space_id` is management/export ownership.
- [x] Keep new session, conversation deletion, selective DMW/NSG clearing, and
  character switching as separate structured controls that do not pass through
  a model.
- [x] Reject the unpublished scope-shaped request and session formats. No 0.5
  compatibility fixtures or decoder remain in the release candidate.
- [x] Keep Core embeddable; `momo-server` is an optional HTTP transport rather
  than a required product process.

## Gate 2: multimodal and model boundary

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

The `v1.0.0-rc.1` prerelease is authorized with this item still open. It remains
a stable `v1.0.0` gate. Local direct-multimodal, fallback, persistence, replay,
and cross-protocol tests are green.

## Gate 3: portable data and fail-closed validation

- [x] Treat the workspace crates as implementation modules rather than
  independently published crates.io SemVer APIs; every package is marked
  `publish = false`. The versioned HTTP wire and documented portable formats
  remain the product stability surface.
- [x] Keep byte-identical Core/mobot `contracts/1.0` golden fixtures.
- [x] Publish [migration notes](migration_0_5_0_to_1_0_0.md) for the
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
- [x] Replace the unpublished MOC v2 shape with MOC v3 Space modules. Export
  selects character ownership, conversations, DMW, and NSG independently;
  import preserves source Space IDs unless an explicit one-to-one `space_map`
  converts them.
- [x] Validate the complete container and all known business payloads before
  committing imported data. Unknown host modules are reported and copied only
  after an explicit claim.
- [x] Keep workspace and maintenance-prompt file access inside validated
  filesystem boundaries; reject traversal, links, missing/empty prompt files,
  and credential-like portable keys.

## Gate 4: release candidate reproducibility

- [x] Provide aligned detailed `momo.example.toml` files, complete referenced
  DMW/NSG Markdown prompts, and Simplified Chinese and English configuration
  and Space/control guides.
- [x] Re-run formatting, all-target/all-feature tests, strict Clippy, rustdoc,
  cross-repository fixture comparison, and Release builds from the final
  revisions.
- [x] Confirm both services remain stopped, scan Release binaries for the
  removed default UUID/constants, and record SHA-256 hashes.
- [x] Commit and move the annotated local `v1.0.0` tag to the verified commit.
  Do not push the commit/tag and do not create a GitHub Release.
- [x] Receive owner authorization for local-only commit/tag creation.
- [x] Receive owner authorization to publish the post-tag corrections as the
  `v1.0.0-rc.1` GitHub prerelease without moving the historical `v1.0.0` tag.

## Explicitly outside 1.0

Audio, video, realtime media, automatic provider failover, arbitrary vendor
extension conversion, MOC v1 migration, incremental MOC, APNG LSB carriers,
and lossy image carriers are not part of the 1.0 gate.

## Previously verified tagged artifacts

- `target/release/momo-server.exe`
  SHA-256: `A67125947B96A8A0557B57736197482C31A8B118A3B829F8F47D100B6200B7E0`
- `D:/mobot/target/release/momo-bot.exe`
  SHA-256: `7E8D397274393F24558CB54EFED3D8D845F6A7038AABCDF656370498898AFACB`

These are local test artifacts, not published release assets.
