# MOMO Core 1.0.0-rc.2

`v1.0.0-rc.2` focuses the release candidate on role-playing quality while
preserving the frozen `momo.responses/1.0` wire contract.

## Highlights

- Adds a default, portable Roleplay Director to the real response path. It
  preserves character voice, emotional and relationship continuity, bounded
  knowledge, scene embodiment, initiative, and user agency.
- Replaces MORP's default mixed memory benchmark with MORP 1.0: 64 bilingual,
  counterfactual role-playing cases across eight equally weighted dimensions.
- Accepts natural role-play prose from candidates instead of requiring a JSON
  answer envelope.
- Uses one identity-bound, evidence-grounded reviewer record for subjective
  scoring. Two external model judges remain an optional compatibility path,
  not a release dependency.
- Makes the three-arm core matrix the scenario default:
  `basic_context`, `dmw_nsg`, and `all_enabled`.
- Reduces the recommended provider topology to one Qwen deployment shared by
  conversation, DMW distillation, and NSG governance, plus one embedding
  deployment.

## Evaluation evidence

The complete MORP 1.0 run scored **88.87/100**. It covered all 64 planned
Chinese and English cases; 61 candidate responses succeeded and three terminal
gateway failures were retained as zeroes without retry. Relationship dynamics
scored 98.44, while narrative coherence (73.44) and world embodiment (76.56)
were the main weaknesses.

This is one public, synthetic, non-human-calibrated run. It supports regression
and release-candidate testing, not a universal role-playing quality claim.

The release revision passes 225 Rust tests, 98 Python benchmark tests, 21
offline runtime-contract checks, strict Clippy, rustfmt, and rustdoc generation.

## Compatibility

- Workspace crate versions remain `1.0.0` and `publish = false`.
- The historical `v1.0.0` and `v1.0.0-rc.1` tags are not moved.
- The stable `v1.0.0` publication gate remains open for broader credentialed,
  human-calibrated, and load evidence.
