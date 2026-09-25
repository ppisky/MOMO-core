use super::super::*;

pub(crate) async fn memory_recovery_status(State(state): State<AppState>) -> Json<Value> {
    Json(json!(runtime_api::memory_recovery_status(
        state.runtime.as_ref()
    )))
}

pub(crate) async fn retry_memory_recovery(
    State(state): State<AppState>,
    Path(space_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let space_id = uuid::Uuid::parse_str(&space_id)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    ok_json(runtime_api::retry_memory_recovery(state.runtime.as_ref(), space_id).await)
}

pub(crate) async fn generate_embeddings(
    Json(request): Json<runtime_api::GenerateEmbeddingsRequest>,
) -> Result<Json<momo_core::EmbeddingBatch>, ApiError> {
    runtime_api::generate_embeddings(request)
        .await
        .map(Json)
        .map_err(embedding_api_error)
}

pub(crate) async fn prepare_context(
    Json(request): Json<PrepareContextRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(runtime_api::prepare_context_request(
        runtime_api::PrepareContextRequest {
            runtime_instructions: request.runtime_instructions,
            character_markdown: request.character_markdown,
            user_markdown: request.user_markdown,
            memory_markdown: request.memory_markdown,
            state_context: request.state_context,
            nsg_markdown: request.nsg_markdown,
            messages: request.messages,
            context_window: request.context_window,
            reserve_output_tokens: request.reserve_output_tokens,
            roleplay_director: String::new(),
        },
    ))
}

pub(crate) async fn resolve_capability(
    State(state): State<AppState>,
    Json(request): Json<ResolveCapabilityRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::resolve_capability(state.runtime.as_ref(), request.provider_id, request.model)
            .await,
    )
}

pub(crate) async fn compile_mo_state(
    State(state): State<AppState>,
    Json(request): Json<CompileMoStateRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    json_result(
        runtime_api::compile_mo_state_with_ddm(
            state.runtime.as_ref(),
            uuid::Uuid::parse_str(&scope_id).map_err(|e| ApiError::bad_request(e.to_string()))?,
            request.retrieved_memory,
            request.retrieved_nsg,
            request.max_context_tokens,
            None,
            None,
        )
        .await,
    )
}

pub(crate) async fn get_mo_state_runtime(
    State(state): State<AppState>,
    Query(request): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::mo_state_runtime_status(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
        )
        .await,
    )
}

pub(crate) async fn retrieve_scoped_memory(
    State(state): State<AppState>,
    Json(request): Json<RetrieveScopedMemoryRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::retrieve_scoped_memory(
            state.runtime.as_ref(),
            runtime_api::ScopedMemoryRequest {
                spaces: request.spaces,
                query: request.query,
                max_tokens: request.max_tokens,
                vector_space_id: request.vector_space_id,
                query_vector: request.query_vector,
                embedding: request.embedding,
                observe_space_ids: Vec::new(),
            },
        )
        .await,
    )
}

pub(crate) async fn run_memory_maintenance(
    State(state): State<AppState>,
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::run_memory_maintenance(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
        )
        .await,
    )
}

/// Records an already-completed turn for trusted local imports and benchmark
/// replay. It never invokes the conversation model; a later drain runs the
/// same DMW/NSG maintenance path as a native response.
pub(crate) async fn record_momo_maintenance_turn(
    State(state): State<AppState>,
    Json(request): Json<RecordMaintenanceTurnRequest>,
) -> Result<Json<Value>, ApiError> {
    let space_id = validate_space_id(request.space_id)?;
    if request.request_id.trim().is_empty() || request.request_id.len() > MAX_RESPONSE_ID_BYTES {
        return Err(ApiError::bad_request("invalid maintenance turn request_id"));
    }
    if request.user_content.trim().is_empty() {
        return Err(ApiError::bad_request(
            "maintenance turn user_content must not be empty",
        ));
    }
    if request.user_content.len() > MAX_RESPONSE_INPUT_BYTES
        || request.assistant_content.len() > MAX_RESPONSE_INPUT_BYTES
    {
        return Err(ApiError::bad_request(
            "maintenance turn content exceeds the local replay limit",
        ));
    }
    runtime_api::append_maintenance_turn(
        state.runtime.as_ref(),
        momo_core::momo_storage::MaintenanceTurn {
            request_id: request.request_id,
            scope_id: space_id,
            user_content: request.user_content,
            assistant_content: request.assistant_content,
        },
        request.memory_enabled,
        request.nsg_enabled,
    )
    .await
    .map_err(runtime_api_error)?;
    Ok(Json(json!({"recorded": true})))
}

/// Local management barrier for reproducible evaluations. Flush even a partial
/// batch before probing a new conversation; failures must not look like success.
pub(crate) async fn drain_momo_maintenance(
    State(state): State<AppState>,
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let space_id = validate_space_id(request.space_id)?;
    drain_momo_space(&state, &space_id).await?;
    Ok(Json(json!({"completed": true, "space_id": space_id})))
}

pub(crate) async fn drain_momo_space(state: &AppState, space_id: &str) -> Result<(), ApiError> {
    // Commit each independently acknowledged maintenance lane before starting
    // the next. If the model gateway fails on NSG, a later drain resumes only
    // NSG instead of cancelling an otherwise successful DMW batch.
    for kind in [
        momo_core::MaintenanceKind::Memory,
        momo_core::MaintenanceKind::SemanticGraph,
    ] {
        let lane = match kind {
            momo_core::MaintenanceKind::Memory => "memory",
            momo_core::MaintenanceKind::SemanticGraph => "semantic_graph",
        };
        tokio::time::timeout(
            state.response_timeout,
            drain_momo_kind(state, space_id, kind),
        )
        .await
        .map_err(|_| ApiError::gateway_timeout(format!("maintenance {lane} drain timed out")))??;
    }
    Ok(())
}

pub(crate) async fn drain_momo_kind(
    state: &AppState,
    space_id: &str,
    kind: momo_core::MaintenanceKind,
) -> Result<(), ApiError> {
    let storage_kind = match kind {
        momo_core::MaintenanceKind::Memory => "memory",
        momo_core::MaintenanceKind::SemanticGraph => "semantic_graph",
    };
    let batch_limit = state.momo_api.maintenance_batch_limit(kind);
    for batch in 0..=64 {
        let pending = runtime_api::pending_maintenance_turns(
            state.runtime.as_ref(),
            space_id.to_owned(),
            storage_kind.to_owned(),
            batch_limit,
        )
        .await
        .map_err(runtime_api_error)?;
        if pending.is_empty() {
            break;
        }
        if batch == 64 {
            return Err(ApiError::conflict(
                "maintenance drain limit reached; stop concurrent writes and retry",
            ));
        }
        state
            .momo_api
            .maintain(space_id, kind, pending.len())
            .await
            .map_err(response_api::momo_api_error)?;
    }
    Ok(())
}

pub(crate) async fn list_memory_documents(
    State(state): State<AppState>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::list_memory_documents(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
        )
        .await,
    )
}

pub(crate) async fn read_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::read_memory_document(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}

pub(crate) async fn update_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateMemoryDocumentRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::update_memory_document(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            id,
            request.markdown,
        )
        .await,
    )
}

pub(crate) async fn archive_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::archive_memory_document(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}

pub(crate) async fn restore_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::restore_memory_document(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}

pub(crate) async fn delete_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        runtime_api::delete_memory_document(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}

pub(crate) async fn apply_memory_patch(
    State(state): State<AppState>,
    Json(request): Json<ApplyMemoryPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(
        runtime_api::apply_memory_patch(state.runtime.as_ref(), scope_id, request.patch_yaml).await,
    )
}

pub(crate) async fn submit_memory_patch_review(
    State(state): State<AppState>,
    Json(request): Json<SubmitMemoryPatchReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::submit_memory_patch_review(
            state.runtime.as_ref(),
            validate_space_id(request.space_id)?,
            request.conversation_id,
            request.patch_yaml,
            request.review_mode,
        )
        .await,
    )
}

pub(crate) async fn list_memory_patch_reviews(
    State(state): State<AppState>,
    Query(query): Query<IncludeResolvedQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::list_memory_patch_reviews(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            query.include_resolved.unwrap_or(false),
        )
        .await,
    )
}

pub(crate) async fn approve_memory_patch_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::approve_memory_patch_review(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}

pub(crate) async fn reject_memory_patch_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        runtime_api::reject_memory_patch_review(
            state.runtime.as_ref(),
            validate_space_id(query.space_id)?,
            id,
        )
        .await,
    )
}
