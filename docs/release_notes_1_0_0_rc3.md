# MOMO Core 1.0.0-rc.3

`v1.0.0-rc.3` keeps the frozen `momo.responses/1.0` wire and completes the
opt-in implementation of the experimental Dynamic Disposition Model (DDM)
inside MO State.

## Highlights

- Implements both `logit_additive` and `multiplicative` `momo.ddm/1` profiles,
  closed typed DMW/NSG/state/scene/request signals, neutral missing input,
  deterministic exclusive groups, top-k selection, hard constraints, and
  latent/salient/dominant expression bands.
- Persists hysteresis bands per managed Space, conversation, and character;
  profile revision changes reset prior bands. The next bands and MO State
  snapshot publish atomically.
- Binds retrieved bodies and their DMW/NSG/scene source identities in one
  ordered lock window. DDM audit also fingerprints the exact normalized signals
  and previous bands used by the evaluator.
- Adds character-owned DDM profile management at `GET`, `PUT`, and `DELETE
  /v1/characters/{id}/ddm-profile`, plus MOC transport at the fixed character
  extension path `extensions/momo-ddm/profile.yaml`. Character Card v2 core
  metadata and the response wire are unchanged.
- Sends only authored expression cues and hard constraints to the conversation
  model. Activations, evidence, matched rules, suppressed dispositions,
  hysteresis decisions, and fingerprints remain in state audit.
- Removes benchmark-derived response lengths, patch counts, and fixed
  maintenance output-token caps from product defaults. Maintenance follows the
  selected route's advertised output capability.
- Removes language guessing as a hidden runtime rejection rule while retaining
  authored guidance to preserve source language and entity spelling.
- Tracks the complete DMW Distiller, NSG Governor, and Roleplay Director prompts
  under `momo_core` and embeds them at compile time; they are not runtime files,
  portable configuration, request replacements, or hot-reloaded assets.
- Replaces the diffuse foreground role-play prompt with a smaller execution
  contract built around knowledge, delegated-authority, and risk ledgers plus a
  final contradiction check.
- Adds a reproducible MORP direct-vs-MOMO A/B harness with per-arm provenance,
  audit checks, paired scoring, and machine-readable reports.

## Verification

The final release commit passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `python -m unittest discover -s benchmarks/morp/tests -q`
- `cargo doc --workspace --all-features --no-deps`
- `cargo build --release --workspace`

This comprised 245 Rust tests, 102 Python tests, and 21/21 offline runtime
contract probes, with no network calls in the offline suite.

Focused conformance tests cover DDM calculation, validation, typed signals,
conflicts, top-k, fingerprinting, persisted hysteresis, isolation, atomic
snapshot publication, MOC round trips, profile management, model-facing cue
injection, and request-ID restart replay.

## Behavior evidence and limits

An eight-case mixed-context A/B run exercised real retrieval and MO State. The
direct and MOMO arms both scored **84.375/100**; each won three cases with two
ties. This establishes that the governed path executed, not that it improved
behavior. The earlier short-context eight-case smoke reached **87.50/100**, but
had retrieval and MO State disabled and is not DDM evidence.

DDM remains an experimental specification. No DDM-specific credentialed
counterfactual provider study is claimed, and the small A/B sample does not
establish statistical benefit. Broader long-run, image-provider, and production
load evidence remains a stable `v1.0.0` gate.

Workspace crates remain `publish = false`; this GitHub prerelease distributes
the source contract and tag rather than crates.io packages.
