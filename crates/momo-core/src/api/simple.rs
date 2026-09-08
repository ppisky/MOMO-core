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

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::json;
use tokio::sync::Notify;
use tokio::sync::OnceCell;

use crate::{
    CapabilityDiscoveryDocument, CapabilityRegistry, ChatInput, ChatParameters, ContextBudget,
    ContextRequest, ContextSections, EmbeddingEndpoint, EmbeddingError, EmbeddingInput,
    EmbeddingProfile, EmbeddingProvider, EmbeddingPurpose, GatewayError, MAX_DISCOVERY_TTL_SECONDS,
    MAX_EMBEDDING_BATCH_SIZE, MomoCore, OpenAiEmbeddingProvider, OpenAiGateway, ProviderEndpoint,
    fetch_capability_document, prepare_context,
};
use momo_storage::{DEFAULT_NSG_VECTOR_TOP_K, NsgVectorStore};

struct CancellationSignal {
    cancelled: AtomicBool,
    notify: Notify,
}

static CANCELLATIONS: LazyLock<Mutex<HashMap<String, Arc<CancellationSignal>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CORE: OnceCell<MomoCore> = OnceCell::const_new();
static CAPABILITIES: LazyLock<tokio::sync::RwLock<CapabilityRegistry>> =
    LazyLock::new(|| tokio::sync::RwLock::new(CapabilityRegistry::default()));
static MEMORY_PATCH_REVIEW_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static CONTROL_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static MO_STATE_SPACE_LOCKS: LazyLock<
    tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));
const MAX_IMPORTED_FONT_BYTES: usize = 64 * 1024 * 1024;

pub(crate) async fn lock_mo_state_space(space_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = MO_STATE_SPACE_LOCKS.lock().await;
        if locks.len() >= 1_024 {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        if let Some(lock) = locks.get(space_id).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(space_id.to_owned(), Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}

pub(crate) trait ChatEventSink: Send + Sync {
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

pub(crate) fn cancel_chat(request_id: String) -> bool {
    let cancellations = CANCELLATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(cancelled) = cancellations.get(&request_id) else {
        return false;
    };
    cancelled.cancelled.store(true, Ordering::Release);
    cancelled.notify.notify_one();
    true
}

fn core() -> Result<&'static MomoCore, String> {
    CORE.get()
        .ok_or_else(|| "MOMO Core has not been initialized".to_owned())
}

async fn approve_memory_patch_review(
    scope_id: uuid::Uuid,
    review_id: uuid::Uuid,
) -> Result<String, String> {
    let _guard = MEMORY_PATCH_REVIEW_LOCK.lock().await;
    let _state_guard = lock_mo_state_space(&scope_id.to_string()).await;
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
        .memory_for_space(scope_id)
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
