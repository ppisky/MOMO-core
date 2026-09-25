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
mod product_prompts;
mod prompt_spaces;
mod recovery;
mod response;
mod runtime;
mod vision;

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
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
    ExternalCharacterImportFormat, PreservedCharacterSource, PreservedCharacterSourceExport,
    export_external_character, export_preserved_character_source, import_external_character,
    read_preserved_character_source, validate_external_charx,
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
    DdmRuntimeConfig, GovernanceError, GovernedOverrides, MaintenanceRuntimeSettings,
    MoStateInjectionMode, MoStateProfile, MoStateRuntimeConfig, MomoRuntimeSettings, OverrideMode,
    RUNTIME_SETTINGS_SCHEMA_VERSION, RequestOverridePolicy, RequestedOverrides,
    RoleplayRuntimeConfig, VisionDescriptionConfig,
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
    export_moc, export_moc_with_host_modules, export_private_moc,
    export_private_moc_with_host_modules, import_moc, import_moc_claiming_unknown_modules,
    import_moc_with_passphrase, import_moc_with_passphrase_and_claims, moc_is_encrypted,
};
pub use prompt_spaces::{
    MAX_PROMPT_SPACE_BYTES, PROMPT_SPACES_SCHEMA, PromptSpace, PromptSpaceId, PromptSpaceSource,
    PromptSpaces, PromptSpacesError,
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
pub use runtime::MomoRuntime;
pub use vision::{
    DEFAULT_VISION_ROUTE, GatewayVisionAdapter, MAX_VISUAL_DESCRIPTION_BYTES,
    VisionDescriptionAdapter, VisionDescriptionBatch, VisionDescriptionRequest, VisionError,
};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Prompts(#[from] PromptSpacesError),
    #[error("Space {space_id} requires memory recovery: {message}")]
    Recovery {
        space_id: uuid::Uuid,
        message: String,
    },
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
    memory_workspaces: Arc<Mutex<MemoryWorkspaceRegistry>>,
    space_locks:
        Arc<tokio::sync::Mutex<HashMap<uuid::Uuid, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
    recovery_failures: Arc<Mutex<std::collections::BTreeMap<uuid::Uuid, String>>>,
    commit_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

const MAX_CACHED_MEMORY_WORKSPACES: usize = 1_024;

#[derive(Debug, Default)]
struct MemoryWorkspaceRegistry {
    generation: u64,
    entries: HashMap<uuid::Uuid, MemoryWorkspaceEntry>,
    initializing: HashMap<uuid::Uuid, Arc<WorkspaceInitialization>>,
}

#[derive(Debug)]
struct MemoryWorkspaceEntry {
    workspace: Arc<MemoryWorkspace>,
    last_used: u64,
}

#[derive(Debug, Default)]
struct WorkspaceInitialization {
    complete: Mutex<bool>,
    changed: Condvar,
}

impl WorkspaceInitialization {
    fn wait(&self) {
        let mut complete = self
            .complete
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*complete {
            complete = self
                .changed
                .wait(complete)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn finish(&self) {
        let mut complete = self
            .complete
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *complete = true;
        self.changed.notify_all();
    }
}

impl MemoryWorkspaceRegistry {
    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn evict_oldest_idle_workspace(&mut self) -> bool {
        let oldest = self
            .entries
            .iter()
            .filter(|(_, entry)| Arc::strong_count(&entry.workspace) == 1)
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(space_id, _)| *space_id);
        if let Some(space_id) = oldest {
            self.entries.remove(&space_id);
            true
        } else {
            false
        }
    }

    fn reserve_initialization(
        &mut self,
        space_id: uuid::Uuid,
    ) -> Result<Arc<WorkspaceInitialization>, momo_memory::MemoryError> {
        if self.entries.len() + self.initializing.len() >= MAX_CACHED_MEMORY_WORKSPACES
            && !self.evict_oldest_idle_workspace()
        {
            return Err(momo_memory::MemoryError::WorkspaceCapacity {
                limit: MAX_CACHED_MEMORY_WORKSPACES,
            });
        }
        let initialization = Arc::new(WorkspaceInitialization::default());
        self.initializing
            .insert(space_id, Arc::clone(&initialization));
        Ok(initialization)
    }
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
        let core = Self {
            data_dir: data_dir.to_path_buf(),
            _instance_lock: Arc::new(instance_lock),
            store,
            vector_store,
            memory_workspaces: Arc::new(Mutex::new(MemoryWorkspaceRegistry::default())),
            space_locks: Arc::default(),
            recovery_failures: Arc::default(),
            commit_tasks: Arc::default(),
        };
        core.recover_maintenance_commits().await?;
        Ok(core)
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
    ) -> Result<Arc<MemoryWorkspace>, momo_memory::MemoryError> {
        if let Some(message) = self.memory_recovery_status().get(&space_id) {
            return Err(momo_memory::MemoryError::InvalidPatch(format!(
                "Space {space_id} requires recovery: {message}"
            )));
        }
        self.memory_for_space_unchecked(space_id)
    }

    fn memory_for_space_unchecked(
        &self,
        space_id: uuid::Uuid,
    ) -> Result<Arc<MemoryWorkspace>, momo_memory::MemoryError> {
        loop {
            let (initialization, initialize) = {
                let mut registry = self
                    .memory_workspaces
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let generation = registry.next_generation();
                if let Some(entry) = registry.entries.get_mut(&space_id) {
                    entry.last_used = generation;
                    return Ok(Arc::clone(&entry.workspace));
                }
                if let Some(initialization) = registry.initializing.get(&space_id) {
                    (Arc::clone(initialization), false)
                } else {
                    (registry.reserve_initialization(space_id)?, true)
                }
            };

            if !initialize {
                initialization.wait();
                continue;
            }

            let result = MemoryWorkspace::initialize(
                self.data_dir
                    .join("spaces")
                    .join(space_id.to_string())
                    .join("memory"),
            )
            .map(Arc::new);
            {
                let mut registry = self
                    .memory_workspaces
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                registry.initializing.remove(&space_id);
                if let Ok(workspace) = &result {
                    let generation = registry.next_generation();
                    registry.entries.insert(
                        space_id,
                        MemoryWorkspaceEntry {
                            workspace: Arc::clone(workspace),
                            last_used: generation,
                        },
                    );
                }
            }
            initialization.finish();
            return result;
        }
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
#[path = "../tests/unit/lib.rs"]
mod tests;
