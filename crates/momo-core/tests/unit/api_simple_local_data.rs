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
