# MOMO runtime settings API v1

**Status:** implemented local administration contract
**Updated:** 2026-10-02

MOMO Core does not read `momo.toml`, `config.toml`, prompt paths, or a
`MOMO_CONFIG_PATH` environment variable. A host owns its user-facing
configuration format and translates the relevant values into typed Rust API
arguments or the equivalent HTTP JSON requests.
The configuration boundary is:

```text
host configuration -> host parsing and validation -> typed Rust API or HTTP PUT -> MOMO
```

The host's filename and configuration schema are outside the MOMO protocol.
MOMO accepts only the typed runtime resource below.

A host is the application that integrates and configures Core. It may be a
desktop client, a backend service or a local adapter; it is not a particular
product. The host chooses desired policy, authorized Spaces and model access.
Core validates inputs and owns its domain rules, orchestration, persistence
and recovery.

## API

```http
GET /v1/runtime-settings
PUT /v1/runtime-settings
Content-Type: application/json
```

`PUT` replaces the complete resource; omitted fields use built-in defaults and
unknown fields are rejected. The instance policy and sparse Space overrides are
atomically persisted to `<data_dir>/runtime-settings.json` before publication.
Initialization validates and restores that file; a malformed file fails startup.
Core still does not discover or reload a host-authored configuration file.

```http
PATCH /v1/runtime-settings
GET|PUT|PATCH|DELETE /v1/runtime-settings/{space_id}
GET /v1/runtime-settings/metadata?space_id=<optional UUID>
GET /v1/runtime-settings/description
```

`PATCH` uses JSON Merge Patch. Instance null fields reset to typed defaults.
Space PATCH stores only the supplied overrides; null removes an override and
restores inheritance. Space PUT stores a complete policy; DELETE removes it.
Unspecified Space fields inherit subsequent instance updates. At most 1024
Space overrides and a 4 MiB persisted policy file are accepted.

GET and successful mutations return an `ETag` containing the instance-wide
monotonic revision. Supply `If-Match: "<revision>"` for compare-and-swap; a stale
revision returns HTTP 409 with `revision_conflict`, without writing or publishing.
Omitting If-Match permits a serialized unconditional update. A global update
must also leave all existing Space overrides valid.

`metadata` returns revision, effective settings, resource source and a dotted
field-to-source map (`instance` or `space_override`). `instance` includes built-in
defaults and the saved instance policy; it does not claim host-file provenance.
`description` enumerates every typed field, default, type, enum and numeric bound.
Responses select policy by `personal_space_id`; background maintenance selects
its write Space policy. A response captures its policy once before orchestration.
Native hosts use the corresponding `MomoRuntime` resource/replace/patch/reset APIs.

## Configuration responsibilities

| Kind | Owner and lifetime | Current entry point |
| --- | --- | --- |
| Data directory, listener, model gateway and credentials | Deployment host/server startup | Startup arguments/environment; excluded from portable assets |
| Override policy, history and automatic maintenance limits | Persisted instance policy with sparse Space overrides | Typed settings API / `GET/PUT/PATCH /v1/runtime-settings` |
| Prompt content | Runtime-owned fixed registry, persistent when loaded by `MomoRuntime` | `PromptSpaces::get/replace/reset` / Prompt Spaces HTTP API |
| Request-specific instructions, sampling and limits | Caller proposal, governed for that request | Typed response request / `momo.responses/1.0` |
| Character definition and character-owned DDM profile | Business asset with explicit identity | Character/import APIs and their documented MOC modules |
| Capabilities, vector indexes and recovery metadata | Runtime observation, disposable cache or durable recovery state | Their respective diagnostic/persistence paths; not user configuration |

`momo-config` provides TOML document utilities for portable assets; it is not a
runtime-settings discovery service. Business assets containing configurable
values may travel in MOC under their own format contracts. Runtime policy,
deployment secrets and Prompt Space overrides do not become character data
merely because they are all called "configuration".

## Effective values and update limits

The policy order is built-in defaults, restored/replaced instance policy,
Space overrides, then request proposals filtered by `request_overrides`.
Each response holds one policy snapshot. Prompt lookups remain independent;
there is no atomic combined settings/prompts transaction or host-file watcher.
Accepted maintenance batches retain captured inputs for deterministic recovery.

A complete replacement example:

```json
{
  "schema_version": 1,
  "history_window": { "enabled": true, "trigger_turns": 12, "evict_turns": 2 },
  "request_overrides": {
    "context_window": "ignore",
    "max_output_tokens": "allow",
    "sampling": "ignore",
    "instructions": "allow",
    "visual_description_prompt": "ignore",
    "tools": "allow",
    "allowed_parameters": []
  },
  "runtime": {
    "memory_distillation_enabled": true,
    "memory_distill_every_turns": 12,
    "semantic_graph_enabled": true,
    "nsg_govern_every_turns": 12
  },
  "mo_state": {
    "profile": "closed_autonomous",
    "scene_management": true,
    "max_reconcile_steps": 4,
    "max_agent_steps": 8,
    "operation_timeout_ms": 30000,
    "injection_mode": "active",
    "memory_lifecycle": {
      "enabled": true,
      "decay_after_turns": 48,
      "forget_after_turns": 240,
      "decay_factor": 0.9,
      "auto_forget": true
    },
    "ddm": { "enabled": false }
  },
  "roleplay": { "enabled": true },
  "vision": { "enabled": true }
}
```

Maintenance intervals must be between 1 and 200 turns. MO State limits use
1–16 reconcile steps, 1–32 agent steps, and a 1,000–300,000 ms timeout.
Provider parameters remain denied unless explicitly listed in
`allowed_parameters`; protocol-owned fields cannot be allow-listed.

## Maintenance counting and lifecycle limits

Each maintenance turn is one queued completed response with user input and
assistant output text, not one chat message. The default 12 means 12 such
pairs. Each lane consumes the oldest unprocessed turns in its write Space:
1–12, then 13–24 after acknowledgement, with independent DMW/NSG completion.
The same setting is the automatic threshold and new-batch size, independent
of foreground history eviction. MO State projects automatically per enabled
response; it does not wait for either maintenance threshold. `history_window`
defaults to enabled with `trigger_turns: 12` and `evict_turns: 2`; it removes
complete old turns only from the prompt copy, then applies the token budget.
Maintenance batches also group by captured identity; parallel conversations
and historical character identities do not share a batch.

The enable flags gate automatic scheduling/recovery, not the collection of
pending turns or explicit management drains. A response's selected write
source determines whether it records pending work. An explicit drain can
process a partial tail; an already staged batch keeps its captured turn set.

The implemented `mo_state.memory_lifecycle` profile advances on completed
responses per Space/conversation/character, not elapsed days. Default decay is
48 unhit turns with factor 0.9; eligible archived events need 240 subsequent
unhit turns plus type, importance, weight and reference checks before forgetting.
Both turn intervals accept 1–1,000,000, and the finite factor is strictly between
0 and 1. `enabled` controls new activity events; `auto_forget` controls physical
forgetting. Accepted events retain their captured settings for deterministic
recovery. Sparse per-Space overrides use the same settings resource. Idle contexts do not age.
See the [implemented lifecycle profile](memory_lifecycle_runtime.md) for
enrollment, protections, legacy migration and management behavior.

See the [code-verified maintenance guide](memory_maintenance_current_behavior.zh-CN.md)
for triggers, manual endpoints, deletion behavior and exact lifecycle checks.

## Enforced MO State limits and automatic DDM

`max_reconcile_steps` bounds model proposal/validation attempts for each DDM
profile generation and each new maintenance patch (including truncated-output
repairs). It is not a count of low-level filesystem or SQL operations.
`max_agent_steps` counts the initial response plus committed tool continuation
responses since the latest ordinary user input. A continuation beyond the cap
is rejected before generation; completed request-ID replay does not consume a
step. Tool continuations require an existing conversation. Hosts still execute
tools; this does not add a paused-run or requires_action protocol.
`operation_timeout_ms` bounds foreground MO State work and each maintenance
batch, including model generation and waits. Timeout is HTTP 504. Already
admitted durable commits retain cancellation ownership and may finish after
the caller times out; retries recover their journal rather than undoing them.

Enabling `mo_state.ddm.enabled` no longer requires a user-authored YAML file.
When no author override exists, MO State proposes a profile from the captured
character name/description through the existing `memory_distillation` model
route and `ddm_profile_generation` Prompt Space. User descriptions and current
conversation text are excluded. Typed validation precedes publication; failed
proposals remain explicit degraded audit entries without installing a profile.
The per-managed-Space/character cache is fingerprinted by character content
and prompt revision. Changes regenerate it; identical requests/restarts reuse it.
Generated profiles are runtime derivatives, do not edit Character Card, and
are not exported as author-owned MOC assets. Generation usage/request IDs are
recorded per call in state audit and included in response usage totals. Author overrides remain optional and take
precedence.

```http
GET|DELETE /v1/mo-state/ddm-profile?space_id=<managed UUID>&character_id=<UUID>
GET|PUT|DELETE /v1/mo-state/contract?space_id=<UUID>
```

The DDM cache GET is read-only; DELETE permits regeneration on the next enabled
response. State-contract PUT accepts `{ "content": "<YAML>" }`, validates at
most 64 KiB, and commits through the owning Space journal. GET returns content,
source and revision. PUT/DELETE accept that revision in If-Match; stale writes
are rejected. DELETE restores the built-in contract. Overrides merge into
reviewed defaults and cannot silently disable protected built-in dimensions.

## Prompt boundary

Prompt bodies are not runtime settings. They use the separate Prompt Spaces
resource:

```http
GET /v1/prompt-spaces
PUT /v1/prompt-spaces/vision_fallback
DELETE /v1/prompt-spaces/vision_fallback
```

All six Core-consumed slots can be replaced. Arbitrary names cannot be created
because Core would have no consumer for them. See
[MOMO Prompt Spaces](maintenance_prompts.en.md).

## Artifact boundary

MOC carries characters, conversations, memory, semantic graph data, and
explicit host extension modules. It no longer carries or applies a
`config/momo.toml` module. Runtime settings and Prompt Space values move through
their typed Rust or HTTP resources, not portable archives.

See [provenance Profile 1](memory_provenance_runtime.md) for history-window bounds, record policies, default-assistant mappings and source controls.
