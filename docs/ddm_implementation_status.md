# DDM implementation status

[简体中文](ddm_implementation_status.zh-CN.md)

**Status date:** 2026-10-02

**Document role:** non-normative implementation report

**Specification:** [`../Dynamic_Disposition_Model_v1.md`](../Dynamic_Disposition_Model_v1.md)

MOMO Core 1.0.0 contains a complete opt-in implementation of the
experimental `momo.ddm/1` projection contract. “Complete” here means the
repository implements the contract and its deterministic conformance cases; it
does not promote the experimental specification to a stable compatibility
surface or claim provider-level behavior quality.

## Implemented boundary

| Area | Current implementation |
| --- | --- |
| Ownership and transport | MO State owns generated per-managed-Space/character caches; optional author overrides use MOC path `extensions/momo-ddm/profile.yaml` and `GET`, `PUT`, `DELETE /v1/characters/{id}/ddm-profile` |
| Runtime enablement | Instance/Space opt-in `mo_state.ddm.enabled`; no character-to-filesystem map and no DDM Space |
| Profile generation | Character name/description and prompt revision determine the cache; user descriptions are excluded; validated bounded proposals use `memory_distillation`; cache supports read/reset and restart reuse |
| Evaluation | Deterministic logit-additive and multiplicative profiles with bounded finite aggregate effects, exact neutral boundaries, and neutral missing input |
| Governed inputs | Closed, typed DMW, NSG, MO State, scene, and request signal families |
| Selection | Authored hard constraints with non-removable budget priority, deterministic exclusive groups, top-k selection, and latent/salient/dominant expression bands |
| Hysteresis | Previous bands persisted per `(managed_space_id, conversation_id, character_id)` and reused only when both profile revision and deterministic typed-profile fingerprint match; profile deletion clears the history |
| Atomicity | Retrieved bodies, DMW/NSG/scene fingerprints, and observations are captured under one ordered source-lock window; the next DDM bands and their profile identity publish in the same SQLite transaction as the MO State snapshot, with audit/update consistency validation |
| Model boundary | Only validated profile expression cues and hard constraints enter `[STATE_CONTEXT]`; activations, matched rules, evidence IDs, suppression, bands, and fingerprints remain in audit |
| Replay | Request-ID replay reuses the published snapshot and does not evaluate or apply modulation twice |

Character Card v2 core metadata is unchanged. Import and export treat the DDM
profile as an optional character-owned extension, so older consumers can ignore
it without misreading the base card.

## Verification evidence

Rust tests cover profile parsing and rejection, signed persistence revision
bounds, exact neutral endpoints, finite multiplicative aggregation,
monotonic positive and negative modulation, deterministic replay, source
fingerprints, typed scene/request signals, exclusive conflicts, top-k,
persisted and cross-band hysteresis, profile-identity reset, hard-constraint
budget priority, cross-scope isolation, and atomic snapshot/band publication.
The server contract test additionally verifies profile management,
actual cue injection into the conversation-model request, durable next-band
state, restart replay, and unchanged `momo.responses/1.0` output semantics.

The rc.3 mixed-context eight-case A/B run exercised real retrieval and MO State
instead of the earlier disabled short smoke path. It scored 84.375 for both the
direct and MOMO arms (three wins each and two ties). That is useful execution
evidence, but the sample is too small and balanced to establish a behavior
benefit. No DDM-specific credentialed counterfactual provider study is claimed.

## Experimental limitations

- `momo.ddm/1` remains experimental and may change before a stable DDM contract.
- The management endpoint is additive and is not part of the frozen
  `momo.responses/1.0` response wire.
- Behavior quality still depends on profile proposals/overrides and the conversation
  model. Deterministic conformance proves mechanism behavior, not universal
  psychological validity or response quality.
- Stable 1.0 release gates, including broader credentialed image and long-run
  evidence, remain tracked separately in [`roadmap_1_0_0.md`](roadmap_1_0_0.md).


The managed generation workflow is verified with mock gateways, including restart
reuse, character/prompt invalidation, user-description exclusion, invalid proposal
degradation and timeout. See [runtime settings](runtime_config_0_1.md); no new
real-model quality result is claimed.
