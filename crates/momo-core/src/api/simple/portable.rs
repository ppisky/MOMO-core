//! Runtime configuration and portable MOC import/export.

use super::*;

pub async fn export_runtime_config_json(
    output_path: String,
    settings_json: String,
) -> Result<(), String> {
    let settings = serde_json::from_str(&settings_json).map_err(|error| error.to_string())?;
    crate::portable::export_runtime_config(core()?, output_path, &settings)
        .map_err(|error| error.to_string())
}

pub async fn import_runtime_config_json(input_path: String) -> Result<String, String> {
    let settings = crate::portable::import_runtime_config(core()?, input_path)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&settings).map_err(|error| error.to_string())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportMocJsonRequest {
    output_path: String,
    scope_id: String,
    #[serde(default)]
    settings: serde_json::Value,
    include_config: bool,
    include_characters: bool,
    include_conversations: bool,
    include_memory: bool,
    include_semantic_graph: bool,
    passphrase: Option<String>,
}

pub async fn export_moc_json(request_json: String) -> Result<String, String> {
    let request: ExportMocJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let scope_id = uuid::Uuid::parse_str(&request.scope_id).map_err(|error| error.to_string())?;
    let selection = crate::ExportSelection {
        config: request.include_config,
        characters: request.include_characters,
        conversations: request.include_conversations,
        memory: request.include_memory,
        semantic_graph: request.include_semantic_graph,
        character_id: None,
    };
    let manifest = if let Some(passphrase) = request.passphrase.filter(|value| !value.is_empty()) {
        crate::portable::export_private_moc(
            core()?,
            request.output_path,
            scope_id,
            &request.settings,
            selection,
            &passphrase,
        )
        .await
    } else {
        crate::portable::export_moc(
            core()?,
            request.output_path,
            scope_id,
            &request.settings,
            selection,
        )
        .await
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&manifest).map_err(|error| error.to_string())
}

pub async fn export_character_moc_json(
    output_path: String,
    scope_id: String,
    character_id: String,
    passphrase: Option<String>,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let character_id = uuid::Uuid::parse_str(&character_id).map_err(|error| error.to_string())?;
    let selection = crate::ExportSelection {
        config: false,
        characters: true,
        conversations: false,
        memory: false,
        semantic_graph: false,
        character_id: Some(character_id),
    };
    let settings = serde_json::json!({});
    let manifest = if let Some(passphrase) = passphrase.filter(|value| !value.is_empty()) {
        crate::portable::export_private_moc(
            core()?,
            output_path,
            scope_id,
            &settings,
            selection,
            &passphrase,
        )
        .await
    } else {
        crate::portable::export_moc(core()?, output_path, scope_id, &settings, selection).await
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&manifest).map_err(|error| error.to_string())
}

pub async fn import_moc_json(
    input_path: String,
    scope_id: String,
    conflict_mode: String,
    passphrase: Option<String>,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let report = if let Some(passphrase) = passphrase {
        crate::portable::import_moc_with_passphrase(
            core()?,
            input_path,
            scope_id,
            &conflict_mode,
            Some(&passphrase),
        )
        .await
    } else {
        crate::portable::import_moc(core()?, input_path, scope_id, &conflict_mode).await
    }
    .map_err(|error| error.to_string())?;
    if report.memory_files_imported > 0 {
        stage_memory_snapshot(scope_id).await?;
    }
    if report.semantic_graph_files_imported > 0 {
        stage_semantic_graph_snapshot(scope_id).await?;
    }
    serde_json::to_string(&report).map_err(|error| error.to_string())
}

pub async fn moc_is_encrypted(input_path: String) -> Result<bool, String> {
    crate::portable::moc_is_encrypted(input_path).map_err(|error| error.to_string())
}
