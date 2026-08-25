# MOMO Core 0.4.0 Status

**Status:** CHARX interoperability baseline
**Updated:** 2026-08-25

MOMO Core 0.4.0 replaces the card-only CHARX reader from 0.3.2 with a complete
container round-trip profile.

## 0.4.0 changes

- imports standard ZIP CHARX and JPEG+ZIP hybrid inputs;
- validates entry paths, duplicates, symlinks, encryption, entry count,
  per-entry expansion, total expansion, and CRCs;
- retains the original source container under
  `character-packages/<character_id>/source.charx`;
- exports `ccv3_charx`, rebuilding `card.json` from the current MOMO character;
- preserves embedded assets, `x_meta`, `module.risum`, and unknown safe entries;
- carries the retained source container through the MOC `tavern_compat` module;
- declared the then-used Character Foundry compatibility reference. Version
  0.4.1 replaces the redistributed snapshot with upstream links and a corrected
  normative/reference-implementation hierarchy.

The output archive is a standard ZIP. Compression bytes, timestamps, and entry
ordering are not promised to match the source archive. A native MOMO character
without source assets exports as a valid card-only CHARX.

## Deferred to 0.4.1

The vectorization-model boundary is incomplete. Version 0.4.0 stores and ranks
caller-provided vectors but does not call an embeddings endpoint or expose an
HTTP index-build workflow. See `docs/vectorization_model_interface_0_4_1.md`.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```
