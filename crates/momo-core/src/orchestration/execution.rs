//! Request lifecycle, replay, cancellation, and final commit.

use std::sync::{Arc, Weak};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::{
    GovernedResponseAttempt, ImageInputHandling, MomoApiError, MomoApiService,
    MomoResponseEventSink, ResolvedResponseInput, generation::GatewayGenerationCapability,
};
use crate::{
    ChatUsage, DEFAULT_VISION_ROUTE, GovernedOverrides, MomoResponse, MomoResponseRequest,
    RequestedOverrides, VisionDescriptionRequest, api::simple,
};

impl MomoApiService {
    /// Executes one idempotent native response operation.
    ///
    /// The caller owns transport limits and event serialization. Core owns the
    /// request identity, replay, cancellation, persistence, and maintenance state.
    pub async fn execute(
        &self,
        request: &MomoResponseRequest,
        request_id: &str,
        stream: Option<&dyn MomoResponseEventSink>,
    ) -> Result<MomoResponse, MomoApiError> {
        request
            .validate()
            .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
        let personal_space_id = request
            .momo
            .personal_space_id
            .as_deref()
            .ok_or_else(|| MomoApiError::bad_request("personal_space_id is required"))?;
        let operation_key = scoped_operation_key(personal_space_id, request_id);
        let _operation = self.enter_operation(&operation_key, personal_space_id);
        self.execute_active(request, request_id, &operation_key, stream)
            .await
    }

    /// Cancels a currently active response and its upstream model request.
    pub fn cancel(&self, scope_id: &str, request_id: &str) -> bool {
        let operation_key = scoped_operation_key(scope_id, request_id);
        let known = {
            let mut states = self
                .operation_states
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            states.get_mut(&operation_key).is_some_and(|state| {
                state.cancelled = true;
                state.active > 0
            })
        };
        let upstream = simple::cancel_chat(operation_key);
        known || upstream
    }

    async fn execute_active(
        &self,
        request: &MomoResponseRequest,
        request_id: &str,
        operation_key: &str,
        stream: Option<&dyn MomoResponseEventSink>,
    ) -> Result<MomoResponse, MomoApiError> {
        let config = self.config_snapshot();
        self.ensure_active(operation_key)?;
        let request_lock = {
            let mut locks = self.operation_locks.lock().await;
            if locks.len() >= 1_024 {
                locks.retain(|_, lock| lock.strong_count() > 0);
            }
            if let Some(lock) = locks.get(operation_key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(operation_key.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        let _request_guard = request_lock.lock().await;
        self.ensure_active(operation_key)?;
        let fingerprint = response_request_fingerprint(request)?;
        let persisted = load_response_operation(operation_key).await?;
        if let Some(operation) = &persisted {
            if operation["request_fingerprint"].as_str() != Some(fingerprint.as_str()) {
                return Err(MomoApiError::conflict(
                    "request_id was already used with a different response request",
                ));
            }
            if let Some(response_json) = operation["response_json"].as_str() {
                return serde_json::from_str(response_json)
                    .map_err(|error| MomoApiError::internal(error.to_string()));
            }
        }
        let _conversation_guard =
            if let Some(conversation_id) = request.momo.conversation_id.as_deref() {
                let conversation_space_id = request
                    .momo
                    .conversation_space_id
                    .as_deref()
                    .expect("validated conversation Space");
                Some(
                    self.lock_active_conversation(
                        conversation_space_id,
                        conversation_id,
                        operation_key,
                    )
                    .await?,
                )
            } else {
                None
            };
        if request.input.has_images() && !config.vision.enabled {
            return Err(MomoApiError::bad_request(
                "image input requires vision.enabled = true in the portable MOMO configuration",
            ));
        }

        let mut warnings = Vec::new();
        let capability = match self.gateway_response_budget().await {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!("capability discovery degraded: {error}"));
                GatewayGenerationCapability {
                    context_window: 8_192,
                    max_output_tokens: 1_024,
                    supports_images: false,
                }
            }
        };
        let direct_multimodal = request.input.has_images() && capability.supports_images;
        let mut governed = config
            .govern(
                capability.context_window,
                capability.max_output_tokens,
                RequestedOverrides {
                    context_window: request.context_window,
                    max_output_tokens: request.max_output_tokens,
                    temperature: request.temperature,
                    instructions: request.instructions.as_deref(),
                    visual_description_prompt: if direct_multimodal {
                        None
                    } else {
                        request.visual_description_prompt.as_deref()
                    },
                    parameters: &request.parameters,
                    tool_configuration_requested: !request.tools.is_empty()
                        || request.tool_choice.is_some(),
                },
            )
            .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
        let resolved_input = self
            .resolve_response_input(
                request,
                operation_key,
                persisted.as_ref(),
                &governed,
                capability.supports_images,
            )
            .await?;
        if resolved_input.image_handling == ImageInputHandling::DirectMultimodal {
            governed.visual_description_prompt = None;
            governed.audit["visual_description_prompt_source"] =
                json!("not_used_direct_multimodal");
        }
        self.ensure_active(operation_key)?;

        let (mut response, mo_state_operation_id) = self
            .respond(
                request,
                GovernedResponseAttempt {
                    request_id,
                    operation_key,
                    request_fingerprint: &fingerprint,
                    persisted: persisted.as_ref(),
                    resolved_input: &resolved_input,
                    governed,
                    warnings,
                    stream,
                },
            )
            .await?;
        response.usage.input_tokens = response
            .usage
            .input_tokens
            .saturating_add(resolved_input.vision_usage.input_tokens);
        response.usage.output_tokens = response
            .usage
            .output_tokens
            .saturating_add(resolved_input.vision_usage.output_tokens);
        response.usage.total_tokens = response
            .usage
            .total_tokens
            .saturating_add(resolved_input.vision_usage.total_tokens);
        let description_fallback =
            resolved_input.image_handling == ImageInputHandling::DescriptionFallback;
        response.momo.request_audit["vision"] = json!({
            "applied": description_fallback,
            "mode": resolved_input.image_handling,
            "input_count": resolved_input.visual_input_count,
            "route": match resolved_input.image_handling {
                ImageInputHandling::DirectMultimodal => Some(request.model.as_str()),
                ImageInputHandling::DescriptionFallback => Some(DEFAULT_VISION_ROUTE),
                ImageInputHandling::None => None,
            },
            "usage": &resolved_input.vision_usage,
            "upstream_request_ids": &resolved_input.vision_upstream_request_ids,
        });
        self.ensure_active(operation_key)?;
        let write_source = request
            .momo
            .memory_write_space_id
            .as_ref()
            .and_then(|space_id| {
                request
                    .momo
                    .memory_sources
                    .iter()
                    .find(|source| &source.space_id == space_id)
            });
        let memory = !response.output_text.is_empty() && write_source.is_some_and(|s| s.memory);
        let semantic_graph = !response.output_text.is_empty()
            && write_source.is_some_and(|source| source.semantic_graph);
        let memory_write_space_id = request.momo.memory_write_space_id.clone();
        let maintenance_turn = (memory || semantic_graph).then(|| {
            json!({
                "request_id": operation_key,
                "scope_id": memory_write_space_id,
                "user_content": resolved_input.maintenance_user_text(),
                "assistant_content": response.output_text,
            })
        });
        simple::commit_response_completion_json(
            json!({
                "request_id": operation_key,
                "conversation_scope_id": request.momo.conversation_space_id,
                "conversation_id": response.momo.conversation_id,
                "assistant_content": (!response.output_text.is_empty()).then_some(response.output_text.as_str()),
                "maintenance_turn": maintenance_turn,
                "memory_enabled": memory,
                "nsg_enabled": semantic_graph,
                "mo_state_operation_id": mo_state_operation_id,
                "response_json": serde_json::to_string(&response)
                    .map_err(|error| MomoApiError::internal(error.to_string()))?,
            })
            .to_string(),
        )
        .await
        .map_err(MomoApiError::internal)?;
        if memory || semantic_graph {
            self.schedule_maintenance(
                memory_write_space_id.expect("maintenance write Space was validated"),
            );
        }
        Ok(response)
    }

    pub(super) async fn lock_active_conversation(
        &self,
        conversation_space_id: &str,
        conversation_id: &str,
        operation_key: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, MomoApiError> {
        let guard = self
            .conversation_lock(conversation_space_id, conversation_id)
            .await
            .lock_owned()
            .await;
        // Cancellation can happen while this request is queued behind another
        // response in the same conversation. Re-check before capability or
        // vision calls are allowed to start.
        self.ensure_active(operation_key)?;
        Ok(guard)
    }

    pub(super) async fn resolve_response_input(
        &self,
        request: &MomoResponseRequest,
        request_id: &str,
        persisted: Option<&Value>,
        governed: &GovernedOverrides,
        chat_supports_images: bool,
    ) -> Result<ResolvedResponseInput, MomoApiError> {
        if let Some(resolved) =
            persisted.and_then(|operation| operation["resolved_input_json"].as_str())
        {
            return serde_json::from_str(resolved)
                .map_err(|error| MomoApiError::internal(error.to_string()));
        }
        let images = request.input.image_inputs();
        if images.is_empty() {
            let text = request
                .input
                .text()
                .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
            let user_text = request
                .input
                .user_text()
                .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
            return Ok(ResolvedResponseInput {
                persisted_user_text: if request.input.has_function_outputs() {
                    (!user_text.is_empty()).then_some(user_text)
                } else {
                    Some(text.clone())
                },
                text,
                image_handling: ImageInputHandling::None,
                visual_input_count: 0,
                vision_usage: ChatUsage::default(),
                vision_upstream_request_ids: Vec::new(),
            });
        }
        if chat_supports_images {
            let text = request
                .input
                .text()
                .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
            let image_count = images.len();
            let text = if text.is_empty() {
                format!("[Image input: {image_count}]")
            } else {
                format!("{text}\n[Image input: {image_count}]")
            };
            return Ok(ResolvedResponseInput {
                persisted_user_text: Some(text.clone()),
                text,
                image_handling: ImageInputHandling::DirectMultimodal,
                visual_input_count: image_count,
                vision_usage: ChatUsage::default(),
                vision_upstream_request_ids: Vec::new(),
            });
        }
        let prompt = governed.visual_description_prompt.clone().ok_or_else(|| {
            MomoApiError::bad_request(
                "image input requires vision.enabled = true in the portable MOMO configuration",
            )
        })?;
        let batch = self
            .vision_adapter
            .describe(VisionDescriptionRequest {
                images,
                prompt,
                request_id: request_id.to_owned(),
            })
            .await
            .map_err(|error| MomoApiError::model(error.to_string()))?;
        self.ensure_active(request_id)?;
        let text = request
            .input
            .text_with_image_descriptions(&batch.descriptions)
            .map_err(|error| MomoApiError::model(error.to_string()))?;
        Ok(ResolvedResponseInput {
            persisted_user_text: Some(text.clone()),
            text,
            image_handling: ImageInputHandling::DescriptionFallback,
            visual_input_count: batch.descriptions.len(),
            vision_usage: batch.usage,
            vision_upstream_request_ids: batch.upstream_request_ids,
        })
    }
}

async fn load_response_operation(request_id: &str) -> Result<Option<Value>, MomoApiError> {
    simple::response_operation_json(request_id.to_owned())
        .await
        .map_err(MomoApiError::internal)?
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| MomoApiError::internal(error.to_string()))
        })
        .transpose()
}

pub(super) fn scoped_operation_key(scope_id: &str, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(scope_id.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("op_{}", hex::encode(digest.finalize()))
}

fn response_request_fingerprint(request: &MomoResponseRequest) -> Result<String, MomoApiError> {
    let mut normalized = request.clone();
    normalized.stream = false;
    normalized.momo.stream = false;
    normalized.momo.request_id = None;
    let encoded = serde_json::to_vec(&normalized)
        .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}
