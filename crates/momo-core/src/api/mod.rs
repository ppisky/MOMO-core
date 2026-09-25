//! Runtime-bound typed application operations.
//!
//! This module is an in-workspace implementation boundary, not an independent
//! crates.io SemVer surface. Every stateful operation receives an explicit
//! [`crate::MomoRuntime`]; there is no process-global Core instance.
//! HTTP and native Rust callers use the same typed operations. Serialization
//! belongs at wire, file-format and persistence boundaries.

pub mod runtime_api;
