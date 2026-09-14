//! Product-owned system prompts compiled into MOMO Core.
//!
//! The Markdown files live under the Rust source tree so they remain directly
//! reviewable while `include_str!` makes them compile-time inputs. Runtime
//! configuration, requests, MOC archives, and filesystem state cannot replace
//! these values.

pub(crate) const MEMORY_DISTILLATION: &str = include_str!("product_prompts/dmw_distiller.md");
pub(crate) const SEMANTIC_GRAPH_GOVERNANCE: &str = include_str!("product_prompts/nsg_governor.md");
pub(crate) const ROLEPLAY_DIRECTOR: &str = include_str!("product_prompts/roleplay_director.md");

#[cfg(test)]
#[path = "../tests/unit/product_prompts.rs"]
mod tests;
