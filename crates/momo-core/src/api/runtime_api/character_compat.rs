//! JSON facade for external Character Card import and export.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportExternalCharacterRequest {
    pub scope_id: String,
    pub input_path: String,
    pub format: crate::ExternalCharacterImportFormat,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportExternalCharacterRequest {
    pub scope_id: String,
    pub character_id: String,
    pub output_path: String,
    pub format: crate::ExternalCharacterExportFormat,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPreservedCharacterSourceRequest {
    pub scope_id: String,
    pub character_id: String,
    pub output_path: String,
}

pub async fn import_external_character_file(
    runtime: &MomoRuntime,
    request: ImportExternalCharacterRequest,
) -> Result<crate::ExternalCharacterImport, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&request.scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let imported = crate::import_external_character(
        runtime.core(),
        scope_id,
        request.input_path,
        request.format,
    )
    .await
    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(imported)
}

pub async fn export_external_character_file(
    runtime: &MomoRuntime,
    request: ExportExternalCharacterRequest,
) -> Result<serde_json::Value, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&request.scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let character_id = uuid::Uuid::parse_str(&request.character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if runtime
        .core()
        .store()
        .character_for_scope(scope_id, character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "character does not exist in the requested scope",
        ));
    }
    crate::export_external_character(
        runtime.core(),
        scope_id,
        character_id,
        request.output_path,
        request.format,
    )
    .await
    .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn export_preserved_character_source_file(
    runtime: &MomoRuntime,
    request: ExportPreservedCharacterSourceRequest,
) -> Result<crate::PreservedCharacterSourceExport, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&request.scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let character_id = uuid::Uuid::parse_str(&request.character_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    if runtime
        .core()
        .store()
        .character_for_scope(scope_id, character_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(RuntimeApiError::not_found(
            "character does not exist in the requested scope",
        ));
    }
    let report = crate::export_preserved_character_source(
        runtime.core(),
        scope_id,
        character_id,
        request.output_path,
    )
    .await
    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(report)
}
