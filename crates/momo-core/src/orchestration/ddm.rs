//! MO State owns generated profile caches; authored overrides remain portable.
use super::{MomoApiError, MomoApiService, generation::GatewayResponseAttempt};
use crate::{
    GatewayMessage, GatewayMessageContent, GatewayMessageRole, MomoRuntimeSettings, PromptSpaceId,
};
use momo_memory::{DdmCalculationProfile, DdmProfile, TokenCounter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const MANAGED_DDM_KIND: &str = "mo_state_ddm_profile";
#[derive(Serialize, Deserialize)]
pub(crate) struct ManagedDdmProfile {
    pub source_fingerprint: String,
    pub profile: DdmProfile,
    pub generation_audit: Value,
}

impl MomoApiService {
    pub(super) async fn resolve_ddm_profile(
        &self,
        space_id: &str,
        character_id: &str,
        character: &momo_domain::CharacterCard,
        operation_key: &str,
        config: &MomoRuntimeSettings,
    ) -> Result<(Option<DdmProfile>, Value), MomoApiError> {
        if !config.mo_state.ddm.enabled {
            return Ok((None, json!({"source":"disabled"})));
        }
        let store = self.runtime().core().store();
        if let Some(yaml) = store
            .portable_metadata("character_ddm_profile", character_id)
            .await
            .map_err(MomoApiError::internal)?
        {
            let profile = DdmProfile::parse_yaml(&yaml).map_err(MomoApiError::internal)?;
            if profile.character_id != character_id {
                return Err(MomoApiError::internal("DDM character identity mismatch"));
            }
            return Ok((
                Some(profile),
                json!({"source":"author_override", "model_calls":0}),
            ));
        }
        let cache_key = format!("{space_id}:{character_id}");
        let _guard = self
            .runtime()
            .lock_control_resource(&format!("ddm:{cache_key}"))
            .await;
        let id = uuid::Uuid::parse_str(character_id).map_err(MomoApiError::internal)?;
        let definition = json!({"character_id": character_id, "name": character.name,
            "character_markdown": character.character_markdown});
        let prompt = self
            .runtime()
            .prompt_spaces()
            .get(PromptSpaceId::DdmProfileGeneration);
        let fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&definition, &prompt.revision)).map_err(MomoApiError::internal)?,
        ));
        if let Some(saved) = store
            .portable_metadata(MANAGED_DDM_KIND, &cache_key)
            .await
            .map_err(MomoApiError::internal)?
        {
            let saved: ManagedDdmProfile =
                serde_json::from_str(&saved).map_err(MomoApiError::internal)?;
            saved.profile.validate().map_err(MomoApiError::internal)?;
            if saved.source_fingerprint == fingerprint && saved.profile.character_id == character_id
            {
                return Ok((
                    Some(saved.profile),
                    json!({"source":"runtime_cache", "source_fingerprint":fingerprint, "model_calls":0}),
                ));
            }
        }
        let capability = self
            .gateway_generation_capability("memory_distillation")
            .await
            .map_err(MomoApiError::model)?;
        let mut error = Value::Null;
        let mut usages = Vec::new();
        for attempt in 1..=config.mo_state.max_reconcile_steps {
            self.coordination.ensure_active(operation_key)?;
            let input = json!({"character_definition":definition, "previous_output_error":error})
                .to_string();
            let defaults = self.purpose_chat_parameters(
                "memory_distillation",
                space_id,
                capability.context_window,
                capability.max_output_tokens,
            )?;
            let output_tokens = defaults.request_parameters["max_tokens"]
                .as_u64()
                .expect("output budget") as usize;
            let tokens = momo_memory::ConservativeTokenCounter.count(&input)
                + momo_memory::ConservativeTokenCounter.count(&prompt.content);
            if tokens.saturating_add(output_tokens) > capability.context_window {
                return Err(MomoApiError::model(
                    "DDM definition exceeds the generation context budget",
                ));
            }
            let completion = self
                .generate_response_completion(GatewayResponseAttempt {
                    request_id: operation_key,
                    operation_key,
                    personal_space_id: space_id,
                    model: "memory_distillation",
                    messages: vec![
                        GatewayMessage {
                            role: GatewayMessageRole::System,
                            content: Some(GatewayMessageContent::Text(prompt.content.clone())),
                            tool_calls: vec![],
                            tool_call_id: None,
                        },
                        GatewayMessage {
                            role: GatewayMessageRole::User,
                            content: Some(GatewayMessageContent::Text(input)),
                            tool_calls: vec![],
                            tool_call_id: None,
                        },
                    ],
                    temperature: defaults.temperature,
                    request_parameters: json!({"max_tokens":output_tokens,"momo_hop":1})
                        .as_object()
                        .expect("object")
                        .clone(),
                    stream: None,
                })
                .await?;
            usages.push(json!({"usage":completion.usage, "upstream_request_id":completion.upstream_request_id}));
            let proposed = if completion.finish_reason.as_deref() != Some("stop")
                || !completion.tool_calls.is_empty()
            {
                Err("DDM proposal must finish normally without tool calls".to_owned())
            } else {
                DdmProfile::parse_yaml(completion.content.trim()).map_err(|e| e.to_string())
                    .and_then(|profile| if profile.character_id != character_id || profile.dispositions.len() > 8
                        || profile.revision != 1 || profile.profile != DdmCalculationProfile::LogitAdditive {
                        Err("DDM proposal requires the captured character, revision 1, logit_additive and at most eight dispositions".into())
                    } else { Ok(profile) })
            };
            match proposed {
                Ok(profile) => {
                    let current = store
                        .character_by_id(id)
                        .await
                        .map_err(MomoApiError::internal)?
                        .ok_or_else(|| MomoApiError::bad_request("DDM character was deleted"))?;
                    if current.name != character.name
                        || current.character_markdown != character.character_markdown
                    {
                        return Err(MomoApiError::conflict(
                            "character changed during DDM generation; retry",
                        ));
                    }
                    self.coordination.ensure_active(operation_key)?;
                    let audit = json!({"source":"runtime_generated", "source_fingerprint":fingerprint,
                        "prompt_revision":prompt.revision, "model_calls":attempt, "calls":usages});
                    let saved = ManagedDdmProfile {
                        source_fingerprint: fingerprint,
                        profile: profile.clone(),
                        generation_audit: audit.clone(),
                    };
                    store
                        .save_portable_metadata(
                            MANAGED_DDM_KIND,
                            &cache_key,
                            &serde_json::to_string(&saved).map_err(MomoApiError::internal)?,
                        )
                        .await
                        .map_err(MomoApiError::internal)?;
                    return Ok((Some(profile), audit));
                }
                Err(message) => error = json!(message),
            }
        }
        Ok((
            None,
            json!({"source":"degraded", "error":format!("DDM profile validation exhausted max_reconcile_steps: {error}"),
            "source_fingerprint":fingerprint,"prompt_revision":prompt.revision,
            "model_calls":config.mo_state.max_reconcile_steps,"calls":usages}),
        ))
    }
}
