use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use chrono::Utc;
use momo_config::ConfigDocument;
use momo_domain::{CharacterCard, Conversation, Message};
use momo_moc::{ExtractionLimits, Manifest, ModuleDefinition, SpaceModuleDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;
use toml::{Table, Value as TomlValue};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::MomoCore;

const PRIVATE_MOC_AAD: &[u8] = b"momo-private-moc-v1";
const PRIVATE_MOC_PAYLOAD: &str = "private/payload.enc";
const PRIVATE_MOC_MAX_BYTES: u64 = 512 * 1024 * 1024;
const EXTERNAL_SOURCE_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PortableError {
    #[error("portable data I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration failed: {0}")]
    Config(#[from] momo_config::ConfigError),
    #[error("MOC operation failed: {0}")]
    Moc(#[from] momo_moc::MocError),
    #[error("local storage failed: {0}")]
    Storage(#[from] momo_storage::StorageError),
    #[error("memory workspace failed: {0}")]
    Memory(#[from] momo_memory::MemoryError),
    #[error("invalid JSON data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid TOML data: {0}")]
    Toml(#[from] toml::ser::Error),
    #[error("invalid UUID: {0}")]
    Uuid(#[from] uuid::Error),
    #[error("configuration contains a credential-like key: {0}")]
    CredentialInConfig(String),
    #[error("no MOC module was selected")]
    EmptySelection,
    #[error("encrypted MOC requires a passphrase")]
    MissingPassphrase,
    #[error("encrypted MOC exceeds the 512 MiB prototype limit")]
    PrivateMocTooLarge,
    #[error("unsupported encrypted MOC profile")]
    EncryptedMocProfile,
    #[error("encrypted MOC failed: {0}")]
    Crypto(#[from] momo_crypto::CryptoError),
    #[error("invalid portable data: {0}")]
    InvalidData(String),
    #[error("directory traversal failed: {0}")]
    Walk(#[from] walkdir::Error),
    #[error("failed to persist data atomically: {0}")]
    Persist(#[from] tempfile::PersistError),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MocModule {
    MomoConfig,
    Characters,
    Conversations,
    Memory,
    SemanticGraph,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MocCompatibility {
    #[default]
    None,
    PreservedSource,
    GeneratedCcv2Json,
    GeneratedCcv3Json,
    GeneratedCcv3Charx,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MocExportPlan {
    #[serde(default)]
    pub include_config: bool,
    #[serde(default)]
    pub characters: Vec<MocCharacterSelection>,
    #[serde(default)]
    pub conversations: Vec<Uuid>,
    #[serde(default)]
    pub memory: Vec<Uuid>,
    #[serde(default)]
    pub semantic_graph: Vec<Uuid>,
    #[serde(default)]
    pub compatibility: MocCompatibility,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MocCharacterSelection {
    pub space_id: Uuid,
    #[serde(default)]
    pub character_ids: Vec<Uuid>,
}

impl MocExportPlan {
    fn validate(&self, allow_empty: bool) -> Result<HashSet<MocModule>, PortableError> {
        let mut modules = HashSet::new();
        if self.include_config {
            modules.insert(MocModule::MomoConfig);
        }
        if !self.characters.is_empty() {
            modules.insert(MocModule::Characters);
        }
        if !self.conversations.is_empty() {
            modules.insert(MocModule::Conversations);
        }
        if !self.memory.is_empty() {
            modules.insert(MocModule::Memory);
        }
        if !self.semantic_graph.is_empty() {
            modules.insert(MocModule::SemanticGraph);
        }
        if modules.is_empty() && !allow_empty {
            return Err(PortableError::EmptySelection);
        }
        if self.compatibility != MocCompatibility::None && !modules.contains(&MocModule::Characters)
        {
            return Err(PortableError::InvalidData(
                "character compatibility requires the characters module".to_owned(),
            ));
        }
        validate_unique_spaces(
            self.characters.iter().map(|selection| selection.space_id),
            "characters",
        )?;
        for selection in &self.characters {
            let unique = selection
                .character_ids
                .iter()
                .copied()
                .collect::<HashSet<_>>();
            if unique.len() != selection.character_ids.len() {
                return Err(PortableError::InvalidData(format!(
                    "character selection for Space {} contains duplicate IDs",
                    selection.space_id
                )));
            }
        }
        validate_unique_spaces(self.conversations.iter().copied(), "conversations")?;
        validate_unique_spaces(self.memory.iter().copied(), "memory")?;
        validate_unique_spaces(self.semantic_graph.iter().copied(), "semantic_graph")?;
        Ok(modules)
    }
}

fn validate_unique_spaces(
    spaces: impl Iterator<Item = Uuid>,
    module: &str,
) -> Result<(), PortableError> {
    let spaces = spaces.collect::<Vec<_>>();
    if spaces.iter().copied().collect::<HashSet<_>>().len() != spaces.len() {
        return Err(PortableError::InvalidData(format!(
            "{module} contains duplicate Space selections"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictMode {
    KeepExisting,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MocImportPlan {
    #[serde(default)]
    pub apply_config: bool,
    #[serde(default)]
    pub space_map: BTreeMap<Uuid, Uuid>,
    #[serde(default = "default_conflict_mode")]
    pub conflict_mode: ConflictMode,
}

fn default_conflict_mode() -> ConflictMode {
    ConflictMode::KeepExisting
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MocProtection {
    #[default]
    None,
    Passphrase {
        value: String,
    },
}

impl MocProtection {
    #[must_use]
    pub fn passphrase(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Passphrase { value } => Some(value),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub source_format_version: u32,
    pub characters_imported: usize,
    pub conversations_imported: usize,
    pub messages_imported: usize,
    pub memory_files_imported: usize,
    pub semantic_graph_files_imported: usize,
    pub skipped_conflicts: usize,
    pub unknown_modules: Vec<UnknownMocModule>,
    pub momo_config: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostMocModule {
    pub id: String,
    pub input_path: PathBuf,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default = "default_host_module_import_order")]
    pub import_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnknownMocModule {
    pub id: String,
    pub declared_path: String,
    pub dependencies: Vec<String>,
    pub import_order: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CharacterMetadata {
    id: String,
    name: String,
    version: String,
    author: CharacterAuthor,
    #[serde(default = "default_character_file")]
    character_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opening_file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CharacterAuthor {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

type ParsedCharacterMetadata = (CharacterMetadata, String);
const DDM_PROFILE_METADATA_KIND: &str = "character_ddm_profile";
const DDM_PROFILE_ASSET: &str = "extensions/momo-ddm/profile.yaml";

pub fn export_momo_config(
    core: &MomoCore,
    output: impl AsRef<Path>,
    settings: &JsonValue,
) -> Result<(), PortableError> {
    let document = merged_momo_config(core, settings)?;
    crate::validate_momo_document(&document.to_toml_string()?)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    export_config_bundle(&document, &momo_config_path(core), output.as_ref())?;
    // Exporting a subset must not turn that subset into the local import
    // baseline. The baseline changes only after an explicit import.
    Ok(())
}

pub fn import_momo_config(
    core: &MomoCore,
    input: impl AsRef<Path>,
) -> Result<JsonValue, PortableError> {
    let input = input.as_ref();
    let document = ConfigDocument::load(input)?;
    crate::validate_momo_document(&document.to_toml_string()?)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    crate::MomoConfig::load(input)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    reject_credentials(document.values(), "")?;
    let destination = momo_config_path(core);
    import_config_bundle(&document, input, &destination)?;
    Ok(serde_json::to_value(document.values())?)
}

pub async fn export_moc(
    core: &MomoCore,
    output: impl AsRef<Path>,
    settings: &JsonValue,
    plan: &MocExportPlan,
) -> Result<Manifest, PortableError> {
    export_moc_with_host_modules(core, output, settings, plan, &[]).await
}

pub async fn export_moc_with_host_modules(
    core: &MomoCore,
    output: impl AsRef<Path>,
    settings: &JsonValue,
    plan: &MocExportPlan,
    host_modules: &[HostMocModule],
) -> Result<Manifest, PortableError> {
    let selected = plan.validate(!host_modules.is_empty())?;
    let staging = TempDir::new()?;
    let mut modules = Vec::new();
    let mut space_modules = Vec::new();
    if selected.contains(&MocModule::MomoConfig) {
        let document = merged_momo_config(core, settings)?;
        export_config_bundle(
            &document,
            &momo_config_path(core),
            &staging.path().join("config/momo.toml"),
        )?;
        modules.push(known_module_definition("config"));
    }
    if selected.contains(&MocModule::Characters) {
        for selection in &plan.characters {
            export_characters(
                core,
                staging.path(),
                selection.space_id,
                &selection.character_ids,
                plan.compatibility,
            )
            .await?;
            space_modules.push(space_module_definition("characters", selection.space_id));
        }
        modules.push(known_module_definition("characters"));
        if staging.path().join("tavern_compat").exists() {
            modules.push(ModuleDefinition {
                id: "tavern_compat".to_owned(),
                path: "tavern_compat".to_owned(),
                dependencies: vec!["characters".to_owned()],
                import_order: 1_000,
            });
        }
    }
    if selected.contains(&MocModule::Conversations) {
        for space_id in &plan.conversations {
            export_conversations(core, staging.path(), *space_id).await?;
            space_modules.push(space_module_definition("conversations", *space_id));
        }
        modules.push(known_module_definition("conversations"));
    }
    if selected.contains(&MocModule::Memory) {
        for space_id in &plan.memory {
            let memory = core.memory_for_space(*space_id)?;
            copy_tree_filtered(
                memory.root(),
                &staging
                    .path()
                    .join("memory/spaces")
                    .join(space_id.to_string()),
                false,
            )?;
            space_modules.push(space_module_definition("memory", *space_id));
        }
        modules.push(known_module_definition("memory"));
    }
    if selected.contains(&MocModule::SemanticGraph) {
        for space_id in &plan.semantic_graph {
            let memory = core.memory_for_space(*space_id)?;
            copy_tree_filtered(
                memory.root(),
                &staging
                    .path()
                    .join("semantic_graph/spaces")
                    .join(space_id.to_string()),
                true,
            )?;
            space_modules.push(space_module_definition("semantic_graph", *space_id));
        }
        modules.push(known_module_definition("semantic_graph"));
    }
    let mut module_ids = modules
        .iter()
        .map(|module| module.id.clone())
        .collect::<HashSet<_>>();
    for module in host_modules {
        validate_host_module(module, &mut module_ids)?;
        let relative = PathBuf::from("extensions").join(&module.id);
        copy_host_module_tree(&module.input_path, &staging.path().join(&relative))?;
        modules.push(ModuleDefinition {
            id: module.id.clone(),
            path: relative.to_string_lossy().replace('\\', "/"),
            dependencies: module.dependencies.clone(),
            import_order: module.import_order,
        });
    }
    if modules.is_empty() {
        return Err(PortableError::EmptySelection);
    }
    Ok(momo_moc::create_from_definitions_and_spaces(
        output,
        staging.path(),
        &modules,
        &space_modules,
    )?)
}

pub async fn import_moc(
    core: &MomoCore,
    input: impl AsRef<Path>,
    plan: &MocImportPlan,
) -> Result<ImportReport, PortableError> {
    import_moc_with_passphrase(core, input, plan, None).await
}

pub async fn import_moc_claiming_unknown_modules(
    core: &MomoCore,
    input: impl AsRef<Path>,
    plan: &MocImportPlan,
    claim_directory: impl AsRef<Path>,
) -> Result<ImportReport, PortableError> {
    import_moc_with_passphrase_and_claims(core, input, plan, None, Some(claim_directory.as_ref()))
        .await
}

pub async fn export_private_moc(
    core: &MomoCore,
    output: impl AsRef<Path>,
    settings: &JsonValue,
    plan: &MocExportPlan,
    passphrase: &str,
) -> Result<Manifest, PortableError> {
    export_private_moc_with_host_modules(core, output, settings, plan, &[], passphrase).await
}

pub async fn export_private_moc_with_host_modules(
    core: &MomoCore,
    output: impl AsRef<Path>,
    settings: &JsonValue,
    plan: &MocExportPlan,
    host_modules: &[HostMocModule],
    passphrase: &str,
) -> Result<Manifest, PortableError> {
    if passphrase.is_empty() {
        return Err(PortableError::MissingPassphrase);
    }
    let temporary = TempDir::new()?;
    let inner_path = temporary.path().join("payload.moc");
    export_moc_with_host_modules(core, &inner_path, settings, plan, host_modules).await?;
    let metadata = fs::metadata(&inner_path)?;
    if metadata.len() > PRIVATE_MOC_MAX_BYTES {
        return Err(PortableError::PrivateMocTooLarge);
    }
    let envelope = momo_crypto::encrypt(
        &fs::read(inner_path)?,
        passphrase,
        PRIVATE_MOC_AAD,
        momo_crypto::KdfParameters::DESKTOP_PROTOTYPE,
    )?;
    let wrapper = TempDir::new()?;
    atomic_write(
        &wrapper.path().join(PRIVATE_MOC_PAYLOAD),
        &momo_crypto::encode(&envelope)?,
    )?;
    Ok(momo_moc::create_with_encryption(
        output,
        wrapper.path(),
        &[("encrypted-container".to_owned(), PathBuf::from("private"))],
        Some(momo_moc::EncryptionMetadata {
            profile: "momo-envelope-v1".to_owned(),
            payload_path: PRIVATE_MOC_PAYLOAD.to_owned(),
            associated_data: String::from_utf8_lossy(PRIVATE_MOC_AAD).into_owned(),
        }),
    )?)
}

pub fn moc_is_encrypted(input: impl AsRef<Path>) -> Result<bool, PortableError> {
    Ok(momo_moc::inspect(input)?.encryption.is_some())
}

pub async fn import_moc_with_passphrase(
    core: &MomoCore,
    input: impl AsRef<Path>,
    plan: &MocImportPlan,
    passphrase: Option<&str>,
) -> Result<ImportReport, PortableError> {
    import_moc_with_passphrase_and_claims(core, input, plan, passphrase, None).await
}

pub async fn import_moc_with_passphrase_and_claims(
    core: &MomoCore,
    input: impl AsRef<Path>,
    plan: &MocImportPlan,
    passphrase: Option<&str>,
    claim_directory: Option<&Path>,
) -> Result<ImportReport, PortableError> {
    let mode = plan.conflict_mode;
    let outer = TempDir::new()?;
    let manifest = momo_moc::extract(input, outer.path(), ExtractionLimits::default())?;
    let mut payload_manifest = manifest.clone();
    let inner = if let Some(encryption) = &manifest.encryption {
        if encryption.profile != "momo-envelope-v1"
            || encryption.payload_path != PRIVATE_MOC_PAYLOAD
            || encryption.associated_data.as_bytes() != PRIVATE_MOC_AAD
        {
            return Err(PortableError::EncryptedMocProfile);
        }
        let passphrase = passphrase
            .filter(|value| !value.is_empty())
            .ok_or(PortableError::MissingPassphrase)?;
        let envelope =
            momo_crypto::decode_envelope(&fs::read(outer.path().join(PRIVATE_MOC_PAYLOAD))?)?;
        let plaintext = momo_crypto::decrypt(&envelope, passphrase, PRIVATE_MOC_AAD)?;
        if u64::try_from(plaintext.len()).unwrap_or(u64::MAX) > PRIVATE_MOC_MAX_BYTES {
            return Err(PortableError::PrivateMocTooLarge);
        }
        let inner_container = outer.path().join("decrypted.moc");
        atomic_write(&inner_container, &plaintext)?;
        let destination = TempDir::new()?;
        payload_manifest = momo_moc::extract(
            inner_container,
            destination.path(),
            ExtractionLimits::default(),
        )?;
        Some(destination)
    } else {
        None
    };
    let extracted = inner.as_ref().map_or(outer.path(), TempDir::path);
    let mut unknown_modules = payload_manifest
        .module_definitions
        .iter()
        .filter(|module| !is_core_moc_module(&module.id))
        .map(|module| UnknownMocModule {
            id: module.id.clone(),
            declared_path: module.path.clone(),
            dependencies: module.dependencies.clone(),
            import_order: module.import_order,
            claimed_path: None,
        })
        .collect::<Vec<_>>();
    let mut report = ImportReport {
        source_format_version: payload_manifest.format_version,
        characters_imported: 0,
        conversations_imported: 0,
        messages_imported: 0,
        memory_files_imported: 0,
        semantic_graph_files_imported: 0,
        skipped_conflicts: 0,
        unknown_modules: Vec::new(),
        momo_config: None,
    };
    let source_spaces = payload_manifest
        .space_modules
        .iter()
        .map(|space| Uuid::parse_str(&space.space_id))
        .collect::<Result<HashSet<_>, _>>()?;
    if plan
        .space_map
        .keys()
        .any(|source| !source_spaces.contains(source))
    {
        return Err(PortableError::InvalidData(
            "space_map contains a source Space absent from the MOC".to_owned(),
        ));
    }
    if plan
        .space_map
        .values()
        .copied()
        .collect::<HashSet<_>>()
        .len()
        != plan.space_map.len()
    {
        return Err(PortableError::InvalidData(
            "space_map cannot collapse multiple source Spaces into one target Space".to_owned(),
        ));
    }
    preflight_moc_payload(core, extracted, &payload_manifest, plan, claim_directory).await?;
    if plan.apply_config
        && payload_manifest
            .modules
            .iter()
            .any(|entry| entry.module == "config")
    {
        let path = extracted.join("config/momo.toml");
        if path.exists() {
            report.momo_config = Some(import_momo_config(core, path)?);
        }
    }
    let mut imported_character_ids = HashSet::new();
    for space in &payload_manifest.space_modules {
        let source_space = Uuid::parse_str(&space.space_id)?;
        let target_space = plan
            .space_map
            .get(&source_space)
            .copied()
            .unwrap_or(source_space);
        let directory = extracted.join(&space.path);
        match space.module.as_str() {
            "characters" => {
                imported_character_ids.extend(
                    import_characters(core, &directory, target_space, mode, &mut report).await?,
                );
            }
            "conversations" => {
                import_conversations(core, &directory, target_space, mode, &mut report).await?;
            }
            "memory" => import_memory(core, &directory, target_space, mode, &mut report)?,
            "semantic_graph" => {
                import_semantic_graph(core, &directory, target_space, mode, &mut report)?;
            }
            _ => {
                return Err(PortableError::InvalidData(format!(
                    "unknown Core Space module: {}",
                    space.module
                )));
            }
        }
    }
    import_external_character_sources(core, extracted, &imported_character_ids).await?;
    if let Some(claim_directory) = claim_directory {
        for module in &mut unknown_modules {
            let claimed = claim_unknown_module(extracted, claim_directory, module)?;
            module.claimed_path = Some(claimed.to_string_lossy().into_owned());
        }
    }
    report.unknown_modules = unknown_modules;
    Ok(report)
}

async fn preflight_moc_payload(
    core: &MomoCore,
    extracted: &Path,
    manifest: &Manifest,
    plan: &MocImportPlan,
    claim_directory: Option<&Path>,
) -> Result<(), PortableError> {
    if plan.apply_config
        && manifest
            .modules
            .iter()
            .any(|entry| entry.module == "config")
    {
        preflight_config_bundle(extracted.join("config/momo.toml"))?;
    }

    let existing_characters = core
        .store()
        .list_characters()
        .await?
        .into_iter()
        .map(|character| character.id)
        .collect::<HashSet<_>>();
    let mut character_ids = existing_characters.clone();
    let mut imported_character_ids = HashSet::new();
    let mut conversation_ids = HashSet::new();
    let mut message_ids = HashSet::new();

    for space in &manifest.space_modules {
        let source_space = Uuid::parse_str(&space.space_id)?;
        let directory = extracted.join(&space.path);
        match space.module.as_str() {
            "characters" => {
                let ids = preflight_characters(&directory)?;
                for id in ids {
                    if !imported_character_ids.insert(id) {
                        return Err(PortableError::InvalidData(format!(
                            "character {id} occurs in more than one Space module"
                        )));
                    }
                    character_ids.insert(id);
                }
            }
            "conversations" => preflight_conversations(
                &directory,
                source_space,
                &mut conversation_ids,
                &mut message_ids,
            )?,
            "memory" | "semantic_graph" => preflight_workspace_tree(&directory)?,
            _ => {
                return Err(PortableError::InvalidData(format!(
                    "unknown Core Space module: {}",
                    space.module
                )));
            }
        }
    }
    preflight_external_character_sources(extracted, &character_ids)?;

    if let Some(claim_directory) = claim_directory {
        for module in manifest
            .module_definitions
            .iter()
            .filter(|module| !is_core_moc_module(&module.id))
        {
            validate_module_id(&module.id)?;
            let destination = claim_directory.join(&module.id);
            if destination.exists() {
                return Err(PortableError::InvalidData(format!(
                    "claimed MOC module destination already exists: {}",
                    destination.display()
                )));
            }
        }
    }
    Ok(())
}

fn preflight_config_bundle(source: PathBuf) -> Result<(), PortableError> {
    let document = ConfigDocument::load(&source)?;
    let text = document.to_toml_string()?;
    crate::validate_momo_document(&text)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    crate::MomoConfig::load(&source)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    reject_credentials(document.values(), "")?;
    let destination = TempDir::new()?;
    import_config_bundle(&document, &source, &destination.path().join("momo.toml"))?;
    Ok(())
}

fn preflight_characters(directory: &Path) -> Result<HashSet<Uuid>, PortableError> {
    if !directory.is_dir() {
        return Err(PortableError::InvalidData(format!(
            "declared character Space is not a directory: {}",
            directory.display()
        )));
    }
    let index: Vec<Uuid> = serde_json::from_slice(&fs::read(directory.join("index.json"))?)?;
    let expected = index.iter().copied().collect::<HashSet<_>>();
    if expected.len() != index.len() {
        return Err(PortableError::InvalidData(
            "character index contains duplicate IDs".to_owned(),
        ));
    }
    let mut discovered = HashSet::new();
    for item in fs::read_dir(directory)? {
        let item = item?;
        if item.file_name() == "index.json" {
            continue;
        }
        if !item.file_type()?.is_dir() {
            return Err(PortableError::InvalidData(format!(
                "character Space contains an unexpected non-directory entry: {}",
                item.path().display()
            )));
        }
        let asset_directory = item.path();
        let directory_id = Uuid::parse_str(&item.file_name().to_string_lossy())?;
        let metadata_document = ConfigDocument::load(asset_directory.join("character.toml"))?;
        let (metadata, _) = parse_character_metadata(metadata_document.values())?;
        validate_character_metadata(&metadata)?;
        let id = parse_character_id(&metadata.id)?;
        if id != directory_id {
            return Err(PortableError::InvalidData(format!(
                "character directory {directory_id} does not match metadata ID {id}"
            )));
        }
        if !discovered.insert(id) {
            return Err(PortableError::InvalidData(format!(
                "character {id} is declared by more than one asset"
            )));
        }
        let character_file = validate_asset_path(&metadata.character_file)?;
        let default_user_file = asset_directory
            .join("user.md")
            .exists()
            .then_some("user.md");
        let user_file = metadata
            .user_file
            .as_deref()
            .or(default_user_file)
            .map(validate_asset_path)
            .transpose()?;
        let default_opening_file = asset_directory
            .join("opening.md")
            .exists()
            .then_some("opening.md");
        let opening_file = metadata
            .opening_file
            .as_deref()
            .or(default_opening_file)
            .map(validate_asset_path)
            .transpose()?;
        let character_key = portable_case_fold(&character_file);
        let user_key = user_file.as_deref().map(portable_case_fold);
        let opening_key = opening_file.as_deref().map(portable_case_fold);
        if user_key.as_ref().is_some_and(|user| user == &character_key)
            || opening_key.as_ref().is_some_and(|opening| {
                opening == &character_key || user_key.as_ref().is_some_and(|user| opening == user)
            })
        {
            return Err(PortableError::InvalidData(
                "character_file, user_file, and opening_file must refer to different files"
                    .to_owned(),
            ));
        }
        read_markdown_asset(&asset_directory, &character_file)?;
        if let Some(path) = user_file.as_deref() {
            read_markdown_asset(&asset_directory, path)?;
        }
        if let Some(path) = opening_file.as_deref() {
            read_markdown_asset(&asset_directory, path)?;
        }
        read_ddm_profile_asset(&asset_directory, id)?;
    }
    if discovered != expected {
        return Err(PortableError::InvalidData(
            "character index does not match the character assets".to_owned(),
        ));
    }
    Ok(discovered)
}

fn preflight_conversations(
    directory: &Path,
    source_space: Uuid,
    all_conversation_ids: &mut HashSet<Uuid>,
    all_message_ids: &mut HashSet<Uuid>,
) -> Result<(), PortableError> {
    if !directory.is_dir() {
        return Err(PortableError::InvalidData(format!(
            "declared conversation Space is not a directory: {}",
            directory.display()
        )));
    }
    let conversations: Vec<Conversation> =
        serde_json::from_slice(&fs::read(directory.join("index.json"))?)?;
    let mut local_conversations = HashSet::new();
    for conversation in conversations {
        if conversation.scope_id != source_space {
            return Err(PortableError::InvalidData(format!(
                "conversation {} declares Space {} but is stored in Space {}",
                conversation.id, conversation.scope_id, source_space
            )));
        }
        if !local_conversations.insert(conversation.id)
            || !all_conversation_ids.insert(conversation.id)
        {
            return Err(PortableError::InvalidData(format!(
                "conversation {} occurs more than once in the MOC",
                conversation.id
            )));
        }
    }
    let messages: Vec<Message> =
        serde_json::from_slice(&fs::read(directory.join("messages.json"))?)?;
    for message in messages {
        if !local_conversations.contains(&message.conversation_id) {
            return Err(PortableError::InvalidData(format!(
                "message {} references conversation {} outside its Space module",
                message.id, message.conversation_id
            )));
        }
        if !all_message_ids.insert(message.id) {
            return Err(PortableError::InvalidData(format!(
                "message {} occurs more than once in the MOC",
                message.id
            )));
        }
    }
    Ok(())
}

fn preflight_workspace_tree(directory: &Path) -> Result<(), PortableError> {
    // ZIP does not preserve an empty directory. A declared Space module with
    // no files is therefore a valid empty selection after extraction.
    if !directory.exists() {
        return Ok(());
    }
    if !directory.is_dir() {
        return Err(PortableError::InvalidData(format!(
            "declared workspace Space is not a directory: {}",
            directory.display()
        )));
    }
    for item in WalkDir::new(directory).follow_links(false) {
        let item = item?;
        if item.file_type().is_symlink()
            || (!item.file_type().is_file() && !item.file_type().is_dir())
        {
            return Err(PortableError::InvalidData(format!(
                "workspace payload contains an unsupported entry: {}",
                item.path().display()
            )));
        }
        if item.file_type().is_file() {
            fs::read(item.path())?;
        }
    }
    Ok(())
}

fn preflight_external_character_sources(
    root: &Path,
    character_ids: &HashSet<Uuid>,
) -> Result<(), PortableError> {
    let directory = root.join("tavern_compat");
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            return Err(PortableError::InvalidData(
                "tavern_compat entries must be character directories".to_owned(),
            ));
        }
        let id = Uuid::parse_str(&entry.file_name().to_string_lossy())?;
        if !character_ids.contains(&id) {
            return Err(PortableError::InvalidData(format!(
                "tavern_compat source references unknown character {id}"
            )));
        }
        let metadata_path = entry.path().join("metadata.json");
        if metadata_path.exists() {
            let source = read_external_source_asset(&metadata_path)?;
            crate::character_compat::validate_preserved_source_asset(&source, &entry.path())
                .map_err(|error| {
                    PortableError::InvalidData(format!(
                        "MOC CHARX source cannot be imported: {error}"
                    ))
                })?;
        }
    }
    Ok(())
}

fn merged_momo_config(
    core: &MomoCore,
    settings: &JsonValue,
) -> Result<ConfigDocument, PortableError> {
    let baseline = momo_config_path(core);
    let document = if baseline.exists() {
        ConfigDocument::load(&baseline)?
    } else {
        ConfigDocument::default()
    };
    let value = TomlValue::try_from(settings)?;
    let incoming = value
        .as_table()
        .ok_or_else(|| PortableError::InvalidData("settings must be an object".to_owned()))?;
    reject_credentials(incoming, "")?;
    let mut values = document.values().clone();
    merge_table(&mut values, incoming);
    let document = ConfigDocument::new(values);
    crate::validate_momo_document(&document.to_toml_string()?)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    Ok(document)
}

fn export_config_bundle(
    document: &ConfigDocument,
    source_config: &Path,
    destination_config: &Path,
) -> Result<(), PortableError> {
    copy_config_bundle(document, source_config, destination_config)
}

fn import_config_bundle(
    document: &ConfigDocument,
    source_config: &Path,
    destination_config: &Path,
) -> Result<(), PortableError> {
    copy_config_bundle(document, source_config, destination_config)
}

fn copy_config_bundle(
    document: &ConfigDocument,
    _source_config: &Path,
    destination_config: &Path,
) -> Result<(), PortableError> {
    let text = document.to_toml_string()?;
    crate::validate_momo_document(&text)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    let destination_base = destination_config
        .parent()
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(destination_base)?;
    document.save(destination_config)?;
    Ok(())
}

fn merge_table(target: &mut Table, incoming: &Table) {
    for (key, value) in incoming {
        if let (Some(TomlValue::Table(target_table)), TomlValue::Table(incoming_table)) =
            (target.get_mut(key), value)
        {
            merge_table(target_table, incoming_table);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn reject_credentials(table: &Table, prefix: &str) -> Result<(), PortableError> {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let normalized = key.to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "api_key" | "password" | "secret" | "access_token" | "refresh_token"
        ) || normalized.ends_with("_api_key")
        {
            return Err(PortableError::CredentialInConfig(path));
        }
        match value {
            TomlValue::Table(child) => reject_credentials(child, &path)?,
            TomlValue::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    if let TomlValue::Table(child) = item {
                        reject_credentials(child, &format!("{path}[{index}]"))?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn export_characters(
    core: &MomoCore,
    root: &Path,
    scope_id: Uuid,
    character_ids: &[Uuid],
    compatibility: MocCompatibility,
) -> Result<(), PortableError> {
    let mut characters = core.store().list_characters_for_scope(scope_id).await?;
    if !character_ids.is_empty() {
        let selected = character_ids.iter().copied().collect::<HashSet<_>>();
        characters.retain(|card| selected.contains(&card.id));
        if characters.len() != selected.len() {
            return Err(PortableError::InvalidData(
                "a selected character does not exist in its declared owner Space".to_owned(),
            ));
        }
    }
    let root_directory = root.join("characters/spaces").join(scope_id.to_string());
    fs::create_dir_all(&root_directory)?;
    atomic_write(
        &root_directory.join("index.json"),
        &serde_json::to_vec_pretty(&characters.iter().map(|card| card.id).collect::<Vec<_>>())?,
    )?;
    for card in characters {
        let card_id = card.id;
        let directory = root_directory.join(card_id.to_string());
        fs::create_dir_all(&directory)?;
        let metadata = CharacterMetadata {
            id: format!("urn:uuid:{}", card.id),
            name: card.name,
            version: card.version,
            author: CharacterAuthor {
                name: card.author_name,
                url: card.author_url,
            },
            character_file: "character.md".to_owned(),
            user_file: (!card.user_markdown.is_empty()).then(|| "user.md".to_owned()),
            opening_file: card
                .opening_markdown
                .as_ref()
                .map(|_| "opening.md".to_owned()),
        };
        let known = TomlValue::try_from(&metadata)?;
        let known = known
            .as_table()
            .ok_or_else(|| PortableError::InvalidData("character metadata".to_owned()))?;
        let mut metadata_values = if let Some(original) = core
            .store()
            .portable_metadata("character", &card_id.to_string())
            .await?
        {
            ConfigDocument::parse(&original)?.values().clone()
        } else {
            Table::new()
        };
        for removed in [
            "description",
            "language",
            "tags",
            "created_at",
            "updated_at",
        ] {
            metadata_values.remove(removed);
        }
        if let Some(TomlValue::Table(author)) = metadata_values.get_mut("author") {
            author.remove("uid");
            author.remove("display_name");
        }
        merge_table(&mut metadata_values, known);
        atomic_write(
            &directory.join("character.toml"),
            ConfigDocument::new(metadata_values)
                .to_toml_string()?
                .as_bytes(),
        )?;
        atomic_write(
            &directory.join("character.md"),
            card.character_markdown.as_bytes(),
        )?;
        if !card.user_markdown.is_empty() {
            atomic_write(&directory.join("user.md"), card.user_markdown.as_bytes())?;
        }
        if let Some(opening) = card.opening_markdown {
            atomic_write(&directory.join("opening.md"), opening.as_bytes())?;
        }
        if let Some(profile_yaml) = core
            .store()
            .portable_metadata(DDM_PROFILE_METADATA_KIND, &card_id.to_string())
            .await?
        {
            let profile = momo_memory::DdmProfile::parse_yaml(&profile_yaml)
                .map_err(|error| PortableError::InvalidData(error.to_string()))?;
            if profile.character_id != card_id.to_string() {
                return Err(PortableError::InvalidData(format!(
                    "DDM profile character_id does not match character {card_id}"
                )));
            }
            atomic_write(&directory.join(DDM_PROFILE_ASSET), profile_yaml.as_bytes())?;
        }
        let compat_directory = root.join("tavern_compat").join(card_id.to_string());
        match compatibility {
            MocCompatibility::None => {}
            MocCompatibility::PreservedSource => {
                if let Some(external) = core
                    .store()
                    .portable_metadata("external_character_card", &card_id.to_string())
                    .await?
                {
                    serde_json::from_str::<serde_json::Value>(&external).map_err(|error| {
                        PortableError::InvalidData(format!(
                            "stored external character metadata is invalid: {error}"
                        ))
                    })?;
                    fs::create_dir_all(&compat_directory)?;
                    atomic_write(&compat_directory.join("metadata.json"), external.as_bytes())?;
                    crate::character_compat::export_preserved_source_asset(
                        core,
                        card_id,
                        &external,
                        &compat_directory,
                    )
                    .map_err(|error| {
                        PortableError::InvalidData(format!(
                            "stored external character source cannot be exported: {error}"
                        ))
                    })?;
                }
            }
            MocCompatibility::GeneratedCcv2Json
            | MocCompatibility::GeneratedCcv3Json
            | MocCompatibility::GeneratedCcv3Charx => {
                let (format, file_name) = match compatibility {
                    MocCompatibility::GeneratedCcv2Json => (
                        crate::ExternalCharacterExportFormat::Ccv2Json,
                        "generated.ccv2.json",
                    ),
                    MocCompatibility::GeneratedCcv3Json => (
                        crate::ExternalCharacterExportFormat::Ccv3Json,
                        "generated.ccv3.json",
                    ),
                    MocCompatibility::GeneratedCcv3Charx => (
                        crate::ExternalCharacterExportFormat::Ccv3Charx,
                        "generated.ccv3.charx",
                    ),
                    MocCompatibility::None | MocCompatibility::PreservedSource => unreachable!(),
                };
                fs::create_dir_all(&compat_directory)?;
                crate::export_external_character(
                    core,
                    scope_id,
                    card_id,
                    compat_directory.join(file_name),
                    format,
                )
                .await
                .map_err(|error| {
                    PortableError::InvalidData(format!(
                        "generated character compatibility export failed: {error}"
                    ))
                })?;
            }
        }
    }
    Ok(())
}

async fn export_conversations(
    core: &MomoCore,
    root: &Path,
    scope_id: Uuid,
) -> Result<(), PortableError> {
    let conversations = core.store().list_conversations_for_scope(scope_id).await?;
    let mut messages = Vec::new();
    for conversation in &conversations {
        messages.extend(core.store().list_messages(conversation.id).await?);
    }
    let directory = root.join("conversations/spaces").join(scope_id.to_string());
    fs::create_dir_all(&directory)?;
    atomic_write(
        &directory.join("index.json"),
        &serde_json::to_vec_pretty(&conversations)?,
    )?;
    atomic_write(
        &directory.join("messages.json"),
        &serde_json::to_vec_pretty(&messages)?,
    )?;
    Ok(())
}

async fn import_characters(
    core: &MomoCore,
    directory: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<HashSet<Uuid>, PortableError> {
    if !directory.exists() {
        return Ok(HashSet::new());
    }
    let existing = core
        .store()
        .list_characters()
        .await?
        .into_iter()
        .map(|card| card.id)
        .collect::<HashSet<_>>();
    let mut imported_ids = HashSet::new();
    for item in fs::read_dir(directory)? {
        let directory = item?.path();
        if !directory.is_dir() {
            continue;
        }
        let metadata_document = ConfigDocument::load(directory.join("character.toml"))?;
        let (metadata, portable_metadata) = parse_character_metadata(metadata_document.values())?;
        validate_character_metadata(&metadata)?;
        let id = parse_character_id(&metadata.id)?;
        if !imported_ids.insert(id) {
            return Err(PortableError::InvalidData(format!(
                "character {id} is declared by more than one asset"
            )));
        }
        if existing.contains(&id) && mode == ConflictMode::KeepExisting {
            report.skipped_conflicts += 1;
            continue;
        }
        let character_file = validate_asset_path(&metadata.character_file)?;
        let default_user_file = directory.join("user.md").exists().then_some("user.md");
        let user_file = metadata
            .user_file
            .as_deref()
            .or(default_user_file)
            .map(validate_asset_path)
            .transpose()?;
        let default_opening_file = directory
            .join("opening.md")
            .exists()
            .then_some("opening.md");
        let opening_file = metadata
            .opening_file
            .as_deref()
            .or(default_opening_file)
            .map(validate_asset_path)
            .transpose()?;
        let character_key = portable_case_fold(&character_file);
        let user_key = user_file.as_deref().map(portable_case_fold);
        let opening_key = opening_file.as_deref().map(portable_case_fold);
        if user_key.as_ref().is_some_and(|user| user == &character_key)
            || opening_key.as_ref().is_some_and(|opening| {
                opening == &character_key || user_key.as_ref().is_some_and(|user| opening == user)
            })
        {
            return Err(PortableError::InvalidData(
                "character_file, user_file, and opening_file must refer to different files"
                    .to_owned(),
            ));
        }
        let character_markdown = read_markdown_asset(&directory, &character_file)?;
        let user_markdown = user_file
            .as_deref()
            .map(|path| read_markdown_asset(&directory, path))
            .transpose()?
            .unwrap_or_default();
        let opening_markdown = opening_file
            .as_deref()
            .map(|path| read_markdown_asset(&directory, path))
            .transpose()?;
        let ddm_profile = read_ddm_profile_asset(&directory, id)?;
        let now = Utc::now();
        let character = CharacterCard {
            id,
            scope_id,
            name: metadata.name,
            version: metadata.version,
            author_name: metadata.author.name,
            author_url: metadata.author.url,
            character_markdown,
            user_markdown,
            opening_markdown,
            created_at: now,
            updated_at: now,
        };
        if existing.contains(&id) {
            core.store().stage_character_update(&character).await?;
        } else {
            core.store().stage_character(&character).await?;
        }
        core.store()
            .save_portable_metadata("character", &id.to_string(), &portable_metadata)
            .await?;
        if let Some(profile_yaml) = ddm_profile {
            core.store()
                .save_portable_metadata(DDM_PROFILE_METADATA_KIND, &id.to_string(), &profile_yaml)
                .await?;
        } else {
            core.store()
                .delete_character_ddm_profile(&id.to_string())
                .await?;
        }
        report.characters_imported += 1;
    }
    Ok(imported_ids
        .into_iter()
        .filter(|id| !existing.contains(id) || mode == ConflictMode::Replace)
        .collect())
}

async fn import_external_character_sources(
    core: &MomoCore,
    root: &Path,
    imported_character_ids: &HashSet<Uuid>,
) -> Result<(), PortableError> {
    let directory = root.join("tavern_compat");
    if !directory.exists() {
        return Ok(());
    }
    let character_ids = core
        .store()
        .list_characters()
        .await?
        .into_iter()
        .map(|character| character.id)
        .collect::<HashSet<_>>();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            return Err(PortableError::InvalidData(
                "tavern_compat entries must be character directories".to_owned(),
            ));
        }
        let id = Uuid::parse_str(&entry.file_name().to_string_lossy())?;
        if !imported_character_ids.contains(&id) {
            continue;
        }
        if !character_ids.contains(&id) {
            return Err(PortableError::InvalidData(format!(
                "tavern_compat source references unknown character {id}"
            )));
        }
        let metadata_path = entry.path().join("metadata.json");
        if !metadata_path.exists() {
            // A generated compatibility profile accompanies the native
            // character for external export only. It is not provenance and
            // must not be imported back as the character's stored source.
            continue;
        }
        let source = read_external_source_asset(&metadata_path)?;
        crate::character_compat::import_preserved_source_asset(core, id, &source, &entry.path())
            .map_err(|error| {
                PortableError::InvalidData(format!("MOC CHARX source cannot be imported: {error}"))
            })?;
        core.store()
            .save_portable_metadata("external_character_card", &id.to_string(), &source)
            .await?;
    }
    Ok(())
}

async fn import_conversations(
    core: &MomoCore,
    directory: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    if !directory.exists() {
        return Ok(());
    }
    let existing_conversations = core
        .store()
        .list_conversations()
        .await?
        .into_iter()
        .map(|conversation| conversation.id)
        .collect::<HashSet<_>>();
    let available_characters = core
        .store()
        .list_characters()
        .await?
        .into_iter()
        .map(|character| character.id)
        .collect::<HashSet<_>>();
    let mut available = existing_conversations.clone();
    let mut conversations: Vec<Conversation> =
        serde_json::from_slice(&fs::read(directory.join("index.json"))?)?;
    for conversation in &mut conversations {
        if existing_conversations.contains(&conversation.id) && mode == ConflictMode::KeepExisting {
            report.skipped_conflicts += 1;
            continue;
        }
        conversation.scope_id = scope_id;
        if conversation
            .character_id
            .is_some_and(|character_id| !available_characters.contains(&character_id))
        {
            // A conversation-only MOC intentionally omits character cards.
            // Keep the conversation importable rather than violating SQLite's
            // foreign key constraint with a missing optional parent.
            conversation.character_id = None;
        }
        if existing_conversations.contains(&conversation.id) {
            core.store().stage_conversation_update(conversation).await?;
        } else {
            core.store().stage_conversation(conversation).await?;
        }
        available.insert(conversation.id);
        report.conversations_imported += 1;
    }

    let messages: Vec<Message> =
        serde_json::from_slice(&fs::read(directory.join("messages.json"))?)?;
    let mut known_message_ids = HashSet::new();
    for conversation_id in &available {
        known_message_ids.extend(
            core.store()
                .list_messages(*conversation_id)
                .await?
                .into_iter()
                .map(|message| message.id),
        );
    }
    for message in messages {
        if !available.contains(&message.conversation_id) {
            report.skipped_conflicts += 1;
            continue;
        }
        if known_message_ids.contains(&message.id) && mode == ConflictMode::KeepExisting {
            report.skipped_conflicts += 1;
            continue;
        }
        if known_message_ids.contains(&message.id) {
            core.store().stage_message_update(&message).await?;
        } else {
            core.store().stage_message(&message).await?;
        }
        known_message_ids.insert(message.id);
        report.messages_imported += 1;
    }
    Ok(())
}

fn import_memory(
    core: &MomoCore,
    source: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    if !source.exists() {
        return Ok(());
    }
    for item in WalkDir::new(source).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(source)
            .map_err(|_| PortableError::InvalidData("invalid memory path".to_owned()))?;
        let target = core.memory_for_space(scope_id)?.root().join(relative);
        if target.exists() && mode == ConflictMode::KeepExisting {
            report.skipped_conflicts += 1;
            continue;
        }
        atomic_write(&target, &fs::read(item.path())?)?;
        report.memory_files_imported += 1;
    }
    Ok(())
}

fn import_semantic_graph(
    core: &MomoCore,
    source: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    if !source.exists() {
        return Ok(());
    }
    import_workspace_tree(core, source, scope_id, mode, report)
}

fn import_workspace_tree(
    core: &MomoCore,
    source: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    for item in WalkDir::new(source).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(source)
            .map_err(|_| PortableError::InvalidData("invalid workspace path".to_owned()))?;
        let target = core.memory_for_space(scope_id)?.root().join(relative);
        if target.exists() && mode == ConflictMode::KeepExisting {
            report.skipped_conflicts += 1;
            continue;
        }
        atomic_write(&target, &fs::read(item.path())?)?;
        report.semantic_graph_files_imported += 1;
    }
    Ok(())
}

fn copy_tree_filtered(
    source: &Path,
    destination: &Path,
    semantic_graph_only: bool,
) -> Result<(), PortableError> {
    fs::create_dir_all(destination)?;
    for item in WalkDir::new(source).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(source)
            .map_err(|_| PortableError::InvalidData("invalid source path".to_owned()))?;
        let normalized = relative.to_string_lossy().replace('\\', "/");
        let is_semantic_graph = normalized.starts_with("lore/")
            || normalized.starts_with("rules/")
            || normalized.starts_with("archive/lore/")
            || normalized.starts_with("archive/rules/");
        if is_semantic_graph != semantic_graph_only {
            continue;
        }
        atomic_write(&destination.join(relative), &fs::read(item.path())?)?;
    }
    Ok(())
}

fn known_module_definition(id: &str) -> ModuleDefinition {
    let (path, dependencies, import_order): (&str, &[&str], u32) = match id {
        "config" => ("config", &[], 10),
        "characters" => ("characters", &[], 20),
        "conversations" => ("conversations", &["characters"], 30),
        "memory" => ("memory", &[], 40),
        "semantic_graph" => ("semantic_graph", &[], 50),
        _ => unreachable!("known module definition requested for {id}"),
    };
    ModuleDefinition {
        id: id.to_owned(),
        path: path.to_owned(),
        dependencies: dependencies
            .iter()
            .map(|dependency| (*dependency).to_owned())
            .collect(),
        import_order,
    }
}

fn space_module_definition(module: &str, space_id: Uuid) -> SpaceModuleDefinition {
    SpaceModuleDefinition {
        space_id: space_id.to_string(),
        module: module.to_owned(),
        path: format!("{module}/spaces/{space_id}"),
    }
}

fn is_core_moc_module(id: &str) -> bool {
    matches!(
        id,
        "config"
            | "characters"
            | "conversations"
            | "memory"
            | "semantic_graph"
            | "tavern_compat"
            | "encrypted-container"
    )
}

fn validate_module_id(id: &str) -> Result<(), PortableError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PortableError::InvalidData(format!(
            "invalid host MOC module id: {id}"
        )));
    }
    Ok(())
}

fn validate_host_module(
    module: &HostMocModule,
    module_ids: &mut HashSet<String>,
) -> Result<(), PortableError> {
    validate_module_id(&module.id)?;
    if is_core_moc_module(&module.id) {
        return Err(PortableError::InvalidData(format!(
            "host module cannot replace Core module {}",
            module.id
        )));
    }
    if !module_ids.insert(module.id.clone()) {
        return Err(PortableError::InvalidData(format!(
            "duplicate MOC module id: {}",
            module.id
        )));
    }
    let mut dependencies = HashSet::new();
    for dependency in &module.dependencies {
        validate_module_id(dependency)?;
        if dependency == &module.id || !dependencies.insert(dependency) {
            return Err(PortableError::InvalidData(format!(
                "invalid dependency for host module {}",
                module.id
            )));
        }
    }
    Ok(())
}

fn copy_host_module_tree(source: &Path, destination: &Path) -> Result<(), PortableError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PortableError::InvalidData(format!(
            "host MOC module source must be a regular directory: {}",
            source.display()
        )));
    }
    fs::create_dir_all(destination)?;
    for item in WalkDir::new(source).follow_links(false) {
        let item = item?;
        if item.file_type().is_symlink() {
            return Err(PortableError::InvalidData(format!(
                "host MOC module contains a symbolic link: {}",
                item.path().display()
            )));
        }
        if !item.file_type().is_file() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(source)
            .map_err(|_| PortableError::InvalidData("invalid host module path".to_owned()))?;
        atomic_write(&destination.join(relative), &fs::read(item.path())?)?;
    }
    Ok(())
}

fn claim_unknown_module(
    extracted: &Path,
    claim_directory: &Path,
    module: &UnknownMocModule,
) -> Result<PathBuf, PortableError> {
    validate_module_id(&module.id)?;
    fs::create_dir_all(claim_directory)?;
    let destination = claim_directory.join(&module.id);
    if destination.exists() {
        return Err(PortableError::InvalidData(format!(
            "claimed MOC module destination already exists: {}",
            destination.display()
        )));
    }
    let temporary = TempDir::new_in(claim_directory)?;
    let staged = temporary.path().join(&module.id);
    copy_host_module_tree(&extracted.join(&module.declared_path), &staged)?;
    fs::rename(&staged, &destination)?;
    Ok(destination)
}

const fn default_host_module_import_order() -> u32 {
    1_000
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PortableError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

fn momo_config_path(core: &MomoCore) -> PathBuf {
    core.data_dir().join("config/momo.toml")
}

fn parse_character_id(value: &str) -> Result<Uuid, PortableError> {
    Ok(Uuid::parse_str(
        value.strip_prefix("urn:uuid:").unwrap_or(value),
    )?)
}

fn validate_asset_path(value: &str) -> Result<PathBuf, PortableError> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        return Err(PortableError::InvalidData(format!(
            "unsafe Markdown asset path: {value}"
        )));
    }
    Ok(path.to_path_buf())
}

fn portable_case_fold(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn read_markdown_asset(root: &Path, relative: &Path) -> Result<String, PortableError> {
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(PortableError::InvalidData(
                "unsafe character asset path".to_owned(),
            ));
        };
        cursor.push(component);
        let metadata = fs::symlink_metadata(&cursor)?;
        if metadata.file_type().is_symlink() {
            return Err(PortableError::InvalidData(format!(
                "character asset cannot be a symbolic link: {}",
                relative.display()
            )));
        }
    }
    if !cursor.is_file() {
        return Err(PortableError::InvalidData(format!(
            "character asset is not a regular file: {}",
            relative.display()
        )));
    }
    let bytes = fs::read(&cursor)?;
    if bytes.len() > 200_000 {
        return Err(PortableError::InvalidData(format!(
            "character asset exceeds 200000 bytes: {}",
            relative.display()
        )));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let text = std::str::from_utf8(bytes).map_err(|error| {
        PortableError::InvalidData(format!(
            "character asset is not UTF-8 ({}): {error}",
            relative.display()
        ))
    })?;
    if contains_frontmatter(text) {
        return Err(PortableError::InvalidData(format!(
            "character Markdown must not contain frontmatter: {}",
            relative.display()
        )));
    }
    Ok(text.to_owned())
}

fn read_external_source_asset(path: &Path) -> Result<String, PortableError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > EXTERNAL_SOURCE_MAX_BYTES
    {
        return Err(PortableError::InvalidData(
            "tavern_compat metadata.json has an invalid type or size".to_owned(),
        ));
    }
    let source = fs::read_to_string(path)?;
    serde_json::from_str::<serde_json::Value>(&source).map_err(|error| {
        PortableError::InvalidData(format!("tavern_compat metadata.json is invalid: {error}"))
    })?;
    Ok(source)
}

fn contains_frontmatter(text: &str) -> bool {
    let mut lines = text.lines();
    let Some(marker @ ("---" | "+++")) = lines.next().map(str::trim) else {
        return false;
    };
    lines.any(|line| line.trim() == marker)
}

fn parse_character_metadata(values: &Table) -> Result<ParsedCharacterMetadata, PortableError> {
    let author = values
        .get("author")
        .and_then(TomlValue::as_table)
        .ok_or_else(|| {
            PortableError::InvalidData("character author table is required".to_owned())
        })?;
    if author.contains_key("name") {
        for forbidden in ["description", "language", "tags"] {
            if values.contains_key(forbidden) {
                return Err(PortableError::InvalidData(format!(
                    "MOMO Character Card v2 forbids {forbidden}"
                )));
            }
        }
        if author.contains_key("uid") || author.contains_key("display_name") {
            return Err(PortableError::InvalidData(
                "MOMO Character Card v2 forbids author uid and display_name".to_owned(),
            ));
        }
        let metadata: CharacterMetadata = TomlValue::Table(values.clone())
            .try_into()
            .map_err(|error: toml::de::Error| PortableError::Config(error.into()))?;
        return Ok((
            metadata,
            ConfigDocument::new(values.clone()).to_toml_string()?,
        ));
    }
    Err(PortableError::InvalidData(
        "native MOC character assets require MOMO Character Card v2 metadata".to_owned(),
    ))
}

fn validate_character_metadata(metadata: &CharacterMetadata) -> Result<(), PortableError> {
    if metadata.name.trim().is_empty() || metadata.name.chars().count() > 120 {
        return Err(PortableError::InvalidData(
            "character name must contain between 1 and 120 characters".to_owned(),
        ));
    }
    semver::Version::parse(&metadata.version).map_err(|error| {
        PortableError::InvalidData(format!("character version is not SemVer: {error}"))
    })?;
    if metadata.author.name.trim().is_empty() || metadata.author.name.chars().count() > 200 {
        return Err(PortableError::InvalidData(
            "character author name must contain between 1 and 200 characters".to_owned(),
        ));
    }
    if let Some(url) = metadata.author.url.as_deref() {
        url::Url::parse(url).map_err(|error| {
            PortableError::InvalidData(format!("character author URL is invalid: {error}"))
        })?;
    }
    validate_asset_path(&metadata.character_file)?;
    if let Some(user_file) = metadata.user_file.as_deref() {
        validate_asset_path(user_file)?;
    }
    if let Some(opening_file) = metadata.opening_file.as_deref() {
        validate_asset_path(opening_file)?;
    }
    Ok(())
}

fn read_ddm_profile_asset(
    character_directory: &Path,
    character_id: Uuid,
) -> Result<Option<String>, PortableError> {
    let path = character_directory.join(DDM_PROFILE_ASSET);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 64 * 1024 {
        return Err(PortableError::InvalidData(format!(
            "{DDM_PROFILE_ASSET} must be a non-empty regular file no larger than 64 KiB"
        )));
    }
    let yaml = fs::read_to_string(path)?;
    if yaml.as_bytes().contains(&0) {
        return Err(PortableError::InvalidData(format!(
            "{DDM_PROFILE_ASSET} must not contain NUL bytes"
        )));
    }
    let profile = momo_memory::DdmProfile::parse_yaml(&yaml)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    if profile.character_id != character_id.to_string() {
        return Err(PortableError::InvalidData(format!(
            "DDM profile character_id does not match character {character_id}"
        )));
    }
    Ok(Some(yaml))
}

fn default_character_file() -> String {
    "character.md".to_owned()
}

#[cfg(test)]
#[path = "../tests/unit/portable.rs"]
mod tests;
