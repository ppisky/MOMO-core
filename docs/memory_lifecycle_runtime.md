# MO State activity-driven memory lifecycle

**Status:** implemented native runtime profile, 2026-09-30

MO State coordinates observation, scene/state projection, source revisions,
durable snapshots, maintenance scheduling/recovery and memory lifecycle.
Projection is one component, not the entirety of the runtime. NSG authority
remains separate: automatic creation uses Draft; Revision Candidates and
Draft-to-Canon promotion require author approval.

## Configuration

The local `PUT /v1/runtime-settings` resource accepts this optional object under
`mo_state` (merge it into the complete resource before PUT):

```json
{
  "memory_lifecycle": {
    "enabled": true,
    "decay_after_turns": 48,
    "forget_after_turns": 240,
    "decay_factor": 0.9,
    "auto_forget": true
  }
}
```

These defaults are configurable, not empirically calibrated universal values.
Both intervals accept integers 1–1,000,000; the finite factor must be strictly
between 0 and 1. Settings are instance-wide and reconciled by the host; there
is no per-Space override. `enabled: false` stops new lifecycle events;
`auto_forget: false` retains decay/archive without physical forgetting for new
events. Accepted events keep their captured settings, including during recovery.

## Clock and protections

Only completed responses with nonempty assistant text, an explicit writable
DMW source, enabled MO State in `closed_autonomous`, and enabled lifecycle
create activity events. Events capture write Space, conversation, character
identity, maintenance user text and effective settings. They are independent
of the model-distillation enable flag and its 12-turn threshold. Failed
generation, response replay, retrieval alone, idle time and manual scans do not
create turns. Prompt changes do not change the assistant identity.

Clocks are partitioned by Space / conversation / character. A record enrolls
through a matching explicit conversation binding or a substantive query or
reference hit; existing character/conversation restrictions are respected.
Unknown legacy records do not acquire an identity simply by being present.
Old timestamps never become accumulated turn ages. Shared records age only
when every enrolled context advances sufficiently; B cannot spend A's retention.

Automatic aging is restricted to non-core `event` records. Character,
relationship, world and current records, importance >= 0.8, long-term relation
or explicit wiki references, current scene/open-thread references, and tags
`commitment`, `promise`, `open`, `pending`, `unresolved` are protected. Tags are
case-insensitive; the engine does not infer unmarked promises from prose.

After 48 subsequent unhit completed turns in every enrolled context, weight
is multiplied by 0.9. Another decay requires another interval. Substantive
hits reset the relevant clock; injection alone does not. Semantic content or
metadata edits reset known clocks conservatively.

Substantive matching of active records is independent of the presentation
budget. A record does not become unused merely because its body did not fit.
Only this identity's scoped hot references count as current reference hits;
other retained references protect their targets without impersonating a hit.
Mentioning an archived record does not refresh it or restore it implicitly.
An explicit owned record policy also enrolls its lifecycle context, so a new
model-authored event does not require the user to repeat it before aging starts.

An eligible active event crossing weight 0.2 on a decay pass is archived.
Forgetting requires archive membership, importance < 0.2, weight < 0.05,
no protections, `auto_forget`, and 240 further unhit completed turns since the
archive/hit baseline in every enrolled context. File/index removal keeps a
minimal tombstone and audit. Wall time is only audit metadata.

## Execution, recovery and controls

Response completion atomically stores the activity event with the assistant
message, ordinary maintenance turn and replay result. Core automatically
processes it under the Space write lock and retries pending work before a
later autonomous response observes sources. A pass processes at most 64 events.

SQLite stores prepared before/after images covering the activity clock,
documents, index, tombstones and audit before file mutation. Restart/cancellation
replays the same plan without aging twice. Unexpected external edits isolate
the Space through the existing memory recovery mechanism. Acknowledgement
occurs only after file commit. Settings changes do not rewrite accepted work.

`POST /v1/memory/maintenance` processes pending activity without invoking a
model or inventing turns. Manual archive moves only the selected document.
The low-level Rust `run_maintenance[_at]` calendar profile remains for explicit
legacy callers/tests; native automatic and HTTP paths no longer use its
seven-day / 180-day rules.

`GET /v1/mo-state/runtime?space_id=...` includes `memory_lifecycle`: live
settings, pending/completed counts and the last completed report. DMW
`clear_memory` clears the activity file and queue. Conversation deletion
preserves memory and pending activity. MOC memory snapshots carry the activity
file; pending SQL work remains local, not a portable evidence chain.

DMW/NSG model calls remain independent default 12-turn batches. The provenance profile adds historical response evidence, scoped scenes,
record eligibility, portable revision history and the configurable 12/2 context
window; see [Profile 1](memory_provenance_runtime.md). These changes preserve
activity counting independently from the distillation threshold.
