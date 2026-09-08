# MOMO Core 1.0 specification index

**Status:** normative document precedence index
**Updated:** 2026-09-05

When documents disagree, the current contracts in this section take
precedence. Historical records explain earlier designs but do not define Core
1.0 behaviour.

## Current contracts

| Area | Current contract | Status |
| --- | --- | --- |
| Product/HTTP boundary | [`http_api_1_0.md`](http_api_1_0.md) | Normative 1.0 |
| Response runtime | [`chat_runtime_v1.md`](chat_runtime_v1.md), `contracts/1.0` | Implemented 1.0 |
| Identity and access vocabulary | [`space_model_1_0.md`](space_model_1_0.md) | Normative 1.0 |
| Structured controls | [`space_model_1_0.md`](space_model_1_0.md), [`control_request.json`](../contracts/1.0/control_request.json), [`control_response.json`](../contracts/1.0/control_response.json) | Implemented 1.0 |
| MOMO Character Card | [`Character_Card_v2.md`](../Character_Card_v2.md) | Implemented normative v2 |
| DMW | [`Dual-Mem_Wiki_v2.md`](../Dual-Mem_Wiki_v2.md) | Implemented normative v2 |
| NSG | [`Narrative_Semantic_Graph_v2.md`](../Narrative_Semantic_Graph_v2.md) | Implemented normative v2 |
| MO State | [`MO_State_v2.md`](../MO_State_v2.md) | Implemented v2 baseline |
| MOC | [`MOMO_Container_v3.md`](../MOMO_Container_v3.md) | Implemented v3 |
| Portable runtime config | [`runtime_config_0_1.md`](runtime_config_0_1.md) | Implemented schema 1 |
| LSB carrier | [`momo_lsb_carrier_v1.md`](momo_lsb_carrier_v1.md) | Implemented carrier v1 |
| Private MOC encryption | [`encryption_profile_0_1.md`](encryption_profile_0_1.md) | Implemented envelope v1 |
| External character compatibility | [`character_card_compatibility.md`](character_card_compatibility.md) | Implemented profile |

The implementation notes in [`memory_nsg_implementation.en.md`](memory_nsg_implementation.en.md)
describe how the current DMW v2 and NSG v2 contracts are realized; they do not
override those specifications.

## Historical and superseded material

- `architecture_0_5.md`, `roadmap_0_5_0.md`, and the 0.5 review/migration notes
  describe the released pre-1.0 boundary.
- `Character_Card_v1.md` is superseded by Character Card v2.
- `Dual-Mem_Wiki_v1.md` is superseded by DMW v2.
- `Narrative_Semantic_Graph_v1.md` is superseded by NSG v2.
- `MO_State_v1.md` is superseded by the MO State v2 autonomous runtime; its
  projector remains the compatible state-projection component.
- `MOMO_Container_v1.md` and `MOMO_Container_v2.md` are unsupported historical
  drafts; Core 1.0 accepts only MOC v3.
- `identity_scope_1_0.md` is a withdrawn design and must not be implemented.

Versioned migration documents describe only movement between their named
revisions. They never override a current contract.

## Non-normative reviews

- [`../benchmarks/morp/README.en.md`](../benchmarks/morp/README.en.md)
  ([简体中文](../benchmarks/morp/README.md)) documents
  MORP-Bench, original ACGN label ablations, scoring, source licenses and the
  offline/opt-in-AI workflow. Benchmark reports are not protocol specifications.
- [`roleplay_implementation_review.en.md`](roleplay_implementation_review.en.md)
  ([简体中文](roleplay_implementation_review.zh-CN.md))
  scores the current roleplay implementation and records known gaps. It does
  not override any contract above.
- [`character_design_mechanism.en.md`](character_design_mechanism.en.md)
  ([简体中文](character_design_mechanism.zh-CN.md)) records non-normative
  experience-grounded authoring and evaluation guidance.

## Design drafts

- [`DYNAMIC_DISPOSITION_MODEL_v1.md`](../DYNAMIC_DISPOSITION_MODEL_v1.md)
  ([简体中文说明](dynamic_disposition_model.zh-CN.md)) defines the proposed DDM
  projection boundary between stable Character Card tendencies, MO State, and
  model-facing expression guidance. It is not implemented by Core 1.0 and does
  not override the current contracts above.
