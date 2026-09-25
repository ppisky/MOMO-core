use super::super::*;

pub(crate) async fn list_characters(
    State(state): State<AppState>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::local_characters(state.runtime.as_ref(), validate_space_id(query.space_id)?)
            .await,
    )
}

pub(crate) async fn create_character(
    State(state): State<AppState>,
    Json(request): Json<CreateCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    json_result(
        runtime_api::stage_character(
            state.runtime.as_ref(),
            scope_id,
            request.author_name,
            request.name,
            request.character_markdown,
            request.user_markdown,
        )
        .await,
    )
}

pub(crate) async fn get_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(runtime_api::local_character(state.runtime.as_ref(), id).await)
}

pub(crate) async fn import_external_character(
    State(state): State<AppState>,
    Json(request): Json<ImportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    json_result(
        runtime_api::import_external_character_file(
            state.runtime.as_ref(),
            runtime_api::ImportExternalCharacterRequest {
                scope_id,
                input_path: request.input_path,
                format: request.format,
            },
        )
        .await,
    )
}

pub(crate) async fn export_external_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ExportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    scoped_json_result(
        runtime_api::export_external_character_file(
            state.runtime.as_ref(),
            runtime_api::ExportExternalCharacterRequest {
                scope_id,
                character_id: id,
                output_path: request.output_path,
                format: request.format,
            },
        )
        .await,
    )
}

pub(crate) async fn export_preserved_character_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ExportPreservedCharacterSourceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    scoped_json_result(
        runtime_api::export_preserved_character_source_file(
            state.runtime.as_ref(),
            runtime_api::ExportPreservedCharacterSourceRequest {
                scope_id,
                character_id: id,
                output_path: request.output_path,
            },
        )
        .await,
    )
}

pub(crate) async fn update_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(character): Json<CharacterCard>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, character.id)?;
    scoped_json_result(
        runtime_api::stage_character_update(
            state.runtime.as_ref(),
            character.scope_id.to_string(),
            character,
        )
        .await,
    )
}

pub(crate) async fn delete_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    runtime_api::stage_character_delete(
        state.runtime.as_ref(),
        validate_space_id(query.space_id)?,
        id,
    )
    .await
    .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

pub(crate) async fn get_character_ddm_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(query.space_id)?;
    let character = runtime_api::local_character(state.runtime.as_ref(), id.clone())
        .await
        .map_err(scoped_api_error)?;
    if character.scope_id.to_string() != scope_id {
        return Err(ApiError::not_found("character does not belong to scope"));
    }
    let profile = runtime_api::character_ddm_profile_yaml(state.runtime.as_ref(), id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(json!({"profile_yaml": profile})))
}

pub(crate) async fn update_character_ddm_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<DdmProfileRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        runtime_api::upsert_character_ddm_profile(
            state.runtime.as_ref(),
            validate_space_id(request.owner_space_id)?,
            id,
            request.profile_yaml,
        )
        .await,
    )
}

pub(crate) async fn delete_character_ddm_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    runtime_api::delete_character_ddm_profile(
        state.runtime.as_ref(),
        validate_space_id(query.space_id)?,
        id,
    )
    .await
    .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}
