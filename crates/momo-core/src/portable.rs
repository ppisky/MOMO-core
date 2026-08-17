use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use momo_config::ConfigDocument;
use momo_domain::{CharacterCard, Conversation, Message};
use momo_moc::{ExtractionLimits, Manifest};
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
    #[error("unsupported conflict mode: {0}")]
    ConflictMode(String),
    #[error("invalid portable data: {0}")]
    InvalidData(String),
    #[error("directory traversal failed: {0}")]
    Walk(#[from] walkdir::Error),
    #[error("failed to persist data atomically: {0}")]
    Persist(#[from] tempfile::PersistError),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ExportSelection {
    pub config: bool,
    pub characters: bool,
    pub conversations: bool,
    pub memory: bool,
    pub semantic_graph: bool,
    /// Limits character export to one card for the role-directory shortcut.
    pub character_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictMode {
    KeepExisting,
    Replace,
}

impl ConflictMode {
    fn parse(value: &str) -> Result<Self, PortableError> {
        match value {
            "keep_existing" => Ok(Self::KeepExisting),
            "replace" => Ok(Self::Replace),
            _ => Err(PortableError::ConflictMode(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub source_format_version: u32,
    pub migrated_to_format_version: u32,
    pub characters_imported: usize,
    pub conversations_imported: usize,
    pub messages_imported: usize,
    pub memory_files_imported: usize,
    pub semantic_graph_files_imported: usize,
    pub deletions_applied: usize,
    pub skipped_conflicts: usize,
    pub unknown_modules_preserved: Vec<String>,
    pub runtime_config: Option<JsonValue>,
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

pub fn export_runtime_config(
    core: &MomoCore,
    output: impl AsRef<Path>,
    settings: &JsonValue,
) -> Result<(), PortableError> {
    let document = merged_runtime_config(core, settings)?;
    document.save(output)?;
    // Exporting a subset must not turn that subset into the local import
    // baseline. The baseline changes only after an explicit import.
    Ok(())
}

pub fn import_runtime_config(
    core: &MomoCore,
    input: impl AsRef<Path>,
) -> Result<JsonValue, PortableError> {
    let document = ConfigDocument::load(input)?;
    reject_credentials(document.values(), "")?;
    document.save(runtime_config_path(core))?;
    Ok(serde_json::to_value(document.values())?)
}

pub async fn export_moc(
    core: &MomoCore,
    output: impl AsRef<Path>,
    scope_id: Uuid,
    settings: &JsonValue,
    selection: ExportSelection,
) -> Result<Manifest, PortableError> {
    if !selection.config
        && !selection.characters
        && !selection.conversations
        && !selection.memory
        && !selection.semantic_graph
    {
        return Err(PortableError::EmptySelection);
    }
    let staging = TempDir::new()?;
    let mut modules = Vec::new();
    if selection.config {
        merged_runtime_config(core, settings)?.save(staging.path().join("config/runtime.toml"))?;
        modules.push(("config".to_owned(), PathBuf::from("config")));
    }
    if selection.characters {
        export_characters(core, staging.path(), scope_id, selection.character_id).await?;
        modules.push(("characters".to_owned(), PathBuf::from("characters")));
    }
    if selection.conversations {
        export_conversations(core, staging.path(), scope_id).await?;
        modules.push(("conversations".to_owned(), PathBuf::from("conversations")));
    }
    if selection.memory {
        let memory = core.memory_for_scope(scope_id)?;
        copy_tree_filtered(memory.root(), &staging.path().join("memory"), false)?;
        modules.push(("memory".to_owned(), PathBuf::from("memory")));
    }
    if selection.semantic_graph {
        let memory = core.memory_for_scope(scope_id)?;
        copy_tree_filtered(memory.root(), &staging.path().join("semantic_graph"), true)?;
        modules.push(("semantic_graph".to_owned(), PathBuf::from("semantic_graph")));
    }
    Ok(momo_moc::create(output, staging.path(), &modules)?)
}

pub async fn import_moc(
    core: &MomoCore,
    input: impl AsRef<Path>,
    scope_id: Uuid,
    conflict_mode: &str,
) -> Result<ImportReport, PortableError> {
    import_moc_with_passphrase(core, input, scope_id, conflict_mode, None).await
}

pub async fn export_private_moc(
    core: &MomoCore,
    output: impl AsRef<Path>,
    scope_id: Uuid,
    settings: &JsonValue,
    selection: ExportSelection,
    passphrase: &str,
) -> Result<Manifest, PortableError> {
    if passphrase.is_empty() {
        return Err(PortableError::MissingPassphrase);
    }
    let temporary = TempDir::new()?;
    let inner_path = temporary.path().join("payload.moc");
    export_moc(core, &inner_path, scope_id, settings, selection).await?;
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
    scope_id: Uuid,
    conflict_mode: &str,
    passphrase: Option<&str>,
) -> Result<ImportReport, PortableError> {
    let mode = ConflictMode::parse(conflict_mode)?;
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
    let unknown_modules_preserved = payload_manifest
        .module_definitions
        .iter()
        .filter(|module| {
            !matches!(
                module.id.as_str(),
                "config"
                    | "characters"
                    | "conversations"
                    | "memory"
                    | "semantic_graph"
                    | "encrypted-container"
            )
        })
        .map(|module| module.id.clone())
        .collect();
    let mut report = ImportReport {
        source_format_version: payload_manifest.format_version,
        migrated_to_format_version: momo_moc::FORMAT_VERSION,
        characters_imported: 0,
        conversations_imported: 0,
        messages_imported: 0,
        memory_files_imported: 0,
        semantic_graph_files_imported: 0,
        deletions_applied: 0,
        skipped_conflicts: 0,
        unknown_modules_preserved,
        runtime_config: None,
    };
    if payload_manifest
        .modules
        .iter()
        .any(|entry| entry.module == "config")
    {
        let path = extracted.join("config/runtime.toml");
        if path.exists() {
            report.runtime_config = Some(import_runtime_config(core, path)?);
        }
    }
    import_characters(core, extracted, scope_id, mode, &mut report).await?;
    import_conversations(core, extracted, scope_id, mode, &mut report).await?;
    import_memory(core, extracted, scope_id, mode, &mut report)?;
    import_semantic_graph(core, extracted, scope_id, mode, &mut report)?;
    Ok(report)
}

fn merged_runtime_config(
    core: &MomoCore,
    settings: &JsonValue,
) -> Result<ConfigDocument, PortableError> {
    let baseline = runtime_config_path(core);
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
    // Schema v2 makes model and system configuration independently portable.
    // Omitted known sections are intentionally excluded, while unrelated
    // future/extension fields still round-trip from the baseline document.
    if incoming
        .get("schema_version")
        .and_then(TomlValue::as_integer)
        .is_some_and(|version| version >= 2)
    {
        if !incoming.contains_key("server") {
            values.remove("server");
        }
        if !incoming.contains_key("context") {
            values.remove("context");
        }
        if !incoming.contains_key("models") {
            values.remove("models");
            values.remove("active_model_profile");
            values.remove("model");
        }
    }
    merge_table(&mut values, incoming);
    Ok(ConfigDocument::new(values))
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
    character_id: Option<Uuid>,
) -> Result<(), PortableError> {
    let mut characters = core.store().list_characters_for_scope(scope_id).await?;
    if let Some(character_id) = character_id {
        characters.retain(|card| card.id == character_id);
        if characters.is_empty() {
            return Err(PortableError::InvalidData(
                "character does not exist or belongs to another user".to_owned(),
            ));
        }
    }
    let root_directory = root.join("characters");
    fs::create_dir_all(&root_directory)?;
    atomic_write(
        &root_directory.join("index.json"),
        &serde_json::to_vec_pretty(&characters.iter().map(|card| card.id).collect::<Vec<_>>())?,
    )?;
    for card in characters {
        let directory = root_directory.join(card.id.to_string());
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
            .portable_metadata("character", &card.id.to_string())
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
    let directory = root.join("conversations");
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
    root: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    let directory = root.join("characters");
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

async fn import_conversations(
    core: &MomoCore,
    root: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    let directory = root.join("conversations");
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
    root: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    let source = root.join("memory");
    if !source.exists() {
        return Ok(());
    }
    for item in WalkDir::new(&source).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(&source)
            .map_err(|_| PortableError::InvalidData("invalid memory path".to_owned()))?;
        let target = core.memory_for_scope(scope_id)?.root().join(relative);
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
    root: &Path,
    scope_id: Uuid,
    mode: ConflictMode,
    report: &mut ImportReport,
) -> Result<(), PortableError> {
    let source = root.join("semantic_graph");
    if !source.exists() {
        return Ok(());
    }
    import_workspace_tree(core, &source, scope_id, mode, report)
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
        let target = core.memory_for_scope(scope_id)?.root().join(relative);
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PortableError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

fn runtime_config_path(core: &MomoCore) -> PathBuf {
    core.data_dir().join("config/runtime.toml")
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
        "native MOC v2 requires MOMO Character Card v2 metadata".to_owned(),
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

    #[tokio::test]
    async fn round_trips_selected_moc_modules_and_rebinds_scope() {
        let source_directory = tempfile::tempdir().expect("source directory");
        let source = MomoCore::initialize(source_directory.path())
            .await
            .expect("source core");
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
            "model": { "base_url": "https://example.com/v1", "id": "model" },
            "future": { "preserved": true }
        });
        let manifest = export_moc(
            &source,
            &output,
            original_scope,
            &settings,
            ExportSelection {
                config: true,
                characters: true,
                conversations: true,
                memory: true,
                semantic_graph: true,
                character_id: None,
            },
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
        let report = import_moc(&destination, &output, new_scope, "replace")
            .await
            .expect("import");
        assert_eq!(report.characters_imported, 1);
        assert_eq!(report.conversations_imported, 1);
        assert_eq!(report.messages_imported, 1);
        assert!(report.memory_files_imported >= 3);
        assert_eq!(
            report.runtime_config.expect("config")["future"]["preserved"],
            true
        );
        let imported_cards = destination.store().list_characters().await.expect("cards");
        assert_eq!(imported_cards[0].scope_id, new_scope);
        assert_eq!(imported_cards[0].opening_markdown.as_deref(), Some("Hello"));

        let second_output = destination_directory.path().join("restored.moc");
        export_moc(
            &destination,
            &second_output,
            new_scope,
            &settings,
            ExportSelection {
                config: false,
                characters: true,
                conversations: false,
                memory: false,
                semantic_graph: false,
                character_id: None,
            },
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
            original_scope,
            &settings,
            ExportSelection {
                config: true,
                characters: false,
                conversations: false,
                memory: false,
                semantic_graph: false,
                character_id: None,
            },
            "private-password",
        )
        .await
        .expect("private export");
        assert!(moc_is_encrypted(&private_output).expect("inspect private"));
        assert!(matches!(
            import_moc_with_passphrase(
                &destination,
                &private_output,
                new_scope,
                "keep_existing",
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
            new_scope,
            "keep_existing",
            Some("private-password"),
        )
        .await
        .expect("private import");
        assert!(private_report.runtime_config.is_some());
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
    async fn imports_character_card_v2_without_user_file() {
        let root = tempfile::tempdir().expect("moc root");
        let character_id = new_id();
        let character_dir = root
            .path()
            .join("characters")
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

        let output = root.path().join("optional-user.moc");
        momo_moc::create(
            &output,
            root.path(),
            &[("characters".to_owned(), PathBuf::from("characters"))],
        )
        .expect("create moc");

        let target_directory = tempfile::tempdir().expect("target directory");
        let target = MomoCore::initialize(target_directory.path())
            .await
            .expect("target core");
        let scope_id = new_id();
        let report = import_moc(&target, &output, scope_id, "replace")
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
            scope_id,
            &serde_json::json!({}),
            ExportSelection {
                config: false,
                characters: false,
                conversations: true,
                memory: false,
                semantic_graph: false,
                character_id: None,
            },
        )
        .await
        .expect("conversation-only export");

        let destination_directory = tempfile::tempdir().expect("destination directory");
        let destination = MomoCore::initialize(destination_directory.path())
            .await
            .expect("destination core");
        let report = import_moc(&destination, &output, new_id(), "replace")
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
    async fn refuses_credentials_in_runtime_config() {
        let directory = tempfile::tempdir().expect("directory");
        let core = MomoCore::initialize(directory.path()).await.expect("core");
        let error = export_runtime_config(
            &core,
            directory.path().join("unsafe.toml"),
            &serde_json::json!({ "model": { "api_key": "secret" } }),
        )
        .expect_err("credential must be rejected");
        assert!(matches!(error, PortableError::CredentialInConfig(_)));

        let array_error = export_runtime_config(
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
    async fn schema_v2_exports_model_and_system_sections_independently() {
        let directory = tempfile::tempdir().expect("directory");
        let core = MomoCore::initialize(directory.path()).await.expect("core");
        export_runtime_config(
            &core,
            directory.path().join("full.toml"),
            &serde_json::json!({
                "schema_version": 2,
                "server": { "base_url": "http://localhost:8080" },
                "context": { "window": 8192 },
                "active_model_profile": "00000000-0000-7000-8000-000000000002",
                "models": [{
                    "profile_id": "00000000-0000-7000-8000-000000000002",
                    "name": "Default",
                    "base_url": "https://example.com/v1",
                    "id": "model"
                }],
                "extension": { "preserved": true }
            }),
        )
        .expect("full export");
        import_runtime_config(&core, directory.path().join("full.toml"))
            .expect("establish imported baseline");

        let model_only = directory.path().join("model-only.toml");
        export_runtime_config(
            &core,
            &model_only,
            &serde_json::json!({
                "schema_version": 2,
                "active_model_profile": "00000000-0000-7000-8000-000000000002",
                "models": [{
                    "profile_id": "00000000-0000-7000-8000-000000000002",
                    "name": "Default",
                    "base_url": "https://example.com/v1",
                    "id": "model"
                }]
            }),
        )
        .expect("model-only export");
        let document = ConfigDocument::load(model_only).expect("model-only document");
        assert!(document.values().contains_key("models"));
        assert!(!document.values().contains_key("server"));
        assert!(!document.values().contains_key("context"));
        assert!(document.values().contains_key("extension"));
    }

    #[tokio::test]
    async fn character_shortcut_exports_only_the_selected_card() {
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
            scope_id,
            &serde_json::json!({}),
            ExportSelection {
                config: false,
                characters: true,
                conversations: false,
                memory: false,
                semantic_graph: false,
                character_id: Some(selected_id),
            },
        )
        .await
        .expect("shortcut export");
        let extracted = tempfile::tempdir().expect("extract directory");
        momo_moc::extract(&output, extracted.path(), ExtractionLimits::default()).expect("extract");
        let card_directories = fs::read_dir(extracted.path().join("characters"))
            .expect("characters")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .collect::<Vec<_>>();
        assert_eq!(card_directories.len(), 1);
        assert!(
            extracted
                .path()
                .join("characters")
                .join(selected_id.to_string())
                .exists()
        );
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
