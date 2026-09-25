# MOMO Core HTTP boundary 1.0

**Status:** normative local transport contract
**Updated:** 2026-09-03

`momo-server` is a loopback-oriented transport for MOMO Core. This document
defines which parts of its `/v1` surface carry the MOMO 1.0 compatibility
promise and which parts are local administration facilities.

`GET /health` is the transport discovery probe used by local hosts. Its
`ok`, `service` and `core_version` fields identify a running `momo-server` and
allow a host to enforce its supported Core version range; it is not a product
data or orchestration endpoint.

## Stable product wire

The following routes are the versioned product wire contract:

| Route | Contract | Purpose |
| --- | --- | --- |
| `POST /v1/momo/responses` | `momo.responses/1.0` | Complete conversation orchestration; JSON or SSE |
| `POST /v1/momo/responses/{request_id}/cancel` | `momo.responses/1.0` identity rules | Cancel an active response in one personal Space |
| `POST /v1/momo/control` | `momo.control/1.0` | Delete a conversation, clear selected memory modules, or switch a character |

The response-creation and control request bodies must explicitly carry their
contract `schema`; an omitted, unknown, or future schema is rejected. The
cancellation request uses the response operation identity in its path and does
not repeat the schema. Unknown request fields are rejected. The frozen examples
under `contracts/1.0` are regression fixtures, while the Rust wire types remain
the executable definition of every field and limit.

Completed response and control operations support persistent idempotent replay.
Reusing an ID with different normalized content returns HTTP 409. Response IDs
are namespaced by `personal_space_id`; control IDs are namespaced by
`actor_space_id`. An identical control that is still executing returns HTTP 409
and may be retried after the first attempt completes.

If the process stops after claiming a control but before storing its response,
the unfinished claim is released on the next initialization and the identical
request may execute again. The defined controls are idempotent state
transitions, so this recovery converges on the requested state; only a response
stored before shutdown is guaranteed to replay byte-for-byte. Exactly one
Core/server process may own a data directory at a time.

Stable routes return the common `{ "error": ResponseError }` JSON envelope on
failure. Invalid JSON or an invalid typed shape (including unknown fields), an
unsupported schema or invalid action, a missing or mismatched resource, and an
idempotency collision are distinguished by HTTP status and the envelope's
machine-readable `code`; callers must not parse human-readable messages.

## Local administration profile

All other `/v1` routes are supported local administration APIs, not independent
1.0 compatibility surfaces. They expose character and conversation CRUD,
memory and NSG inspection, embedding/index maintenance, capability inspection,
context diagnostics, runtime settings, Prompt Spaces, MOC operations, and LSB
file operations. Their request and response shapes may evolve with the workspace and
must not be used as a cross-version or remote public API without a host-owned
adapter.

`GET/PUT /v1/runtime-settings` manages the typed process-wide behavior resource.
`GET/PUT/DELETE /v1/prompt-spaces/{id}` manages the fixed prompt slots. MOMO
does not read `momo.toml`; hosts translate their user configuration into these
JSON APIs. MOC import/export does not carry runtime settings or prompt content.

`GET /v1/mo-state/runtime?space_id=<uuid>` exposes the local MO State manager
status for diagnostics: the active profile, independent DMW/NSG/scene
revisions, snapshot revision, degraded state, last error, and current durable
snapshot. Before a Space has handled an autonomous MO State event it returns
`status: "not_initialized"`. This route is observational and does not start or
advance a scene.

The direct deletion routes in this profile are trusted administrative actions.
They intentionally do not accept `momo.control/1.0` request IDs. End-user
workflows must use the structured control route so that action identity,
ownership checks, and replay semantics are preserved.

## Trust and deployment

The server binds to loopback by default and has no end-user authentication or
Space access-control database. Core validates supplied UUIDs and resource
ownership relationships, but the host decides which Spaces a caller may read
or write. A non-loopback bind is allowed only through the explicit environment
override and must sit behind a trusted authenticated proxy that also prevents
ordinary users from reaching the administration profile.

A data directory has one active process owner. Running multiple Core or server
processes against the same SQLite and file-backed Space tree is unsupported.

Several administration routes read or write host filesystem paths. They are
appropriate only for trusted local operators. They are not upload/download
endpoints and must not be exposed directly to untrusted remote clients.

## Artifact compatibility

HTTP stability and artifact stability are separate. The current portable
contracts are listed in [`spec_index.md`](spec_index.md). In particular, MOC,
MOMO Character Card, memory/NSG files, and LSB payloads keep their documented
format versions even when invoked through an
administration route.

## Rust embedding boundary

The workspace crates are `publish = false` implementation modules. Embedders
may use their public Rust items from the same pinned workspace revision, but
those items do not receive an independent crates.io SemVer promise.
`momo_core::MomoRuntime` is the instance-owned composition root used by
`momo-server`. Runtime-bound typed application operations live under
`momo_core::api::runtime_api`; they require an explicit runtime reference and
do not use process-global Core state. HTTP handlers use these typed operations
directly. The internal JSON wrapper API has been removed; native callers update
to typed operations with the workspace. Runtime settings and persistent Prompt
Spaces belong to `MomoRuntime`, not independently constructed execution services.
See [architecture_1_0.md](architecture_1_0.md) for ownership and recovery boundaries.

## Memory recovery (local management)

`GET /v1/memory/recovery` returns an object mapping affected Space UUIDs to
recovery diagnostics. A pending or conflicting Space is unavailable for memory
access; unrelated Spaces remain usable. After restoring conflicting files to
their original or intended bytes, `POST /v1/memory/recovery/:space_id/retry`
replays the durable plan and returns `{"ok":true}`. An unresolved conflict is
HTTP 409. Retry does not discard journals or overwrite unexpected contents.

## Evaluation maintenance barrier (local management)

`POST /v1/momo/maintenance/turns` records an already-completed user/assistant
turn for trusted local replay. It writes a pending DMW/NSG maintenance turn but
does not invoke the conversation model. MORP's `history_mode: "recorded"` also
uses the ordinary conversation/message administration routes so current-context
and extracted-memory probes see the same fixed transcript. This route is not a
public ingestion API and must remain behind the local administration boundary.

`POST /v1/momo/maintenance/drain` accepts `{"space_id":"<UUID>"}` and returns
`{"completed":true,"space_id":"<UUID>"}` after flushing pending DMW and NSG
maintenance, including partial batches. This invokes configured model routes;
it is not a read-only status check and is not part of the stable 1.0 wire.
Run against an isolated evaluation instance with no concurrent writers. Errors
and timeouts do not report completion; pending work can be retried. Processing
is bounded by the server response timeout and at most 64 batches of up to 32
turns per kind. See [MORP paired experiments](../benchmarks/morp/SCENARIOS.md).
