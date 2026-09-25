//! MO State v2 runtime observation, projection journal, and status access.

use serde_json::json;

use super::*;

pub async fn ddm_projection_state(
    runtime: &MomoRuntime,
    managed_space_id: String,
    conversation_id: String,
    character_id: String,
) -> Result<Option<momo_storage::DdmProjectionState>, RuntimeApiError> {
    uuid::Uuid::parse_str(&managed_space_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    uuid::Uuid::parse_str(&conversation_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    uuid::Uuid::parse_str(&character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    runtime
        .core()
        .store()
        .ddm_projection_state(&managed_space_id, &conversation_id, &character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn fail_mo_state_operation(
    runtime: &MomoRuntime,
    operation_id: String,
    error: String,
) -> Result<(), RuntimeApiError> {
    runtime
        .core()
        .store()
        .fail_mo_state_operation(&operation_id, &error)
        .await
        .map_err(|storage_error| RuntimeApiError::internal(storage_error.to_string()))
}

pub async fn mo_state_runtime_status(
    runtime: &MomoRuntime,
    space_id: String,
) -> Result<serde_json::Value, RuntimeApiError> {
    uuid::Uuid::parse_str(&space_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let status = runtime
        .core()
        .store()
        .mo_state_runtime_status(&space_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    match status {
        Some(status) => serde_json::to_value(&status)
            .map_err(|error| RuntimeApiError::internal(error.to_string())),
        None => serde_json::to_value(json!({
            "space_id": space_id,
            "status": "not_initialized"
        }))
        .map_err(|error| RuntimeApiError::internal(error.to_string())),
    }
}
