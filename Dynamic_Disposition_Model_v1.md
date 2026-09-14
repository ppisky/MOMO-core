# MOMO-RFC-0015: Dynamic Disposition Model (DDM) v1.0.0

**Status:** experimental specification; not a stable Core 1.0 contract

**Language:** English

**Scope:** per-character, per-conversation runtime disposition projection

## Abstract

The Dynamic Disposition Model turns relatively stable character tendencies
into the tendencies that matter in the current moment:

```text
stable disposition
  + current context
  + internal state
  -> effective disposition
  -> expression guidance
  -> model-generated behavior
```

DDM is not another memory store and does not replace Character Card, DMW, NSG,
or MO State. It is a deterministic projection component owned by the MO State
runtime. Character Card owns the stable disposition; DMW and NSG provide
source-attributed evidence and constraints; MO State owns the current scene and
state snapshot; DDM computes a bounded, auditable view for the current turn.

## 1. Architectural placement

```text
Character Card: stable dispositions and expression vocabulary
                         \
DMW: events and relationship evidence --------------------\
NSG: world facts, Canon, and rules -------------------------+--> consistent MO State snapshot
Scene/request: current situational signals -----------------/              |
MO State: current internal-state signals ------------------/               v
                                                        DDM evaluator
                                                             |
                                                   effective dispositions
                                                             |
                                                     State Projector
                                                             |
                                                    context assembly
                                                             |
                                                   conversation model
                                                             |
                                                         behavior
```

DDM SHOULD be implemented inside MO State, after snapshot inputs have been
assembled and before the State Projector formats model context. It MAY remain a
separately named component and contract, but it MUST NOT become a fifth
authoritative narrative store.

The evaluation scope is `(managed_space_id, conversation_id, character_id)`.
The same character can therefore have different effective dispositions in two
conversations without changing the character's base definition.

## 2. Ownership

| Information | Owner | Persistence |
| --- | --- | --- |
| Stable disposition and base activation | Character Card semantics | Portable and author-owned |
| Machine-readable DDM profile | Character-author-owned extension | MOC character asset `extensions/momo-ddm/profile.yaml`; management API outside Character Card v2 metadata |
| Shared events and relationship evidence | DMW | Persistent evidence |
| World rules and Canon | NSG | Persistent governed facts |
| Current scene and internal state | MO State snapshot | Rebuildable runtime projection |
| Effective disposition | DDM within MO State | Snapshot/audit only |
| Natural-language behavior | Conversation model | Conversation history/event flow |

The DDM evaluator MUST NOT write an effective activation back into the
Character Card. A generated response MUST NOT change a base disposition merely
because it expressed that disposition. Durable change requires a separately
authorized character-editing workflow.

Character Card v2 remains valid without DDM. Authors SHOULD still describe
behavioral tendencies in `character.md`. Structured parameters MUST NOT be
added to the Character Card v2 core metadata. The optional machine-readable
profile is instead a character-owned MOC extension at the fixed path
`extensions/momo-ddm/profile.yaml`. Core management transports MAY expose that
asset directly, but MUST validate its `character_id` against the managed
character and its owner Space. `momo.toml` contains only the deployment enable
switch; it MUST NOT map characters to host filesystem paths. DDM does not
authorize a new Space or authoritative storage layer.

If no profile is configured for the active character, Core MUST preserve the
existing Markdown-only behavior and MUST NOT invent numeric parameters from
prose during an ordinary response.

## 3. Activation model

For disposition `i` at time `t`, the simple multiplicative form is:

```text
EffectiveWeight_i(t) = BaseWeight_i * Context_i(t) * State_i(t)
```

DDM retains this as the **multiplicative profile**, with two clarifications:

- `Context_i(t)` and `State_i(t)` are multipliers whose neutral value is `1`,
  not scores whose neutral value is `0`;
- missing or unknown input is neutral and MUST NOT erase a disposition.

The bounded form is:

```text
E_i(t) = clamp(B_i * M_context_i(t) * M_state_i(t), 0, 1)
```

Multipliers MAY both suppress and amplify, for example within `[0.25, 2.0]`.
If every factor were limited to `[0, 1]`, the model could only suppress a base
disposition and one zero would collapse the result. Authors SHOULD therefore
avoid treating absence of evidence as a zero multiplier.

The recommended **logit-additive profile** is more stable:

```text
E_i(t) = sigmoid(
  logit(clamp(B_i, epsilon, 1 - epsilon))
  + delta_context_i(t)
  + delta_state_i(t)
)
```

Here, `B_i` and `E_i(t)` are bounded to `[0, 1]`; matched modulation rules
contribute positive or negative deltas; and an unknown signal contributes zero.
Relationship evidence is a context signal rather than an implicit global
friendliness score.

Wire formats SHOULD use `base_activation` and `effective_activation`, not the
unqualified name `weight`, because DMW already uses `weight` for retrieval and
state-signal importance.

## 4. Profile example

```yaml
schema: momo.ddm/1
character_id: 018f0000-0000-7000-8000-000000000001
revision: 1
profile: logit_additive

selection:
  top_k: 3
  salient_threshold: 0.65
  dominant_threshold: 0.85
  hysteresis_margin: 0.05

dispositions:
  - id: protect_companion
    base_activation: 0.72
    exclusive_group: immediate_response
    description: Prefer concrete protection over reassurance alone.
    modulation:
      context:
        - id: immediate_danger
          signal: dmw.tag.danger
          when: true
          missing: neutral
          delta: 0.90
        - id: refusal_of_help
          signal: nsg.node.help_refused
          when: true
          delta: -0.55
      state:
        - id: exhausted
          signal: state.dimension.physiological_state
          when: true
          delta: -0.35
    expression:
      latent: Keep concern implicit unless it becomes relevant.
      salient: Offer one concrete form of help and explain the risk briefly.
      dominant: Prioritize immediate safety while preserving the user's agency.
    constraints:
      - Never decide the companion's voluntary actions for them.
```

Numbers are authoring controls, not universal psychological measurements.
Profiles MAY expose qualitative values such as `low`, `medium`, and `high` and
compile them to documented defaults. Exact values MUST NOT imply scientific
precision or comparability between unrelated characters.

## 5. Signal contract

Every modulation rule MUST reference a governed signal with:

- a stable signal ID and type;
- a source (`dmw`, `nsg`, or an active MO State dimension in the current
  v1 profile);
- the source revision or event ID;
- an explicit missing-value policy, which defaults to neutral;
- a bounded effect and deterministic conflict order.

The v1 signal vocabulary is intentionally closed:

- `dmw.tag.<tag>` and `dmw.kind.<kind>` are booleans derived from the bound DMW
  retrieval snapshot;
- `nsg.node.<node-id>` is a boolean derived from the bound NSG snapshot;
- `state.dimension.<dimension>` is a boolean for an active MO State dimension;
- `scene.status` is one of `inactive`, `active`, `transitioning`, or `closed`;
- `scene.participant.<id>` and `scene.source_ref.<id>` are booleans from the
  observed governed scene snapshot;
- `request.event_type` is `user_message` or `tool_result`;
- `request.has_image` is a boolean from validated request input.

Identifiers following a dotted family use only lowercase ASCII letters, digits,
`_`, `-`, `.`, and `:`. Unknown families, invalid enum values, and type mismatches MUST be
rejected during profile validation. `missing` currently accepts only `neutral`;
future policies require a schema revision.

Free-form model interpretation MUST NOT silently update numeric state. A host
may use a model to propose signals, but the proposal must pass the existing MO
State event, validation, idempotency, and audit path before it affects DDM.

Canon, safety constraints, explicit consent boundaries, and user agency are
gates, not dispositions. A high effective activation MUST NOT override them.

## 6. From activation to expression

DDM does not directly select dialogue. It compiles active dispositions into a
small set of behavioral cues for the conversation model. The compiler SHOULD:

1. apply authored hard constraints;
2. compute all effective activations;
3. resolve only explicitly declared conflicts;
4. choose the highest activation inside each explicitly named
   `exclusive_group`, breaking ties by disposition ID, then select at most the
   configured top `k` dispositions;
5. map activation bands to authored expression guidance;
6. emit source-free guidance to the model and source-rich detail to audit.

Global normalization is NOT RECOMMENDED. Two compatible dispositions may both be
salient. Softmax or winner selection MAY be used only inside an explicitly
mutually exclusive group.

A conforming evaluator MUST define band thresholds and hysteresis so small input
changes cannot cause uncontrolled band oscillation. The previous bands MUST be
persisted at `(managed_space_id, conversation_id, character_id)`, invalidated by
a profile revision change, and updated atomically with the published MO State
snapshot. Promotion of this specification from experimental still requires
broader interoperability and behavior evidence; persisted hysteresis alone does
not make it stable.

An initial Core integration SHOULD append a `## Effective dispositions`
subsection inside the existing `[STATE_CONTEXT]` block. A future wire revision
MAY add a separate `[DISPOSITION_CONTEXT]` section if it receives its own token
budget and context audit entry.

Example model-facing output:

```markdown
## Effective dispositions
- Salient — protective care: offer concrete help, but preserve the user's choice.
- Latent — public self-protection: keep vulnerable feelings understated in this scene.
```

The model remains responsible for wording, action selection, and natural
variation. DDM constrains likely expression; it does not claim to simulate a
complete mind or deterministically predict behavior.

## 7. Snapshot and audit

Effective dispositions are a rebuildable projection. A snapshot SHOULD record
audit data equivalent to:

```yaml
effective_dispositions:
  - id: protect_companion
    base_activation: 0.72
    context_delta: 0.90
    state_delta: -0.35
    effective_activation: 0.82
    band: salient
    matched_rule_ids: [immediate_danger, exhausted]
    evidence_ids: [event:01J...]
ddm_profile_revision: 1
previous_bands: {protect_companion: latent}
next_bands: {protect_companion: salient}
hysteresis_applied: []
suppressed_disposition_ids: []
source_fingerprint: sha256:...
```

The model-facing context MUST NOT expose private audit identifiers, numeric
reasoning traces, or hidden control-plane data. The audit MUST preserve enough
information for deterministic replay and diagnosis. `source_fingerprint` MUST
bind the normalized signal snapshot and the previous-band map used by the
evaluation; the MO State source-version audit separately binds retrieved DMW,
NSG, and scene bodies under one ordered lock window.

## 8. Required invariants and tests

A conforming implementation MUST verify:

- neutral modulation reproduces the base activation;
- a missing signal is neutral rather than zero;
- activation remains within `[0, 1]`;
- the same snapshot and profile produce the same result;
- one Space or conversation cannot leak state into another;
- duplicate events do not apply modulation twice;
- hard constraints dominate disposition activation;
- expected positive and negative effects are monotonic;
- projection never writes back into the base profile.

Conformance tests MUST also cover persisted hysteresis, profile-revision reset,
closed scene/request signal typing, deterministic exclusive-group selection,
top-k suppression, and atomic publication of the snapshot and next-band state.

MORP-Bench MAY evaluate DDM with counterfactual pairs: keep the character,
history, request, and all other signals fixed, then change one context or state
signal and score the expected direction of behavioral change. It SHOULD judge
mechanism consistency and boundary preservation, not exact wording.

## 9. Conformance and adoption

This document defines the proposed DDM behavior and ownership boundary. It does
not claim that the repository, a release, or a branch currently conforms.
Implementation progress, test evidence, source-control state, and known gaps
belong in the separate non-normative
[`docs/ddm_implementation_status.md`](docs/ddm_implementation_status.md).

Adopting DDM does not require a new authoritative storage layer or a change to
the frozen response wire. The fixed MOC extension and management endpoint are
character-management surfaces; Character Card v2 core metadata is unchanged.
Any future profile schema or stable-status promotion requires its own reviewed
contract.
