use std::{collections::HashMap, env, net::SocketAddr, path::PathBuf, sync::Arc};

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
    MomoConfig,
    api::simple,
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
    data_dir: String,
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

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    core_version: String,
    data_dir: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversationRequest {
    space_id: String,
    title: String,
    character_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateMessageRequest {
    space_id: String,
    conversation_id: String,
    role: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCharacterRequest {
    owner_space_id: String,
    name: String,
    author_name: String,
    character_markdown: String,
    #[serde(default)]
    user_markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DdmProfileRequest {
    owner_space_id: String,
    profile_yaml: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportExternalCharacterRequest {
    owner_space_id: String,
    input_path: String,
    format: momo_core::ExternalCharacterImportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportExternalCharacterRequest {
    owner_space_id: String,
    output_path: String,
    format: momo_core::ExternalCharacterExportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportPreservedCharacterSourceRequest {
    owner_space_id: String,
    output_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveCapabilityRequest {
    provider_id: String,
    model: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RetrieveMemorySpace {
    space_id: String,
    label: String,
    weight: u8,
    memory: bool,
    semantic_graph: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrieveScopedMemoryRequest {
    spaces: Vec<RetrieveMemorySpace>,
    query: String,
    #[serde(default = "default_memory_tokens")]
    max_tokens: usize,
    vector_space_id: Option<String>,
    query_vector: Option<Vec<f64>>,
    #[serde(default)]
    embedding: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileMoStateRequest {
    space_id: String,
    #[serde(default)]
    retrieved_memory: Vec<Value>,
    #[serde(default)]
    retrieved_nsg: Vec<Value>,
    #[serde(default = "default_context_window")]
    max_context_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareContextRequest {
    #[serde(default)]
    runtime_instructions: String,
    #[serde(default)]
    character_markdown: String,
    #[serde(default)]
    user_markdown: String,
    #[serde(default)]
    memory_markdown: String,
    #[serde(default)]
    state_context: String,
    #[serde(default)]
    nsg_markdown: String,
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default = "default_context_window")]
    context_window: usize,
    #[serde(default = "default_reserve_output_tokens")]
    reserve_output_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateMemoryDocumentRequest {
    space_id: String,
    markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyMemoryPatchRequest {
    space_id: String,
    patch_yaml: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitMemoryPatchReviewRequest {
    space_id: String,
    conversation_id: String,
    patch_yaml: String,
    review_mode: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IncludeResolvedQuery {
    space_id: String,
    include_resolved: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteNsgNodeRequest {
    space_id: String,
    target_file: String,
    node: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyNsgPatchRequest {
    space_id: String,
    patch_yaml: String,
    #[serde(default)]
    manual_authority: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceRequest {
    space_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordMaintenanceTurnRequest {
    request_id: String,
    space_id: String,
    user_content: String,
    assistant_content: String,
    memory_enabled: bool,
    nsg_enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NsgTargetRequest {
    space_id: String,
    target_file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceNsgTargetRequest {
    space_id: String,
    target_file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NsgVectorStatusQuery {
    space_id: String,
    vector_space_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MomoConfigExportRequest {
    output_path: String,
    #[serde(default)]
    settings: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MomoConfigImportRequest {
    input_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MocExportRequest {
    output_path: String,
    #[serde(default)]
    settings: Value,
    plan: momo_core::MocExportPlan,
    #[serde(default)]
    protection: momo_core::MocProtection,
    #[serde(default)]
    host_modules: Vec<momo_core::HostMocModule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MocImportRequest {
    input_path: String,
    plan: momo_core::MocImportPlan,
    #[serde(default)]
    protection: momo_core::MocProtection,
    claim_unknown_to: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MocEncryptedQuery {
    input_path: String,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "momo_server=info,tower_http=info".into()),
        )
        .init();

    let bind = env::var("MOMO_SERVER_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let addr: SocketAddr = bind.parse()?;
    ensure_bind_allowed(addr, env_flag(ALLOW_REMOTE_ENV))?;
    let data_dir = env::var("MOMO_DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.to_owned());
    let initialized_dir = simple::initialize_core(data_dir)
        .await
        .map_err(to_io_error)?;
    let momo_config_path = env::var_os("MOMO_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&initialized_dir).join("config/momo.toml"));
    let momo_config = Arc::new(
        MomoConfig::load_or_default(&momo_config_path)
            .map_err(|error| to_io_error(error.to_string()))?,
    );
    if momo_config_path.exists() {
        simple::import_momo_config_json(momo_config_path.to_string_lossy().into_owned())
            .await
            .map_err(to_io_error)?;
    }
    let gateway_origin = env::var("MOMO_MODEL_GATEWAY_ORIGIN")
        .unwrap_or_else(|_| "http://127.0.0.1:8788/v1".to_owned());
    let gateway_api_key = env::var("MOMO_MODEL_GATEWAY_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let gateway_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let momo_api = Arc::new(MomoApiService::new(
        gateway_origin.clone(),
        gateway_api_key.clone(),
        gateway_client.clone(),
        Arc::clone(&momo_config),
    ));
    let state = AppState {
        data_dir: initialized_dir,
        momo_api,
        response_concurrency: Arc::new(Semaphore::new(env_usize(
            "MOMO_RESPONSE_MAX_CONCURRENCY",
            8,
            1,
            256,
        )?)),
        response_timeout: std::time::Duration::from_secs(env_usize(
            "MOMO_RESPONSE_TIMEOUT_SECONDS",
            120,
            1,
            3_600,
        )? as u64),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };

    let momo_api = Arc::clone(&state.momo_api);
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("MOMO server listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    momo_api.wait_for_maintenance().await;
    Ok(())
}

fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .nest("/v1", api_routes())
        .layer(DefaultBodyLimit::max(MAX_RESPONSE_REQUEST_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"))
}

fn env_usize(name: &str, default: usize, min: usize, max: usize) -> std::io::Result<usize> {
    let value = match env::var(name) {
        Ok(value) => value.parse::<usize>().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{name} must be an integer between {min} and {max}"),
            )
        })?,
        Err(env::VarError::NotPresent) => default,
        Err(error) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("failed to read {name}: {error}"),
            ));
        }
    };
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{name} must be between {min} and {max}"),
        ))
    }
}

fn ensure_bind_allowed(addr: SocketAddr, allow_remote: bool) -> std::io::Result<()> {
    if addr.ip().is_loopback() || allow_remote {
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!(
            "refusing non-loopback bind {addr}; set {ALLOW_REMOTE_ENV}=1 only behind a trusted authenticated proxy"
        ),
    ))
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
        .route("/momo/control", post(execute_control))
        .route("/metrics", get(metrics))
        .route("/embeddings/generate", post(generate_embeddings))
        .route("/capabilities/resolve", post(resolve_capability))
        .route("/context/prepare", post(prepare_context))
        .route("/mo-state/compile", post(compile_mo_state))
        .route("/mo-state/runtime", get(get_mo_state_runtime))
        .route("/memory/retrieve-scoped", post(retrieve_scoped_memory))
        .route("/memory/maintenance", post(run_memory_maintenance))
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
        .route("/momo-config/export", post(export_momo_config))
        .route("/momo-config/import", post(import_momo_config))
        .route("/moc/export", post(export_moc))
        .route("/moc/import", post(import_moc))
        .route("/moc/encrypted", get(moc_is_encrypted))
        .route("/lsb/embed", post(embed_lsb_image))
        .route("/lsb/extract", post(extract_lsb_image))
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "momo-server",
        core_version: simple::core_version(),
        data_dir: state.data_dir,
    })
}

async fn execute_control(
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
    let value = simple::execute_control_json(to_json_string(&request)?)
        .await
        .map_err(control_api_error)?;
    serde_json::from_str(&value)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

async fn metrics(State(state): State<AppState>) -> Json<Value> {
    let routes = state.metrics.lock().await.clone();
    Json(json!({
        "schema": "momo.metrics/1.0",
        "routes": routes,
    }))
}

async fn update_route_metrics(
    state: &AppState,
    route: &str,
    update: impl FnOnce(&mut RouteMetrics),
) {
    let mut metrics = state.metrics.lock().await;
    update(metrics.entry(route.to_owned()).or_default());
}

async fn list_characters(Query(query): Query<SpaceRequest>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_characters_json(validate_space_id(query.space_id)?).await)
}

async fn create_character(
    Json(request): Json<CreateCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    json_result(
        simple::stage_character_json(
            scope_id,
            request.author_name,
            request.name,
            request.character_markdown,
            request.user_markdown,
        )
        .await,
    )
}

async fn get_character(Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    scoped_json_result(simple::local_character_json(id).await)
}

async fn import_external_character(
    Json(request): Json<ImportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    json_result(
        simple::import_external_character_json(
            json!({
                "scope_id": scope_id,
                "input_path": request.input_path,
                "format": request.format,
            })
            .to_string(),
        )
        .await,
    )
}

async fn export_external_character(
    Path(id): Path<String>,
    Json(request): Json<ExportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    scoped_json_result(
        simple::export_external_character_json(
            json!({
                "scope_id": scope_id,
                "character_id": id,
                "output_path": request.output_path,
                "format": request.format,
            })
            .to_string(),
        )
        .await,
    )
}

async fn export_preserved_character_source(
    Path(id): Path<String>,
    Json(request): Json<ExportPreservedCharacterSourceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.owner_space_id)?;
    scoped_json_result(
        simple::export_preserved_character_source_json(
            json!({
                "scope_id": scope_id,
                "character_id": id,
                "output_path": request.output_path,
            })
            .to_string(),
        )
        .await,
    )
}

async fn update_character(
    Path(id): Path<String>,
    Json(character): Json<CharacterCard>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, character.id)?;
    scoped_json_result(
        simple::stage_character_update_json(
            character.scope_id.to_string(),
            to_json_string(&character)?,
        )
        .await,
    )
}

async fn delete_character(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_character_delete(validate_space_id(query.space_id)?, id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn get_character_ddm_profile(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(query.space_id)?;
    let character = simple::local_character_json(id.clone())
        .await
        .map_err(scoped_api_error)?;
    let character: CharacterCard =
        serde_json::from_str(&character).map_err(|error| ApiError::internal(error.to_string()))?;
    if character.scope_id.to_string() != scope_id {
        return Err(ApiError::not_found("character does not belong to scope"));
    }
    let profile = simple::character_ddm_profile_yaml(id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(json!({"profile_yaml": profile})))
}

async fn update_character_ddm_profile(
    Path(id): Path<String>,
    Json(request): Json<DdmProfileRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        simple::upsert_character_ddm_profile_json(
            validate_space_id(request.owner_space_id)?,
            id,
            request.profile_yaml,
        )
        .await,
    )
}

async fn delete_character_ddm_profile(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::delete_character_ddm_profile(validate_space_id(query.space_id)?, id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn list_conversations(Query(query): Query<SpaceRequest>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_conversations_json(validate_space_id(query.space_id)?).await)
}

async fn create_conversation(
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    scoped_json_result(
        simple::stage_conversation_json(None, scope_id, request.title, request.character_id).await,
    )
}

async fn update_conversation(
    Path(id): Path<String>,
    Json(conversation): Json<Conversation>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, conversation.id)?;
    scoped_json_result(
        simple::stage_conversation_update_json(
            conversation.scope_id.to_string(),
            to_json_string(&conversation)?,
        )
        .await,
    )
}

async fn delete_conversation(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_conversation_delete(validate_space_id(query.space_id)?, id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn list_messages(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(simple::local_messages_json(validate_space_id(query.space_id)?, id).await)
}

async fn create_message(
    Json(request): Json<CreateMessageRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        simple::stage_message_json(
            validate_space_id(request.space_id)?,
            request.conversation_id,
            request.role,
            request.content,
        )
        .await,
    )
}

async fn update_message(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
    Json(message): Json<Message>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, message.id)?;
    scoped_json_result(
        simple::stage_message_update_json(
            validate_space_id(query.space_id)?,
            to_json_string(&message)?,
        )
        .await,
    )
}

async fn delete_message(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_message_delete(validate_space_id(query.space_id)?, id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn generate_embeddings(Json(request): Json<Value>) -> Result<Json<Value>, ApiError> {
    let response = simple::generate_embeddings_json_typed(to_json_string(&request)?)
        .await
        .map_err(embedding_api_error)?;
    serde_json::from_str(&response)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

async fn prepare_context(
    Json(request): Json<PrepareContextRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::prepare_context_json(to_json_string(&json!({
        "runtime_instructions": request.runtime_instructions,
        "character_markdown": request.character_markdown,
        "user_markdown": request.user_markdown,
        "memory_markdown": request.memory_markdown,
        "state_context": request.state_context,
        "nsg_markdown": request.nsg_markdown,
        "messages": request.messages,
        "context_window": request.context_window,
        "reserve_output_tokens": request.reserve_output_tokens,
    }))?))
}

async fn resolve_capability(
    Json(request): Json<ResolveCapabilityRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::resolve_capability_json(request.provider_id, request.model).await)
}

async fn compile_mo_state(
    Json(request): Json<CompileMoStateRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    json_result(
        simple::compile_mo_state_json(
            scope_id,
            to_json_string(&request.retrieved_memory)?,
            to_json_string(&request.retrieved_nsg)?,
            request.max_context_tokens,
        )
        .await,
    )
}

async fn get_mo_state_runtime(
    Query(request): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::mo_state_runtime_status_json(validate_space_id(request.space_id)?).await)
}

async fn retrieve_scoped_memory(
    Json(request): Json<RetrieveScopedMemoryRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::retrieve_scoped_memory_json(to_json_string(&json!({
            "spaces": request.spaces,
            "query": request.query,
            "max_tokens": request.max_tokens,
            "vector_space_id": request.vector_space_id,
            "query_vector": request.query_vector,
            "embedding": request.embedding,
        }))?)
        .await,
    )
}

async fn run_memory_maintenance(
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::run_memory_maintenance_json(validate_space_id(request.space_id)?).await)
}

/// Records an already-completed turn for trusted local imports and benchmark
/// replay. It never invokes the conversation model; a later drain runs the
/// same DMW/NSG maintenance path as a native response.
async fn record_momo_maintenance_turn(
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
    simple::append_maintenance_turn_json(
        json!({
            "request_id": request.request_id,
            "scope_id": space_id,
            "user_content": request.user_content,
            "assistant_content": request.assistant_content,
        })
        .to_string(),
        request.memory_enabled,
        request.nsg_enabled,
    )
    .await
    .map_err(|error| {
        if error.starts_with("maintenance turn conflict:") {
            ApiError::conflict("maintenance turn request_id was reused with different content")
        } else {
            ApiError::internal(error)
        }
    })?;
    Ok(Json(json!({"recorded": true})))
}

/// Local management barrier for reproducible evaluations. Flush even a partial
/// batch before probing a new conversation; failures must not look like success.
async fn drain_momo_maintenance(
    State(state): State<AppState>,
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let space_id = validate_space_id(request.space_id)?;
    drain_momo_space(&state, &space_id).await?;
    Ok(Json(json!({"completed": true, "space_id": space_id})))
}

async fn drain_momo_space(state: &AppState, space_id: &str) -> Result<(), ApiError> {
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

async fn drain_momo_kind(
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
        let pending = simple::pending_maintenance_turns_json(
            space_id.to_owned(),
            storage_kind.to_owned(),
            batch_limit,
        )
        .await
        .map_err(ApiError::internal)?;
        let pending: Vec<Value> = serde_json::from_str(&pending)
            .map_err(|error| ApiError::internal(error.to_string()))?;
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

async fn list_memory_documents(Query(query): Query<SpaceRequest>) -> Result<Json<Value>, ApiError> {
    json_result(simple::list_memory_documents_json(validate_space_id(query.space_id)?).await)
}

async fn read_memory_document(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::read_memory_document_json(validate_space_id(query.space_id)?, id).await)
}

async fn update_memory_document(
    Path(id): Path<String>,
    Json(request): Json<UpdateMemoryDocumentRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::update_memory_document_json(
            validate_space_id(request.space_id)?,
            id,
            request.markdown,
        )
        .await,
    )
}

async fn archive_memory_document(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::archive_memory_document_json(validate_space_id(query.space_id)?, id).await)
}

async fn restore_memory_document(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::restore_memory_document_json(validate_space_id(query.space_id)?, id).await)
}

async fn delete_memory_document(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::delete_memory_document_json(validate_space_id(query.space_id)?, id).await)
}

async fn apply_memory_patch(
    Json(request): Json<ApplyMemoryPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(simple::apply_memory_patch_json(scope_id, request.patch_yaml).await)
}

async fn submit_memory_patch_review(
    Json(request): Json<SubmitMemoryPatchReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::submit_memory_patch_review_json(
            validate_space_id(request.space_id)?,
            request.conversation_id,
            request.patch_yaml,
            request.review_mode,
        )
        .await,
    )
}

async fn list_memory_patch_reviews(
    Query(query): Query<IncludeResolvedQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::list_memory_patch_reviews_json(
            validate_space_id(query.space_id)?,
            query.include_resolved.unwrap_or(false),
        )
        .await,
    )
}

async fn approve_memory_patch_review(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::approve_memory_patch_review_json(validate_space_id(query.space_id)?, id).await,
    )
}

async fn reject_memory_patch_review(
    Path(id): Path<String>,
    Query(query): Query<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::reject_memory_patch_review_json(validate_space_id(query.space_id)?, id).await,
    )
}

async fn list_nsg_nodes(Json(request): Json<SpaceRequest>) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    json_result(simple::list_nsg_nodes_json(scope_id, false).await)
}

async fn write_nsg_node(Json(request): Json<WriteNsgNodeRequest>) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::write_nsg_node_json(
            validate_space_id(request.space_id)?,
            request.target_file,
            to_json_string(&request.node)?,
        )
        .await,
    )
}

async fn archive_nsg_node(Json(request): Json<NsgTargetRequest>) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::archive_nsg_node_json(validate_space_id(request.space_id)?, request.target_file)
            .await,
    )
}

async fn delete_nsg_node(Json(request): Json<NsgTargetRequest>) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::delete_nsg_node_json(validate_space_id(request.space_id)?, request.target_file)
            .await,
    )
}

async fn apply_nsg_patch(
    Json(request): Json<ApplyNsgPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(
        simple::apply_nsg_patch_json(scope_id, request.patch_yaml, request.manual_authority).await,
    )
}

async fn list_nsg_pending_candidates(
    Json(request): Json<SpaceRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    json_result(simple::list_nsg_pending_candidates_json(scope_id).await)
}

async fn approve_nsg_pending_candidate(
    Json(request): Json<SpaceNsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(simple::approve_nsg_pending_candidate_json(scope_id, request.target_file).await)
}

async fn reject_nsg_pending_candidate(
    Json(request): Json<SpaceNsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_space_id(request.space_id)?;
    ok_json(simple::reject_nsg_pending_candidate_json(scope_id, request.target_file).await)
}

async fn nsg_vector_status(
    Query(query): Query<NsgVectorStatusQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::nsg_vector_status_json(
            validate_space_id(query.space_id)?,
            query.vector_space_id.unwrap_or_default(),
        )
        .await,
    )
}

async fn rebuild_nsg_vector_index(Json(mut request): Json<Value>) -> Result<Json<Value>, ApiError> {
    let scope_id = request
        .as_object_mut()
        .and_then(|object| object.remove("space_id"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| ApiError::bad_request("space_id is required"))?;
    json_result(
        simple::rebuild_nsg_vector_index_json(
            validate_space_id(scope_id)?,
            to_json_string(&request)?,
        )
        .await,
    )
}

async fn export_momo_config(
    Json(request): Json<MomoConfigExportRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::export_momo_config_json(request.output_path, to_json_string(&request.settings)?)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn import_momo_config(
    Json(request): Json<MomoConfigImportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::import_momo_config_json(request.input_path).await)
}

async fn export_moc(Json(request): Json<MocExportRequest>) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::export_moc_json(to_json_string(&json!({
            "output_path": request.output_path,
            "settings": request.settings,
            "plan": request.plan,
            "protection": request.protection,
            "host_modules": request.host_modules,
        }))?)
        .await,
    )
}

async fn import_moc(Json(request): Json<MocImportRequest>) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::import_moc_json(
            request.input_path,
            request.plan,
            request.protection,
            request.claim_unknown_to,
        )
        .await,
    )
}

async fn moc_is_encrypted(Query(query): Query<MocEncryptedQuery>) -> Result<Json<Value>, ApiError> {
    let encrypted = simple::moc_is_encrypted(query.input_path)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "encrypted": encrypted,
    })))
}

async fn embed_lsb_image(Json(request): Json<Value>) -> Result<Json<Value>, ApiError> {
    json_result(simple::embed_lsb_image_json(to_json_string(&request)?).await)
}

async fn extract_lsb_image(Json(request): Json<Value>) -> Result<Json<Value>, ApiError> {
    json_result(simple::extract_lsb_image_json(to_json_string(&request)?).await)
}

fn json_result(result: Result<String, String>) -> Result<Json<Value>, ApiError> {
    let value = result.map_err(ApiError::internal)?;
    serde_json::from_str(&value)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn scoped_json_result(result: Result<String, String>) -> Result<Json<Value>, ApiError> {
    let value = result.map_err(scoped_api_error)?;
    serde_json::from_str(&value)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn scoped_api_error(error: String) -> ApiError {
    if error.contains("does not belong to")
        || error.contains("does not exist in the requested scope")
    {
        ApiError::not_found(error)
    } else {
        ApiError::internal(error)
    }
}

fn control_api_error(error: String) -> ApiError {
    if error.contains("reused with different content") || error.contains("already in progress") {
        ApiError::conflict(error)
    } else if error.contains("does not belong to") || error.contains("does not exist") {
        ApiError::not_found(error)
    } else {
        ApiError::internal(error)
    }
}

fn embedding_api_error(error: simple::GenerateEmbeddingsError) -> ApiError {
    match error {
        simple::GenerateEmbeddingsError::InvalidRequest(error) => {
            ApiError::bad_request(error.to_string())
        }
        simple::GenerateEmbeddingsError::Provider(
            error @ (momo_core::EmbeddingError::InvalidProfile(_)
            | momo_core::EmbeddingError::InvalidInput(_)
            | momo_core::EmbeddingError::InvalidBaseUrl(_)
            | momo_core::EmbeddingError::InvalidAuthorization),
        ) => ApiError::bad_request(error.to_string()),
        simple::GenerateEmbeddingsError::Provider(momo_core::EmbeddingError::Request(error))
            if error.is_timeout() =>
        {
            ApiError::gateway_timeout(error.to_string())
        }
        simple::GenerateEmbeddingsError::Provider(
            error @ momo_core::EmbeddingError::SerializeProfile(_),
        ) => ApiError::internal(error.to_string()),
        simple::GenerateEmbeddingsError::Provider(error) => {
            ApiError::bad_gateway(error.to_string())
        }
        simple::GenerateEmbeddingsError::Serialize(error) => ApiError::internal(error.to_string()),
    }
}

fn ok_json(result: Result<String, String>) -> Result<Json<Value>, ApiError> {
    result.map_err(ApiError::internal)?;
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

fn to_json_string<T: Serialize>(value: &T) -> Result<String, ApiError> {
    serde_json::to_string(value).map_err(|error| ApiError::bad_request(error.to_string()))
}

fn validate_space_id(space_id: String) -> Result<String, ApiError> {
    uuid::Uuid::parse_str(&space_id)
        .map_err(|_| ApiError::bad_request("Space id must be a UUID"))?;
    Ok(space_id)
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

fn to_io_error(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

#[cfg(test)]
#[path = "../tests/unit/main.rs"]
mod tests;
