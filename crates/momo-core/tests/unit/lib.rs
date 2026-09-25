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
async fn concurrent_workspace_initialization_reuses_one_instance() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    let space_id = momo_domain::new_id();
    let worker_count = 16;
    let start = Arc::new(std::sync::Barrier::new(worker_count));
    let workers = (0..worker_count)
        .map(|_| {
            let core = core.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                core.memory_for_space(space_id).expect("workspace")
            })
        })
        .collect::<Vec<_>>();
    let workspaces = workers
        .into_iter()
        .map(|worker| worker.join().expect("workspace worker"))
        .collect::<Vec<_>>();

    assert!(
        workspaces
            .iter()
            .all(|workspace| Arc::ptr_eq(&workspaces[0], workspace))
    );
    assert!(
        core.memory_workspaces
            .lock()
            .expect("workspace registry")
            .initializing
            .is_empty()
    );
}

#[tokio::test]
async fn workspace_registry_rejects_growth_when_every_slot_is_active() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    let workspace = core
        .memory_for_space(momo_domain::new_id())
        .expect("workspace");
    {
        let mut registry = core.memory_workspaces.lock().expect("workspace registry");
        while registry.entries.len() < MAX_CACHED_MEMORY_WORKSPACES {
            let generation = registry.next_generation();
            registry.entries.insert(
                momo_domain::new_id(),
                MemoryWorkspaceEntry {
                    workspace: Arc::clone(&workspace),
                    last_used: generation,
                },
            );
        }
    }

    let error = core
        .memory_for_space(momo_domain::new_id())
        .expect_err("active capacity must be enforced");
    assert!(matches!(
        error,
        momo_memory::MemoryError::WorkspaceCapacity {
            limit: MAX_CACHED_MEMORY_WORKSPACES
        }
    ));
    assert_eq!(
        core.memory_workspaces
            .lock()
            .expect("workspace registry")
            .entries
            .len(),
        MAX_CACHED_MEMORY_WORKSPACES
    );
}

#[tokio::test]
async fn workspace_registry_evicts_idle_before_rejecting_new_space() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    let active_id = momo_domain::new_id();
    let active = core.memory_for_space(active_id).expect("active workspace");
    let idle_id = momo_domain::new_id();
    core.memory_for_space(idle_id).expect("idle workspace");
    {
        let mut registry = core.memory_workspaces.lock().expect("workspace registry");
        while registry.entries.len() < MAX_CACHED_MEMORY_WORKSPACES {
            let generation = registry.next_generation();
            registry.entries.insert(
                momo_domain::new_id(),
                MemoryWorkspaceEntry {
                    workspace: Arc::clone(&active),
                    last_used: generation,
                },
            );
        }
    }

    let replacement_id = momo_domain::new_id();
    core.memory_for_space(replacement_id)
        .expect("idle entry should make room");
    let registry = core.memory_workspaces.lock().expect("workspace registry");
    assert_eq!(registry.entries.len(), MAX_CACHED_MEMORY_WORKSPACES);
    assert!(!registry.entries.contains_key(&idle_id));
    assert!(registry.entries.contains_key(&replacement_id));
}

#[tokio::test]
async fn failed_workspace_initialization_releases_capacity_reservation() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    let space_id = momo_domain::new_id();
    let blocked_path = core.data_dir().join("spaces").join(space_id.to_string());
    std::fs::write(&blocked_path, "not a directory").expect("blocking file");

    core.memory_for_space(space_id)
        .expect_err("invalid workspace path must fail");
    assert!(
        core.memory_workspaces
            .lock()
            .expect("workspace registry")
            .initializing
            .is_empty()
    );

    std::fs::remove_file(blocked_path).expect("remove blocking file");
    core.memory_for_space(space_id)
        .expect("workspace initialization can be retried");
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
