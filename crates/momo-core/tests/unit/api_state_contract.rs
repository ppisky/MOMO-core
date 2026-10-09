use super::*;

#[tokio::test]
async fn contract_read_waits_for_space_recovery_admission() {
    let root = tempfile::tempdir().unwrap();
    let runtime = MomoRuntime::initialize(root.path()).await.unwrap();
    let space = uuid::Uuid::now_v7();
    runtime.core().mark_memory_commit_pending(space);
    let contract = mo_state_contract(&runtime, space).await.unwrap();
    assert_eq!(contract.source, "builtin");
    assert!(!runtime.core().memory_recovery_status().contains_key(&space));
}
