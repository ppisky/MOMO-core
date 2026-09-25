use super::*;
use std::time::Duration;

#[tokio::test]
async fn cancelled_clear_finishes_database_cleanup_before_releasing_space() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = Arc::new(
        MomoRuntime::initialize(directory.path())
            .await
            .expect("runtime"),
    );
    let space = uuid::Uuid::now_v7();
    let workspace = runtime.core().memory_for_space(space).expect("workspace");
    let path = workspace.root().join("events/to-clear.md");
    fs::write(&path, "pending memory").expect("fixture");
    runtime
        .core()
        .store()
        .append_maintenance_turn(
            &momo_storage::MaintenanceTurn {
                request_id: "pending-clear".to_owned(),
                scope_id: space.to_string(),
                user_content: "user".to_owned(),
                assistant_content: "assistant".to_owned(),
            },
            true,
            true,
        )
        .await
        .expect("pending turn");
    // Hold the sole SQLite connection so the action pauses after file deletion.
    let connection = runtime
        .core()
        .store()
        .pool()
        .acquire()
        .await
        .expect("connection");
    let task_runtime = Arc::clone(&runtime);
    let caller = tokio::spawn(async move {
        execute_control_action(
            &task_runtime,
            &crate::MomoControlAction::ClearMemory {
                target_space_id: space,
                memory: true,
                semantic_graph: true,
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("file deletion started");
    caller.abort();
    assert!(caller.await.expect_err("caller aborted").is_cancelled());
    let blocked = tokio::time::timeout(Duration::from_millis(25), runtime.lock_space(space))
        .await
        .is_err();
    drop(connection);
    let _guard = tokio::time::timeout(Duration::from_secs(3), runtime.lock_space(space))
        .await
        .expect("cleanup completed");
    assert!(blocked, "Space must stay locked through SQLite cleanup");
    for kind in [
        momo_storage::MaintenanceKind::Memory,
        momo_storage::MaintenanceKind::SemanticGraph,
    ] {
        assert!(
            runtime
                .core()
                .store()
                .pending_maintenance_turns(&space.to_string(), kind, 10)
                .await
                .expect("pending turns")
                .is_empty(),
        );
    }
}
