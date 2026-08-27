//! Portable `momo.toml` configuration and MOC import/export.

use super::*;

pub async fn export_momo_config_json(
    output_path: String,
    settings_json: String,
) -> Result<(), String> {
    let settings = serde_json::from_str(&settings_json).map_err(|error| error.to_string())?;
    crate::portable::export_momo_config(core()?, output_path, &settings)
        .map_err(|error| error.to_string())
}

pub async fn import_momo_config_json(input_path: String) -> Result<String, String> {
    let settings = crate::portable::import_momo_config(core()?, input_path)
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
    modules: Vec<crate::MocModule>,
    #[serde(default)]
    compatibility: crate::MocCompatibility,
    #[serde(default)]
    character_id: Option<uuid::Uuid>,
    #[serde(default)]
    protection: crate::MocProtection,
}

pub async fn export_moc_json(request_json: String) -> Result<String, String> {
    let request: ExportMocJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let scope_id = uuid::Uuid::parse_str(&request.scope_id).map_err(|error| error.to_string())?;
    let plan = crate::MocExportPlan {
        modules: request.modules,
        character_id: request.character_id,
        compatibility: request.compatibility,
    };
    let manifest = if let Some(passphrase) = request.protection.passphrase() {
        crate::portable::export_private_moc(
            core()?,
            request.output_path,
            scope_id,
            &request.settings,
            &plan,
            passphrase,
        )
        .await
    } else {
        crate::portable::export_moc(
            core()?,
            request.output_path,
            scope_id,
            &request.settings,
            &plan,
        )
        .await
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&manifest).map_err(|error| error.to_string())
}

pub async fn import_moc_json(
    input_path: String,
    scope_id: String,
    conflict_mode: crate::ConflictMode,
    protection: crate::MocProtection,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let report = if let Some(passphrase) = protection.passphrase() {
        crate::portable::import_moc_with_passphrase(
            core()?,
            input_path,
            scope_id,
            conflict_mode,
            Some(passphrase),
        )
        .await
    } else {
        crate::portable::import_moc(core()?, input_path, scope_id, conflict_mode).await
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&report).map_err(|error| error.to_string())
}

pub async fn moc_is_encrypted(input_path: String) -> Result<bool, String> {
    crate::portable::moc_is_encrypted(input_path).map_err(|error| error.to_string())
}
