use super::*;

#[tokio::test]
async fn initializes_local_data_layout() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    assert!(core.data_dir().join("momo.sqlite3").exists());
    assert!(core.data_dir().join("nsg-vectors.db").exists());
    let scope_id = momo_domain::new_id();
    core.memory_for_space(scope_id).expect("memory");
    assert!(
        core.data_dir()
            .join("spaces")
            .join(scope_id.to_string())
            .join("memory/current/scene.md")
            .exists()
    );
}

#[tokio::test]
async fn reuses_memory_workspace_state_for_the_same_space() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    let space_id = momo_domain::new_id();

    let first = core.memory_for_space(space_id).expect("first workspace");
    let second = core.memory_for_space(space_id).expect("second workspace");

    assert!(Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn converts_the_legacy_memory_directory_once() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let space_id = momo_domain::new_id();
    let legacy = directory
        .path()
        .join("memory/users")
        .join(space_id.to_string());
    std::fs::create_dir_all(&legacy).expect("legacy directory");
    std::fs::write(legacy.join("marker"), "ok").expect("legacy marker");

    MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");

    assert!(!directory.path().join("memory/users").exists());
    assert!(
        directory
            .path()
            .join("spaces")
            .join(space_id.to_string())
            .join("memory/marker")
            .exists()
    );
}

#[tokio::test]
async fn data_directory_has_one_live_owner() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let first = MomoCore::initialize(directory.path())
        .await
        .expect("first owner");
    let error = MomoCore::initialize(directory.path())
        .await
        .expect_err("second owner must be rejected");
    assert!(matches!(error, CoreError::InstanceLock { .. }));
    drop(first);
    MomoCore::initialize(directory.path())
        .await
        .expect("lock is released when owner drops");
}
