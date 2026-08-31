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
    #[serde(default)]
    settings: serde_json::Value,
    plan: crate::MocExportPlan,
    #[serde(default)]
    protection: crate::MocProtection,
    #[serde(default)]
    host_modules: Vec<crate::HostMocModule>,
}

pub async fn export_moc_json(request_json: String) -> Result<String, String> {
    let request: ExportMocJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let manifest = if let Some(passphrase) = request.protection.passphrase() {
        crate::portable::export_private_moc_with_host_modules(
            core()?,
            request.output_path,
            &request.settings,
            &request.plan,
            &request.host_modules,
            passphrase,
        )
        .await
    } else {
        crate::portable::export_moc_with_host_modules(
            core()?,
            request.output_path,
            &request.settings,
            &request.plan,
            &request.host_modules,
        )
        .await
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&manifest).map_err(|error| error.to_string())
}

pub async fn import_moc_json(
    input_path: String,
    plan: crate::MocImportPlan,
    protection: crate::MocProtection,
    claim_unknown_to: Option<String>,
) -> Result<String, String> {
    let report = if let Some(passphrase) = protection.passphrase() {
        crate::portable::import_moc_with_passphrase_and_claims(
            core()?,
            input_path,
            &plan,
            Some(passphrase),
            claim_unknown_to.as_deref().map(std::path::Path::new),
        )
        .await
    } else if let Some(claim_directory) = claim_unknown_to {
        crate::portable::import_moc_claiming_unknown_modules(
            core()?,
            input_path,
            &plan,
            claim_directory,
        )
        .await
    } else {
        crate::portable::import_moc(core()?, input_path, &plan).await
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&report).map_err(|error| error.to_string())
}

pub async fn moc_is_encrypted(input_path: String) -> Result<bool, String> {
    crate::portable::moc_is_encrypted(input_path).map_err(|error| error.to_string())
}
