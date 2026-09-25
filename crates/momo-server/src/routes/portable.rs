use super::super::*;

pub(crate) async fn export_moc(
    State(state): State<AppState>,
    Json(request): Json<MocExportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::export_moc_file(
            state.runtime.as_ref(),
            runtime_api::ExportMocRequest {
                output_path: request.output_path,
                plan: request.plan,
                protection: request.protection,
                host_modules: request.host_modules,
            },
        )
        .await,
    )
}

pub(crate) async fn import_moc(
    State(state): State<AppState>,
    Json(request): Json<MocImportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::import_moc_file(
            state.runtime.as_ref(),
            request.input_path,
            request.plan,
            request.protection,
            request.claim_unknown_to,
        )
        .await,
    )
}

pub(crate) async fn moc_is_encrypted(
    Query(query): Query<MocEncryptedQuery>,
) -> Result<Json<Value>, ApiError> {
    let encrypted = runtime_api::moc_is_encrypted(query.input_path)
        .await
        .map_err(runtime_api_error)?;
    Ok(Json(json!({
        "encrypted": encrypted,
    })))
}

pub(crate) async fn embed_lsb_image(
    State(state): State<AppState>,
    Json(request): Json<runtime_api::EmbedLsbImageRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(runtime_api::embed_lsb_image_file(state.runtime.as_ref(), request).await)
}

pub(crate) async fn extract_lsb_image(
    Json(request): Json<runtime_api::ExtractLsbImageRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(runtime_api::extract_lsb_image_file(request).await)
}
