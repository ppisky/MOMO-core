use std::{fs, path::Component};

use serde_json::json;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::*;

#[cfg(test)]
#[path = "../../../tests/unit/api_control.rs"]
mod tests;

pub async fn execute_control(
    runtime: &MomoRuntime,
    request: crate::MomoControlRequest,
) -> Result<serde_json::Value, RuntimeApiError> {
    request.validate().map_err(RuntimeApiError::invalid)?;
    let resource_key = control_resource_key(&request.action);
    let _guard = runtime.lock_control_resource(&resource_key).await;
    let operation_key = control_operation_key(&request);
    let request_fingerprint = control_request_fingerprint(&request)?;
    let store = runtime.core().store();
    if let Some(existing) = store
        .control_operation(&operation_key)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
    {
        if existing.request_fingerprint != request_fingerprint {
            return Err(RuntimeApiError::conflict(
                "control request_id was reused with different content",
            ));
        }
        if let Some(response) = existing.response_json {
            return serde_json::from_str(&response)
                .map_err(|error| RuntimeApiError::internal(error.to_string()));
        }
        return Err(RuntimeApiError::conflict(
            "control request is already in progress",
        ));
    }
    if !store
        .begin_control_operation(&operation_key, &request_fingerprint)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
    {
        return Err(RuntimeApiError::conflict(
            "control request is already in progress",
        ));
    }
    let result = match execute_control_action(runtime, &request.action).await {
        Ok(result) => result,
        Err(error) => {
            store
                .abandon_control_operation(&operation_key)
                .await
                .map_err(|storage_error| RuntimeApiError::internal(storage_error.to_string()))?;
            return Err(error);
        }
    };
    let response = json!({
        "schema": crate::MOMO_CONTROL_SCHEMA,
        "request_id": request.request_id,
        "actor_space_id": request.actor_space_id,
        "result": result,
    });
    let response_json = serde_json::to_string(&response)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    if let Err(error) = store
        .complete_control_operation(&operation_key, &response_json)
        .await
    {
        // The action already converged on its target state, but no replay
        // response was committed. Release the claim when possible so an
        // identical retry can safely rebuild the response.
        let _ = store.abandon_control_operation(&operation_key).await;
        return Err(RuntimeApiError::internal(error.to_string()));
    }
    Ok(response)
}

async fn execute_control_action(
    runtime: &MomoRuntime,
    action: &crate::MomoControlAction,
) -> Result<serde_json::Value, RuntimeApiError> {
    let result = match action {
        crate::MomoControlAction::DeleteConversation {
            conversation_space_id,
            conversation_id,
        } => {
            let store = runtime.core().store();
            if store
                .conversation_for_scope(*conversation_space_id, *conversation_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .is_some()
            {
                store
                    .stage_conversation_delete(*conversation_id)
                    .await
                    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
            } else if !store
                .is_tombstoned("conversation", *conversation_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            {
                return Err(RuntimeApiError::not_found(
                    "conversation does not belong to conversation_space_id",
                ));
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
            let state_guard = runtime
                .lock_space(*target_space_id)
                .await
                .map_err(RuntimeApiError::recovery)?;
            let core = runtime.core_handle();
            let target_space_id = *target_space_id;
            let memory = *memory;
            let semantic_graph = *semantic_graph;
            let removed_files = runtime
                .finish_commit(async move {
                    let _state_guard = state_guard;
                    let file_core = Arc::clone(&core);
                    let removed_files = run_blocking("clear memory", move || {
                        clear_memory_components(
                            file_core.as_ref(),
                            target_space_id,
                            memory,
                            semantic_graph,
                        )
                    })
                    .await?;
                    core.store()
                        .clear_space_memory_state(target_space_id, memory, semantic_graph)
                        .await
                        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
                    if semantic_graph {
                        core.vector_store()
                            .remove_nsg_vectors(target_space_id, None)
                            .await
                            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
                    }
                    Ok::<_, RuntimeApiError>(removed_files)
                })
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))??;
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
            let store = runtime.core().store();
            if store
                .character_by_id(*character_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .is_none()
            {
                return Err(RuntimeApiError::not_found("character_id does not exist"));
            }
            let mut conversation = store
                .conversation_for_scope(*conversation_space_id, *conversation_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .ok_or_else(|| {
                    RuntimeApiError::not_found(
                        "conversation does not belong to conversation_space_id",
                    )
                })?;
            conversation.character_id = Some(*character_id);
            conversation.updated_at = chrono::Utc::now();
            store
                .save_conversation(&conversation)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
            json!({
                "type": "switch_character",
                "conversation_space_id": conversation_space_id,
                "conversation_id": conversation_id,
                "character_id": character_id,
            })
        }
    };
    Ok(result)
}

fn control_operation_key(request: &crate::MomoControlRequest) -> String {
    let mut digest = Sha256::new();
    digest.update(request.actor_space_id.as_bytes());
    digest.update([0]);
    digest.update(request.request_id.as_bytes());
    format!("control_{}", hex::encode(digest.finalize()))
}

fn control_resource_key(action: &crate::MomoControlAction) -> String {
    match action {
        crate::MomoControlAction::DeleteConversation {
            conversation_id, ..
        }
        | crate::MomoControlAction::SwitchCharacter {
            conversation_id, ..
        } => format!("conversation:{conversation_id}"),
        crate::MomoControlAction::ClearMemory {
            target_space_id, ..
        } => format!("memory:{target_space_id}"),
    }
}

fn control_request_fingerprint(
    request: &crate::MomoControlRequest,
) -> Result<String, RuntimeApiError> {
    let encoded = serde_json::to_vec(request)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn clear_memory_components(
    core: &MomoCore,
    space_id: uuid::Uuid,
    clear_memory: bool,
    clear_semantic_graph: bool,
) -> Result<usize, RuntimeApiError> {
    let root = core
        .data_dir()
        .join("spaces")
        .join(space_id.to_string())
        .join("memory");
    if !root.exists() {
        return Ok(0);
    }
    let canonical_spaces = fs::canonicalize(core.data_dir().join("spaces"))
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let canonical_root =
        fs::canonicalize(&root).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    if !canonical_root.starts_with(&canonical_spaces) {
        return Err(RuntimeApiError::invalid(
            "memory Space resolves outside the instance root",
        ));
    }
    let mut files = Vec::new();
    let mut directories = Vec::new();
    for entry in WalkDir::new(&canonical_root).follow_links(false) {
        let entry = entry.map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        if entry.file_type().is_symlink() {
            return Err(RuntimeApiError::invalid(
                "memory Space contains a symbolic link",
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(&canonical_root)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            directories.push(entry.path().to_path_buf());
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(RuntimeApiError::invalid(
                "memory Space contains an unsupported entry",
            ));
        }
        let nsg = is_semantic_graph_path(relative);
        if (nsg && clear_semantic_graph) || (!nsg && clear_memory) {
            files.push(entry.path().to_path_buf());
        }
    }
    for path in &files {
        fs::remove_file(path).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        let _ = fs::remove_dir(path);
    }
    core.memory_for_space(space_id)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
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
