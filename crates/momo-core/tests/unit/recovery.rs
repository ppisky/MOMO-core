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
async fn recovery_conflict_preserves_operator_edit_and_durable_plan() {
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
    std::fs::write(&path, "operator edit").expect("edit");
    drop(memory);
    drop(core);
    let reopened = MomoCore::initialize(directory.path())
        .await
        .expect("other Spaces must start");
    assert!(reopened.memory_recovery_status().contains_key(&space));
    assert!(reopened.lock_spaces([space]).await.is_err());
    assert!(reopened.memory_for_space(space).is_err());
    let healthy = momo_domain::new_id();
    let _healthy_guard = reopened
        .lock_spaces([healthy])
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
    std::fs::remove_file(&path).expect("operator restores the missing before-image");
    let _guard = reopened
        .lock_spaces([space])
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
    std::fs::write(&target, "conflicting edit").expect("conflict");
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
    std::fs::remove_file(&target).expect("restore before-image");
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
    let guard = target.lock_spaces([space]).await.expect("block import");
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
                .lock()
                .expect("tasks")
                .iter()
                .any(|task| !task.is_finished())
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
    let guard = core.lock_spaces([space]).await.expect("writer");
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
