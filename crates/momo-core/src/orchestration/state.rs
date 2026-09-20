//! MO State observation, compilation, and publication.

use serde_json::{Value, json};

use super::{MomoApiError, MomoApiService, ResolvedResponseInput};
use crate::{MoStateInjectionMode, MoStateProfile, MomoConfig, MomoResponseRequest, api::simple};

/// Immutable evidence required to produce one governed state projection.
pub(super) struct StateRequest<'a> {
    pub(super) request: &'a MomoResponseRequest,
    pub(super) config: &'a MomoConfig,
    pub(super) operation_key: &'a str,
    pub(super) request_fingerprint: &'a str,
    pub(super) managed_space_id: &'a str,
    pub(super) conversation_id: &'a str,
    pub(super) character_id: &'a str,
    pub(super) source_observations: &'a Value,
    pub(super) memory: &'a [Value],
    pub(super) nsg: &'a [Value],
    pub(super) state_input_audit: Value,
    pub(super) context_window: usize,
    pub(super) resolved_input: &'a ResolvedResponseInput,
}

pub(super) struct CompiledResponseState {
    pub(super) result: Value,
    pub(super) operation_id: Option<String>,
    pub(super) context_for_prompt: String,
    pub(super) injection_status: &'static str,
    pub(super) guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    pub(super) warnings: Vec<String>,
}

impl MomoApiService {
    pub(super) async fn compile_response_state(
        &self,
        input: StateRequest<'_>,
    ) -> Result<CompiledResponseState, MomoApiError> {
        let StateRequest {
            request,
            config,
            operation_key,
            request_fingerprint,
            managed_space_id,
            conversation_id,
            character_id,
            source_observations,
            memory,
            nsg,
            state_input_audit,
            context_window,
            resolved_input,
        } = input;
        let mut warnings = Vec::new();
        let autonomous =
            request.momo.mo_state && config.mo_state.profile == MoStateProfile::ClosedAutonomous;
        let guard = if autonomous {
            Some(simple::lock_mo_state_space(managed_space_id).await)
        } else {
            None
        };
        let mut operation_id = None;
        let mut operation = None;
        let mut result = if request.momo.mo_state {
            let persisted_snapshot = if autonomous {
                match simple::observe_mo_state_runtime_json(
                    operation_key.to_owned(),
                    managed_space_id.to_owned(),
                    source_observations.to_string(),
                    if request.input.has_function_outputs() {
                        "tool_result".to_owned()
                    } else {
                        "user_message".to_owned()
                    },
                    request_fingerprint.to_owned(),
                    config.mo_state.profile.as_str().to_owned(),
                )
                .await
                {
                    Ok(operation_json) => {
                        let observed: momo_storage::MoStateOperation =
                            serde_json::from_str(&operation_json)
                                .map_err(|error| MomoApiError::internal(error.to_string()))?;
                        operation_id = Some(observed.operation_id.clone());
                        let snapshot_json = observed.snapshot_json.clone();
                        operation = Some(observed);
                        snapshot_json
                    }
                    Err(error) => {
                        warnings.push(format!("MO State runtime degraded: {error}"));
                        None
                    }
                }
            } else {
                None
            };
            if let Some(snapshot_json) = persisted_snapshot {
                let snapshot: momo_storage::MoStateSnapshot = serde_json::from_str(&snapshot_json)
                    .map_err(|error| MomoApiError::internal(error.to_string()))?;
                state_result_from_snapshot(&snapshot)
            } else {
                let ddm_profile_json = if config.mo_state.ddm.enabled {
                    simple::character_ddm_profile_json(character_id.to_owned())
                        .await
                        .map_err(MomoApiError::internal)?
                } else {
                    None
                };
                let previous_ddm_state = if autonomous && ddm_profile_json.is_some() {
                    simple::ddm_projection_state_json(
                        managed_space_id.to_owned(),
                        conversation_id.to_owned(),
                        character_id.to_owned(),
                    )
                    .await
                    .map_err(MomoApiError::internal)?
                    .map(|state| serde_json::from_str::<Value>(&state))
                    .transpose()
                    .map_err(|error| MomoApiError::internal(error.to_string()))?
                } else {
                    None
                };
                let profile_identity = ddm_profile_json
                    .as_deref()
                    .map(serde_json::from_str::<momo_memory::DdmProfile>)
                    .transpose()
                    .map_err(|error| MomoApiError::internal(error.to_string()))?
                    .map(|profile| (profile.revision, profile.profile_fingerprint()));
                let previous_bands =
                    compatible_ddm_bands(previous_ddm_state.as_ref(), profile_identity.as_ref());
                let observed_scene = operation
                    .as_ref()
                    .map(|operation| operation.observed_scene.clone())
                    .or_else(|| {
                        source_observations.as_array().and_then(|sources| {
                            sources.iter().find_map(|source| {
                                (source["space_id"].as_str() == Some(managed_space_id))
                                    .then(|| source["scene_json"].as_str())
                                    .flatten()
                                    .and_then(|scene| serde_json::from_str::<Value>(scene).ok())
                            })
                        })
                    });
                let ddm_runtime_json = ddm_profile_json.as_ref().map(|_| {
                    json!({
                        "previous_bands": previous_bands,
                        "scene": observed_scene,
                        "request_event_type": if request.input.has_function_outputs() {
                            "tool_result"
                        } else {
                            "user_message"
                        },
                        "request_has_image": resolved_input.visual_input_count > 0,
                        "request_evidence_id": operation_key,
                    })
                    .to_string()
                });
                let (compiled, degraded, compile_error) =
                    match simple::compile_mo_state_with_ddm_json(
                        managed_space_id.to_owned(),
                        serde_json::to_string(memory)
                            .map_err(|error| MomoApiError::internal(error.to_string()))?,
                        serde_json::to_string(nsg)
                            .map_err(|error| MomoApiError::internal(error.to_string()))?,
                        context_window,
                        ddm_profile_json,
                        ddm_runtime_json,
                    )
                    .await
                    {
                        Ok(value) => (
                            serde_json::from_str::<Value>(&value)
                                .map_err(|error| MomoApiError::internal(error.to_string()))?,
                            false,
                            None,
                        ),
                        Err(error) => {
                            warnings.push(format!("MO State degraded: {error}"));
                            (
                                json!({"context": "", "audit": {"degraded": true}}),
                                true,
                                Some(error),
                            )
                        }
                    };
                if let Some(state_operation_id) = operation_id.as_ref() {
                    let ddm_update_json = compiled
                        .get("audit")
                        .and_then(|audit| audit.get("ddm"))
                        .map(|ddm| {
                            json!({
                                "managed_space_id": managed_space_id,
                                "conversation_id": conversation_id,
                                "character_id": character_id,
                                "profile_revision": ddm["profile_revision"],
                                "profile_fingerprint": ddm["profile_fingerprint"],
                                "source_fingerprint": ddm["source_fingerprint"],
                                "bands": ddm["next_bands"],
                            })
                            .to_string()
                        });
                    match simple::publish_mo_state_snapshot_json(
                        state_operation_id.clone(),
                        serde_json::to_string(&compiled)
                            .map_err(|error| MomoApiError::internal(error.to_string()))?,
                        degraded,
                        compile_error.clone(),
                        ddm_update_json,
                    )
                    .await
                    {
                        Ok(snapshot_json) => {
                            let snapshot: momo_storage::MoStateSnapshot =
                                serde_json::from_str(&snapshot_json)
                                    .map_err(|error| MomoApiError::internal(error.to_string()))?;
                            state_result_from_snapshot(&snapshot)
                        }
                        Err(error) => {
                            warnings
                                .push(format!("MO State snapshot persistence degraded: {error}"));
                            let _ =
                                simple::fail_mo_state_operation(state_operation_id.clone(), error)
                                    .await;
                            operation_id = None;
                            compiled
                        }
                    }
                } else {
                    compiled
                }
            }
        } else {
            json!({"context": "", "audit": {}})
        };
        if let Some(audit) = result.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("input".to_owned(), state_input_audit);
            audit.insert(
                "manager".to_owned(),
                json!({
                    "enabled": autonomous,
                    "profile": config.mo_state.profile.as_str(),
                    "scene_management": config.mo_state.scene_management,
                    "max_reconcile_steps": config.mo_state.max_reconcile_steps,
                    "max_agent_steps": config.mo_state.max_agent_steps,
                    "operation_timeout_ms": config.mo_state.operation_timeout_ms,
                    "injection_mode": config.mo_state.injection_mode.as_str(),
                }),
            );
        }
        let (context_for_prompt, injection_status) = state_context_for_prompt(
            &result,
            request.momo.mo_state,
            config.mo_state.injection_mode,
        );
        if let Some(audit) = result.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("injection_status".to_owned(), json!(injection_status));
        }
        Ok(CompiledResponseState {
            result,
            operation_id,
            context_for_prompt,
            injection_status,
            guard,
            warnings,
        })
    }
}

fn state_result_from_snapshot(snapshot: &momo_storage::MoStateSnapshot) -> Value {
    let mut audit = snapshot
        .state_audit
        .as_object()
        .cloned()
        .unwrap_or_default();
    audit.insert(
        "runtime_snapshot".to_owned(),
        json!({
            "snapshot_id": snapshot.snapshot_id,
            "space_id": snapshot.space_id,
            "profile": snapshot.profile,
            "dmw_revision": snapshot.dmw_revision,
            "nsg_revision": snapshot.nsg_revision,
            "scene_revision": snapshot.scene_revision,
            "source_versions": snapshot.source_versions,
            "snapshot_revision": snapshot.snapshot_revision,
            "scene": snapshot.scene,
            "degraded": snapshot.degraded,
            "created_at": snapshot.created_at,
        }),
    );
    json!({
        "context": snapshot.state_context,
        "audit": audit,
    })
}

pub(super) fn state_context_for_prompt(
    state_result: &Value,
    enabled: bool,
    mode: MoStateInjectionMode,
) -> (String, &'static str) {
    if !enabled {
        return (String::new(), "disabled");
    }
    if mode == MoStateInjectionMode::Shadow {
        return (String::new(), "shadow");
    }
    if state_result
        .pointer("/audit/degraded")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return (String::new(), "suppressed_degraded");
    }
    if state_result
        .pointer("/audit/input/retrieval_status")
        .and_then(Value::as_str)
        == Some("degraded")
    {
        return (String::new(), "suppressed_degraded_input");
    }
    (
        state_result
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        "active",
    )
}

pub(super) fn compatible_ddm_bands(
    state: Option<&Value>,
    profile_identity: Option<&(u64, String)>,
) -> Value {
    state
        .filter(|state| {
            profile_identity.is_some_and(|(revision, fingerprint)| {
                state["profile_revision"].as_u64() == Some(*revision)
                    && state["profile_fingerprint"].as_str() == Some(fingerprint.as_str())
            })
        })
        .and_then(|state| state.get("bands"))
        .cloned()
        .unwrap_or_else(|| json!({}))
}
