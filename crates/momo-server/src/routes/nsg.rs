use super::super::*;

pub(crate) async fn list_nsg_nodes(
    State(state): State<AppState>,
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    json_result(runtime_api::list_nsg_nodes(state.runtime.as_ref(), scope_id, false).await)
}

pub(crate) async fn write_nsg_node(
    State(state): State<AppState>,
    Json(request): Json<WriteNsgNodeRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::write_nsg_node(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            request.target_file,
            request.node,
        )
        .await,
    )
}

pub(crate) async fn archive_nsg_node(
    State(state): State<AppState>,
    Json(request): Json<NsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::archive_nsg_node(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            request.target_file,
        )
        .await,
    )
}

pub(crate) async fn delete_nsg_node(
    State(state): State<AppState>,
    Json(request): Json<NsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::delete_nsg_node(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            request.target_file,
        )
        .await,
    )
}

pub(crate) async fn apply_nsg_patch(
    State(state): State<AppState>,
    Json(request): Json<ApplyNsgPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(
        runtime_api::apply_nsg_patch(
            state.runtime.as_ref(),
            scope_id,
            request.patch_yaml,
            request.manual_authority,
        )
        .await,
    )
}

pub(crate) async fn list_nsg_pending_candidates(
    State(state): State<AppState>,
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    json_result(runtime_api::list_nsg_pending_candidates(state.runtime.as_ref(), scope_id).await)
}

pub(crate) async fn approve_nsg_pending_candidate(
    State(state): State<AppState>,
    Json(request): Json<SpaceNsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(
        runtime_api::approve_nsg_pending_candidate(
            state.runtime.as_ref(),
            scope_id,
            request.target_file,
        )
        .await,
    )
}

pub(crate) async fn reject_nsg_pending_candidate(
    State(state): State<AppState>,
    Json(request): Json<SpaceNsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(
        runtime_api::reject_nsg_pending_candidate(
            state.runtime.as_ref(),
            scope_id,
            request.target_file,
        )
        .await,
    )
}

pub(crate) async fn nsg_vector_status(
    State(state): State<AppState>,
    Query(query): Query<NsgVectorStatusQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::nsg_vector_status(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            query.vector_space_id.unwrap_or_default(),
        )
        .await,
    )
}

#[derive(Deserialize)]
pub(crate) struct RebuildIndexRequest {
    space_id: String,
    #[serde(flatten)]
    index: runtime_api::RebuildNsgIndexRequest,
}

pub(crate) async fn rebuild_nsg_vector_index(
    State(state): State<AppState>,
    Json(request): Json<RebuildIndexRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::rebuild_nsg_vector_index(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            request.index,
        )
        .await,
    )
}
