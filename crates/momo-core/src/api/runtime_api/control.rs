use serde_json::json;
use sha2::{Digest, Sha256};

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
                .reserve_space(*target_space_id)
                .await
                .map_err(RuntimeApiError::recovery)?;
            let core = runtime.core_handle();
            let target_space_id = *target_space_id;
            let memory = *memory;
            let semantic_graph = *semantic_graph;
            let removed_files = runtime
                .finish_commit(async move {
                    let _state_guard = state_guard;
                    let removed_files = core
                        .clear_space_memory(target_space_id, memory, semantic_graph)
                        .await
                        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
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
