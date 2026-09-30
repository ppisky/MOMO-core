use super::*;
use momo_storage::{MaintenanceBatch, MaintenanceKind, MaintenanceTurn};
use std::sync::Arc;

const PATCH: &str = r#"patches:
  - target_file: events/recovered.md
    operations:
      - type: create
        frontmatter:
          id: recovered_event
          type: event
          importance: 0.8
          weight: 0.8
          decay_at: 1
          relations: {}
          tags: [recovered]
          status: active
        content: Recovered exactly once.
"#;

#[tokio::test]
async fn pending_clear_supersedes_prepared_plans_and_retries_cleanup_after_restart() {
    for cleaned_sql in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let space = momo_domain::new_id();
        {
            let core = MomoCore::initialize(directory.path()).await.unwrap();
            let batch = stage(&core, space, PATCH).await;
            let worker = core.memory_for_space(space).unwrap();
            let plan = worker.prepare_patch_commit(PATCH).unwrap();
            core.store()
                .prepare_maintenance_commit(
                    &batch.batch_key,
                    &serde_json::to_string(&plan).unwrap(),
                )
                .await
                .unwrap();
            worker.apply_prepared_commit(&plan).unwrap();
            worker.clear_memory(true, false).unwrap();
            assert!(
                worker.export_snapshot().is_err(),
                "no admission before cleanup"
            );
            if cleaned_sql {
                core.store()
                    .clear_space_memory_state(space, true, false)
                    .await
                    .unwrap();
            }
            // Drop before acknowledging the clear in the Space journal.
        }
        let core = MomoCore::initialize(directory.path()).await.unwrap();
        assert!(core.memory_recovery_status().is_empty());
        assert!(
            core.store()
                .pending_maintenance_batches()
                .await
                .unwrap()
                .is_empty()
        );
        let worker = core.memory_for_space(space).unwrap();
        assert!(worker.pending_clear().unwrap().is_none());
        assert!(
            worker.read_document_by_id("recovered_event").is_err(),
            "old prepared plan must never resurrect cleared data"
        );
    }
}

#[tokio::test]
async fn failed_clear_cleanup_blocks_space_until_durable_intent_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let space = momo_domain::new_id();
    stage(&core, space, PATCH).await;
    let worker = core.memory_for_space(space).unwrap();
    worker.apply_patch(PATCH).unwrap();
    sqlx::query("CREATE TRIGGER fail_clear BEFORE DELETE ON maintenance_turns BEGIN SELECT RAISE(ABORT, 'injected cleanup failure'); END")
        .execute(core.store().pool()).await.unwrap();
    {
        let _reservation = core.reserve_spaces([space]).await.unwrap();
        assert!(core.clear_space_memory(space, true, true).await.is_err());
    }
    assert!(worker.pending_clear().unwrap().is_some());
    assert!(worker.apply_patch(PATCH).is_err());
    assert!(core.reserve_spaces([space]).await.is_err());
    core.memory_for_space(momo_domain::new_id())
        .unwrap()
        .apply_patch(PATCH)
        .unwrap();
    sqlx::query("DROP TRIGGER fail_clear")
        .execute(core.store().pool())
        .await
        .unwrap();
    let _reservation = core.reserve_spaces([space]).await.unwrap();
    assert!(core.memory_recovery_status().is_empty());
    assert!(worker.pending_clear().unwrap().is_none());
    assert!(worker.read_document_by_id("recovered_event").is_err());
    worker.apply_patch(PATCH).unwrap();
}

async fn lifecycle_event(core: &MomoCore, space: uuid::Uuid, request: &str) {
    let activity = momo_memory::lifecycle::LifecycleActivity {
        identity: None,
        conversation_id: "00000000-0000-4000-8000-000000000001".into(),
        character_id: "00000000-0000-4000-8000-000000000002".into(),
        query: "hello".into(),
        settings: Default::default(),
    };
    sqlx::query(
        "INSERT INTO memory_lifecycle_events(request_id, space_id, activity_json) VALUES (?,?,?)",
    )
    .bind(request)
    .bind(space.to_string())
    .bind(serde_json::to_string(&activity).unwrap())
    .execute(core.store().pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_prepared_commit_recovers_once_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let space = momo_domain::new_id();
    lifecycle_event(&core, space, "lifecycle-recovery").await;
    let pending = core
        .store()
        .pending_lifecycle_events(Some(&space.to_string()))
        .await
        .unwrap();
    let activity = serde_json::from_str(&pending[0].activity_json).unwrap();
    let memory = core.memory_for_space(space).unwrap();
    let (plan, report) = memory.prepare_lifecycle_activity(&activity, 1).unwrap();
    core.store()
        .prepare_lifecycle_event(
            "lifecycle-recovery",
            &serde_json::to_string(&plan).unwrap(),
            &serde_json::to_string(&report).unwrap(),
        )
        .await
        .unwrap();
    memory.apply_prepared_commit(&plan).unwrap();
    drop(memory);
    drop(core);
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    assert!(
        core.store()
            .pending_lifecycle_events(Some(&space.to_string()))
            .await
            .unwrap()
            .is_empty()
    );
    let path = directory
        .path()
        .join("spaces")
        .join(space.to_string())
        .join("memory/indexes/lifecycle_activity.json");
    let before = std::fs::read(&path).unwrap();
    core.process_memory_lifecycle(space).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let ledger: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(
        ledger["contexts"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap(),
        1
    );
    lifecycle_event(&core, space, "next-completed-turn").await;
    core.process_memory_lifecycle(space).await.unwrap();
    let ledger: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        ledger["contexts"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn lifecycle_clear_removes_pending_events_without_advancing_memory() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let space = momo_domain::new_id();
    lifecycle_event(&core, space, "clear-pending-lifecycle").await;
    core.store()
        .clear_space_memory_state(space, true, false)
        .await
        .unwrap();
    assert!(
        core.store()
            .pending_lifecycle_events(Some(&space.to_string()))
            .await
            .unwrap()
            .is_empty()
    );
    core.process_memory_lifecycle(space).await.unwrap();
    assert!(
        !directory
            .path()
            .join("spaces")
            .join(space.to_string())
            .join("memory/indexes/lifecycle_activity.json")
            .exists()
    );
}

#[tokio::test]
async fn approved_review_recovers_files_and_audit_after_restart() {
    let directory = tempfile::tempdir().expect("directory");
    let core = MomoCore::initialize(directory.path()).await.expect("core");
    let space = momo_domain::new_id();
    let review = core
        .store()
        .create_memory_patch_review(
            space,
            "conversation",
            PATCH,
            &["events/recovered.md".to_owned()],
            1,
            "require_confirmation",
        )
        .await
        .expect("review");
    let memory = core.memory_for_space(space).expect("memory");
    let plan = memory.prepare_patch_commit(PATCH).expect("plan");
    core.store()
        .prepare_memory_patch_review_commit(
            space,
            review.id,
            &serde_json::to_string(&plan).expect("json"),
        )
        .await
        .expect("persist approval intent");
    memory
        .apply_prepared_commit(&plan)
        .expect("files written, audit still pending");
    assert!(
        core.store()
            .resolve_memory_patch_review(
                space,
                review.id,
                momo_storage::MemoryPatchReviewStatus::Rejected,
                None,
                None
            )
            .await
            .expect("reject blocked")
            .is_none()
    );
    drop(memory);
    drop(core);
    let reopened = MomoCore::initialize(directory.path())
        .await
        .expect("recover review");
    let review = reopened
        .store()
        .memory_patch_review(space, review.id)
        .await
        .expect("read review")
        .expect("review");
    assert_eq!(
        review.status,
        momo_storage::MemoryPatchReviewStatus::Approved
    );
    assert!(
        reopened
            .store()
            .pending_memory_patch_commits()
            .await
            .expect("journal empty")
            .is_empty()
    );
    assert!(
        reopened
            .memory_for_space(space)
            .expect("memory")
            .read_document_by_id("recovered_event")
            .is_ok()
    );
}

async fn stage(core: &MomoCore, space: uuid::Uuid, patch: &str) -> MaintenanceBatch {
    let turn = MaintenanceTurn {
        request_id: "recovery-turn".to_owned(),
        scope_id: space.to_string(),
        user_content: "remember".to_owned(),
        assistant_content: "remembered".to_owned(),
    };
    core.store()
        .append_maintenance_turn(&turn, true, false)
        .await
        .expect("turn");
    let batch = MaintenanceBatch {
        batch_key: "recovery-batch".to_owned(),
        scope_id: space.to_string(),
        kind: "memory".to_owned(),
        request_ids: vec![turn.request_id],
        patch_yaml: patch.to_owned(),
    };
    core.store()
        .stage_maintenance_batch(&batch)
        .await
        .expect("stage");
    batch
}

#[tokio::test]
async fn recovery_acknowledges_every_prepared_batch_in_a_space() {
    let directory = tempfile::tempdir().expect("directory");
    let core = MomoCore::initialize(directory.path()).await.expect("core");
    let space = momo_domain::new_id();
    let batch = stage(&core, space, PATCH).await;
    let memory = core.memory_for_space(space).expect("memory");
    let plan = memory.prepare_patch_commit(PATCH).expect("plan");
    let encoded = serde_json::to_string(&plan).expect("json");
    core.store()
        .prepare_maintenance_commit(&batch.batch_key, &encoded)
        .await
        .expect("first journal");
    let second = MaintenanceBatch {
        batch_key: "second-recovery-batch".to_owned(),
        request_ids: Vec::new(),
        ..batch
    };
    core.store()
        .stage_maintenance_batch(&second)
        .await
        .expect("second batch");
    core.store()
        .prepare_maintenance_commit(&second.batch_key, &encoded)
        .await
        .expect("second journal");
    drop(memory);
    drop(core);

    let reopened = MomoCore::initialize(directory.path())
        .await
        .expect("recover all");
    assert!(reopened.memory_recovery_status().is_empty());
    assert!(
        reopened
            .store()
            .pending_maintenance_batches()
            .await
            .expect("journal")
            .is_empty()
    );
}

#[tokio::test]
async fn startup_recovers_before_first_write_mid_write_and_before_acknowledgement() {
    for written_files in [0, 1, 2] {
        let directory = tempfile::tempdir().expect("directory");
        let core = MomoCore::initialize(directory.path()).await.expect("core");
        let space = momo_domain::new_id();
        let batch = stage(&core, space, PATCH).await;
        let memory = core.memory_for_space(space).expect("memory");
        let plan = memory.prepare_patch_commit(PATCH).expect("validated plan");
        let encoded = serde_json::to_string(&plan).expect("encode plan");
        core.store()
            .prepare_maintenance_commit(&batch.batch_key, &encoded)
            .await
            .expect("journal");
        let files = serde_json::to_value(&plan).expect("plan files");
        for file in files["files"]
            .as_array()
            .expect("files")
            .iter()
            .take(written_files)
        {
            let content: Vec<u8> =
                serde_json::from_value(file["after"].clone()).expect("after image");
            std::fs::write(
                memory.root().join(file["path"].as_str().expect("path")),
                content,
            )
            .expect("partial commit");
        }
        drop(memory);
        drop(core);

        let reopened = MomoCore::initialize(directory.path())
            .await
            .expect("recover before publish");
        assert!(
            reopened
                .store()
                .pending_maintenance_batches()
                .await
                .expect("batches")
                .is_empty()
        );
        assert!(
            reopened
                .store()
                .pending_maintenance_turns(&space.to_string(), MaintenanceKind::Memory, 32)
                .await
                .expect("turns")
                .is_empty()
        );
        let memory = reopened.memory_for_space(space).expect("recovered memory");
        let document = memory
            .read_document_by_id("recovered_event")
            .expect("recovered index");
        assert_eq!(document.body.matches("Recovered exactly once.").count(), 1);
    }
}

#[tokio::test]
async fn recovery_conflict_preserves_journaled_edit_and_durable_plan() {
    let directory = tempfile::tempdir().expect("directory");
    let core = MomoCore::initialize(directory.path()).await.expect("core");
    let space = momo_domain::new_id();
    let batch = stage(&core, space, PATCH).await;
    let memory = core.memory_for_space(space).expect("memory");
    let plan = memory.prepare_patch_commit(PATCH).expect("plan");
    core.store()
        .prepare_maintenance_commit(
            &batch.batch_key,
            &serde_json::to_string(&plan).expect("json"),
        )
        .await
        .expect("journal");
    let path = memory.root().join("events/recovered.md");
    memory
        .call(|workspace| {
            std::fs::write(
                workspace.root().join("events/recovered.md"),
                "operator edit",
            )?;
            Ok(())
        })
        .expect("journaled edit");
    drop(memory);
    drop(core);
    let reopened = MomoCore::initialize(directory.path())
        .await
        .expect("other Spaces must start");
    assert!(reopened.memory_recovery_status().contains_key(&space));
    assert!(reopened.reserve_spaces([space]).await.is_err());
    assert!(reopened.memory_for_space(space).is_err());
    let healthy = momo_domain::new_id();
    let _healthy_guard = reopened
        .reserve_spaces([healthy])
        .await
        .expect("healthy Space available");
    reopened
        .memory_for_space(healthy)
        .expect("healthy workspace");
    assert_eq!(
        std::fs::read_to_string(&path).expect("edit preserved"),
        "operator edit"
    );
    let store = momo_storage::LocalStore::open(directory.path().join("momo.sqlite3"))
        .await
        .expect("store");
    assert_eq!(
        store
            .pending_maintenance_batches()
            .await
            .expect("durable journal")
            .len(),
        1
    );
    reopened
        .memory_for_space_unchecked(space)
        .unwrap()
        .call(|workspace| {
            std::fs::remove_file(workspace.root().join("events/recovered.md"))?;
            Ok(())
        })
        .expect("restore the missing before-image through the owner");
    let _guard = reopened
        .reserve_spaces([space])
        .await
        .expect("retry recovery without restart");
    assert!(reopened.memory_recovery_status().is_empty());
    assert!(
        reopened
            .memory_for_space(space)
            .expect("recovered Space")
            .read_document_by_id("recovered_event")
            .is_ok()
    );
}

#[tokio::test]
async fn live_pending_commit_must_recover_before_a_subsequent_write() {
    use crate::api::runtime_api::{RuntimeApiErrorKind, apply_memory_patch};
    let directory = tempfile::tempdir().expect("directory");
    let runtime = crate::MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let space = momo_domain::new_id();
    let core = runtime.core();
    let batch = stage(core, space, PATCH).await;
    let memory = core.memory_for_space(space).expect("workspace");
    let plan = memory.prepare_patch_commit(PATCH).expect("prepare");
    core.mark_memory_commit_pending(space);
    core.store()
        .prepare_maintenance_commit(
            &batch.batch_key,
            &serde_json::to_string(&plan).expect("JSON"),
        )
        .await
        .expect("persist");
    let target = memory.root().join("events/recovered.md");
    memory
        .call(|workspace| {
            std::fs::write(
                workspace.root().join("events/recovered.md"),
                "conflicting edit",
            )?;
            Ok(())
        })
        .expect("journaled conflict");
    let error = apply_memory_patch(&runtime, space.to_string(), "patches: []".to_owned())
        .await
        .expect_err("must not write past pending commit");
    assert_eq!(error.kind, RuntimeApiErrorKind::Conflict);
    assert_eq!(
        std::fs::read_to_string(&target).expect("preserved"),
        "conflicting edit"
    );
    apply_memory_patch(
        &runtime,
        momo_domain::new_id().to_string(),
        "patches: []".to_owned(),
    )
    .await
    .expect("unrelated writer");
    memory
        .call(|workspace| {
            std::fs::remove_file(workspace.root().join("events/recovered.md"))?;
            Ok(())
        })
        .expect("restore before-image");
    apply_memory_patch(&runtime, space.to_string(), "patches: []".to_owned())
        .await
        .expect("recover before next write");
    assert!(
        core.store()
            .pending_maintenance_batches()
            .await
            .expect("journal")
            .is_empty()
    );
    assert!(memory.read_document_by_id("recovered_event").is_ok());
}

#[tokio::test]
async fn moc_import_keeps_space_ownership_after_its_caller_is_cancelled() {
    let directory = tempfile::tempdir().expect("directory");
    let source = MomoCore::initialize(directory.path().join("source"))
        .await
        .expect("source");
    let space = momo_domain::new_id();
    source
        .memory_for_space(space)
        .expect("workspace")
        .apply_patch(PATCH)
        .expect("fixture");
    let package = directory.path().join("source.moc");
    let export = crate::MocExportPlan {
        memory: vec![space],
        characters: Vec::new(),
        conversations: Vec::new(),
        semantic_graph: Vec::new(),
        compatibility: crate::MocCompatibility::None,
    };
    crate::export_moc(&source, &package, &export)
        .await
        .expect("export");
    let target = MomoCore::initialize(directory.path().join("target"))
        .await
        .expect("target");
    let guard = target.reserve_spaces([space]).await.expect("block import");
    let owned = target.clone();
    let caller = tokio::spawn(async move {
        crate::import_moc(
            &owned,
            package,
            &crate::MocImportPlan {
                space_map: Default::default(),
                conflict_mode: crate::ConflictMode::Replace,
            },
        )
        .await
    });
    // lock_spaces itself leaves a completed task; wait for the live import task.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if target
                .commit_tasks
                .call(|tasks| tasks.iter().any(|task| !task.is_finished()))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("import dispatched");
    assert!(!caller.is_finished());
    caller.abort();
    assert!(caller.await.expect_err("caller cancelled").is_cancelled());
    let path = target
        .memory_for_space(space)
        .expect("target memory")
        .root()
        .join("events/recovered.md");
    assert!(
        !path.exists(),
        "import may not bypass the existing Space lock"
    );
    drop(guard);
    target.wait_for_commits().await;
    assert!(path.exists(), "runtime must finish the admitted import");
}

#[tokio::test]
async fn moc_export_waits_for_the_same_space_writer() {
    let directory = tempfile::tempdir().expect("directory");
    let core = MomoCore::initialize(directory.path().join("data"))
        .await
        .expect("core");
    let space = momo_domain::new_id();
    let guard = core.reserve_spaces([space]).await.expect("writer");
    let plan = crate::MocExportPlan {
        memory: vec![space],
        characters: Vec::new(),
        conversations: Vec::new(),
        semantic_graph: Vec::new(),
        compatibility: crate::MocCompatibility::None,
    };
    let output = directory.path().join("snapshot.moc");
    let mut export = Box::pin(crate::export_moc(&core, &output, &plan));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut export)
            .await
            .is_err()
    );
    core.memory_for_space(space)
        .expect("workspace")
        .apply_patch(PATCH)
        .expect("write while owned");
    drop(guard);
    export.await.expect("consistent snapshot");
    assert!(output.exists());
}

#[tokio::test]
async fn staged_batch_keeps_original_evidence_when_threshold_changes() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = Arc::new(
        crate::MomoRuntime::initialize(directory.path())
            .await
            .expect("runtime"),
    );
    let space = momo_domain::new_id();
    stage(runtime.core(), space, "patches: []").await;
    runtime
        .core()
        .store()
        .append_maintenance_turn(
            &MaintenanceTurn {
                request_id: "later-turn".to_owned(),
                scope_id: space.to_string(),
                user_content: "later".to_owned(),
                assistant_content: "later".to_owned(),
            },
            true,
            false,
        )
        .await
        .expect("later turn");
    let service = {
        let runtime = runtime.clone();
        runtime
            .update_runtime_settings((*Arc::new(crate::MomoRuntimeSettings::default())).clone())
            .expect("runtime settings");
        crate::MomoApiService::new(
            runtime,
            "http://127.0.0.1:1/v1",
            None,
            reqwest::Client::new(),
        )
    };
    assert!(
        service
            .maintain(&space.to_string(), crate::MaintenanceKind::Memory, 32)
            .await
            .expect("resume without model")
    );
    let pending = runtime
        .core()
        .store()
        .pending_maintenance_turns(&space.to_string(), MaintenanceKind::Memory, 32)
        .await
        .expect("remaining evidence");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request_id, "later-turn");
}
