# MOMO Core 0.4.2 Progress

**Status:** released
**Date:** 2026-08-25

## Delivered

- Removed the remaining vendored external Character Card specification and its
  license copy. CCv1/v2/v3, RisuAI, Character Foundry, and SillyTavern sources
  are now referenced only through pinned upstream links with explicit roles and
  licensing provenance.
- Replaced prefix-based character-card version guessing with exact CCv2 `2.0`
  handling and numeric CCv3 `[3.0, 4.0)` handling. Newer supported CCv3
  revisions produce a warning while unknown fields remain preserved.
- Documented unresolved CCv3 field-limit/version ambiguity and incomplete
  cross-application CHARX validation instead of claiming universal
  interoperability.
- Aligned the outbound embeddings request/response profile with the official
  OpenAI API fields, retained token usage, and validated optional protocol
  object metadata.
- Corrected the local embeddings endpoint to return 400 for caller errors, 504
  for upstream timeouts, and 502 for other upstream/protocol failures.

## Compatibility boundary

The outbound provider is OpenAI-compatible. MOMO local endpoints remain custom
orchestration contracts because they also carry endpoint configuration,
deterministic vector-space identity, input IDs, and query/document purpose.
They are not advertised as drop-in OpenAI server endpoints.
