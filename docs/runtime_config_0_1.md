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

[mo_state.ddm]
enabled = false

[roleplay]
enabled = true

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

### Experimental DDM projection

`mo_state.ddm.enabled` is a deployment-wide opt-in. Per-character YAML profiles
are author-owned character data, not runtime file paths: Core manages them at
`GET`, `PUT`, and `DELETE /v1/characters/{id}/ddm-profile` and transports them
inside that character's MOC module as
`extensions/momo-ddm/profile.yaml`. Character Card v2 core metadata is
unchanged. Missing profiles are neutral, and `momo.toml` cannot map characters
to host files.

The evaluator accepts only the closed, typed signal families documented in the
[experimental DDM specification](../Dynamic_Disposition_Model_v1.md): DMW
kind/tag, NSG node, active MO State dimension, governed scene status/
participants/source references, and validated request event/image facts. It
never infers numeric dispositions from conversation prose. Previous activation
bands persist per managed Space, conversation, and character to provide
hysteresis. A profile revision or deterministic typed-profile fingerprint
change resets that history, and deleting the profile clears it.

Model-facing context receives only authored expression cues and hard
constraints. Hard constraints have non-removable state-budget priority;
optional disposition cues are trimmed first, and a constraint overflow degrades
the projection instead of being reported as active. Numeric activation, matched
rules, evidence IDs, suppressed dispositions, previous/next bands, hysteresis
decisions, a deterministic profile fingerprint, and a normalized source
fingerprint remain in state audit. See
[`ddm_implementation_status.md`](ddm_implementation_status.md).

## Role-play runtime

`roleplay.enabled = true` injects a product-owned Roleplay Director into every
character-bound conversation-model request. It keeps performance inside the fiction, preserves
character voice, emotional and relationship continuity, physical constraints,
user agency, bounded initiative, and the character's knowledge boundary. The
director is separate from the author-owned Character Card: the card defines who
the character is; the director defines how the runtime performs that character.
Core renders this director after character, DMW, MO State, and NSG evidence so
its response-time constraints remain the final section of the system message.

Set `enabled = false` only when a host intentionally supplies a different
performance layer. Memory and MO State remain evidence sources and are not a
substitute for this runtime direction.

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

## Product prompts are not runtime configuration

The DMW Distiller, NSG Governor, and Roleplay Director are fixed Markdown source
files under `crates/momo-core/src/product_prompts/`. `momo_core` embeds all three
with `include_str!` at compile time. They are not deployment files, selected by
`momo.toml`, accepted from a request, reloaded while the process is running, or
carried by MOC import/export.

A `[prompts]` table is explicitly rejected because product prompts are not
portable configuration.
Compilation fails when a required prompt source is absent. See
[MOMO compiled product prompts](maintenance_prompts.en.md).

## Compatibility

- Core accepts schema version 1 only.
- Unknown product sections are preserved by portable import/export, but Core
  does not claim to execute fields it does not own.
- Runtime maintenance intervals must be between 1 and 200 turns.
- MO State limits must be inside the ranges documented above.
- The vision prompt must contain 1 to 65,536 bytes.
- Product prompts are not fields in `momo.toml` or payloads in a MOC config module.
- Secret-shaped fields and host-only top-level sections are rejected.

The former schema-v2 document that described `active_model_profile`, embedded
provider URLs, and `[[models]]` was an obsolete pre-0.5 design and is not a
supported migration source.
