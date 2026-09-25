# MOMO runtime settings API v1

**Status:** implemented local administration contract
**Updated:** 2026-09-23

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
