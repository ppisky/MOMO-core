mod capabilities;
mod character_compat;
mod chat;
mod crypto;
mod fonts;
mod local_data;
mod memory;
mod nsg;
mod portable;
mod sync;

pub use capabilities::*;
pub use character_compat::*;
pub use chat::*;
pub use crypto::*;
pub use fonts::*;
pub use local_data::*;
pub use memory::*;
pub use nsg::*;
pub use portable::*;
pub use sync::*;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::json;
use tokio::sync::OnceCell;

use crate::{
    CapabilityDiscoveryDocument, CapabilityRegistry, ChatInput, ChatParameters, ContextBudget,
    ContextRequest, ContextSections, GatewayError, MAX_DISCOVERY_TTL_SECONDS, MomoCore,
    OpenAiGateway, ProviderEndpoint, fetch_capability_document, prepare_context,
};
use momo_storage::{DEFAULT_NSG_VECTOR_TOP_K, NsgVectorStore};

static CANCELLATIONS: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CORE: OnceCell<MomoCore> = OnceCell::const_new();
static CAPABILITIES: LazyLock<tokio::sync::RwLock<CapabilityRegistry>> =
    LazyLock::new(|| tokio::sync::RwLock::new(CapabilityRegistry::default()));
static MEMORY_PATCH_REVIEW_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
const MAX_IMPORTED_FONT_BYTES: usize = 64 * 1024 * 1024;

pub trait ChatEventSink: Send + Sync {
    fn add(&self, event_json: String) -> Result<(), String>;
}

impl<F> ChatEventSink for F
where
    F: Fn(String) -> Result<(), String> + Send + Sync,
{
    fn add(&self, event_json: String) -> Result<(), String> {
        self(event_json)
    }
}

pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

pub fn new_request_id() -> String {
    momo_domain::new_id().to_string()
}

pub fn cancel_chat(request_id: String) -> bool {
    let cancellations = CANCELLATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(cancelled) = cancellations.get(&request_id) else {
        return false;
    };
    cancelled.store(true, Ordering::Release);
    true
}

fn core() -> Result<&'static MomoCore, String> {
    CORE.get()
        .ok_or_else(|| "MOMO Core has not been initialized".to_owned())
}

async fn stage_memory_snapshot(scope_id: uuid::Uuid) -> Result<(), String> {
    let _ = scope_id;
    Ok(())
}

async fn stage_semantic_graph_snapshot(scope_id: uuid::Uuid) -> Result<(), String> {
    let _ = scope_id;
    Ok(())
}

async fn approve_memory_patch_review(
    scope_id: uuid::Uuid,
    review_id: uuid::Uuid,
) -> Result<String, String> {
    let _guard = MEMORY_PATCH_REVIEW_LOCK.lock().await;
    let existing = core()?
        .store()
        .memory_patch_review(scope_id, review_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "memory patch review was not found".to_owned())?;
    if existing.status != momo_storage::MemoryPatchReviewStatus::Pending {
        return serde_json::to_string(&existing).map_err(|error| error.to_string());
    }

    let workspace = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    if let Err(error) = workspace.apply_patch(&existing.patch_yaml) {
        let message = error.to_string();
        core()?
            .store()
            .resolve_memory_patch_review(
                scope_id,
                review_id,
                momo_storage::MemoryPatchReviewStatus::Failed,
                None,
                Some(&message),
            )
            .await
            .map_err(|storage_error| storage_error.to_string())?;
        return Err(message);
    }
    stage_memory_snapshot(scope_id).await?;
    let resolved = core()?
        .store()
        .resolve_memory_patch_review(
            scope_id,
            review_id,
            momo_storage::MemoryPatchReviewStatus::Approved,
            Some("ok"),
            None,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "memory patch review was already resolved".to_owned())?;
    serde_json::to_string(&resolved).map_err(|error| error.to_string())
}

fn find_entry_path(index_text: &str, document_id: &str) -> Result<String, String> {
    let index: yaml_serde::Value =
        yaml_serde::from_str(index_text).map_err(|error| error.to_string())?;
    let entries = index
        .get("entries")
        .and_then(|value| value.as_mapping())
        .ok_or_else(|| "invalid index structure".to_owned())?;
    let entry = entries
        .get(document_id)
        .and_then(|value| value.get("path"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| format!("document not found: {document_id}"))?;
    Ok(entry.to_owned())
}
