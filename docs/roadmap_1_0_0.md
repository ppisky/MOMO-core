# MOMO Core 1.0.0 release contract

**Status:** `v1.0.0-rc.5` completes the current runtime-ownership and recovery
candidate while keeping the frozen product and portable contracts unchanged.
The architecture described in `architecture_1_0.md` is the stabilization baseline;
further changes require a concrete defect, measured bottleneck or agreed product
requirement. Stable `v1.0.0` follows the final candidate only after every stable
gate is closed.

**Updated:** 2026-09-25

The rc.5 candidate is described in
[its release notes](release_notes_1_0_0_rc5.md). It removes internal compatibility
facades, closes Space ownership gaps and isolates memory-recovery conflicts.
Its publication requires CI on the exact candidate commit; prior release
evidence is not reused as proof for the modified candidate.

The checks and artifact hashes below describe their named tagged candidates,
not later work made after those tags. Current contract precedence is
defined by [`spec_index.md`](spec_index.md), and the supported transport
surface is defined by [`http_api_1_0.md`](http_api_1_0.md).

MOMO 1.0 is a stability release, not a version-number-only release. Core owns
native orchestration and instance runtime policy, while provider and application
integrations remain adapters. The unpublished 0.5 wire and ownership shapes are
not compatibility surfaces.

## Release-train authority

- GitHub `origin/main`, its public tags, and GitHub Releases are the release
  source of truth. Local-only refs never define a published version.
- Public `v1.0.0-rc.1` through `v1.0.0-rc.4` tags are immutable. Each later RC
  receives a new tag, release note, complete CI run, and reproducible evidence.
- The working plan may continue through rc.10. Candidate numbering is an upper
  planning target rather than permission to waive a gate or add scope merely to
  consume a number.
- The final RC is a soak candidate: no new product scope, frozen wire and
  portable formats, only release-blocking corrections and evidence collection.
- GitHub has no published `v1.0.0` tag as of 2026-09-21. The old local-only
  test ref was removed, leaving `v1.0.0` available for the eventual stable
  release after the final RC.

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

The `v1.0.0-rc.2` prerelease is authorized with this item still open. It remains
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
- [x] Replace abbreviated inline prompts with complete tracked Markdown
  defaults under `momo_core`, then expose all Core-consumed slots as Prompt
  Spaces with GET, idempotent PUT replacement, and DELETE-to-default semantics.
  Do not read replacement paths from `momo.toml` or move prompts through MOC.
- [x] Replace the unpublished MOC v2 shape with MOC v3 Space modules. Export
  selects character ownership, conversations, DMW, and NSG independently;
  import preserves source Space IDs unless an explicit one-to-one `space_map`
  converts them.
- [x] Validate the complete container and all known business payloads before
  committing imported data. Unknown host modules are reported and copied only
  after an explicit claim.
- [x] Keep workspace file access inside validated filesystem boundaries and
  reject credential-like portable keys. Product prompt defaults are compile-time
  Core inputs; runtime overrides are validated Prompt Space values persisted as
  internal Core state rather than user-selected filesystem paths.

## Gate 4: release candidate reproducibility

- [x] Provide an English `momo.example.toml`, complete tracked DMW/NSG/Roleplay
  Director Markdown assets, and Simplified Chinese and English configuration
  and Space/control guides.
- [x] Re-run formatting, all-target/all-feature tests, strict Clippy, rustdoc,
  cross-repository fixture comparison, and Release builds from the final
  revisions.
- [x] Confirm both services remain stopped, scan Release binaries for the
  removed default UUID/constants, and record SHA-256 hashes.
- [x] Create an annotated local-only `v1.0.0` test ref for the early candidate
  exercise without publishing it. That non-authoritative ref was later removed
  when GitHub was established as the release source of truth.
- [x] Receive owner authorization for local-only commit/tag creation.
- [x] Receive owner authorization to publish the corrections as the
  `v1.0.0-rc.1` GitHub prerelease.
- [x] Add the default Roleplay Director and MORP 1.0 role-play-only benchmark,
  run all 64 bilingual cases, and retain provider failures as scored zeroes.
- [x] Reduce the recommended rc.2 topology to one Qwen deployment shared by
  the three core scenarios plus one embedding deployment; use one auditable
  Codex reviewer instead of requiring two external judge models.
- [x] Receive owner authorization to publish `v1.0.0-rc.2` without moving the
  public `v1.0.0-rc.1` tag.
- [x] Complete the optional experimental DDM projection: author-owned MOC
  profile transport, management API, closed typed signals, deterministic
  conflicts/top-k, persisted scoped hysteresis, source fingerprints, and
  atomic MO State publication.
- [x] Remove benchmark-derived product defaults from runtime policy.
- [x] Record an rc.3 credentialed eight-case MORP smoke run with one case per
  primary role-play dimension and an auditable Codex review.
- [x] Record an eight-case mixed-context A/B run that exercises real retrieval
  and MO State, with direct and MOMO arms both scoring 84.375. Treat the tie as
  execution evidence, not evidence of benefit.
- [ ] Complete broader release-grade longitudinal and DDM-specific credentialed
  counterfactual behavior evidence before stable `v1.0.0`.
- [x] Receive explicit owner authorization to commit, create
  `v1.0.0-rc.3`, push, and publish the GitHub prerelease without moving earlier
  tags.
- [x] Make Core own bounded per-Space memory-workspace lifetimes, retain index
  validation across external edits, and move retrieval filesystem work off
  asynchronous request workers for the rc.4 candidate.
- [x] Merge and publish the `v1.0.0-rc.4` candidate without moving any earlier
  tag. Post-tag changes require a fresh complete GitHub CI matrix before the
  next release.
- [ ] Publish rc.5 through the final planned candidate as distinct immutable
  GitHub prereleases, each with scoped notes and a complete CI/security record.
- [ ] Treat approximately rc.10 as the final soak candidate: close all remaining
  credentialed, longitudinal, fault-recovery, latency, and independent-quality
  evidence without introducing new product scope.
- [ ] Publish stable `v1.0.0` only after the final candidate completes its soak,
  all gates above are checked, and the exact GitHub commit is revalidated.

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
