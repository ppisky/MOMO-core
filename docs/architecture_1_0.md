# MOMO Core 1.0 architecture

This describes the current implementation and its ownership rules. Product
and artifact contracts retain the precedence defined in [spec_index.md](spec_index.md).
MOMO is a local-first modular monolith: one live Core owns a data directory.

## Dependencies and application boundaries

```text
momo-server (startup, DTOs, routes, HTTP/SSE)
    -> momo-core (typed application operations and response orchestration)
        -> momo-memory (DMW, NSG, retrieval, projection, file plans)
        -> momo-storage (SQLite transactions and disposable Turso vectors)
        -> momo-moc / momo-crypto / momo-config (portable artifacts)
            -> momo-domain (shared domain records where needed)
```

`momo-server/src/main.rs` only starts the application. The library owns HTTP
assembly; `startup.rs` owns environment wiring and shutdown; `routes/` groups
handlers by capability. Handlers translate HTTP into typed Core arguments.
There is no internal compatibility `*_json` facade. Handlers must not serialize
a Rust record to a string merely to pass it to another Rust function.

`api::runtime_api` exposes the runtime-bound application operations. Character,
conversation, message, patch review, context, embedding and portable requests
use Rust records; command functions return records or `()`. Native callers
update together with the workspace; old internal signatures are not retained.
JSON is still appropriate at actual persistence/wire boundaries and for
extensible provider parameters, audit payloads and heterogeneous retrieval
records. Eliminating every `Value` is not an architectural objective.

`MomoApiService::execute` owns the response operation. Its internal generation
path passes `GatewayMessage`, `ChatCompletion`, `CharacterCard` and persisted
`ResponseOperation` records directly. Serialization happens when a durable
replay record or a protocol message actually needs bytes.

## Runtime ownership

| Owner | Resources and responsibilities |
| --- | --- |
| `MomoCore` | Data-directory lock, SQLite/Turso handles, workspace cache, typed Space exclusion, tracked file operations and per-Space recovery |
| `MomoRuntime` | Instance settings, persistent Prompt Spaces, capabilities, cancellation registrations, control/review locks and shared response coordination |
| `ResponseCoordination` | Request and conversation exclusion, generation lanes, active operation state, tracked background work |
| `MomoApiService` | Response/maintenance orchestration and model gateway wiring; reads policy and prompts from its runtime |
| `momo-server` | HTTP limits, timeout policy, metrics, transport serialization and graceful shutdown |

Constructing two services around the same runtime shares their coordination
state, policy and prompts. Constructing a service cannot reset runtime policy.
Creating an unrelated runtime does not share it. Runtime owns resource lifetime;
business decisions stay in application operations. No process-global
service registry or replaceable repository hierarchy is needed.

Per-resource locks serialize operations on the same resource. DMW and NSG
maintenance have separate batch lanes, then share the Space write lock while
preparing/applying their file plans. Reviewed patches take the review lock
before the Space lock. Multi-Space retrieval and MOC operations acquire typed
UUID Space locks in sorted order, deduplicated by identity. Code acquiring
several locks must preserve these orders.

Cancellation registrations have a drop guard. Cancelling or dropping an
upstream request cannot leave a stale registration behind. Foreground streaming
and non-streaming generation both listen for cancellation. A journaled file
commit that has started is instead owned through completion: cancelling its
caller does not release its locks while files are still being written.
Ordinary DMW/NSG writes also keep their Space lock in the file task after caller
cancellation. Memory clearing retains the lock through file and database cleanup.
`wait_for_maintenance` drains these writes, journaled commits, retrieval and
portable operations through the shared Core task tracker during shutdown.
This does not add crash-recovery journals to ordinary writes or MOC imports.
Retrieval updates activity and therefore also retains its locks through caller
cancellation. Native response UUIDs are canonicalized before request identity,
fingerprinting and conversation coordination.

## Authority and durability

| Data | Authority | Recovery |
| --- | --- | --- |
| Characters, conversations, messages, replay records, reviews, state audit | SQLite | SQL transactions and persisted operation identity |
| DMW/NSG content | Markdown/YAML files | Validated mutations; journaled maintenance/review plans |
| NSG vectors | Disposable Turso index | Source hashes and vector-space identity reject stale entries; rebuild from source |
| Portable snapshots | Explicit MOC modules | Preflight validation and documented import conflict policy |

Storage is grouped by repository responsibility under `momo-storage/src/local/`.
All groups use the same `LocalStore` and SQLite pool. Response completion still
commits the assistant message, maintenance evidence, state-operation completion
and replay response in one transaction. Splitting source files must never split
that transaction into independent repository calls.

## Cross-store memory commits

SQLite migration `0024_prepared_maintenance_commit.sql` extends the existing
maintenance and review records with a private prepared-file plan. It is runtime
recovery data, not a new portable artifact or a vector-cache payload.

The maintenance lifecycle is:

```text
pending turns -> staged model patch -> prepared file plan -> applied files
                                                      -> SQL acknowledgement
```

The staged batch binds the original Space, maintenance kind and ordered request
IDs. Changes to the configured batch size cannot change that evidence window.
The prepared plan freezes every target's relative path, before-image and
after-image. It is persisted before the first file mutation. Preparation and
application run under the Space lock. Successful acknowledgement marks the exact
source turns and removes the batch in the same SQL transaction. Other maintenance
lanes remain independently pending.

Patch approval uses the same file-plan mechanism. A review with a prepared plan
records approval intent; it cannot subsequently be rejected while that commit
is pending. Its final approved audit status clears the plan. Validation failures
before a plan exists can still mark the review failed.

Before publishing a newly initialized Core, startup replays all prepared plans
under the exclusive data-directory lock, then acknowledges their SQL records.
It performs no model calls. Unprepared patches remain staged for the normal
maintenance path. During replay each target must contain either its original
bytes or its exact intended bytes. Every target is checked before any file is
written; an unexpected edit, unsafe path or unsupported plan version isolates
the affected Space and preserves the journal for diagnosis. Other Spaces and
the management API remain available. Existing after-images are
skipped, including when all files were written before the process exited.

Live prepared commits are marked pending before their first file mutation.
The next operation must recover that Space before accessing memory, so a
failed acknowledgement cannot be silently superseded by a later write.
`GET /v1/memory/recovery` reports affected Spaces; after repairing the conflicting
file, `POST /v1/memory/recovery/:space_id/retry` retries without restarting.
There is no force-overwrite or discard-journal operation.

This provides process-interruption recovery for background DMW/NSG maintenance
and reviewed DMW patches. It is not a distributed transaction or a universal
power-loss guarantee. Direct low-level file edits, ordinary administrative
mutations retain their existing validation/rollback boundaries. MOC operations
share Space exclusion and own admitted work through caller cancellation, but
complete multi-module imports are not crash-atomic. Offline tools must not
mutate files concurrently with a live Core.
Turso rebuilding is deliberately outside the commit: an outdated cache can be
ignored and rebuilt without undoing authoritative memory.

## Evolution and verification

Extract a new owner when it has an independent lifecycle or invariant. A long
file alone is not enough reason to add a service, crate, trait or network hop.
The response pipeline remains together because its replay and commit rules
span generation, state publication and maintenance scheduling.

Regression tests cover shared coordination across separately constructed
services, cancellation cleanup, in-flight commit ownership, source-window
preservation after a threshold change, recovery before/mid/after file writes,
operator-edit conflicts, and reviewed-patch audit recovery. Existing HTTP/SSE
and `contracts/1.0` fixtures remain the product compatibility gates.

See [development.en.md](development.en.md) for checks. Future expansion should
extend these invariants and fault-injection cases before adding new abstraction
layers. A multi-process shared writer or remote multi-tenant deployment would
require a new ownership and authorization design, not a change of listener address.
