# MOMO Core 0.3 Status

**Status:** public implementation baseline  
**Updated:** 2026-08-16

MOMO Core 0.3 consolidates the current Rust workspace without changing the
runtime behavior established by its test suite.

## Included systems

- local domain models and UUIDv7 identities;
- SQLite persistence for characters, conversations, messages, deletion state,
  review records, portable metadata, and vector cache records;
- Character Card v2 and MOC v2 import/export;
- private MOC encryption;
- DMW, NSG, and MO State;
- context assembly and OpenAI-compatible completion/streaming;
- capability discovery and tokenizer-aware budgeting;
- the `NsgVectorStore` storage boundary;
- the local `momo-server` HTTP/SSE interface.

The crates are internal implementation modules of one Core workspace. The
status document does not define external applications, local machine layouts,
or deployment repository boundaries.

## Vector storage status

Turso refers to the standalone database library selected for the vector
database backend and fits behind `NsgVectorStore`. The current 0.3 source also
contains a deterministic exact-ranking implementation used for local
verification.

## Compatibility

- MOC v2 remains the native portable format.
- Existing local persistence and retrieval behavior is unchanged.
- The HTTP/SSE request and event shapes covered by the regression suite remain
  stable for 0.3.
- Compatibility entry points that are disabled continue to fail closed.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```

The public `v0.3.0` release is cut only from a revision that passes these checks
in GitHub Actions.
