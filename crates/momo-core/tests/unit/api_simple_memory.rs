use super::*;

fn source(id: &str, label: &str, weight: u8) -> MemorySpaceSource {
    MemorySpaceSource {
        space_id: id.to_owned(),
        label: label.to_owned(),
        weight,
        memory: true,
        semantic_graph: true,
    }
}

#[test]
fn space_budget_is_weighted_and_conserves_total() {
    let sources = [
        source("01900000-0000-7000-8000-000000000101", "personal", 3),
        source("01900000-0000-7000-8000-000000000102", "room", 2),
    ];
    let budgets = weighted_space_budgets(&sources, 1_024);
    assert_eq!(budgets, [615, 409]);
    assert_eq!(budgets.iter().sum::<usize>(), 1_024);
}

#[test]
fn space_sources_require_unique_valid_ids() {
    let sources = [
        source("01900000-0000-7000-8000-000000000101", "personal", 1),
        source("01900000-0000-7000-8000-000000000101", "room", 1),
    ];
    assert!(validate_memory_spaces(&sources).is_err());
}

#[test]
fn hybrid_retrieval_reserves_forty_percent_for_nsg() {
    let max_tokens = 961;
    let memory_budget = dmw_retrieval_budget(max_tokens, true, true);
    assert_eq!(memory_budget, 576);
    assert_eq!(max_tokens - memory_budget, 385);
}

#[test]
fn hybrid_retrieval_returns_unused_nsg_budget_to_dmw() {
    let max_tokens = 961;
    assert_eq!(
        effective_dmw_retrieval_budget(max_tokens, true, true, 0),
        961
    );
    assert_eq!(
        effective_dmw_retrieval_budget(max_tokens, true, true, 233),
        728
    );
    assert_eq!(
        effective_dmw_retrieval_budget(max_tokens, true, false, 0),
        961
    );
    assert_eq!(
        effective_dmw_retrieval_budget(max_tokens, false, true, 233),
        0
    );
}
