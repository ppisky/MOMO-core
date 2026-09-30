use super::*;

const PATCH: &str = r#"patches:
  - target_file: events/owned.md
    operations:
      - type: create
        frontmatter:
          id: owned_event
          type: event
          importance: 0.8
          weight: 0.8
          decay_at: 1
          relations: {}
          tags: [owned]
          status: active
        content: Durable worker event.
"#;

fn fault(handle: &SpaceHandle, point: u8) {
    let (reply, done) = mpsc::sync_channel(1);
    handle
        .sender
        .as_ref()
        .unwrap()
        .send(Box::new(move |worker| {
            worker.fault = Some(point);
            reply.send(()).unwrap();
        }))
        .unwrap();
    done.recv().unwrap();
}

#[tokio::test]
async fn panic_before_journal_discards_private_changes_and_keeps_other_spaces_available() {
    let directory = tempfile::tempdir().unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    let id = Uuid::now_v7();
    let worker = core.memory_for_space(id).unwrap();
    let healthy = core.memory_for_space(Uuid::now_v7()).unwrap();
    let before = worker.export_snapshot().unwrap();
    let error = worker
        .call::<()>(|workspace| {
            workspace.apply_patch(PATCH)?;
            panic!("abandon private state");
        })
        .unwrap_err();
    assert!(error.to_string().contains("interrupted"));
    healthy.apply_patch(PATCH).unwrap();
    assert_eq!(worker.export_snapshot().unwrap().files, before.files);
    assert!(core.memory_recovery_status().is_empty());
    worker.apply_patch(PATCH).unwrap();
    assert_eq!(
        worker.read_document_by_id("owned_event").unwrap().body,
        "Durable worker event."
    );
}

#[tokio::test]
async fn journal_admission_is_the_commit_point_across_panic_and_process_reopen() {
    for point in 0..=3 {
        let directory = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        {
            let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
            let worker = core.memory_for_space(id).unwrap();
            fault(&worker, point);
            assert!(worker.apply_patch(PATCH).is_err());
            // Do not issue another request: exercise startup recovery from disk.
        }
        let reopened = crate::MomoCore::initialize(directory.path()).await.unwrap();
        let worker = reopened.memory_for_space(id).unwrap();
        let documents = worker.list_documents().unwrap();
        assert_eq!(
            documents
                .iter()
                .filter(|doc| doc.id == "owned_event")
                .count(),
            usize::from(point > 0)
        );
        assert!(reopened.memory_recovery_status().is_empty());
        if point > 0 {
            assert_eq!(
                worker
                    .read_document_by_id("owned_event")
                    .unwrap()
                    .body
                    .matches("Durable worker event.")
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn failed_command_does_not_commit_partial_cache_mutations() {
    let directory = tempfile::tempdir().unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    let worker = core.memory_for_space(Uuid::now_v7()).unwrap();
    let before = worker.export_snapshot().unwrap();
    assert!(
        worker
            .call::<()>(|workspace| {
                workspace.apply_patch(PATCH)?;
                Err(MemoryError::InvalidPatch("rejected after preparing".into()))
            })
            .is_err()
    );
    assert_eq!(worker.export_snapshot().unwrap().files, before.files);
}

#[tokio::test]
async fn bounded_registry_evicts_idle_addresses_but_never_active_owners() {
    let directory = tempfile::tempdir().unwrap();
    let instance_lock = Arc::new(crate::acquire_instance_lock(directory.path()).unwrap());
    let supervisor =
        SpaceSupervisor::start_with_limit(directory.path().join("spaces"), instance_lock, 2)
            .unwrap();
    let first_id = Uuid::now_v7();
    let first = supervisor.space(first_id).unwrap();
    let second_id = Uuid::now_v7();
    let second = supervisor.space(second_id).unwrap();
    assert!(matches!(
        supervisor.space(Uuid::now_v7()),
        Err(MemoryError::WorkspaceCapacity { limit: 2 })
    ));
    assert!(Arc::ptr_eq(&first, &supervisor.space(first_id).unwrap()));
    second.apply_patch(PATCH).unwrap();
    drop(second);
    supervisor.space(Uuid::now_v7()).unwrap();
    assert!(
        supervisor
            .space(second_id)
            .unwrap()
            .read_document_by_id("owned_event")
            .is_ok()
    );
}

#[tokio::test]
async fn corrupt_journal_isolates_only_its_space_and_retains_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let id = Uuid::now_v7();
    {
        let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
        core.memory_for_space(id)
            .unwrap()
            .apply_patch(PATCH)
            .unwrap();
    }
    let options = SqliteConnectOptions::new().filename(
        directory
            .path()
            .join("spaces")
            .join(id.to_string())
            .join("memory-journal.sqlite3"),
    );
    let mut journal = SqliteConnection::connect_with(&options).await.unwrap();
    // Even an acknowledged snapshot must be validated on a clean restart.
    sqlx::query("UPDATE memory_journal SET snapshot = '{' WHERE revision = (SELECT MAX(revision) FROM memory_journal)").execute(&mut journal).await.unwrap();
    journal.close().await.unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    assert!(core.memory_recovery_status().contains_key(&id));
    assert!(core.memory_for_space(id).is_err());
    core.memory_for_space(Uuid::now_v7())
        .unwrap()
        .apply_patch(PATCH)
        .unwrap();
}

#[tokio::test]
async fn unrelated_space_is_not_blocked_by_a_busy_worker() {
    let directory = tempfile::tempdir().unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    let slow = core.memory_for_space(Uuid::now_v7()).unwrap();
    let fast = core.memory_for_space(Uuid::now_v7()).unwrap();
    let (started, waiting) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let task = std::thread::spawn(move || {
        slow.call(move |_| {
            started.send(()).unwrap();
            resume.recv().unwrap();
            Ok(())
        })
    });
    waiting.recv().unwrap();
    fast.apply_patch(PATCH).unwrap();
    release.send(()).unwrap();
    task.join().unwrap().unwrap();
}

// Invoked in a child test process by the test below. exit(91) deliberately
// skips unwinding, destructors, SQLite close and supervisor shutdown.
#[tokio::test]
async fn process_crash_probe() {
    let Some(root) = std::env::var_os("MOMO_WORKER_CRASH_ROOT") else {
        return;
    };
    let point = std::env::var("MOMO_WORKER_CRASH_POINT")
        .unwrap()
        .parse::<u8>()
        .unwrap();
    let id = Uuid::parse_str(&std::env::var("MOMO_WORKER_CRASH_SPACE").unwrap()).unwrap();
    let core = crate::MomoCore::initialize(PathBuf::from(root))
        .await
        .unwrap();
    let worker = core.memory_for_space(id).unwrap();
    let clearing = std::env::var_os("MOMO_WORKER_CRASH_CLEAR").is_some();
    if clearing {
        worker.apply_patch(PATCH).unwrap();
        core.store()
            .append_maintenance_turn(
                &momo_storage::MaintenanceTurn {
                    request_id: "clear-crash".into(),
                    scope_id: id.to_string(),
                    user_content: "old user".into(),
                    assistant_content: "old assistant".into(),
                },
                true,
                true,
            )
            .await
            .unwrap();
    }
    let (reply, done) = mpsc::sync_channel(1);
    worker
        .sender
        .as_ref()
        .unwrap()
        .send(Box::new(move |worker| {
            worker.fault = Some(point);
            worker.terminate_on_fault = true;
            reply.send(()).unwrap();
        }))
        .unwrap();
    done.recv().unwrap();
    if clearing {
        worker.clear_memory(true, true).unwrap();
    } else {
        worker.apply_patch(PATCH).unwrap();
    }
    panic!("crash point was not reached");
}

#[tokio::test]
async fn acknowledged_state_is_restored_and_external_files_never_become_authority() {
    let directory = tempfile::tempdir().unwrap();
    let id = Uuid::now_v7();
    let expected;
    let root;
    {
        let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
        let worker = core.memory_for_space(id).unwrap();
        worker.apply_patch(PATCH).unwrap();
        expected = worker.export_snapshot().unwrap();
        root = worker.root().to_owned();
        std::fs::write(root.join("events/owned.md"), "unjournaled edit").unwrap();
        assert_eq!(worker.export_snapshot().unwrap().files, expected.files);
    }
    // Simulate loss of an already acknowledged materialized file.
    std::fs::remove_file(root.join("events/owned.md")).unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    assert_eq!(
        core.memory_for_space(id)
            .unwrap()
            .export_snapshot()
            .unwrap()
            .files,
        expected.files
    );
    assert_eq!(
        std::fs::read_to_string(root.join("events/owned.md")).unwrap(),
        expected.files["events/owned.md"]
    );
}

#[tokio::test]
async fn clear_intent_survives_process_exit_and_finishes_sql_cleanup() {
    for point in 0..=3 {
        let directory = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "space_worker::tests::process_crash_probe",
                "--nocapture",
            ])
            .env("MOMO_WORKER_CRASH_ROOT", directory.path())
            .env("MOMO_WORKER_CRASH_POINT", point.to_string())
            .env("MOMO_WORKER_CRASH_SPACE", id.to_string())
            .env("MOMO_WORKER_CRASH_CLEAR", "1")
            .output()
            .unwrap();
        assert_eq!(
            child.status.code(),
            Some(91),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
        assert!(core.memory_recovery_status().is_empty());
        let worker = core.memory_for_space(id).unwrap();
        assert_eq!(
            worker.read_document_by_id("owned_event").is_ok(),
            point == 0
        );
        assert!(worker.pending_clear().unwrap().is_none());
        for kind in [
            momo_storage::MaintenanceKind::Memory,
            momo_storage::MaintenanceKind::SemanticGraph,
        ] {
            let pending = core
                .store()
                .pending_maintenance_turns(&id.to_string(), kind, 10)
                .await
                .unwrap();
            assert_eq!(pending.len(), usize::from(point == 0));
        }
        worker.apply_patch(PATCH).unwrap();
    }
}

#[tokio::test]
async fn durable_journal_survives_abrupt_process_exit_at_each_commit_boundary() {
    for point in 0..=3 {
        let directory = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "space_worker::tests::process_crash_probe",
                "--nocapture",
            ])
            .env("MOMO_WORKER_CRASH_ROOT", directory.path())
            .env("MOMO_WORKER_CRASH_POINT", point.to_string())
            .env("MOMO_WORKER_CRASH_SPACE", id.to_string())
            .output()
            .unwrap();
        assert_eq!(
            child.status.code(),
            Some(91),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
        assert!(core.memory_recovery_status().is_empty());
        let documents = core.memory_for_space(id).unwrap().list_documents().unwrap();
        assert_eq!(
            documents
                .iter()
                .filter(|doc| doc.id == "owned_event")
                .count(),
            usize::from(point > 0)
        );
    }
}

#[tokio::test]
async fn admitted_command_commits_even_when_its_reply_is_abandoned() {
    let directory = tempfile::tempdir().unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    let worker = core.memory_for_space(Uuid::now_v7()).unwrap();
    worker
        .sender
        .as_ref()
        .unwrap()
        .send(Box::new(|worker| {
            worker
                .execute(|workspace| workspace.apply_patch(PATCH))
                .unwrap();
        }))
        .unwrap();
    assert!(worker.read_document_by_id("owned_event").is_ok());
}

#[tokio::test]
async fn failed_durable_write_never_publishes_the_private_after_image() {
    let directory = tempfile::tempdir().unwrap();
    let core = crate::MomoCore::initialize(directory.path()).await.unwrap();
    let id = Uuid::now_v7();
    let worker = core.memory_for_space(id).unwrap();
    let before = worker.export_snapshot().unwrap();
    let (reply, ready) = mpsc::sync_channel(1);
    worker
        .sender
        .as_ref()
        .unwrap()
        .send(Box::new(move |worker| {
            worker
                .runtime
                .block_on(sqlx::query("PRAGMA query_only = ON").execute(&mut worker.journal))
                .unwrap();
            reply.send(()).unwrap();
        }))
        .unwrap();
    ready.recv().unwrap();
    assert!(worker.apply_patch(PATCH).is_err());
    assert!(!worker.root().join("events/owned.md").exists());
    assert_eq!(worker.export_snapshot().unwrap().files, before.files);
    assert!(core.memory_recovery_status().is_empty());
    worker.apply_patch(PATCH).unwrap();
}
