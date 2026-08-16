# MOMO Vector Storage Profile 0.3

**Status:** implementation profile  
**Updated:** 2026-08-16

## Scope

NSG vector storage is accessed through `NsgVectorStore`. The contract covers:

- scope and vector-space isolation;
- dimension and finite-value validation;
- source-hash freshness checks;
- deterministic top-k ordering;
- replaceable storage and ranking backends.

Embedding generation remains a caller responsibility. The caller supplies the
vector-space identifier, query vector, and source vectors; Core validates and
uses them through the storage contract.

## Turso backend

Turso in MOMO documentation means the standalone database library. It is the
selected vector-database backend and must be integrated through
`NsgVectorStore`, so database-specific details do not leak into memory, MOC, or
client-facing contracts.

The current source tree also contains `LocalStore` exact cosine ranking. That
path supplies deterministic compatibility semantics and test coverage while a
Turso adapter uses the same observable contract. It must not be described as a
different vector-database product.

## Required behavior

An implementation MUST:

1. reject empty, zero, non-finite, or dimensionally inconsistent vectors;
2. keep records isolated by scope and vector-space identifier;
3. ignore records whose source hash is stale;
4. return at most the requested bounded top-k;
5. use a stable tie-breaker after similarity score;
6. treat stored vectors as a reproducible cache rather than a portable source
   of truth.

MOC exports therefore carry DMW and NSG source documents, not vector indexes.
An index may be rebuilt from those sources and the selected embedding model.

## Current validation baseline

The exact-ranking tests cover scope isolation, vector-space isolation, input
validation, stale hashes, status reporting, cosine ranking, and deterministic
ties. The included benchmark is a local regression tool, not a product-level
performance guarantee.
