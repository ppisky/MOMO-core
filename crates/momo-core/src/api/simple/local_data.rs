//! Local cache access and outbox staging operations.

use super::*;

pub async fn initialize_core(data_dir: String) -> Result<String, String> {
    let core = CORE
        .get_or_try_init(|| MomoCore::initialize(PathBuf::from(data_dir)))
        .await
        .map_err(|error| error.to_string())?;
    Ok(core.data_dir().to_string_lossy().into_owned())
}

pub async fn cache_character_json(character_json: String) -> Result<(), String> {
    let character = serde_json::from_str(&character_json).map_err(|error| error.to_string())?;
    core()?
        .store()
        .save_character(&character)
        .await
        .map_err(|error| error.to_string())
}

pub async fn stage_character_from_json(character_json: String) -> Result<(), String> {
    let character = serde_json::from_str(&character_json).map_err(|error| error.to_string())?;
    core()?
        .store()
        .save_character(&character)
        .await
        .map_err(|error| error.to_string())
}

pub async fn cache_conversation_json(conversation_json: String) -> Result<(), String> {
    let conversation =
        serde_json::from_str(&conversation_json).map_err(|error| error.to_string())?;
    core()?
        .store()
        .save_conversation(&conversation)
        .await
        .map_err(|error| error.to_string())
}

pub async fn cache_messages_json(messages_json: String) -> Result<(), String> {
    let messages: Vec<momo_domain::Message> =
        serde_json::from_str(&messages_json).map_err(|error| error.to_string())?;
    for message in messages {
        core()?
            .store()
            .save_message(&message)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub async fn local_messages_json(conversation_id: String) -> Result<String, String> {
    let conversation_id =
        uuid::Uuid::parse_str(&conversation_id).map_err(|error| error.to_string())?;
    let messages = core()?
        .store()
        .list_messages(conversation_id)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&messages).map_err(|error| error.to_string())
}

pub async fn local_characters_json(scope_id: String) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let characters = core()?
        .store()
        .list_characters_for_scope(scope_id)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&characters).map_err(|error| error.to_string())
}

pub async fn local_conversations_json(scope_id: String) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let conversations = core()?
        .store()
        .list_conversations_for_scope(scope_id)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&conversations).map_err(|error| error.to_string())
}

pub async fn migrate_guest_data_json(
    _guest_scope_id: String,
    _account_scope_id: String,
    _account_author_name: String,
) -> Result<String, String> {
    Err("账号迁移已禁用：当前 MOMO Core 只支持本地单用户数据空间".to_owned())
}

pub async fn stage_message_json(
    conversation_id: String,
    role: String,
    content: String,
) -> Result<String, String> {
    let conversation_id =
        uuid::Uuid::parse_str(&conversation_id).map_err(|error| error.to_string())?;
    let role = momo_domain::MessageRole::try_from(role.as_str()).map_err(str::to_owned)?;
    let message = momo_domain::Message {
        id: momo_domain::new_id(),
        conversation_id,
        role,
        content,
        created_at: chrono::Utc::now(),
    };
    let core = core()?;
    core.store()
        .save_message(&message)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&message).map_err(|error| error.to_string())
}

pub async fn stage_message_update_json(message_json: String) -> Result<String, String> {
    let message: momo_domain::Message =
        serde_json::from_str(&message_json).map_err(|error| error.to_string())?;
    core()?
        .store()
        .save_message(&message)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&message).map_err(|error| error.to_string())
}

pub async fn stage_message_delete(id: String) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&id).map_err(|error| error.to_string())?;
    let store = core()?.store();
    store
        .stage_message_delete(id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn stage_character_json(
    scope_id: String,
    author_display_name: String,
    name: String,
    _description: String,
    character_markdown: String,
    user_markdown: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let now = chrono::Utc::now();
    let card = momo_domain::CharacterCard {
        id: momo_domain::new_id(),
        scope_id,
        name,
        version: "2.0.0".to_owned(),
        author_name: author_display_name,
        author_url: None,
        character_markdown,
        user_markdown,
        opening_markdown: None,
        created_at: now,
        updated_at: now,
    };
    core()?
        .store()
        .save_character(&card)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&card).map_err(|error| error.to_string())
}

pub async fn stage_character_update_json(character_json: String) -> Result<String, String> {
    let mut card: momo_domain::CharacterCard =
        serde_json::from_str(&character_json).map_err(|error| error.to_string())?;
    card.updated_at = chrono::Utc::now();
    core()?
        .store()
        .save_character(&card)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&card).map_err(|error| error.to_string())
}

pub async fn stage_character_delete(id: String) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&id).map_err(|error| error.to_string())?;
    let store = core()?.store();
    store
        .stage_character_delete(id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn stage_conversation_json(
    id: Option<String>,
    scope_id: String,
    title: String,
    character_id: Option<String>,
) -> Result<String, String> {
    let id = id
        .map(|value| uuid::Uuid::parse_str(&value))
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_else(momo_domain::new_id);
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let character_id = character_id
        .map(|id| uuid::Uuid::parse_str(&id))
        .transpose()
        .map_err(|error| error.to_string())?;
    let now = chrono::Utc::now();
    let conversation = momo_domain::Conversation {
        id,
        scope_id,
        character_id,
        title,
        created_at: now,
        updated_at: now,
    };
    core()?
        .store()
        .save_conversation(&conversation)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&conversation).map_err(|error| error.to_string())
}

pub async fn stage_conversation_update_json(conversation_json: String) -> Result<String, String> {
    let mut conversation: momo_domain::Conversation =
        serde_json::from_str(&conversation_json).map_err(|error| error.to_string())?;
    conversation.updated_at = chrono::Utc::now();
    core()?
        .store()
        .save_conversation(&conversation)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&conversation).map_err(|error| error.to_string())
}

pub async fn stage_conversation_delete(id: String) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&id).map_err(|error| error.to_string())?;
    let store = core()?.store();
    store
        .stage_conversation_delete(id)
        .await
        .map_err(|error| error.to_string())
}
