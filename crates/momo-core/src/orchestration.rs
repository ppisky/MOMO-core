//! Client-independent implementation of the native high-level MomoApi response pipeline.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as SyncMutex, Weak},
    time::Duration,
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};

use crate::{
    ChatUsage, DEFAULT_VISION_ROUTE, EmbeddingNormalization, EmbeddingProfile,
    GatewayVisionAdapter, GovernedOverrides, MoStateInjectionMode, MoStateProfile, MomoConfig,
    MomoResponse, MomoResponseMetadata, MomoResponseRequest, RequestedOverrides,
    ResponseOutputContent, ResponseOutputItem, ResponseUsage, VisionDescriptionAdapter,
    VisionDescriptionRequest, api::simple,
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

fn maintenance_batch_key(scope_id: &str, kind: &str, request_ids: &[String]) -> String {
    let mut hasher = Sha256::new();
    for value in std::iter::once(scope_id)
        .chain(std::iter::once(kind))
        .chain(request_ids.iter().map(String::as_str))
    {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    format!("maintenance:{}", hex::encode(hasher.finalize()))
}

fn opaque_hyphenated_identifiers(value: &str) -> std::collections::HashSet<&str> {
    value
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        })
        .filter(|token| {
            token.len() >= 8
                && token.matches('-').count() >= 2
                && token
                    .chars()
                    .any(|character| character.is_ascii_alphabetic())
                && token.chars().any(|character| character.is_ascii_digit())
        })
        .collect()
}

fn validate_generated_opaque_identifiers(source: &str, generated: &str) -> Result<(), String> {
    let allowed = opaque_hyphenated_identifiers(source);
    if let Some(identifier) = opaque_hyphenated_identifiers(generated)
        .into_iter()
        .find(|identifier| !allowed.contains(identifier))
    {
        return Err(format!(
            "generated patch altered or invented opaque identifier {identifier:?}; copy identifiers from evidence character-for-character"
        ));
    }
    Ok(())
}

fn memory_patch_is_noop(patch: &str) -> bool {
    patch.split_whitespace().collect::<String>() == "patches:[]"
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
    generation_gates: Arc<Mutex<HashMap<String, Weak<Semaphore>>>>,
    maintenance_tasks: Arc<SyncMutex<Vec<tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug, Default)]
struct OperationState {
    active: usize,
    cancelled: bool,
    scope_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ResolvedResponseInput {
    text: String,
    /// User-authored text to append to conversation history. Tool-only
    /// continuations intentionally leave this empty so tool JSON is not later
    /// replayed to the model as a forged user message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    persisted_user_text: Option<String>,
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
            generation_gates: Arc::new(Mutex::new(HashMap::new())),
            maintenance_tasks: Arc::new(SyncMutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn with_vision_adapter(mut self, adapter: Arc<dyn VisionDescriptionAdapter>) -> Self {
        self.vision_adapter = adapter;
        self
    }

    /// Maximum number of pending turns supplied to one maintenance-model call.
    /// Explicit drains use the same configured batch boundary as background
    /// maintenance instead of collapsing an entire backlog into one prompt.
    #[must_use]
    pub fn maintenance_batch_limit(&self, kind: MaintenanceKind) -> usize {
        match kind {
            MaintenanceKind::Memory => self.config.runtime.memory_distill_every_turns,
            MaintenanceKind::SemanticGraph => self.config.runtime.nsg_govern_every_turns,
        }
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
        let autonomous_mo_state = request.momo.mo_state
            && self.config.mo_state.profile == MoStateProfile::ClosedAutonomous;
        let managed_space_id = request
            .momo
            .memory_write_space_id
            .as_deref()
            .unwrap_or_else(|| {
                request
                    .momo
                    .personal_space_id
                    .as_deref()
                    .expect("validated personal Space")
            });
        if autonomous_mo_state {
            // Recover pending work without serializing the next user response
            // behind one or more maintenance-model calls. The configured batch
            // sizes remain authoritative; explicit drains provide a strict
            // consistency barrier when a caller needs one.
            self.schedule_maintenance(managed_space_id.to_owned());
        }
        let _state_guard = if autonomous_mo_state {
            Some(simple::lock_mo_state_space(managed_space_id).await)
        } else {
            None
        };
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
                "user_content": &resolved_input.text,
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

    fn enter_operation(&self, request_id: &str, scope_id: &str) -> OperationGuard {
        let mut states = self
            .operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = states.entry(request_id.to_owned()).or_default();
        state.active += 1;
        state.scope_id = scope_id.to_owned();
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
            let task = tokio::spawn(async move {
                // Give a follow-up foreground request a short window to claim
                // the Space. During a burst, defer maintenance until the user
                // pauses or an explicit drain establishes a barrier.
                tokio::time::sleep(Duration::from_millis(100)).await;
                if service.has_active_responses(&scope_id) {
                    return;
                }
                if let Err(error) = service.maintain(&scope_id, kind, threshold).await {
                    tracing::warn!(?kind, %error, "background response maintenance failed; turns remain pending");
                }
            });
            let mut tasks = self
                .maintenance_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.retain(|task| !task.is_finished());
            tasks.push(task);
        }
    }

    fn has_active_responses(&self, scope_id: &str) -> bool {
        self.operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|state| state.active > 0 && state.scope_id == scope_id)
    }

    async fn generation_gate(&self, scope_id: &str, lane: &str) -> Arc<Semaphore> {
        let mut gates = self.generation_gates.lock().await;
        let gate_key = format!("{lane}:{scope_id}");
        if gates.len() >= 1_024 {
            gates.retain(|_, gate| gate.strong_count() > 0);
        }
        if let Some(gate) = gates.get(&gate_key).and_then(Weak::upgrade) {
            gate
        } else {
            // Reserve one independent slot for foreground generation and one
            // for maintenance. Slow maintenance must not hold the foreground
            // slot; the two maintenance routes share their single slot.
            let gate = Arc::new(Semaphore::new(1));
            gates.insert(gate_key, Arc::downgrade(&gate));
            gate
        }
    }

    /// Waits until all background maintenance spawned by this service has
    /// finished. Transports should call this after they stop accepting work so
    /// an ordinary graceful shutdown does not strand an in-flight patch.
    pub async fn wait_for_maintenance(&self) {
        loop {
            let tasks = {
                let mut tasks = self
                    .maintenance_tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *tasks)
            };
            if tasks.is_empty() {
                return;
            }
            for task in tasks {
                if let Err(error) = task.await {
                    tracing::warn!(%error, "background maintenance task did not shut down cleanly");
                }
            }
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
        let request_ids = turns
            .iter()
            .filter_map(|turn| turn.get("request_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let batch_key = maintenance_batch_key(scope_id, storage_kind, &request_ids);
        let mut repair_error = None;
        let mut staged = simple::maintenance_batch_patch(batch_key.clone())
            .await
            .map_err(MomoApiError::internal)?;
        if kind == MaintenanceKind::Memory
            && let Some(patch) = staged.clone()
            && let Err(error) = simple::validate_memory_patch_json(scope_id.to_owned(), patch).await
        {
            if !simple::discard_maintenance_batch(batch_key.clone())
                .await
                .map_err(MomoApiError::internal)?
            {
                return Err(MomoApiError::internal(
                    "invalid staged maintenance batch disappeared before regeneration",
                ));
            }
            tracing::warn!(%batch_key, %error, "discarded invalid staged memory patch");
            repair_error = Some(error);
            staged = None;
        }
        let patch = if let Some(staged) = staged {
            staged
        } else {
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
                "mo_state_profile": self.config.mo_state.profile.as_str(),
                "scene_management": self.config.mo_state.scene_management,
                "current_unix_timestamp": Utc::now().timestamp(),
                "existing_context": existing_context,
                "pending_turns": turns,
            });
            let maintenance_source = maintenance_input.to_string();
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
            let maintenance_max_tokens = self
                .gateway_generation_capability(route)
                .await
                // Maintenance output is a compact patch, and allowing a model
                // to fill a large route limit turns pathological generations
                // into multi-minute drain failures. Invalid or truncated DMW
                // patches still use the bounded repair pass below.
                .map(|capability| capability.max_output_tokens.min(1_024))
                .unwrap_or(1_024);
            let generated_patch = loop {
                let user_content = repair_error.as_ref().map_or_else(
                    || maintenance_input.to_string(),
                    |error| {
                        json!({
                            "maintenance_input": &maintenance_input,
                            "previous_output_error": error,
                            "required_correction": "Regenerate the complete patch. Obey every field and target constraint; do not repeat the invalid output. For create operations, frontmatter type must be exactly character under characters/, relationship under relationships/, event under events/, or world under world/."
                        })
                        .to_string()
                    },
                );
                let generation_gate = self.generation_gate(scope_id, "maintenance").await;
                let generation_permit = generation_gate
                    .acquire_owned()
                    .await
                    .map_err(|_| MomoApiError::internal("generation gate closed"))?;
                let completion = simple::chat_complete_json(
                    json!({
                        "base_url": self.gateway_origin,
                        "api_key": self.gateway_api_key,
                        "model": route,
                        "messages": [
                            {"role": "system", "content": system},
                            {"role": "user", "content": user_content}
                        ],
                        "request_parameters": {"max_tokens": maintenance_max_tokens, "momo_hop": 1}
                    })
                    .to_string(),
                )
                .await
                .map_err(MomoApiError::model)?;
                // Validation and application may next need the MO State write
                // lock. Release the model slot first so a foreground response
                // that already owns that write lock cannot deadlock behind us.
                drop(generation_permit);
                let completion: Value = serde_json::from_str(&completion)
                    .map_err(|error| MomoApiError::model(error.to_string()))?;
                let generated = completion
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| MomoApiError::model("maintenance model returned no text"))?
                    .to_owned();
                if kind != MaintenanceKind::Memory {
                    break generated;
                }
                let validation =
                    match validate_generated_opaque_identifiers(&maintenance_source, &generated) {
                        Ok(()) => {
                            simple::validate_memory_patch_json(
                                scope_id.to_owned(),
                                generated.clone(),
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    };
                match validation {
                    Ok(_) if memory_patch_is_noop(&generated) && repair_error.is_none() => {
                        repair_error = Some(
                            "The first pass returned an empty patch for a non-empty maintenance batch. Recheck every pending turn for explicit durable facts, corrections, commitments, boundaries, and activated outcomes. Return patches: [] again only if none are supported."
                                .to_owned(),
                        );
                    }
                    Ok(_) => break generated,
                    Err(error) if repair_error.is_none() => repair_error = Some(error),
                    Err(error) => {
                        return Err(MomoApiError::model(format!(
                            "memory maintenance model returned an invalid patch after repair: {error}"
                        )));
                    }
                }
            };
            simple::stage_maintenance_batch_json(
                json!({
                    "batch_key": batch_key,
                    "scope_id": scope_id,
                    "kind": storage_kind,
                    "request_ids": request_ids,
                    "patch_yaml": generated_patch,
                })
                .to_string(),
            )
            .await
            .map_err(MomoApiError::internal)?
        };
        match kind {
            MaintenanceKind::Memory => {
                simple::apply_memory_patch_json(scope_id.to_owned(), patch).await
            }
            MaintenanceKind::SemanticGraph => {
                simple::apply_nsg_patch_json(scope_id.to_owned(), patch, false).await
            }
        }
        .map_err(MomoApiError::internal)?;
        simple::complete_maintenance_batch(batch_key, request_ids, storage_kind.to_owned())
            .await
            .map_err(MomoApiError::internal)?;
        Ok(true)
    }

    async fn respond(
        &self,
        request: &MomoResponseRequest,
        attempt: GovernedResponseAttempt<'_>,
    ) -> Result<(MomoResponse, Option<String>), MomoApiError> {
        let GovernedResponseAttempt {
            request_id,
            operation_key,
            request_fingerprint,
            persisted,
            resolved_input,
            mut governed,
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
            let persisted_user_text = resolved_input
                .persisted_user_text
                .as_deref()
                .or_else(|| (!request.input.has_function_outputs()).then_some(input));
            if let Some(user_text) = persisted_user_text {
                simple::append_response_user_message_json(
                    operation_key.to_owned(),
                    conversation_space_id.clone(),
                    conversation_id.clone(),
                    user_text.to_owned(),
                )
                .await
                .map_err(MomoApiError::internal)?;
            } else {
                simple::mark_response_user_written(operation_key.to_owned())
                    .await
                    .map_err(MomoApiError::internal)?;
            }
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
        let (retrieved, retrieval_status) = if retrieval_enabled {
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
                Ok(value) => (
                    serde_json::from_str::<Value>(&value)
                        .map_err(|error| MomoApiError::internal(error.to_string()))?,
                    "ok",
                ),
                Err(error) => {
                    warnings.push(format!("retrieval degraded: {error}"));
                    (json!([]), "degraded")
                }
            }
        } else {
            (json!([]), "disabled")
        };
        let items = retrieved
            .as_array()
            .cloned()
            .ok_or_else(|| MomoApiError::internal("retrieval result must be an array"))?;
        governed.audit["memory_retrieval"] =
            retrieval_audit(&items, retrieval_enabled, retrieval_status);
        let (nsg, memory): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|item| item.get("graph_id").is_some());
        let state_input_audit = state_input_audit(&memory, &nsg, retrieval_status)?;
        let managed_space_id = request
            .momo
            .memory_write_space_id
            .clone()
            .unwrap_or_else(|| personal_space_id.clone());
        let autonomous_mo_state = request.momo.mo_state
            && self.config.mo_state.profile == MoStateProfile::ClosedAutonomous;
        let mut mo_state_operation_id = None;
        let mut state_result = if request.momo.mo_state {
            let persisted_snapshot = if autonomous_mo_state {
                match simple::observe_mo_state_runtime_json(
                    operation_key.to_owned(),
                    managed_space_id.clone(),
                    if request.input.has_function_outputs() {
                        "tool_result".to_owned()
                    } else {
                        "user_message".to_owned()
                    },
                    request_fingerprint.to_owned(),
                    self.config.mo_state.profile.as_str().to_owned(),
                )
                .await
                {
                    Ok(operation_json) => {
                        let operation: momo_storage::MoStateOperation =
                            serde_json::from_str(&operation_json)
                                .map_err(|error| MomoApiError::internal(error.to_string()))?;
                        mo_state_operation_id = Some(operation.operation_id);
                        operation.snapshot_json
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
                let (compiled, degraded, compile_error) = match simple::compile_mo_state_json(
                    managed_space_id.clone(),
                    serde_json::to_string(&memory)
                        .map_err(|error| MomoApiError::internal(error.to_string()))?,
                    serde_json::to_string(&nsg)
                        .map_err(|error| MomoApiError::internal(error.to_string()))?,
                    context_window,
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
                if let Some(state_operation_id) = mo_state_operation_id.as_ref() {
                    match simple::publish_mo_state_snapshot_json(
                        state_operation_id.clone(),
                        serde_json::to_string(&compiled)
                            .map_err(|error| MomoApiError::internal(error.to_string()))?,
                        degraded,
                        compile_error.clone(),
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
                            mo_state_operation_id = None;
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
        if let Some(audit) = state_result.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("input".to_owned(), state_input_audit);
            audit.insert(
                "manager".to_owned(),
                json!({
                    "enabled": autonomous_mo_state,
                    "profile": self.config.mo_state.profile.as_str(),
                    "scene_management": self.config.mo_state.scene_management,
                    "max_reconcile_steps": self.config.mo_state.max_reconcile_steps,
                    "max_agent_steps": self.config.mo_state.max_agent_steps,
                    "operation_timeout_ms": self.config.mo_state.operation_timeout_ms,
                    "injection_mode": self.config.mo_state.injection_mode.as_str(),
                }),
            );
        }
        let (state_context_for_prompt, state_injection_status) = state_context_for_prompt(
            &state_result,
            request.momo.mo_state,
            self.config.mo_state.injection_mode,
        );
        if let Some(audit) = state_result.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("injection_status".to_owned(), json!(state_injection_status));
        }

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
        let prepared = parse_json(simple::prepare_context_json(
            json!({
                "runtime_instructions": governed.instructions.as_deref().unwrap_or_default(),
                "roleplay_director": if self.config.roleplay.enabled
                    && character.get("character_markdown").and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                {
                    self.config.prompts.roleplay_director.as_str()
                } else {
                    ""
                },
                "character_markdown": character.get("character_markdown").and_then(Value::as_str).unwrap_or_default(),
                "user_markdown": character.get("user_markdown").and_then(Value::as_str).unwrap_or_default(),
                "memory_markdown": if include_memory { joined_bodies(&memory) } else { String::new() },
                "state_context": state_context_for_prompt,
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
        governed.audit["text_context"] = json!({
            "estimated_input_tokens": prepared.get("estimated_input_tokens"),
            "omitted_messages": prepared.get("omitted_messages"),
            "truncated_messages": prepared.get("truncated_messages"),
            "sections": prepared.get("section_audit"),
            "excludes_appended_structured_input": request.input.is_structured()
                && (direct_multimodal || !request.input.has_images()),
        });
        if prepared
            .get("section_audit")
            .and_then(Value::as_array)
            .is_some_and(|sections| {
                sections
                    .iter()
                    .any(|section| section.get("truncated").and_then(Value::as_bool) == Some(true))
            })
        {
            warnings.push("system context sections were truncated; inspect request_audit.text_context.sections".to_owned());
        }
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
        // A background model request may take minutes. It gets its own bounded
        // lane so normal foreground generation can proceed using its snapshot.
        // Provider-wide limits remain the gateway's responsibility.
        let generation_gate = self.generation_gate(&personal_space_id, "foreground").await;
        let generation_permit = generation_gate
            .acquire_owned()
            .await
            .map_err(|_| MomoApiError::internal("generation gate closed"))?;
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
        drop(generation_permit);
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
        Ok((
            MomoResponse {
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
            },
            mo_state_operation_id,
        ))
    }

    async fn gateway_response_budget(&self) -> Result<GatewayGenerationCapability, String> {
        self.gateway_generation_capability("conversation").await
    }

    async fn gateway_generation_capability(
        &self,
        alias: &str,
    ) -> Result<GatewayGenerationCapability, String> {
        let discovery = self.gateway_model_detail(alias).await?;
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
            return Err(format!(
                "gateway returned an invalid token budget for route {alias}"
            ));
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

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, MomoApiError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| MomoApiError::internal(format!("response is missing string field {field}")))
}

fn joined_bodies(values: &[Value]) -> String {
    values
        .iter()
        .filter_map(|value| {
            let body = value.get("body").and_then(Value::as_str)?.trim();
            if body.is_empty() {
                return None;
            }
            let source = value.get("memory_space");
            let label = source
                .and_then(|source| source.get("label"))
                .and_then(Value::as_str);
            let record_id = value
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("graph_id").and_then(Value::as_str));
            Some(match (label, record_id) {
                (Some(label), Some(record_id)) => format!(
                    "[Memory record: {} | source: {}]\n{body}",
                    serde_json::to_string(record_id).expect("string serializes"),
                    serde_json::to_string(label).expect("string serializes")
                ),
                (None, Some(record_id)) => format!(
                    "[Memory record: {}]\n{body}",
                    serde_json::to_string(record_id).expect("string serializes")
                ),
                _ => body.to_owned(),
            })
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn retrieval_audit(items: &[Value], enabled: bool, status: &str) -> Value {
    let entries = items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("graph_id").and_then(Value::as_str))?;
            Some(json!({
                "id": id,
                "kind": if item.get("graph_id").is_some() { "nsg" } else { "dmw" },
                "space_id": item.pointer("/memory_space/id").and_then(Value::as_str),
                "space_label": item.pointer("/memory_space/label").and_then(Value::as_str),
                "estimated_tokens": item.get("estimated_tokens").and_then(Value::as_u64),
            }))
        })
        .collect::<Vec<_>>();
    json!({"enabled": enabled, "status": status, "count": entries.len(), "entries": entries})
}

fn state_input_audit(
    memory: &[Value],
    nsg: &[Value],
    retrieval_status: &str,
) -> Result<Value, MomoApiError> {
    let encoded = serde_json::to_vec(&json!({"memory": memory, "nsg": nsg}))
        .map_err(|error| MomoApiError::internal(error.to_string()))?;
    let source_spaces = memory
        .iter()
        .chain(nsg)
        .filter_map(|item| item.pointer("/memory_space/id").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    Ok(json!({
        "scope": "retrieved_subset",
        "retrieval_status": retrieval_status,
        "fingerprint_sha256": hex::encode(Sha256::digest(encoded)),
        "memory_count": memory.len(),
        "nsg_count": nsg.len(),
        "source_space_ids": source_spaces,
    }))
}

fn state_context_for_prompt(
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

    #[test]
    fn maintenance_rejects_mutated_opaque_identifiers() {
        let source = r#"Meet at Glass-Archive-f41e9 with Cobalt-Key-dae44."#;
        assert!(validate_generated_opaque_identifiers(source, source).is_ok());
        assert!(
            validate_generated_opaque_identifiers(
                source,
                "Store it at Glass-Archive-f41e with Cobalt-Key-dae44."
            )
            .is_err()
        );
    }

    #[test]
    fn maintenance_noop_detection_accepts_only_an_empty_patch() {
        assert!(memory_patch_is_noop("patches: []"));
        assert!(memory_patch_is_noop("patches:\n  []\n"));
        assert!(!memory_patch_is_noop(
            "patches:\n  - target_file: events/example.md"
        ));
    }

    #[test]
    fn mo_state_shadow_and_degraded_results_never_enter_the_prompt() {
        let healthy = json!({"context": "[STATE_CONTEXT]", "audit": {"degraded": false}});
        assert_eq!(
            state_context_for_prompt(&healthy, true, MoStateInjectionMode::Active),
            ("[STATE_CONTEXT]".to_owned(), "active")
        );
        assert_eq!(
            state_context_for_prompt(&healthy, true, MoStateInjectionMode::Shadow),
            (String::new(), "shadow")
        );
        let degraded = json!({"context": "unsafe", "audit": {"degraded": true}});
        assert_eq!(
            state_context_for_prompt(&degraded, true, MoStateInjectionMode::Active),
            (String::new(), "suppressed_degraded")
        );
    }

    #[test]
    fn retrieval_audit_exposes_ids_and_sources_without_memory_bodies() {
        let audit = retrieval_audit(
            &[json!({"id": "memory-1", "body": "private text",
            "estimated_tokens": 12, "memory_space": {"id": "space-1", "label": "personal"}})],
            true,
            "ok",
        );
        assert_eq!(audit["entries"][0]["id"], "memory-1");
        assert_eq!(audit["entries"][0]["space_id"], "space-1");
        assert_eq!(audit["status"], "ok");
        assert!(!audit.to_string().contains("private text"));
    }

    #[test]
    fn mo_state_input_audit_binds_projection_without_exposing_bodies() {
        let memory = [json!({"id": "m1", "body": "secret body",
            "memory_space": {"id": "s1"}})];
        let audit = state_input_audit(&memory, &[], "ok").expect("audit");
        assert_eq!(audit["scope"], "retrieved_subset");
        assert_eq!(audit["memory_count"], 1);
        assert_eq!(audit["source_space_ids"], json!(["s1"]));
        assert_eq!(audit["fingerprint_sha256"].as_str().unwrap().len(), 64);
        assert!(!audit.to_string().contains("secret body"));

        let degraded = json!({"context": "unsafe", "audit": {
            "degraded": false, "input": {"retrieval_status": "degraded"}}});
        assert_eq!(
            state_context_for_prompt(&degraded, true, MoStateInjectionMode::Active),
            (String::new(), "suppressed_degraded_input")
        );
    }

    #[tokio::test]
    async fn slow_maintenance_does_not_occupy_foreground_generation_slot() {
        let service = MomoApiService::new(
            "http://127.0.0.1:9/v1",
            None,
            reqwest::Client::new(),
            Arc::new(MomoConfig::default()),
        );
        let maintenance = service.generation_gate("space", "maintenance").await;
        let first = maintenance
            .clone()
            .acquire_owned()
            .await
            .expect("maintenance slot");
        assert!(
            service
                .generation_gate("space", "maintenance")
                .await
                .try_acquire_owned()
                .is_err()
        );
        let foreground = service.generation_gate("space", "foreground").await;
        let reply = foreground
            .clone()
            .try_acquire_owned()
            .expect("foreground must remain available");
        assert!(foreground.clone().try_acquire_owned().is_err());
        drop(first);
        assert!(maintenance.try_acquire_owned().is_ok());
        drop(reply);
        assert!(foreground.try_acquire_owned().is_ok());
    }

    #[test]
    fn roleplay_context_keeps_multi_space_memory_provenance() {
        let rendered = joined_bodies(&[
            json!({
                "id": "preference-rain",
                "body": "The user dislikes rain.",
                "memory_space": {"id": "space-personal", "label": "personal"}
            }),
            json!({
                "id": "scene-rain",
                "body": "It is raining in the shared scene.",
                "memory_space": {"id": "space-group", "label": "group"}
            }),
        ]);

        assert!(rendered.contains("[Memory record: \"preference-rain\" | source: \"personal\"]"));
        assert!(rendered.contains("[Memory record: \"scene-rain\" | source: \"group\"]"));
        assert!(!rendered.contains("space-personal"));
        assert!(!rendered.contains("space-group"));
    }

    #[test]
    fn explicit_drain_uses_configured_maintenance_batch_limits() {
        let mut config = MomoConfig::default();
        config.runtime.memory_distill_every_turns = 7;
        config.runtime.nsg_govern_every_turns = 11;
        let service = MomoApiService::new(
            "http://127.0.0.1:9/v1",
            None,
            reqwest::Client::new(),
            Arc::new(config),
        );
        assert_eq!(service.maintenance_batch_limit(MaintenanceKind::Memory), 7);
        assert_eq!(
            service.maintenance_batch_limit(MaintenanceKind::SemanticGraph),
            11
        );
    }
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
        let operation = service.enter_operation(&operation_key, scope_id);
        assert!(service.has_active_responses(scope_id));
        assert!(!service.has_active_responses("01900000-0000-7000-8000-000000000102"));
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
        assert!(!service.has_active_responses(scope_id));
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
        let operation = service.enter_operation("request-vision-1", "vision-space");
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
        let operation = service.enter_operation("request-vision-1", "vision-space");
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
        let operation = service.enter_operation("request-direct-vision-1", "direct-vision-space");
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
