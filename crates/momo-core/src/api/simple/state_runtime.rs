//! MO State v2 runtime observation, projection journal, and status access.

use serde_json::json;

use super::*;

pub async fn observe_mo_state_runtime_json(
    operation_id: String,
    space_id: String,
    event_type: String,
    event_fingerprint: String,
    profile: String,
) -> Result<String, String> {
    let parsed_space_id = uuid::Uuid::parse_str(&space_id).map_err(|error| error.to_string())?;
    let source = core()?
        .memory_for_space(parsed_space_id)
        .map_err(|error| error.to_string())?
        .mo_state_source_fingerprint()
        .map_err(|error| error.to_string())?;
    let operation = core()?
        .store()
        .observe_mo_state_operation(&momo_storage::MoStateObservation {
            operation_id,
            space_id,
            event_type,
            event_fingerprint,
            profile,
            dmw_fingerprint: source.dmw,
            nsg_fingerprint: source.nsg,
            scene_fingerprint: source.scene,
            scene_json: serde_json::to_string(&source.scene_snapshot)
                .map_err(|error| error.to_string())?,
        })
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&operation).map_err(|error| error.to_string())
}

pub async fn publish_mo_state_snapshot_json(
    operation_id: String,
    state_result_json: String,
    degraded: bool,
    error: Option<String>,
) -> Result<String, String> {
    let snapshot = core()?
        .store()
        .publish_mo_state_snapshot(
            &operation_id,
            &state_result_json,
            degraded,
            error.as_deref(),
        )
        .await
        .map_err(|storage_error| storage_error.to_string())?;
    serde_json::to_string(&snapshot).map_err(|serde_error| serde_error.to_string())
}

pub async fn fail_mo_state_operation(operation_id: String, error: String) -> Result<(), String> {
    core()?
        .store()
        .fail_mo_state_operation(&operation_id, &error)
        .await
        .map_err(|storage_error| storage_error.to_string())
}

pub async fn mo_state_runtime_status_json(space_id: String) -> Result<String, String> {
    uuid::Uuid::parse_str(&space_id).map_err(|error| error.to_string())?;
    let status = core()?
        .store()
        .mo_state_runtime_status(&space_id)
        .await
        .map_err(|error| error.to_string())?;
    match status {
        Some(status) => serde_json::to_string(&status).map_err(|error| error.to_string()),
        None => serde_json::to_string(&json!({
            "space_id": space_id,
            "status": "not_initialized"
        }))
        .map_err(|error| error.to_string()),
    }
}
