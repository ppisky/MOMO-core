# MOMO portable runtime configuration v1

**Status:** implemented portable contract
**Updated:** 2026-09-05

This document describes the fields that MOMO Core currently reads from and
writes to `momo.toml`. Host-local provider wiring belongs in the adapter host's
`config.toml`; API keys, provider URLs, listener addresses, executable paths,
and account identifiers must never enter this portable document.

Only the fields shown below are executed by this repository. Portable import
and export preserve safe unknown product sections for another host, but
preservation is not execution: in particular, Core does not read
`[model_use]`, a default `character_id`, Space weights/write targets, chat
concurrency, or retry policy from `momo.toml`. The host resolves those choices
into logical gateway routes and per-request `momo.responses/1.0` fields.

```toml
schema_version = 1

[runtime]
memory_distillation_enabled = true
memory_distill_every_turns = 12
semantic_graph_enabled = true
nsg_govern_every_turns = 12

[mo_state]
profile = "closed_autonomous"
scene_management = true
max_reconcile_steps = 4
max_agent_steps = 8
operation_timeout_ms = 30000
injection_mode = "active"

[request_overrides]
context_window = "ignore"
max_output_tokens = "allow"
sampling = "ignore"
instructions = "allow"
visual_description_prompt = "ignore"
tools = "allow"
allowed_parameters = []

[vision]
enabled = false
prompt = "Describe only visible facts that are relevant to the conversation. Do not infer identity, intent, private attributes, or text that is not legible."

[prompts]
memory_distillation_file = "prompts/dmw_distiller.md"
semantic_graph_governance_file = "prompts/nsg_governor.md"
```

## Request governance

Each override uses one explicit mode:

- `allow`: apply the request value after type, range, and capability checks;
- `ignore`: retain the configured or discovered value and record the ignored
  field in `momo.request_audit`;
- `reject`: reject the whole request when that field is supplied.

Provider parameters are rejected unless their names occur in
`allowed_parameters`. Protocol-owned fields such as `model`, `messages`,
`stream`, `max_tokens`, and `temperature` cannot be placed in that allow-list.

## MO State runtime

`profile = "closed_autonomous"` enables the MO State v2 per-Space manager for
requests whose native `momo.mo_state` switch is true. Core serializes stateful
work in that Memory Space, recovers due maintenance before observing the next
turn, versions DMW/NSG/scene source identities, and publishes a durable state
snapshot. With `scene_management = true`, DMW scene distillation is eligible
after every completed text turn instead of waiting for the ordinary batch
interval.

`profile = "v1_projection"` retains the v1 behaviour: MO State only projects
the retrieved DMW/NSG material into `[STATE_CONTEXT]` and does not create the v2
runtime journal. Deployments migrating from an older release should pin this
profile until they intentionally enable autonomous scene management.

`max_reconcile_steps` accepts 1 through 16, `max_agent_steps` accepts 1 through
32, and `operation_timeout_ms` accepts 1,000 through 300,000. External tool
implementations remain host-owned. In the current 1.0 wire, a continuation is a
new response operation in the same conversation and carries the prior
`function_call` plus its `function_call_output`; Core classifies it as a
`tool_result`. Durable paused-run identity and enforcement of the step/timeout
values are reserved for a later wire generation, so these limits are currently
validated and reported in audit metadata only.

`injection_mode = "shadow"` keeps observation, deterministic projection,
snapshot persistence, and audit active while withholding `[STATE_CONTEXT]`
from the conversation model. In active mode, a projection whose audit is
degraded is also withheld. A degraded retrieval input is withheld separately,
and the audit binds the projection to a SHA-256 of the exact retrieved subset
without exposing memory bodies. This supports safe rollout and causal
evaluation without letting an uncertain state projection influence replies and
later memory maintenance.

## Vision

`vision.enabled = true` permits image-bearing native response requests. Core
first checks the `conversation` route's discovered modalities. If that route
advertises `image`, Core preserves the original image content blocks in the
final roleplay request; it does not call the visual-description adapter or use
the vision prompt. If the conversation route is text-only, Core uses the
governed prompt with the host-local logical route `vision` and converts each
image to bounded text before retrieval and context assembly. Resolved handling
mode, fallback usage, and upstream request IDs are stored with the response
operation so an idempotent retry does not describe the same image again.

An enabled portable switch does not configure a provider or credential. The
adapter host must separately wire the `vision` route. A missing route or a route
that advertises only text is an explicit model-adapter error only when the
conversation route itself is text-only. The prompt is fallback policy, not an
extra instruction for a multimodal conversation model.

## Maintenance prompts

`prompts.memory_distillation_file` and
`prompts.semantic_graph_governance_file` reference the complete system
instructions for Core-owned background maintenance. Paths are relative to the
directory containing `momo.toml`; they must be safe `.md` paths and cannot use
absolute paths, `..`, symlinks that escape the directory, or files larger than
256 KiB. A MOC config module carries the referenced files with `momo.toml`.

The `[prompts]` table may be omitted when the standard files exist at
`prompts/dmw_distiller.md` and `prompts/nsg_governor.md`. Write the table only
to select different safe relative Markdown files. There are no abbreviated
inline prompt fields.

The shipped prompt files combine the complete v1 Distiller rules with the v2
discipline additions. TOML contains references rather than abbreviated inline
prompts. An embedding host that deliberately constructs `MomoConfig::default()`
uses the same full bundled Markdown content.

## Compatibility

- Core accepts schema version 1 only.
- Unknown product sections are preserved by portable import/export, but Core
  does not claim to execute fields it does not own.
- Runtime maintenance intervals must be between 1 and 200 turns.
- MO State limits must be inside the ranges documented above.
- The vision prompt must contain 1 to 65,536 bytes.
- Omitting `[prompts]` selects the two standard relative files; custom
  references must provide both fields.
- Each referenced prompt must be non-empty UTF-8 Markdown no larger than 256 KiB.
- Secret-shaped fields and host-only top-level sections are rejected.

The former schema-v2 document that described `active_model_profile`, embedded
provider URLs, and `[[models]]` was an obsolete pre-0.5 design and is not a
supported migration source.
