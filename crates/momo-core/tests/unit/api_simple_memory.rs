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
fn maximum_space_budget_finishes_and_conserves_total() {
    let one = [source("unused", "single", 100)];
    assert_eq!(weighted_space_budgets(&one, usize::MAX), [usize::MAX]);
    let sources = [
        source("unused-a", "a", 100),
        source("unused-b", "b", 3),
        source("unused-c", "c", 1),
    ];
    for total in [0, 1, 2, 1_024, usize::MAX / 100, usize::MAX] {
        let budgets = weighted_space_budgets(&sources, total);
        assert_eq!(budgets.iter().sum::<usize>(), total);
        for (source, budget) in sources.iter().zip(budgets) {
            let floor = (total as u128) * u128::from(source.weight) / 104;
            assert!((floor..=floor + 1).contains(&(budget as u128)));
        }
    }
}

#[tokio::test]
async fn retrieval_waits_for_writer_using_a_different_uuid_spelling() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let id = "abcdefab-1234-4567-89ab-abcdefabcdef";
    let guard = runtime
        .lock_space(uuid::Uuid::parse_str(id).expect("UUID"))
        .await
        .expect("Space available");
    let mut retrieval = Box::pin(retrieve_scoped_memory_snapshot(
        &runtime,
        ScopedMemoryRequest {
            spaces: vec![source(&id.to_uppercase(), "memory", 1)],
            observe_space_ids: Vec::new(),
            query: "scene".to_owned(),
            max_tokens: 100,
            vector_space_id: None,
            query_vector: None,
            embedding: None,
        },
    ));
    let blocked = tokio::time::timeout(std::time::Duration::from_millis(50), &mut retrieval)
        .await
        .is_err();
    drop(guard);
    assert!(blocked, "retrieval must wait for the existing Space writer");
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(3), retrieval)
        .await
        .expect("writer released")
        .expect("retrieval");
    assert_eq!(snapshot.source_observations[0].space_id, id.to_uppercase());
}

#[tokio::test]
async fn retrieval_deduplicates_uuid_locks_but_preserves_observation_names() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let id = uuid::Uuid::parse_str("abcdefab-1234-4567-89ab-abcdefabcdef").expect("id");
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        retrieve_scoped_memory_snapshot(
            &runtime,
            ScopedMemoryRequest {
                spaces: vec![source(&id.to_string(), "memory", 1)],
                observe_space_ids: vec![id.to_string().to_uppercase(), id.simple().to_string()],
                query: "scene".to_owned(),
                max_tokens: 100,
                vector_space_id: None,
                query_vector: None,
                embedding: None,
            },
        ),
    )
    .await
    .expect("must not acquire the same Space lock twice")
    .expect("retrieval");
    assert_eq!(snapshot.source_observations.len(), 3);
    assert!(snapshot.source_observations.iter().all(|observation| {
        observation.dmw_fingerprint == snapshot.source_observations[0].dmw_fingerprint
    }));
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
