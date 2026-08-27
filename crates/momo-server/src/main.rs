use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    env,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize},
    },
};

mod http_error;
mod response_stream;

use http_error::ApiError;
#[cfg(test)]
use http_error::sanitize_error_message;
use response_stream::ResponseStream;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, rejection::JsonRejection},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use momo_core::{
    MAX_RESPONSE_REQUEST_BYTES, MomoApiError, MomoApiErrorKind, MomoApiService, MomoConfig,
    MomoResponse, MomoResponseRequest, ResponseOutputItem,
    api::simple,
    momo_domain::{CharacterCard, Conversation, Message},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_stream::wrappers::{ReceiverStream, UnboundedReceiverStream};
use tower_http::trace::TraceLayer;

const DEFAULT_BIND: &str = "127.0.0.1:8765";
const DEFAULT_DATA_DIR: &str = ".momo-data";
const DEFAULT_SCOPE_ID: &str = "00000000-0000-4000-8000-000000000001";
const ALLOW_REMOTE_ENV: &str = "MOMO_SERVER_ALLOW_REMOTE";
const MAX_PUBLIC_ERROR_BYTES: usize = 2 * 1024;
const RESPONSE_STREAM_BUFFER: usize = 64;

#[derive(Clone)]
struct AppState {
    data_dir: String,
    scope_id: String,
    gateway_origin: String,
    gateway_api_key: Option<String>,
    responses: Arc<Mutex<HashMap<String, MomoResponse>>>,
    response_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    momo_api: Arc<MomoApiService>,
    maintenance_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    response_cancellations: Arc<Mutex<HashSet<String>>>,
    response_concurrency: Arc<Semaphore>,
    response_timeout: std::time::Duration,
    metrics: Arc<Mutex<HashMap<String, RouteMetrics>>>,
    memory_distill_every: usize,
    nsg_govern_every: usize,
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
    title: String,
    character_id: Option<String>,
}

#[derive(Deserialize)]
struct CreateMessageRequest {
    conversation_id: String,
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct CreateCharacterRequest {
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
    input_path: String,
    format: momo_core::ExternalCharacterImportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportExternalCharacterRequest {
    output_path: String,
    format: momo_core::ExternalCharacterExportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportPreservedCharacterSourceRequest {
    output_path: String,
}

#[derive(Deserialize)]
struct ChatRequest {
    base_url: String,
    api_key: Option<String>,
    model: String,
    messages: Vec<Value>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    request_parameters: Value,
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
    conversation_id: String,
    patch_yaml: String,
    review_mode: String,
}

#[derive(Deserialize)]
struct IncludeResolvedQuery {
    include_resolved: Option<bool>,
}

#[derive(Deserialize)]
struct WriteNsgNodeRequest {
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
    modules: Vec<momo_core::MocModule>,
    #[serde(default)]
    compatibility: momo_core::MocCompatibility,
    #[serde(default)]
    character_id: Option<uuid::Uuid>,
    #[serde(default)]
    protection: momo_core::MocProtection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MocImportRequest {
    input_path: String,
    #[serde(default = "default_conflict_mode")]
    conflict_mode: momo_core::ConflictMode,
    #[serde(default)]
    protection: momo_core::MocProtection,
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
    let scope_id = env::var("MOMO_SCOPE_ID").unwrap_or_else(|_| DEFAULT_SCOPE_ID.to_owned());
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
        scope_id.clone(),
        gateway_origin.clone(),
        gateway_api_key.clone(),
        gateway_client.clone(),
        Arc::clone(&momo_config),
    ));
    let state = AppState {
        data_dir: initialized_dir,
        scope_id,
        gateway_origin,
        gateway_api_key,
        responses: Arc::new(Mutex::new(HashMap::new())),
        response_locks: Arc::new(Mutex::new(HashMap::new())),
        momo_api,
        maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
        response_cancellations: Arc::new(Mutex::new(HashSet::new())),
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
        memory_distill_every: env_usize("MOMO_MEMORY_DISTILL_EVERY", 12, 1, 200)?,
        nsg_govern_every: env_usize("MOMO_NSG_GOVERN_EVERY", 12, 1, 200)?,
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
        .route("/chat/complete", post(chat_complete))
        .route("/chat/stream", post(chat_stream))
        .route("/chat/cancel/:request_id", post(cancel_chat))
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
        "schema": "momo.metrics/0.5",
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

async fn ensure_response_active(state: &AppState, request_id: &str) -> Result<(), ApiError> {
    if state
        .response_cancellations
        .lock()
        .await
        .contains(request_id)
    {
        Err(ApiError::cancelled())
    } else {
        Ok(())
    }
}

async fn list_characters(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_characters_json(state.scope_id).await)
}

async fn create_character(
    State(state): State<AppState>,
    Json(request): Json<CreateCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::stage_character_json(
            state.scope_id,
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
    State(state): State<AppState>,
    Json(request): Json<ImportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::import_external_character_json(
            json!({
                "scope_id": state.scope_id,
                "input_path": request.input_path,
                "format": request.format,
            })
            .to_string(),
        )
        .await,
    )
}

async fn export_external_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ExportExternalCharacterRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::export_external_character_json(
            json!({
                "scope_id": state.scope_id,
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
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ExportPreservedCharacterSourceRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::export_preserved_character_source_json(
            json!({
                "scope_id": state.scope_id,
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
    json_result(simple::stage_character_update_json(to_json_string(&character)?).await)
}

async fn delete_character(Path(id): Path<String>) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_character_delete(id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn list_conversations(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_conversations_json(state.scope_id).await)
}

async fn create_conversation(
    State(state): State<AppState>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::stage_conversation_json(None, state.scope_id, request.title, request.character_id)
            .await,
    )
}

async fn update_conversation(
    Path(id): Path<String>,
    Json(conversation): Json<Conversation>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, conversation.id)?;
    json_result(simple::stage_conversation_update_json(to_json_string(&conversation)?).await)
}

async fn delete_conversation(Path(id): Path<String>) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_conversation_delete(id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn list_messages(Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    json_result(simple::local_messages_json(id).await)
}

async fn create_message(
    Json(request): Json<CreateMessageRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::stage_message_json(request.conversation_id, request.role, request.content).await,
    )
}

async fn update_message(
    Path(id): Path<String>,
    Json(message): Json<Message>,
) -> Result<Json<Value>, ApiError> {
    ensure_resource_id(&id, message.id)?;
    json_result(simple::stage_message_update_json(to_json_string(&message)?).await)
}

async fn delete_message(Path(id): Path<String>) -> Result<Json<OkResponse>, ApiError> {
    simple::stage_message_delete(id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn create_response(
    State(state): State<AppState>,
    payload: Result<Json<MomoResponseRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|rejection| {
        let status = rejection.status();
        let code = if status == StatusCode::PAYLOAD_TOO_LARGE {
            "request_too_large"
        } else {
            "invalid_json"
        };
        ApiError::new(status, code, rejection.body_text(), false)
    })?;
    let input = request
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let request_id = request
        .momo
        .request_id
        .clone()
        .unwrap_or_else(simple::new_request_id);
    let request_fingerprint = response_request_fingerprint(&request)?;
    if request.stream || request.momo.stream {
        let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(RESPONSE_STREAM_BUFFER);
        let stream = ResponseStream {
            tx,
            sequence: Arc::new(AtomicU64::new(0)),
            transmitted_bytes: Arc::new(AtomicUsize::new(0)),
        };
        stream
            .send(json!({
                "type": "response.created",
                "request_id": request_id,
                "response": {"id": format!("resp_{request_id}"), "status": "in_progress"},
            }))
            .map_err(ApiError::internal)?;
        tokio::spawn({
            let stream = stream.clone();
            let request_id = request_id.clone();
            async move {
                match execute_response_bounded(
                    state,
                    request,
                    request_id.clone(),
                    request_fingerprint,
                    input,
                    Some(stream.clone()),
                )
                .await
                {
                    Ok(response) => {
                        let message_id = format!("msg_{request_id}");
                        let _ = stream.send(json!({
                            "type": "response.output_text.done",
                            "request_id": request_id,
                            "item_id": message_id.as_str(),
                            "output_index": 0,
                            "content_index": 0,
                            "text": response.output_text.as_str(),
                        }));
                        let _ = stream.send(json!({
                            "type": "response.content_part.done",
                            "request_id": request_id,
                            "item_id": message_id.as_str(),
                            "output_index": 0,
                            "content_index": 0,
                            "part": {
                                "type": "output_text",
                                "text": response.output_text.as_str(),
                                "annotations": [],
                            },
                        }));
                        if let Some(item) = response.output.first() {
                            let _ = stream.send(json!({
                                "type": "response.output_item.done",
                                "request_id": request_id,
                                "output_index": 0,
                                "item": item,
                            }));
                        }
                        for (output_index, item) in response.output.iter().enumerate().skip(1) {
                            if let ResponseOutputItem::FunctionCall { id, arguments, .. } = item {
                                let _ = stream.send(json!({
                                    "type": "response.function_call_arguments.done",
                                    "request_id": request_id,
                                    "item_id": id,
                                    "output_index": output_index,
                                    "arguments": arguments,
                                }));
                            }
                            let _ = stream.send(json!({
                                "type": "response.output_item.done",
                                "request_id": request_id,
                                "output_index": output_index,
                                "item": item,
                            }));
                        }
                        let _ = stream.send(json!({
                            "type": "response.completed",
                            "request_id": request_id,
                            "response": response,
                        }));
                    }
                    Err(error) => {
                        let _ = stream.send(json!({
                            "type": "response.failed",
                            "request_id": request_id,
                            "error": error.error,
                        }));
                    }
                }
            }
        });
        return Ok(Sse::new(ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response());
    }
    let response =
        execute_response_bounded(state, request, request_id, request_fingerprint, input, None)
            .await?;
    Ok(Json(response).into_response())
}

async fn execute_response_bounded(
    state: AppState,
    request: MomoResponseRequest,
    request_id: String,
    request_fingerprint: String,
    input: String,
    stream: Option<ResponseStream>,
) -> Result<MomoResponse, ApiError> {
    let started = std::time::Instant::now();
    let route = request.model.clone();
    let permit = Arc::clone(&state.response_concurrency)
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::rate_limited("response concurrency limit reached")
                .with_request_id(&request_id)
        })?;
    update_route_metrics(&state, &route, |metrics| {
        metrics.requests = metrics.requests.saturating_add(1);
        metrics.inflight = metrics.inflight.saturating_add(1);
    })
    .await;
    let result = match tokio::time::timeout(
        state.response_timeout,
        execute_response(
            state.clone(),
            request,
            request_id.clone(),
            request_fingerprint,
            input,
            stream,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            let _ = simple::cancel_chat(request_id.clone());
            Err(ApiError::gateway_timeout(
                "response orchestration timed out",
            ))
        }
    };
    drop(permit);
    state
        .response_cancellations
        .lock()
        .await
        .remove(&request_id);
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    update_route_metrics(&state, &route, |metrics| {
        metrics.inflight = metrics.inflight.saturating_sub(1);
        metrics.latency_ms = metrics.latency_ms.saturating_add(elapsed);
        match &result {
            Ok(response) => {
                metrics.succeeded = metrics.succeeded.saturating_add(1);
                metrics.input_tokens = metrics
                    .input_tokens
                    .saturating_add(response.usage.input_tokens);
                metrics.output_tokens = metrics
                    .output_tokens
                    .saturating_add(response.usage.output_tokens);
            }
            Err(error) if error.error.code == "cancelled" => {
                metrics.cancelled = metrics.cancelled.saturating_add(1);
            }
            Err(error) if error.error.code == "timeout" => {
                metrics.timed_out = metrics.timed_out.saturating_add(1);
            }
            Err(_) => metrics.failed = metrics.failed.saturating_add(1),
        }
    })
    .await;
    result.map_err(|error| error.with_request_id(&request_id))
}

async fn execute_response(
    state: AppState,
    request: MomoResponseRequest,
    request_id: String,
    request_fingerprint: String,
    input: String,
    stream: Option<ResponseStream>,
) -> Result<MomoResponse, ApiError> {
    ensure_response_active(&state, &request_id).await?;
    let request_lock = {
        let mut locks = state.response_locks.lock().await;
        Arc::clone(
            locks
                .entry(request_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    };
    let _request_guard = request_lock.lock().await;
    ensure_response_active(&state, &request_id).await?;
    let persisted = load_response_operation(&request_id).await?;
    if let Some(operation) = &persisted {
        if operation["request_fingerprint"].as_str() != Some(request_fingerprint.as_str()) {
            return Err(ApiError::conflict(
                "request_id was already used with a different response request",
            ));
        }
        if let Some(response_json) = operation["response_json"].as_str() {
            let response: MomoResponse = serde_json::from_str(response_json)
                .map_err(|error| ApiError::internal(error.to_string()))?;
            return Ok(response);
        }
    }
    let response = if let Some(cached) = state.responses.lock().await.get(&request_id).cloned() {
        cached
    } else {
        let mut response = state
            .momo_api
            .respond(
                &request,
                &request_id,
                &request_fingerprint,
                persisted.as_ref(),
                &input,
                stream
                    .as_ref()
                    .map(|value| value as &dyn momo_core::MomoResponseEventSink),
            )
            .await
            .map_err(momo_api_error)?;
        let maintenance_scope = request
            .momo
            .scope_id
            .clone()
            .unwrap_or_else(|| state.scope_id.clone());
        let memory_maintenance =
            !response.output_text.is_empty() && (request.momo.memory || request.momo.mo_state);
        let nsg_maintenance = !response.output_text.is_empty()
            && (request.momo.semantic_graph || request.momo.mo_state);
        let maintenance_registered = if memory_maintenance || nsg_maintenance {
            match simple::append_maintenance_turn_json(
                json!({
                    "request_id": request_id,
                    "scope_id": maintenance_scope,
                    "user_content": input,
                    "assistant_content": response.output_text,
                })
                .to_string(),
                memory_maintenance,
                nsg_maintenance,
            )
            .await
            {
                Ok(()) => true,
                Err(error) => {
                    response.momo.warnings.push(format!(
                        "background maintenance was not registered: {error}"
                    ));
                    false
                }
            }
        } else {
            false
        };
        simple::complete_response_operation(
            request_id.clone(),
            serde_json::to_string(&response)
                .map_err(|error| ApiError::internal(error.to_string()))?,
        )
        .await
        .map_err(ApiError::internal)?;
        let mut responses = state.responses.lock().await;
        if responses.len() >= 1_024 {
            responses.clear();
            state.momo_api.clear_attempts().await;
            state.response_locks.lock().await.clear();
        }
        responses.insert(request_id.clone(), response.clone());
        if maintenance_registered {
            schedule_background_maintenance(state.clone(), maintenance_scope);
        }
        response
    };

    Ok(response)
}

fn model_api_error(error: String) -> ApiError {
    let lowercase = error.to_ascii_lowercase();
    if lowercase.contains("cancelled") {
        return ApiError::cancelled();
    }
    if lowercase.contains("timed out") || lowercase.contains("timeout") {
        return ApiError::gateway_timeout("model gateway timed out");
    }
    if let Some(status) = parse_upstream_status(&error) {
        if status == 429 {
            let mut api_error = ApiError::rate_limited("model gateway rate limit exceeded");
            api_error.error.upstream_status = Some(status);
            return api_error;
        }
        let mut api_error = ApiError::bad_gateway(format!("model gateway returned HTTP {status}"));
        api_error.error.upstream_status = Some(status);
        api_error.error.retryable = status >= 500;
        return api_error;
    }
    ApiError::bad_gateway(error)
}

fn momo_api_error(error: MomoApiError) -> ApiError {
    match error.kind {
        MomoApiErrorKind::BadRequest => ApiError::bad_request(error.message),
        MomoApiErrorKind::Model => model_api_error(error.message),
        MomoApiErrorKind::Internal => ApiError::internal(error.message),
    }
}

fn parse_upstream_status(message: &str) -> Option<u16> {
    let marker = "HTTP ";
    let start = message.find(marker)? + marker.len();
    message[start..]
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

async fn load_response_operation(request_id: &str) -> Result<Option<Value>, ApiError> {
    simple::response_operation_json(request_id.to_owned())
        .await
        .map_err(ApiError::internal)?
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| ApiError::internal(error.to_string()))
        })
        .transpose()
}

fn response_request_fingerprint(request: &MomoResponseRequest) -> Result<String, ApiError> {
    let mut normalized = request.clone();
    normalized.stream = false;
    normalized.momo.stream = false;
    normalized.momo.request_id = None;
    let encoded = serde_json::to_vec(&normalized)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn schedule_background_maintenance(state: AppState, scope_id: String) {
    for (kind, threshold) in [
        ("memory", state.memory_distill_every),
        ("semantic_graph", state.nsg_govern_every),
    ] {
        let state = state.clone();
        let scope_id = scope_id.clone();
        tokio::spawn(async move {
            if let Err(error) = run_background_maintenance(state, scope_id, kind, threshold).await {
                tracing::warn!(kind, %error, "background response maintenance failed; turns remain pending");
            }
        });
    }
}

async fn run_background_maintenance(
    state: AppState,
    scope_id: String,
    kind: &'static str,
    threshold: usize,
) -> Result<(), String> {
    let lock_key = format!("{scope_id}:{kind}");
    let task_lock = {
        let mut locks = state.maintenance_locks.lock().await;
        Arc::clone(
            locks
                .entry(lock_key)
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    };
    let _guard = task_lock.lock().await;
    let pending =
        simple::pending_maintenance_turns_json(scope_id.clone(), kind.to_owned(), threshold)
            .await?;
    let turns: Vec<Value> = serde_json::from_str(&pending).map_err(|error| error.to_string())?;
    if turns.len() < threshold {
        return Ok(());
    }
    let transcript = turns
        .iter()
        .enumerate()
        .map(|(index, turn)| {
            format!(
                "Turn {}\nUser:\n{}\nAssistant:\n{}",
                index + 1,
                turn.get("user_content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                turn.get("assistant_content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let (route, system) = match kind {
        "memory" => (
            "memory_distillation",
            concat!(
                "You are the MOMO DMW memory distiller. Output YAML only with root key patches. ",
                "Store only explicit durable facts useful in future conversations. Never store guesses, ",
                "questions, greetings, transient mood, secrets, or a general transcript summary. ",
                "Supported operations are create, append, replace, and update_frontmatter. ",
                "Use safe relative .md targets. If nothing qualifies, output exactly: patches: []"
            ),
        ),
        "semantic_graph" => (
            "semantic_graph_governance",
            concat!(
                "You are the MOMO NSG governor. Output YAML only with root key patches. ",
                "Create only durable narrative rules, lore, causal relations, or constraints. ",
                "Automatic create_node operations must use mode draft, status active, and zone auto. ",
                "Never directly modify Canon; use revision_candidate with evidence. ",
                "If nothing qualifies, output exactly: patches: []"
            ),
        ),
        _ => return Err("unknown maintenance kind".to_owned()),
    };
    let completion = simple::chat_complete_json(
        json!({
            "base_url": state.gateway_origin,
            "api_key": state.gateway_api_key,
            "model": route,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": transcript}
            ],
            "request_parameters": {
                "max_tokens": 2048,
                "momo_hop": 1
            }
        })
        .to_string(),
    )
    .await?;
    let completion: Value = serde_json::from_str(&completion).map_err(|error| error.to_string())?;
    let patch = completion
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "maintenance model returned no text".to_owned())?;
    match kind {
        "memory" => {
            simple::apply_memory_patch_json(scope_id.clone(), patch.to_owned()).await?;
        }
        "semantic_graph" => {
            simple::apply_nsg_patch_json(scope_id.clone(), patch.to_owned(), false).await?;
        }
        _ => unreachable!(),
    }
    let request_ids = turns
        .iter()
        .filter_map(|turn| turn.get("request_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    simple::mark_maintenance_turns_done(request_ids, kind.to_owned()).await?;
    Ok(())
}

async fn chat_complete(Json(request): Json<ChatRequest>) -> Result<Json<Value>, ApiError> {
    let request_json = json!({
        "base_url": request.base_url,
        "api_key": request.api_key,
        "model": request.model,
        "messages": request.messages,
        "temperature": request.temperature,
        "request_parameters": normalized_request_parameters(&request.request_parameters)?,
    })
    .to_string();
    json_result(simple::chat_complete_json(request_json).await)
}

async fn chat_stream(
    Json(request): Json<ChatRequest>,
) -> Result<Sse<UnboundedReceiverStream<Result<Event, Infallible>>>, ApiError> {
    let request_id = simple::new_request_id();
    let request_json = json!({
        "request_id": request_id,
        "base_url": request.base_url,
        "api_key": request.api_key,
        "model": request.model,
        "messages": request.messages,
        "temperature": request.temperature,
        "request_parameters": normalized_request_parameters(&request.request_parameters)?,
    })
    .to_string();
    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    let sink_tx = tx.clone();
    let sink = move |event_json: String| {
        sink_tx
            .send(Ok(Event::default().data(event_json)))
            .map_err(|error| error.to_string())
    };

    tokio::spawn(async move {
        let start = json!({
            "type": "start",
            "request_id": request_id,
        });
        let _ = tx.send(Ok(Event::default().data(start.to_string())));
        if let Err(error) = simple::chat_stream_json(request_json, sink).await {
            let _ = tx.send(Ok(Event::default().data(
                json!({
                    "type": "error",
                    "error": error,
                })
                .to_string(),
            )));
        }
    });

    Ok(Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

async fn cancel_chat(Path(request_id): Path<String>) -> Json<Value> {
    Json(json!({
        "cancelled": simple::cancel_chat(request_id),
    }))
}

async fn cancel_response(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Json<Value> {
    let known = state.response_locks.lock().await.contains_key(&request_id);
    if known {
        state
            .response_cancellations
            .lock()
            .await
            .insert(request_id.clone());
    }
    let upstream = simple::cancel_chat(request_id.clone());
    Json(json!({
        "request_id": request_id,
        "cancelled": known || upstream,
    }))
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

async fn run_memory_maintenance(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    json_result(simple::run_memory_maintenance_json(state.scope_id).await)
}

async fn list_memory_documents(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    json_result(simple::list_memory_documents_json(state.scope_id).await)
}

async fn read_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::read_memory_document_json(state.scope_id, id).await)
}

async fn update_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateMemoryDocumentRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::update_memory_document_json(state.scope_id, id, request.markdown).await)
}

async fn archive_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::archive_memory_document_json(state.scope_id, id).await)
}

async fn restore_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::restore_memory_document_json(state.scope_id, id).await)
}

async fn delete_memory_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::delete_memory_document_json(state.scope_id, id).await)
}

async fn apply_memory_patch(
    Json(request): Json<ApplyMemoryPatchRequest>,
) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    ok_json(simple::apply_memory_patch_json(scope_id, request.patch_yaml).await)
}

async fn submit_memory_patch_review(
    State(state): State<AppState>,
    Json(request): Json<SubmitMemoryPatchReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::submit_memory_patch_review_json(
            state.scope_id,
            request.conversation_id,
            request.patch_yaml,
            request.review_mode,
        )
        .await,
    )
}

async fn list_memory_patch_reviews(
    State(state): State<AppState>,
    Query(query): Query<IncludeResolvedQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::list_memory_patch_reviews_json(
            state.scope_id,
            query.include_resolved.unwrap_or(false),
        )
        .await,
    )
}

async fn approve_memory_patch_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::approve_memory_patch_review_json(state.scope_id, id).await)
}

async fn reject_memory_patch_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::reject_memory_patch_review_json(state.scope_id, id).await)
}

async fn list_nsg_nodes(Json(request): Json<ScopeRequest>) -> Result<Json<Value>, ApiError> {
    let scope_id = validate_scope_id(request.scope_id)?;
    json_result(simple::list_nsg_nodes_json(scope_id, false).await)
}

async fn write_nsg_node(
    State(state): State<AppState>,
    Json(request): Json<WriteNsgNodeRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(
        simple::write_nsg_node_json(
            state.scope_id,
            request.target_file,
            to_json_string(&request.node)?,
        )
        .await,
    )
}

async fn archive_nsg_node(
    State(state): State<AppState>,
    Json(request): Json<NsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::archive_nsg_node_json(state.scope_id, request.target_file).await)
}

async fn delete_nsg_node(
    State(state): State<AppState>,
    Json(request): Json<NsgTargetRequest>,
) -> Result<Json<Value>, ApiError> {
    ok_json(simple::delete_nsg_node_json(state.scope_id, request.target_file).await)
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
    State(state): State<AppState>,
    Query(query): Query<NsgVectorStatusQuery>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::nsg_vector_status_json(state.scope_id, query.vector_space_id.unwrap_or_default())
            .await,
    )
}

async fn rebuild_nsg_vector_index(
    State(state): State<AppState>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::rebuild_nsg_vector_index_json(state.scope_id, to_json_string(&request)?).await,
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

async fn export_moc(
    State(state): State<AppState>,
    Json(request): Json<MocExportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::export_moc_json(to_json_string(&json!({
            "output_path": request.output_path,
            "scope_id": state.scope_id,
            "settings": request.settings,
            "modules": request.modules,
            "compatibility": request.compatibility,
            "character_id": request.character_id,
            "protection": request.protection,
        }))?)
        .await,
    )
}

async fn import_moc(
    State(state): State<AppState>,
    Json(request): Json<MocImportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::import_moc_json(
            request.input_path,
            state.scope_id,
            request.conflict_mode,
            request.protection,
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

fn normalized_request_parameters(value: &Value) -> Result<Value, ApiError> {
    match value {
        Value::Object(_) => Ok(value.clone()),
        Value::Null => Ok(json!({})),
        _ => Err(ApiError::bad_request(
            "request_parameters must be an object",
        )),
    }
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
            DEFAULT_SCOPE_ID,
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
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
            gateway_origin: "http://127.0.0.1:9/v1".to_owned(),
            gateway_api_key: None,
            responses: Arc::new(Mutex::new(HashMap::new())),
            response_locks: Arc::new(Mutex::new(HashMap::new())),
            momo_api: test_momo_api("http://127.0.0.1:9/v1"),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
            response_cancellations: Arc::new(Mutex::new(HashSet::new())),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
            memory_distill_every: 12,
            nsg_govern_every: 12,
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
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
            gateway_origin: "http://127.0.0.1:9/v1".to_owned(),
            gateway_api_key: None,
            responses: Arc::new(Mutex::new(HashMap::new())),
            response_locks: Arc::new(Mutex::new(HashMap::new())),
            momo_api: test_momo_api("http://127.0.0.1:9/v1"),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
            response_cancellations: Arc::new(Mutex::new(HashSet::new())),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
            memory_distill_every: 12,
            nsg_govern_every: 12,
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
                        json!({"input_path": external_path, "format": "ccv2_json"}).to_string(),
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
                    .uri("/v1/characters")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
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
                                "scope_id": DEFAULT_SCOPE_ID,
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
                    .uri(format!("/v1/conversations/{conversation_id}/messages"))
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
                    .uri("/v1/characters")
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
    async fn responses_forwards_upstream_sse_deltas_before_completion() {
        let _test_guard = TEST_LOCK.lock().await;
        let initialized_dir = initialize_test_core().await;
        simple::stage_character_json(
            DEFAULT_SCOPE_ID.to_owned(),
            "stream-contract".to_owned(),
            "MO".to_owned(),
            "stream contract character".to_owned(),
            "Be concise.".to_owned(),
            String::new(),
        )
        .await
        .expect("stage character");

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
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
            gateway_origin: format!("http://{address}/v1"),
            gateway_api_key: None,
            responses: Arc::new(Mutex::new(HashMap::new())),
            response_locks: Arc::new(Mutex::new(HashMap::new())),
            momo_api: test_momo_api(format!("http://{address}/v1")),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
            response_cancellations: Arc::new(Mutex::new(HashSet::new())),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
            memory_distill_every: 12,
            nsg_govern_every: 12,
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
                                "schema": "momo.responses/0.5",
                                "request_id": format!("stream-{}", simple::new_request_id()),
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
        simple::stage_character_json(
            DEFAULT_SCOPE_ID.to_owned(),
            "contract".to_owned(),
            "MO".to_owned(),
            "contract character".to_owned(),
            "Be concise.".to_owned(),
            "The user is testing the contract.".to_owned(),
        )
        .await
        .expect("stage character");

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
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
            gateway_origin: format!("http://{address}/v1"),
            gateway_api_key: None,
            responses: Arc::new(Mutex::new(HashMap::new())),
            response_locks: Arc::new(Mutex::new(HashMap::new())),
            momo_api: test_momo_api(format!("http://{address}/v1")),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
            response_cancellations: Arc::new(Mutex::new(HashSet::new())),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
            memory_distill_every: 12,
            nsg_govern_every: 12,
        });
        let request_body = include_str!("../../../contracts/0.5/response_request.json");
        let invoke = || {
            Request::builder()
                .method(Method::POST)
                .uri("/v1/momo/responses")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body))
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
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
            gateway_origin: format!("http://{address}/v1"),
            gateway_api_key: None,
            responses: Arc::new(Mutex::new(HashMap::new())),
            response_locks: Arc::new(Mutex::new(HashMap::new())),
            momo_api: test_momo_api(format!("http://{address}/v1")),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
            response_cancellations: Arc::new(Mutex::new(HashSet::new())),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
            memory_distill_every: 12,
            nsg_govern_every: 12,
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
                        "Hello from the cross-repository contract.",
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
            &simple::local_messages_json(conversation_id.to_owned())
                .await
                .expect("stored messages"),
        )
        .expect("messages JSON");
        assert_eq!(messages.as_array().map(Vec::len), Some(2));
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
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
            gateway_origin: format!("http://{address}/v1"),
            gateway_api_key: None,
            responses: Arc::new(Mutex::new(HashMap::new())),
            response_locks: Arc::new(Mutex::new(HashMap::new())),
            momo_api: test_momo_api(format!("http://{address}/v1")),
            maintenance_locks: Arc::new(Mutex::new(HashMap::new())),
            response_cancellations: Arc::new(Mutex::new(HashSet::new())),
            response_concurrency: Arc::new(Semaphore::new(8)),
            response_timeout: std::time::Duration::from_secs(120),
            metrics: Arc::new(Mutex::new(HashMap::new())),
            memory_distill_every: 1,
            nsg_govern_every: 1,
        };
        run_background_maintenance(state.clone(), scope_id.clone(), "memory", 1)
            .await
            .expect("memory maintenance");
        run_background_maintenance(state, scope_id.clone(), "semantic_graph", 1)
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
