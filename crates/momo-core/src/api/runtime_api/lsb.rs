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
pub enum LsbExportPayload {
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
    PreservedCharacterSource {
        owner_space_id: uuid::Uuid,
        character_id: uuid::Uuid,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LsbCarrierSource {
    PreservedCharacterSource {
        owner_space_id: uuid::Uuid,
        character_id: uuid::Uuid,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedLsbImageRequest {
    #[serde(default)]
    pub carrier_path: Option<String>,
    #[serde(default)]
    pub carrier: Option<LsbCarrierSource>,
    pub output_path: String,
    pub format: crate::LsbImageFormat,
    pub payload: LsbExportPayload,
    #[serde(default = "default_true")]
    pub compress: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractLsbImageRequest {
    pub input_path: String,
    pub output_path: String,
    pub format: crate::LsbImageFormat,
    pub expected_payload_type: crate::LsbPayloadType,
}

#[derive(Debug, Serialize)]
pub struct LsbFileReport {
    pub output_path: String,
    pub format: crate::LsbImageFormat,
    pub payload_type: crate::LsbPayloadType,
    pub bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_format: Option<crate::ExternalCharacterImportFormat>,
}

pub async fn embed_lsb_image_file(
    runtime: &MomoRuntime,
    request: EmbedLsbImageRequest,
) -> Result<LsbFileReport, RuntimeApiError> {
    embed_lsb_image_with_core(runtime.core(), request).await
}

async fn embed_lsb_image_with_core(
    core: &crate::MomoCore,
    request: EmbedLsbImageRequest,
) -> Result<LsbFileReport, RuntimeApiError> {
    validate_image_extension(&request.output_path, request.format)?;
    let carrier = match (request.carrier_path, request.carrier) {
        (Some(path), None) => {
            validate_image_extension(&path, request.format)?;
            run_blocking("read LSB carrier", move || {
                read_bounded_file(path, crate::MAX_LSB_IMAGE_BYTES as u64)
            })
            .await?
        }
        (
            None,
            Some(LsbCarrierSource::PreservedCharacterSource {
                owner_space_id,
                character_id,
            }),
        ) => {
            let source = crate::read_preserved_character_source(core, owner_space_id, character_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
            if !matches!(
                source.source_format,
                crate::ExternalCharacterImportFormat::Ccv1Png
                    | crate::ExternalCharacterImportFormat::Ccv2Png
                    | crate::ExternalCharacterImportFormat::Ccv3Png
            ) || request.format != crate::LsbImageFormat::Png
            {
                return Err(RuntimeApiError::invalid(
                    "a preserved LSB carrier must be an imported PNG and use format png",
                ));
            }
            source.bytes
        }
        _ => {
            return Err(RuntimeApiError::invalid(
                "exactly one of carrier_path or carrier must be provided",
            ));
        }
    };
    let (payload_type, payload, source_format) = match request.payload {
        LsbExportPayload::MomoCharacter {
            owner_space_id,
            character_id,
        } => {
            let character = core
                .store()
                .list_characters_for_scope(owner_space_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .into_iter()
                .find(|character| character.id == character_id)
                .ok_or_else(|| {
                    RuntimeApiError::not_found("character does not exist in the requested scope")
                })?;
            let bytes = serde_json::to_vec(&json!({
                "schema": crate::MOMO_LSB_CHARACTER_SCHEMA,
                "character": character,
            }))
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
            (crate::LsbPayloadType::CharacterData, bytes, None)
        }
        LsbExportPayload::Moc { input_path } => {
            let bytes = run_blocking("read MOC LSB payload", move || {
                let path = Path::new(&input_path);
                crate::momo_moc::inspect(path)
                    .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
                read_bounded_file(path, MAX_LSB_SOURCE_FILE_BYTES)
            })
            .await?;
            (crate::LsbPayloadType::Moc, bytes, None)
        }
        LsbExportPayload::Charx { input_path } => {
            let bytes = run_blocking("read CHARX LSB payload", move || {
                let path = Path::new(&input_path);
                crate::validate_external_charx(path)
                    .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
                read_bounded_file(path, MAX_LSB_SOURCE_FILE_BYTES)
            })
            .await?;
            (crate::LsbPayloadType::Charx, bytes, None)
        }
        LsbExportPayload::PreservedCharacterSource {
            owner_space_id,
            character_id,
        } => {
            let source = crate::read_preserved_character_source(core, owner_space_id, character_id)
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
            let payload_type = match source.source_format {
                crate::ExternalCharacterImportFormat::Ccv1Json
                | crate::ExternalCharacterImportFormat::Ccv2Json
                | crate::ExternalCharacterImportFormat::Ccv3Json => {
                    crate::LsbPayloadType::ExternalCharacterJson
                }
                crate::ExternalCharacterImportFormat::Ccv1Png
                | crate::ExternalCharacterImportFormat::Ccv2Png
                | crate::ExternalCharacterImportFormat::Ccv3Png => {
                    crate::LsbPayloadType::ExternalCharacterPng
                }
                crate::ExternalCharacterImportFormat::Ccv3Charx => crate::LsbPayloadType::Charx,
            };
            (payload_type, source.bytes, Some(source.source_format))
        }
    };
    run_blocking("embed LSB image", move || {
        let output = crate::embed_lsb_image(
            &carrier,
            request.format,
            payload_type,
            &payload,
            request.compress,
        )
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
        atomic_write(Path::new(&request.output_path), &output)?;
        Ok(LsbFileReport {
            output_path: request.output_path,
            format: request.format,
            payload_type,
            bytes: output.len(),
            source_format,
        })
    })
    .await
}

pub async fn extract_lsb_image_file(
    request: ExtractLsbImageRequest,
) -> Result<LsbFileReport, RuntimeApiError> {
    run_blocking("extract LSB image", move || extract_lsb_image(request)).await
}

fn extract_lsb_image(request: ExtractLsbImageRequest) -> Result<LsbFileReport, RuntimeApiError> {
    validate_image_extension(&request.input_path, request.format)?;
    let input = read_bounded_file(&request.input_path, crate::MAX_LSB_IMAGE_BYTES as u64)?;
    let payload = crate::extract_lsb_image(&input, request.format)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    if payload.payload_type != request.expected_payload_type {
        return Err(RuntimeApiError::invalid(format!(
            "LSB payload type mismatch: expected {:?}, found {:?}",
            request.expected_payload_type, payload.payload_type
        )));
    }
    atomic_write(Path::new(&request.output_path), &payload.bytes)?;
    Ok(LsbFileReport {
        output_path: request.output_path,
        format: request.format,
        payload_type: payload.payload_type,
        bytes: payload.bytes.len(),
        source_format: None,
    })
}

fn validate_image_extension(
    path: &str,
    format: crate::LsbImageFormat,
) -> Result<(), RuntimeApiError> {
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
        Err(RuntimeApiError::invalid(format!(
            "image extension {extension:?} does not match declared format {format:?}"
        )))
    }
}

fn read_bounded_file(path: impl AsRef<Path>, max_bytes: u64) -> Result<Vec<u8>, RuntimeApiError> {
    let path = path.as_ref();
    let metadata =
        fs::symlink_metadata(path).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RuntimeApiError::invalid("input must be a regular file"));
    }
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(RuntimeApiError::invalid(format!(
            "input size must be between 1 and {max_bytes} bytes"
        )));
    }
    fs::read(path).map_err(|error| RuntimeApiError::internal(error.to_string()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), RuntimeApiError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    temporary
        .write_all(bytes)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    temporary
        .persist(path)
        .map_err(|error| RuntimeApiError::internal(error.error.to_string()))?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/api_simple_lsb.rs"]
mod tests;
