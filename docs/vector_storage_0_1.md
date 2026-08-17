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

## Two-database runtime layout

MOMO Core uses two database files with different responsibilities:

| File | Engine | Contents |
| --- | --- | --- |
| `momo.sqlite3` | SQLx/SQLite | characters, conversations, messages, deletion state, patch reviews, and portable metadata |
| `nsg-vectors.db` | standalone Turso Rust library | NSG vector records and their source-hash/vector-space metadata |

`TursoVectorStore` is the production `NsgVectorStore` implementation. It uses
the standalone Turso engine in-process; it does not require Turso Cloud or
network credentials. Exact cosine ranking is deterministic and runs over the
records read from the Turso database.

The `NsgVectorStore` trait is an internal boundary so Turso-specific details do
not leak into memory, MOC, or client-facing contracts. It does not denote an
additional data store.

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
Migration `0014_remove_sqlite_vector_cache.sql` removes the pre-0.3.2 vector
table from `momo.sqlite3`; its cache records are deliberately not migrated.

## Current validation baseline

The Turso-backed exact-ranking tests cover scope isolation, vector-space
isolation, input validation, stale hashes, status reporting, cosine ranking,
and deterministic ties. The included benchmark is a local regression tool,
not a product-level performance guarantee.
