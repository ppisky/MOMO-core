//! Import and export adapters for the external Character Card v1/v2/v3 formats.

use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use crc32fast::Hasher;
use momo_domain::CharacterCard;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::MomoCore;

const MAX_JSON_BYTES: u64 = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHARX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CHARX_ENTRIES: usize = 10_000;
const EXTERNAL_METADATA_CATEGORY: &str = "external_character_card";

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
enum ExternalCharacterSourceFormat {
    Ccv1Json,
    Ccv1Png,
    Ccv2Json,
    Ccv2Png,
    Ccv3Json,
    Ccv3Png,
    Ccv3Charx,
}

impl ExternalCharacterSourceFormat {
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
}

impl std::str::FromStr for ExternalCharacterExportFormat {
    type Err = CharacterCompatError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ccv2_json" => Ok(Self::Ccv2Json),
            "ccv3_json" => Ok(Self::Ccv3Json),
            _ => Err(CharacterCompatError::Invalid(format!(
                "unsupported export format {value:?}; expected ccv2_json or ccv3_json"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredExternalCharacter {
    source_format: ExternalCharacterSourceFormat,
    card: Value,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalCharacterImport {
    pub character: CharacterCard,
    pub source_format: String,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct ParsedExternalCharacter {
    format: ExternalCharacterSourceFormat,
    card: Value,
    name: String,
    author_name: String,
    version: String,
    character_markdown: String,
    opening_markdown: Option<String>,
    warnings: Vec<String>,
}

pub async fn import_external_character(
    core: &MomoCore,
    scope_id: Uuid,
    input_path: impl AsRef<Path>,
) -> Result<ExternalCharacterImport, CharacterCompatError> {
    let parsed = parse_external_path(input_path.as_ref())?;
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
    let stored = StoredExternalCharacter {
        source_format: parsed.format,
        card: parsed.card,
        warnings: parsed.warnings.clone(),
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
                ExternalCharacterSourceFormat::Ccv2Json | ExternalCharacterSourceFormat::Ccv2Png,
                ExternalCharacterExportFormat::Ccv2Json
            ) | (
                ExternalCharacterSourceFormat::Ccv3Json
                    | ExternalCharacterSourceFormat::Ccv3Png
                    | ExternalCharacterSourceFormat::Ccv3Charx,
                ExternalCharacterExportFormat::Ccv3Json
            )
        )
    });
    let source = preserved_source
        .then(|| stored.as_ref().map(|source| source.card.clone()))
        .flatten();
    let value = match format {
        ExternalCharacterExportFormat::Ccv2Json => export_ccv2(&character, source)?,
        ExternalCharacterExportFormat::Ccv3Json => export_ccv3(&character, source)?,
    };
    atomic_write_json(output_path.as_ref(), &value)?;
    Ok(json!({
        "character_id": character_id,
        "format": match format {
            ExternalCharacterExportFormat::Ccv2Json => "ccv2_json",
            ExternalCharacterExportFormat::Ccv3Json => "ccv3_json",
        },
        "output_path": output_path.as_ref(),
        "preserved_source_fields": preserved_source,
    }))
}

fn parse_external_path(path: &Path) -> Result<ParsedExternalCharacter, CharacterCompatError> {
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
    match extension.as_str() {
        "json" => {
            ensure_size(metadata.len(), MAX_JSON_BYTES, "JSON")?;
            parse_external_json(&fs::read(path)?, None)
        }
        "png" | "apng" => {
            ensure_size(metadata.len(), MAX_IMAGE_BYTES, "PNG/APNG")?;
            let (json, chunk) = extract_png_card(&fs::read(path)?)?;
            let hinted = match chunk {
                PngCardChunk::Ccv3 => Some(ExternalCharacterSourceFormat::Ccv3Png),
                PngCardChunk::Chara => Some(ExternalCharacterSourceFormat::Ccv2Png),
            };
            parse_external_json(&json, hinted)
        }
        "charx" => {
            ensure_size(metadata.len(), MAX_CHARX_BYTES, "CHARX")?;
            let json = read_charx_card(path)?;
            let mut parsed = parse_external_json(
                json.as_bytes(),
                Some(ExternalCharacterSourceFormat::Ccv3Charx),
            )?;
            if parsed
                .card
                .pointer("/data/assets")
                .and_then(Value::as_array)
                .is_some_and(|assets| !assets.is_empty())
            {
                parsed.warnings.push(
                    "CHARX assets were not imported; card.json was preserved for round-trip metadata"
                        .to_owned(),
                );
            }
            Ok(parsed)
        }
        _ => Err(CharacterCompatError::Invalid(format!(
            "unsupported input extension {extension:?}; expected json, png, apng, or charx"
        ))),
    }
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
    hinted_format: Option<ExternalCharacterSourceFormat>,
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
    if let Some(hint) = hinted_format
        && ((hint.major() == 3) != (detected == 3))
    {
        return Err(CharacterCompatError::Invalid(
            "embedded chunk format does not match card spec".to_owned(),
        ));
    }
    if detected > 1 {
        let spec_version = required_string(root, "spec_version")?;
        if !spec_version.starts_with(if detected == 2 { '2' } else { '3' }) {
            return Err(CharacterCompatError::Invalid(format!(
                "card spec_version {spec_version:?} does not match spec"
            )));
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
    let mut warnings = Vec::new();
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
        1 => ExternalCharacterSourceFormat::Ccv1Json,
        2 => ExternalCharacterSourceFormat::Ccv2Json,
        _ => ExternalCharacterSourceFormat::Ccv3Json,
    };
    let format = match (hinted_format, detected) {
        (Some(ExternalCharacterSourceFormat::Ccv2Png), 1) => ExternalCharacterSourceFormat::Ccv1Png,
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
        "PNG/APNG does not contain a ccv3 or chara tEXt chunk".to_owned(),
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

fn read_charx_card(path: &Path) -> Result<String, CharacterCompatError> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() > MAX_CHARX_ENTRIES {
        return Err(CharacterCompatError::Invalid(format!(
            "CHARX contains more than {MAX_CHARX_ENTRIES} entries"
        )));
    }
    let mut card = archive.by_name("card.json")?;
    if !card.is_file() || card.size() == 0 || card.size() > MAX_JSON_BYTES {
        return Err(CharacterCompatError::Invalid(
            "CHARX card.json has an invalid size or type".to_owned(),
        ));
    }
    let mut output = String::new();
    card.read_to_string(&mut output)?;
    Ok(output)
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

const fn source_format_name(format: ExternalCharacterSourceFormat) -> &'static str {
    match format {
        ExternalCharacterSourceFormat::Ccv1Json => "ccv1_json",
        ExternalCharacterSourceFormat::Ccv1Png => "ccv1_png",
        ExternalCharacterSourceFormat::Ccv2Json => "ccv2_json",
        ExternalCharacterSourceFormat::Ccv2Png => "ccv2_png",
        ExternalCharacterSourceFormat::Ccv3Json => "ccv3_json",
        ExternalCharacterSourceFormat::Ccv3Png => "ccv3_png",
        ExternalCharacterSourceFormat::Ccv3Charx => "ccv3_charx",
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
        assert_eq!(parsed.format, ExternalCharacterSourceFormat::Ccv1Json);
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

    #[tokio::test]
    async fn import_and_export_preserve_unknown_source_fields() {
        let directory = tempfile::tempdir().expect("data directory");
        let source = directory.path().join("source.json");
        fs::write(&source, serde_json::to_vec_pretty(&ccv2()).expect("JSON")).expect("source");
        let core = MomoCore::initialize(directory.path().join("core"))
            .await
            .expect("core");
        let scope_id = momo_domain::new_id();
        let imported = import_external_character(&core, scope_id, &source)
            .await
            .expect("import");
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
            scope_id,
            &json!({}),
            crate::ExportSelection {
                config: false,
                characters: true,
                conversations: false,
                memory: false,
                semantic_graph: false,
                character_id: Some(imported.character.id),
            },
        )
        .await
        .expect("MOC export");
        let destination = MomoCore::initialize(directory.path().join("destination"))
            .await
            .expect("destination");
        let destination_scope = momo_domain::new_id();
        crate::import_moc(&destination, &moc, destination_scope, "replace")
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
            Some(ExternalCharacterSourceFormat::Ccv3Png),
        )
        .expect_err("mismatched format");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn extracts_ccv3_png_and_validates_crc() {
        let png = embedded_png(b"ccv3", &ccv3());
        let (json, chunk) = extract_png_card(&png).expect("embedded CCv3");
        assert!(matches!(chunk, PngCardChunk::Ccv3));
        let parsed = parse_external_json(&json, Some(ExternalCharacterSourceFormat::Ccv3Png))
            .expect("CCv3 PNG");
        assert_eq!(parsed.name, "Snowball");

        let mut corrupted = png;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        assert!(extract_png_card(&corrupted).is_err());
    }

    #[test]
    fn reads_charx_card_json_without_extracting_assets() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("card.charx");
        let file = fs::File::create(&path).expect("CHARX output");
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "card.json",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("card entry");
        archive
            .write_all(&serde_json::to_vec(&ccv3()).expect("CCv3 JSON"))
            .expect("card JSON");
        archive.finish().expect("finish CHARX");

        let parsed = parse_external_path(&path).expect("CHARX import");
        assert_eq!(parsed.format, ExternalCharacterSourceFormat::Ccv3Charx);
        assert_eq!(parsed.name, "Snowball");
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
