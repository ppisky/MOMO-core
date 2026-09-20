//! Client-independent implementation of the native high-level MomoApi response pipeline.

mod execution;
mod generation;
mod maintenance;
mod response;
mod state;

#[cfg(test)]
use execution::scoped_operation_key;
#[cfg(test)]
use maintenance::{
    maintenance_finish_error, memory_patch_is_noop, validate_generated_opaque_identifiers,
};
#[cfg(test)]
use response::{filter_memory_for_prompt, joined_bodies, retrieval_audit, state_input_audit};
#[cfg(test)]
use state::{compatible_ddm_bands, state_context_for_prompt};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as SyncMutex, Weak},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};

use crate::{
    ChatUsage, DEFAULT_VISION_ROUTE, GatewayVisionAdapter, GovernanceError, GovernedOverrides,
    MomoConfig, VisionDescriptionAdapter,
};
#[cfg(test)]
use crate::{
    MoStateInjectionMode, MomoResponseRequest, RequestedOverrides, VisionDescriptionRequest,
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
    config: Arc<SyncMutex<MomoConfig>>,
    vision_adapter: Arc<dyn VisionDescriptionAdapter>,
    response_attempts: Arc<Mutex<HashMap<String, String>>>,
    operation_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    conversation_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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

impl ResolvedResponseInput {
    fn maintenance_user_text(&self) -> &str {
        self.persisted_user_text.as_deref().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ImageInputHandling {
    #[default]
    None,
    DirectMultimodal,
    DescriptionFallback,
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
            config: Arc::new(SyncMutex::new((*config).clone())),
            vision_adapter,
            response_attempts: Arc::new(Mutex::new(HashMap::new())),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            conversation_locks: Arc::new(Mutex::new(HashMap::new())),
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

    /// Atomically replaces the portable runtime configuration used by future
    /// response and maintenance operations.
    pub fn update_config(&self, config: MomoConfig) -> Result<(), GovernanceError> {
        config.validate()?;
        *self
            .config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        Ok(())
    }

    fn config_snapshot(&self) -> MomoConfig {
        self.config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Maximum number of pending turns supplied to one maintenance-model call.
    /// Explicit drains use the same configured batch boundary as background
    /// maintenance instead of collapsing an entire backlog into one prompt.
    #[must_use]
    pub fn maintenance_batch_limit(&self, kind: MaintenanceKind) -> usize {
        let config = self.config_snapshot();
        match kind {
            MaintenanceKind::Memory => config.runtime.memory_distill_every_turns,
            MaintenanceKind::SemanticGraph => config.runtime.nsg_govern_every_turns,
        }
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
            let gate = Arc::new(Semaphore::new(1));
            gates.insert(gate_key, Arc::downgrade(&gate));
            gate
        }
    }

    async fn conversation_lock(&self, scope_id: &str, conversation_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.conversation_locks.lock().await;
        if locks.len() >= 1_024 {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        let key = format!("{scope_id}:{conversation_id}");
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/orchestration.rs"]
mod tests;
