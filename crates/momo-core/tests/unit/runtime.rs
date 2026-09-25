use super::*;

#[tokio::test]
async fn caller_cancellation_does_not_release_an_owned_commit_lock() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = Arc::new(
        MomoRuntime::initialize(directory.path())
            .await
            .expect("runtime"),
    );
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, proceed) = tokio::sync::oneshot::channel();
    let (finished, completed) = tokio::sync::oneshot::channel();
    let task_runtime = runtime.clone();
    let caller = tokio::spawn(async move {
        let guard = task_runtime
            .lock_space(uuid::Uuid::nil())
            .await
            .expect("Space available");
        task_runtime
            .finish_commit(async move {
                let _guard = guard;
                started.send(()).expect("started");
                proceed.await.expect("release");
                finished.send(()).expect("finished");
            })
            .await
    });
    ready.await.expect("commit entered");
    caller.abort();
    assert!(caller.await.expect_err("aborted caller").is_cancelled());
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            runtime.lock_space(uuid::Uuid::nil())
        )
        .await
        .is_err()
    );
    release.send(()).expect("continue commit");
    completed.await.expect("commit completed after caller left");
    let _guard = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime.lock_space(uuid::Uuid::nil()),
    )
    .await
    .expect("lock released");
}

#[tokio::test]
async fn cancellation_registration_is_removed_on_drop_and_preserves_replacement() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let first = runtime.register_cancellation("request".to_owned());
    let replacement = runtime.register_cancellation("request".to_owned());
    drop(first);
    assert!(runtime.cancel_chat("request"));
    replacement.notified().await;
    drop(replacement);
    assert!(!runtime.cancel_chat("request"));
}

#[tokio::test]
async fn keyed_locks_serialize_only_matching_resources() {
    let locks = tokio::sync::Mutex::new(HashMap::new());
    let first = lock_keyed(&locks, "space-a").await;

    let different = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        lock_keyed(&locks, "space-b"),
    )
    .await
    .expect("a different resource must not share the lock");
    drop(different);

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            lock_keyed(&locks, "space-a"),
        )
        .await
        .is_err(),
        "the same resource must remain serialized",
    );

    drop(first);
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        lock_keyed(&locks, "space-a"),
    )
    .await
    .expect("the resource lock must be released with its guard");
}
