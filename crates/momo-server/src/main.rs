use std::{convert::Infallible, env, net::SocketAddr};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use momo_core::{
    api::simple,
    momo_domain::{CharacterCard, Conversation, Message},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tower_http::trace::TraceLayer;

const DEFAULT_BIND: &str = "127.0.0.1:8765";
const DEFAULT_DATA_DIR: &str = ".momo-data";
const DEFAULT_SCOPE_ID: &str = "00000000-0000-4000-8000-000000000001";
const ALLOW_REMOTE_ENV: &str = "MOMO_SERVER_ALLOW_REMOTE";

#[derive(Clone)]
struct AppState {
    data_dir: String,
    scope_id: String,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportExternalCharacterRequest {
    output_path: String,
    format: String,
}

#[derive(Deserialize)]
struct ChatRequest {
    base_url: String,
    api_key: Option<String>,
    model: String,
    messages: Vec<Value>,
    #[serde(default = "default_temperature")]
    temperature: f32,
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
struct RuntimeConfigExportRequest {
    output_path: String,
    #[serde(default)]
    settings: Value,
}

#[derive(Deserialize)]
struct RuntimeConfigImportRequest {
    input_path: String,
}

#[derive(Deserialize)]
struct MocExportRequest {
    output_path: String,
    #[serde(default)]
    settings: Value,
    #[serde(default)]
    include_config: bool,
    #[serde(default)]
    include_characters: bool,
    #[serde(default)]
    include_conversations: bool,
    #[serde(default)]
    include_memory: bool,
    #[serde(default)]
    include_semantic_graph: bool,
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct CharacterMocExportRequest {
    output_path: String,
    character_id: String,
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct MocImportRequest {
    input_path: String,
    #[serde(default = "default_conflict_mode")]
    conflict_mode: String,
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct MocEncryptedQuery {
    input_path: String,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
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
    let state = AppState {
        data_dir: initialized_dir,
        scope_id,
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
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"))
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
        .route("/chat/complete", post(chat_complete))
        .route("/chat/stream", post(chat_stream))
        .route("/chat/cancel/:request_id", post(cancel_chat))
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
        .route("/runtime-config/export", post(export_runtime_config))
        .route("/runtime-config/import", post(import_runtime_config))
        .route("/moc/export", post(export_moc))
        .route("/moc/export-character", post(export_character_moc))
        .route("/moc/import", post(import_moc))
        .route("/moc/encrypted", get(moc_is_encrypted))
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "momo-server",
        core_version: simple::core_version(),
        data_dir: state.data_dir,
    })
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

async fn export_runtime_config(
    Json(request): Json<RuntimeConfigExportRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    simple::export_runtime_config_json(request.output_path, to_json_string(&request.settings)?)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn import_runtime_config(
    Json(request): Json<RuntimeConfigImportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(simple::import_runtime_config_json(request.input_path).await)
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
            "include_config": request.include_config,
            "include_characters": request.include_characters,
            "include_conversations": request.include_conversations,
            "include_memory": request.include_memory,
            "include_semantic_graph": request.include_semantic_graph,
            "passphrase": request.passphrase,
        }))?)
        .await,
    )
}

async fn export_character_moc(
    State(state): State<AppState>,
    Json(request): Json<CharacterMocExportRequest>,
) -> Result<Json<Value>, ApiError> {
    json_result(
        simple::export_character_moc_json(
            request.output_path,
            state.scope_id,
            request.character_id,
            request.passphrase,
        )
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
            request.passphrase,
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

fn json_result(result: Result<String, String>) -> Result<Json<Value>, ApiError> {
    let value = result.map_err(ApiError::internal)?;
    serde_json::from_str(&value)
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
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

fn default_temperature() -> f32 {
    0.7
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

fn default_conflict_mode() -> String {
    "rename".to_owned()
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
    use tower::ServiceExt;

    #[test]
    fn remote_bind_requires_explicit_opt_in() {
        let loopback = "127.0.0.1:8765".parse().expect("loopback address");
        let remote = "0.0.0.0:8765".parse().expect("remote address");
        assert!(ensure_bind_allowed(loopback, false).is_ok());
        assert!(ensure_bind_allowed(remote, false).is_err());
        assert!(ensure_bind_allowed(remote, true).is_ok());
    }

    #[tokio::test]
    async fn http_core_contract_and_character_round_trip() {
        let data_dir = tempfile::tempdir().expect("temporary data directory");
        let initialized_dir =
            simple::initialize_core(data_dir.path().to_string_lossy().into_owned())
                .await
                .expect("initialize core");
        let app = build_app(AppState {
            data_dir: initialized_dir,
            scope_id: DEFAULT_SCOPE_ID.to_owned(),
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

        let external_path = data_dir.path().join("external-card.json");
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
                    .body(Body::from(json!({"input_path": external_path}).to_string()))
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
        let exported_path = data_dir.path().join("exported-card.json");
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
        assert_eq!(characters.as_array().map(Vec::len), Some(2));
    }
}
