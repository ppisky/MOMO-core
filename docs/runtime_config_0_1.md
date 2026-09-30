# MOMO runtime settings API v1

**Status:** implemented local administration contract
**Updated:** 2026-09-30

MOMO Core does not read `momo.toml`, `config.toml`, prompt paths, or a
`MOMO_CONFIG_PATH` environment variable. A host owns its user-facing
configuration format and translates the relevant values into JSON requests.
For mobot the boundary is:

```text
momo.toml -> mobot parsing and validation -> PUT /v1/runtime-settings -> MOMO
```

The filename and TOML schema are therefore mobot concerns, not MOMO protocol.
MOMO accepts only the typed runtime resource below.

## API

```http
GET /v1/runtime-settings
PUT /v1/runtime-settings
Content-Type: application/json
```

`PUT` replaces the complete runtime resource for subsequent operations. Fields
omitted from the JSON payload take their documented defaults; unknown fields
are rejected. The settings are process state. A host should reconcile them
after starting or connecting to Core instead of expecting Core to discover a
file.

```json
{
  "schema_version": 1,
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
recovery. There is no per-Space setting override. Idle contexts do not age.
See the [implemented lifecycle profile](memory_lifecycle_runtime.md) for
enrollment, protections, legacy migration and management behavior.

See the [code-verified maintenance guide](memory_maintenance_current_behavior.zh-CN.md)
for triggers, manual endpoints, deletion behavior and exact lifecycle checks.

## Prompt boundary

Prompt bodies are not runtime settings. They use the separate Prompt Spaces
resource:

```http
GET /v1/prompt-spaces
PUT /v1/prompt-spaces/vision_fallback
DELETE /v1/prompt-spaces/vision_fallback
```

All five Core-consumed slots can be replaced. Arbitrary names cannot be created
because Core would have no consumer for them. See
[MOMO Prompt Spaces](maintenance_prompts.en.md).

## Artifact boundary

MOC carries characters, conversations, memory, semantic graph data, and
explicit host extension modules. It no longer carries or applies a
`config/momo.toml` module. Runtime settings and Prompt Space values move through
their HTTP resources, not portable archives.

See [provenance Profile 1](memory_provenance_runtime.md) for history-window bounds, record policies, default-assistant mappings and source controls.
