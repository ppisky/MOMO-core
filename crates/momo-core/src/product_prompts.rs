//! Product-owned default prompts compiled into MOMO Core.
//!
//! The Markdown files live under the Rust source tree so they remain directly
//! reviewable while `include_str!` makes them compile-time defaults. Active
//! values are owned by the Prompt Spaces registry and may be replaced through
//! its API without turning host configuration into Core configuration.

pub(crate) const ASSISTANT: &str = include_str!("product_prompts/assistant.md");
pub(crate) const VISION_FALLBACK: &str = include_str!("product_prompts/vision_fallback.md");
pub(crate) const MEMORY_DISTILLATION: &str = include_str!("product_prompts/dmw_distiller.md");
pub(crate) const SEMANTIC_GRAPH_GOVERNANCE: &str = include_str!("product_prompts/nsg_governor.md");
pub(crate) const ROLEPLAY_DIRECTOR: &str = include_str!("product_prompts/roleplay_director.md");

#[cfg(test)]
#[path = "../tests/unit/product_prompts.rs"]
mod tests;
