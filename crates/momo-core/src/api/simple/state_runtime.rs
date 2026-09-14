//! MO State v2 runtime observation, projection journal, and status access.

use serde_json::json;

use super::*;

pub async fn observe_mo_state_runtime_json(
    operation_id: String,
    space_id: String,
    source_observations_json: String,
    event_type: String,
    event_fingerprint: String,
    profile: String,
) -> Result<String, String> {
    uuid::Uuid::parse_str(&space_id).map_err(|error| error.to_string())?;
    let mut source_observations: Vec<momo_storage::MoStateSourceObservation> =
        serde_json::from_str(&source_observations_json).map_err(|error| error.to_string())?;
    let managed_index = source_observations
        .iter()
        .position(|source| source.space_id == space_id);
    let managed_source = managed_index.map(|index| source_observations.remove(index));
    let source =
        managed_source.ok_or_else(|| "managed Space source was not observed".to_owned())?;
    let operation = core()?
        .store()
        .observe_mo_state_operation(&momo_storage::MoStateObservation {
            operation_id,
            space_id,
            event_type,
            event_fingerprint,
            profile,
            dmw_fingerprint: source.dmw_fingerprint,
            nsg_fingerprint: source.nsg_fingerprint,
            scene_fingerprint: source.scene_fingerprint,
            scene_json: source.scene_json,
            source_observations,
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
    ddm_update_json: Option<String>,
) -> Result<String, String> {
    let ddm_update = ddm_update_json
        .as_deref()
        .map(serde_json::from_str::<momo_storage::DdmProjectionUpdate>)
        .transpose()
        .map_err(|error| error.to_string())?;
    let snapshot = core()?
        .store()
        .publish_mo_state_snapshot(
            &operation_id,
            &state_result_json,
            degraded,
            error.as_deref(),
            ddm_update.as_ref(),
        )
        .await
        .map_err(|storage_error| storage_error.to_string())?;
    serde_json::to_string(&snapshot).map_err(|serde_error| serde_error.to_string())
}

pub async fn ddm_projection_state_json(
    managed_space_id: String,
    conversation_id: String,
    character_id: String,
) -> Result<Option<String>, String> {
    uuid::Uuid::parse_str(&managed_space_id).map_err(|error| error.to_string())?;
    uuid::Uuid::parse_str(&conversation_id).map_err(|error| error.to_string())?;
    uuid::Uuid::parse_str(&character_id).map_err(|error| error.to_string())?;
    core()?
        .store()
        .ddm_projection_state(&managed_space_id, &conversation_id, &character_id)
        .await
        .map_err(|error| error.to_string())?
        .map(|state| serde_json::to_string(&state).map_err(|error| error.to_string()))
        .transpose()
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
