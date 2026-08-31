use std::{fs, path::Component};

use serde_json::json;
use walkdir::WalkDir;

use super::*;

pub async fn execute_control_json(request_json: String) -> Result<String, String> {
    let request: crate::MomoControlRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    request.validate()?;
    let _guard = CONTROL_LOCK.lock().await;
    let result = match request.action {
        crate::MomoControlAction::DeleteConversation {
            conversation_space_id,
            conversation_id,
        } => {
            let store = core()?.store();
            if store
                .conversation_for_scope(conversation_space_id, conversation_id)
                .await
                .map_err(|error| error.to_string())?
                .is_some()
            {
                store
                    .stage_conversation_delete(conversation_id)
                    .await
                    .map_err(|error| error.to_string())?;
            } else if !store
                .is_tombstoned("conversation", conversation_id)
                .await
                .map_err(|error| error.to_string())?
            {
                return Err("conversation does not belong to conversation_space_id".to_owned());
            }
            json!({
                "type": "delete_conversation",
                "conversation_space_id": conversation_space_id,
                "conversation_id": conversation_id,
            })
        }
        crate::MomoControlAction::ClearMemory {
            target_space_id,
            memory,
            semantic_graph,
        } => {
            let removed_files = clear_memory_components(target_space_id, memory, semantic_graph)?;
            core()?
                .store()
                .clear_space_memory_state(target_space_id, memory, semantic_graph)
                .await
                .map_err(|error| error.to_string())?;
            if semantic_graph {
                core()?
                    .vector_store()
                    .remove_nsg_vectors(target_space_id, None)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            json!({
                "type": "clear_memory",
                "target_space_id": target_space_id,
                "memory": memory,
                "semantic_graph": semantic_graph,
                "removed_files": removed_files,
            })
        }
        crate::MomoControlAction::SwitchCharacter {
            conversation_space_id,
            conversation_id,
            character_id,
        } => {
            let store = core()?.store();
            if store
                .character_by_id(character_id)
                .await
                .map_err(|error| error.to_string())?
                .is_none()
            {
                return Err("character_id does not exist".to_owned());
            }
            let mut conversation = store
                .conversation_for_scope(conversation_space_id, conversation_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "conversation does not belong to conversation_space_id".to_owned()
                })?;
            conversation.character_id = Some(character_id);
            conversation.updated_at = chrono::Utc::now();
            store
                .save_conversation(&conversation)
                .await
                .map_err(|error| error.to_string())?;
            json!({
                "type": "switch_character",
                "conversation_space_id": conversation_space_id,
                "conversation_id": conversation_id,
                "character_id": character_id,
            })
        }
    };
    serde_json::to_string(&json!({
        "schema": crate::MOMO_CONTROL_SCHEMA,
        "request_id": request.request_id,
        "actor_space_id": request.actor_space_id,
        "result": result,
    }))
    .map_err(|error| error.to_string())
}

fn clear_memory_components(
    space_id: uuid::Uuid,
    clear_memory: bool,
    clear_semantic_graph: bool,
) -> Result<usize, String> {
    let core = core()?;
    let root = core
        .data_dir()
        .join("spaces")
        .join(space_id.to_string())
        .join("memory");
    if !root.exists() {
        return Ok(0);
    }
    let canonical_spaces =
        fs::canonicalize(core.data_dir().join("spaces")).map_err(|error| error.to_string())?;
    let canonical_root = fs::canonicalize(&root).map_err(|error| error.to_string())?;
    if !canonical_root.starts_with(&canonical_spaces) {
        return Err("memory Space resolves outside the instance root".to_owned());
    }
    let mut files = Vec::new();
    let mut directories = Vec::new();
    for entry in WalkDir::new(&canonical_root).follow_links(false) {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_type().is_symlink() {
            return Err("memory Space contains a symbolic link".to_owned());
        }
        let relative = entry
            .path()
            .strip_prefix(&canonical_root)
            .map_err(|error| error.to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            directories.push(entry.path().to_path_buf());
            continue;
        }
        if !entry.file_type().is_file() {
            return Err("memory Space contains an unsupported entry".to_owned());
        }
        let nsg = is_semantic_graph_path(relative);
        if (nsg && clear_semantic_graph) || (!nsg && clear_memory) {
            files.push(entry.path().to_path_buf());
        }
    }
    for path in &files {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        let _ = fs::remove_dir(path);
    }
    core.memory_for_space(space_id)
        .map_err(|error| error.to_string())?;
    Ok(files.len())
}

fn is_semantic_graph_path(path: &std::path::Path) -> bool {
    let parts = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    matches!(parts.as_slice(), ["lore", ..] | ["rules", ..])
        || matches!(
            parts.as_slice(),
            ["archive", "lore", ..] | ["archive", "rules", ..]
        )
}
