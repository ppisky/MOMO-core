//! MO State observation, compilation, and publication.

use serde_json::{Value, json};

use super::{MomoApiError, MomoApiService, ResolvedResponseInput};
use crate::{MoStateInjectionMode, MoStateProfile, MomoResponseRequest, MomoRuntimeSettings};

/// Immutable evidence required to produce one governed state projection.
pub(super) struct StateRequest<'a> {
    pub(super) request: &'a MomoResponseRequest,
    pub(super) config: &'a MomoRuntimeSettings,
    pub(super) operation_key: &'a str,
    pub(super) request_fingerprint: &'a str,
    pub(super) managed_space_id: &'a str,
    pub(super) conversation_id: &'a str,
    pub(super) character_id: &'a str,
    pub(super) source_observations: &'a [momo_storage::MoStateSourceObservation],
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
            Some(
                self.runtime()
                    .lock_space(
                        uuid::Uuid::parse_str(managed_space_id)
                            .map_err(MomoApiError::bad_request)?,
                    )
                    .await
                    .map_err(MomoApiError::internal)?,
            )
        } else {
            None
        };
        let mut operation_id = None;
        let mut operation = None;
        let mut result = if request.momo.mo_state {
            let persisted_snapshot = if autonomous {
                let mut observed_sources = source_observations.to_vec();
                let managed_source = observed_sources
                    .iter()
                    .position(|source| source.space_id == managed_space_id)
                    .map(|index| observed_sources.remove(index));
                let observed = if let Some(source) = managed_source {
                    self.runtime()
                        .core()
                        .store()
                        .observe_mo_state_operation(&momo_storage::MoStateObservation {
                            operation_id: operation_key.to_owned(),
                            space_id: managed_space_id.to_owned(),
                            event_type: if request.input.has_function_outputs() {
                                "tool_result".to_owned()
                            } else {
                                "user_message".to_owned()
                            },
                            event_fingerprint: request_fingerprint.to_owned(),
                            profile: config.mo_state.profile.as_str().to_owned(),
                            dmw_fingerprint: source.dmw_fingerprint,
                            nsg_fingerprint: source.nsg_fingerprint,
                            scene_fingerprint: source.scene_fingerprint,
                            scene_json: source.scene_json,
                            source_observations: observed_sources,
                        })
                        .await
                        .map_err(|error| error.to_string())
                } else {
                    Err("managed Space source was not observed".to_owned())
                };
                match observed {
                    Ok(observed) => {
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
                let ddm_profile = if config.mo_state.ddm.enabled {
                    let character_uuid =
                        uuid::Uuid::parse_str(character_id).map_err(MomoApiError::internal)?;
                    self.runtime()
                        .core()
                        .store()
                        .portable_metadata("character_ddm_profile", &character_uuid.to_string())
                        .await
                        .map_err(MomoApiError::internal)?
                        .map(|yaml| momo_memory::DdmProfile::parse_yaml(&yaml))
                        .transpose()
                        .map_err(MomoApiError::internal)?
                } else {
                    None
                };
                let previous_ddm_state = if autonomous && ddm_profile.is_some() {
                    self.runtime()
                        .core()
                        .store()
                        .ddm_projection_state(managed_space_id, conversation_id, character_id)
                        .await
                        .map_err(MomoApiError::internal)?
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(MomoApiError::internal)?
                } else {
                    None
                };
                let profile_identity = ddm_profile
                    .as_ref()
                    .map(|profile| (profile.revision, profile.profile_fingerprint()));
                let previous_bands =
                    compatible_ddm_bands(previous_ddm_state.as_ref(), profile_identity.as_ref());
                let observed_scene = operation
                    .as_ref()
                    .map(|operation| operation.observed_scene.clone())
                    .or_else(|| {
                        source_observations.iter().find_map(|source| {
                            (source.space_id == managed_space_id)
                                .then(|| serde_json::from_str::<Value>(&source.scene_json).ok())
                                .flatten()
                        })
                    });
                let ddm_runtime = ddm_profile
                    .as_ref()
                    .map(|_| {
                        serde_json::from_value(json!({
                            "previous_bands": previous_bands,
                            "scene": observed_scene,
                            "request_event_type": if request.input.has_function_outputs() {
                                "tool_result"
                            } else {
                                "user_message"
                            },
                            "request_has_image": resolved_input.visual_input_count > 0,
                            "request_evidence_id": operation_key,
                        }))
                    })
                    .transpose()
                    .map_err(MomoApiError::internal)?;
                let parsed_scope_id =
                    uuid::Uuid::parse_str(managed_space_id).map_err(MomoApiError::internal)?;
                let retrieved_memory = serde_json::from_value::<Vec<momo_memory::RetrievedMemory>>(
                    Value::Array(memory.to_vec()),
                )
                .map_err(MomoApiError::internal)?;
                let retrieved_nsg = serde_json::from_value::<Vec<momo_memory::nsg::RetrievedNsg>>(
                    Value::Array(nsg.to_vec()),
                )
                .map_err(MomoApiError::internal)?;
                let (compiled, degraded, compile_error) =
                    match crate::api::runtime_api::compile_mo_state_with_ddm(
                        self.runtime(),
                        parsed_scope_id,
                        retrieved_memory,
                        retrieved_nsg,
                        context_window,
                        ddm_profile,
                        ddm_runtime,
                    )
                    .await
                    {
                        Ok(value) => (
                            serde_json::to_value(value).map_err(MomoApiError::internal)?,
                            false,
                            None,
                        ),
                        Err(error) => {
                            warnings.push(format!("MO State degraded: {error}"));
                            (
                                json!({"context": "", "audit": {"degraded": true}}),
                                true,
                                Some(error.to_string()),
                            )
                        }
                    };
                if let Some(state_operation_id) = operation_id.as_ref() {
                    let ddm_update = compiled
                        .get("audit")
                        .and_then(|audit| audit.get("ddm"))
                        .map(|ddm| {
                            serde_json::from_value::<momo_storage::DdmProjectionUpdate>(json!({
                                "managed_space_id": managed_space_id,
                                "conversation_id": conversation_id,
                                "character_id": character_id,
                                "profile_revision": ddm["profile_revision"],
                                "profile_fingerprint": ddm["profile_fingerprint"],
                                "source_fingerprint": ddm["source_fingerprint"],
                                "bands": ddm["next_bands"],
                            }))
                        })
                        .transpose()
                        .map_err(MomoApiError::internal)?;
                    let compiled_json =
                        serde_json::to_string(&compiled).map_err(MomoApiError::internal)?;
                    match self
                        .runtime()
                        .core()
                        .store()
                        .publish_mo_state_snapshot(
                            state_operation_id,
                            &compiled_json,
                            degraded,
                            compile_error.as_deref(),
                            ddm_update.as_ref(),
                        )
                        .await
                    {
                        Ok(snapshot) => state_result_from_snapshot(&snapshot),
                        Err(error) => {
                            warnings
                                .push(format!("MO State snapshot persistence degraded: {error}"));
                            let _ = self
                                .runtime()
                                .core()
                                .store()
                                .fail_mo_state_operation(state_operation_id, &error.to_string())
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
