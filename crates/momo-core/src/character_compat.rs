//! Import and export adapters for the external Character Card v1/v2/v3 formats.

use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use crc32fast::Hasher;
use momo_domain::CharacterCard;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::MomoCore;

const MAX_JSON_BYTES: u64 = 10 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHARX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CHARX_ENTRY_BYTES: u64 = 50 * 1024 * 1024;
const MAX_CHARX_EXPANDED_BYTES: u64 = 200 * 1024 * 1024;
const MAX_CHARX_ENTRIES: usize = 10_000;
const EXTERNAL_METADATA_CATEGORY: &str = "external_character_card";
const SOURCE_DIRECTORY: &str = "character-packages";

#[derive(Debug, Error)]
pub enum CharacterCompatError {
    #[error("character-card I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("character-card JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("character-card storage failed: {0}")]
    Storage(#[from] momo_storage::StorageError),
    #[error("CHARX archive is invalid: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid external character card: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalCharacterImportFormat {
    Ccv1Json,
    Ccv1Png,
    Ccv2Json,
    Ccv2Png,
    Ccv3Json,
    Ccv3Png,
    Ccv3Charx,
}

impl ExternalCharacterImportFormat {
    const fn major(self) -> u8 {
        match self {
            Self::Ccv1Json | Self::Ccv1Png | Self::Ccv2Json | Self::Ccv2Png => 2,
            Self::Ccv3Json | Self::Ccv3Png | Self::Ccv3Charx => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalCharacterExportFormat {
    Ccv2Json,
    Ccv3Json,
    Ccv3Charx,
}

impl std::str::FromStr for ExternalCharacterExportFormat {
    type Err = CharacterCompatError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ccv2_json" => Ok(Self::Ccv2Json),
            "ccv3_json" => Ok(Self::Ccv3Json),
            "ccv3_charx" => Ok(Self::Ccv3Charx),
            _ => Err(CharacterCompatError::Invalid(format!(
                "unsupported export format {value:?}; expected ccv2_json, ccv3_json, or ccv3_charx"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredExternalCharacter {
    source_format: ExternalCharacterImportFormat,
    card: Value,
    warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<StoredSourceFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    charx: Option<StoredCharxPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredSourceFile {
    sha256: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredCharxPackage {
    schema_version: u8,
    archive_sha256: String,
    entry_count: usize,
    asset_count: usize,
    has_x_meta: bool,
    has_module_risum: bool,
    jpeg_zip_hybrid: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalCharacterImport {
    pub character: CharacterCard,
    pub source_format: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreservedCharacterSourceExport {
    pub character_id: Uuid,
    pub source_format: String,
    pub output_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
struct ParsedExternalCharacter {
    format: ExternalCharacterImportFormat,
    card: Value,
    name: String,
    author_name: String,
    version: String,
    character_markdown: String,
    opening_markdown: Option<String>,
    warnings: Vec<String>,
    charx: Option<ParsedCharxPackage>,
}

#[derive(Debug)]
struct ParsedCharxPackage {
    bytes: Vec<u8>,
    card_json: Vec<u8>,
    info: StoredCharxPackage,
    warnings: Vec<String>,
}

pub async fn import_external_character(
    core: &MomoCore,
    scope_id: Uuid,
    input_path: impl AsRef<Path>,
    format: ExternalCharacterImportFormat,
) -> Result<ExternalCharacterImport, CharacterCompatError> {
    let input_path = input_path.as_ref();
    let parsed = parse_external_path(input_path, format)?;
    let source_bytes = fs::read(input_path)?;
    let now = Utc::now();
    let character = CharacterCard {
        id: momo_domain::new_id(),
        scope_id,
        name: parsed.name,
        version: parsed.version,
        author_name: parsed.author_name,
        author_url: None,
        character_markdown: parsed.character_markdown,
        user_markdown: String::new(),
        opening_markdown: parsed.opening_markdown,
        created_at: now,
        updated_at: now,
    };
    core.store().stage_character(&character).await?;
    save_preserved_source(core, character.id, parsed.format, &source_bytes)?;
    let stored = StoredExternalCharacter {
        source_format: parsed.format,
        card: parsed.card,
        warnings: parsed.warnings.clone(),
        source: Some(StoredSourceFile {
            sha256: hex::encode(Sha256::digest(&source_bytes)),
            size: source_bytes.len() as u64,
        }),
        charx: parsed.charx.as_ref().map(|package| package.info.clone()),
    };
    core.store()
        .save_portable_metadata(
            EXTERNAL_METADATA_CATEGORY,
            &character.id.to_string(),
            &serde_json::to_string(&stored)?,
        )
        .await?;
    Ok(ExternalCharacterImport {
        character,
        source_format: source_format_name(parsed.format).to_owned(),
        warnings: parsed.warnings,
    })
}

pub async fn export_preserved_character_source(
    core: &MomoCore,
    scope_id: Uuid,
    character_id: Uuid,
    output_path: impl AsRef<Path>,
) -> Result<PreservedCharacterSourceExport, CharacterCompatError> {
    if !core
        .store()
        .list_characters_for_scope(scope_id)
        .await?
        .iter()
        .any(|character| character.id == character_id)
    {
        return Err(CharacterCompatError::Invalid(
            "character does not exist in the requested scope".to_owned(),
        ));
    }
    let stored = load_stored_external_character(core, character_id).await?;
    let bytes = load_preserved_source(core, character_id, &stored)?;
    atomic_write_bytes(output_path.as_ref(), &bytes)?;
    let source = stored.source.ok_or_else(|| {
        CharacterCompatError::Invalid(
            "the imported character predates exact source preservation".to_owned(),
        )
    })?;
    Ok(PreservedCharacterSourceExport {
        character_id,
        source_format: source_format_name(stored.source_format).to_owned(),
        output_path: output_path.as_ref().to_path_buf(),
        bytes: source.size,
        sha256: source.sha256,
    })
}

pub fn validate_external_charx(input_path: impl AsRef<Path>) -> Result<(), CharacterCompatError> {
    let input_path = input_path.as_ref();
    let metadata = fs::symlink_metadata(input_path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CharacterCompatError::Invalid(
            "CHARX input must be a regular file".to_owned(),
        ));
    }
    ensure_size(metadata.len(), MAX_CHARX_BYTES, "CHARX")?;
    parse_charx_package(fs::read(input_path)?)?;
    Ok(())
}

pub async fn export_external_character(
    core: &MomoCore,
    scope_id: Uuid,
    character_id: Uuid,
    output_path: impl AsRef<Path>,
    format: ExternalCharacterExportFormat,
) -> Result<Value, CharacterCompatError> {
    let character = core
        .store()
        .list_characters_for_scope(scope_id)
        .await?
        .into_iter()
        .find(|character| character.id == character_id)
        .ok_or_else(|| {
            CharacterCompatError::Invalid(
                "character does not exist in the requested scope".to_owned(),
            )
        })?;
    let stored = core
        .store()
        .portable_metadata(EXTERNAL_METADATA_CATEGORY, &character_id.to_string())
        .await?
        .map(|value| serde_json::from_str::<StoredExternalCharacter>(&value))
        .transpose()?;
    let preserved_source = stored.as_ref().is_some_and(|source| {
        matches!(
            (source.source_format, format),
            (
                ExternalCharacterImportFormat::Ccv2Json | ExternalCharacterImportFormat::Ccv2Png,
                ExternalCharacterExportFormat::Ccv2Json
            ) | (
                ExternalCharacterImportFormat::Ccv3Json
                    | ExternalCharacterImportFormat::Ccv3Png
                    | ExternalCharacterImportFormat::Ccv3Charx,
                ExternalCharacterExportFormat::Ccv3Json | ExternalCharacterExportFormat::Ccv3Charx
            )
        )
    });
    let source = preserved_source
        .then(|| stored.as_ref().map(|source| source.card.clone()))
        .flatten();
    let preserved_source_assets = match format {
        ExternalCharacterExportFormat::Ccv2Json => {
            let value = export_ccv2(&character, source)?;
            atomic_write_json(output_path.as_ref(), &value)?;
            false
        }
        ExternalCharacterExportFormat::Ccv3Json => {
            let value = export_ccv3(&character, source)?;
            atomic_write_json(output_path.as_ref(), &value)?;
            false
        }
        ExternalCharacterExportFormat::Ccv3Charx => {
            let value = export_ccv3(&character, source)?;
            let source_archive = stored
                .as_ref()
                .and_then(|stored| stored.charx.as_ref())
                .map(|info| load_charx_source(core, character_id, info))
                .transpose()?;
            write_charx(output_path.as_ref(), &value, source_archive.as_deref())?;
            source_archive.is_some()
        }
    };
    Ok(json!({
        "character_id": character_id,
        "format": match format {
            ExternalCharacterExportFormat::Ccv2Json => "ccv2_json",
            ExternalCharacterExportFormat::Ccv3Json => "ccv3_json",
            ExternalCharacterExportFormat::Ccv3Charx => "ccv3_charx",
        },
        "output_path": output_path.as_ref(),
        "preserved_source_fields": preserved_source,
        "preserved_source_assets": preserved_source_assets,
    }))
}

fn parse_external_path(
    path: &Path,
    expected_format: ExternalCharacterImportFormat,
) -> Result<ParsedExternalCharacter, CharacterCompatError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(CharacterCompatError::Invalid(
            "input path is not a regular file".to_owned(),
        ));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let parsed = match extension.as_str() {
        "json" => {
            ensure_size(metadata.len(), MAX_JSON_BYTES, "JSON")?;
            parse_external_json(&fs::read(path)?, None)
        }
        "png" => {
            ensure_size(metadata.len(), MAX_IMAGE_BYTES, "PNG")?;
            let (json, chunk) = extract_png_card(&fs::read(path)?)?;
            let hinted = match chunk {
                PngCardChunk::Ccv3 => Some(ExternalCharacterImportFormat::Ccv3Png),
                PngCardChunk::Chara => Some(ExternalCharacterImportFormat::Ccv2Png),
            };
            parse_external_json(&json, hinted)
        }
        "charx" => {
            ensure_size(metadata.len(), MAX_CHARX_BYTES, "CHARX")?;
            let package = read_charx_package(path)?;
            let mut parsed = parse_external_json(
                &package.card_json,
                Some(ExternalCharacterImportFormat::Ccv3Charx),
            )?;
            parsed.warnings.extend(package.warnings.clone());
            parsed.charx = Some(package);
            Ok(parsed)
        }
        _ => Err(CharacterCompatError::Invalid(format!(
            "unsupported input extension {extension:?}; expected json, png, or charx"
        ))),
    }?;
    if parsed.format != expected_format {
        return Err(CharacterCompatError::Invalid(format!(
            "declared import format {} does not match detected format {}",
            source_format_name(expected_format),
            source_format_name(parsed.format)
        )));
    }
    Ok(parsed)
}

fn ensure_size(actual: u64, limit: u64, label: &str) -> Result<(), CharacterCompatError> {
    if actual == 0 || actual > limit {
        return Err(CharacterCompatError::Invalid(format!(
            "{label} input size must be between 1 and {limit} bytes"
        )));
    }
    Ok(())
}

fn parse_external_json(
    bytes: &[u8],
    hinted_format: Option<ExternalCharacterImportFormat>,
) -> Result<ParsedExternalCharacter, CharacterCompatError> {
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(CharacterCompatError::Invalid(
            "embedded character JSON exceeds 2 MiB".to_owned(),
        ));
    }
    let card: Value = serde_json::from_slice(bytes)?;
    let root = card
        .as_object()
        .ok_or_else(|| CharacterCompatError::Invalid("card JSON must be an object".to_owned()))?;
    let spec = root.get("spec").and_then(Value::as_str);
    let detected = match spec {
        Some("chara_card_v2") => 2,
        Some("chara_card_v3") => 3,
        None => 1,
        Some(other) => {
            return Err(CharacterCompatError::Invalid(format!(
                "unsupported character-card spec {other:?}"
            )));
        }
    };
    let mut warnings = Vec::new();
    if let Some(hint) = hinted_format
        && ((hint.major() == 3) != (detected == 3))
    {
        return Err(CharacterCompatError::Invalid(
            "embedded chunk format does not match card spec".to_owned(),
        ));
    }
    if detected > 1 {
        let spec_version = required_string(root, "spec_version")?;
        if detected == 2 {
            if spec_version != "2.0" {
                return Err(CharacterCompatError::Invalid(format!(
                    "CCv2 spec_version must be \"2.0\", received {spec_version:?}"
                )));
            }
        } else {
            let numeric_version = spec_version.parse::<f64>().map_err(|_| {
                CharacterCompatError::Invalid(format!(
                    "CCv3 spec_version {spec_version:?} must be a decimal number"
                ))
            })?;
            if !numeric_version.is_finite() || !(3.0..4.0).contains(&numeric_version) {
                return Err(CharacterCompatError::Invalid(format!(
                    "CCv3 spec_version {spec_version:?} must be at least 3.0 and lower than 4.0"
                )));
            }
            if numeric_version > 3.0 {
                warnings.push(format!(
                    "CCv3 spec_version {spec_version:?} is newer than the implemented 3.0 profile; unknown fields will be preserved"
                ));
            }
        }
    }
    let data = if detected == 1 {
        root
    } else {
        root.get("data").and_then(Value::as_object).ok_or_else(|| {
            CharacterCompatError::Invalid("card data must be an object".to_owned())
        })?
    };
    let name = required_string(data, "name")?.trim().to_owned();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(CharacterCompatError::Invalid(
            "card name must contain 1 to 120 characters".to_owned(),
        ));
    }
    let description = required_string(data, "description")?;
    let personality = required_string(data, "personality")?;
    let scenario = required_string(data, "scenario")?;
    let first_message = required_string(data, "first_mes")?;
    let examples = required_string(data, "mes_example")?;
    let author_name = if detected == 1 {
        "Unknown".to_owned()
    } else {
        required_string(data, "creator")?.trim().to_owned()
    };
    let author_name = if author_name.is_empty() {
        "Unknown".to_owned()
    } else {
        author_name
    };
    let raw_version = if detected == 1 {
        "1.0.0"
    } else {
        required_string(data, "character_version")?
    };
    let version = if semver::Version::parse(raw_version).is_ok() {
        raw_version.to_owned()
    } else {
        warnings.push(format!(
            "external character_version {raw_version:?} is not SemVer; MOMO version was set to 1.0.0"
        ));
        "1.0.0".to_owned()
    };
    if detected > 1 && has_external_only_content(data) {
        warnings.push(
            "external-only runtime, catalog, lorebook, extension, or asset fields were preserved as source metadata"
                .to_owned(),
        );
    }
    let base_format = match detected {
        1 => ExternalCharacterImportFormat::Ccv1Json,
        2 => ExternalCharacterImportFormat::Ccv2Json,
        _ => ExternalCharacterImportFormat::Ccv3Json,
    };
    let format = match (hinted_format, detected) {
        (Some(ExternalCharacterImportFormat::Ccv2Png), 1) => ExternalCharacterImportFormat::Ccv1Png,
        (Some(hint), _) => hint,
        (None, _) => base_format,
    };
    let character_markdown =
        render_character_markdown(&name, description, personality, scenario, examples);
    let opening_markdown = (!first_message.is_empty()).then(|| first_message.to_owned());
    Ok(ParsedExternalCharacter {
        format,
        card,
        name: name.clone(),
        author_name,
        version,
        character_markdown,
        opening_markdown,
        warnings,
        charx: None,
    })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, CharacterCompatError> {
    object.get(field).and_then(Value::as_str).ok_or_else(|| {
        CharacterCompatError::Invalid(format!("card field {field:?} must be a string"))
    })
}

fn has_external_only_content(data: &Map<String, Value>) -> bool {
    [
        "system_prompt",
        "post_history_instructions",
        "alternate_greetings",
        "character_book",
        "tags",
        "creator_notes",
        "extensions",
        "assets",
        "group_only_greetings",
        "source",
    ]
    .iter()
    .any(|field| data.get(*field).is_some_and(value_has_content))
}

fn value_has_content(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(value) => *value,
        Value::Number(_) => true,
    }
}

fn render_character_markdown(
    name: &str,
    description: &str,
    personality: &str,
    scenario: &str,
    examples: &str,
) -> String {
    let mut output = format!("# {name}");
    append_markdown_section(&mut output, None, description);
    append_markdown_section(&mut output, Some("Personality"), personality);
    append_markdown_section(&mut output, Some("Scenario"), scenario);
    append_markdown_section(&mut output, Some("Dialogue examples"), examples);
    output.push('\n');
    output
}

fn append_markdown_section(output: &mut String, heading: Option<&str>, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    output.push_str("\n\n");
    if let Some(heading) = heading {
        output.push_str("## ");
        output.push_str(heading);
        output.push_str("\n\n");
    }
    output.push_str(value.trim());
}

#[derive(Debug)]
enum PngCardChunk {
    Chara,
    Ccv3,
}

fn extract_png_card(bytes: &[u8]) -> Result<(Vec<u8>, PngCardChunk), CharacterCompatError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return Err(CharacterCompatError::Invalid(
            "PNG signature is invalid".to_owned(),
        ));
    }
    let mut cursor = SIGNATURE.len();
    let mut chara = None;
    let mut ccv3 = None;
    while cursor < bytes.len() {
        if bytes.len().saturating_sub(cursor) < 12 {
            return Err(CharacterCompatError::Invalid(
                "PNG chunk header is truncated".to_owned(),
            ));
        }
        let length =
            u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().expect("four bytes")) as usize;
        let chunk_end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| CharacterCompatError::Invalid("PNG chunk size overflow".to_owned()))?;
        if chunk_end > bytes.len() {
            return Err(CharacterCompatError::Invalid(
                "PNG chunk is truncated".to_owned(),
            ));
        }
        let chunk_type = &bytes[cursor + 4..cursor + 8];
        let data = &bytes[cursor + 8..cursor + 8 + length];
        let expected_crc = u32::from_be_bytes(
            bytes[cursor + 8 + length..chunk_end]
                .try_into()
                .expect("four bytes"),
        );
        let mut hasher = Hasher::new();
        hasher.update(chunk_type);
        hasher.update(data);
        if hasher.finalize() != expected_crc {
            return Err(CharacterCompatError::Invalid(
                "PNG chunk CRC is invalid".to_owned(),
            ));
        }
        if chunk_type == b"acTL" {
            return Err(CharacterCompatError::Invalid(
                "APNG character cards are not supported; select a static PNG frame explicitly"
                    .to_owned(),
            ));
        }
        if chunk_type == b"tEXt"
            && let Some(separator) = data.iter().position(|byte| *byte == 0)
        {
            let keyword = &data[..separator];
            let encoded = &data[separator + 1..];
            if keyword == b"ccv3" {
                ccv3 = Some(decode_png_json(encoded)?);
            } else if keyword == b"chara" || keyword == b"Chara" {
                chara = Some(decode_png_json(encoded)?);
            }
        }
        cursor = chunk_end;
        if chunk_type == b"IEND" {
            break;
        }
    }
    if let Some(json) = ccv3 {
        return Ok((json, PngCardChunk::Ccv3));
    }
    if let Some(json) = chara {
        return Ok((json, PngCardChunk::Chara));
    }
    Err(CharacterCompatError::Invalid(
        "PNG does not contain a ccv3 or chara tEXt chunk".to_owned(),
    ))
}

fn decode_png_json(encoded: &[u8]) -> Result<Vec<u8>, CharacterCompatError> {
    let decoded = BASE64.decode(encoded).map_err(|error| {
        CharacterCompatError::Invalid(format!("PNG character chunk is not valid base64: {error}"))
    })?;
    if decoded.len() as u64 > MAX_JSON_BYTES {
        return Err(CharacterCompatError::Invalid(
            "PNG character JSON exceeds 2 MiB".to_owned(),
        ));
    }
    Ok(decoded)
}

fn read_charx_package(path: &Path) -> Result<ParsedCharxPackage, CharacterCompatError> {
    let bytes = fs::read(path)?;
    parse_charx_package(bytes)
}

fn parse_charx_package(bytes: Vec<u8>) -> Result<ParsedCharxPackage, CharacterCompatError> {
    ensure_size(bytes.len() as u64, MAX_CHARX_BYTES, "CHARX")?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice()))?;
    if archive.len() > MAX_CHARX_ENTRIES {
        return Err(CharacterCompatError::Invalid(format!(
            "CHARX contains more than {MAX_CHARX_ENTRIES} entries"
        )));
    }
    let archive_offset = usize::try_from(archive.offset()).map_err(|_| {
        CharacterCompatError::Invalid("CHARX archive offset is not representable".to_owned())
    })?;
    let jpeg_zip_hybrid = archive_offset > 0;
    if jpeg_zip_hybrid && !valid_jpeg_prefix(&bytes[..archive_offset]) {
        return Err(CharacterCompatError::Invalid(
            "bytes before the CHARX ZIP are not a complete JPEG preview".to_owned(),
        ));
    }

    let entry_count = archive.len();
    let mut seen = HashSet::new();
    let mut entry_paths = HashSet::new();
    let mut card_json = None;
    let mut asset_count = 0;
    let mut has_x_meta = false;
    let mut has_module_risum = false;
    let mut expanded_bytes = 0_u64;
    let mut non_ascii_path = false;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        if !seen.insert(name.clone()) {
            return Err(CharacterCompatError::Invalid(format!(
                "CHARX contains duplicate entry {name:?}"
            )));
        }
        if entry.enclosed_name().is_none() || name.contains('\\') {
            return Err(CharacterCompatError::Invalid(format!(
                "CHARX contains unsafe entry path {name:?}"
            )));
        }
        if entry.encrypted() {
            return Err(CharacterCompatError::Invalid(format!(
                "CHARX entry {name:?} is encrypted"
            )));
        }
        if entry.is_symlink() {
            return Err(CharacterCompatError::Invalid(format!(
                "CHARX entry {name:?} is a symbolic link"
            )));
        }
        if !name.is_ascii() {
            non_ascii_path = true;
        }
        if entry.is_dir() {
            if entry.size() != 0 {
                return Err(CharacterCompatError::Invalid(format!(
                    "CHARX directory entry {name:?} contains data"
                )));
            }
            continue;
        }
        if !entry.is_file() {
            return Err(CharacterCompatError::Invalid(format!(
                "CHARX entry {name:?} is not a regular file"
            )));
        }
        let entry_limit = if name == "card.json" {
            MAX_JSON_BYTES
        } else {
            MAX_CHARX_ENTRY_BYTES
        };
        if entry.size() > entry_limit || (name == "card.json" && entry.size() == 0) {
            return Err(CharacterCompatError::Invalid(format!(
                "CHARX entry {name:?} has an invalid expanded size"
            )));
        }
        let data = read_bounded_zip_entry(&mut entry, entry_limit, &name)?;
        expanded_bytes = expanded_bytes
            .checked_add(data.len() as u64)
            .filter(|total| *total <= MAX_CHARX_EXPANDED_BYTES)
            .ok_or_else(|| {
                CharacterCompatError::Invalid(format!(
                    "CHARX expands beyond {MAX_CHARX_EXPANDED_BYTES} bytes"
                ))
            })?;
        entry_paths.insert(name.clone());
        match name.as_str() {
            "card.json" => card_json = Some(data),
            "module.risum" => has_module_risum = true,
            _ => {
                asset_count += usize::from(name.starts_with("assets/"));
                has_x_meta |= name.starts_with("x_meta/") && name.ends_with(".json");
            }
        }
    }

    let card_json = card_json.ok_or_else(|| {
        CharacterCompatError::Invalid(
            "CHARX must contain exactly one card.json file at the ZIP root".to_owned(),
        )
    })?;
    let mut warnings = embedded_asset_warnings(&card_json, &entry_paths)?;
    if non_ascii_path {
        warnings.push(
            "CHARX contains non-ASCII entry paths; they were preserved, but the CCv3 profile recommends ASCII paths"
                .to_owned(),
        );
    }
    let archive_sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(ParsedCharxPackage {
        bytes,
        card_json,
        info: StoredCharxPackage {
            schema_version: 1,
            archive_sha256,
            entry_count,
            asset_count,
            has_x_meta,
            has_module_risum,
            jpeg_zip_hybrid,
        },
        warnings,
    })
}

fn read_bounded_zip_entry(
    entry: &mut zip::read::ZipFile<'_, impl Read>,
    limit: u64,
    name: &str,
) -> Result<Vec<u8>, CharacterCompatError> {
    let mut output = Vec::new();
    entry
        .take(limit.saturating_add(1))
        .read_to_end(&mut output)?;
    if output.len() as u64 > limit {
        return Err(CharacterCompatError::Invalid(format!(
            "CHARX entry {name:?} exceeds {limit} expanded bytes"
        )));
    }
    Ok(output)
}

fn valid_jpeg_prefix(prefix: &[u8]) -> bool {
    prefix.starts_with(b"\xFF\xD8\xFF")
        && prefix
            .windows(2)
            .rposition(|window| window == b"\xFF\xD9")
            .is_some_and(|end| end + 2 == prefix.len())
}

fn embedded_asset_warnings(
    card_json: &[u8],
    entry_paths: &HashSet<String>,
) -> Result<Vec<String>, CharacterCompatError> {
    let card: Value = serde_json::from_slice(card_json)?;
    let mut warnings = Vec::new();
    let Some(assets) = card.pointer("/data/assets").and_then(Value::as_array) else {
        return Ok(warnings);
    };
    for (index, asset) in assets.iter().enumerate() {
        let Some(uri) = asset.get("uri").and_then(Value::as_str) else {
            continue;
        };
        let path = uri
            .strip_prefix("embeded://")
            .or_else(|| uri.strip_prefix("embedded://"));
        if uri.starts_with("embedded://") {
            warnings.push(format!(
                "asset {index} uses the non-standard embedded:// spelling; CHARX specifies embeded://"
            ));
        }
        if let Some(path) = path
            && !entry_paths.contains(path)
        {
            warnings.push(format!(
                "asset {index} references missing CHARX entry {path:?}; the descriptor was preserved"
            ));
        }
    }
    Ok(warnings)
}

fn source_file_name(format: ExternalCharacterImportFormat) -> &'static str {
    match format {
        ExternalCharacterImportFormat::Ccv1Json
        | ExternalCharacterImportFormat::Ccv2Json
        | ExternalCharacterImportFormat::Ccv3Json => "source.json",
        ExternalCharacterImportFormat::Ccv1Png
        | ExternalCharacterImportFormat::Ccv2Png
        | ExternalCharacterImportFormat::Ccv3Png => "source.png",
        ExternalCharacterImportFormat::Ccv3Charx => "source.charx",
    }
}

fn preserved_source_path(
    core: &MomoCore,
    character_id: Uuid,
    format: ExternalCharacterImportFormat,
) -> PathBuf {
    core.data_dir()
        .join(SOURCE_DIRECTORY)
        .join(character_id.to_string())
        .join(source_file_name(format))
}

fn save_preserved_source(
    core: &MomoCore,
    character_id: Uuid,
    format: ExternalCharacterImportFormat,
    bytes: &[u8],
) -> Result<(), CharacterCompatError> {
    atomic_write_bytes(&preserved_source_path(core, character_id, format), bytes)
}

async fn load_stored_external_character(
    core: &MomoCore,
    character_id: Uuid,
) -> Result<StoredExternalCharacter, CharacterCompatError> {
    let document = core
        .store()
        .portable_metadata(EXTERNAL_METADATA_CATEGORY, &character_id.to_string())
        .await?
        .ok_or_else(|| {
            CharacterCompatError::Invalid("character has no preserved external source".to_owned())
        })?;
    Ok(serde_json::from_str(&document)?)
}

fn load_preserved_source(
    core: &MomoCore,
    character_id: Uuid,
    stored: &StoredExternalCharacter,
) -> Result<Vec<u8>, CharacterCompatError> {
    let expected = stored.source.as_ref().ok_or_else(|| {
        CharacterCompatError::Invalid(
            "the imported character predates exact source preservation".to_owned(),
        )
    })?;
    let path = preserved_source_path(core, character_id, stored.source_format);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        CharacterCompatError::Invalid(format!(
            "preserved source is unavailable at {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != expected.size {
        return Err(CharacterCompatError::Invalid(
            "preserved source type or size does not match its metadata".to_owned(),
        ));
    }
    ensure_size(metadata.len(), MAX_CHARX_BYTES, "preserved source")?;
    let bytes = fs::read(path)?;
    if hex::encode(Sha256::digest(&bytes)) != expected.sha256 {
        return Err(CharacterCompatError::Invalid(
            "preserved source hash does not match its metadata".to_owned(),
        ));
    }
    Ok(bytes)
}

fn load_charx_source(
    core: &MomoCore,
    character_id: Uuid,
    expected: &StoredCharxPackage,
) -> Result<Vec<u8>, CharacterCompatError> {
    let stored = StoredExternalCharacter {
        source_format: ExternalCharacterImportFormat::Ccv3Charx,
        card: Value::Null,
        warnings: Vec::new(),
        source: Some(StoredSourceFile {
            sha256: expected.archive_sha256.clone(),
            size: fs::metadata(preserved_source_path(
                core,
                character_id,
                ExternalCharacterImportFormat::Ccv3Charx,
            ))?
            .len(),
        }),
        charx: Some(expected.clone()),
    };
    let bytes = load_preserved_source(core, character_id, &stored)?;
    if hex::encode(Sha256::digest(&bytes)) != expected.archive_sha256 {
        return Err(CharacterCompatError::Invalid(
            "preserved CHARX source hash does not match its metadata".to_owned(),
        ));
    }
    let parsed = parse_charx_package(bytes)?;
    if parsed.info != *expected {
        return Err(CharacterCompatError::Invalid(
            "preserved CHARX source structure does not match its metadata".to_owned(),
        ));
    }
    Ok(parsed.bytes)
}

fn write_charx(
    path: &Path,
    card: &Value,
    source_archive: Option<&[u8]>,
) -> Result<(), CharacterCompatError> {
    if source_archive.is_none() && has_embedded_asset_references(card) {
        return Err(CharacterCompatError::Invalid(
            "cannot export CHARX because the card references embedded assets but no preserved source archive is available"
                .to_owned(),
        ));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = zip::ZipWriter::new(temporary.as_file_mut());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        writer.start_file("card.json", options)?;
        serde_json::to_writer_pretty(&mut writer, card)?;
        writer.write_all(b"\n")?;

        if let Some(source_archive) = source_archive {
            let mut source = zip::ZipArchive::new(Cursor::new(source_archive))?;
            for index in 0..source.len() {
                let mut entry = source.by_index(index)?;
                let name = entry.name().to_owned();
                if name == "card.json" {
                    continue;
                }
                let entry_options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(entry.unix_mode().unwrap_or(if entry.is_dir() {
                        0o755
                    } else {
                        0o644
                    }));
                if entry.is_dir() {
                    writer.add_directory(name, entry_options)?;
                    continue;
                }
                writer.start_file(name, entry_options)?;
                std::io::copy(&mut entry, &mut writer)?;
            }
        }
        writer.finish()?;
    }
    temporary.as_file().sync_all()?;
    ensure_size(
        temporary.as_file().metadata()?.len(),
        MAX_CHARX_BYTES,
        "CHARX output",
    )?;
    temporary
        .persist(path)
        .map_err(|error| CharacterCompatError::Io(error.error))?;
    Ok(())
}

fn has_embedded_asset_references(card: &Value) -> bool {
    card.pointer("/data/assets")
        .and_then(Value::as_array)
        .is_some_and(|assets| {
            assets.iter().any(|asset| {
                asset.get("uri").and_then(Value::as_str).is_some_and(|uri| {
                    uri.starts_with("embeded://") || uri.starts_with("embedded://")
                })
            })
        })
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), CharacterCompatError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| CharacterCompatError::Io(error.error))?;
    Ok(())
}

pub(crate) fn export_preserved_source_asset(
    core: &MomoCore,
    character_id: Uuid,
    metadata_document: &str,
    output_directory: &Path,
) -> Result<Option<String>, CharacterCompatError> {
    let stored: StoredExternalCharacter = serde_json::from_str(metadata_document)?;
    let Some(_) = stored.source.as_ref() else {
        return Ok(None);
    };
    let bytes = load_preserved_source(core, character_id, &stored)?;
    let file_name = source_file_name(stored.source_format).to_owned();
    atomic_write_bytes(&output_directory.join(&file_name), &bytes)?;
    Ok(Some(file_name))
}

pub(crate) fn import_preserved_source_asset(
    core: &MomoCore,
    character_id: Uuid,
    metadata_document: &str,
    input_directory: &Path,
) -> Result<Option<String>, CharacterCompatError> {
    let Some((stored, bytes, file_name)) =
        validate_preserved_source(metadata_document, input_directory)?
    else {
        return Ok(None);
    };
    save_preserved_source(core, character_id, stored.source_format, &bytes)?;
    Ok(Some(file_name))
}

pub(crate) fn validate_preserved_source_asset(
    metadata_document: &str,
    input_directory: &Path,
) -> Result<(), CharacterCompatError> {
    validate_preserved_source(metadata_document, input_directory).map(|_| ())
}

fn validate_preserved_source(
    metadata_document: &str,
    input_directory: &Path,
) -> Result<Option<(StoredExternalCharacter, Vec<u8>, String)>, CharacterCompatError> {
    let stored: StoredExternalCharacter = serde_json::from_str(metadata_document)?;
    let Some(expected_source) = stored.source.as_ref() else {
        return Ok(None);
    };
    let file_name = source_file_name(stored.source_format);
    let input_path = input_directory.join(file_name);
    let metadata = fs::symlink_metadata(&input_path).map_err(|error| {
        CharacterCompatError::Invalid(format!(
            "MOC is missing the declared preserved source {}: {error}",
            input_path.display()
        ))
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != expected_source.size
    {
        return Err(CharacterCompatError::Invalid(
            "MOC preserved source type or size does not match metadata".to_owned(),
        ));
    }
    ensure_size(metadata.len(), MAX_CHARX_BYTES, "MOC preserved source")?;
    let bytes = fs::read(&input_path)?;
    if hex::encode(Sha256::digest(&bytes)) != expected_source.sha256 {
        return Err(CharacterCompatError::Invalid(
            "MOC preserved source does not match its declared hash".to_owned(),
        ));
    }
    if let Some(expected_charx) = stored.charx.as_ref() {
        let parsed = parse_charx_package(bytes.clone())?;
        if parsed.info != *expected_charx {
            return Err(CharacterCompatError::Invalid(
                "MOC CHARX source does not match its declared structure".to_owned(),
            ));
        }
    }
    Ok(Some((stored, bytes, file_name.to_owned())))
}

fn export_ccv2(
    character: &CharacterCard,
    source: Option<Value>,
) -> Result<Value, CharacterCompatError> {
    let mut root = source
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let had_source = root.get("spec").and_then(Value::as_str) == Some("chara_card_v2");
    let mut data = root
        .remove("data")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    root.insert("spec".to_owned(), json!("chara_card_v2"));
    root.insert("spec_version".to_owned(), json!("2.0"));
    overlay_common_fields(&mut data, character, had_source);
    data.entry("creator_notes").or_insert_with(|| json!(""));
    data.entry("system_prompt").or_insert_with(|| json!(""));
    data.entry("post_history_instructions")
        .or_insert_with(|| json!(""));
    data.entry("alternate_greetings")
        .or_insert_with(|| json!([]));
    data.entry("tags").or_insert_with(|| json!([]));
    data.entry("extensions").or_insert_with(|| json!({}));
    root.insert("data".to_owned(), Value::Object(data));
    Ok(Value::Object(root))
}

fn export_ccv3(
    character: &CharacterCard,
    source: Option<Value>,
) -> Result<Value, CharacterCompatError> {
    let mut root = source
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let had_source = root.get("spec").and_then(Value::as_str) == Some("chara_card_v3");
    let mut data = root
        .remove("data")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    root.insert("spec".to_owned(), json!("chara_card_v3"));
    root.insert("spec_version".to_owned(), json!("3.0"));
    overlay_common_fields(&mut data, character, had_source);
    data.entry("creator_notes").or_insert_with(|| json!(""));
    data.entry("system_prompt").or_insert_with(|| json!(""));
    data.entry("post_history_instructions")
        .or_insert_with(|| json!(""));
    data.entry("alternate_greetings")
        .or_insert_with(|| json!([]));
    data.entry("tags").or_insert_with(|| json!([]));
    data.entry("extensions").or_insert_with(|| json!({}));
    data.entry("group_only_greetings")
        .or_insert_with(|| json!([]));
    root.insert("data".to_owned(), Value::Object(data));
    Ok(Value::Object(root))
}

fn overlay_common_fields(
    data: &mut Map<String, Value>,
    character: &CharacterCard,
    had_source: bool,
) {
    data.insert("name".to_owned(), json!(character.name));
    data.insert("creator".to_owned(), json!(character.author_name));
    data.insert("character_version".to_owned(), json!(character.version));
    data.insert(
        "first_mes".to_owned(),
        json!(character.opening_markdown.as_deref().unwrap_or_default()),
    );
    if !had_source {
        data.insert(
            "description".to_owned(),
            json!(character.character_markdown),
        );
        data.insert("personality".to_owned(), json!(""));
        data.insert("scenario".to_owned(), json!(""));
        data.insert("mes_example".to_owned(), json!(""));
    } else {
        for field in ["description", "personality", "scenario", "mes_example"] {
            data.entry(field).or_insert_with(|| json!(""));
        }
    }
}

fn atomic_write_json(path: &Path, value: &Value) -> Result<(), CharacterCompatError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| CharacterCompatError::Io(error.error))?;
    Ok(())
}

const fn source_format_name(format: ExternalCharacterImportFormat) -> &'static str {
    match format {
        ExternalCharacterImportFormat::Ccv1Json => "ccv1_json",
        ExternalCharacterImportFormat::Ccv1Png => "ccv1_png",
        ExternalCharacterImportFormat::Ccv2Json => "ccv2_json",
        ExternalCharacterImportFormat::Ccv2Png => "ccv2_png",
        ExternalCharacterImportFormat::Ccv3Json => "ccv3_json",
        ExternalCharacterImportFormat::Ccv3Png => "ccv3_png",
        ExternalCharacterImportFormat::Ccv3Charx => "ccv3_charx",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ccv3() -> Value {
        let mut card = ccv2();
        card["spec"] = json!("chara_card_v3");
        card["spec_version"] = json!("3.0");
        card["data"]["group_only_greetings"] = json!([]);
        card["data"]["assets"] = json!([]);
        card
    }

    fn ccv1() -> Value {
        json!({
            "name": "Legacy Snowball",
            "description": "Legacy description",
            "personality": "Warm",
            "scenario": "At home",
            "first_mes": "Hello",
            "mes_example": "<BOT>: Hello"
        })
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(&(data.len() as u32).to_be_bytes());
        output.extend_from_slice(kind);
        output.extend_from_slice(data);
        let mut hasher = Hasher::new();
        hasher.update(kind);
        hasher.update(data);
        output.extend_from_slice(&hasher.finalize().to_be_bytes());
        output
    }

    fn embedded_png(keyword: &[u8], card: &Value) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut text = keyword.to_vec();
        text.push(0);
        text.extend_from_slice(
            BASE64
                .encode(serde_json::to_vec(card).expect("card JSON"))
                .as_bytes(),
        );
        png.extend(png_chunk(b"tEXt", &text));
        png.extend(png_chunk(b"IEND", &[]));
        png
    }

    fn charx_bytes(card: &Value, entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        archive
            .start_file("card.json", options)
            .expect("card entry");
        archive
            .write_all(&serde_json::to_vec(card).expect("CCv3 JSON"))
            .expect("card JSON");
        for (name, data) in entries {
            archive.start_file(*name, options).expect("CHARX entry");
            archive.write_all(data).expect("CHARX data");
        }
        archive.finish().expect("finish CHARX").into_inner()
    }

    fn ccv2() -> Value {
        json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Snowball",
                "description": "A gentle cat.",
                "personality": "Warm",
                "scenario": "At home",
                "first_mes": "Welcome home.",
                "mes_example": "{{char}}: Hello",
                "creator_notes": "Visible note",
                "system_prompt": "runtime only",
                "post_history_instructions": "",
                "alternate_greetings": [],
                "character_book": {"entries": []},
                "tags": ["cat"],
                "creator": "Creator",
                "character_version": "1.2.3",
                "extensions": {"vendor": {"voice": "one"}}
            }
        })
    }

    #[test]
    fn parses_ccv2_without_injecting_runtime_fields() {
        let parsed =
            parse_external_json(&serde_json::to_vec(&ccv2()).expect("JSON"), None).expect("CCv2");
        assert_eq!(parsed.name, "Snowball");
        assert_eq!(parsed.version, "1.2.3");
        assert!(parsed.character_markdown.contains("A gentle cat."));
        assert!(!parsed.character_markdown.contains("runtime only"));
        assert_eq!(parsed.opening_markdown.as_deref(), Some("Welcome home."));
        assert!(!parsed.warnings.is_empty());
    }

    #[test]
    fn parses_legacy_ccv1_and_exports_v3() {
        let parsed =
            parse_external_json(&serde_json::to_vec(&ccv1()).expect("JSON"), None).expect("CCv1");
        assert_eq!(parsed.format, ExternalCharacterImportFormat::Ccv1Json);
        assert_eq!(parsed.author_name, "Unknown");
        let now = Utc::now();
        let character = CharacterCard {
            id: momo_domain::new_id(),
            scope_id: momo_domain::new_id(),
            name: parsed.name,
            version: parsed.version,
            author_name: parsed.author_name,
            author_url: None,
            character_markdown: parsed.character_markdown,
            user_markdown: String::new(),
            opening_markdown: parsed.opening_markdown,
            created_at: now,
            updated_at: now,
        };
        let exported = export_ccv3(&character, None).expect("CCv3 export");
        assert_eq!(exported["spec"], "chara_card_v3");
        assert_eq!(exported["data"]["name"], "Legacy Snowball");
        assert_eq!(exported["data"]["group_only_greetings"], json!([]));
    }

    #[test]
    fn validates_external_spec_versions_without_prefix_guessing() {
        let mut future_v3 = ccv3();
        future_v3["spec_version"] = json!("3.1");
        let parsed = parse_external_json(
            &serde_json::to_vec(&future_v3).expect("future CCv3 JSON"),
            None,
        )
        .expect("compatible future CCv3 revision");
        assert!(
            parsed
                .warnings
                .iter()
                .any(|warning| warning.contains("newer than"))
        );

        for invalid in ["3bad", "30", "2.9"] {
            let mut card = ccv3();
            card["spec_version"] = json!(invalid);
            assert!(
                parse_external_json(&serde_json::to_vec(&card).expect("invalid CCv3 JSON"), None,)
                    .is_err()
            );
        }

        let mut nonstandard_v2 = ccv2();
        nonstandard_v2["spec_version"] = json!("2.1");
        assert!(
            parse_external_json(
                &serde_json::to_vec(&nonstandard_v2).expect("nonstandard CCv2 JSON"),
                None,
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn import_and_export_preserve_unknown_source_fields() {
        let directory = tempfile::tempdir().expect("data directory");
        let source = directory.path().join("source.json");
        fs::write(&source, serde_json::to_vec_pretty(&ccv2()).expect("JSON")).expect("source");
        let core = MomoCore::initialize(directory.path().join("core"))
            .await
            .expect("core");
        let scope_id = momo_domain::new_id();
        let imported = import_external_character(
            &core,
            scope_id,
            &source,
            ExternalCharacterImportFormat::Ccv2Json,
        )
        .await
        .expect("import");
        let preserved = directory.path().join("preserved.json");
        let preserved_report =
            export_preserved_character_source(&core, scope_id, imported.character.id, &preserved)
                .await
                .expect("preserved source export");
        assert_eq!(preserved_report.source_format, "ccv2_json");
        assert_eq!(
            fs::read(&preserved).expect("preserved bytes"),
            fs::read(&source).expect("source bytes")
        );
        let output = directory.path().join("output.json");
        let report = export_external_character(
            &core,
            scope_id,
            imported.character.id,
            &output,
            ExternalCharacterExportFormat::Ccv2Json,
        )
        .await
        .expect("export");
        assert_eq!(report["preserved_source_fields"], true);
        let exported: Value =
            serde_json::from_slice(&fs::read(output).expect("output")).expect("exported JSON");
        assert_eq!(exported["data"]["extensions"]["vendor"]["voice"], "one");
        assert_eq!(exported["data"]["system_prompt"], "runtime only");

        let moc = directory.path().join("character.moc");
        crate::export_moc(
            &core,
            &moc,
            &json!({}),
            &crate::MocExportPlan {
                include_config: false,
                characters: vec![crate::MocCharacterSelection {
                    space_id: scope_id,
                    character_ids: vec![imported.character.id],
                }],
                conversations: vec![],
                memory: vec![],
                semantic_graph: vec![],
                compatibility: crate::MocCompatibility::PreservedSource,
            },
        )
        .await
        .expect("MOC export");
        let destination = MomoCore::initialize(directory.path().join("destination"))
            .await
            .expect("destination");
        let destination_scope = momo_domain::new_id();
        crate::import_moc(
            &destination,
            &moc,
            &crate::MocImportPlan {
                apply_config: false,
                space_map: [(scope_id, destination_scope)].into_iter().collect(),
                conflict_mode: crate::ConflictMode::Replace,
            },
        )
        .await
        .expect("MOC import");
        let round_trip = directory.path().join("round-trip.json");
        let report = export_external_character(
            &destination,
            destination_scope,
            imported.character.id,
            &round_trip,
            ExternalCharacterExportFormat::Ccv2Json,
        )
        .await
        .expect("round-trip export");
        assert_eq!(report["preserved_source_fields"], true);
        let round_trip: Value =
            serde_json::from_slice(&fs::read(round_trip).expect("round-trip output"))
                .expect("round-trip JSON");
        assert_eq!(round_trip["data"]["extensions"]["vendor"]["voice"], "one");
    }

    #[test]
    fn rejects_mismatched_png_chunk_and_spec() {
        let error = parse_external_json(
            &serde_json::to_vec(&ccv2()).expect("JSON"),
            Some(ExternalCharacterImportFormat::Ccv3Png),
        )
        .expect_err("mismatched format");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn extracts_ccv3_png_and_validates_crc() {
        let png = embedded_png(b"ccv3", &ccv3());
        let (json, chunk) = extract_png_card(&png).expect("embedded CCv3");
        assert!(matches!(chunk, PngCardChunk::Ccv3));
        let parsed = parse_external_json(&json, Some(ExternalCharacterImportFormat::Ccv3Png))
            .expect("CCv3 PNG");
        assert_eq!(parsed.name, "Snowball");

        let mut corrupted = png;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        assert!(extract_png_card(&corrupted).is_err());
    }

    #[test]
    fn rejects_apng_instead_of_selecting_a_frame() {
        let mut png = embedded_png(b"ccv3", &ccv3());
        let iend = png
            .windows(4)
            .position(|window| window == b"IEND")
            .expect("IEND")
            - 4;
        png.splice(iend..iend, png_chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        let error = extract_png_card(&png).expect_err("APNG must be rejected");
        assert!(error.to_string().contains("APNG"));
    }

    #[test]
    fn reads_charx_card_and_validates_container() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("card.charx");
        fs::write(&path, charx_bytes(&ccv3(), &[])).expect("CHARX output");

        let parsed = parse_external_path(&path, ExternalCharacterImportFormat::Ccv3Charx)
            .expect("CHARX import");
        assert_eq!(parsed.format, ExternalCharacterImportFormat::Ccv3Charx);
        assert_eq!(parsed.name, "Snowball");
        let package = parsed.charx.expect("CHARX package");
        assert_eq!(package.info.entry_count, 1);
        assert_eq!(package.info.asset_count, 0);
    }

    #[test]
    fn reads_jpeg_zip_hybrid_and_rejects_unsafe_entries() {
        let zip = charx_bytes(&ccv3(), &[]);
        let mut hybrid = b"\xFF\xD8\xFFpreview\xFF\xD9".to_vec();
        hybrid.extend_from_slice(&zip);
        let parsed = parse_charx_package(hybrid).expect("JPEG+ZIP CHARX");
        assert!(parsed.info.jpeg_zip_hybrid);

        let unsafe_archive = charx_bytes(&ccv3(), &[("../outside.txt", b"bad")]);
        assert!(parse_charx_package(unsafe_archive).is_err());
    }

    #[tokio::test]
    async fn charx_assets_and_risu_extensions_round_trip_through_moc() {
        let directory = tempfile::tempdir().expect("data directory");
        let mut card = ccv3();
        card["data"]["assets"] = json!([{
            "type": "icon",
            "uri": "embeded://assets/icon/images/avatar.png",
            "name": "main",
            "ext": "png"
        }]);
        let input = directory.path().join("source.charx");
        fs::write(
            &input,
            charx_bytes(
                &card,
                &[
                    ("assets/icon/images/avatar.png", b"image-bytes"),
                    ("x_meta/0.json", br#"{"type":"PNG"}"#),
                    ("module.risum", b"opaque-module"),
                    ("app.json", br#"{"future":true}"#),
                ],
            ),
        )
        .expect("source CHARX");
        let core = MomoCore::initialize(directory.path().join("core"))
            .await
            .expect("core");
        let scope_id = momo_domain::new_id();
        let imported = import_external_character(
            &core,
            scope_id,
            &input,
            ExternalCharacterImportFormat::Ccv3Charx,
        )
        .await
        .expect("CHARX import");
        assert!(
            imported
                .warnings
                .iter()
                .all(|warning| !warning.contains("missing CHARX entry"))
        );
        let mut updated = imported.character.clone();
        updated.name = "Snowball 0.4".to_owned();
        updated.updated_at = Utc::now();
        core.store()
            .stage_character_update(&updated)
            .await
            .expect("character update");

        let output = directory.path().join("output.charx");
        let report = export_external_character(
            &core,
            scope_id,
            imported.character.id,
            &output,
            ExternalCharacterExportFormat::Ccv3Charx,
        )
        .await
        .expect("CHARX export");
        assert_eq!(report["preserved_source_assets"], true);
        assert_charx_entry(&output, "assets/icon/images/avatar.png", b"image-bytes");
        assert_charx_entry(&output, "module.risum", b"opaque-module");
        assert_charx_entry(&output, "app.json", br#"{"future":true}"#);
        assert_eq!(read_charx_card(&output)["data"]["name"], "Snowball 0.4");

        let moc = directory.path().join("character.moc");
        crate::export_moc(
            &core,
            &moc,
            &json!({}),
            &crate::MocExportPlan {
                include_config: false,
                characters: vec![crate::MocCharacterSelection {
                    space_id: scope_id,
                    character_ids: vec![imported.character.id],
                }],
                conversations: vec![],
                memory: vec![],
                semantic_graph: vec![],
                compatibility: crate::MocCompatibility::PreservedSource,
            },
        )
        .await
        .expect("MOC export");
        let destination = MomoCore::initialize(directory.path().join("destination"))
            .await
            .expect("destination");
        let destination_scope = momo_domain::new_id();
        crate::import_moc(
            &destination,
            &moc,
            &crate::MocImportPlan {
                apply_config: false,
                space_map: [(scope_id, destination_scope)].into_iter().collect(),
                conflict_mode: crate::ConflictMode::Replace,
            },
        )
        .await
        .expect("MOC import");
        let round_trip = directory.path().join("round-trip.charx");
        export_external_character(
            &destination,
            destination_scope,
            imported.character.id,
            &round_trip,
            ExternalCharacterExportFormat::Ccv3Charx,
        )
        .await
        .expect("round-trip CHARX export");
        assert_charx_entry(&round_trip, "assets/icon/images/avatar.png", b"image-bytes");
        assert_charx_entry(&round_trip, "x_meta/0.json", br#"{"type":"PNG"}"#);
        assert_eq!(read_charx_card(&round_trip)["data"]["name"], "Snowball 0.4");
    }

    #[tokio::test]
    async fn native_character_exports_card_only_charx() {
        let directory = tempfile::tempdir().expect("data directory");
        let core = MomoCore::initialize(directory.path().join("core"))
            .await
            .expect("core");
        let scope_id = momo_domain::new_id();
        let now = Utc::now();
        let character = CharacterCard {
            id: momo_domain::new_id(),
            scope_id,
            name: "Native".to_owned(),
            version: "1.0.0".to_owned(),
            author_name: "MOMO".to_owned(),
            author_url: None,
            character_markdown: "# Native".to_owned(),
            user_markdown: String::new(),
            opening_markdown: Some("Hello".to_owned()),
            created_at: now,
            updated_at: now,
        };
        core.store()
            .stage_character(&character)
            .await
            .expect("character");
        let output = directory.path().join("native.charx");
        let report = export_external_character(
            &core,
            scope_id,
            character.id,
            &output,
            ExternalCharacterExportFormat::Ccv3Charx,
        )
        .await
        .expect("CHARX export");
        assert_eq!(report["preserved_source_assets"], false);
        assert_eq!(read_charx_card(&output)["data"]["name"], "Native");
        let archive = zip::ZipArchive::new(fs::File::open(output).expect("CHARX file"))
            .expect("CHARX archive");
        assert_eq!(archive.len(), 1);
    }

    fn assert_charx_entry(path: &Path, name: &str, expected: &[u8]) {
        let file = fs::File::open(path).expect("CHARX file");
        let mut archive = zip::ZipArchive::new(file).expect("CHARX archive");
        let mut entry = archive.by_name(name).expect("CHARX entry");
        let mut actual = Vec::new();
        entry.read_to_end(&mut actual).expect("entry data");
        assert_eq!(actual, expected);
    }

    fn read_charx_card(path: &Path) -> Value {
        let file = fs::File::open(path).expect("CHARX file");
        let mut archive = zip::ZipArchive::new(file).expect("CHARX archive");
        let mut entry = archive.by_name("card.json").expect("card.json");
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("card JSON");
        serde_json::from_slice(&data).expect("CCv3 card")
    }

    #[test]
    fn rejects_non_object_and_unknown_specs() {
        assert!(parse_external_json(b"[]", None).is_err());
        assert!(
            parse_external_json(
                br#"{"spec":"future_card","spec_version":"9.0","data":{}}"#,
                None
            )
            .is_err()
        );
    }
}
