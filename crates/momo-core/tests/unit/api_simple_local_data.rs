use super::*;

fn card() -> momo_domain::CharacterCard {
    let now = chrono::Utc::now();
    momo_domain::CharacterCard {
        id: momo_domain::new_id(),
        scope_id: momo_domain::new_id(),
        name: "Momo".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "Author".to_owned(),
        author_url: Some("https://example.test/author".to_owned()),
        character_markdown: "# Momo\n\nStay in character.".to_owned(),
        user_markdown: String::new(),
        opening_markdown: Some("Hello.".to_owned()),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn character_crud_uses_native_card_validation() {
    validate_runtime_character(&card()).expect("valid character");

    let mut invalid = card();
    invalid.author_name.clear();
    assert!(validate_runtime_character(&invalid).is_err());

    let mut invalid = card();
    invalid.version = "latest".to_owned();
    assert!(validate_runtime_character(&invalid).is_err());

    let mut invalid = card();
    invalid.character_markdown = "---\nsecret: true\n---\n# Momo".to_owned();
    assert!(validate_runtime_character(&invalid).is_err());
}

#[tokio::test]
async fn conversation_update_cannot_replace_the_bound_character() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path())
        .await
        .expect("initialize core");
    let now = chrono::Utc::now();
    let scope_id = momo_domain::new_id();
    let conversation = momo_domain::Conversation {
        id: momo_domain::new_id(),
        scope_id,
        character_id: None,
        title: "Before".to_owned(),
        created_at: now,
        updated_at: now,
    };
    core.store()
        .save_conversation(&conversation)
        .await
        .expect("save conversation");
    let submitted = momo_domain::Conversation {
        character_id: Some(momo_domain::new_id()),
        title: "After".to_owned(),
        created_at: now + chrono::Duration::days(1),
        ..conversation.clone()
    };

    let updated = stage_conversation_update_for_core(&core, scope_id, submitted)
        .await
        .expect("update conversation");
    assert_eq!(updated.character_id, None);
    assert_eq!(updated.created_at, conversation.created_at);
    assert_eq!(updated.title, "After");
    let stored = core
        .store()
        .conversation_for_scope(scope_id, conversation.id)
        .await
        .expect("load conversation")
        .expect("stored conversation");
    assert_eq!(stored.character_id, None);
    assert_eq!(stored.title, "After");
}
