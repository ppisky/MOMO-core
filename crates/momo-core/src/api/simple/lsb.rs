//! Strict file-oriented facade for lossless MOMO LSB image carriers.

use std::{fs, io::Write, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;

const MAX_LSB_SOURCE_FILE_BYTES: u64 = 512 * 1024 * 1024;

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum LsbExportPayload {
    MomoCharacter {
        owner_space_id: uuid::Uuid,
        character_id: uuid::Uuid,
    },
    Moc {
        input_path: String,
    },
    Charx {
        input_path: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbedLsbImageRequest {
    carrier_path: String,
    output_path: String,
    format: crate::LsbImageFormat,
    payload: LsbExportPayload,
    #[serde(default = "default_true")]
    compress: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtractLsbImageRequest {
    input_path: String,
    output_path: String,
    format: crate::LsbImageFormat,
    expected_payload_type: crate::LsbPayloadType,
}

#[derive(Debug, Serialize)]
struct LsbFileReport {
    output_path: String,
    format: crate::LsbImageFormat,
    payload_type: crate::LsbPayloadType,
    bytes: usize,
}

pub async fn embed_lsb_image_json(request_json: String) -> Result<String, String> {
    let request: EmbedLsbImageRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    validate_image_extension(&request.carrier_path, request.format)?;
    validate_image_extension(&request.output_path, request.format)?;
    let carrier = read_bounded_file(&request.carrier_path, crate::MAX_LSB_IMAGE_BYTES as u64)?;
    let (payload_type, payload) = match request.payload {
        LsbExportPayload::MomoCharacter {
            owner_space_id,
            character_id,
        } => {
            let character = core()?
                .store()
                .list_characters_for_scope(owner_space_id)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|character| character.id == character_id)
                .ok_or_else(|| "character does not exist in the requested scope".to_owned())?;
            let bytes = serde_json::to_vec(&json!({
                "schema": crate::MOMO_LSB_CHARACTER_SCHEMA,
                "character": character,
            }))
            .map_err(|error| error.to_string())?;
            (crate::LsbPayloadType::CharacterData, bytes)
        }
        LsbExportPayload::Moc { input_path } => {
            let path = Path::new(&input_path);
            crate::momo_moc::inspect(path).map_err(|error| error.to_string())?;
            (
                crate::LsbPayloadType::Moc,
                read_bounded_file(path, MAX_LSB_SOURCE_FILE_BYTES)?,
            )
        }
        LsbExportPayload::Charx { input_path } => {
            let path = Path::new(&input_path);
            crate::validate_external_charx(path).map_err(|error| error.to_string())?;
            (
                crate::LsbPayloadType::Charx,
                read_bounded_file(path, MAX_LSB_SOURCE_FILE_BYTES)?,
            )
        }
    };
    let output = crate::embed_lsb_image(
        &carrier,
        request.format,
        payload_type,
        &payload,
        request.compress,
    )
    .map_err(|error| error.to_string())?;
    atomic_write(Path::new(&request.output_path), &output)?;
    serde_json::to_string(&LsbFileReport {
        output_path: request.output_path,
        format: request.format,
        payload_type,
        bytes: output.len(),
    })
    .map_err(|error| error.to_string())
}

pub async fn extract_lsb_image_json(request_json: String) -> Result<String, String> {
    let request: ExtractLsbImageRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    validate_image_extension(&request.input_path, request.format)?;
    let input = read_bounded_file(&request.input_path, crate::MAX_LSB_IMAGE_BYTES as u64)?;
    let payload =
        crate::extract_lsb_image(&input, request.format).map_err(|error| error.to_string())?;
    if payload.payload_type != request.expected_payload_type {
        return Err(format!(
            "LSB payload type mismatch: expected {:?}, found {:?}",
            request.expected_payload_type, payload.payload_type
        ));
    }
    atomic_write(Path::new(&request.output_path), &payload.bytes)?;
    serde_json::to_string(&LsbFileReport {
        output_path: request.output_path,
        format: request.format,
        payload_type: payload.payload_type,
        bytes: payload.bytes.len(),
    })
    .map_err(|error| error.to_string())
}

fn validate_image_extension(path: &str, format: crate::LsbImageFormat) -> Result<(), String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid = match format {
        crate::LsbImageFormat::Png => extension == "png",
        crate::LsbImageFormat::WebpLossless => extension == "webp",
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "image extension {extension:?} does not match declared format {format:?}"
        ))
    }
}

fn read_bounded_file(path: impl AsRef<Path>, max_bytes: u64) -> Result<Vec<u8>, String> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("input must be a regular file".to_owned());
    }
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(format!(
            "input size must be between 1 and {max_bytes} bytes"
        ));
    }
    fs::read(path).map_err(|error| error.to_string())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    temporary
        .write_all(bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}
