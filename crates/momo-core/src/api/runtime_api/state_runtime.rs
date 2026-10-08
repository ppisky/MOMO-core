//! MO State v2 runtime observation, projection journal, and status access.

use serde_json::json;

use super::*;

pub async fn managed_ddm_profile(
    runtime: &MomoRuntime,
    space_id: uuid::Uuid,
    character_id: uuid::Uuid,
) -> RuntimeApiResult<Option<serde_json::Value>> {
    runtime
        .core()
        .store()
        .portable_metadata(
            crate::orchestration::ddm::MANAGED_DDM_KIND,
            &format!("{space_id}:{character_id}"),
        )
        .await
        .map_err(|e| RuntimeApiError::internal(e.to_string()))?
        .map(|value| {
            serde_json::from_str(&value).map_err(|e| RuntimeApiError::internal(e.to_string()))
        })
        .transpose()
}

pub async fn reset_managed_ddm_profile(
    runtime: &MomoRuntime,
    space_id: uuid::Uuid,
    character_id: uuid::Uuid,
) -> RuntimeApiResult<()> {
    let key = format!("{space_id}:{character_id}");
    let _guard = runtime.lock_control_resource(&format!("ddm:{key}")).await;
    runtime
        .core()
        .store()
        .delete_portable_metadata(crate::orchestration::ddm::MANAGED_DDM_KIND, &key)
        .await
        .map_err(|e| RuntimeApiError::internal(e.to_string()))
}

fn contract_error(error: momo_memory::MemoryError) -> RuntimeApiError {
    match error {
        momo_memory::MemoryError::RevisionConflict { .. } => {
            RuntimeApiError::conflict(error.to_string())
        }
        momo_memory::MemoryError::InvalidAccess(_) | momo_memory::MemoryError::Yaml(_) => {
            RuntimeApiError::invalid(error.to_string())
        }
        _ => RuntimeApiError::internal(error.to_string()),
    }
}

pub async fn mo_state_contract(
    runtime: &MomoRuntime,
    space_id: uuid::Uuid,
) -> RuntimeApiResult<momo_memory::StateContractResource> {
    let core = runtime.core_handle();
    run_blocking("read state contract", move || {
        core.memory_for_space(space_id)
            .map_err(contract_error)?
            .state_contract_resource()
            .map_err(contract_error)
    })
    .await
}

pub async fn replace_mo_state_contract(
    runtime: &MomoRuntime,
    space_id: uuid::Uuid,
    content: Option<String>,
    expected_revision: Option<String>,
) -> RuntimeApiResult<momo_memory::StateContractResource> {
    let core = runtime.core_handle();
    run_space_write(runtime, space_id, "replace state contract", move || {
        core.memory_for_space(space_id)
            .map_err(contract_error)?
            .replace_state_contract(content, expected_revision)
            .map_err(contract_error)
    })
    .await
}

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
    let mut value = match status {
        Some(status) => serde_json::to_value(&status)
            .map_err(|error| RuntimeApiError::internal(error.to_string())),
        None => serde_json::to_value(json!({
            "space_id": space_id,
            "status": "not_initialized"
        }))
        .map_err(|error| RuntimeApiError::internal(error.to_string())),
    }?;
    let mut lifecycle = runtime
        .core()
        .store()
        .lifecycle_status(&space_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    lifecycle["settings"] = serde_json::to_value(
        runtime
            .runtime_settings_for_space(
                uuid::Uuid::parse_str(&space_id)
                    .map_err(|e| RuntimeApiError::invalid(e.to_string()))?,
            )
            .mo_state
            .memory_lifecycle,
    )
    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    value["memory_lifecycle"] = lifecycle;
    Ok(value)
}
