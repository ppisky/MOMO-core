//! Client-independent MOMO orchestration and outbound model-adapter access.

pub mod api;
mod capability;
mod character_compat;
mod context;
mod control;
mod embedding;
mod gateway;
mod governance;
mod lsb;
mod orchestration;
mod portable;
mod response;
mod vision;

use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use fs2::FileExt;
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
    ContextBudget, ContextRequest, ContextSectionAudit, ContextSections, PreparedContext,
    estimate_text_tokens, prepare_context, prepare_context_with_tokenizer,
};
pub use control::{MOMO_CONTROL_SCHEMA, MomoControlAction, MomoControlRequest};
pub use embedding::{
    EmbeddingBatch, EmbeddingEndpoint, EmbeddingError, EmbeddingInput, EmbeddingNormalization,
    EmbeddingProfile, EmbeddingProvider, EmbeddingPurpose, EmbeddingUsage, EmbeddingVector,
    MAX_EMBEDDING_BATCH_SIZE, MAX_EMBEDDING_DIMENSION, OpenAiEmbeddingProvider,
};
pub use gateway::{
    ChatCompletion, ChatFunctionCall, ChatInput, ChatParameters, ChatStreamDelta,
    ChatStreamFunctionCallDelta, ChatStreamToolCallDelta, ChatToolCall, ChatUsage,
    GatewayContentPart, GatewayError, GatewayImageUrl, GatewayMessage, GatewayMessageContent,
    GatewayMessageRole, OpenAiGateway, ProviderEndpoint, SseDecoder,
};
pub use governance::{
    GovernanceError, GovernedOverrides, MOMO_CONFIG_SCHEMA_VERSION, MaintenancePromptConfig,
    MoStateInjectionMode, MoStateProfile, MoStateRuntimeConfig, MomoConfig, MomoRuntimeConfig,
    OverrideMode, RequestOverridePolicy, RequestedOverrides, RoleplayRuntimeConfig,
    VisionDescriptionConfig, validate_momo_document,
};
pub use lsb::{
    LSB_CARRIER_MAGIC, LSB_CARRIER_VERSION, LSB_HEADER_BYTES, LsbCarrierError, LsbCarrierInfo,
    LsbImageFormat, LsbPayload, LsbPayloadType, MAX_LSB_IMAGE_BYTES, MAX_LSB_IMAGE_PIXELS,
    MAX_LSB_PAYLOAD_BYTES, MOMO_LSB_CHARACTER_SCHEMA, embed_lsb_carrier, embed_lsb_image,
    embed_lsb_png, embed_lsb_webp, extract_lsb_carrier, extract_lsb_image, extract_lsb_png,
    extract_lsb_webp, lsb_capacity,
};
pub use momo_config;
pub use momo_crypto;
pub use momo_domain;
pub use momo_memory;
pub use momo_memory::{MoStateAudit, MoStateContext};
pub use momo_moc;
pub use momo_storage;
pub use momo_storage::{DEFAULT_NSG_VECTOR_TOP_K, MAX_NSG_VECTOR_TOP_K, NsgVectorStatus};
pub use orchestration::{
    MaintenanceKind, MomoApiError, MomoApiErrorKind, MomoApiService, MomoResponseEventSink,
};
pub use portable::{
    ConflictMode, HostMocModule, ImportReport, MocCharacterSelection, MocCompatibility,
    MocExportPlan, MocImportPlan, MocModule, MocProtection, PortableError, UnknownMocModule,
    export_moc, export_moc_with_host_modules, export_momo_config, export_private_moc,
    export_private_moc_with_host_modules, import_moc, import_moc_claiming_unknown_modules,
    import_moc_with_passphrase, import_moc_with_passphrase_and_claims, import_momo_config,
    moc_is_encrypted,
};
pub use response::{
    MAX_GATEWAY_HOPS, MAX_RESPONSE_ID_BYTES, MAX_RESPONSE_IMAGE_REFERENCE_BYTES,
    MAX_RESPONSE_IMAGES, MAX_RESPONSE_INPUT_BYTES, MAX_RESPONSE_INSTRUCTIONS_BYTES,
    MAX_RESPONSE_REQUEST_BYTES, MAX_RESPONSE_SSE_EVENT_BYTES, MAX_RESPONSE_STREAM_BYTES,
    MAX_RESPONSE_TOOL_ARGUMENT_BYTES, MAX_RESPONSE_TOOL_OUTPUT_BYTES,
    MAX_RESPONSE_TOOL_SCHEMA_BYTES, MAX_RESPONSE_TOOLS, MOMO_RESPONSE_SCHEMA, MomoResponse,
    MomoResponseExtension, MomoResponseMetadata, MomoResponseRequest, ResponseContentBlock,
    ResponseContractError, ResponseError, ResponseImageInput, ResponseInput, ResponseInputItem,
    ResponseMessageContent, ResponseOutputContent, ResponseOutputItem, ResponseTool, ResponseUsage,
};
pub use vision::{
    DEFAULT_VISION_ROUTE, GatewayVisionAdapter, MAX_VISUAL_DESCRIPTION_BYTES,
    VisionDescriptionAdapter, VisionDescriptionBatch, VisionDescriptionRequest, VisionError,
};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("data directory {path} is already owned by another MOMO Core process: {source}")]
    InstanceLock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("local storage initialization failed: {0}")]
    Storage(#[from] StorageError),
    #[error("memory initialization failed: {0}")]
    Memory(#[from] momo_memory::MemoryError),
}

#[derive(Debug, Clone)]
pub struct MomoCore {
    data_dir: PathBuf,
    _instance_lock: Arc<File>,
    store: LocalStore,
    vector_store: TursoVectorStore,
}

impl MomoCore {
    pub async fn initialize(data_dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).map_err(momo_memory::MemoryError::from)?;
        let instance_lock = acquire_instance_lock(data_dir)?;
        let store = LocalStore::open(data_dir.join("momo.sqlite3")).await?;
        let vector_store = TursoVectorStore::open(data_dir.join("nsg-vectors.db")).await?;
        migrate_space_directories(data_dir)?;
        std::fs::create_dir_all(data_dir.join("spaces")).map_err(momo_memory::MemoryError::from)?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            _instance_lock: Arc::new(instance_lock),
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

    pub fn memory_for_space(
        &self,
        space_id: uuid::Uuid,
    ) -> Result<MemoryWorkspace, momo_memory::MemoryError> {
        MemoryWorkspace::initialize(
            self.data_dir
                .join("spaces")
                .join(space_id.to_string())
                .join("memory"),
        )
    }
}

fn acquire_instance_lock(data_dir: &Path) -> Result<File, CoreError> {
    let path = data_dir.join(".momo.lock");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| CoreError::InstanceLock {
            path: path.clone(),
            source,
        })?;
    file.try_lock_exclusive()
        .map_err(|source| CoreError::InstanceLock {
            path: path.clone(),
            source,
        })?;
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| writeln!(file, "pid={}", std::process::id()))
        .and_then(|()| file.sync_data())
        .map_err(|source| CoreError::InstanceLock { path, source })?;
    Ok(file)
}

fn migrate_space_directories(data_dir: &Path) -> Result<(), CoreError> {
    let legacy = data_dir.join("memory/users");
    let scopes = data_dir.join("memory/scopes");
    if legacy.exists() && scopes.exists() {
        return Err(momo_memory::MemoryError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "both memory/users and memory/scopes exist; merge them before starting Core",
        ))
        .into());
    }
    if legacy.exists() {
        std::fs::rename(&legacy, &scopes).map_err(momo_memory::MemoryError::from)?;
    }
    let spaces = data_dir.join("spaces");
    std::fs::create_dir_all(&spaces).map_err(momo_memory::MemoryError::from)?;
    if scopes.exists() {
        for entry in std::fs::read_dir(&scopes).map_err(momo_memory::MemoryError::from)? {
            let entry = entry.map_err(momo_memory::MemoryError::from)?;
            if !entry
                .file_type()
                .map_err(momo_memory::MemoryError::from)?
                .is_dir()
                || uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
            {
                return Err(momo_memory::MemoryError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "memory/scopes may contain only UUID Space directories",
                ))
                .into());
            }
            let target = spaces.join(entry.file_name()).join("memory");
            if target.exists() {
                return Err(momo_memory::MemoryError::Io(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "both legacy and Space memory exist for {}",
                        entry.file_name().to_string_lossy()
                    ),
                ))
                .into());
            }
            std::fs::create_dir_all(target.parent().expect("Space memory parent"))
                .map_err(momo_memory::MemoryError::from)?;
            std::fs::rename(entry.path(), target).map_err(momo_memory::MemoryError::from)?;
        }
        std::fs::remove_dir(&scopes).map_err(momo_memory::MemoryError::from)?;
        let memory = data_dir.join("memory");
        if std::fs::read_dir(&memory)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false)
        {
            std::fs::remove_dir(memory).map_err(momo_memory::MemoryError::from)?;
        }
    }
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
        core.memory_for_space(scope_id).expect("memory");
        assert!(
            core.data_dir()
                .join("spaces")
                .join(scope_id.to_string())
                .join("memory/current/scene.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn converts_the_legacy_memory_directory_once() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let space_id = momo_domain::new_id();
        let legacy = directory
            .path()
            .join("memory/users")
            .join(space_id.to_string());
        std::fs::create_dir_all(&legacy).expect("legacy directory");
        std::fs::write(legacy.join("marker"), "ok").expect("legacy marker");

        MomoCore::initialize(directory.path())
            .await
            .expect("initialize core");

        assert!(!directory.path().join("memory/users").exists());
        assert!(
            directory
                .path()
                .join("spaces")
                .join(space_id.to_string())
                .join("memory/marker")
                .exists()
        );
    }

    #[tokio::test]
    async fn data_directory_has_one_live_owner() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = MomoCore::initialize(directory.path())
            .await
            .expect("first owner");
        let error = MomoCore::initialize(directory.path())
            .await
            .expect_err("second owner must be rejected");
        assert!(matches!(error, CoreError::InstanceLock { .. }));
        drop(first);
        MomoCore::initialize(directory.path())
            .await
            .expect("lock is released when owner drops");
    }
}
