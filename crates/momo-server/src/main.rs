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

#[cfg(test)]
use axum::http::StatusCode;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    routing::{get, post, put},
};
use momo_core::{
    MAX_RESPONSE_REQUEST_BYTES, MomoApiService, MomoConfig,
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
struct CreateConversationRequest {
    scope_id: String,
    character_scope_id: String,
    title: String,
    character_id: String,
}

#[derive(Deserialize)]
struct CreateMessageRequest {
    scope_id: String,
    conversation_id: String,
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct CreateCharacterRequest {
    scope_id: String,
    name: String,
    #[serde(default)]
    author_name: String,
    #[serde(default)]
    description: String,
    character_markdown: String,
    #[serde(default)]
    user_markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportExternalCharacterRequest {
    scope_id: String,
    input_path: String,
    format: momo_core::ExternalCharacterImportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportExternalCharacterRequest {
    scope_id: String,
    output_path: String,
    format: momo_core::ExternalCharacterExportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportPreservedCharacterSourceRequest {
    scope_id: String,
    output_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveCapabilityRequest {
    provider_id: String,
    model: String,
}

#[derive(Deserialize, Serialize)]
struct RetrieveMemoryScope {
    scope_id: String,
    label: String,
    #[serde(default = "default_scope_weight")]
    weight: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrieveScopedMemoryRequest {
    scopes: Vec<RetrieveMemoryScope>,
    query: String,
    #[serde(default = "default_memory_tokens")]
    max_tokens: usize,
    #[serde(default = "default_true")]
    include_memory: bool,
    #[serde(default = "default_true")]
    include_semantic_graph: bool,
    vector_space_id: Option<String>,
    query_vector: Option<Vec<f64>>,
    #[serde(default)]
    embedding: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileMoStateRequest {
    scope_id: String,
    #[serde(default)]
    retrieved_memory: Vec<Value>,
    #[serde(default)]
    retrieved_nsg: Vec<Value>,
    #[serde(default = "default_context_window")]
    max_context_tokens: usize,
}

#[derive(Deserialize)]
struct PrepareContextRequest {
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
struct UpdateMemoryDocumentRequest {
    scope_id: String,
    markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyMemoryPatchRequest {
    scope_id: String,
    patch_yaml: String,
}

#[derive(Deserialize)]
struct SubmitMemoryPatchReviewRequest {
    scope_id: String,
    conversation_id: String,
    patch_yaml: String,
    review_mode: String,
}

#[derive(Deserialize)]
struct IncludeResolvedQuery {
    scope_id: String,
    include_resolved: Option<bool>,
}

#[derive(Deserialize)]
struct WriteNsgNodeRequest {
    scope_id: String,
    target_file: String,
    node: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyNsgPatchRequest {
    scope_id: String,
    patch_yaml: String,
    #[serde(default)]
    manual_authority: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeRequest {
    scope_id: String,
}

#[derive(Deserialize)]
struct NsgTargetRequest {
    scope_id: String,
    target_file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedNsgTargetRequest {
    scope_id: String,
    target_file: String,
}

#[derive(Deserialize)]
struct NsgVectorStatusQuery {
    scope_id: String,
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
    scope_id: String,
    output_path: String,
    #[serde(default)]
    settings: Value,
    modules: Vec<momo_core::MocModule>,
    #[serde(default)]
    compatibility: momo_core::MocCompatibility,
    #[serde(default)]
    character_id: Option<uuid::Uuid>,
    #[serde(default)]
    protection: momo_core::MocProtection,
    #[serde(default)]
    host_modules: Vec<momo_core::HostMocModule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MocImportRequest {
    scope_id: String,
    input_path: String,
    #[serde(default = "default_conflict_mode")]
    conflict_mode: momo_core::ConflictMode,
    #[serde(default)]
    protection: momo_core::MocProtection,
    claim_unknown_to: Option<String>,
}

#[derive(Deserialize)]
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

    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("MOMO server listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
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
            put(update_character).delete(delete_character),
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
        .route("/metrics", get(metrics))
        .route("/embeddings/generate", post(generate_embeddings))
        .route("/capabilities/resolve", post(resolve_capability))
        .route("/context/prepare", post(prepare_context))
        .route("/mo-state/compile", post(compile_mo_state))
        .route("/memory/retrieve-scoped", post(retrieve_scoped_memory))
        .route("/memory/maintenance", post(run_memory_maintenance))
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

async fn list_characters(Query(query): Query<ScopeRequest>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_characters_json(validate_scope_id(query.scope_id)?).await)
}

async fn create_character(
    Json(request): Json<CreateCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    json_result(
        simple::stage_character_json(
            scope_id,
            request.author_name,
            request.name,
            request.description,
            request.character_markdown,
            request.user_markdown,
        )
        .await,
    )
}

async fn import_external_character(
    Json(request): Json<ImportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
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
    let scope_id = validate_scope_id(request.scope_id)?;
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
    let scope_id = validate_scope_id(request.scope_id)?;
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
    Query(query): Query<ScopeRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_character_delete(validate_scope_id(query.scope_id)?, id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn list_conversations(Query(query): Query<ScopeRequest>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_conversations_json(validate_scope_id(query.scope_id)?).await)
}

async fn create_conversation(
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    let character_scope_id = validate_scope_id(request.character_scope_id)?;
    scoped_json_result(
        simple::stage_conversation_json(
            None,
            scope_id,
            character_scope_id,
            request.title,
            request.character_id,
        )
        .await,
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
    Query(query): Query<ScopeRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_conversation_delete(validate_scope_id(query.scope_id)?, id)
        .await
        .map_err(scoped_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn list_messages(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(simple::local_messages_json(validate_scope_id(query.scope_id)?, id).await)
}

async fn create_message(
    Json(request): Json<CreateMessageRequest>,
) -> Result<Json<Value>, ApiError> {
    scoped_json_result(
        simple::stage_message_json(
            validate_scope_id(request.scope_id)?,
            request.conversation_id,
            request.role,
            request.content,
        )
        .await,
    )
}

async fn update_message(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
    Json(message): Json<Message>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, message.id)?;
    scoped_json_result(
        simple::stage_message_update_json(
            validate_scope_id(query.scope_id)?,
            to_json_string(&message)?,
        )
        .await,
    )
}

async fn delete_message(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_message_delete(validate_scope_id(query.scope_id)?, id)
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
    let scope_id = validate_scope_id(request.scope_id)?;
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

async fn retrieve_scoped_memory(
    Json(request): Json<RetrieveScopedMemoryRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::retrieve_scoped_memory_json(to_json_string(&json!({
            "scopes": request.scopes,
            "query": request.query,
            "max_tokens": request.max_tokens,
            "include_memory": request.include_memory,
            "include_semantic_graph": request.include_semantic_graph,
            "vector_space_id": request.vector_space_id,
            "query_vector": request.query_vector,
            "embedding": request.embedding,
        }))?)
        .await,
    )
}

async fn run_memory_maintenance(
    Json(request): Json<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::run_memory_maintenance_json(validate_scope_id(request.scope_id)?).await)
}

async fn list_memory_documents(Query(query): Query<ScopeRequest>) -> Result<Json<Value>, ApiError> {
    json_result(simple::list_memory_documents_json(validate_scope_id(query.scope_id)?).await)
}

async fn read_memory_document(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::read_memory_document_json(validate_scope_id(query.scope_id)?, id).await)
}

async fn update_memory_document(
    Path(id): Path<String>,
    Json(request): Json<UpdateMemoryDocumentRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::update_memory_document_json(
            validate_scope_id(request.scope_id)?,
            id,
            request.markdown,
        )
        .await,
    )
}

async fn archive_memory_document(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::archive_memory_document_json(validate_scope_id(query.scope_id)?, id).await)
}

async fn restore_memory_document(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::restore_memory_document_json(validate_scope_id(query.scope_id)?, id).await)
}

async fn delete_memory_document(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::delete_memory_document_json(validate_scope_id(query.scope_id)?, id).await)
}

async fn apply_memory_patch(
    Json(request): Json<ApplyMemoryPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    ok_json(simple::apply_memory_patch_json(scope_id, request.patch_yaml).await)
}

async fn submit_memory_patch_review(
    Json(request): Json<SubmitMemoryPatchReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::submit_memory_patch_review_json(
            validate_scope_id(request.scope_id)?,
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
            validate_scope_id(query.scope_id)?,
            query.include_resolved.unwrap_or(false),
        )
        .await,
    )
}

async fn approve_memory_patch_review(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::approve_memory_patch_review_json(validate_scope_id(query.scope_id)?, id).await,
    )
}

async fn reject_memory_patch_review(
    Path(id): Path<String>,
    Query(query): Query<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::reject_memory_patch_review_json(validate_scope_id(query.scope_id)?, id).await,
    )
}

async fn list_nsg_nodes(Json(request): Json<ScopeRequest>) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    json_result(simple::list_nsg_nodes_json(scope_id, false).await)
}

async fn write_nsg_node(Json(request): Json<WriteNsgNodeRequest>) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::write_nsg_node_json(
            validate_scope_id(request.scope_id)?,
            request.target_file,
            to_json_string(&request.node)?,
        )
        .await,
    )
}

async fn archive_nsg_node(Json(request): Json<NsgTargetRequest>) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::archive_nsg_node_json(validate_scope_id(request.scope_id)?, request.target_file)
            .await,
    )
}

async fn delete_nsg_node(Json(request): Json<NsgTargetRequest>) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::delete_nsg_node_json(validate_scope_id(request.scope_id)?, request.target_file)
            .await,
    )
}

async fn apply_nsg_patch(
    Json(request): Json<ApplyNsgPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    ok_json(
        simple::apply_nsg_patch_json(scope_id, request.patch_yaml, request.manual_authority).await,
    )
}

async fn list_nsg_pending_candidates(
    Json(request): Json<ScopeRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    json_result(simple::list_nsg_pending_candidates_json(scope_id).await)
}

async fn approve_nsg_pending_candidate(
    Json(request): Json<ScopedNsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    ok_json(simple::approve_nsg_pending_candidate_json(scope_id, request.target_file).await)
}

async fn reject_nsg_pending_candidate(
    Json(request): Json<ScopedNsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    ok_json(simple::reject_nsg_pending_candidate_json(scope_id, request.target_file).await)
}

async fn nsg_vector_status(
    Query(query): Query<NsgVectorStatusQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::nsg_vector_status_json(
            validate_scope_id(query.scope_id)?,
            query.vector_space_id.unwrap_or_default(),
        )
        .await,
    )
}

async fn rebuild_nsg_vector_index(Json(mut request): Json<Value>) -> Result<Json<Value>, ApiError> {
    let scope_id = request
        .as_object_mut()
        .and_then(|object| object.remove("scope_id"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| ApiError::bad_request("scope_id is required"))?;
    json_result(
        simple::rebuild_nsg_vector_index_json(
            validate_scope_id(scope_id)?,
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
            "scope_id": validate_scope_id(request.scope_id)?,
            "settings": request.settings,
            "modules": request.modules,
            "compatibility": request.compatibility,
            "character_id": request.character_id,
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
            validate_scope_id(request.scope_id)?,
            request.conflict_mode,
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

fn validate_scope_id(scope_id: String) -> Result<String, ApiError> {
    uuid::Uuid::parse_str(&scope_id)
        .map_err(|_| ApiError::bad_request("memory scope id must be a UUID"))?;
    Ok(scope_id)
}

const fn default_true() -> bool {
    true
}

fn default_memory_tokens() -> usize {
    2_000
}

const fn default_scope_weight() -> usize {
    1
}

fn default_context_window() -> usize {
    8_192
}

fn default_reserve_output_tokens() -> usize {
    1_024
}

const fn default_conflict_mode() -> momo_core::ConflictMode {
    momo_core::ConflictMode::KeepExisting
}

fn to_io_error(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, header},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_stream::StreamExt;
    use tower::ServiceExt;

    static TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));
    static TEST_DATA_DIR: std::sync::LazyLock<tempfile::TempDir> =
        std::sync::LazyLock::new(|| tempfile::tempdir().expect("shared test data directory"));

    async fn initialize_test_core() -> String {
        simple::initialize_core(TEST_DATA_DIR.path().to_string_lossy().into_owned())
            .await
            .expect("initialize core")
    }

    fn test_momo_api(origin: impl Into<String>) -> Arc<MomoApiService> {
        Arc::new(MomoApiService::new(
            origin,
            None,
            reqwest::Client::new(),
            Arc::new(MomoConfig::default()),
        ))
    }

    #[test]
    fn remote_bind_requires_explicit_opt_in() {
        let loopback = "127.0.0.1:8765".parse().expect("loopback address");
        let remote = "0.0.0.0:8765".parse().expect("remote address");
        assert!(ensure_bind_allowed(loopback, false).is_ok());
        assert!(ensure_bind_allowed(remote, false).is_err());
        assert!(ensure_bind_allowed(remote, true).is_ok());
    }

    #[test]
    fn embedding_errors_use_client_and_gateway_status_codes() {
        let client = embedding_api_error(simple::GenerateEmbeddingsError::Provider(
            momo_core::EmbeddingError::InvalidInput("bad input".to_owned()),
        ));
        assert_eq!(client.status, StatusCode::BAD_REQUEST);

        let upstream = embedding_api_error(simple::GenerateEmbeddingsError::Provider(
            momo_core::EmbeddingError::InvalidResponse("bad response".to_owned()),
        ));
        assert_eq!(upstream.status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn response_errors_are_typed_bounded_and_redacted() {
        let rate_limit = model_api_error(
            "model endpoint returned HTTP 429: Bearer should-not-leak sk-secret-value".to_owned(),
        );
        assert_eq!(rate_limit.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(rate_limit.error.code, "rate_limit");
        assert_eq!(rate_limit.error.upstream_status, Some(429));
        assert!(!rate_limit.error.message.contains("should-not-leak"));
        assert!(!rate_limit.error.message.contains("sk-secret-value"));

        let sanitized = sanitize_error_message(&format!(
            "api_key={} trailing",
            "x".repeat(MAX_PUBLIC_ERROR_BYTES * 2)
        ));
        assert!(sanitized.len() <= MAX_PUBLIC_ERROR_BYTES + 32);
        assert!(!sanitized.contains(&"x".repeat(64)));
    }

    #[tokio::test]
    async fn response_json_rejections_use_the_unified_error_envelope() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api("http://127.0.0.1:9/v1"),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/momo/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body"),
        )
        .expect("JSON");
        assert_eq!(body["error"]["code"], "invalid_json");
    }

    #[tokio::test]
    async fn http_core_contract_and_character_round_trip() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api("http://127.0.0.1:9/v1"),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("health request"),
            )
            .await
            .expect("health response");
        assert_eq!(response.status(), StatusCode::OK);
        let health: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("health body"),
        )
        .expect("health JSON");
        assert_eq!(health["ok"], true);
        assert_eq!(health["service"], "momo-server");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/capabilities/resolve")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"provider_id": "test", "model": "unknown-model"}).to_string(),
                    ))
                    .expect("capability request"),
            )
            .await
            .expect("capability response");
        assert_eq!(response.status(), StatusCode::OK);
        let capability: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("capability body"),
        )
        .expect("capability JSON");
        assert_eq!(capability["profile"]["context_window"], 8192);
        assert_eq!(capability["source"], "conservative_fallback");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/memory/retrieve-scoped")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scopes": [
                                {"scope_id": "00000000-0000-4000-8000-000000000011", "label": "personal", "weight": 3},
                                {"scope_id": "00000000-0000-4000-8000-000000000012", "label": "channel", "weight": 2}
                            ],
                            "query": "hello",
                            "max_tokens": 1024
                        })
                        .to_string(),
                    ))
                    .expect("scoped memory request"),
            )
            .await
            .expect("scoped memory response");
        assert_eq!(response.status(), StatusCode::OK);
        let scoped: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 128 * 1024)
                .await
                .expect("scoped memory body"),
        )
        .expect("scoped memory JSON");
        let labels = scoped
            .as_array()
            .expect("scoped memory array")
            .iter()
            .filter_map(|item| item["memory_scope"]["label"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(labels, ["channel", "personal"].into_iter().collect());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/semantic-graph/pending")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": "00000000-0000-4000-8000-000000000011"
                        })
                        .to_string(),
                    ))
                    .expect("scoped pending request"),
            )
            .await
            .expect("scoped pending response");
        assert_eq!(response.status(), StatusCode::OK);
        let pending: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("pending body"),
        )
        .expect("pending JSON");
        assert_eq!(pending.as_array().map(Vec::len), Some(0));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/mo-state/compile")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "owner_id": "00000000-0000-4000-8000-000000000011",
                            "retrieved_memory": [],
                            "retrieved_nsg": []
                        })
                        .to_string(),
                    ))
                    .expect("removed owner_id request"),
            )
            .await
            .expect("removed owner_id response");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let external_path = TEST_DATA_DIR.path().join("external-card.json");
        std::fs::write(
            &external_path,
            serde_json::to_vec_pretty(&json!({
                "spec": "chara_card_v2",
                "spec_version": "2.0",
                "data": {
                    "name": "Imported external character",
                    "description": "Imported description",
                    "personality": "Careful",
                    "scenario": "A contract test",
                    "first_mes": "Hello from CCv2",
                    "mes_example": "{{char}}: Hello",
                    "creator_notes": "Keep this note",
                    "system_prompt": "external runtime field",
                    "post_history_instructions": "",
                    "alternate_greetings": [],
                    "tags": ["test"],
                    "creator": "External creator",
                    "character_version": "1.2.3",
                    "extensions": {"contract": {"preserve": true}}
                }
            }))
            .expect("external card JSON"),
        )
        .expect("external card fixture");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/characters/import-external")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"scope_id": TEST_SCOPE_ID, "input_path": external_path, "format": "ccv2_json"}).to_string(),
                    ))
                    .expect("external import request"),
            )
            .await
            .expect("external import response");
        assert_eq!(response.status(), StatusCode::OK);
        let imported: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 128 * 1024)
                .await
                .expect("external import body"),
        )
        .expect("external import JSON");
        assert_eq!(imported["source_format"], "ccv2_json");
        assert_eq!(imported["character"]["name"], "Imported external character");
        let imported_id = imported["character"]["id"]
            .as_str()
            .expect("imported character id");
        let exported_path = TEST_DATA_DIR.path().join("exported-card.json");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/v1/characters/{imported_id}/export-external"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "output_path": exported_path,
                            "format": "ccv2_json"
                        })
                        .to_string(),
                    ))
                    .expect("external export request"),
            )
            .await
            .expect("external export response");
        assert_eq!(response.status(), StatusCode::OK);
        let exported: Value =
            serde_json::from_slice(&std::fs::read(exported_path).expect("exported external card"))
                .expect("exported external JSON");
        assert_eq!(exported["data"]["extensions"]["contract"]["preserve"], true);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/v1/characters?scope_id={TEST_SCOPE_ID}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "name": "HTTP test character",
                            "author_name": "momo-server test",
                            "description": "round trip",
                            "character_markdown": "Stay in character.",
                            "user_markdown": ""
                        })
                        .to_string(),
                    ))
                    .expect("create request"),
            )
            .await
            .expect("create response");
        assert_eq!(response.status(), StatusCode::OK);
        let created: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("create body"),
        )
        .expect("created JSON");
        assert_eq!(created["name"], "HTTP test character");
        let character_id = created["id"]
            .as_str()
            .expect("created character id")
            .to_owned();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/conversations")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "character_scope_id": TEST_SCOPE_ID,
                            "title": "core contract",
                            "character_id": character_id,
                        })
                        .to_string(),
                    ))
                    .expect("conversation request"),
            )
            .await
            .expect("conversation response");
        assert_eq!(response.status(), StatusCode::OK);
        let conversation: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("conversation body"),
        )
        .expect("conversation JSON");
        let conversation_id = conversation["id"]
            .as_str()
            .expect("created conversation id")
            .to_owned();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "conversation_id": conversation_id,
                            "role": "user",
                            "content": "hello from client",
                        })
                        .to_string(),
                    ))
                    .expect("message request"),
            )
            .await
            .expect("message response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/v1/semantic-graph/nodes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "target_file": "lore/vector-test.nsg",
                            "node": {
                                "id": "vector_test",
                                "graph_id": "default",
                                "type": "fact",
                                "importance": 0.8,
                                "mode": "canon",
                                "status": "active",
                                "zone": "2",
                                "anchors": [],
                                "condition": "",
                                "trigger": "",
                                "consequence": "The hidden lake is north of the village.",
                                "constraint": "",
                                "source_character_ids": [],
                                "inject_character_ids": [],
                                "edges": []
                            }
                        })
                        .to_string(),
                    ))
                    .expect("NSG write request"),
            )
            .await
            .expect("NSG write response");
        assert_eq!(response.status(), StatusCode::OK);

        let embedding_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("embedding listener");
        let embedding_address = embedding_listener.local_addr().expect("embedding address");
        tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = embedding_listener.accept().await.expect("accept embedding");
                let mut request = vec![0_u8; 32 * 1024];
                let read = socket
                    .read(&mut request)
                    .await
                    .expect("read embedding request");
                let request = String::from_utf8_lossy(&request[..read]);
                assert!(request.starts_with("POST /v1/embeddings "));
                let body = json!({
                    "model": "test-embedding",
                    "object": "list",
                    "data": [{"object": "embedding", "index": 0, "embedding": [1.0, 0.0, 0.0]}],
                    "usage": {"prompt_tokens": 3, "total_tokens": 3}
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write embedding response");
            }
        });
        let embedding = json!({
            "endpoint": {
                "base_url": format!("http://{embedding_address}/v1")
            },
            "profile": {
                "provider_id": "test",
                "endpoint_id": "local-fixture",
                "model": "test-embedding",
                "dimension": 3,
                "normalization": "l2"
            },
            "timeout_seconds": 10
        });
        let mut invalid_embedding = embedding.clone();
        invalid_embedding["profile"]["dimension"] = json!(0);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/embeddings/generate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "embedding": invalid_embedding,
                            "inputs": [{
                                "id": "invalid",
                                "text": "must not reach provider",
                                "purpose": "document"
                            }]
                        })
                        .to_string(),
                    ))
                    .expect("invalid embedding request"),
            )
            .await
            .expect("invalid embedding response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/embeddings/generate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "embedding": embedding,
                            "inputs": [{
                                "id": "direct",
                                "text": "direct embedding",
                                "purpose": "document"
                            }]
                        })
                        .to_string(),
                    ))
                    .expect("embedding request"),
            )
            .await
            .expect("embedding response");
        assert_eq!(response.status(), StatusCode::OK);
        let generated: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("embedding body"),
        )
        .expect("embedding JSON");
        assert_eq!(generated["model"], "test-embedding");
        assert_eq!(generated["usage"]["total_tokens"], 3);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/semantic-graph/vectors/rebuild")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "embedding": embedding,
                            "mode": "full",
                            "batch_size": 1
                        })
                        .to_string(),
                    ))
                    .expect("vector rebuild request"),
            )
            .await
            .expect("vector rebuild response");
        assert_eq!(response.status(), StatusCode::OK);
        let rebuilt: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("vector rebuild body"),
        )
        .expect("vector rebuild JSON");
        assert_eq!(rebuilt["node_count"], 1);
        assert_eq!(rebuilt["embedded_count"], 1);
        assert!(
            rebuilt["vector_space_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("momo-embedding-v1:"))
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/memory/retrieve-scoped")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scopes": [{
                                "scope_id": TEST_SCOPE_ID,
                                "label": "default",
                                "weight": 1
                            }],
                            "query": "unrelated query text",
                            "max_tokens": 1024,
                            "include_memory": false,
                            "include_semantic_graph": true,
                            "embedding": embedding
                        })
                        .to_string(),
                    ))
                    .expect("vectorized retrieval request"),
            )
            .await
            .expect("vectorized retrieval response");
        assert_eq!(response.status(), StatusCode::OK);
        let retrieved: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 128 * 1024)
                .await
                .expect("vectorized retrieval body"),
        )
        .expect("vectorized retrieval JSON");
        assert!(
            retrieved
                .as_array()
                .is_some_and(|items| { items.iter().any(|item| item["id"] == "vector_test") })
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/conversations/{conversation_id}/messages?scope_id={TEST_SCOPE_ID}"
                    ))
                    .body(Body::empty())
                    .expect("message list request"),
            )
            .await
            .expect("message list response");
        assert_eq!(response.status(), StatusCode::OK);
        let messages: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("message list body"),
        )
        .expect("message list JSON");
        assert_eq!(messages.as_array().map(Vec::len), Some(1));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/mo-state/compile")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": "00000000-0000-4000-8000-000000000011",
                            "retrieved_memory": [],
                            "retrieved_nsg": [],
                            "max_context_tokens": 8192,
                        })
                        .to_string(),
                    ))
                    .expect("state request"),
            )
            .await
            .expect("state response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/context/prepare")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "character_markdown": "Stay in character.",
                            "user_markdown": "",
                            "memory_markdown": "",
                            "state_context": "",
                            "nsg_markdown": "",
                            "messages": [],
                            "context_window": 8192,
                            "reserve_output_tokens": 1024,
                        })
                        .to_string(),
                    ))
                    .expect("context request"),
            )
            .await
            .expect("context response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/semantic-graph/nodes/list")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": "00000000-0000-4000-8000-000000000011",
                        })
                        .to_string(),
                    ))
                    .expect("NSG list request"),
            )
            .await
            .expect("NSG list response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/v1/characters/00000000-0000-4000-8000-000000000099")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(created.to_string()))
                    .expect("mismatched update request"),
            )
            .await
            .expect("mismatched update response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/characters?scope_id={TEST_SCOPE_ID}"))
                    .body(Body::empty())
                    .expect("list request"),
            )
            .await
            .expect("list response");
        assert_eq!(response.status(), StatusCode::OK);
        let characters: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("list body"),
        )
        .expect("characters JSON");
        assert!(characters.as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("name").and_then(Value::as_str) == Some("HTTP test character"))
        }));
    }

    #[tokio::test]
    async fn http_moc_host_modules_export_and_claim() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let extension = TEST_DATA_DIR.path().join("host-weather-module");
        std::fs::create_dir_all(&extension).expect("host module directory");
        std::fs::write(extension.join("module.json"), br#"{"unit":"celsius"}"#)
            .expect("host module payload");
        let output = TEST_DATA_DIR.path().join("host-module.moc");
        let claims = TEST_DATA_DIR.path().join("host-module-claims");
        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api("http://127.0.0.1:9/v1"),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(5),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });

        let export = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/moc/export")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "output_path": output,
                            "modules": [],
                            "host_modules": [{
                                "id": "weather",
                                "input_path": extension,
                                "dependencies": ["config"],
                                "import_order": 900
                            }]
                        })
                        .to_string(),
                    ))
                    .expect("host module export request"),
            )
            .await
            .expect("host module export response");
        assert_eq!(export.status(), StatusCode::OK);

        let import = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/moc/import")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": TEST_SCOPE_ID,
                            "input_path": output,
                            "conflict_mode": "replace",
                            "claim_unknown_to": claims
                        })
                        .to_string(),
                    ))
                    .expect("host module import request"),
            )
            .await
            .expect("host module import response");
        assert_eq!(import.status(), StatusCode::OK);
        let report: Value = serde_json::from_slice(
            &to_bytes(import.into_body(), 128 * 1024)
                .await
                .expect("host module report body"),
        )
        .expect("host module report JSON");
        assert_eq!(report["unknown_modules"][0]["id"], "weather");
        assert_eq!(
            std::fs::read_to_string(claims.join("weather/module.json"))
                .expect("claimed host payload"),
            r#"{"unit":"celsius"}"#
        );
    }

    #[tokio::test]
    async fn conversation_and_messages_fail_closed_across_scopes() {
        const OWNER_SCOPE: &str = "00000000-0000-4000-8000-000000000081";
        const OTHER_SCOPE: &str = "00000000-0000-4000-8000-000000000082";

        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let character: Value = serde_json::from_str(
            &simple::stage_character_json(
                OWNER_SCOPE.to_owned(),
                "scope-test".to_owned(),
                "Scoped character".to_owned(),
                String::new(),
                "Stay scoped.".to_owned(),
                String::new(),
            )
            .await
            .expect("stage scoped character"),
        )
        .expect("character JSON");
        let character_id = character["id"].as_str().expect("character ID").to_owned();
        let conversation: Value = serde_json::from_str(
            &simple::stage_conversation_json(
                None,
                OWNER_SCOPE.to_owned(),
                OWNER_SCOPE.to_owned(),
                "private conversation".to_owned(),
                character_id.clone(),
            )
            .await
            .expect("stage scoped conversation"),
        )
        .expect("conversation JSON");
        let conversation_id = conversation["id"]
            .as_str()
            .expect("conversation ID")
            .to_owned();
        let message: Value = serde_json::from_str(
            &simple::stage_message_json(
                OWNER_SCOPE.to_owned(),
                conversation_id.clone(),
                "user".to_owned(),
                "owner-only history".to_owned(),
            )
            .await
            .expect("stage owner message"),
        )
        .expect("message JSON");
        let message_id = message["id"].as_str().expect("message ID").to_owned();

        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api("http://127.0.0.1:9/v1"),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(5),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });

        let owner_read = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/conversations/{conversation_id}/messages?scope_id={OWNER_SCOPE}"
                    ))
                    .body(Body::empty())
                    .expect("owner read"),
            )
            .await
            .expect("owner response");
        assert_eq!(owner_read.status(), StatusCode::OK);

        let cross_scope_read = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/conversations/{conversation_id}/messages?scope_id={OTHER_SCOPE}"
                    ))
                    .body(Body::empty())
                    .expect("cross-scope read"),
            )
            .await
            .expect("cross-scope response");
        assert_eq!(cross_scope_read.status(), StatusCode::NOT_FOUND);

        let cross_scope_write = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": OTHER_SCOPE,
                            "conversation_id": conversation_id,
                            "role": "user",
                            "content": "must not be written"
                        })
                        .to_string(),
                    ))
                    .expect("cross-scope write"),
            )
            .await
            .expect("cross-scope write response");
        assert_eq!(cross_scope_write.status(), StatusCode::NOT_FOUND);

        let mut foreign_character = character.clone();
        foreign_character["scope_id"] = json!(OTHER_SCOPE);
        let mut foreign_conversation = conversation.clone();
        foreign_conversation["scope_id"] = json!(OTHER_SCOPE);
        let export_path = TEST_DATA_DIR.path().join("cross-scope-export.json");
        let cross_scope_resource_requests = vec![
            (
                "character export",
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/v1/characters/{character_id}/export-external"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "scope_id": OTHER_SCOPE,
                            "output_path": export_path,
                            "format": "ccv2_json"
                        })
                        .to_string(),
                    ))
                    .expect("cross-scope character export"),
            ),
            (
                "character update",
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/v1/characters/{character_id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(foreign_character.to_string()))
                    .expect("cross-scope character update"),
            ),
            (
                "character delete",
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!(
                        "/v1/characters/{character_id}?scope_id={OTHER_SCOPE}"
                    ))
                    .body(Body::empty())
                    .expect("cross-scope character delete"),
            ),
            (
                "conversation update",
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/v1/conversations/{conversation_id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(foreign_conversation.to_string()))
                    .expect("cross-scope conversation update"),
            ),
            (
                "conversation delete",
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!(
                        "/v1/conversations/{conversation_id}?scope_id={OTHER_SCOPE}"
                    ))
                    .body(Body::empty())
                    .expect("cross-scope conversation delete"),
            ),
            (
                "message update",
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/v1/messages/{message_id}?scope_id={OTHER_SCOPE}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(message.to_string()))
                    .expect("cross-scope message update"),
            ),
            (
                "message delete",
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/v1/messages/{message_id}?scope_id={OTHER_SCOPE}"))
                    .body(Body::empty())
                    .expect("cross-scope message delete"),
            ),
        ];
        for (route, request) in cross_scope_resource_requests {
            let response = app
                .clone()
                .oneshot(request)
                .await
                .unwrap_or_else(|error| panic!("{route} response: {error}"));
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{route}");
        }
        assert!(!export_path.exists());

        let cross_scope_response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/momo/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "input": "must not see owner history",
                            "momo": {
                                "request_id": "cross-scope-response",
                                "scope_id": OTHER_SCOPE,
                                "conversation_scope_id": OTHER_SCOPE,
                                "character_scope_id": OWNER_SCOPE,
                                "character_id": character_id,
                                "conversation_id": conversation_id,
                                "memory": false,
                                "semantic_graph": false,
                                "mo_state": false
                            }
                        })
                        .to_string(),
                    ))
                    .expect("cross-scope response request"),
            )
            .await
            .expect("cross-scope native response");
        assert_eq!(cross_scope_response.status(), StatusCode::BAD_REQUEST);

        let owner_messages: Value = serde_json::from_str(
            &simple::local_messages_json(OWNER_SCOPE.to_owned(), conversation_id)
                .await
                .expect("owner messages"),
        )
        .expect("messages JSON");
        assert_eq!(owner_messages.as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn responses_forwards_upstream_sse_deltas_before_completion() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let character: Value = serde_json::from_str(
            &simple::stage_character_json(
                TEST_SCOPE_ID.to_owned(),
                "stream-contract".to_owned(),
                "MO".to_owned(),
                "stream contract character".to_owned(),
                "Be concise.".to_owned(),
                String::new(),
            )
            .await
            .expect("stage character"),
        )
        .expect("character JSON");
        let character_id = character["id"].as_str().expect("character ID").to_owned();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock gateway");
        let address = listener.local_addr().expect("gateway address");
        tokio::spawn(async move {
            let (mut capability_socket, _) = listener.accept().await.expect("capability request");
            let mut request = vec![0_u8; 8 * 1024];
            let read = capability_socket
                .read(&mut request)
                .await
                .expect("read capability request");
            assert!(
                String::from_utf8_lossy(&request[..read])
                    .starts_with("GET /v1/models/conversation ")
            );
            let capability = json!({
                "id": "conversation",
                "object": "model",
                "momo": {"category": "chat", "context_window": 8192, "max_output_tokens": 128}
            })
            .to_string();
            capability_socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        capability.len(), capability
                    )
                    .as_bytes(),
                )
                .await
                .expect("write capability response");

            let (mut stream_socket, _) = listener.accept().await.expect("stream request");
            let read = stream_socket
                .read(&mut request)
                .await
                .expect("read stream request");
            let request_text = String::from_utf8_lossy(&request[..read]);
            assert!(request_text.starts_with("POST /v1/chat/completions "));
            assert!(request_text.contains("\"stream\":true"));
            let first = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n";
            let rest = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\\\"Shanghai\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
                "data: [DONE]\n\n"
            );
            stream_socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        first.len() + rest.len(),
                        first
                    )
                    .as_bytes(),
                )
                .await
                .expect("write first delta");
            stream_socket.flush().await.expect("flush first delta");
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            stream_socket
                .write_all(rest.as_bytes())
                .await
                .expect("write remaining deltas");
        });

        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api(format!("http://{address}/v1")),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/momo/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "model": "conversation",
                            "input": "stream this response",
                            "stream": true,
                            "tools": [{
                                "type": "function",
                                "name": "weather",
                                "parameters": {"type": "object"}
                            }],
                            "momo": {
                                "schema": "momo.responses/1.0",
                                "request_id": format!("stream-{}", simple::new_request_id()),
                                "scope_id": TEST_SCOPE_ID,
                                "conversation_scope_id": TEST_SCOPE_ID,
                                "character_scope_id": TEST_SCOPE_ID,
                                "character_id": character_id,
                                "memory": false,
                                "semantic_graph": false,
                                "mo_state": false
                            }
                        })
                        .to_string(),
                    ))
                    .expect("stream response request"),
            )
            .await
            .expect("stream response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );
        let mut chunks = response.into_body().into_data_stream();
        let first_chunk =
            tokio::time::timeout(std::time::Duration::from_millis(100), chunks.next())
                .await
                .expect("created event must be available before model completion")
                .expect("created chunk")
                .expect("created bytes");
        let mut body = first_chunk.to_vec();
        while let Some(chunk) = chunks.next().await {
            body.extend_from_slice(&chunk.expect("stream bytes"));
        }
        let body = String::from_utf8(body).expect("UTF-8 SSE");
        let events = body
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str::<Value>(data).expect("SSE JSON"))
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| event["sequence"].as_u64())
                .collect::<Vec<_>>(),
            (0..events.len() as u64).map(Some).collect::<Vec<_>>()
        );
        let deltas = events
            .iter()
            .filter(|event| event["type"] == "response.output_text.delta")
            .collect::<Vec<_>>();
        assert_eq!(deltas[0]["delta"], "Hel");
        assert_eq!(deltas[1]["delta"], "lo");
        assert_eq!(deltas[0]["output_index"], 0);
        assert_eq!(deltas[0]["content_index"], 0);
        assert!(deltas[0]["item_id"].as_str().is_some());
        let tool_added = events
            .iter()
            .find(|event| {
                event["type"] == "response.output_item.added"
                    && event["item"]["type"] == "function_call"
            })
            .expect("tool item added");
        assert_eq!(tool_added["item"]["call_id"], "call_1");
        assert_eq!(tool_added["output_index"], 1);
        let argument_delta = events
            .iter()
            .find(|event| event["type"] == "response.function_call_arguments.delta")
            .expect("function arguments delta");
        assert_eq!(argument_delta["delta"], "{\"city\":\"Shanghai\"}");
        let completed = events.last().expect("completed event");
        assert_eq!(completed["type"], "response.completed");
        assert_eq!(completed["response"]["output_text"], "Hello");
        assert_eq!(completed["response"]["output"][1]["call_id"], "call_1");
        assert_eq!(completed["response"]["usage"]["total_tokens"], 7);
    }

    #[tokio::test]
    async fn responses_orchestrates_once_and_replays_by_request_id() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let character: Value = serde_json::from_str(
            &simple::stage_character_json(
                TEST_SCOPE_ID.to_owned(),
                "contract".to_owned(),
                "MO".to_owned(),
                "contract character".to_owned(),
                "Be concise.".to_owned(),
                "The user is testing the contract.".to_owned(),
            )
            .await
            .expect("stage character"),
        )
        .expect("character JSON");
        let character_id = character["id"].as_str().expect("character ID").to_owned();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock gateway");
        let address = listener.local_addr().expect("gateway address");
        tokio::spawn(async move {
            for operation in 0..4 {
                let (mut socket, _) = listener.accept().await.expect("accept gateway request");
                let mut request = vec![0_u8; 32 * 1024];
                let read = socket
                    .read(&mut request)
                    .await
                    .expect("read gateway request");
                let request = String::from_utf8_lossy(&request[..read]);
                let body = match operation {
                    0 => {
                        assert!(request.starts_with("GET /v1/models/conversation "));
                        json!({
                            "id": "conversation",
                            "object": "model",
                            "momo": {
                                "category": "chat",
                                "context_window": 8192,
                                "max_output_tokens": 1024
                            }
                        })
                    }
                    1 => {
                        assert!(request.starts_with("GET /v1/models/embedding "));
                        json!({
                            "id": "embedding",
                            "object": "model",
                            "momo": {
                                "category": "embedding",
                                "embedding_profile": {
                                    "dimension": 3,
                                    "normalization": "none",
                                    "send_dimensions": false,
                                    "query_prefix": "",
                                    "document_prefix": ""
                                }
                            }
                        })
                    }
                    2 => {
                        assert!(request.starts_with("POST /v1/embeddings "));
                        assert!(request.contains("\"model\":\"embedding\""));
                        json!({
                            "object": "list",
                            "model": "embedding",
                            "data": [{"index": 0, "embedding": [0.1, 0.2, 0.3]}],
                            "usage": {"prompt_tokens": 2, "total_tokens": 2}
                        })
                    }
                    _ => {
                        assert!(request.starts_with("POST /v1/chat/completions "));
                        assert!(request.contains("\"model\":\"conversation\""));
                        json!({
                            "id": "chatcmpl-contract",
                            "choices": [{
                                "message": {"role": "assistant", "content": "Aligned response"},
                                "finish_reason": "stop"
                            }],
                            "usage": {
                                "prompt_tokens": 11,
                                "completion_tokens": 4,
                                "total_tokens": 15
                            }
                        })
                    }
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write gateway response");
            }
        });

        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api(format!("http://{address}/v1")),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });
        let mut request_body: Value =
            serde_json::from_str(include_str!("../../../contracts/1.0/response_request.json"))
                .expect("1.0 response fixture");
        request_body["momo"]["scope_id"] = json!(TEST_SCOPE_ID);
        request_body["momo"]["conversation_scope_id"] = json!(TEST_SCOPE_ID);
        request_body["momo"]["character_scope_id"] = json!(TEST_SCOPE_ID);
        request_body["momo"]["character_id"] = json!(character_id);
        let request_body = request_body.to_string();
        let invoke = || {
            Request::builder()
                .method(Method::POST)
                .uri("/v1/momo/responses")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.clone()))
                .expect("response request")
        };
        let first = app.clone().oneshot(invoke()).await.expect("first response");
        assert_eq!(first.status(), StatusCode::OK);
        let first: Value = serde_json::from_slice(
            &to_bytes(first.into_body(), 1024 * 1024)
                .await
                .expect("first body"),
        )
        .expect("first JSON");
        assert_eq!(first["output_text"], "Aligned response");
        assert_eq!(first["momo"]["warnings"], json!([]));
        assert_eq!(first["usage"]["total_tokens"], 15);

        drop(app);
        let restarted_app = build_app(AppState {
            data_dir: TEST_DATA_DIR.path().to_string_lossy().into_owned(),
            momo_api: test_momo_api(format!("http://{address}/v1")),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });
        let replay = restarted_app
            .clone()
            .oneshot(invoke())
            .await
            .expect("replayed response");
        assert_eq!(replay.status(), StatusCode::OK);
        let replay: Value = serde_json::from_slice(
            &to_bytes(replay.into_body(), 1024 * 1024)
                .await
                .expect("replay body"),
        )
        .expect("replay JSON");
        assert_eq!(replay, first);
        let conflict = restarted_app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/momo/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.replace(
                        "Hello from the MOMO 1.0 cross-repository contract.",
                        "Different input.",
                    )))
                    .expect("conflicting request"),
            )
            .await
            .expect("conflict response");
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let conversation_id = first["momo"]["conversation_id"]
            .as_str()
            .expect("conversation id");
        let messages: Value = serde_json::from_str(
            &simple::local_messages_json(TEST_SCOPE_ID.to_owned(), conversation_id.to_owned())
                .await
                .expect("stored messages"),
        )
        .expect("messages JSON");
        assert_eq!(messages.as_array().map(Vec::len), Some(2));
    }

    #[tokio::test]
    async fn multimodal_conversation_receives_original_image_and_roleplay_prompt() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let character: Value = serde_json::from_str(
            &simple::stage_character_json(
                TEST_SCOPE_ID.to_owned(),
                "multimodal-test".to_owned(),
                "Direct multimodal character".to_owned(),
                String::new(),
                "Keep the roleplay voice.".to_owned(),
                String::new(),
            )
            .await
            .expect("stage multimodal character"),
        )
        .expect("character JSON");
        let character_id = character["id"].as_str().expect("character id").to_owned();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock multimodal gateway");
        let address = listener.local_addr().expect("gateway address");
        let gateway = tokio::spawn(async move {
            let (mut capability_socket, _) = listener.accept().await.expect("capability request");
            let mut request = vec![0_u8; 64 * 1024];
            let read = capability_socket
                .read(&mut request)
                .await
                .expect("read capability request");
            assert!(
                String::from_utf8_lossy(&request[..read])
                    .starts_with("GET /v1/models/conversation ")
            );
            let capability = json!({
                "id": "conversation",
                "object": "model",
                "momo": {
                    "category": "chat",
                    "context_window": 8192,
                    "max_output_tokens": 1024,
                    "modalities": ["text", "image"]
                }
            })
            .to_string();
            capability_socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        capability.len(), capability
                    )
                    .as_bytes(),
                )
                .await
                .expect("write capability response");

            let (mut chat_socket, _) = listener.accept().await.expect("chat request");
            let read = chat_socket
                .read(&mut request)
                .await
                .expect("read chat request");
            let request_text = String::from_utf8_lossy(&request[..read]);
            assert!(request_text.starts_with("POST /v1/chat/completions "));
            assert!(request_text.contains("Keep the roleplay voice."));
            assert!(request_text.contains("https://example.test/original.png"));
            assert!(request_text.contains("\"type\":\"image_url\""));
            assert!(!request_text.contains("FALLBACK_ONLY_PROMPT"));
            let completion = json!({
                "id": "chatcmpl-multimodal",
                "choices": [{
                    "message": {"role": "assistant", "content": "I can see it directly."},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 9, "completion_tokens": 5, "total_tokens": 14}
            })
            .to_string();
            chat_socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        completion.len(), completion
                    )
                    .as_bytes(),
                )
                .await
                .expect("write chat response");
        });

        let mut config = MomoConfig::default();
        config.vision.enabled = true;
        config.vision.prompt = "FALLBACK_ONLY_PROMPT".to_owned();
        let momo_api = Arc::new(MomoApiService::new(
            format!("http://{address}/v1"),
            None,
            reqwest::Client::new(),
            Arc::new(config),
        ));
        let app = build_app(AppState {
            data_dir: initialized_dir,
            momo_api,
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/momo/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "model": "conversation",
                            "input": [{
                                "type": "message",
                                "role": "user",
                                "content": [
                                    {"type": "input_text", "text": "Respond in character."},
                                    {"type": "input_image", "image_url": "https://example.test/original.png", "detail": "high"}
                                ]
                            }],
                            "momo": {
                                "request_id": "direct-multimodal-contract-1",
                                "character_id": character_id,
                                "scope_id": TEST_SCOPE_ID,
                                "conversation_scope_id": TEST_SCOPE_ID,
                                "character_scope_id": TEST_SCOPE_ID,
                                "memory": false,
                                "semantic_graph": false,
                                "mo_state": false
                            }
                        })
                        .to_string(),
                    ))
                    .expect("multimodal response request"),
            )
            .await
            .expect("multimodal response");
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("response body"),
        )
        .expect("response JSON");
        assert_eq!(body["output_text"], "I can see it directly.");
        assert_eq!(body["momo"]["request_audit"]["vision"]["applied"], false);
        assert_eq!(
            body["momo"]["request_audit"]["vision"]["mode"],
            "direct_multimodal"
        );
        gateway.await.expect("mock gateway task");
    }

    #[tokio::test]
    async fn successful_turns_drive_persistent_background_maintenance() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        let scope_id = "00000000-0000-4000-8000-000000000077".to_owned();
        simple::append_maintenance_turn_json(
            json!({
                "request_id": "maintenance-contract-1",
                "scope_id": scope_id,
                "user_content": "The moon gate requires a silver key.",
                "assistant_content": "Understood."
            })
            .to_string(),
            true,
            true,
        )
        .await
        .expect("register maintenance turn");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("maintenance gateway");
        let address = listener.local_addr().expect("maintenance address");
        tokio::spawn(async move {
            for expected_model in ["memory_distillation", "semantic_graph_governance"] {
                let (mut socket, _) = listener.accept().await.expect("accept maintenance");
                let mut request = vec![0_u8; 32 * 1024];
                let read = socket.read(&mut request).await.expect("read maintenance");
                let request = String::from_utf8_lossy(&request[..read]);
                assert!(request.contains(&format!("\"model\":\"{expected_model}\"")));
                let request_body = request
                    .split_once("\r\n\r\n")
                    .expect("maintenance HTTP body")
                    .1;
                let request_json: Value =
                    serde_json::from_str(request_body).expect("maintenance request JSON");
                let maintenance_input: Value = serde_json::from_str(
                    request_json["messages"][1]["content"]
                        .as_str()
                        .expect("structured maintenance input"),
                )
                .expect("maintenance input JSON");
                assert_eq!(
                    maintenance_input["maintenance_kind"],
                    if expected_model == "memory_distillation" {
                        "memory"
                    } else {
                        "semantic_graph"
                    }
                );
                assert!(maintenance_input["existing_context"].is_array());
                assert_eq!(
                    maintenance_input["pending_turns"][0]["request_id"],
                    "maintenance-contract-1"
                );
                assert!(
                    maintenance_input["current_unix_timestamp"]
                        .as_i64()
                        .is_some()
                );
                let body = json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": "patches: []"},
                        "finish_reason": "stop"
                    }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write maintenance");
            }
        });
        let state = AppState {
            data_dir: initialized_dir,
            momo_api: test_momo_api(format!("http://{address}/v1")),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
        };
        state
            .momo_api
            .maintain(&scope_id, momo_core::MaintenanceKind::Memory, 1)
            .await
            .expect("memory maintenance");
        state
            .momo_api
            .maintain(&scope_id, momo_core::MaintenanceKind::SemanticGraph, 1)
            .await
            .expect("NSG maintenance");
        for kind in ["memory", "semantic_graph"] {
            let pending: Value = serde_json::from_str(
                &simple::pending_maintenance_turns_json(scope_id.clone(), kind.to_owned(), 10)
                    .await
                    .expect("pending turns"),
            )
            .expect("pending JSON");
            assert_eq!(pending, json!([]));
        }
    }
}
