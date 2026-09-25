mod capabilities;
mod character_compat;
mod chat;
mod control;
mod embeddings;
mod fonts;
mod local_data;
mod lsb;
mod memory;
mod nsg;
mod portable;
mod state_runtime;

pub use capabilities::*;
pub use character_compat::*;
pub use chat::*;
pub use control::*;
pub use embeddings::*;
pub use fonts::*;
pub use local_data::*;
pub use lsb::*;
pub use memory::*;
pub use nsg::*;
pub use portable::*;
pub use state_runtime::*;

use std::sync::Arc;

use crate::{
    CapabilityDiscoveryDocument, ChatInput, ContextBudget, ContextRequest, ContextSections,
    EmbeddingEndpoint, EmbeddingError, EmbeddingInput, EmbeddingProfile, EmbeddingProvider,
    EmbeddingPurpose, MAX_DISCOVERY_TTL_SECONDS, MAX_EMBEDDING_BATCH_SIZE, MomoCore, MomoRuntime,
    OpenAiEmbeddingProvider, fetch_capability_document, prepare_context,
};
use momo_storage::{DEFAULT_NSG_VECTOR_TOP_K, NsgVectorStore};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeApiErrorKind {
    InvalidRequest,
    NotFound,
    Conflict,
    Upstream,
    Internal,
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct RuntimeApiError {
    pub kind: RuntimeApiErrorKind,
    pub message: String,
}

impl RuntimeApiError {
    fn recovery(error: crate::CoreError) -> Self {
        match error {
            crate::CoreError::Recovery { .. } => Self::conflict(error.to_string()),
            _ => Self::internal(error.to_string()),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeApiErrorKind::InvalidRequest,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeApiErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeApiErrorKind::Conflict,
            message: message.into(),
        }
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeApiErrorKind::Upstream,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeApiErrorKind::Internal,
            message: message.into(),
        }
    }
}

pub type RuntimeApiResult<T> = Result<T, RuntimeApiError>;

async fn run_blocking<T, F>(operation: &'static str, task: F) -> RuntimeApiResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> RuntimeApiResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(task).await.map_err(|error| {
        RuntimeApiError::internal(format!("{operation} blocking task failed: {error}"))
    })?
}

/// Keep the Space lock with the file task through caller cancellation, and
/// include the write in the runtime's graceful-shutdown drain.
async fn run_space_write<T, F>(
    runtime: &MomoRuntime,
    space_id: uuid::Uuid,
    operation: &'static str,
    task: F,
) -> RuntimeApiResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> RuntimeApiResult<T> + Send + 'static,
{
    let guard = runtime
        .lock_space(space_id)
        .await
        .map_err(RuntimeApiError::recovery)?;
    runtime
        .finish_commit(async move {
            run_blocking(operation, move || {
                let _guard = guard;
                task()
            })
            .await
        })
        .await
        .map_err(|error| {
            RuntimeApiError::internal(format!("{operation} commit task failed: {error}"))
        })?
}

#[cfg(test)]
#[path = "../../tests/unit/runtime_api.rs"]
mod tests;

const MAX_IMPORTED_FONT_BYTES: usize = 64 * 1024 * 1024;

pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

pub fn new_request_id() -> String {
    momo_domain::new_id().to_string()
}

async fn approve_memory_patch_review_inner(
    runtime: &MomoRuntime,
    scope_id: uuid::Uuid,
    review_id: uuid::Uuid,
) -> Result<momo_storage::MemoryPatchReview, RuntimeApiError> {
    let _guard = runtime
        .lock_memory_patch_reviews(&scope_id.to_string())
        .await;
    let _state_guard = runtime
        .lock_space(scope_id)
        .await
        .map_err(RuntimeApiError::recovery)?;
    let existing = runtime
        .core()
        .store()
        .memory_patch_review(scope_id, review_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .ok_or_else(|| RuntimeApiError::not_found("memory patch review was not found"))?;
    if existing.status != momo_storage::MemoryPatchReviewStatus::Pending {
        return Ok(existing);
    }

    let core = runtime.core_handle();
    runtime
        .finish_commit(async move {
            let _review_guard = _guard;
            let _space_guard = _state_guard;
            let pending = core
                .store()
                .prepared_memory_patch_review(scope_id, review_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
            let pending = if let Some(pending) = pending {
                pending
            } else {
                let patch = existing.patch_yaml.clone();
                let prepare_core = Arc::clone(&core);
                let plan = run_blocking("prepare reviewed patch", move || {
                    prepare_core
                        .memory_for_space(scope_id)
                        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                        .prepare_patch_commit(&patch)
                        .map_err(|error| RuntimeApiError::internal(error.to_string()))
                })
                .await;
                let plan = match plan {
                    Ok(plan) => plan,
                    Err(error) => {
                        core.store()
                            .resolve_memory_patch_review(
                                scope_id,
                                review_id,
                                momo_storage::MemoryPatchReviewStatus::Failed,
                                None,
                                Some(&error.to_string()),
                            )
                            .await
                            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
                        return Err(error);
                    }
                };
                let encoded = serde_json::to_string(&plan)
                    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
                core.mark_memory_commit_pending(scope_id);
                let encoded = core
                    .store()
                    .prepare_memory_patch_review_commit(scope_id, review_id, &encoded)
                    .await
                    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
                momo_storage::PendingMemoryPatchReview {
                    review: existing,
                    prepared_commit_json: encoded,
                }
            };
            core.commit_prepared_review(&pending)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))
        })
        .await
        .map_err(|error| RuntimeApiError::internal(format!("review commit task failed: {error}")))?
}

fn find_entry_path(index_text: &str, document_id: &str) -> Result<String, RuntimeApiError> {
    let index: yaml_serde::Value = yaml_serde::from_str(index_text)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let entries = index
        .get("entries")
        .and_then(|value| value.as_mapping())
        .ok_or_else(|| RuntimeApiError::internal("invalid index structure"))?;
    let entry = entries
        .get(document_id)
        .and_then(|value| value.get("path"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| RuntimeApiError::not_found(format!("document not found: {document_id}")))?;
    Ok(entry.to_owned())
}
