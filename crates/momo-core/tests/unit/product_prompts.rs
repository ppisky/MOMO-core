use super::*;

#[test]
fn compiled_product_prompts_are_complete() {
    assert_eq!(ASSISTANT.trim(), "You are a helpful assistant.");
    assert!(VISION_FALLBACK.contains("visible facts"));
    assert!(MEMORY_DISTILLATION.contains("preserve the concrete outcome in DMW"));
    assert!(SEMANTIC_GRAPH_GOVERNANCE.contains("Never create Canon"));
    assert!(ROLEPLAY_DIRECTOR.contains("Narration and dialogue have the same knowledge boundary"));
    assert!(ROLEPLAY_DIRECTOR.contains("Final audit: before emitting"));
}
