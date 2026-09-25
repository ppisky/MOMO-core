//! Character, conversation, message and maintenance-evidence operations.

use super::*;

pub async fn append_maintenance_turn(
    runtime: &MomoRuntime,
    turn: momo_storage::MaintenanceTurn,
    memory_enabled: bool,
    nsg_enabled: bool,
) -> Result<(), RuntimeApiError> {
    runtime
        .core()
        .store()
        .append_maintenance_turn(&turn, memory_enabled, nsg_enabled)
        .await
        .map_err(|error| match error {
            momo_storage::StorageError::MaintenanceTurnConflict(_) => {
                RuntimeApiError::conflict(error.to_string())
            }
            _ => RuntimeApiError::internal(error.to_string()),
        })
}

pub async fn pending_maintenance_turns(
    runtime: &MomoRuntime,
    scope_id: String,
    kind: String,
    limit: usize,
) -> Result<Vec<momo_storage::MaintenanceTurn>, RuntimeApiError> {
    let kind = match kind.as_str() {
        "memory" => momo_storage::MaintenanceKind::Memory,
        "semantic_graph" => momo_storage::MaintenanceKind::SemanticGraph,
        _ => return Err(RuntimeApiError::invalid("unknown maintenance kind")),
    };
    let turns = runtime
        .core()
        .store()
        .pending_maintenance_turns(&scope_id, kind, limit)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(turns)
}

pub async fn local_messages(
    runtime: &MomoRuntime,
    scope_id: String,
    conversation_id: String,
) -> Result<Vec<momo_domain::Message>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let conversation_id = uuid::Uuid::parse_str(&conversation_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core();
    if core
        .store()
        .conversation_for_scope(scope_id, conversation_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "conversation does not belong to scope",
        ));
    }
    let messages = core
        .store()
        .list_messages_for_scope(scope_id, conversation_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(messages)
}

pub async fn local_characters(
    runtime: &MomoRuntime,
    scope_id: String,
) -> Result<Vec<momo_domain::CharacterCard>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let characters = runtime
        .core()
        .store()
        .list_characters_for_scope(scope_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(characters)
}

pub async fn local_character(
    runtime: &MomoRuntime,
    character_id: String,
) -> Result<momo_domain::CharacterCard, RuntimeApiError> {
    let character_id = uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let character = runtime
        .core()
        .store()
        .character_by_id(character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .ok_or_else(|| RuntimeApiError::not_found("character does not exist"))?;
    Ok(character)
}

const DDM_PROFILE_METADATA_KIND: &str = "character_ddm_profile";
const MAX_DDM_PROFILE_BYTES: usize = 64 * 1024;

pub async fn character_ddm_profile(
    runtime: &MomoRuntime,
    character_id: String,
) -> Result<Option<momo_memory::DdmProfile>, RuntimeApiError> {
    let character_id = uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if runtime
        .core()
        .store()
        .character_by_id(character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found("character does not exist"));
    }
    let Some(yaml) = runtime
        .core()
        .store()
        .portable_metadata(DDM_PROFILE_METADATA_KIND, &character_id.to_string())
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
    else {
        return Ok(None);
    };
    let profile = momo_memory::DdmProfile::parse_yaml(&yaml)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(Some(profile))
}

pub async fn character_ddm_profile_yaml(
    runtime: &MomoRuntime,
    character_id: String,
) -> Result<Option<String>, RuntimeApiError> {
    let character_id = uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    runtime
        .core()
        .store()
        .portable_metadata(DDM_PROFILE_METADATA_KIND, &character_id.to_string())
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn upsert_character_ddm_profile(
    runtime: &MomoRuntime,
    scope_id: String,
    character_id: String,
    profile_yaml: String,
) -> Result<momo_memory::DdmProfile, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let character_id = uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if profile_yaml.is_empty()
        || profile_yaml.len() > MAX_DDM_PROFILE_BYTES
        || profile_yaml.as_bytes().contains(&0)
    {
        return Err(RuntimeApiError::invalid(
            "DDM profile must be non-empty UTF-8 text no larger than 64 KiB without NUL bytes",
        ));
    }
    if runtime
        .core()
        .store()
        .character_for_scope(scope_id, character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "character does not belong to scope",
        ));
    }
    let profile = momo_memory::DdmProfile::parse_yaml(&profile_yaml)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    if profile.character_id != character_id.to_string() {
        return Err(RuntimeApiError::invalid(
            "DDM profile character_id does not match the character",
        ));
    }
    runtime
        .core()
        .store()
        .save_portable_metadata(
            DDM_PROFILE_METADATA_KIND,
            &character_id.to_string(),
            &profile_yaml,
        )
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(profile)
}

pub async fn delete_character_ddm_profile(
    runtime: &MomoRuntime,
    scope_id: String,
    character_id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let character_id = uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if runtime
        .core()
        .store()
        .character_for_scope(scope_id, character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "character does not belong to scope",
        ));
    }
    runtime
        .core()
        .store()
        .delete_character_ddm_profile(&character_id.to_string())
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn local_conversations(
    runtime: &MomoRuntime,
    scope_id: String,
) -> Result<Vec<momo_domain::Conversation>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let conversations = runtime
        .core()
        .store()
        .list_conversations_for_scope(scope_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(conversations)
}

pub async fn stage_message(
    runtime: &MomoRuntime,
    scope_id: String,
    conversation_id: String,
    role: String,
    content: String,
) -> Result<momo_domain::Message, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let conversation_id = uuid::Uuid::parse_str(&conversation_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let role =
        momo_domain::MessageRole::try_from(role.as_str()).map_err(RuntimeApiError::invalid)?;
    let message = momo_domain::Message {
        id: momo_domain::new_id(),
        conversation_id,
        role,
        content,
        created_at: chrono::Utc::now(),
    };
    let core = runtime.core();
    if core
        .store()
        .conversation_for_scope(scope_id, conversation_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "conversation does not belong to scope",
        ));
    }
    core.store()
        .save_message(&message)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(message)
}

pub async fn stage_message_update(
    runtime: &MomoRuntime,
    scope_id: String,
    message: momo_domain::Message,
) -> Result<momo_domain::Message, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core();
    if core
        .store()
        .message_for_scope(scope_id, message.id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "message does not belong to scope",
        ));
    }
    core.store()
        .save_message(&message)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(message)
}

pub async fn stage_message_delete(
    runtime: &MomoRuntime,
    scope_id: String,
    id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let store = runtime.core().store();
    if store
        .message_for_scope(scope_id, id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "message does not belong to scope",
        ));
    }
    store
        .stage_message_delete(id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn stage_character(
    runtime: &MomoRuntime,
    scope_id: String,
    author_display_name: String,
    name: String,
    character_markdown: String,
    user_markdown: String,
) -> Result<momo_domain::CharacterCard, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
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
    validate_runtime_character(&card)?;
    runtime
        .core()
        .store()
        .save_character(&card)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(card)
}

pub async fn stage_character_update(
    runtime: &MomoRuntime,
    scope_id: String,
    mut card: momo_domain::CharacterCard,
) -> Result<momo_domain::CharacterCard, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if card.scope_id != scope_id {
        return Err(RuntimeApiError::invalid(
            "character body scope does not match request scope",
        ));
    }
    validate_runtime_character(&card)?;
    let core = runtime.core();
    if core
        .store()
        .character_for_scope(scope_id, card.id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "character does not belong to scope",
        ));
    }
    card.updated_at = chrono::Utc::now();
    core.store()
        .save_character(&card)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(card)
}

fn validate_runtime_character(card: &momo_domain::CharacterCard) -> Result<(), RuntimeApiError> {
    if card.name.trim().is_empty() || card.name.chars().count() > 120 {
        return Err(RuntimeApiError::invalid(
            "character name must contain between 1 and 120 characters",
        ));
    }
    semver::Version::parse(&card.version).map_err(|error| {
        RuntimeApiError::invalid(format!("character version is not SemVer: {error}"))
    })?;
    if card.author_name.trim().is_empty() || card.author_name.chars().count() > 200 {
        return Err(RuntimeApiError::invalid(
            "character author name must contain between 1 and 200 characters",
        ));
    }
    if let Some(url) = card.author_url.as_deref() {
        url::Url::parse(url).map_err(|error| {
            RuntimeApiError::invalid(format!("character author URL is invalid: {error}"))
        })?;
    }
    for (name, markdown) in [
        ("character_markdown", Some(card.character_markdown.as_str())),
        ("user_markdown", Some(card.user_markdown.as_str())),
        ("opening_markdown", card.opening_markdown.as_deref()),
    ] {
        let Some(markdown) = markdown else {
            continue;
        };
        if markdown.len() > 200_000 {
            return Err(RuntimeApiError::invalid(format!(
                "{name} exceeds 200000 bytes"
            )));
        }
        let mut lines = markdown.lines();
        if let Some(marker @ ("---" | "+++")) = lines.next().map(str::trim)
            && lines.any(|line| line.trim() == marker)
        {
            return Err(RuntimeApiError::invalid(format!(
                "{name} must not contain frontmatter"
            )));
        }
    }
    Ok(())
}

pub async fn stage_character_delete(
    runtime: &MomoRuntime,
    scope_id: String,
    id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let store = runtime.core().store();
    if store
        .character_for_scope(scope_id, id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "character does not belong to scope",
        ));
    }
    store
        .stage_character_delete(id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn stage_conversation(
    runtime: &MomoRuntime,
    id: Option<String>,
    scope_id: String,
    title: String,
    character_id: String,
) -> Result<momo_domain::Conversation, RuntimeApiError> {
    let id = id
        .map(|value| uuid::Uuid::parse_str(&value))
        .transpose()
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .unwrap_or_else(momo_domain::new_id);
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let character_id = uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core();
    if core
        .store()
        .character_by_id(character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found("character does not exist"));
    }
    let now = chrono::Utc::now();
    let conversation = momo_domain::Conversation {
        id,
        scope_id,
        character_id: Some(character_id),
        title,
        created_at: now,
        updated_at: now,
    };
    core.store()
        .save_conversation(&conversation)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(conversation)
}

pub async fn stage_conversation_update(
    runtime: &MomoRuntime,
    scope_id: String,
    conversation: momo_domain::Conversation,
) -> Result<momo_domain::Conversation, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if conversation.scope_id != scope_id {
        return Err(RuntimeApiError::invalid(
            "conversation body scope does not match request scope",
        ));
    }
    let core = runtime.core();
    let conversation = stage_conversation_update_for_core(core, scope_id, conversation).await?;
    Ok(conversation)
}

async fn stage_conversation_update_for_core(
    core: &MomoCore,
    scope_id: uuid::Uuid,
    mut conversation: momo_domain::Conversation,
) -> Result<momo_domain::Conversation, RuntimeApiError> {
    let existing = core
        .store()
        .conversation_for_scope(scope_id, conversation.id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .ok_or_else(|| RuntimeApiError::not_found("conversation does not belong to scope"))?;
    // The generic conversation update endpoint edits mutable presentation
    // fields only. Character changes must go through the explicit
    // switch_character control so its authorization and audit boundary cannot
    // be bypassed by submitting a replacement Conversation object.
    conversation.character_id = existing.character_id;
    conversation.created_at = existing.created_at;
    conversation.updated_at = chrono::Utc::now();
    core.store()
        .stage_conversation_update(&conversation)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(conversation)
}

pub async fn stage_conversation_delete(
    runtime: &MomoRuntime,
    scope_id: String,
    id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let store = runtime.core().store();
    if store
        .conversation_for_scope(scope_id, id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "conversation does not belong to scope",
        ));
    }
    store
        .stage_conversation_delete(id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

#[cfg(test)]
#[path = "../../../tests/unit/api_simple_local_data.rs"]
mod character_validation_tests;
