//! MOC import/export.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportMocRequest {
    pub output_path: String,
    pub plan: crate::MocExportPlan,
    #[serde(default)]
    pub protection: crate::MocProtection,
    #[serde(default)]
    pub host_modules: Vec<crate::HostMocModule>,
}

pub async fn export_moc_file(
    runtime: &MomoRuntime,
    request: ExportMocRequest,
) -> Result<momo_moc::Manifest, RuntimeApiError> {
    let manifest = if let Some(passphrase) = request.protection.passphrase() {
        crate::portable::export_private_moc_with_host_modules(
            runtime.core(),
            request.output_path,
            &request.plan,
            &request.host_modules,
            passphrase,
        )
        .await
    } else {
        crate::portable::export_moc_with_host_modules(
            runtime.core(),
            request.output_path,
            &request.plan,
            &request.host_modules,
        )
        .await
    }
    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(manifest)
}

pub async fn import_moc_file(
    runtime: &MomoRuntime,
    input_path: String,
    plan: crate::MocImportPlan,
    protection: crate::MocProtection,
    claim_unknown_to: Option<String>,
) -> Result<crate::ImportReport, RuntimeApiError> {
    let report = if let Some(passphrase) = protection.passphrase() {
        crate::portable::import_moc_with_passphrase_and_claims(
            runtime.core(),
            input_path,
            &plan,
            Some(passphrase),
            claim_unknown_to.as_deref().map(std::path::Path::new),
        )
        .await
    } else if let Some(claim_directory) = claim_unknown_to {
        crate::portable::import_moc_claiming_unknown_modules(
            runtime.core(),
            input_path,
            &plan,
            claim_directory,
        )
        .await
    } else {
        crate::portable::import_moc(runtime.core(), input_path, &plan).await
    }
    .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(report)
}

pub async fn moc_is_encrypted(input_path: String) -> Result<bool, RuntimeApiError> {
    crate::portable::moc_is_encrypted(input_path)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}
