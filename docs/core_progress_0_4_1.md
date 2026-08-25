# MOMO Core 0.4.1 Progress

**Status:** released
**Date:** 2026-08-25

## Delivered

- Added a vendor-neutral `EmbeddingProvider` contract and an
  OpenAI-compatible `/embeddings` implementation.
- Added deterministic embedding profiles and vector-space identifiers covering
  model, revision, dimension, normalization, endpoint identity, and purpose
  prefixes without including URLs or credentials.
- Added HTTP batch embedding generation, atomic full/incremental NSG vector
  rebuilds, and query-text embedding for scoped memory retrieval.
- Added strict input, response-count, response-index, model, dimension,
  finite-value, nonzero-vector, batch-size, text-size, response-size, and
  timeout validation, plus redacted endpoint debug output.
- Added vector-space identity to vector-status responses and atomic replacement
  tests that prevent deleted NSG nodes from surviving an index rebuild.
- Corrected the CHARX provenance policy: only the MIT-licensed CCv3 normative
  snapshot remains vendored. CCv1/v2, RisuAI extension behavior, and Character
  Foundry documentation are linked with explicit roles and licensing notes,
  rather than redistributed.

## Compatibility

The raw `vector_space_id` plus `query_vector` retrieval path remains available
for advanced callers. The higher-level `embedding` request path is mutually
exclusive with those raw fields and derives the vector-space identifier from
the declared profile.

CHARX behavior shipped in 0.4.0 is retained. The 0.4.1 change corrects source
attribution and redistribution boundaries; it does not claim clean-room
independence from the explicitly identified implementation references.
