# MOMO-RFC-0015: Dynamic Disposition Model (DDM) v1.0.0

**Status:** design draft; not implemented by MOMO Core 1.0

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

DDM SHOULD be implemented inside MO State, immediately after a
version-consistent snapshot has been assembled and before the State Projector
formats model context. It MAY remain a separately named component and contract,
but it MUST NOT become a fifth authoritative narrative store.

The evaluation scope is `(managed_space_id, conversation_id, character_id)`.
The same character can therefore have different effective dispositions in two
conversations without changing the character's base definition.

## 2. Ownership

| Information | Owner | Persistence |
| --- | --- | --- |
| Stable disposition and base activation | Character Card semantics | Portable and author-owned |
| Machine-readable DDM profile | Optional MOC extension | Portable with the host extension |
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
added to the Character Card v2 core metadata. A future implementation SHOULD
use an optional MOC extension such as:

```text
extensions/momo.ddm/
  dispositions.yaml
```

If the extension is absent, Core MUST preserve the existing Markdown-only
behavior and MUST NOT invent numeric parameters from prose during an ordinary
response.

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

dispositions:
  - id: protect_companion
    base_activation: 0.72
    description: Prefer concrete protection over reassurance alone.
    modulation:
      context:
        - id: immediate_danger
          signal: scene.danger
          when: high
          delta: 0.90
        - id: refusal_of_help
          signal: request.help_refused
          when: true
          delta: -0.55
      state:
        - id: exhausted
          signal: physiological_state.exhaustion
          when: high
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
- a source (`scene`, `request`, `dmw`, `nsg`, or a MO State dimension);
- the source revision or event ID;
- an explicit missing-value policy, which defaults to neutral;
- a bounded effect and deterministic conflict order.

Free-form model interpretation MUST NOT silently update numeric state. A host
may use a model to propose signals, but the proposal must pass the existing MO
State event, validation, idempotency, and audit path before it affects DDM.

Canon, safety constraints, explicit consent boundaries, and user agency are
gates, not dispositions. A high effective activation MUST NOT override them.

## 6. From activation to expression

DDM does not directly select dialogue. It compiles active dispositions into a
small set of behavioral cues for the conversation model. The compiler SHOULD:

1. apply hard constraints;
2. compute all effective activations;
3. resolve only explicitly declared conflicts;
4. select at most a configured top `k` dispositions;
5. map activation bands to authored expression guidance;
6. emit source-free guidance to the model and source-rich detail to audit.

Global normalization is NOT RECOMMENDED. Two compatible dispositions may both
be salient. Softmax or winner selection MAY be used only inside an explicitly
mutually exclusive group.

Activation bands SHOULD use hysteresis, for example entering `salient` at
`0.65` and leaving it below `0.55`, so minor input changes do not make a
character oscillate between styles on adjacent turns.

The initial Core integration SHOULD append a `## Effective dispositions`
subsection inside the existing `[STATE_CONTEXT]` block. A future wire revision
may add a separate `[DISPOSITION_CONTEXT]` section if it receives its own token
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

Effective dispositions are a rebuildable projection. A snapshot SHOULD record:

```yaml
effective_dispositions:
  - id: protect_companion
    base_activation: 0.72
    context_delta: 0.90
    state_delta: -0.35
    effective_activation: 0.82
    band: salient
    matched_rule_ids: [immediate_danger, exhausted]
    evidence_ids: [scene:7, event:01J...]
ddm_profile_revision: 1
source_fingerprint: sha256:...
```

The model-facing context MUST NOT expose private audit identifiers, numeric
reasoning traces, or hidden control-plane data. The audit MUST preserve enough
information for deterministic replay and diagnosis.

## 8. Required invariants and tests

An implementation MUST verify at least:

- neutral modulation reproduces the base activation;
- a missing signal is neutral rather than zero;
- activation remains within `[0, 1]`;
- the same snapshot and profile produce the same result;
- one Space or conversation cannot leak state into another;
- duplicate events do not apply modulation twice;
- hard constraints dominate disposition activation;
- expected positive and negative effects are monotonic;
- hysteresis prevents threshold oscillation;
- projection never writes back into the base profile.

MORP-Bench MAY evaluate DDM with counterfactual pairs: keep the character,
history, request, and all other signals fixed, then change one context or state
signal and score the expected direction of behavioral change. It SHOULD judge
mechanism consistency and boundary preservation, not exact wording.

## 9. Adoption plan

DDM can be added without destabilizing the Core 1.0 contracts:

1. keep this document and the profile schema experimental;
2. add a pure evaluator with deterministic unit tests;
3. attach its output to the existing MO State snapshot and `state` context;
4. add opt-in MORP counterfactual cases;
5. stabilize a portable extension only after profile round-trip and replay are
   proven.

This order reuses the current Character Card, DMW, NSG, MO State, and context
boundaries. No new authoritative storage layer or immediate HTTP wire change is
required.
