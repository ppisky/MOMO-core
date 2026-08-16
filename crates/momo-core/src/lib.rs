//! Client-independent orchestration and OpenAI-compatible model access.

pub mod api;
mod capability;
mod context;
mod gateway;
mod portable;

use std::path::{Path, PathBuf};

use momo_memory::MemoryWorkspace;
use momo_storage::{LocalStore, StorageError};
use thiserror::Error;

pub use capability::{
    CapabilityDiscoveryDocument, CapabilityError, CapabilityProfile, CapabilityRegistry,
    CapabilitySource, MAX_DISCOVERY_TTL_SECONDS, ResolvedCapability, TokenizerProfile,
    fetch_capability_document,
};
pub use context::{
    ContextBudget, ContextRequest, ContextSections, PreparedContext, estimate_text_tokens,
    prepare_context, prepare_context_with_tokenizer,
};
pub use gateway::{
    ChatCompletion, ChatInput, ChatParameters, ChatStreamDelta, GatewayError, OpenAiGateway,
    ProviderEndpoint, SseDecoder,
};
pub use momo_config;
pub use momo_crypto;
pub use momo_domain;
pub use momo_memory;
pub use momo_memory::{MoStateAudit, MoStateContext};
pub use momo_moc;
pub use momo_storage;
pub use momo_storage::{DEFAULT_NSG_VECTOR_TOP_K, MAX_NSG_VECTOR_TOP_K, NsgVectorStatus};
pub use portable::{
    ExportSelection, ImportReport, PortableError, export_moc, export_private_moc, import_moc,
    import_moc_with_passphrase, moc_is_encrypted,
};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("local storage initialization failed: {0}")]
    Storage(#[from] StorageError),
    #[error("memory initialization failed: {0}")]
    Memory(#[from] momo_memory::MemoryError),
}

#[derive(Debug, Clone)]
pub struct MomoCore {
    data_dir: PathBuf,
    store: LocalStore,
}

impl MomoCore {
    pub async fn initialize(data_dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).map_err(momo_memory::MemoryError::from)?;
        let store = LocalStore::open(data_dir.join("momo.sqlite3")).await?;
        migrate_scope_directory(data_dir)?;
        std::fs::create_dir_all(data_dir.join("memory/scopes"))
            .map_err(momo_memory::MemoryError::from)?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            store,
        })
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub const fn store(&self) -> &LocalStore {
        &self.store
    }

    #[must_use]
    pub const fn vector_store(&self) -> &LocalStore {
        &self.store
    }

    pub fn memory_for_scope(
        &self,
        scope_id: uuid::Uuid,
    ) -> Result<MemoryWorkspace, momo_memory::MemoryError> {
        MemoryWorkspace::initialize(
            self.data_dir
                .join("memory/scopes")
                .join(scope_id.to_string()),
        )
    }
}

fn migrate_scope_directory(data_dir: &Path) -> Result<(), CoreError> {
    let legacy = data_dir.join("memory/users");
    if !legacy.exists() {
        return Ok(());
    }
    let scopes = data_dir.join("memory/scopes");
    if scopes.exists() {
        return Err(momo_memory::MemoryError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "both memory/users and memory/scopes exist; merge them before starting Core",
        ))
        .into());
    }
    std::fs::rename(legacy, scopes).map_err(momo_memory::MemoryError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initializes_local_data_layout() {
        let directory = tempfile::tempdir().expect("data directory");
        let core = MomoCore::initialize(directory.path())
            .await
            .expect("initialize core");
        assert!(core.data_dir().join("momo.sqlite3").exists());
        let scope_id = momo_domain::new_id();
        core.memory_for_scope(scope_id).expect("memory");
        assert!(
            core.data_dir()
                .join("memory/scopes")
                .join(scope_id.to_string())
                .join("current/scene.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn converts_the_legacy_memory_directory_once() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let legacy = directory.path().join("memory/users/example");
        std::fs::create_dir_all(&legacy).expect("legacy directory");
        std::fs::write(legacy.join("marker"), "ok").expect("legacy marker");

        MomoCore::initialize(directory.path())
            .await
            .expect("initialize core");

        assert!(!directory.path().join("memory/users").exists());
        assert!(
            directory
                .path()
                .join("memory/scopes/example/marker")
                .exists()
        );
    }
}
