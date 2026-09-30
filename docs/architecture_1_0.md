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
| `MomoCore` | Data-directory lock, SQLite/Turso handles, Space supervisor, mailbox admission, tracked operations and per-Space recovery |
| `SpaceWorker` | Privately owned DMW/NSG workspace, disposable working copy, durable journal and file materialization |
| `MomoRuntime` | Instance settings, persistent Prompt Spaces, capabilities, cancellation registrations, control/review locks and shared response coordination |
| `ResponseCoordination` | Request and conversation exclusion, generation lanes, active operation state, tracked background work |
| `MomoApiService` | Response/maintenance orchestration and model gateway wiring; reads policy and prompts from its runtime |
| `momo-server` | HTTP limits, timeout policy, metrics, transport serialization and graceful shutdown |

Constructing two services around the same runtime shares their coordination
state, policy and prompts. Constructing a service cannot reset runtime policy.
Creating an unrelated runtime does not share it. Runtime owns resource lifetime;
business decisions stay in application operations. No process-global
service registry or replaceable repository hierarchy is needed.

Each UUID Space has one `SpaceWorker`. Core's private `SpaceHandle` contains a
bounded mailbox, not an `Arc<MemoryWorkspace>` or a state guard. DMW, NSG,
retrieval, provenance, export, import and clearing all send owned commands.
`MemoryWorkspace` is no longer cloneable or `Sync`; its index is a private
`RefCell` cache. Transport/application callers cannot obtain the workspace.

The supervisor owns the address registry, with at most 128 live workers and
64 queued commands per worker. Idle addresses can be evicted; addresses held
by callers or running commands cannot. Registry and worker shutdown are joined
before releasing their ownership of the data-directory lock.

DMW and NSG maintenance retain separate batch lanes. Operations spanning Space
commands and SQLite work request a mailbox reservation. A multi-Space request
reserves its deduplicated UUID set together, without holding a partial set.
Conflicting requests retain arrival order; unrelated Spaces can proceed.
Dropping a reservation sends a release message. Review/control/conversation
exclusion and generation semaphores coordinate workflows; they do not expose
memory state. Reviewed patches reserve their review lane before Space admission.

Runtime settings, Prompt Spaces, capabilities, response-attempt metadata,
cancellation registrations, recovery diagnostics and task tracking use private
state owners. There is no poisoned-lock `into_inner` recovery. A failed volatile
owner closes its mailbox and fails closed until the runtime is recreated.
Persistent Prompt Spaces publish their file snapshot before replacing the
cached overrides. Worker failure recovery is described below.

Cancellation registrations have a drop guard. Cancelling or dropping an
upstream request cannot leave a stale registration behind. Foreground streaming
and non-streaming generation both listen for cancellation. A journaled file
commit that has started is instead owned through completion: cancelling its
caller does not release its reservation while files are still being written.
Ordinary DMW/NSG writes also retain their reservation after caller cancellation.
Memory clearing retains it through file and database cleanup.
`wait_for_maintenance` drains these writes, journaled commits, retrieval and
portable operations through the shared Core task tracker during shutdown.
Ordinary writes and each imported Space module now use the worker journal too.
Retrieval updates activity and therefore also retains its reservation through caller
cancellation. Native response UUIDs are canonicalized before request identity,
fingerprinting and conversation coordination.

## Authority and durability

| Data | Authority | Recovery |
| --- | --- | --- |
| Characters, conversations, messages, replay records, reviews, state audit | SQLite | SQL transactions and persisted operation identity |
| DMW/NSG content | Per-Space SQLite after-image journal; Markdown/YAML files are a materialized copy | Restore the latest snapshot on every worker start; prepared maintenance/review plans retain their SQL acknowledgement |
| NSG vectors | Disposable Turso index | Source hashes and vector-space identity reject stale entries; rebuild from source |
| Portable snapshots | Explicit MOC modules | Preflight validation and documented import conflict policy |

Storage is grouped by repository responsibility under `momo-storage/src/local/`.
All groups use the same `LocalStore` and SQLite pool. Response completion still
commits the assistant message, maintenance evidence, state-operation completion
and replay response in one transaction. Splitting source files must never split
that transaction into independent repository calls.

## Cross-store memory commits

Every worker mutation follows this sequence, including index repairs and
retrieval activity that happen inside apparent reads:

```text
owned command -> private working copy -> validated complete after-image
             -> SQLite journal COMMIT (WAL, synchronous=FULL)
             -> materialize changed files -> mark applied -> reply
```

The journal is `spaces/<space_id>/memory-journal.sqlite3`; the working copy is
the reserved sibling `.memory-worker-cache`. Neither is a portable MOC module.
The working copy is never a recovery source and is discarded when rebuilding.
Existing file workspaces receive an initial journal checkpoint on first use.
The journal retains the latest two complete after-images, compacting older
applied entries in the same transaction that admits the next image. This is
bounded recovery history, not a permanent event/audit history.

A command error rolls back its private working copy before serving another
command. A panic discards the whole incarnation, including all cached index
state. The worker supervisor reconstructs from the latest journal snapshot
before handling another queued command. Cold startup and idle-worker reload use
the same latest-snapshot recovery, even when the snapshot is marked applied.
An applied marker never makes materialized files authoritative. Initialization
and recovery panics are also contained at the worker boundary.
Corrupt journals and unsafe paths isolate their Space and remain visible in
`memory_recovery_status`; a later admission can retry after operator repair.
No failed command closure is automatically executed a second time. A caller
whose worker panicked receives an uncertain-outcome error and must reconcile
against recovered state. Other workers continue independently.

The release profile uses `panic = "unwind"` so command panics reach this
supervision boundary. Abrupt process termination is handled on next startup.
Tests terminate child processes before journal commit, after commit, during
materialization and before acknowledgement, as well as injecting caught panics.

Memory clearing commits its file after-image and a `pending_clear` record in
the same Space journal transaction. Until SQL memory-state cleanup and any NSG
vector cleanup finish and the record is acknowledged, ordinary worker commands
are rejected. Startup/admission recovery finishes this idempotent cleanup before
replaying prepared maintenance, lifecycle or review plans; cleared plans cannot
resurrect content. Cleanup failure isolates the Space and preserves the intent
for retry. Tests cover process exits during clearing and SQL cleanup failure.

This first implementation stores complete after-images and scans snapshots at
command boundaries; it trades additional I/O and disk space for one recovery
path covering existing low-level mutations. Incremental logging and scheduling
optimizations must preserve this commit point and the fault-injection tests.
An MOC operation covering multiple modules/Spaces and SQL repositories still
is not one distributed transaction; each worker command is independently durable.
After the initial migration checkpoint, direct filesystem edits are never
ingested into the worker cache and are overwritten by journal reconstruction.
Use the import APIs to change managed content durably.

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
application run under a Space reservation and execute through its worker. Successful acknowledgement marks the exact
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
`GET /v1/memory/recovery` reports affected Spaces; after repairing the underlying
storage fault, `POST /v1/memory/recovery/:space_id/retry` retries without restarting.
Logical conflicts concern journaled content and prepared plans; editing the
materialized copy cannot repair those durable records.
There is no force-overwrite or discard-journal operation.

This provides process-interruption recovery for background DMW/NSG maintenance
and reviewed DMW patches. It is not a distributed transaction or a universal
power-loss guarantee. Administrative memory commands use the same worker
journal and validation boundary. MOC operations
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
