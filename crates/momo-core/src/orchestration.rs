//! Client-independent implementation of the native high-level MomoApi response pipeline.

pub(crate) mod coordination;
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

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use thiserror::Error;

use crate::{
    ChatUsage, DEFAULT_VISION_ROUTE, GatewayVisionAdapter, GovernedOverrides, MomoRuntime,
    MomoRuntimeSettings, VisionDescriptionAdapter,
};
#[cfg(test)]
use crate::{
    MoStateInjectionMode, MomoResponseRequest, PromptSpaceId, RequestedOverrides,
    VisionDescriptionRequest,
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
    fn bad_request(message: impl ToString) -> Self {
        Self {
            kind: MomoApiErrorKind::BadRequest,
            message: message.to_string(),
        }
    }

    fn model(message: impl ToString) -> Self {
        Self {
            kind: MomoApiErrorKind::Model,
            message: message.to_string(),
        }
    }

    fn conflict(message: impl ToString) -> Self {
        Self {
            kind: MomoApiErrorKind::Conflict,
            message: message.to_string(),
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: MomoApiErrorKind::Cancelled,
            message: "response request was cancelled".to_owned(),
        }
    }

    fn internal(message: impl ToString) -> Self {
        Self {
            kind: MomoApiErrorKind::Internal,
            message: message.to_string(),
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
    runtime: Arc<MomoRuntime>,
    gateway_origin: String,
    gateway_api_key: Option<String>,
    gateway_client: reqwest::Client,
    vision_adapter: Arc<dyn VisionDescriptionAdapter>,
    coordination: Arc<coordination::ResponseCoordination>,
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
    persisted: Option<&'a momo_storage::ResponseOperation>,
    resolved_input: &'a ResolvedResponseInput,
    governed: GovernedOverrides,
    warnings: Vec<String>,
    stream: Option<&'a dyn MomoResponseEventSink>,
}

impl MomoApiService {
    #[must_use]
    pub fn new(
        runtime: Arc<MomoRuntime>,
        gateway_origin: impl Into<String>,
        gateway_api_key: Option<String>,
        gateway_client: reqwest::Client,
    ) -> Self {
        let gateway_origin = gateway_origin.into();
        let vision_adapter = Arc::new(GatewayVisionAdapter::new(
            gateway_client.clone(),
            gateway_origin.clone(),
            gateway_api_key.clone(),
            DEFAULT_VISION_ROUTE,
        ));
        Self {
            coordination: runtime.response_coordination(),
            runtime,
            gateway_origin,
            gateway_api_key,
            gateway_client,
            vision_adapter,
        }
    }

    /// Returns the single runtime instance owned by this application service.
    #[must_use]
    pub(crate) fn runtime(&self) -> &MomoRuntime {
        &self.runtime
    }

    #[must_use]
    pub fn with_vision_adapter(mut self, adapter: Arc<dyn VisionDescriptionAdapter>) -> Self {
        self.vision_adapter = adapter;
        self
    }

    fn config_snapshot(&self) -> MomoRuntimeSettings {
        self.runtime.runtime_settings()
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
}

#[cfg(test)]
#[path = "../tests/unit/orchestration.rs"]
mod tests;
