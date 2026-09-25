#[cfg(test)]
use std::path::PathBuf;
use std::{collections::HashMap, env, net::SocketAddr, sync::Arc};

mod dto;
mod routes;
mod startup;

use dto::*;
use routes::*;
#[cfg(test)]
use startup::ensure_bind_allowed;
pub use startup::run;

mod http_error;
mod response_api;
mod response_stream;

use http_error::ApiError;
#[cfg(test)]
use http_error::sanitize_error_message;
#[cfg(test)]
use response_api::model_api_error;
use response_api::{cancel_response, create_response};

use axum::http::StatusCode;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, rejection::JsonRejection},
    routing::{get, post, put},
};
use momo_core::{
    MAX_RESPONSE_ID_BYTES, MAX_RESPONSE_INPUT_BYTES, MAX_RESPONSE_REQUEST_BYTES, MomoApiService,
    MomoRuntime, MomoRuntimeSettings, PromptSpace, PromptSpaceId, PromptSpacesError,
    api::runtime_api,
    momo_domain::{CharacterCard, Conversation, Message},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore};
use tower_http::trace::TraceLayer;

const DEFAULT_BIND: &str = "127.0.0.1:8765";
const DEFAULT_DATA_DIR: &str = ".momo-data";
#[cfg(test)]
const TEST_SCOPE_ID: &str = "01900000-0000-7000-8000-000000000101";
const ALLOW_REMOTE_ENV: &str = "MOMO_SERVER_ALLOW_REMOTE";
const MAX_PUBLIC_ERROR_BYTES: usize = 2 * 1024;

#[derive(Clone)]
struct AppState {
    runtime: Arc<MomoRuntime>,
    momo_api: Arc<MomoApiService>,
    response_concurrency: Arc<Semaphore>,
    response_timeout: std::time::Duration,
    metrics: Arc<Mutex<HashMap<String, RouteMetrics>>>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RouteMetrics {
    requests: u64,
    succeeded: u64,
    failed: u64,
    cancelled: u64,
    timed_out: u64,
    input_tokens: u64,
    output_tokens: u64,
    latency_ms: u64,
    inflight: u64,
}

fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .nest("/v1", api_routes())
        .layer(DefaultBodyLimit::max(MAX_RESPONSE_REQUEST_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/characters", get(list_characters).post(create_character))
        .route(
            "/characters/import-external",
            post(import_external_character),
        )
        .route(
            "/characters/:id/export-external",
            post(export_external_character),
        )
        .route(
            "/characters/:id/export-stored-source",
            post(export_preserved_character_source),
        )
        .route(
            "/characters/:id",
            get(get_character)
                .put(update_character)
                .delete(delete_character),
        )
        .route(
            "/characters/:id/ddm-profile",
            get(get_character_ddm_profile)
                .put(update_character_ddm_profile)
                .delete(delete_character_ddm_profile),
        )
        .route(
            "/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/conversations/:id",
            put(update_conversation).delete(delete_conversation),
        )
        .route("/conversations/:id/messages", get(list_messages))
        .route("/messages", post(create_message))
        .route("/messages/:id", put(update_message).delete(delete_message))
        .route("/momo/responses", post(create_response))
        .route("/momo/responses/:request_id/cancel", post(cancel_response))
        .route("/prompt-spaces", get(list_prompt_spaces))
        .route(
            "/prompt-spaces/:id",
            get(get_prompt_space)
                .put(replace_prompt_space)
                .delete(reset_prompt_space),
        )
        .route(
            "/runtime-settings",
            get(get_runtime_settings).put(replace_runtime_settings),
        )
        .route("/momo/control", post(execute_control))
        .route("/metrics", get(metrics))
        .route("/embeddings/generate", post(generate_embeddings))
        .route("/capabilities/resolve", post(resolve_capability))
        .route("/context/prepare", post(prepare_context))
        .route("/mo-state/compile", post(compile_mo_state))
        .route("/mo-state/runtime", get(get_mo_state_runtime))
        .route("/memory/retrieve-scoped", post(retrieve_scoped_memory))
        .route("/memory/maintenance", post(run_memory_maintenance))
        .route("/memory/recovery", get(memory_recovery_status))
        .route(
            "/memory/recovery/:space_id/retry",
            post(retry_memory_recovery),
        )
        .route(
            "/momo/maintenance/turns",
            post(record_momo_maintenance_turn),
        )
        .route("/momo/maintenance/drain", post(drain_momo_maintenance))
        .route("/memory/documents", get(list_memory_documents))
        .route(
            "/memory/documents/:id",
            get(read_memory_document)
                .put(update_memory_document)
                .delete(delete_memory_document),
        )
        .route(
            "/memory/documents/:id/archive",
            post(archive_memory_document),
        )
        .route(
            "/memory/documents/:id/restore",
            post(restore_memory_document),
        )
        .route("/memory/patches/apply", post(apply_memory_patch))
        .route(
            "/memory/patch-reviews",
            get(list_memory_patch_reviews).post(submit_memory_patch_review),
        )
        .route(
            "/memory/patch-reviews/:id/approve",
            post(approve_memory_patch_review),
        )
        .route(
            "/memory/patch-reviews/:id/reject",
            post(reject_memory_patch_review),
        )
        .route("/semantic-graph/nodes", put(write_nsg_node))
        .route("/semantic-graph/nodes/list", post(list_nsg_nodes))
        .route("/semantic-graph/nodes/archive", post(archive_nsg_node))
        .route("/semantic-graph/nodes/delete", post(delete_nsg_node))
        .route("/semantic-graph/patches/apply", post(apply_nsg_patch))
        .route("/semantic-graph/pending", post(list_nsg_pending_candidates))
        .route(
            "/semantic-graph/pending/approve",
            post(approve_nsg_pending_candidate),
        )
        .route(
            "/semantic-graph/pending/reject",
            post(reject_nsg_pending_candidate),
        )
        .route("/semantic-graph/vector-status", get(nsg_vector_status))
        .route(
            "/semantic-graph/vectors/rebuild",
            post(rebuild_nsg_vector_index),
        )
        .route("/moc/export", post(export_moc))
        .route("/moc/import", post(import_moc))
        .route("/moc/encrypted", get(moc_is_encrypted))
        .route("/lsb/embed", post(embed_lsb_image))
        .route("/lsb/extract", post(extract_lsb_image))
}

fn json_result<T: Serialize>(
    result: runtime_api::RuntimeApiResult<T>,
) -> Result<Json<Value>, ApiError> {
    serde_json::to_value(result.map_err(runtime_api_error)?)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn scoped_json_result<T: Serialize>(
    result: runtime_api::RuntimeApiResult<T>,
) -> Result<Json<Value>, ApiError> {
    serde_json::to_value(result.map_err(runtime_api_error)?)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn scoped_api_error(error: runtime_api::RuntimeApiError) -> ApiError {
    runtime_api_error(error)
}

fn control_api_error(error: runtime_api::RuntimeApiError) -> ApiError {
    runtime_api_error(error)
}

fn runtime_api_error(error: runtime_api::RuntimeApiError) -> ApiError {
    use runtime_api::RuntimeApiErrorKind;

    match error.kind {
        RuntimeApiErrorKind::InvalidRequest => ApiError::bad_request(error.message),
        RuntimeApiErrorKind::NotFound => ApiError::not_found(error.message),
        RuntimeApiErrorKind::Conflict => ApiError::conflict(error.message),
        RuntimeApiErrorKind::Upstream => ApiError::bad_gateway(error.message),
        RuntimeApiErrorKind::Internal => ApiError::internal(error.message),
    }
}

fn embedding_api_error(error: momo_core::EmbeddingError) -> ApiError {
    use momo_core::EmbeddingError;
    match error {
        error @ (EmbeddingError::InvalidProfile(_)
        | EmbeddingError::InvalidInput(_)
        | EmbeddingError::InvalidBaseUrl(_)
        | EmbeddingError::InvalidAuthorization) => ApiError::bad_request(error.to_string()),
        EmbeddingError::Request(error) if error.is_timeout() => {
            ApiError::gateway_timeout(error.to_string())
        }
        error @ EmbeddingError::SerializeProfile(_) => ApiError::internal(error.to_string()),
        error => ApiError::bad_gateway(error.to_string()),
    }
}

fn ok_json<T>(result: runtime_api::RuntimeApiResult<T>) -> Result<Json<Value>, ApiError> {
    result.map_err(runtime_api_error)?;
    Ok(Json(json!({ "ok": true })))
}

fn ensure_resource_id(path_id: &str, body_id: uuid::Uuid) -> Result<(), ApiError> {
    let path_id = uuid::Uuid::parse_str(path_id)
        .map_err(|_| ApiError::bad_request("resource id in path must be a UUID"))?;
    if path_id == body_id {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "resource id in path must match request body",
        ))
    }
}

fn validate_space_id(space_id: String) -> Result<String, ApiError> {
    uuid::Uuid::parse_str(&space_id)
        .map(|id| id.to_string())
        .map_err(|_| ApiError::bad_request("Space id must be a UUID"))
}

fn default_memory_tokens() -> usize {
    2_000
}

fn default_context_window() -> usize {
    8_192
}

fn default_reserve_output_tokens() -> usize {
    1_024
}

#[cfg(test)]
#[path = "../tests/unit/main.rs"]
mod tests;
