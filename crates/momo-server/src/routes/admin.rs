use super::super::*;

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
) -> Json<MomoRuntimeSettings> {
    Json(state.runtime.runtime_settings())
}

pub(crate) async fn replace_runtime_settings(
    State(state): State<AppState>,
    Json(settings): Json<MomoRuntimeSettings>,
) -> Result<Json<MomoRuntimeSettings>, ApiError> {
    state
        .runtime
        .update_runtime_settings(settings)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(state.runtime.runtime_settings()))
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
