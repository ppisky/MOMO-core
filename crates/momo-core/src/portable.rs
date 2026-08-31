use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
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
                import_characters(core, &directory, target_space, mode, &mut report).await?;
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
    import_external_character_sources(core, extracted).await?;
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
    source_config: &Path,
    destination_config: &Path,
) -> Result<(), PortableError> {
    let text = document.to_toml_string()?;
    crate::validate_momo_document(&text)
        .map_err(|error| PortableError::InvalidData(error.to_string()))?;
    let config: crate::MomoConfig =
        toml::from_str(&text).map_err(|error| PortableError::InvalidData(error.to_string()))?;
    let source_base = source_config.parent().unwrap_or_else(|| Path::new("."));
    let destination_base = destination_config
        .parent()
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(destination_base)?;
    let canonical_source_base = fs::canonicalize(source_base).map_err(|error| {
        PortableError::InvalidData(format!(
            "cannot resolve portable config directory {}: {error}",
            source_base.display()
        ))
    })?;
    let canonical_destination_base = fs::canonicalize(destination_base)?;
    for reference in [
        &config.prompts.memory_distillation_file,
        &config.prompts.semantic_graph_governance_file,
    ] {
        let relative = validate_asset_path(&reference.to_string_lossy())?;
        let source = source_base.join(&relative);
        let canonical_source = fs::canonicalize(&source).map_err(|error| {
            PortableError::InvalidData(format!(
                "cannot read referenced maintenance prompt {}: {error}",
                source.display()
            ))
        })?;
        if !canonical_source.starts_with(&canonical_source_base) {
            return Err(PortableError::InvalidData(format!(
                "maintenance prompt escapes the portable config directory: {}",
                reference.display()
            )));
        }
        let metadata = fs::metadata(&canonical_source)?;
        if !metadata.is_file() || metadata.len() > 256 * 1024 {
            return Err(PortableError::InvalidData(format!(
                "maintenance prompt must be a regular file no larger than 262144 bytes: {}",
                reference.display()
            )));
        }
        let bytes = fs::read(&canonical_source)?;
        let prompt = std::str::from_utf8(&bytes).map_err(|error| {
            PortableError::InvalidData(format!(
                "maintenance prompt is not UTF-8 ({}): {error}",
                reference.display()
            ))
        })?;
        if prompt.trim().is_empty() || bytes.contains(&0) {
            return Err(PortableError::InvalidData(format!(
                "maintenance prompt is empty or contains NUL bytes: {}",
                reference.display()
            )));
        }
        let destination = destination_base.join(relative);
        let destination_parent = destination.parent().ok_or_else(|| {
            PortableError::InvalidData("prompt destination has no parent".to_owned())
        })?;
        fs::create_dir_all(destination_parent)?;
        if !fs::canonicalize(destination_parent)?.starts_with(&canonical_destination_base) {
            return Err(PortableError::InvalidData(format!(
                "maintenance prompt destination escapes the portable config directory: {}",
                destination.display()
            )));
        }
        if canonical_source != destination.canonicalize().unwrap_or_default() {
            atomic_write(&destination, &bytes)?;
        }
    }
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
) -> Result<(), PortableError> {
    if !directory.exists() {
        return Ok(());
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
        report.characters_imported += 1;
    }
    Ok(())
}

async fn import_external_character_sources(
    core: &MomoCore,
    root: &Path,
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

fn default_character_file() -> String {
    "character.md".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use momo_domain::{MessageRole, new_id};

    fn write_prompt_bundle(core: &MomoCore) {
        let base = momo_config_path(core)
            .parent()
            .expect("config directory")
            .to_path_buf();
        atomic_write(
            &base.join("prompts/dmw_distiller.md"),
            b"full DMW test prompt",
        )
        .expect("DMW prompt");
        atomic_write(
            &base.join("prompts/nsg_governor.md"),
            b"full NSG test prompt",
        )
        .expect("NSG prompt");
    }

    fn test_export_plan(
        space_id: Uuid,
        modules: &[MocModule],
        character_id: Option<Uuid>,
        compatibility: MocCompatibility,
    ) -> MocExportPlan {
        let selected = modules.iter().copied().collect::<HashSet<_>>();
        MocExportPlan {
            include_config: selected.contains(&MocModule::MomoConfig),
            characters: selected
                .contains(&MocModule::Characters)
                .then(|| MocCharacterSelection {
                    space_id,
                    character_ids: character_id.into_iter().collect(),
                })
                .into_iter()
                .collect(),
            conversations: selected
                .contains(&MocModule::Conversations)
                .then_some(space_id)
                .into_iter()
                .collect(),
            memory: selected
                .contains(&MocModule::Memory)
                .then_some(space_id)
                .into_iter()
                .collect(),
            semantic_graph: selected
                .contains(&MocModule::SemanticGraph)
                .then_some(space_id)
                .into_iter()
                .collect(),
            compatibility,
        }
    }

    fn test_import_plan(source: Uuid, target: Uuid, conflict_mode: ConflictMode) -> MocImportPlan {
        MocImportPlan {
            apply_config: true,
            space_map: [(source, target)].into_iter().collect(),
            conflict_mode,
        }
    }

    #[tokio::test]
    async fn round_trips_selected_moc_modules_and_rebinds_scope() {
        let source_directory = tempfile::tempdir().expect("source directory");
        let source = MomoCore::initialize(source_directory.path())
            .await
            .expect("source core");
        write_prompt_bundle(&source);
        let original_scope = new_id();
        let character_id = new_id();
        let now = Utc::now();
        source
            .store()
            .save_character(&CharacterCard {
                id: character_id,
                scope_id: original_scope,
                name: "雪球".to_owned(),
                version: "2.0.0".to_owned(),
                author_name: "Tester".to_owned(),
                author_url: Some("https://example.com/creator".to_owned()),
                character_markdown: "# Character".to_owned(),
                user_markdown: "# User".to_owned(),
                opening_markdown: Some("Hello".to_owned()),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("character");
        source
            .store()
            .save_portable_metadata(
                "character",
                &character_id.to_string(),
                "future_field = \"preserve-me\"\n",
            )
            .await
            .expect("portable metadata");
        let conversation_id = new_id();
        source
            .store()
            .save_conversation(&Conversation {
                id: conversation_id,
                scope_id: original_scope,
                character_id: Some(character_id),
                title: "Portable".to_owned(),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("conversation");
        source
            .store()
            .append_message(&Message {
                id: new_id(),
                conversation_id,
                role: MessageRole::User,
                content: "hello".to_owned(),
                created_at: now,
            })
            .await
            .expect("message");

        let output = source_directory.path().join("backup.moc");
        let settings = serde_json::json!({
            "schema_version": 1,
            "model_use": { "chat": "primary" },
            "prompts": {
                "memory_distillation_file": "prompts/dmw_distiller.md",
                "semantic_graph_governance_file": "prompts/nsg_governor.md"
            },
            "future": { "preserved": true }
        });
        let manifest = export_moc(
            &source,
            &output,
            &settings,
            &test_export_plan(
                original_scope,
                &[
                    MocModule::MomoConfig,
                    MocModule::Characters,
                    MocModule::Conversations,
                    MocModule::Memory,
                    MocModule::SemanticGraph,
                ],
                None,
                MocCompatibility::None,
            ),
        )
        .await
        .expect("export");
        assert!(
            manifest
                .modules
                .iter()
                .any(|entry| entry.module == "characters")
        );
        assert!(manifest.module_definitions.iter().any(|module| {
            module.id == "conversations" && module.dependencies == ["characters"]
        }));

        let destination_directory = tempfile::tempdir().expect("destination directory");
        let destination = MomoCore::initialize(destination_directory.path())
            .await
            .expect("destination core");
        let new_scope = new_id();
        let report = import_moc(
            &destination,
            &output,
            &test_import_plan(original_scope, new_scope, ConflictMode::Replace),
        )
        .await
        .expect("import");
        assert_eq!(report.characters_imported, 1);
        assert_eq!(report.conversations_imported, 1);
        assert_eq!(report.messages_imported, 1);
        assert!(report.memory_files_imported >= 3);
        assert_eq!(
            report.momo_config.expect("config")["future"]["preserved"],
            true
        );
        assert_eq!(
            fs::read_to_string(
                momo_config_path(&destination)
                    .parent()
                    .expect("destination config directory")
                    .join("prompts/dmw_distiller.md")
            )
            .expect("imported DMW prompt"),
            "full DMW test prompt"
        );
        assert_eq!(
            fs::read_to_string(
                momo_config_path(&destination)
                    .parent()
                    .expect("destination config directory")
                    .join("prompts/nsg_governor.md")
            )
            .expect("imported NSG prompt"),
            "full NSG test prompt"
        );
        let imported_cards = destination.store().list_characters().await.expect("cards");
        assert_eq!(imported_cards[0].scope_id, new_scope);
        assert_eq!(imported_cards[0].opening_markdown.as_deref(), Some("Hello"));

        let second_output = destination_directory.path().join("restored.moc");
        export_moc(
            &destination,
            &second_output,
            &settings,
            &test_export_plan(
                new_scope,
                &[MocModule::Characters],
                None,
                MocCompatibility::None,
            ),
        )
        .await
        .expect("re-export");
        let inspected = tempfile::tempdir().expect("inspect directory");
        momo_moc::extract(
            &second_output,
            inspected.path(),
            ExtractionLimits::default(),
        )
        .expect("inspect MOC");
        let metadata = fs::read_to_string(
            inspected
                .path()
                .join("characters")
                .join("spaces")
                .join(new_scope.to_string())
                .join(character_id.to_string())
                .join("character.toml"),
        )
        .expect("metadata");
        assert!(metadata.contains("future_field = \"preserve-me\""));
        assert!(metadata.contains("opening_file = \"opening.md\""));
        assert_eq!(
            fs::read_to_string(
                inspected
                    .path()
                    .join("characters")
                    .join("spaces")
                    .join(new_scope.to_string())
                    .join(character_id.to_string())
                    .join("opening.md")
            )
            .expect("opening"),
            "Hello"
        );

        let private_output = source_directory.path().join("private.moc");
        export_private_moc(
            &source,
            &private_output,
            &settings,
            &test_export_plan(
                original_scope,
                &[MocModule::MomoConfig],
                None,
                MocCompatibility::None,
            ),
            "private-password",
        )
        .await
        .expect("private export");
        assert!(moc_is_encrypted(&private_output).expect("inspect private"));
        let config_import = MocImportPlan {
            apply_config: true,
            space_map: BTreeMap::new(),
            conflict_mode: ConflictMode::KeepExisting,
        };
        assert!(matches!(
            import_moc_with_passphrase(
                &destination,
                &private_output,
                &config_import,
                Some("wrong-password")
            )
            .await,
            Err(PortableError::Crypto(
                momo_crypto::CryptoError::AuthenticationFailed
            ))
        ));
        let private_report = import_moc_with_passphrase(
            &destination,
            &private_output,
            &config_import,
            Some("private-password"),
        )
        .await
        .expect("private import");
        assert!(private_report.momo_config.is_some());
        assert_eq!(
            destination
                .store()
                .list_conversations()
                .await
                .expect("conversations")[0]
                .scope_id,
            new_scope
        );
    }

    #[tokio::test]
    async fn host_modules_are_explicitly_exported_reported_and_claimed() {
        let source_directory = tempfile::tempdir().expect("source directory");
        let source = MomoCore::initialize(source_directory.path())
            .await
            .expect("source core");
        let extension = source_directory.path().join("weather-module");
        fs::create_dir_all(&extension).expect("extension directory");
        fs::write(
            extension.join("module.json"),
            br#"{"provider":"local-weather"}"#,
        )
        .expect("extension payload");
        let output = source_directory.path().join("extension-only.moc");
        let host_module = HostMocModule {
            id: "weather".to_owned(),
            input_path: extension,
            dependencies: vec!["config".to_owned()],
            import_order: 900,
        };
        let manifest = export_moc_with_host_modules(
            &source,
            &output,
            &serde_json::json!({}),
            &test_export_plan(new_id(), &[], None, MocCompatibility::None),
            std::slice::from_ref(&host_module),
        )
        .await
        .expect("export host module");
        assert_eq!(manifest.module_definitions[0].id, "weather");
        assert_eq!(manifest.module_definitions[0].dependencies, ["config"]);

        let destination_directory = tempfile::tempdir().expect("destination directory");
        let destination = MomoCore::initialize(destination_directory.path())
            .await
            .expect("destination core");
        let host_import = MocImportPlan {
            apply_config: false,
            space_map: BTreeMap::new(),
            conflict_mode: ConflictMode::Replace,
        };
        let report = import_moc(&destination, &output, &host_import)
            .await
            .expect("report unknown module");
        assert_eq!(report.unknown_modules.len(), 1);
        assert_eq!(report.unknown_modules[0].id, "weather");
        assert_eq!(report.unknown_modules[0].claimed_path, None);

        let claims = destination_directory.path().join("claims");
        let report =
            import_moc_claiming_unknown_modules(&destination, &output, &host_import, &claims)
                .await
                .expect("claim unknown module");
        assert_eq!(
            report.unknown_modules[0].claimed_path.as_deref(),
            Some(claims.join("weather").to_string_lossy().as_ref())
        );
        assert_eq!(
            fs::read_to_string(claims.join("weather/module.json")).expect("claimed payload"),
            r#"{"provider":"local-weather"}"#
        );

        let reserved = HostMocModule {
            id: "characters".to_owned(),
            ..host_module
        };
        assert!(matches!(
            export_moc_with_host_modules(
                &source,
                source_directory.path().join("invalid.moc"),
                &serde_json::json!({}),
                &test_export_plan(new_id(), &[], None, MocCompatibility::None),
                &[reserved],
            )
            .await,
            Err(PortableError::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn imports_character_card_v3_without_user_file() {
        let root = tempfile::tempdir().expect("moc root");
        let character_id = new_id();
        let scope_id = new_id();
        let character_dir = root
            .path()
            .join("characters")
            .join("spaces")
            .join(scope_id.to_string())
            .join(character_id.to_string());
        fs::create_dir_all(&character_dir).expect("character directory");
        atomic_write(
            &character_dir.join("character.toml"),
            format!(
                r#"
id = "urn:uuid:{character_id}"
name = "No User Context"
version = "2.0.0"
character_file = "character.md"

[author]
name = "Creator"
"#
            )
            .as_bytes(),
        )
        .expect("metadata");
        atomic_write(&character_dir.join("character.md"), b"# Character").expect("character");
        atomic_write(
            &character_dir
                .parent()
                .expect("character Space")
                .join("index.json"),
            &serde_json::to_vec_pretty(&vec![character_id]).expect("index JSON"),
        )
        .expect("character index");

        let output = root.path().join("optional-user.moc");
        momo_moc::create_from_definitions_and_spaces(
            &output,
            root.path(),
            &[known_module_definition("characters")],
            &[space_module_definition("characters", scope_id)],
        )
        .expect("create moc");

        let target_directory = tempfile::tempdir().expect("target directory");
        let target = MomoCore::initialize(target_directory.path())
            .await
            .expect("target core");
        let report = import_moc(
            &target,
            &output,
            &test_import_plan(scope_id, scope_id, ConflictMode::Replace),
        )
        .await
        .expect("import moc");

        assert_eq!(report.characters_imported, 1);
        let characters = target
            .store()
            .list_characters()
            .await
            .expect("loaded characters");
        let imported = characters
            .iter()
            .find(|character| character.id == character_id)
            .expect("imported character");
        assert_eq!(imported.user_markdown, "");
    }

    #[tokio::test]
    async fn preflight_rejects_late_invalid_payload_before_committing_earlier_modules() {
        let root = tempfile::tempdir().expect("MOC root");
        let space_id = new_id();
        let character_id = new_id();
        let character_space = root
            .path()
            .join("characters/spaces")
            .join(space_id.to_string());
        let character_directory = character_space.join(character_id.to_string());
        fs::create_dir_all(&character_directory).expect("character directory");
        atomic_write(
            &character_directory.join("character.toml"),
            format!(
                r#"id = "urn:uuid:{character_id}"
name = "Preflight"
version = "2.0.0"
character_file = "character.md"

[author]
name = "Tester"
"#
            )
            .as_bytes(),
        )
        .expect("character metadata");
        atomic_write(&character_directory.join("character.md"), b"# Character")
            .expect("character Markdown");
        atomic_write(
            &character_space.join("index.json"),
            &serde_json::to_vec_pretty(&vec![character_id]).expect("character index"),
        )
        .expect("character index");

        let conversation_space = root
            .path()
            .join("conversations/spaces")
            .join(space_id.to_string());
        fs::create_dir_all(&conversation_space).expect("conversation directory");
        let conversation_id = new_id();
        let now = Utc::now();
        atomic_write(
            &conversation_space.join("index.json"),
            &serde_json::to_vec_pretty(&vec![Conversation {
                id: conversation_id,
                scope_id: space_id,
                character_id: Some(character_id),
                title: "Preflight".to_owned(),
                created_at: now,
                updated_at: now,
            }])
            .expect("conversation index"),
        )
        .expect("conversation index");
        atomic_write(
            &conversation_space.join("messages.json"),
            &serde_json::to_vec_pretty(&vec![Message {
                id: new_id(),
                conversation_id: new_id(),
                role: MessageRole::User,
                content: "orphan".to_owned(),
                created_at: now,
            }])
            .expect("messages"),
        )
        .expect("messages");

        let output = root.path().join("invalid-late-module.moc");
        momo_moc::create_from_definitions_and_spaces(
            &output,
            root.path(),
            &[
                known_module_definition("characters"),
                known_module_definition("conversations"),
            ],
            &[
                space_module_definition("characters", space_id),
                space_module_definition("conversations", space_id),
            ],
        )
        .expect("create MOC");

        let destination_directory = tempfile::tempdir().expect("destination directory");
        let destination = MomoCore::initialize(destination_directory.path())
            .await
            .expect("destination Core");
        let error = import_moc(
            &destination,
            &output,
            &test_import_plan(space_id, space_id, ConflictMode::Replace),
        )
        .await
        .expect_err("orphan message must fail preflight");
        assert!(matches!(error, PortableError::InvalidData(_)));
        assert!(
            destination
                .store()
                .list_characters()
                .await
                .expect("characters")
                .is_empty(),
            "a later invalid module must not leave an earlier character committed"
        );
        assert!(
            destination
                .store()
                .list_conversations()
                .await
                .expect("conversations")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn imports_conversation_only_moc_without_missing_character_foreign_key() {
        let source_directory = tempfile::tempdir().expect("source directory");
        let source = MomoCore::initialize(source_directory.path())
            .await
            .expect("source core");
        let scope_id = new_id();
        let character_id = new_id();
        let conversation_id = new_id();
        let now = Utc::now();
        source
            .store()
            .save_character(&CharacterCard {
                id: character_id,
                scope_id,
                name: "Parent card".to_owned(),
                version: "2.0.0".to_owned(),
                author_name: "Tester".to_owned(),
                author_url: None,
                character_markdown: String::new(),
                user_markdown: String::new(),
                opening_markdown: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("character");
        source
            .store()
            .save_conversation(&Conversation {
                id: conversation_id,
                scope_id,
                character_id: Some(character_id),
                title: "Conversation without exported card".to_owned(),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("conversation");

        let output = source_directory.path().join("conversation-only.moc");
        export_moc(
            &source,
            &output,
            &serde_json::json!({}),
            &test_export_plan(
                scope_id,
                &[MocModule::Conversations],
                None,
                MocCompatibility::None,
            ),
        )
        .await
        .expect("conversation-only export");

        let destination_directory = tempfile::tempdir().expect("destination directory");
        let destination = MomoCore::initialize(destination_directory.path())
            .await
            .expect("destination core");
        let target_space = new_id();
        let report = import_moc(
            &destination,
            &output,
            &test_import_plan(scope_id, target_space, ConflictMode::Replace),
        )
        .await
        .expect("conversation-only import");
        assert_eq!(report.conversations_imported, 1);
        assert_eq!(report.messages_imported, 0);
        let imported = destination
            .store()
            .list_conversations()
            .await
            .expect("imported conversations");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].character_id, None);
    }

    #[tokio::test]
    async fn refuses_credentials_in_momo_config() {
        let directory = tempfile::tempdir().expect("directory");
        let core = MomoCore::initialize(directory.path()).await.expect("core");
        write_prompt_bundle(&core);
        let error = export_momo_config(
            &core,
            directory.path().join("unsafe.toml"),
            &serde_json::json!({ "model": { "api_key": "secret" } }),
        )
        .expect_err("credential must be rejected");
        assert!(matches!(error, PortableError::CredentialInConfig(_)));

        let array_error = export_momo_config(
            &core,
            directory.path().join("unsafe-models.toml"),
            &serde_json::json!({
                "models": [{ "profile_id": "profile", "api_key": "secret" }]
            }),
        )
        .expect_err("credential in an array table must be rejected");
        assert!(matches!(array_error, PortableError::CredentialInConfig(_)));
    }

    #[tokio::test]
    async fn momo_config_preserves_product_extensions_and_rejects_host_wiring() {
        let directory = tempfile::tempdir().expect("directory");
        let core = MomoCore::initialize(directory.path()).await.expect("core");
        write_prompt_bundle(&core);
        let output = directory.path().join("portable.toml");
        export_momo_config(
            &core,
            &output,
            &serde_json::json!({
                "schema_version": 1,
                "model_use": { "chat": "primary" },
                "runtime": { "memory_enabled": true },
                "prompts": {
                    "memory_distillation_file": "prompts/dmw_distiller.md",
                    "semantic_graph_governance_file": "prompts/nsg_governor.md"
                },
                "extension": { "preserved": true }
            }),
        )
        .expect("portable export");
        let document = ConfigDocument::load(output).expect("portable document");
        assert!(document.values().contains_key("model_use"));
        assert!(document.values().contains_key("runtime"));
        assert!(document.values().contains_key("extension"));
        assert_eq!(
            fs::read_to_string(directory.path().join("prompts/dmw_distiller.md"))
                .expect("exported DMW prompt"),
            "full DMW test prompt"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("prompts/nsg_governor.md"))
                .expect("exported NSG prompt"),
            "full NSG test prompt"
        );

        let error = export_momo_config(
            &core,
            directory.path().join("host-wiring.toml"),
            &serde_json::json!({
                "schema_version": 1,
                "providers": [{ "provider_id": "not-portable" }]
            }),
        )
        .expect_err("host wiring must be rejected");
        assert!(error.to_string().contains("config.toml"));
    }

    #[tokio::test]
    async fn selected_character_export_and_generated_compatibility_are_explicit() {
        let directory = tempfile::tempdir().expect("directory");
        let core = MomoCore::initialize(directory.path()).await.expect("core");
        let scope_id = new_id();
        let selected_id = new_id();
        let now = Utc::now();
        for (id, name) in [(selected_id, "Selected"), (new_id(), "Other")] {
            core.store()
                .save_character(&CharacterCard {
                    id,
                    scope_id,
                    name: name.to_owned(),
                    version: "2.0.0".to_owned(),
                    author_name: "Tester".to_owned(),
                    author_url: None,
                    character_markdown: "# Character".to_owned(),
                    user_markdown: String::new(),
                    opening_markdown: None,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .expect("character");
        }
        let output = directory.path().join("selected.moc");
        export_moc(
            &core,
            &output,
            &serde_json::json!({}),
            &test_export_plan(
                scope_id,
                &[MocModule::Characters],
                Some(selected_id),
                MocCompatibility::None,
            ),
        )
        .await
        .expect("shortcut export");
        let extracted = tempfile::tempdir().expect("extract directory");
        momo_moc::extract(&output, extracted.path(), ExtractionLimits::default()).expect("extract");
        let character_space = extracted
            .path()
            .join("characters/spaces")
            .join(scope_id.to_string());
        let card_directories = fs::read_dir(&character_space)
            .expect("characters")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .collect::<Vec<_>>();
        assert_eq!(card_directories.len(), 1);
        assert!(
            extracted
                .path()
                .join("characters/spaces")
                .join(scope_id.to_string())
                .join(selected_id.to_string())
                .exists()
        );

        let compatible_output = directory.path().join("selected-compatible.moc");
        export_moc(
            &core,
            &compatible_output,
            &serde_json::json!({}),
            &test_export_plan(
                scope_id,
                &[MocModule::Characters],
                Some(selected_id),
                MocCompatibility::GeneratedCcv2Json,
            ),
        )
        .await
        .expect("generated compatibility export");
        let compatible = tempfile::tempdir().expect("compatible extract directory");
        momo_moc::extract(
            &compatible_output,
            compatible.path(),
            ExtractionLimits::default(),
        )
        .expect("extract compatible MOC");
        assert!(
            compatible
                .path()
                .join("tavern_compat")
                .join(selected_id.to_string())
                .join("generated.ccv2.json")
                .is_file()
        );
        let destination_dir = tempfile::tempdir().expect("destination");
        let destination = MomoCore::initialize(destination_dir.path())
            .await
            .expect("destination core");
        let report = import_moc(
            &destination,
            &compatible_output,
            &test_import_plan(scope_id, new_id(), ConflictMode::Replace),
        )
        .await
        .expect("generated compatibility does not become provenance");
        assert_eq!(report.characters_imported, 1);
    }

    fn valid_character_metadata() -> CharacterMetadata {
        CharacterMetadata {
            id: format!("urn:uuid:{}", new_id()),
            name: "Snowball".to_owned(),
            version: "1.2.3".to_owned(),
            author: CharacterAuthor {
                name: "Creator".to_owned(),
                url: Some("https://example.com".to_owned()),
            },
            character_file: "content/character.md".to_owned(),
            user_file: Some("content/user.md".to_owned()),
            opening_file: Some("content/opening.md".to_owned()),
        }
    }

    #[test]
    fn character_v2_metadata_enforces_semver_author_and_safe_markdown_paths() {
        let mut metadata = valid_character_metadata();
        validate_character_metadata(&metadata).expect("valid metadata");

        metadata.version = "release-one".to_owned();
        assert!(validate_character_metadata(&metadata).is_err());
        metadata.version = "1.0.0".to_owned();
        metadata.author.url = Some("not a URL".to_owned());
        assert!(validate_character_metadata(&metadata).is_err());
        metadata.author.url = None;
        metadata.character_file = "../character.md".to_owned();
        assert!(validate_character_metadata(&metadata).is_err());
        metadata.character_file = "character.md".to_owned();
        metadata.opening_file = Some("opening.txt".to_owned());
        assert!(validate_character_metadata(&metadata).is_err());
    }

    #[test]
    fn legacy_character_metadata_is_rejected_by_native_v2_import() {
        let values = ConfigDocument::parse(&format!(
            r#"
id = "urn:uuid:{}"
name = "Legacy"
version = "1.0.0"
description = "removed"
language = "en"
tags = ["removed"]
future_field = "preserved"
character_file = "character.md"
user_file = "user.md"

[author]
uid = "legacy_uid"
display_name = "Legacy Author"
"#,
            new_id()
        ))
        .expect("legacy metadata");
        assert!(parse_character_metadata(values.values()).is_err());
    }

    #[test]
    fn markdown_asset_accepts_utf8_bom_but_rejects_frontmatter_and_links() {
        let directory = tempfile::tempdir().expect("directory");
        let content = directory.path().join("content");
        fs::create_dir_all(&content).expect("content directory");
        fs::write(content.join("character.md"), b"\xEF\xBB\xBF# Character")
            .expect("write markdown");
        assert_eq!(
            read_markdown_asset(directory.path(), Path::new("content/character.md"))
                .expect("BOM accepted"),
            "# Character"
        );

        fs::write(
            content.join("frontmatter.md"),
            "---\nsecret: true\n---\nbody",
        )
        .expect("write frontmatter");
        assert!(
            read_markdown_asset(directory.path(), Path::new("content/frontmatter.md")).is_err()
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(content.join("character.md"), content.join("linked.md"))
                .expect("symlink");
            assert!(read_markdown_asset(directory.path(), Path::new("content/linked.md")).is_err());
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(
                content.join("character.md"),
                content.join("linked.md"),
            )
            .is_ok()
            {
                assert!(
                    read_markdown_asset(directory.path(), Path::new("content/linked.md")).is_err()
                );
            }
        }
    }
}
