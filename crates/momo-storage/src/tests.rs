use super::*;

#[tokio::test]
async fn migrated_schema_uses_scope_id_exclusively() {
    let store = LocalStore::in_memory().await.expect("store");
    for table_info_sql in [
        "PRAGMA table_info(character_cards)",
        "PRAGMA table_info(conversations)",
        "PRAGMA table_info(memory_patch_reviews)",
    ] {
        let columns = sqlx::query(table_info_sql)
            .fetch_all(&store.pool)
            .await
            .expect("table info")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect::<Vec<_>>();
        assert!(columns.iter().any(|column| column == "scope_id"));
        assert!(!columns.iter().any(|column| column == "owner_id"));
    }
    let vector_columns = sqlx::query("PRAGMA table_info(nsg_vectors)")
        .fetch_all(&store.pool)
        .await
        .expect("legacy vector table info");
    assert!(vector_columns.is_empty());
}

fn mo_state_observation(operation_id: &str, event_fingerprint: &str) -> MoStateObservation {
    MoStateObservation {
        operation_id: operation_id.to_owned(),
        space_id: "01900000-0000-7000-8000-000000000101".to_owned(),
        event_type: "user_message".to_owned(),
        event_fingerprint: event_fingerprint.to_owned(),
        profile: "closed_autonomous".to_owned(),
        dmw_fingerprint: "dmw-a".to_owned(),
        nsg_fingerprint: "nsg-a".to_owned(),
        scene_fingerprint: "scene-a".to_owned(),
        scene_json: r#"{"scene_id":"scene_initial","status":"inactive"}"#.to_owned(),
    }
}

#[tokio::test]
async fn mo_state_runtime_versions_sources_and_replays_snapshots() {
    let store = LocalStore::in_memory().await.expect("store");
    let observation = mo_state_observation("state-operation-1", "event-a");
    let operation = store
        .observe_mo_state_operation(&observation)
        .await
        .expect("observe");
    assert_eq!(operation.base_dmw_revision, 1);
    assert_eq!(operation.base_nsg_revision, 1);
    assert_eq!(operation.base_scene_revision, 1);

    let state_result = r#"{"context":"[STATE_CONTEXT]","audit":{"dimensions_active":2}}"#;
    let snapshot = store
        .publish_mo_state_snapshot("state-operation-1", state_result, false, None)
        .await
        .expect("publish");
    assert_eq!(snapshot.snapshot_revision, 1);
    assert_eq!(snapshot.state_context, "[STATE_CONTEXT]");

    let replayed_operation = store
        .observe_mo_state_operation(&observation)
        .await
        .expect("replay observation");
    let replayed = store
        .publish_mo_state_snapshot("state-operation-1", state_result, false, None)
        .await
        .expect("replay snapshot");
    assert!(replayed_operation.snapshot_json.is_some());
    assert_eq!(replayed, snapshot);

    let mut second = mo_state_observation("state-operation-2", "event-b");
    second.dmw_fingerprint = "dmw-b".to_owned();
    second.scene_fingerprint = "scene-b".to_owned();
    let operation = store
        .observe_mo_state_operation(&second)
        .await
        .expect("observe changed sources");
    assert_eq!(operation.base_dmw_revision, 2);
    assert_eq!(operation.base_nsg_revision, 1);
    assert_eq!(operation.base_scene_revision, 2);

    let status = store
        .mo_state_runtime_status(&observation.space_id)
        .await
        .expect("status")
        .expect("runtime");
    assert_eq!(status.snapshot_revision, 1);
    assert_eq!(status.current_snapshot, Some(snapshot));
}

#[tokio::test]
async fn mo_state_operation_identity_is_immutable() {
    let store = LocalStore::in_memory().await.expect("store");
    let original = mo_state_observation("state-operation-conflict", "event-a");
    store
        .observe_mo_state_operation(&original)
        .await
        .expect("observe");
    let changed = mo_state_observation("state-operation-conflict", "event-b");
    assert!(matches!(
        store.observe_mo_state_operation(&changed).await,
        Err(StorageError::MoStateOperationConflict(_))
    ));
}

#[tokio::test]
async fn response_operations_survive_reopen_semantics() {
    let store = LocalStore::in_memory().await.expect("store");
    store
        .begin_response_operation(
            "request-1",
            "fingerprint-1",
            "conversation-1",
            r#"{"text":"hello"}"#,
        )
        .await
        .expect("begin response");
    let pending = store
        .response_operation("request-1")
        .await
        .expect("read pending")
        .expect("pending operation");
    assert!(!pending.user_written);
    assert_eq!(
        pending.resolved_input_json.as_deref(),
        Some(r#"{"text":"hello"}"#)
    );
    assert!(pending.response_json.is_none());

    store
        .mark_response_user_written("request-1")
        .await
        .expect("mark user message");
    store
        .complete_response_operation("request-1", r#"{"status":"completed"}"#)
        .await
        .expect("complete response");
    let completed = store
        .response_operation("request-1")
        .await
        .expect("read completed")
        .expect("completed operation");
    assert!(completed.user_written);
    assert_eq!(
        completed.response_json.as_deref(),
        Some(r#"{"status":"completed"}"#)
    );

    store
        .begin_response_operation(
            "request-1",
            "different",
            "different",
            r#"{"text":"different"}"#,
        )
        .await
        .expect("duplicate begin is idempotent");
    let unchanged = store
        .response_operation("request-1")
        .await
        .expect("read unchanged")
        .expect("unchanged operation");
    assert_eq!(unchanged.request_fingerprint, "fingerprint-1");
    assert_eq!(unchanged.conversation_id, "conversation-1");
}

#[tokio::test]
async fn control_operations_claim_and_replay_persistently() {
    let store = LocalStore::in_memory().await.expect("store");
    assert!(
        store
            .begin_control_operation("control-1", "fingerprint-1")
            .await
            .expect("claim control")
    );
    assert!(
        !store
            .begin_control_operation("control-1", "fingerprint-1")
            .await
            .expect("duplicate claim")
    );
    store
        .complete_control_operation("control-1", r#"{"status":"completed"}"#)
        .await
        .expect("complete control");
    let operation = store
        .control_operation("control-1")
        .await
        .expect("read control")
        .expect("stored control");
    assert_eq!(operation.request_fingerprint, "fingerprint-1");
    assert_eq!(
        operation.response_json.as_deref(),
        Some(r#"{"status":"completed"}"#)
    );
}

#[tokio::test]
async fn reopening_releases_only_interrupted_control_claims() {
    let path =
        std::env::temp_dir().join(format!("momo-control-recovery-{}.sqlite3", Uuid::new_v4()));
    let store = LocalStore::open(&path).await.expect("store");
    assert!(
        store
            .begin_control_operation("pending", "pending-fingerprint")
            .await
            .expect("claim pending control")
    );
    assert!(
        store
            .begin_control_operation("completed", "completed-fingerprint")
            .await
            .expect("claim completed control")
    );
    store
        .complete_control_operation("completed", r#"{"status":"completed"}"#)
        .await
        .expect("complete control");
    store.pool.close().await;

    let reopened = LocalStore::open(&path).await.expect("reopen store");
    assert!(
        reopened
            .control_operation("pending")
            .await
            .expect("read interrupted control")
            .is_none()
    );
    assert_eq!(
        reopened
            .control_operation("completed")
            .await
            .expect("read completed control")
            .expect("completed control")
            .response_json
            .as_deref(),
        Some(r#"{"status":"completed"}"#)
    );
    reopened.pool.close().await;
    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn response_user_message_and_phase_marker_commit_atomically() {
    let store = LocalStore::in_memory().await.expect("store");
    let scope_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let now = Utc::now();
    store
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id,
            character_id: None,
            title: "idempotency".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("conversation");
    store
        .begin_response_operation(
            "request-atomic",
            "fingerprint-atomic",
            &conversation_id.to_string(),
            r#"{"text":"only once"}"#,
        )
        .await
        .expect("operation");
    let first = Message {
        id: Uuid::new_v4(),
        conversation_id,
        role: MessageRole::User,
        content: "only once".to_owned(),
        created_at: now,
    };
    assert!(
        store
            .append_response_user_message("request-atomic", scope_id, &first)
            .await
            .expect("first append")
    );
    let retry = Message {
        id: Uuid::new_v4(),
        ..first.clone()
    };
    assert!(
        !store
            .append_response_user_message("request-atomic", scope_id, &retry)
            .await
            .expect("idempotent retry")
    );
    assert_eq!(
        store
            .list_messages(conversation_id)
            .await
            .expect("messages"),
        vec![first]
    );
    assert!(
        store
            .response_operation("request-atomic")
            .await
            .expect("operation")
            .expect("stored operation")
            .user_written
    );
}

#[tokio::test]
async fn response_completion_commits_assistant_maintenance_and_replay_once() {
    let store = LocalStore::in_memory().await.expect("store");
    let scope_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let now = Utc::now();
    store
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id,
            character_id: None,
            title: "atomic completion".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("conversation");
    store
        .begin_response_operation(
            "request-completion",
            "fingerprint-completion",
            &conversation_id.to_string(),
            r#"{"text":"hello"}"#,
        )
        .await
        .expect("operation");
    let assistant = Message {
        id: Uuid::new_v4(),
        conversation_id,
        role: MessageRole::Assistant,
        content: "completed answer".to_owned(),
        created_at: now,
    };
    let maintenance = MaintenanceTurn {
        request_id: "request-completion".to_owned(),
        scope_id: scope_id.to_string(),
        user_content: "hello".to_owned(),
        assistant_content: "completed answer".to_owned(),
    };
    let state_observation = MoStateObservation {
        operation_id: "request-completion".to_owned(),
        space_id: scope_id.to_string(),
        event_type: "user_message".to_owned(),
        event_fingerprint: "state-event-completion".to_owned(),
        profile: "closed_autonomous".to_owned(),
        dmw_fingerprint: "dmw-completion".to_owned(),
        nsg_fingerprint: "nsg-completion".to_owned(),
        scene_fingerprint: "scene-completion".to_owned(),
        scene_json: "{}".to_owned(),
    };
    store
        .observe_mo_state_operation(&state_observation)
        .await
        .expect("state operation");
    store
        .publish_mo_state_snapshot(
            "request-completion",
            r#"{"context":"state","audit":{}}"#,
            false,
            None,
        )
        .await
        .expect("state snapshot");
    assert!(
        store
            .commit_response_completion(ResponseCompletion {
                request_id: "request-completion",
                conversation_scope_id: scope_id,
                assistant_message: Some(&assistant),
                maintenance_turn: Some(&maintenance),
                memory_enabled: true,
                nsg_enabled: true,
                mo_state_operation_id: Some("request-completion"),
                response_json: r#"{"status":"completed"}"#,
            })
            .await
            .expect("commit completion")
    );
    assert_eq!(
        store
            .list_messages(conversation_id)
            .await
            .expect("messages"),
        vec![assistant]
    );
    assert_eq!(
        store
            .pending_maintenance_turns(&scope_id.to_string(), MaintenanceKind::Memory, 10)
            .await
            .expect("maintenance"),
        vec![maintenance]
    );
    assert_eq!(
        store
            .response_operation("request-completion")
            .await
            .expect("operation")
            .expect("stored operation")
            .response_json
            .as_deref(),
        Some(r#"{"status":"completed"}"#)
    );
    assert_eq!(
        store
            .observe_mo_state_operation(&state_observation)
            .await
            .expect("completed state operation")
            .phase,
        "completed"
    );

    let retry = Message {
        id: Uuid::new_v4(),
        conversation_id,
        role: MessageRole::Assistant,
        content: "must not be appended".to_owned(),
        created_at: Utc::now(),
    };
    assert!(
        !store
            .commit_response_completion(ResponseCompletion {
                request_id: "request-completion",
                conversation_scope_id: scope_id,
                assistant_message: Some(&retry),
                maintenance_turn: None,
                memory_enabled: false,
                nsg_enabled: false,
                mo_state_operation_id: None,
                response_json: r#"{"status":"different"}"#,
            })
            .await
            .expect("idempotent completion replay")
    );
    assert_eq!(
        store
            .list_messages(conversation_id)
            .await
            .expect("unchanged messages")
            .len(),
        1
    );
}

#[tokio::test]
async fn maintenance_batch_reuses_patch_and_acknowledges_atomically() {
    let store = LocalStore::in_memory().await.expect("store");
    let scope_id = Uuid::new_v4().to_string();
    let turn = MaintenanceTurn {
        request_id: "maintenance-turn-1".to_owned(),
        scope_id: scope_id.clone(),
        user_content: "user".to_owned(),
        assistant_content: "assistant".to_owned(),
    };
    store
        .append_maintenance_turn(&turn, true, false)
        .await
        .expect("turn");
    let batch = MaintenanceBatch {
        batch_key: "maintenance-batch-1".to_owned(),
        scope_id,
        kind: "memory".to_owned(),
        request_ids: vec![turn.request_id.clone()],
        patch_yaml: "patch-v1".to_owned(),
    };
    assert_eq!(
        store
            .stage_maintenance_batch(&batch)
            .await
            .expect("stage batch"),
        "patch-v1"
    );
    let mut retry = batch.clone();
    retry.patch_yaml = "different-regeneration".to_owned();
    assert_eq!(
        store
            .stage_maintenance_batch(&retry)
            .await
            .expect("reuse original batch"),
        "patch-v1"
    );
    assert!(
        store
            .discard_maintenance_batch(&batch.batch_key)
            .await
            .expect("discard invalid patch")
    );
    assert!(
        store
            .maintenance_batch_patch(&batch.batch_key)
            .await
            .expect("read discarded batch")
            .is_none()
    );
    assert_eq!(
        store
            .pending_maintenance_turns(&batch.scope_id, MaintenanceKind::Memory, 10)
            .await
            .expect("pending turn survives discard"),
        vec![turn.clone()]
    );
    assert!(
        !store
            .discard_maintenance_batch(&batch.batch_key)
            .await
            .expect("discard is idempotent")
    );
    store
        .stage_maintenance_batch(&batch)
        .await
        .expect("restage valid patch");
    store
        .complete_maintenance_batch(
            &batch.batch_key,
            &batch.request_ids,
            MaintenanceKind::Memory,
        )
        .await
        .expect("complete batch");
    assert!(
        store
            .maintenance_batch_patch(&batch.batch_key)
            .await
            .expect("read batch")
            .is_none()
    );
    assert!(
        store
            .pending_maintenance_turns(&batch.scope_id, MaintenanceKind::Memory, 10)
            .await
            .expect("pending turns")
            .is_empty()
    );

    let cleanup_batch = MaintenanceBatch {
        batch_key: "maintenance-batch-cleanup".to_owned(),
        patch_yaml: "cleanup".to_owned(),
        ..batch
    };
    store
        .stage_maintenance_batch(&cleanup_batch)
        .await
        .expect("stage cleanup batch");
    store
        .clear_space_memory_state(
            Uuid::parse_str(&cleanup_batch.scope_id).expect("scope id"),
            true,
            false,
        )
        .await
        .expect("clear memory state");
    assert!(
        store
            .maintenance_batch_patch(&cleanup_batch.batch_key)
            .await
            .expect("read cleared batch")
            .is_none()
    );
}

#[tokio::test]
async fn maintenance_turns_are_independently_acknowledged() {
    let store = LocalStore::in_memory().await.expect("store");
    let turn = MaintenanceTurn {
        request_id: "request-maintenance-1".to_owned(),
        scope_id: "scope-1".to_owned(),
        user_content: "The moon gate requires a key.".to_owned(),
        assistant_content: "I will remember that rule.".to_owned(),
    };
    store
        .append_maintenance_turn(&turn, true, true)
        .await
        .expect("append turn");
    store
        .append_maintenance_turn(&turn, true, true)
        .await
        .expect("identical replay is idempotent");
    let mut conflicting = turn.clone();
    conflicting.user_content = "different immutable input".to_owned();
    assert!(matches!(
        store
            .append_maintenance_turn(&conflicting, true, true)
            .await,
        Err(StorageError::MaintenanceTurnConflict(_))
    ));
    assert_eq!(
        store
            .pending_maintenance_turns("scope-1", MaintenanceKind::Memory, 10)
            .await
            .expect("memory pending"),
        std::slice::from_ref(&turn)
    );
    store
        .mark_maintenance_turns_done(
            std::slice::from_ref(&turn.request_id),
            MaintenanceKind::Memory,
        )
        .await
        .expect("ack memory");
    assert!(
        store
            .pending_maintenance_turns("scope-1", MaintenanceKind::Memory, 10)
            .await
            .expect("memory done")
            .is_empty()
    );
    assert_eq!(
        store
            .pending_maintenance_turns("scope-1", MaintenanceKind::SemanticGraph, 10)
            .await
            .expect("nsg remains")
            .len(),
        1
    );
}
use momo_domain::new_id;

#[tokio::test]
async fn memory_patch_reviews_are_scope_isolated_and_auditable() {
    let store = LocalStore::in_memory().await.expect("store");
    let scope_id = new_id();
    let other_scope_id = new_id();
    let targets = vec!["events/arrival.md".to_owned()];
    let review = store
        .create_memory_patch_review(
            scope_id,
            "conversation-1",
            "patches: []",
            &targets,
            2,
            "require_confirmation",
        )
        .await
        .expect("create review");

    assert_eq!(
        store
            .list_memory_patch_reviews(scope_id, false)
            .await
            .expect("pending"),
        vec![review.clone()]
    );
    assert!(
        store
            .list_memory_patch_reviews(other_scope_id, true)
            .await
            .expect("other scope")
            .is_empty()
    );
    assert!(
        store
            .resolve_memory_patch_review(
                other_scope_id,
                review.id,
                MemoryPatchReviewStatus::Rejected,
                None,
                None,
            )
            .await
            .expect("scope-isolated decision")
            .is_none()
    );

    let approved = store
        .resolve_memory_patch_review(
            scope_id,
            review.id,
            MemoryPatchReviewStatus::Approved,
            Some("ok"),
            None,
        )
        .await
        .expect("approve")
        .expect("updated review");
    assert_eq!(approved.status, MemoryPatchReviewStatus::Approved);
    assert_eq!(approved.result.as_deref(), Some("ok"));
    assert!(
        store
            .list_memory_patch_reviews(scope_id, false)
            .await
            .expect("no pending")
            .is_empty()
    );
    assert_eq!(
        store
            .list_memory_patch_reviews(scope_id, true)
            .await
            .expect("history"),
        vec![approved]
    );
    assert!(
        store
            .resolve_memory_patch_review(
                scope_id,
                review.id,
                MemoryPatchReviewStatus::Rejected,
                None,
                None,
            )
            .await
            .expect("decision is idempotent")
            .is_none()
    );
}

#[tokio::test]
async fn persists_conversation_messages() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let conversation = Conversation {
        id: new_id(),
        scope_id: new_id(),
        character_id: None,
        title: "测试会话".to_owned(),
        created_at: now,
        updated_at: now,
    };
    store
        .save_conversation(&conversation)
        .await
        .expect("save conversation");
    let message = Message {
        id: new_id(),
        conversation_id: conversation.id,
        role: MessageRole::User,
        content: "你好".to_owned(),
        created_at: now,
    };
    store.append_message(&message).await.expect("save message");
    assert_eq!(
        store
            .list_messages(conversation.id)
            .await
            .expect("messages"),
        vec![message.clone()]
    );

    let second_message = Message {
        id: new_id(),
        content: "本地优先".to_owned(),
        ..message
    };
    store
        .stage_message(&second_message)
        .await
        .expect("save message");
    assert_eq!(
        store
            .list_messages(conversation.id)
            .await
            .expect("messages")
            .len(),
        2
    );
}

#[tokio::test]
async fn message_edits_deletes_and_restores_are_local_only() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let conversation = Conversation {
        id: new_id(),
        scope_id: new_id(),
        character_id: None,
        title: "Editable messages".to_owned(),
        created_at: now,
        updated_at: now,
    };
    store
        .save_conversation(&conversation)
        .await
        .expect("conversation");
    let message = Message {
        id: new_id(),
        conversation_id: conversation.id,
        role: MessageRole::User,
        content: "original".to_owned(),
        created_at: now,
    };
    store.stage_message(&message).await.expect("stage create");

    let edited = Message {
        content: "edited".to_owned(),
        ..message.clone()
    };
    store
        .stage_message_update(&edited)
        .await
        .expect("edit local message");
    assert_eq!(
        store
            .list_messages(conversation.id)
            .await
            .expect("messages")[0]
            .content,
        "edited"
    );

    store
        .stage_message_delete(message.id)
        .await
        .expect("stage delete");
    assert!(
        store
            .list_messages(conversation.id)
            .await
            .expect("messages")
            .is_empty()
    );
    assert!(
        store
            .restore_recently_deleted("message", message.id)
            .await
            .expect("restore")
    );
}

#[tokio::test]
async fn messages_are_immutable_idempotent_and_scope_isolated() {
    let store = LocalStore::in_memory().await.expect("store");
    let scope_id = new_id();
    let other_scope_id = new_id();
    let now = Utc::now();
    let conversation = Conversation {
        id: new_id(),
        scope_id,
        character_id: None,
        title: "Immutable messages".to_owned(),
        created_at: now,
        updated_at: now,
    };
    store
        .save_conversation(&conversation)
        .await
        .expect("conversation");
    let message = Message {
        id: new_id(),
        conversation_id: conversation.id,
        role: MessageRole::User,
        content: "original".to_owned(),
        created_at: now,
    };

    store.append_message(&message).await.expect("first append");
    store
        .append_message(&message)
        .await
        .expect("identical append");
    assert_eq!(
        store
            .list_messages_for_scope(scope_id, conversation.id)
            .await
            .expect("scope messages"),
        vec![message.clone()]
    );
    assert!(
        store
            .list_messages_for_scope(other_scope_id, conversation.id)
            .await
            .expect("other scope messages")
            .is_empty()
    );

    let changed = Message {
        content: "rewritten".to_owned(),
        ..message.clone()
    };
    assert!(matches!(
        store.append_message(&changed).await,
        Err(StorageError::ImmutableMessageConflict(id)) if id == message.id
    ));
    assert!(matches!(
        store.stage_message(&changed).await,
        Err(StorageError::ImmutableMessageConflict(id)) if id == message.id
    ));

    store.stage_message(&message).await.expect("identical save");
    assert_eq!(
        store
            .list_messages_for_scope(scope_id, conversation.id)
            .await
            .expect("unchanged messages"),
        vec![message]
    );
}

#[tokio::test]
async fn conversation_updates_cannot_replace_the_bound_character() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let original_character_id = new_id();
    let scope_id = new_id();
    let card = CharacterCard {
        id: original_character_id,
        scope_id,
        name: "Original character".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "Owner".to_owned(),
        author_url: None,
        character_markdown: String::new(),
        user_markdown: String::new(),
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    store.save_character(&card).await.expect("save character");
    let conversation = Conversation {
        id: new_id(),
        scope_id,
        character_id: Some(original_character_id),
        title: "Original".to_owned(),
        created_at: now,
        updated_at: now,
    };
    store
        .save_conversation(&conversation)
        .await
        .expect("save conversation");

    let changed = Conversation {
        character_id: Some(new_id()),
        title: "Renamed".to_owned(),
        updated_at: Utc::now(),
        ..conversation
    };
    store
        .stage_conversation_update(&changed)
        .await
        .expect("update conversation");

    let stored = store
        .list_conversations()
        .await
        .expect("conversations")
        .remove(0);
    assert_eq!(stored.character_id, Some(original_character_id));
    assert_eq!(stored.title, "Renamed");
}

#[tokio::test]
async fn stages_character_and_conversation_as_local_operations() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let scope_id = new_id();
    let card = CharacterCard {
        id: new_id(),
        scope_id,
        name: "Offline".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "Owner".to_owned(),
        author_url: None,
        character_markdown: "# Offline".to_owned(),
        user_markdown: String::new(),
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    store.stage_character(&card).await.expect("stage card");
    let conversation = Conversation {
        id: new_id(),
        scope_id,
        character_id: Some(card.id),
        title: "Offline conversation".to_owned(),
        created_at: now,
        updated_at: now,
    };
    store
        .stage_conversation(&conversation)
        .await
        .expect("stage conversation");

    let mut updated_card = card.clone();
    updated_card.name = "Offline updated".to_owned();
    updated_card.updated_at = Utc::now();
    store
        .stage_character_update(&updated_card)
        .await
        .expect("update staged card");
    assert_eq!(
        store
            .list_characters()
            .await
            .expect("characters")
            .remove(0)
            .name,
        "Offline updated"
    );
    store
        .stage_conversation_delete(conversation.id)
        .await
        .expect("delete conversation");
    assert!(
        store
            .list_conversations()
            .await
            .expect("conversations")
            .is_empty()
    );
    store
        .save_conversation(&conversation)
        .await
        .expect("ignore tombstoned local copy");
    assert!(
        store
            .list_conversations()
            .await
            .expect("still deleted")
            .is_empty()
    );
    let deleted = store.recently_deleted(10).await.expect("recently deleted");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].object_type, "conversation");
    assert_eq!(deleted[0].object_id, conversation.id.to_string());
    assert_eq!(
        deleted[0].display_name.as_deref(),
        Some("Offline conversation")
    );
    assert!(deleted[0].can_restore);
    store
        .save_conversation(&conversation)
        .await
        .expect("still ignore tombstoned local copy");
    assert!(
        store
            .list_conversations()
            .await
            .expect("not restored")
            .is_empty()
    );
    assert!(
        store
            .restore_recently_deleted("conversation", conversation.id)
            .await
            .expect("restore conversation")
    );
    assert_eq!(
        store
            .list_conversations()
            .await
            .expect("restored conversations"),
        vec![conversation.clone()]
    );
    assert!(
        store
            .recently_deleted(10)
            .await
            .expect("recently deleted after restore")
            .is_empty()
    );
}

#[tokio::test]
async fn purging_recent_delete_hides_snapshot_but_keeps_tombstone() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let scope_id = new_id();
    let card = CharacterCard {
        id: new_id(),
        scope_id,
        name: "Disposable".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "Owner".to_owned(),
        author_url: None,
        character_markdown: "# Disposable".to_owned(),
        user_markdown: String::new(),
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    store.stage_character(&card).await.expect("stage card");
    store
        .stage_character_delete(card.id)
        .await
        .expect("delete card");

    assert!(
        store
            .purge_recently_deleted("character", card.id)
            .await
            .expect("purge")
    );
    assert!(
        store
            .recently_deleted(10)
            .await
            .expect("recently deleted")
            .is_empty()
    );
    store
        .save_character(&card)
        .await
        .expect("ignore tombstoned local card");
    assert!(
        store
            .list_characters()
            .await
            .expect("characters")
            .is_empty()
    );
    assert!(
        !store
            .restore_recently_deleted("character", card.id)
            .await
            .expect("restore purged")
    );
}

#[tokio::test]
async fn forgetting_recent_delete_removes_guest_tombstone() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let scope_id = new_id();
    let card = CharacterCard {
        id: new_id(),
        scope_id,
        name: "Guest disposable".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "Guest".to_owned(),
        author_url: None,
        character_markdown: "# Guest disposable".to_owned(),
        user_markdown: String::new(),
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    store.stage_character(&card).await.expect("stage card");
    store
        .stage_character_delete(card.id)
        .await
        .expect("delete card");

    assert!(
        store
            .forget_recently_deleted("character", card.id)
            .await
            .expect("forget")
    );
    assert!(
        store
            .recently_deleted(10)
            .await
            .expect("recently deleted")
            .is_empty()
    );
    store
        .save_character(&card)
        .await
        .expect("forgotten card can be saved again");
    assert_eq!(
        store.list_characters().await.expect("characters"),
        vec![card]
    );
}

#[tokio::test]
async fn staged_objects_can_move_from_guest_to_account_scope() {
    let store = LocalStore::in_memory().await.expect("store");
    let now = Utc::now();
    let guest_scope = new_id();
    let account_scope = new_id();
    let mut card = CharacterCard {
        id: new_id(),
        scope_id: guest_scope,
        name: "Guest card".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "Guest".to_owned(),
        author_url: None,
        character_markdown: "# Guest".to_owned(),
        user_markdown: String::new(),
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    store.stage_character(&card).await.expect("guest card");
    card.scope_id = account_scope;
    card.author_name = "Account".to_owned();
    store.stage_character(&card).await.expect("move card");

    let guest_cards = store
        .list_characters_for_scope(guest_scope)
        .await
        .expect("guest cards");
    let account_cards = store
        .list_characters_for_scope(account_scope)
        .await
        .expect("account cards");
    assert!(guest_cards.is_empty());
    assert_eq!(account_cards, vec![card]);
}

#[tokio::test]
async fn nsg_vectors_are_scope_and_space_isolated_and_validate_input() {
    let store = TursoVectorStore::in_memory().await.expect("store");
    let scope = new_id();
    let record = NsgVectorRecord {
        scope_id: scope,
        node_id: "lore_lake".to_owned(),
        source_hash: "a".repeat(64),
        vector_space_id: "provider|model|3".to_owned(),
        dimension: 3,
        vector: vec![0.1, 0.2, 0.3],
        created_at: Utc::now(),
    };
    store
        .upsert_nsg_vectors(std::slice::from_ref(&record))
        .await
        .expect("save vector");
    assert_eq!(
        store
            .list_nsg_vectors(scope, "provider|model|3")
            .await
            .expect("load")
            .len(),
        1
    );
    assert!(
        store
            .list_nsg_vectors(new_id(), "provider|model|3")
            .await
            .expect("other scope")
            .is_empty()
    );
    let invalid = NsgVectorRecord {
        vector: vec![f64::NAN],
        dimension: 1,
        ..record
    };
    assert!(matches!(
        store.upsert_nsg_vectors(&[invalid]).await,
        Err(StorageError::InvalidNsgVector(_))
    ));
}

#[tokio::test]
async fn nsg_vector_snapshot_replacement_is_atomic_and_removes_stale_nodes() {
    let store = TursoVectorStore::in_memory().await.expect("store");
    let scope = new_id();
    let space = "momo-embedding-v1:test";
    let record = |node_id: &str, dimension: usize| NsgVectorRecord {
        scope_id: scope,
        node_id: node_id.to_owned(),
        source_hash: "a".repeat(64),
        vector_space_id: space.to_owned(),
        dimension,
        vector: vec![1.0; dimension],
        created_at: Utc::now(),
    };
    store
        .replace_nsg_vectors(scope, space, &[record("old", 2)])
        .await
        .expect("initial snapshot");
    store
        .replace_nsg_vectors(scope, space, &[record("new", 2)])
        .await
        .expect("replacement snapshot");
    let stored = store
        .list_nsg_vectors(scope, space)
        .await
        .expect("stored vectors");
    assert_eq!(
        stored
            .iter()
            .map(|item| item.node_id.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );

    let invalid = [record("a", 2), record("b", 3)];
    assert!(matches!(
        store.replace_nsg_vectors(scope, space, &invalid).await,
        Err(StorageError::InvalidNsgVector(_))
    ));
    assert_eq!(
        store
            .list_nsg_vectors(scope, space)
            .await
            .expect("snapshot after failed validation")[0]
            .node_id,
        "new"
    );
}

#[tokio::test]
async fn exact_vector_ranking_filters_stale_records_and_is_deterministic() {
    let store = TursoVectorStore::in_memory().await.expect("store");
    let scope = new_id();
    let now = Utc::now();
    let records = [
        NsgVectorRecord {
            scope_id: scope,
            node_id: "node_a".to_owned(),
            source_hash: "a".repeat(64),
            vector_space_id: "provider|embedding-small|3".to_owned(),
            dimension: 3,
            vector: vec![1.0, 0.0, 0.0],
            created_at: now,
        },
        NsgVectorRecord {
            scope_id: scope,
            node_id: "node_b".to_owned(),
            source_hash: "b".repeat(64),
            vector_space_id: "provider|embedding-small|3".to_owned(),
            dimension: 3,
            vector: vec![0.8, 0.2, 0.0],
            created_at: now,
        },
        NsgVectorRecord {
            scope_id: scope,
            node_id: "node_stale".to_owned(),
            source_hash: "c".repeat(64),
            vector_space_id: "provider|embedding-small|3".to_owned(),
            dimension: 3,
            vector: vec![0.99, 0.01, 0.0],
            created_at: now,
        },
    ];
    store
        .upsert_nsg_vectors(&records)
        .await
        .expect("save vectors");
    let current_hashes = HashMap::from([
        ("node_a".to_owned(), "a".repeat(64)),
        ("node_b".to_owned(), "b".repeat(64)),
        ("node_stale".to_owned(), "d".repeat(64)),
        ("node_missing".to_owned(), "e".repeat(64)),
    ]);

    let ranked = store
        .rank_nsg_vectors(
            scope,
            "provider|embedding-small|3",
            &[1.0, 0.0, 0.0],
            &current_hashes,
            2,
        )
        .await
        .expect("rank vectors");
    assert_eq!(ranked, vec!["node_a", "node_b"]);

    let status = store
        .nsg_vector_status(scope, "provider|embedding-small|3", &current_hashes)
        .await
        .expect("vector status");
    assert_eq!(status.indexed_count, 2);
    assert_eq!(status.stale_count, 1);
    assert_eq!(status.missing_count, 2);

    assert!(matches!(
        store
            .rank_nsg_vectors(
                scope,
                "provider|embedding-small|3",
                &[0.0, 0.0, 0.0],
                &current_hashes,
                2,
            )
            .await,
        Err(StorageError::InvalidNsgVector(_))
    ));
}
