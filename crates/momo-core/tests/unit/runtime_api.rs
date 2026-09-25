use super::*;
use std::time::Duration;

#[tokio::test]
async fn space_write_keeps_lock_and_shutdown_pending_after_caller_cancellation() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = Arc::new(
        MomoRuntime::initialize(directory.path())
            .await
            .expect("runtime"),
    );
    let space = uuid::Uuid::now_v7();
    let path = directory.path().join("committed.txt");
    let write_path = path.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let task_runtime = Arc::clone(&runtime);
    let caller = tokio::spawn(async move {
        run_space_write(&task_runtime, space, "test write", move || {
            started.send(()).expect("started");
            proceed
                .recv_timeout(Duration::from_secs(5))
                .expect("write released");
            std::fs::write(write_path, b"committed")
                .map_err(|error| RuntimeApiError::internal(error.to_string()))
        })
        .await
    });
    ready.await.expect("file task entered");
    caller.abort();
    assert!(caller.await.expect_err("caller aborted").is_cancelled());

    let same_space_blocked =
        tokio::time::timeout(Duration::from_millis(25), runtime.lock_space(space))
            .await
            .is_err();
    let other_space = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.lock_space(uuid::Uuid::now_v7()),
    )
    .await
    .expect("unrelated Space remains available");
    drop(other_space);

    let service = {
        let runtime = Arc::clone(&runtime);
        runtime
            .update_runtime_settings((*Arc::new(crate::MomoRuntimeSettings::default())).clone())
            .expect("runtime settings");
        crate::MomoApiService::new(
            runtime,
            "http://127.0.0.1:1/v1".to_owned(),
            None,
            reqwest::Client::new(),
        )
    };
    let mut drain = Box::pin(service.wait_for_maintenance());
    let shutdown_blocked = tokio::time::timeout(Duration::from_millis(25), &mut drain)
        .await
        .is_err();
    release.send(()).expect("release write");
    tokio::time::timeout(Duration::from_secs(3), drain)
        .await
        .expect("shutdown drains completed write");
    let _guard = tokio::time::timeout(Duration::from_secs(1), runtime.lock_space(space))
        .await
        .expect("completed write releases lock");
    assert!(
        same_space_blocked,
        "cancellation must not admit another writer"
    );
    assert!(shutdown_blocked, "shutdown must wait for detached writes");
    assert_eq!(std::fs::read(path).expect("written file"), b"committed");
}

#[tokio::test]
async fn failed_space_write_releases_lock_and_preserves_error() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let space = uuid::Uuid::now_v7();
    let result = run_space_write(&runtime, space, "rejected write", || {
        Err::<(), _>(RuntimeApiError::invalid("invalid patch"))
    })
    .await;
    assert_eq!(
        result.expect_err("rejected").kind,
        RuntimeApiErrorKind::InvalidRequest
    );
    tokio::time::timeout(
        Duration::from_secs(1),
        run_space_write(&runtime, space, "next write", || Ok(())),
    )
    .await
    .expect("lock released after error")
    .expect("next write succeeds");
}

#[tokio::test]
async fn space_locks_share_uuid_identity_across_spellings() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let id = uuid::Uuid::parse_str("abcdefab-1234-4567-89ab-abcdefabcdef").expect("id");
    let _guard = runtime.lock_space(id).await.expect("Space available");
    for spelling in [
        id.to_string().to_uppercase(),
        id.simple().to_string(),
        id.urn().to_string(),
    ] {
        assert!(
            tokio::time::timeout(
                Duration::from_millis(25),
                runtime.lock_space(uuid::Uuid::parse_str(&spelling).expect("UUID")),
            )
            .await
            .is_err(),
            "UUID spelling bypassed the existing writer: {spelling}",
        );
    }
}
