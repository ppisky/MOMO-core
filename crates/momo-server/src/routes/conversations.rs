use super::super::*;

pub(crate) async fn list_conversations(
    State(state): State<AppState>,
    Query(query): Query<ConversationsRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(query.space_id)?;
    json_result(match query.archived {
        Some(archived) => {
            runtime_api::local_conversations_by_archive(state.runtime.as_ref(), scope_id, archived)
                .await
        }
        None => runtime_api::local_conversations(state.runtime.as_ref(), scope_id).await,
    })
}

pub(crate) async fn archive_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ArchiveConversationRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        runtime_api::set_conversation_archived(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            id,
            request.archived,
        )
        .await,
    )
}

pub(crate) async fn create_conversation(
    State(state): State<AppState>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    scoped_json_result(
        runtime_api::stage_conversation(
            state.runtime.as_ref(),
            None,
            scope_id,
            request.title,
            request.character_id,
        )
        .await,
    )
}

pub(crate) async fn update_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(conversation): Json<Conversation>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, conversation.id)?;
    scoped_json_result(
        runtime_api::stage_conversation_update(
            state.runtime.as_ref(),
            conversation.scope_id.to_string(),
            conversation,
        )
        .await,
    )
}

pub(crate) async fn delete_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    runtime_api::stage_conversation_delete(
        state.runtime.as_ref(),
        validate_space_id(query.space_id)?,
        id,
    )
    .await
    .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

pub(crate) async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        runtime_api::local_messages(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}

pub(crate) async fn create_message(
    State(state): State<AppState>,
    Json(request): Json<CreateMessageRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        runtime_api::stage_message(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            request.conversation_id,
            request.role,
            request.content,
        )
        .await,
    )
}

pub(crate) async fn update_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
    Json(message): Json<Message>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, message.id)?;
    scoped_json_result(
        runtime_api::stage_message_update(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            message,
        )
        .await,
    )
}

pub(crate) async fn delete_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    runtime_api::stage_message_delete(
        state.runtime.as_ref(),
        validate_space_id(query.space_id)?,
        id,
    )
    .await
    .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}
