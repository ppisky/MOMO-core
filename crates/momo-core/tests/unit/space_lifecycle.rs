use super::*;
use chrono::Utc;
use momo_domain::provenance::{Evidence, EvidenceKind, FactKind, MemoryIdentity, RecordPolicy};
use momo_domain::{CharacterCard, Conversation, Message, MessageRole};

async fn seed(core: &MomoCore, space: Uuid) -> (CharacterCard, Conversation, Message) {
    let now = Utc::now();
    let character = CharacterCard {
        id: Uuid::now_v7(),
        scope_id: space,
        name: "Character".into(),
        version: "1.0.0".into(),
        author_name: "Test".into(),
        author_url: None,
        character_markdown: "Persona".into(),
        user_markdown: String::new(),
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    core.store.save_character(&character).await.unwrap();
    let conversation = Conversation {
        id: Uuid::now_v7(),
        scope_id: space,
        character_id: Some(character.id),
        title: "Story".into(),
        archived_at: None,
        created_at: now,
        updated_at: now,
    };
    core.store.save_conversation(&conversation).await.unwrap();
    let message = Message {
        id: Uuid::now_v7(),
        conversation_id: conversation.id,
        role: MessageRole::User,
        content: "Promise".into(),
        created_at: now,
    };
    core.store.save_message(&message).await.unwrap();
    (character, conversation, message)
}

fn request(source: Uuid, revision: String, disposition: SpaceDisposition) -> SpaceRemovalRequest {
    SpaceRemovalRequest {
        operation_id: Uuid::now_v7(),
        source_id: source,
        expected_revision: revision,
        disposition,
    }
}

fn add_memory(core: &MomoCore, space: Uuid, character: Uuid, conversation: Uuid) {
    let memory = core.memory_for_space(space).unwrap();
    let identity = MemoryIdentity {
        personal_space_id: space,
        conversation_id: conversation,
        character_id: character,
        continuity_id: conversation,
        function_id: None,
    };
    let patch = format!(
        "patches:\n  - target_file: events/promise.md\n    operations:\n      - type: create\n        frontmatter:\n          id: promise\n          type: event\n          importance: 0.7\n          weight: 0.9\n          decay_at: 0\n          status: active\n          injection_scope: conversation\n          injection_conversation_id: {conversation}\n          injection_character_id: {character}\n        content: '# Promise'\n"
    );
    let mut context = RevisionContext::manual("test");
    context.identity = Some(identity);
    context.evidence.push(Evidence {
        id: "original".into(),
        kind: EvidenceKind::Conversation,
        original_space_id: space,
        personal_space_id: space,
        conversation_id: Some(conversation),
        character_id: Some(character),
        message_ids: vec![],
        configuration: BTreeMap::new(),
    });
    let plan = memory
        .trace_commit(
            memory.prepare_patch_commit(&patch).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    memory.apply_prepared_commit(&plan).unwrap();
    memory
        .set_record_policy(
            false,
            "promise",
            RecordPolicy {
                revision: 2,
                local_space_id: space,
                personal_space_id: space,
                owner_character_id: Some(character),
                readers: vec![],
                continuity_ids: vec![conversation],
                function_ids: vec![],
                fact_kind: FactKind::Experience,
                subjects: vec![],
                participants: vec![character],
                responsible_characters: vec![character],
                valid_from: None,
                valid_until: None,
            },
        )
        .unwrap();
    memory.nsg().unwrap().apply_patch("patches:\n  - target_file: lore/place.nsg\n    operations:\n      - type: create_node\n        metadata:\n          id: place\n          type: lore\n          importance: 0.9\n          mode: draft\n          status: active\n          zone: auto\n        anchors: place\n        condition: always\n        trigger: enter\n        consequence: remember\n        constraint: none\n").unwrap();
}

#[tokio::test]
async fn moves_owned_assets_preserving_ids_scope_provenance_and_replays_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let (character, conversation, message) = seed(&core, source).await;
    let archived = core
        .store
        .set_conversation_archived(source, conversation.id, true)
        .await
        .unwrap()
        .unwrap();
    add_memory(&core, source, character.id, conversation.id);
    core.store.stage_message_delete(message.id).await.unwrap();
    let before = core.space_assets(source).await.unwrap();
    assert_eq!(
        (
            before.characters,
            before.conversations,
            before.memories,
            before.nsg_nodes,
            before.deleted_items
        ),
        (1, 1, 1, 1, 1)
    );
    let operation = request(
        source,
        before.revision,
        SpaceDisposition::Migrate { target_id: target },
    );
    let receipt = core.remove_space_assets(operation.clone()).await.unwrap();
    assert_eq!(
        receipt,
        core.remove_space_assets(operation.clone()).await.unwrap()
    );
    assert_eq!(
        core.store
            .character_by_id(character.id)
            .await
            .unwrap()
            .unwrap()
            .scope_id,
        target
    );
    assert_eq!(
        core.store
            .conversation_for_scope(target, conversation.id)
            .await
            .unwrap()
            .unwrap()
            .scope_id,
        target
    );
    let memory = core.memory_for_space(target).unwrap();
    assert_eq!(
        core.store
            .conversation_for_scope(target, conversation.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at,
        archived.archived_at
    );
    let document = memory.read_document_by_id("promise").unwrap();
    assert_eq!(
        document.metadata.injection_conversation_id,
        Some(conversation.id.to_string())
    );
    assert_eq!(
        document.metadata.injection_character_id,
        Some(character.id.to_string())
    );
    let history = &memory.provenance(false).unwrap().records["promise"];
    assert_eq!(history.policy.as_ref().unwrap().local_space_id, target);
    assert_eq!(history.policy.as_ref().unwrap().personal_space_id, target);
    assert_eq!(history.evidence["original"].original_space_id, source);
    assert_eq!(history.evidence["original"].personal_space_id, source);
    assert_eq!(
        history.revisions.last().unwrap().context.trigger,
        "space_migration"
    );
    assert!(core.memory_for_space(source).is_err());
    assert!(
        core.store
            .restore_recently_deleted("message", message.id)
            .await
            .unwrap()
    );
    assert_eq!(
        core.store.list_messages(conversation.id).await.unwrap()[0].id,
        message.id
    );
    drop(memory);
    core.wait_for_commits().await;
    drop(core);
    let restarted = MomoCore::initialize(directory.path()).await.unwrap();
    assert!(restarted.memory_for_space(source).is_err());
    assert_eq!(
        restarted.remove_space_assets(operation).await.unwrap(),
        receipt
    );
    assert_eq!(restarted.space_assets(target).await.unwrap().memories, 1);
}

#[tokio::test]
async fn destructive_delete_removes_owned_assets_and_preserves_external_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let other = Uuid::now_v7();
    let (character, conversation, message) = seed(&core, source).await;
    add_memory(&core, source, character.id, conversation.id);
    let (_, mut external, external_message) = seed(&core, other).await;
    external.character_id = Some(character.id);
    core.store.save_conversation(&external).await.unwrap();
    core.store.stage_message_delete(message.id).await.unwrap();
    let assets = core.space_assets(source).await.unwrap();
    assert_eq!(assets.external_conversations, 1);
    core.remove_space_assets(request(
        source,
        assets.revision,
        SpaceDisposition::DeleteAssets,
    ))
    .await
    .unwrap();
    assert!(
        core.store
            .character_by_id(character.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        core.store
            .conversation_for_scope(source, conversation.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !core
            .store
            .restore_recently_deleted("message", message.id)
            .await
            .unwrap()
    );
    assert_eq!(
        core.store
            .conversation_for_scope(other, external.id)
            .await
            .unwrap()
            .unwrap()
            .character_id,
        None
    );
    assert_eq!(
        core.store.list_messages(external.id).await.unwrap()[0].id,
        external_message.id
    );
    assert!(core.memory_for_space(source).is_err());
    let snapshot = core
        .memory_for_space_unchecked(source)
        .unwrap()
        .export_snapshot()
        .unwrap();
    assert!(snapshot.files.is_empty());
    core.store.save_character(&character).await.unwrap(); // Tombstone refuses resurrection.
    assert!(
        core.store
            .character_by_id(character.id)
            .await
            .unwrap()
            .is_none()
    );
    let mut new_character = character.clone();
    new_character.id = Uuid::now_v7();
    assert!(core.store.save_character(&new_character).await.is_err());
}

#[tokio::test]
async fn stale_confirmation_and_target_conflicts_do_not_prepare_or_delete() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let (character, conversation, _) = seed(&core, source).await;
    let before = core.space_assets(source).await.unwrap();
    let (_, _, _) = seed(&core, source).await;
    let error = core
        .remove_space_assets(request(
            source,
            before.revision,
            SpaceDisposition::DeleteAssets,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, Some("space_assets_changed"));
    add_memory(&core, source, character.id, conversation.id);
    add_memory(&core, target, character.id, conversation.id);
    let before = core.space_assets(source).await.unwrap();
    let error = core
        .remove_space_assets(request(
            source,
            before.revision,
            SpaceDisposition::Migrate { target_id: target },
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, Some("space_migration_conflict"));
    assert!(core.memory_for_space(source).is_ok());
    assert!(core.memory_for_space(target).is_ok());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM space_removals")
        .fetch_one(core.store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn restart_finishes_prepared_after_images_before_publishing_core() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let (character, conversation, _) = seed(&core, source).await;
    add_memory(&core, source, character.id, conversation.id);
    let assets = core.space_assets(source).await.unwrap();
    let operation = request(
        source,
        assets.revision.clone(),
        SpaceDisposition::Migrate { target_id: target },
    );
    let source_image = core
        .memory_for_space(source)
        .unwrap()
        .export_snapshot()
        .unwrap();
    let target_image = core
        .memory_for_space(target)
        .unwrap()
        .export_snapshot()
        .unwrap();
    let plan = RemovalPlan {
        target: Some(
            merge_snapshots(
                source_image,
                target_image,
                source,
                target,
                operation.operation_id,
                directory.path(),
            )
            .unwrap(),
        ),
        settings: None,
    };
    let receipt = SpaceRemovalReceipt {
        operation_id: operation.operation_id,
        source_id: source,
        disposition: operation.disposition.clone(),
        assets,
    };
    sqlx::query("INSERT INTO space_removals(operation_id,source_id,request_json,plan_json,receipt_json) VALUES(?,?,?,?,?)")
        .bind(operation.operation_id.to_string()).bind(source.to_string()).bind(serde_json::to_string(&operation).unwrap()).bind(serde_json::to_string(&plan).unwrap()).bind(serde_json::to_string(&receipt).unwrap()).execute(core.store.pool()).await.unwrap();
    // Crash point: destination already materialized, SQL ownership still source.
    let image = plan.target.unwrap();
    core.memory_for_space_unchecked(target)
        .unwrap()
        .call(move |workspace| workspace.import_snapshot(&image))
        .unwrap();
    drop(core);
    let recovered = MomoCore::initialize(directory.path()).await.unwrap();
    assert_eq!(
        recovered.remove_space_assets(operation).await.unwrap(),
        receipt
    );
    assert_eq!(
        recovered
            .store
            .conversation_for_scope(target, conversation.id)
            .await
            .unwrap()
            .unwrap()
            .scope_id,
        target
    );
    assert_eq!(recovered.space_assets(target).await.unwrap().memories, 1);
}

#[tokio::test]
async fn removal_waits_for_admitted_work_and_rejects_changed_replay() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let assets = core.space_assets(source).await.unwrap();
    let operation = request(source, assets.revision, SpaceDisposition::DeleteAssets);
    let admission = core.admit_space_operation().await;
    let task = {
        let core = core.clone();
        let operation = operation.clone();
        tokio::spawn(async move { core.remove_space_assets(operation).await })
    };
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    drop(admission);
    task.await.unwrap().unwrap();
    let mut changed = operation;
    changed.disposition = SpaceDisposition::Migrate {
        target_id: Uuid::now_v7(),
    };
    assert!(core.remove_space_assets(changed).await.is_err());
}

#[tokio::test]
async fn sql_failure_fences_both_spaces_and_retry_finishes_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let previous_source = Uuid::now_v7();
    let previous_target = Uuid::now_v7();
    let previous_assets = core.space_assets(previous_source).await.unwrap();
    let previous = request(
        previous_source,
        previous_assets.revision,
        SpaceDisposition::Migrate {
            target_id: previous_target,
        },
    );
    core.remove_space_assets(previous.clone()).await.unwrap();
    let previous_target_assets = core.space_assets(previous_target).await.unwrap();
    core.remove_space_assets(request(
        previous_target,
        previous_target_assets.revision,
        SpaceDisposition::DeleteAssets,
    ))
    .await
    .unwrap();
    core.remove_space_assets(previous.clone()).await.unwrap();
    assert!(core.memory_for_space(previous_target).is_err());
    let (character, conversation, _) = seed(&core, source).await;
    add_memory(&core, source, character.id, conversation.id);
    let before = core.space_assets(source).await.unwrap();
    sqlx::query("CREATE TRIGGER test_migration_failure BEFORE UPDATE ON character_cards BEGIN SELECT RAISE(ABORT,'test crash after files'); END").execute(core.store.pool()).await.unwrap();
    let operation = request(
        source,
        before.revision,
        SpaceDisposition::Migrate { target_id: target },
    );
    assert!(core.remove_space_assets(operation.clone()).await.is_err());
    assert!(core.memory_for_space(source).is_err());
    assert!(core.memory_for_space(target).is_err());
    assert!(core.admit_settings_operation().is_err());
    core.remove_space_assets(previous).await.unwrap();
    assert!(core.space_transition_pending());
    assert!(core.admit_settings_operation().is_err());
    assert!(core.memory_for_space(previous_target).is_err());
    assert_eq!(
        core.store
            .character_by_id(character.id)
            .await
            .unwrap()
            .unwrap()
            .scope_id,
        source
    );
    sqlx::query("DROP TRIGGER test_migration_failure")
        .execute(core.store.pool())
        .await
        .unwrap();
    core.remove_space_assets(operation.clone()).await.unwrap();
    assert_eq!(core.space_assets(target).await.unwrap().memories, 1);
    assert!(core.admit_settings_operation().is_ok());
    core.remove_space_assets(operation).await.unwrap();
    assert_eq!(core.space_assets(target).await.unwrap().memories, 1);
}

#[tokio::test]
async fn runtime_overrides_migrate_and_publish_without_resetting_instance_or_other_spaces() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = crate::MomoRuntime::initialize(directory.path())
        .await
        .unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let other = Uuid::now_v7();
    runtime
        .patch_runtime_settings(
            Some(source),
            serde_json::json!({"runtime":{"memory_distill_every_turns":7}}),
            None,
        )
        .unwrap();
    runtime
        .patch_runtime_settings(
            Some(other),
            serde_json::json!({"runtime":{"memory_distill_every_turns":9}}),
            None,
        )
        .unwrap();
    let assets = runtime.core().space_assets(source).await.unwrap();
    assert_eq!(assets.settings_overrides, 1);
    runtime
        .remove_space_assets(request(
            source,
            assets.revision,
            SpaceDisposition::Migrate { target_id: target },
        ))
        .await
        .unwrap();
    assert_eq!(
        runtime.runtime_settings_resource(Some(source)).source,
        "instance"
    );
    assert!(
        runtime
            .patch_runtime_settings(
                Some(source),
                serde_json::json!({"runtime":{"memory_distill_every_turns":11}}),
                None
            )
            .is_err()
    );
    assert_eq!(
        runtime
            .runtime_settings_for_space(target)
            .runtime
            .memory_distill_every_turns,
        7
    );
    assert_eq!(
        runtime
            .runtime_settings_for_space(other)
            .runtime
            .memory_distill_every_turns,
        9
    );
    let assets = runtime.core().space_assets(target).await.unwrap();
    runtime
        .remove_space_assets(request(
            target,
            assets.revision,
            SpaceDisposition::DeleteAssets,
        ))
        .await
        .unwrap();
    assert_eq!(
        runtime.runtime_settings_resource(Some(target)).source,
        "instance"
    );
}

#[tokio::test]
async fn default_assistant_is_owned_configuration_and_conflicts_are_not_discarded() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let (source_character, _, _) = seed(&core, source).await;
    let (target_character, _, _) = seed(&core, target).await;
    for (space, character) in [(source, source_character.id), (target, target_character.id)] {
        sqlx::query("INSERT INTO default_assistants(personal_space_id,character_id) VALUES(?,?)")
            .bind(space.to_string())
            .bind(character.to_string())
            .execute(core.store.pool())
            .await
            .unwrap();
    }
    let before = core.space_assets(source).await.unwrap();
    assert_eq!(before.settings_overrides, 1);
    assert_eq!(
        core.remove_space_assets(request(
            source,
            before.revision.clone(),
            SpaceDisposition::Migrate { target_id: target }
        ))
        .await
        .unwrap_err()
        .code,
        Some("space_migration_conflict")
    );
    sqlx::query("DELETE FROM default_assistants WHERE personal_space_id=?")
        .bind(target.to_string())
        .execute(core.store.pool())
        .await
        .unwrap();
    core.remove_space_assets(request(
        source,
        before.revision,
        SpaceDisposition::Migrate { target_id: target },
    ))
    .await
    .unwrap();
    let character: String =
        sqlx::query_scalar("SELECT character_id FROM default_assistants WHERE personal_space_id=?")
            .bind(target.to_string())
            .fetch_one(core.store.pool())
            .await
            .unwrap();
    assert_eq!(character, source_character.id.to_string());
    assert!(
        sqlx::query("INSERT INTO default_assistants(personal_space_id,character_id) VALUES(?,?)")
            .bind(source.to_string())
            .bind(source_character.id.to_string())
            .execute(core.store.pool())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn matching_current_body_does_not_discard_different_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(directory.path()).await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let mut snapshot = core
        .memory_for_space(source)
        .unwrap()
        .export_snapshot()
        .unwrap();
    let mut document =
        momo_memory::MemoryDocument::parse(&snapshot.files["current/scene.md"]).unwrap();
    document.metadata.tags = vec!["source-world".into()];
    snapshot
        .files
        .insert("current/scene.md".into(), document.encode().unwrap());
    let destination = core
        .memory_for_space(target)
        .unwrap()
        .export_snapshot()
        .unwrap();
    assert!(
        merge_snapshots(
            snapshot,
            destination,
            source,
            target,
            Uuid::now_v7(),
            directory.path()
        )
        .is_err()
    );
}
