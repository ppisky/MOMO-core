use super::super::*;
use axum::http::{HeaderMap, HeaderValue, header};

pub(crate) async fn get_model_configuration(
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    state
        .model_connections
        .public()
        .map(Json)
        .map_err(settings_error)
}
pub(crate) async fn discover_provider_models(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Value>, ApiError> {
    state
        .model_connections
        .discover(id)
        .await
        .map(Json)
        .map_err(settings_error)
}
pub(crate) async fn put_model_configuration(
    State(state): State<AppState>,
    Json(value): Json<momo_core::ModelConfiguration>,
) -> Result<Json<Value>, ApiError> {
    tokio::task::spawn_blocking(move || state.model_connections.update(value))
        .await
        .map_err(|_| ApiError::internal("model configuration task failed"))?
        .map(Json)
        .map_err(settings_error)
}

fn settings_revision(headers: &HeaderMap) -> Result<Option<u64>, ApiError> {
    headers
        .get(header::IF_MATCH)
        .map(|value| {
            let value = value
                .to_str()
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            value
                .trim_matches('"')
                .parse()
                .map_err(|_| ApiError::bad_request("If-Match must contain a settings revision"))
        })
        .transpose()
}

fn settings_error(error: momo_core::GovernanceError) -> ApiError {
    match error {
        momo_core::GovernanceError::Conflict { .. } => ApiError::new(
            StatusCode::CONFLICT,
            "revision_conflict",
            error.to_string(),
            false,
        ),
        momo_core::GovernanceError::Persistence(_) => ApiError::internal(error.to_string()),
        _ => ApiError::bad_request(error.to_string()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceStateContractRequest {
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedDdmQuery {
    space_id: uuid::Uuid,
    character_id: uuid::Uuid,
}

pub(crate) async fn get_managed_ddm_profile(
    State(state): State<AppState>,
    Query(query): Query<ManagedDdmQuery>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        runtime_api::managed_ddm_profile(
            state.runtime.as_ref(),
            query.space_id,
            query.character_id,
        )
        .await,
    )
}

pub(crate) async fn reset_managed_ddm_profile(
    State(state): State<AppState>,
    Query(query): Query<ManagedDdmQuery>,
) -> Result<Json<Value>, ApiError> {
    runtime_api::reset_managed_ddm_profile(
        state.runtime.as_ref(),
        query.space_id,
        query.character_id,
    )
    .await
    .map_err(scoped_api_error)?;
    Ok(Json(json!({"ok":true})))
}

fn contract_revision(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    headers
        .get(header::IF_MATCH)
        .map(|value| {
            value
                .to_str()
                .map(|v| v.trim_matches('"').to_owned())
                .map_err(|e| ApiError::bad_request(e.to_string()))
        })
        .transpose()
}

pub(crate) async fn get_state_contract(
    State(state): State<AppState>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let id = uuid::Uuid::parse_str(&validate_space_id(query.space_id)?)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    scoped_json_result(runtime_api::mo_state_contract(state.runtime.as_ref(), id).await)
}

pub(crate) async fn replace_state_contract(
    State(state): State<AppState>,
    Query(query): Query<SpaceRequest>,
    headers: HeaderMap,
    Json(request): Json<ReplaceStateContractRequest>,
) -> Result<Json<Value>, ApiError> {
    let id = uuid::Uuid::parse_str(&validate_space_id(query.space_id)?)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    scoped_json_result(
        runtime_api::replace_mo_state_contract(
            state.runtime.as_ref(),
            id,
            Some(request.content),
            contract_revision(&headers)?,
        )
        .await,
    )
}

pub(crate) async fn reset_state_contract(
    State(state): State<AppState>,
    Query(query): Query<SpaceRequest>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let id = uuid::Uuid::parse_str(&validate_space_id(query.space_id)?)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    scoped_json_result(
        runtime_api::replace_mo_state_contract(
            state.runtime.as_ref(),
            id,
            None,
            contract_revision(&headers)?,
        )
        .await,
    )
}

fn settings_response(
    resource: momo_core::RuntimeSettingsResource,
) -> (HeaderMap, Json<MomoRuntimeSettings>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", resource.revision)).expect("revision"),
    );
    headers.insert(
        "x-momo-settings-source",
        HeaderValue::from_str(&resource.source).expect("source"),
    );
    (headers, Json(resource.settings))
}

pub(crate) async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "momo-server",
        core_version: runtime_api::core_version(),
        data_dir: state.runtime.data_dir().to_string_lossy().into_owned(),
    })
}

pub(crate) async fn list_prompt_spaces(State(state): State<AppState>) -> Json<Vec<PromptSpace>> {
    Json(state.runtime.prompt_spaces().list())
}

pub(crate) async fn get_runtime_settings(
    State(state): State<AppState>,
) -> (HeaderMap, Json<MomoRuntimeSettings>) {
    settings_response(state.runtime.runtime_settings_resource(None))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsMetadataQuery {
    space_id: Option<uuid::Uuid>,
}

pub(crate) async fn get_settings_description() -> Json<Value> {
    Json(momo_core::runtime_settings_description())
}

pub(crate) async fn get_settings_metadata(
    State(state): State<AppState>,
    Query(query): Query<SettingsMetadataQuery>,
) -> Json<momo_core::RuntimeSettingsResource> {
    Json(state.runtime.runtime_settings_resource(query.space_id))
}

pub(crate) async fn replace_runtime_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<MomoRuntimeSettings>,
) -> Result<(HeaderMap, Json<MomoRuntimeSettings>), ApiError> {
    state
        .runtime
        .replace_runtime_settings(None, settings, settings_revision(&headers)?)
        .map(settings_response)
        .map_err(settings_error)
}

pub(crate) async fn patch_runtime_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<Value>,
) -> Result<(HeaderMap, Json<MomoRuntimeSettings>), ApiError> {
    state
        .runtime
        .patch_runtime_settings(None, patch, settings_revision(&headers)?)
        .map(settings_response)
        .map_err(settings_error)
}

pub(crate) async fn get_space_runtime_settings(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> (HeaderMap, Json<MomoRuntimeSettings>) {
    settings_response(state.runtime.runtime_settings_resource(Some(id)))
}

pub(crate) async fn replace_space_runtime_settings(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Json(settings): Json<MomoRuntimeSettings>,
) -> Result<(HeaderMap, Json<MomoRuntimeSettings>), ApiError> {
    state
        .runtime
        .replace_runtime_settings(Some(id), settings, settings_revision(&headers)?)
        .map(settings_response)
        .map_err(settings_error)
}

pub(crate) async fn patch_space_runtime_settings(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Json(patch): Json<Value>,
) -> Result<(HeaderMap, Json<MomoRuntimeSettings>), ApiError> {
    state
        .runtime
        .patch_runtime_settings(Some(id), patch, settings_revision(&headers)?)
        .map(settings_response)
        .map_err(settings_error)
}

pub(crate) async fn reset_space_runtime_settings(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Json<MomoRuntimeSettings>), ApiError> {
    state
        .runtime
        .reset_runtime_settings(Some(id), settings_revision(&headers)?)
        .map(settings_response)
        .map_err(settings_error)
}

pub(crate) async fn get_prompt_space(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PromptSpace>, ApiError> {
    Ok(Json(
        state
            .runtime
            .prompt_spaces()
            .get(parse_prompt_space_id(&id)?),
    ))
}

pub(crate) async fn replace_prompt_space(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReplacePromptSpaceRequest>,
) -> Result<Json<PromptSpace>, ApiError> {
    state
        .runtime
        .prompt_spaces()
        .replace(parse_prompt_space_id(&id)?, request.content)
        .map(Json)
        .map_err(prompt_spaces_api_error)
}

pub(crate) async fn reset_prompt_space(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PromptSpace>, ApiError> {
    state
        .runtime
        .prompt_spaces()
        .reset(parse_prompt_space_id(&id)?)
        .map(Json)
        .map_err(prompt_spaces_api_error)
}

pub(crate) fn parse_prompt_space_id(id: &str) -> Result<PromptSpaceId, ApiError> {
    id.parse().map_err(prompt_spaces_api_error)
}

pub(crate) fn prompt_spaces_api_error(error: PromptSpacesError) -> ApiError {
    match error {
        PromptSpacesError::Unknown(_) => ApiError::not_found(error.to_string()),
        PromptSpacesError::InvalidContent { .. } => ApiError::bad_request(error.to_string()),
        PromptSpacesError::UnsupportedSchema(_)
        | PromptSpacesError::Io(_)
        | PromptSpacesError::Json(_) => ApiError::internal(error.to_string()),
    }
}

pub(crate) async fn execute_control(
    State(state): State<AppState>,
    payload: Result<Json<momo_core::MomoControlRequest>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(request) = payload.map_err(|rejection| {
        let status = rejection.status();
        let code = if status == StatusCode::PAYLOAD_TOO_LARGE {
            "request_too_large"
        } else {
            "invalid_json"
        };
        ApiError::new(status, code, rejection.body_text(), false)
    })?;
    request.validate().map_err(ApiError::bad_request)?;
    runtime_api::execute_control(state.runtime.as_ref(), request)
        .await
        .map(Json)
        .map_err(control_api_error)
}

pub(crate) async fn metrics(State(state): State<AppState>) -> Json<Value> {
    let routes = state.metrics.lock().await.clone();
    Json(json!({
        "schema": "momo.metrics/1.0",
        "routes": routes,
    }))
}

pub(crate) async fn update_route_metrics(
    state: &AppState,
    route: &str,
    update: impl FnOnce(&mut RouteMetrics),
) {
    let mut metrics = state.metrics.lock().await;
    update(metrics.entry(route.to_owned()).or_default());
}
