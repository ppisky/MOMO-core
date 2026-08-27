//! Client-independent orchestration and OpenAI-compatible model access.

pub mod api;
mod capability;
mod character_compat;
mod context;
mod embedding;
mod gateway;
mod governance;
mod lsb;
mod orchestration;
mod portable;
mod response;

use std::path::{Path, PathBuf};

use momo_memory::MemoryWorkspace;
use momo_storage::{LocalStore, StorageError, TursoVectorStore};
use thiserror::Error;

pub use capability::{
    CapabilityDiscoveryDocument, CapabilityError, CapabilityProfile, CapabilityRegistry,
    CapabilitySource, MAX_DISCOVERY_TTL_SECONDS, ResolvedCapability, TokenizerProfile,
    fetch_capability_document,
};
pub use character_compat::{
    CharacterCompatError, ExternalCharacterExportFormat, ExternalCharacterImport,
    ExternalCharacterImportFormat, PreservedCharacterSourceExport, export_external_character,
    export_preserved_character_source, import_external_character, validate_external_charx,
};
pub use context::{
    ContextBudget, ContextRequest, ContextSections, PreparedContext, estimate_text_tokens,
    prepare_context, prepare_context_with_tokenizer,
};
pub use embedding::{
    EmbeddingBatch, EmbeddingEndpoint, EmbeddingError, EmbeddingInput, EmbeddingNormalization,
    EmbeddingProfile, EmbeddingProvider, EmbeddingPurpose, EmbeddingUsage, EmbeddingVector,
    MAX_EMBEDDING_BATCH_SIZE, MAX_EMBEDDING_DIMENSION, OpenAiEmbeddingProvider,
};
pub use gateway::{
    ChatCompletion, ChatFunctionCall, ChatInput, ChatParameters, ChatStreamDelta,
    ChatStreamFunctionCallDelta, ChatStreamToolCallDelta, ChatToolCall, ChatUsage, GatewayError,
    GatewayMessage, GatewayMessageRole, OpenAiGateway, ProviderEndpoint, SseDecoder,
};
pub use governance::{
    GovernanceError, GovernedOverrides, MOMO_CONFIG_SCHEMA_VERSION, MomoConfig, OverrideMode,
    RequestOverridePolicy, RequestedOverrides, VisionDescriptionConfig, validate_momo_document,
};
pub use lsb::{
    LSB_CARRIER_MAGIC, LSB_CARRIER_VERSION, LSB_HEADER_BYTES, LsbCarrierError, LsbCarrierInfo,
    LsbImageFormat, LsbPayload, LsbPayloadType, MAX_LSB_IMAGE_BYTES, MAX_LSB_IMAGE_PIXELS,
    MAX_LSB_PAYLOAD_BYTES, embed_lsb_carrier, embed_lsb_image, embed_lsb_png, embed_lsb_webp,
    extract_lsb_carrier, extract_lsb_image, extract_lsb_png, extract_lsb_webp, lsb_capacity,
};
pub use momo_config;
pub use momo_crypto;
pub use momo_domain;
pub use momo_memory;
pub use momo_memory::{MoStateAudit, MoStateContext};
pub use momo_moc;
pub use momo_storage;
pub use momo_storage::{DEFAULT_NSG_VECTOR_TOP_K, MAX_NSG_VECTOR_TOP_K, NsgVectorStatus};
pub use orchestration::{MomoApiError, MomoApiErrorKind, MomoApiService, MomoResponseEventSink};
pub use portable::{
    ConflictMode, ImportReport, MocCompatibility, MocExportPlan, MocModule, MocProtection,
    PortableError, export_moc, export_momo_config, export_private_moc, import_moc,
    import_moc_with_passphrase, import_momo_config, moc_is_encrypted,
};
pub use response::{
    MAX_GATEWAY_HOPS, MAX_RESPONSE_ID_BYTES, MAX_RESPONSE_IMAGE_REFERENCE_BYTES,
    MAX_RESPONSE_INPUT_BYTES, MAX_RESPONSE_INSTRUCTIONS_BYTES, MAX_RESPONSE_REQUEST_BYTES,
    MAX_RESPONSE_SSE_EVENT_BYTES, MAX_RESPONSE_STREAM_BYTES, MAX_RESPONSE_TOOL_ARGUMENT_BYTES,
    MAX_RESPONSE_TOOL_OUTPUT_BYTES, MAX_RESPONSE_TOOL_SCHEMA_BYTES, MAX_RESPONSE_TOOLS,
    MOMO_RESPONSE_SCHEMA, MomoResponse, MomoResponseExtension, MomoResponseMetadata,
    MomoResponseRequest, ResponseContentBlock, ResponseContractError, ResponseError, ResponseInput,
    ResponseInputItem, ResponseMessageContent, ResponseOutputContent, ResponseOutputItem,
    ResponseTool, ResponseUsage,
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
    vector_store: TursoVectorStore,
}

impl MomoCore {
    pub async fn initialize(data_dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).map_err(momo_memory::MemoryError::from)?;
        let store = LocalStore::open(data_dir.join("momo.sqlite3")).await?;
        let vector_store = TursoVectorStore::open(data_dir.join("nsg-vectors.db")).await?;
        migrate_scope_directory(data_dir)?;
        std::fs::create_dir_all(data_dir.join("memory/scopes"))
            .map_err(momo_memory::MemoryError::from)?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            store,
            vector_store,
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
    pub const fn vector_store(&self) -> &TursoVectorStore {
        &self.vector_store
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
        assert!(core.data_dir().join("nsg-vectors.db").exists());
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
