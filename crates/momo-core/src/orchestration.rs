//! Client-independent implementation of the native high-level MomoApi response pipeline.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as SyncMutex, Weak},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{
    ChatUsage, DEFAULT_VISION_ROUTE, EmbeddingNormalization, EmbeddingProfile,
    GatewayVisionAdapter, GovernedOverrides, MomoConfig, MomoResponse, MomoResponseMetadata,
    MomoResponseRequest, RequestedOverrides, ResponseOutputContent, ResponseOutputItem,
    ResponseUsage, VisionDescriptionAdapter, VisionDescriptionRequest, api::simple,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MomoApiErrorKind {
    BadRequest,
    Conflict,
    Cancelled,
    Model,
    Internal,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct MomoApiError {
    pub kind: MomoApiErrorKind,
    pub message: String,
}

impl MomoApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: MomoApiErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn model(message: impl Into<String>) -> Self {
        Self {
            kind: MomoApiErrorKind::Model,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: MomoApiErrorKind::Conflict,
            message: message.into(),
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: MomoApiErrorKind::Cancelled,
            message: "response request was cancelled".to_owned(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: MomoApiErrorKind::Internal,
            message: message.into(),
        }
    }
}

pub trait MomoResponseEventSink: Send + Sync {
    fn send(&self, event: Value) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceKind {
    Memory,
    SemanticGraph,
}

impl MaintenanceKind {
    const fn storage_name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::SemanticGraph => "semantic_graph",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MomoApiService {
    gateway_origin: String,
    gateway_api_key: Option<String>,
    gateway_client: reqwest::Client,
    config: Arc<MomoConfig>,
    vision_adapter: Arc<dyn VisionDescriptionAdapter>,
    response_attempts: Arc<Mutex<HashMap<String, String>>>,
    operation_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    operation_states: Arc<SyncMutex<HashMap<String, OperationState>>>,
    maintenance_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
}

#[derive(Debug, Default)]
struct OperationState {
    active: usize,
    cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ResolvedResponseInput {
    text: String,
    #[serde(default)]
    image_handling: ImageInputHandling,
    #[serde(default)]
    visual_input_count: usize,
    #[serde(default)]
    vision_usage: ChatUsage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    vision_upstream_request_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ImageInputHandling {
    #[default]
    None,
    DirectMultimodal,
    DescriptionFallback,
}

#[derive(Debug, Clone, Copy)]
struct GatewayGenerationCapability {
    context_window: usize,
    max_output_tokens: usize,
    supports_images: bool,
}

struct GovernedResponseAttempt<'a> {
    request_id: &'a str,
    operation_key: &'a str,
    request_fingerprint: &'a str,
    persisted: Option<&'a Value>,
    resolved_input: &'a ResolvedResponseInput,
    governed: GovernedOverrides,
    warnings: Vec<String>,
    stream: Option<&'a dyn MomoResponseEventSink>,
}

struct OperationGuard {
    request_id: String,
    states: Arc<SyncMutex<HashMap<String, OperationState>>>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = states.get_mut(&self.request_id) else {
            return;
        };
        state.active -= 1;
        if state.active == 0 {
            states.remove(&self.request_id);
        }
    }
}

impl MomoApiService {
    #[must_use]
    pub fn new(
        gateway_origin: impl Into<String>,
        gateway_api_key: Option<String>,
        gateway_client: reqwest::Client,
        config: Arc<MomoConfig>,
    ) -> Self {
        let gateway_origin = gateway_origin.into();
        let vision_adapter = Arc::new(GatewayVisionAdapter::new(
            gateway_client.clone(),
            gateway_origin.clone(),
            gateway_api_key.clone(),
            DEFAULT_VISION_ROUTE,
        ));
        Self {
            gateway_origin,
            gateway_api_key,
            gateway_client,
            config,
            vision_adapter,
            response_attempts: Arc::new(Mutex::new(HashMap::new())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            operation_states: Arc::new(SyncMutex::new(HashMap::new())),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[must_use]
    pub fn with_vision_adapter(mut self, adapter: Arc<dyn VisionDescriptionAdapter>) -> Self {
        self.vision_adapter = adapter;
        self
    }

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
        let _operation = self.enter_operation(&operation_key);
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
        if request.input.has_images() && !self.config.vision.enabled {
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
        let mut governed = self
            .config
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

        let mut response = self
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
        let maintenance_registered = if memory || semantic_graph {
            match simple::append_maintenance_turn_json(
                json!({
                    "request_id": operation_key,
                    "scope_id": memory_write_space_id,
                    "user_content": &resolved_input.text,
                    "assistant_content": response.output_text,
                })
                .to_string(),
                memory,
                semantic_graph,
            )
            .await
            {
                Ok(()) => true,
                Err(error) => {
                    response.momo.warnings.push(format!(
                        "background maintenance was not registered: {error}"
                    ));
                    false
                }
            }
        } else {
            false
        };
        simple::complete_response_operation(
            operation_key.to_owned(),
            serde_json::to_string(&response)
                .map_err(|error| MomoApiError::internal(error.to_string()))?,
        )
        .await
        .map_err(MomoApiError::internal)?;
        if maintenance_registered {
            self.schedule_maintenance(
                memory_write_space_id.expect("maintenance write Space was validated"),
            );
        }
        Ok(response)
    }

    async fn resolve_response_input(
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
            return Ok(ResolvedResponseInput {
                text: request
                    .input
                    .text()
                    .map_err(|error| MomoApiError::bad_request(error.to_string()))?,
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
            text,
            image_handling: ImageInputHandling::DescriptionFallback,
            visual_input_count: batch.descriptions.len(),
            vision_usage: batch.usage,
            vision_upstream_request_ids: batch.upstream_request_ids,
        })
    }

    fn enter_operation(&self, request_id: &str) -> OperationGuard {
        let mut states = self
            .operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states.entry(request_id.to_owned()).or_default().active += 1;
        OperationGuard {
            request_id: request_id.to_owned(),
            states: Arc::clone(&self.operation_states),
        }
    }

    fn ensure_active(&self, request_id: &str) -> Result<(), MomoApiError> {
        if self
            .operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(request_id)
            .is_some_and(|state| state.cancelled)
        {
            Err(MomoApiError::cancelled())
        } else {
            Ok(())
        }
    }

    fn schedule_maintenance(&self, scope_id: String) {
        for (kind, enabled, threshold) in [
            (
                MaintenanceKind::Memory,
                self.config.runtime.memory_distillation_enabled,
                self.config.runtime.memory_distill_every_turns,
            ),
            (
                MaintenanceKind::SemanticGraph,
                self.config.runtime.semantic_graph_enabled,
                self.config.runtime.nsg_govern_every_turns,
            ),
        ] {
            if !enabled {
                continue;
            }
            let service = self.clone();
            let scope_id = scope_id.clone();
            tokio::spawn(async move {
                if let Err(error) = service.maintain(&scope_id, kind, threshold).await {
                    tracing::warn!(?kind, %error, "background response maintenance failed; turns remain pending");
                }
            });
        }
    }

    /// Runs one governed Core maintenance batch. Callers only choose when to schedule it.
    pub async fn maintain(
        &self,
        scope_id: &str,
        kind: MaintenanceKind,
        threshold: usize,
    ) -> Result<bool, MomoApiError> {
        if threshold == 0 {
            return Err(MomoApiError::bad_request(
                "maintenance threshold must be positive",
            ));
        }
        let storage_kind = kind.storage_name();
        let lock_key = format!("{scope_id}:{storage_kind}");
        let task_lock = {
            let mut locks = self.maintenance_locks.lock().await;
            if locks.len() >= 1_024 {
                locks.retain(|_, lock| lock.strong_count() > 0);
            }
            if let Some(lock) = locks.get(&lock_key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(lock_key, Arc::downgrade(&lock));
                lock
            }
        };
        let _guard = task_lock.lock().await;
        let pending = simple::pending_maintenance_turns_json(
            scope_id.to_owned(),
            storage_kind.to_owned(),
            threshold,
        )
        .await
        .map_err(MomoApiError::internal)?;
        let turns: Vec<Value> = serde_json::from_str(&pending)
            .map_err(|error| MomoApiError::internal(error.to_string()))?;
        if turns.len() < threshold {
            return Ok(false);
        }
        let transcript = turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                format!(
                    "Turn {}\nUser:\n{}\nAssistant:\n{}",
                    index + 1,
                    turn.get("user_content")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    turn.get("assistant_content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let existing_context = simple::retrieve_memory_json(
            scope_id.to_owned(),
            transcript.clone(),
            4_096,
        )
        .await
        .map_err(|error| {
            MomoApiError::internal(format!(
                "maintenance context retrieval failed; refusing transcript-only write: {error}"
            ))
        })?;
        let existing_context: Vec<Value> = serde_json::from_str(&existing_context)
            .map_err(|error| MomoApiError::internal(error.to_string()))?;
        let maintenance_input = json!({
            "maintenance_kind": storage_kind,
            "current_unix_timestamp": Utc::now().timestamp(),
            "existing_context": existing_context,
            "pending_turns": turns,
        });
        let (route, system) = match kind {
            MaintenanceKind::Memory => (
                "memory_distillation",
                self.config.prompts.memory_distillation.as_str(),
            ),
            MaintenanceKind::SemanticGraph => (
                "semantic_graph_governance",
                self.config.prompts.semantic_graph_governance.as_str(),
            ),
        };
        let completion = simple::chat_complete_json(
            json!({
                "base_url": self.gateway_origin,
                "api_key": self.gateway_api_key,
                "model": route,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": maintenance_input.to_string()}
                ],
                "request_parameters": {"max_tokens": 2048, "momo_hop": 1}
            })
            .to_string(),
        )
        .await
        .map_err(MomoApiError::model)?;
        let completion: Value = serde_json::from_str(&completion)
            .map_err(|error| MomoApiError::model(error.to_string()))?;
        let patch = completion
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| MomoApiError::model("maintenance model returned no text"))?;
        match kind {
            MaintenanceKind::Memory => {
                simple::apply_memory_patch_json(scope_id.to_owned(), patch.to_owned()).await
            }
            MaintenanceKind::SemanticGraph => {
                simple::apply_nsg_patch_json(scope_id.to_owned(), patch.to_owned(), false).await
            }
        }
        .map_err(MomoApiError::internal)?;
        let request_ids = turns
            .iter()
            .filter_map(|turn| turn.get("request_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        simple::mark_maintenance_turns_done(request_ids, storage_kind.to_owned())
            .await
            .map_err(MomoApiError::internal)?;
        Ok(true)
    }

    async fn respond(
        &self,
        request: &MomoResponseRequest,
        attempt: GovernedResponseAttempt<'_>,
    ) -> Result<MomoResponse, MomoApiError> {
        let GovernedResponseAttempt {
            request_id,
            operation_key,
            request_fingerprint,
            persisted,
            resolved_input,
            governed,
            mut warnings,
            stream,
        } = attempt;
        let input = resolved_input.text.as_str();
        let direct_multimodal =
            resolved_input.image_handling == ImageInputHandling::DirectMultimodal;
        let personal_space_id = request
            .momo
            .personal_space_id
            .clone()
            .expect("validated personal Space");
        let conversation_space_id = request
            .momo
            .conversation_space_id
            .clone()
            .expect("validated conversation Space");
        let requested_character_id = request.momo.character_id.as_deref();
        let attempted_conversation = persisted
            .and_then(|operation| operation["conversation_id"].as_str())
            .map(str::to_owned)
            .or(self
                .response_attempts
                .lock()
                .await
                .get(operation_key)
                .cloned());
        let existing_conversation_id = request
            .momo
            .conversation_id
            .as_deref()
            .map(str::to_owned)
            .or(attempted_conversation);
        let (conversation_id, character_id) = if let Some(id) = existing_conversation_id {
            let conversations =
                parse_json(simple::local_conversations_json(conversation_space_id.clone()).await)?;
            let conversation = conversations
                .as_array()
                .and_then(|items| {
                    items.iter().find(|conversation| {
                        conversation.get("id").and_then(Value::as_str) == Some(id.as_str())
                    })
                })
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "conversation_id does not belong to conversation_space_id",
                    )
                })?;
            let stored_character_id = conversation
                .get("character_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "conversation has no character; use switch_character control first",
                    )
                })?;
            if requested_character_id.is_some_and(|value| value != stored_character_id) {
                return Err(MomoApiError::bad_request(
                    "character_id differs from the conversation; use switch_character control",
                ));
            }
            (id, stored_character_id.to_owned())
        } else {
            let character_id = requested_character_id.ok_or_else(|| {
                MomoApiError::bad_request("character_id is required for a new conversation")
            })?;
            let created = parse_json(
                simple::stage_conversation_json(
                    None,
                    conversation_space_id.clone(),
                    request
                        .momo
                        .title
                        .clone()
                        .unwrap_or_else(|| "MOMO response".to_owned()),
                    character_id.to_owned(),
                )
                .await,
            )?;
            let id = required_str(&created, "id")?.to_owned();
            self.response_attempts
                .lock()
                .await
                .insert(operation_key.to_owned(), id.clone());
            (id, character_id.to_owned())
        };
        let character = parse_json(simple::local_character_json(character_id.clone()).await)
            .map_err(|_| MomoApiError::bad_request("character_id does not exist"))?;
        self.response_attempts
            .lock()
            .await
            .entry(operation_key.to_owned())
            .or_insert_with(|| conversation_id.clone());
        simple::begin_response_operation(
            operation_key.to_owned(),
            request_fingerprint.to_owned(),
            conversation_id.clone(),
            serde_json::to_string(resolved_input)
                .map_err(|error| MomoApiError::internal(error.to_string()))?,
        )
        .await
        .map_err(MomoApiError::internal)?;
        self.response_attempts.lock().await.remove(operation_key);

        let user_already_written = persisted
            .and_then(|operation| operation["user_written"].as_bool())
            .unwrap_or(false);
        if !user_already_written {
            simple::append_response_user_message_json(
                operation_key.to_owned(),
                conversation_space_id.clone(),
                conversation_id.clone(),
                input.to_owned(),
            )
            .await
            .map_err(MomoApiError::internal)?;
        }

        let context_window = governed.context_window;
        let reserve_output_tokens = governed.max_output_tokens;
        let include_memory = request
            .momo
            .memory_sources
            .iter()
            .any(|source| source.memory);
        let include_semantic_graph = request
            .momo
            .memory_sources
            .iter()
            .any(|source| source.semantic_graph);
        let query_embedding = if include_semantic_graph {
            match self.generate_query_embedding(input).await {
                Ok(value) => Some(value),
                Err(error) => {
                    warnings.push(format!("embedding degraded: {error}"));
                    None
                }
            }
        } else {
            None
        };
        let retrieval_enabled = !request.momo.memory_sources.is_empty();
        let retrieved = if retrieval_enabled {
            let payload = json!({
                "spaces": request.momo.memory_sources,
                "query": input,
                "max_tokens": context_window
                    .saturating_sub(reserve_output_tokens)
                    .saturating_div(8)
                    .clamp(128, 2_048),
                "vector_space_id": query_embedding.as_ref().map(|value| &value.0),
                "query_vector": query_embedding.as_ref().map(|value| &value.1),
            });
            match simple::retrieve_scoped_memory_json(payload.to_string()).await {
                Ok(value) => serde_json::from_str::<Value>(&value)
                    .map_err(|error| MomoApiError::internal(error.to_string()))?,
                Err(error) => {
                    warnings.push(format!("retrieval degraded: {error}"));
                    json!([])
                }
            }
        } else {
            json!([])
        };
        let items = retrieved.as_array().cloned().unwrap_or_default();
        let (nsg, memory): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|item| item.get("graph_id").is_some());
        let state_result = if request.momo.mo_state {
            match simple::compile_mo_state_json(
                personal_space_id.clone(),
                serde_json::to_string(&memory)
                    .map_err(|error| MomoApiError::internal(error.to_string()))?,
                serde_json::to_string(&nsg)
                    .map_err(|error| MomoApiError::internal(error.to_string()))?,
                context_window,
            )
            .await
            {
                Ok(value) => serde_json::from_str::<Value>(&value)
                    .map_err(|error| MomoApiError::internal(error.to_string()))?,
                Err(error) => {
                    warnings.push(format!("MO State degraded: {error}"));
                    json!({"context": "", "audit": {}})
                }
            }
        } else {
            json!({"context": "", "audit": {}})
        };

        let history = parse_json(
            simple::local_messages_json(conversation_space_id.clone(), conversation_id.clone())
                .await,
        )?;
        let mut messages = history
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|message| {
                Some(json!({
                    "role": message.get("role")?.as_str()?,
                    "content": message.get("content")?.as_str()?,
                }))
            })
            .collect::<Vec<_>>();
        if request.input.is_structured()
            && (direct_multimodal || !request.input.has_images())
            && messages.last().is_some_and(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && message.get("content").and_then(Value::as_str) == Some(input)
            })
        {
            messages.pop();
        }
        if let Some(instructions) = governed
            .instructions
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            messages.insert(0, json!({"role": "system", "content": instructions}));
        }
        let prepared = parse_json(simple::prepare_context_json(
            json!({
                "character_markdown": character.get("character_markdown").and_then(Value::as_str).unwrap_or_default(),
                "user_markdown": character.get("user_markdown").and_then(Value::as_str).unwrap_or_default(),
                "memory_markdown": if include_memory { joined_bodies(&memory) } else { String::new() },
                "state_context": state_result.get("context").and_then(Value::as_str).unwrap_or_default(),
                "nsg_markdown": if include_semantic_graph { joined_bodies(&nsg) } else { String::new() },
                "messages": messages,
                "context_window": context_window,
                "reserve_output_tokens": reserve_output_tokens,
            })
            .to_string(),
        ))?;
        let mut gateway_messages = prepared
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if request.input.is_structured() && (direct_multimodal || !request.input.has_images()) {
            gateway_messages.extend(
                request
                    .input
                    .gateway_messages()
                    .map_err(|error| MomoApiError::bad_request(error.to_string()))?
                    .into_iter()
                    .map(|message| serde_json::to_value(message).expect("message serializes")),
            );
        }
        let mut request_parameters = json!({
            "max_tokens": reserve_output_tokens,
            "momo_hop": 1,
            "momo_request_id": operation_key,
        });
        let parameter_object = request_parameters
            .as_object_mut()
            .expect("request parameters are an object");
        parameter_object.extend(governed.parameters.clone());
        if governed.allow_tools && !request.tools.is_empty() {
            parameter_object.insert("tools".to_owned(), tools_to_chat(&request.tools)?);
        }
        if governed.allow_tools
            && let Some(choice) = &request.tool_choice
        {
            parameter_object.insert("tool_choice".to_owned(), tool_choice_to_chat(choice)?);
        }
        let gateway_request = json!({
            "base_url": self.gateway_origin,
            "api_key": self.gateway_api_key,
            "model": request.model,
            "messages": gateway_messages,
            "temperature": governed.temperature,
            "request_parameters": request_parameters,
        });
        let completion = if let Some(stream) = stream {
            let message_item_id = format!("msg_{request_id}");
            stream.send(json!({
                "type": "response.output_item.added",
                "request_id": request_id,
                "output_index": 0,
                "item": {"id": message_item_id, "type": "message", "role": "assistant", "status": "in_progress", "content": []},
            })).map_err(MomoApiError::internal)?;
            stream
                .send(json!({
                    "type": "response.content_part.added",
                    "request_id": request_id,
                    "item_id": message_item_id,
                    "output_index": 0,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": "", "annotations": []},
                }))
                .map_err(MomoApiError::internal)?;
            let streamed_tools = Arc::new(std::sync::Mutex::new(HashMap::<
                usize,
                (String, String, bool),
            >::new()));
            let sink = |event_json: String| {
                let event: Value =
                    serde_json::from_str(&event_json).map_err(|error| error.to_string())?;
                if event.get("type").and_then(Value::as_str) != Some("delta") {
                    return Ok(());
                }
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !delta.is_empty() {
                    stream.send(json!({
                        "type": "response.output_text.delta", "request_id": request_id,
                        "item_id": message_item_id, "output_index": 0, "content_index": 0, "delta": delta,
                    }))?;
                }
                let mut tools = streamed_tools.lock().map_err(|error| error.to_string())?;
                for call in event
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let tool = tools
                        .entry(index)
                        .or_insert_with(|| (String::new(), String::new(), false));
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        tool.0 = id.to_owned();
                    }
                    if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                        tool.1 = name.to_owned();
                    }
                    if !tool.2 && !tool.0.is_empty() && !tool.1.is_empty() {
                        stream.send(json!({
                            "type": "response.output_item.added", "request_id": request_id,
                            "output_index": index + 1,
                            "item": {"id": tool.0, "call_id": tool.0, "type": "function_call", "name": tool.1, "arguments": "", "status": "in_progress"},
                        }))?;
                        tool.2 = true;
                    }
                    if let Some(arguments) = call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                    {
                        stream.send(json!({
                            "type": "response.function_call_arguments.delta", "request_id": request_id,
                            "item_id": tool.0, "output_index": index + 1, "delta": arguments,
                        }))?;
                    }
                }
                Ok(())
            };
            let completion = simple::chat_stream_json(
                json!({
                    "request_id": operation_key,
                    "base_url": gateway_request["base_url"],
                    "api_key": gateway_request["api_key"],
                    "model": gateway_request["model"],
                    "messages": gateway_request["messages"],
                    "request_parameters": gateway_request["request_parameters"],
                })
                .to_string(),
                sink,
            )
            .await
            .map_err(MomoApiError::model)?;
            serde_json::from_str(&completion).map_err(|error| {
                MomoApiError::model(format!("model returned invalid JSON: {error}"))
            })?
        } else {
            let completion = simple::chat_complete_json(gateway_request.to_string())
                .await
                .map_err(MomoApiError::model)?;
            serde_json::from_str(&completion).map_err(|error| {
                MomoApiError::model(format!("model returned invalid JSON: {error}"))
            })?
        };
        let content = required_str(&completion, "content")?.trim().to_owned();
        let tool_calls = completion
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if content.is_empty() && tool_calls.is_empty() {
            return Err(MomoApiError::model(
                "model gateway returned neither text nor tool calls",
            ));
        }
        if !content.is_empty() {
            simple::stage_message_json(
                conversation_space_id,
                conversation_id.clone(),
                "assistant".to_owned(),
                content.clone(),
            )
            .await
            .map_err(MomoApiError::internal)?;
        }
        let mut output = vec![ResponseOutputItem::Message {
            id: format!("msg_{request_id}"),
            role: "assistant".to_owned(),
            status: "completed".to_owned(),
            content: vec![ResponseOutputContent::OutputText {
                text: content.clone(),
                annotations: Vec::new(),
            }],
        }];
        for call in tool_calls {
            let id = required_str(&call, "id")?.to_owned();
            let function = call
                .get("function")
                .ok_or_else(|| MomoApiError::model("tool call is missing function"))?;
            output.push(ResponseOutputItem::FunctionCall {
                id: id.clone(),
                call_id: id,
                name: required_str(function, "name")?.to_owned(),
                arguments: required_str(function, "arguments")?.to_owned(),
                status: "completed".to_owned(),
            });
        }
        Ok(MomoResponse {
            id: format!("resp_{request_id}"),
            object: "response".to_owned(),
            status: "completed".to_owned(),
            model: request.model.clone(),
            output,
            output_text: content,
            usage: response_usage(&completion),
            finish_reason: completion
                .get("finish_reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            momo: MomoResponseMetadata {
                schema: crate::MOMO_RESPONSE_SCHEMA.to_owned(),
                request_id: request_id.to_owned(),
                conversation_id,
                route: request.model.clone(),
                upstream_request_id: completion
                    .get("upstream_request_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                warnings,
                state_audit: state_result
                    .get("audit")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                request_audit: governed.audit,
            },
        })
    }

    async fn gateway_response_budget(&self) -> Result<GatewayGenerationCapability, String> {
        let discovery = self.gateway_model_detail("conversation").await?;
        let context_window = discovery
            .pointer("/momo/context_window")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(8_192);
        let max_output_tokens = discovery
            .pointer("/momo/max_output_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(1_024);
        if context_window < 1_024 || max_output_tokens == 0 || max_output_tokens >= context_window {
            return Err("gateway returned an invalid conversation token budget".to_owned());
        }
        let supports_images = discovery
            .pointer("/momo/modalities")
            .and_then(Value::as_array)
            .is_some_and(|modalities| {
                modalities
                    .iter()
                    .any(|modality| modality.as_str() == Some("image"))
            });
        Ok(GatewayGenerationCapability {
            context_window,
            max_output_tokens,
            supports_images,
        })
    }

    async fn gateway_model_detail(&self, alias: &str) -> Result<Value, String> {
        let url = format!(
            "{}/models/{alias}",
            self.gateway_origin.trim_end_matches('/')
        );
        let mut request = self
            .gateway_client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(api_key) = self.gateway_api_key.as_deref() {
            request = request.bearer_auth(api_key);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "gateway model discovery returned HTTP {}",
                status.as_u16()
            ));
        }
        response.json().await.map_err(|error| error.to_string())
    }

    async fn generate_query_embedding(&self, input: &str) -> Result<(String, Vec<f64>), String> {
        let discovery = self.gateway_model_detail("embedding").await?;
        let metadata: GatewayEmbeddingProfile = serde_json::from_value(
            discovery
                .pointer("/momo/embedding_profile")
                .cloned()
                .ok_or_else(|| "gateway embedding route has no embedding_profile".to_owned())?,
        )
        .map_err(|error| error.to_string())?;
        let profile = EmbeddingProfile {
            provider_id: "mobot-gateway".to_owned(),
            endpoint_id: self.gateway_origin.clone(),
            model: "embedding".to_owned(),
            dimension: metadata.dimension,
            model_revision: metadata.model_revision,
            normalization: metadata.normalization,
            send_dimensions: metadata.send_dimensions,
            query_prefix: metadata.query_prefix,
            document_prefix: metadata.document_prefix,
        };
        let result = simple::generate_embeddings_json(json!({
            "embedding": {"endpoint": {"base_url": self.gateway_origin, "api_key": self.gateway_api_key}, "profile": profile, "timeout_seconds": 120},
            "inputs": [{"id": "query", "text": input, "purpose": "query"}],
        }).to_string()).await?;
        let batch: Value = serde_json::from_str(&result).map_err(|error| error.to_string())?;
        let vector_space_id = batch
            .get("vector_space_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "embedding response lacks vector_space_id".to_owned())?
            .to_owned();
        let vector = batch
            .pointer("/vectors/0/vector")
            .and_then(Value::as_array)
            .ok_or_else(|| "embedding response lacks vectors[0].vector".to_owned())?
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .ok_or_else(|| "embedding vector contains a non-number".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((vector_space_id, vector))
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

fn scoped_operation_key(scope_id: &str, request_id: &str) -> String {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayEmbeddingProfile {
    dimension: usize,
    normalization: EmbeddingNormalization,
    #[serde(default)]
    send_dimensions: bool,
    #[serde(default)]
    model_revision: Option<String>,
    #[serde(default)]
    query_prefix: String,
    #[serde(default)]
    document_prefix: String,
}

fn parse_json(result: Result<String, String>) -> Result<Value, MomoApiError> {
    let value = result.map_err(MomoApiError::internal)?;
    serde_json::from_str(&value).map_err(|error| MomoApiError::internal(error.to_string()))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, MomoApiError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| MomoApiError::internal(format!("response is missing string field {field}")))
}

fn joined_bodies(values: &[Value]) -> String {
    values
        .iter()
        .filter_map(|value| value.get("body").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn tools_to_chat(tools: &[crate::ResponseTool]) -> Result<Value, MomoApiError> {
    let tools = serde_json::to_value(tools)
        .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
    tools
        .as_array()
        .ok_or_else(|| MomoApiError::bad_request("tools must be an array"))?
        .iter()
        .map(|tool| {
            let mut function = tool
                .as_object()
                .cloned()
                .ok_or_else(|| MomoApiError::bad_request("tool must be an object"))?;
            function.remove("type");
            Ok(json!({"type": "function", "function": function}))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn tool_choice_to_chat(choice: &Value) -> Result<Value, MomoApiError> {
    if choice.is_string() {
        return Ok(choice.clone());
    }
    let name = choice
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| MomoApiError::bad_request("function tool_choice requires name"))?;
    Ok(json!({"type": "function", "function": {"name": name}}))
}

fn response_usage(completion: &Value) -> ResponseUsage {
    let input_tokens = completion
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let output_tokens = completion
        .pointer("/usage/output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let total_tokens = completion
        .pointer("/usage/total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    ResponseUsage {
        input_tokens,
        output_tokens,
        total_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{FutureExt, future::BoxFuture};

    #[derive(Debug)]
    struct FixedVisionAdapter;

    impl VisionDescriptionAdapter for FixedVisionAdapter {
        fn describe(
            &self,
            request: VisionDescriptionRequest,
        ) -> BoxFuture<'static, Result<crate::VisionDescriptionBatch, crate::VisionError>> {
            async move {
                assert_eq!(request.prompt, "Describe visible facts.");
                assert_eq!(request.images.len(), 1);
                Ok(crate::VisionDescriptionBatch {
                    descriptions: vec!["A red umbrella on a wet street.".to_owned()],
                    usage: ChatUsage {
                        input_tokens: 12,
                        output_tokens: 8,
                        total_tokens: 20,
                    },
                    upstream_request_ids: vec!["vision-upstream-1".to_owned()],
                })
            }
            .boxed()
        }
    }

    #[derive(Debug)]
    struct UnexpectedVisionAdapter;

    impl VisionDescriptionAdapter for UnexpectedVisionAdapter {
        fn describe(
            &self,
            _request: VisionDescriptionRequest,
        ) -> BoxFuture<'static, Result<crate::VisionDescriptionBatch, crate::VisionError>> {
            async move { panic!("direct multimodal input must not call the vision adapter") }
                .boxed()
        }
    }

    #[test]
    fn cancellation_state_is_active_only_and_drop_safe() {
        let service = MomoApiService::new(
            "http://127.0.0.1:9/v1",
            None,
            reqwest::Client::new(),
            Arc::new(MomoConfig::default()),
        );
        let scope_id = "01900000-0000-7000-8000-000000000101";
        let operation_key = scoped_operation_key(scope_id, "request-1");
        assert!(!service.cancel(scope_id, "request-1"));
        let operation = service.enter_operation(&operation_key);
        assert!(!service.cancel("01900000-0000-7000-8000-000000000102", "request-1"));
        service
            .ensure_active(&operation_key)
            .expect("another scope cannot cancel this operation");
        assert!(service.cancel(scope_id, "request-1"));
        assert!(matches!(
            service.ensure_active(&operation_key),
            Err(MomoApiError {
                kind: MomoApiErrorKind::Cancelled,
                ..
            })
        ));
        drop(operation);
        assert!(!service.cancel(scope_id, "request-1"));
        service
            .ensure_active(&operation_key)
            .expect("dropped operations leave no stale cancellation marker");
    }

    #[tokio::test]
    async fn governed_vision_adapter_resolves_images_to_retryable_text() {
        let mut config = MomoConfig::default();
        config.vision.enabled = true;
        config.vision.prompt = "Describe visible facts.".to_owned();
        let service = MomoApiService::new(
            "http://127.0.0.1:9/v1",
            None,
            reqwest::Client::new(),
            Arc::new(config),
        )
        .with_vision_adapter(Arc::new(FixedVisionAdapter));
        let request: MomoResponseRequest = serde_json::from_value(json!({
            "input": [
                {"type": "input_text", "text": "What do you see?"},
                {"type": "input_image", "image_url": "https://example.test/image.png", "detail": "high"}
            ]
        }))
        .expect("request");
        let governed = service
            .config
            .govern(
                8_192,
                1_024,
                RequestedOverrides {
                    context_window: None,
                    max_output_tokens: None,
                    temperature: None,
                    instructions: None,
                    visual_description_prompt: None,
                    parameters: &serde_json::Map::new(),
                    tool_configuration_requested: false,
                },
            )
            .expect("governance");
        let operation = service.enter_operation("request-vision-1");
        let resolved = service
            .resolve_response_input(&request, "request-vision-1", None, &governed, false)
            .await
            .expect("resolved input");
        drop(operation);
        assert_eq!(
            resolved.image_handling,
            ImageInputHandling::DescriptionFallback
        );
        assert_eq!(resolved.visual_input_count, 1);
        assert_eq!(resolved.vision_usage.total_tokens, 20);
        assert_eq!(
            resolved.text,
            "What do you see?\n[Visual description for image 1]\nA red umbrella on a wet street."
        );

        let persisted = json!({
            "resolved_input_json": serde_json::to_string(&resolved).expect("stored resolution")
        });
        let operation = service.enter_operation("request-vision-1");
        let replayed = service
            .resolve_response_input(
                &request,
                "request-vision-1",
                Some(&persisted),
                &governed,
                false,
            )
            .await
            .expect("replayed resolution");
        drop(operation);
        assert_eq!(replayed, resolved);
    }

    #[tokio::test]
    async fn multimodal_chat_uses_original_images_without_vision_prompt() {
        let mut config = MomoConfig::default();
        config.vision.enabled = true;
        config.vision.prompt = "This fallback prompt must not be used.".to_owned();
        let service = MomoApiService::new(
            "http://127.0.0.1:9/v1",
            None,
            reqwest::Client::new(),
            Arc::new(config),
        )
        .with_vision_adapter(Arc::new(UnexpectedVisionAdapter));
        let request: MomoResponseRequest = serde_json::from_value(json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "What do you think?"},
                    {"type": "input_image", "image_url": "https://example.test/image.png", "detail": "high"}
                ]
            }]
        }))
        .expect("request");
        let governed = service
            .config
            .govern(
                8_192,
                1_024,
                RequestedOverrides {
                    context_window: None,
                    max_output_tokens: None,
                    temperature: None,
                    instructions: None,
                    visual_description_prompt: Some("Also unused."),
                    parameters: &serde_json::Map::new(),
                    tool_configuration_requested: false,
                },
            )
            .expect("governance");
        let operation = service.enter_operation("request-direct-vision-1");
        let resolved = service
            .resolve_response_input(&request, "request-direct-vision-1", None, &governed, true)
            .await
            .expect("direct multimodal input");
        drop(operation);
        assert_eq!(
            resolved.image_handling,
            ImageInputHandling::DirectMultimodal
        );
        assert_eq!(resolved.text, "What do you think?\n[Image input: 1]");
        assert_eq!(resolved.vision_usage, ChatUsage::default());
        let messages = request.input.gateway_messages().expect("gateway messages");
        assert!(matches!(
            messages[0].content,
            Some(crate::GatewayMessageContent::Parts(ref parts))
                if parts.iter().any(|part| matches!(part, crate::GatewayContentPart::ImageUrl { .. }))
        ));
    }
}
