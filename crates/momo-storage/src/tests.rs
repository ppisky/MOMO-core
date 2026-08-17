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
