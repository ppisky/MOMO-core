//! JSON facade for external Character Card import and export.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportExternalCharacterRequest {
    scope_id: String,
    input_path: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportExternalCharacterRequest {
    scope_id: String,
    character_id: String,
    output_path: String,
    format: String,
}

pub async fn import_external_character_json(request_json: String) -> Result<String, String> {
    let request: ImportExternalCharacterRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let scope_id = uuid::Uuid::parse_str(&request.scope_id).map_err(|error| error.to_string())?;
    let imported = crate::import_external_character(core()?, scope_id, request.input_path)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&imported).map_err(|error| error.to_string())
}

pub async fn export_external_character_json(request_json: String) -> Result<String, String> {
    let request: ExportExternalCharacterRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let scope_id = uuid::Uuid::parse_str(&request.scope_id).map_err(|error| error.to_string())?;
    let character_id =
        uuid::Uuid::parse_str(&request.character_id).map_err(|error| error.to_string())?;
    let format = request
        .format
        .parse()
        .map_err(|error: crate::CharacterCompatError| error.to_string())?;
    crate::export_external_character(core()?, scope_id, character_id, request.output_path, format)
        .await
        .map(|report| report.to_string())
        .map_err(|error| error.to_string())
}
