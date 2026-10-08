use super::super::*;

/// Lifecycle operations take the Core writer gate themselves. Other HTTP work
/// keeps a reader through its handler; SSE producers clone that same admission.
pub(crate) async fn space_admission(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.uri().path().starts_with("/v1/spaces/")
        || request.uri().path().starts_with("/v1/momo/responses/")
            && request.uri().path().ends_with("/cancel")
    {
        return next.run(request).await;
    }
    let admission = Arc::new(state.runtime.core().admit_space_operation().await);
    if state.runtime.core().space_transition_pending()
        && request.method() != axum::http::Method::GET
        && request.method() != axum::http::Method::HEAD
    {
        use axum::response::IntoResponse;
        return ApiError::new(
            StatusCode::CONFLICT,
            "space_operation_pending",
            "Finish the prepared Space operation before changing data",
            true,
        )
        .into_response();
    }
    request.extensions_mut().insert(admission.clone());
    let response = next.run(request).await;
    drop(admission);
    response
}

pub(crate) async fn get_space_assets(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Value>, ApiError> {
    json_result(state.runtime.core().space_assets(id).await)
}

pub(crate) async fn remove_space_assets(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(request): Json<momo_core::SpaceRemovalRequest>,
) -> Result<Json<Value>, ApiError> {
    if id != request.source_id {
        return Err(ApiError::bad_request("Space ID does not match removal"));
    }
    // No new HTTP work can enter after acquiring the writer. Background
    // maintenance shares Space reservations and must finish before transition.
    json_result(state.runtime.remove_space_assets(request).await)
}
