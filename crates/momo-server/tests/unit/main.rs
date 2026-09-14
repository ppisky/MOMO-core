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

async fn read_test_http_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = socket.read(&mut buffer).await.expect("read test request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8(request).expect("UTF-8 test request")
}

async fn write_test_json_response(socket: &mut tokio::net::TcpStream, body: Value) {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket
        .write_all(response.as_bytes())
        .await
        .expect("write test response");
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
                        "spaces": [
                            {"space_id": "00000000-0000-4000-8000-000000000011", "label": "personal", "weight": 60, "memory": true, "semantic_graph": true},
                            {"space_id": "00000000-0000-4000-8000-000000000012", "label": "channel", "weight": 40, "memory": true, "semantic_graph": true}
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
        .filter_map(|item| item["memory_space"]["label"].as_str())
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
                        "space_id": "00000000-0000-4000-8000-000000000011"
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
                    json!({"owner_space_id": TEST_SCOPE_ID, "input_path": external_path, "format": "ccv2_json"}).to_string(),
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
                        "owner_space_id": TEST_SCOPE_ID,
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
                .uri(format!("/v1/characters?space_id={TEST_SCOPE_ID}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "owner_space_id": TEST_SCOPE_ID,
                        "name": "HTTP test character",
                        "author_name": "momo-server test",
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

    let ddm_profile = format!(
        "schema: momo.ddm/1\ncharacter_id: {character_id}\nrevision: 1\nprofile: logit_additive\ndispositions:\n  - id: attentive\n    base_activation: 0.7\n    expression:\n      latent: Observe.\n      salient: Respond carefully.\n      dominant: Prioritize the immediate concern.\n"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/characters/{character_id}/ddm-profile"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "owner_space_id": TEST_SCOPE_ID,
                        "profile_yaml": ddm_profile,
                    })
                    .to_string(),
                ))
                .expect("DDM update request"),
        )
        .await
        .expect("DDM update response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/characters/{character_id}/ddm-profile?space_id={TEST_SCOPE_ID}"
                ))
                .body(Body::empty())
                .expect("DDM get request"),
        )
        .await
        .expect("DDM get response");
    assert_eq!(response.status(), StatusCode::OK);
    let stored_profile: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("DDM profile body"),
    )
    .expect("DDM profile JSON");
    assert!(
        stored_profile["profile_yaml"]
            .as_str()
            .is_some_and(|profile| profile.contains("id: attentive"))
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!(
                    "/v1/characters/{character_id}/ddm-profile?space_id={TEST_SCOPE_ID}"
                ))
                .body(Body::empty())
                .expect("DDM delete request"),
        )
        .await
        .expect("DDM delete response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/characters/{character_id}/ddm-profile?space_id={TEST_SCOPE_ID}"
                ))
                .body(Body::empty())
                .expect("DDM get-after-delete request"),
        )
        .await
        .expect("DDM get-after-delete response");
    let deleted_profile: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("DDM deleted profile body"),
    )
    .expect("DDM deleted profile JSON");
    assert!(deleted_profile["profile_yaml"].is_null());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/conversations")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "space_id": TEST_SCOPE_ID,
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
                        "space_id": TEST_SCOPE_ID,
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
                        "space_id": TEST_SCOPE_ID,
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
                        "space_id": TEST_SCOPE_ID,
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
                        "spaces": [{
                            "space_id": TEST_SCOPE_ID,
                            "label": "default",
                            "weight": 100,
                            "memory": false,
                            "semantic_graph": true
                        }],
                        "query": "unrelated query text",
                        "max_tokens": 1024,
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
                    "/v1/conversations/{conversation_id}/messages?space_id={TEST_SCOPE_ID}"
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
                        "space_id": "00000000-0000-4000-8000-000000000011",
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
                        "space_id": "00000000-0000-4000-8000-000000000011",
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
                .uri(format!("/v1/characters?space_id={TEST_SCOPE_ID}"))
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
                        "output_path": output,
                        "plan": {
                            "include_config": false,
                            "characters": [],
                            "conversations": [],
                            "memory": [],
                            "semantic_graph": []
                        },
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
                        "input_path": output,
                        "plan": {
                            "apply_config": false,
                            "space_map": {},
                            "conflict_mode": "replace"
                        },
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
        std::fs::read_to_string(claims.join("weather/module.json")).expect("claimed host payload"),
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
                    "/v1/conversations/{conversation_id}/messages?space_id={OWNER_SCOPE}"
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
                    "/v1/conversations/{conversation_id}/messages?space_id={OTHER_SCOPE}"
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
                        "space_id": OTHER_SCOPE,
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
    foreign_character["owner_space_id"] = json!(OTHER_SCOPE);
    let mut foreign_conversation = conversation.clone();
    foreign_conversation["space_id"] = json!(OTHER_SCOPE);
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
                        "owner_space_id": OTHER_SCOPE,
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
                    "/v1/characters/{character_id}?space_id={OTHER_SCOPE}"
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
                    "/v1/conversations/{conversation_id}?space_id={OTHER_SCOPE}"
                ))
                .body(Body::empty())
                .expect("cross-scope conversation delete"),
        ),
        (
            "message update",
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/messages/{message_id}?space_id={OTHER_SCOPE}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(message.to_string()))
                .expect("cross-scope message update"),
        ),
        (
            "message delete",
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/messages/{message_id}?space_id={OTHER_SCOPE}"))
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
                            "schema": "momo.responses/1.0",
                            "request_id": "cross-scope-response",
                            "personal_space_id": OTHER_SCOPE,
                            "conversation_space_id": OTHER_SCOPE,
                            "character_id": character_id,
                            "conversation_id": conversation_id,
                            "memory_sources": [],
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
            String::from_utf8_lossy(&request[..read]).starts_with("GET /v1/models/conversation ")
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
        let first =
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n";
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
                            "personal_space_id": TEST_SCOPE_ID,
                            "conversation_space_id": TEST_SCOPE_ID,
                            "character_id": character_id,
                            "memory_sources": [],
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
    let first_chunk = tokio::time::timeout(std::time::Duration::from_millis(100), chunks.next())
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
            "Be concise.".to_owned(),
            "The user is testing the contract.".to_owned(),
        )
        .await
        .expect("stage character"),
    )
    .expect("character JSON");
    let character_id = character["id"].as_str().expect("character ID").to_owned();
    simple::upsert_character_ddm_profile_json(
        TEST_SCOPE_ID.to_owned(),
        character_id.clone(),
        format!(
            "schema: momo.ddm/1\ncharacter_id: {character_id}\nrevision: 1\nprofile: logit_additive\ndispositions:\n  - id: attentive\n    base_activation: 0.7\n    modulation:\n      context:\n        - id: user_turn\n          signal: request.event_type\n          when: user_message\n          delta: 0.2\n    expression:\n      latent: Observe.\n      salient: Respond carefully.\n      dominant: Prioritize the immediate concern.\n"
        ),
    )
    .await
    .expect("DDM profile");

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
                    assert!(request.contains("Respond carefully."));
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

    let mut config = MomoConfig::default();
    config.mo_state.ddm.enabled = true;
    let app = build_app(AppState {
        data_dir: initialized_dir,
        momo_api: Arc::new(MomoApiService::new(
            format!("http://{address}/v1"),
            None,
            reqwest::Client::new(),
            Arc::new(config.clone()),
        )),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    });
    let mut request_body: Value = serde_json::from_str(include_str!(
        "../../../../contracts/1.0/response_request.json"
    ))
    .expect("1.0 response fixture");
    request_body["momo"]["personal_space_id"] = json!(TEST_SCOPE_ID);
    request_body["momo"]["conversation_space_id"] = json!(TEST_SCOPE_ID);
    request_body["momo"]["memory_sources"] = json!([
        {
            "space_id": TEST_SCOPE_ID,
            "label": "personal",
            "weight": 75,
            "memory": true,
            "semantic_graph": true
        },
        {
            "space_id": "01900000-0000-7000-8000-000000000202",
            "label": "shared",
            "weight": 25,
            "memory": true,
            "semantic_graph": true
        }
    ]);
    request_body["momo"]["memory_write_space_id"] = json!(TEST_SCOPE_ID);
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
    assert_eq!(
        first["momo"]["state_audit"]["manager"]["profile"],
        "closed_autonomous"
    );
    assert_eq!(
        first["momo"]["state_audit"]["runtime_snapshot"]["snapshot_revision"],
        1
    );
    assert_eq!(
        first["momo"]["state_audit"]["runtime_snapshot"]["source_versions"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        first["momo"]["state_audit"]["ddm"]["effective_dispositions"][0]["id"],
        "attentive"
    );
    assert_eq!(
        first["momo"]["state_audit"]["ddm"]["effective_dispositions"][0]["matched_rule_ids"],
        json!(["user_turn"])
    );
    let conversation_id = first["momo"]["conversation_id"]
        .as_str()
        .expect("conversation ID")
        .to_owned();
    let ddm_state = simple::ddm_projection_state_json(
        TEST_SCOPE_ID.to_owned(),
        conversation_id,
        character_id.clone(),
    )
    .await
    .expect("DDM state query")
    .expect("persisted DDM state");
    let ddm_state: Value = serde_json::from_str(&ddm_state).expect("DDM state JSON");
    assert_eq!(ddm_state["bands"]["attentive"], "salient");
    let runtime = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/mo-state/runtime?space_id={TEST_SCOPE_ID}"))
                .body(Body::empty())
                .expect("runtime request"),
        )
        .await
        .expect("runtime response");
    assert_eq!(runtime.status(), StatusCode::OK);
    let runtime: Value = serde_json::from_slice(
        &to_bytes(runtime.into_body(), 1024 * 1024)
            .await
            .expect("runtime body"),
    )
    .expect("runtime JSON");
    assert_eq!(runtime["profile"], "closed_autonomous");
    assert_eq!(runtime["snapshot_revision"], 1);
    assert_eq!(
        runtime["current_snapshot"]["source_versions"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );

    drop(app);
    let restarted_app = build_app(AppState {
        data_dir: TEST_DATA_DIR.path().to_string_lossy().into_owned(),
        momo_api: Arc::new(MomoApiService::new(
            format!("http://{address}/v1"),
            None,
            reqwest::Client::new(),
            Arc::new(config),
        )),
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
            String::from_utf8_lossy(&request[..read]).starts_with("GET /v1/models/conversation ")
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
        assert!(request_text.contains("You perform the character defined below."));
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
                            "schema": "momo.responses/1.0",
                            "request_id": "direct-multimodal-contract-1",
                            "character_id": character_id,
                            "personal_space_id": TEST_SCOPE_ID,
                            "conversation_space_id": TEST_SCOPE_ID,
                            "memory_sources": [],
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("maintenance gateway");
    let address = listener.local_addr().expect("maintenance address");
    tokio::spawn(async move {
        let mut generated = std::collections::HashSet::new();
        let mut memory_generations = 0;
        for _ in 0..5 {
            let (mut socket, _) = listener.accept().await.expect("accept maintenance");
            let request = read_test_http_request(&mut socket).await;
            if request.starts_with("GET /v1/models/") {
                assert!(
                    request.starts_with("GET /v1/models/memory_distillation ")
                        || request.starts_with("GET /v1/models/semantic_graph_governance ")
                );
                write_test_json_response(
                    &mut socket,
                    json!({
                        "momo": {
                            "context_window": 8192,
                            "max_output_tokens": 1024,
                            "modalities": ["text"]
                        }
                    }),
                )
                .await;
                continue;
            }
            let request_body = request
                .split_once("\r\n\r\n")
                .expect("maintenance HTTP body")
                .1;
            let request_json: Value =
                serde_json::from_str(request_body).expect("maintenance request JSON");
            let expected_model = request_json["model"]
                .as_str()
                .expect("maintenance model alias");
            assert!(matches!(
                expected_model,
                "memory_distillation" | "semantic_graph_governance"
            ));
            generated.insert(expected_model.to_owned());
            if expected_model == "memory_distillation" {
                memory_generations += 1;
            }
            assert_eq!(request_json["max_tokens"], 1024);
            let maintenance_request: Value = serde_json::from_str(
                request_json["messages"][1]["content"]
                    .as_str()
                    .expect("structured maintenance input"),
            )
            .expect("maintenance input JSON");
            let maintenance_input = maintenance_request
                .get("maintenance_input")
                .unwrap_or(&maintenance_request);
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
            write_test_json_response(
                &mut socket,
                json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": "patches: []"},
                        "finish_reason": "stop"
                    }]
                }),
            )
            .await;
        }
        assert_eq!(
            generated,
            std::collections::HashSet::from([
                "memory_distillation".to_owned(),
                "semantic_graph_governance".to_owned(),
            ])
        );
        assert_eq!(memory_generations, 2);
    });
    let state = AppState {
        data_dir: initialized_dir,
        momo_api: test_momo_api(format!("http://{address}/v1")),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = api_routes().with_state(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/momo/maintenance/turns")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "request_id": "maintenance-contract-1",
                        "space_id": scope_id,
                        "user_content": "The moon gate requires a silver key.",
                        "assistant_content": "Understood.",
                        "memory_enabled": true,
                        "nsg_enabled": true
                    })
                    .to_string(),
                ))
                .expect("recorded maintenance turn request"),
        )
        .await
        .expect("recorded maintenance turn response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/momo/maintenance/turns")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "request_id": "maintenance-contract-1",
                        "space_id": scope_id,
                        "user_content": "Conflicting replay content.",
                        "assistant_content": "Understood.",
                        "memory_enabled": true,
                        "nsg_enabled": true
                    })
                    .to_string(),
                ))
                .expect("conflicting maintenance turn request"),
        )
        .await
        .expect("conflicting maintenance turn response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    // The HTTP barrier flushes a batch smaller than the normal automatic
    // threshold and is safe to repeat without issuing model calls again.
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/momo/maintenance/drain")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"space_id": scope_id}).to_string()))
                    .expect("drain request"),
            )
            .await
            .expect("drain response");
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 4096)
                .await
                .expect("drain body"),
        )
        .expect("drain JSON");
        assert_eq!(body["completed"], true);
    }
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

#[tokio::test]
async fn hybrid_retrieval_keeps_room_for_a_complete_nsg_node() {
    let _test_guard = TEST_LOCK.lock().await;
    initialize_test_core().await;
    let scope_id = uuid::Uuid::now_v7().to_string();
    let padding = "x".repeat(500);
    simple::apply_memory_patch_json(
        scope_id.clone(),
        format!(
            r#"
patches:
  - target_file: world/astrolabe_context.md
    operations:
      - type: create
        frontmatter:
          id: world_astrolabe_context
          type: world
          importance: 0.8
          weight: 0.8
          decay_at: 2000000000
          relations: {{}}
          tags: [astrolabe]
          aliases: [Mara]
          status: active
        content: |-
          # Mara Astrolabe Context

          {padding}
"#
        ),
    )
    .await
    .expect("create large DMW document");
    simple::apply_nsg_patch_json(
        scope_id.clone(),
        r#"
patches:
  - target_file: "lore/mara_navigation_storage.nsg"
    operations:
      - type: "create_node"
        metadata:
          id: "lore_mara_navigation_storage"
          type: "lore"
          importance: 0.8
          mode: "canon"
          status: "active"
          zone: "auto"
        anchors: "Glass-Archive-52636, Mara navigation instruments, winter storage"
        condition: "The item is a navigation instrument owned by Mara."
        trigger: "Mara stores the instrument for winter."
        consequence: "The instrument is placed in Glass-Archive-52636."
        constraint: "This rule does not confirm that any specific instrument is currently stored there."
"#
        .to_owned(),
        true,
    )
    .await
    .expect("create NSG node");

    let retrieved: Value = serde_json::from_str(
        &simple::retrieve_scoped_memory_json(
            json!({
                "spaces": [{
                    "space_id": scope_id,
                    "label": "test",
                    "weight": 100,
                    "memory": true,
                    "semantic_graph": true
                }],
                "query": "Where should I look for Mara's silver astrolabe now?",
                "max_tokens": 961
            })
            .to_string(),
        )
        .await
        .expect("hybrid retrieval"),
    )
    .expect("retrieval JSON");
    assert!(
        retrieved
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["id"]
                == "lore_mara_navigation_storage"
                && item["estimated_tokens"]
                    .as_u64()
                    .is_some_and(|tokens| tokens <= 385))),
        "hybrid retrieval omitted NSG node: {retrieved}"
    );
}

#[tokio::test]
async fn maintenance_repairs_invalid_memory_patch_before_staging() {
    let _test_guard = TEST_LOCK.lock().await;
    let initialized_dir = initialize_test_core().await;
    let scope_id = uuid::Uuid::now_v7().to_string();
    simple::append_maintenance_turn_json(
        json!({
            "request_id": uuid::Uuid::now_v7().to_string(),
            "scope_id": scope_id,
            "user_content": "Remember that I prefer tea.",
            "assistant_content": "Understood."
        })
        .to_string(),
        true,
        false,
    )
    .await
    .expect("pending memory turn");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("maintenance gateway");
    let address = listener.local_addr().expect("maintenance address");
    let gateway = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept model discovery");
        let request = read_test_http_request(&mut socket).await;
        assert!(request.starts_with("GET /v1/models/memory_distillation "));
        write_test_json_response(
            &mut socket,
            json!({
                "momo": {
                    "context_window": 8192,
                    "max_output_tokens": 2048,
                    "modalities": ["text"]
                }
            }),
        )
        .await;

        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("accept maintenance");
            let request = read_test_http_request(&mut socket).await;
            let request_body = request
                .split_once("\r\n\r\n")
                .expect("maintenance HTTP body")
                .1;
            let request_json: Value =
                serde_json::from_str(request_body).expect("maintenance request JSON");
            assert_eq!(request_json["max_tokens"], 2048);
            let user_content: Value = serde_json::from_str(
                request_json["messages"][1]["content"]
                    .as_str()
                    .expect("maintenance user content"),
            )
            .expect("structured maintenance input");
            if attempt == 0 {
                assert!(user_content.get("previous_output_error").is_none());
            } else {
                assert!(
                    user_content["previous_output_error"]
                        .as_str()
                        .is_some_and(|error| error.contains("decay_at"))
                );
                assert!(
                    user_content["required_correction"]
                        .as_str()
                        .is_some_and(
                            |instruction| instruction.contains("type must be exactly character")
                        )
                );
            }
            let patch = if attempt == 0 {
                "patches:\n  - target_file: characters/player.md\n    operations:\n      - type: create\n        frontmatter:\n          id: character_player\n          type: character\n          importance: 0.7\n          weight: 0.7\n          status: active\n        content: '# Player'"
            } else {
                "patches: []"
            };
            write_test_json_response(
                &mut socket,
                json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": patch},
                        "finish_reason": "stop"
                    }]
                }),
            )
            .await;
        }
    });
    let state = AppState {
        data_dir: initialized_dir,
        momo_api: test_momo_api(format!("http://{address}/v1")),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let result = drain_momo_maintenance(
        State(state),
        Json(SpaceRequest {
            space_id: scope_id.clone(),
        }),
    )
    .await;
    assert!(result.is_ok(), "repair invalid maintenance patch");
    gateway.await.expect("mock gateway task");
    let pending: Value = serde_json::from_str(
        &simple::pending_maintenance_turns_json(scope_id, "memory".to_owned(), 10)
            .await
            .expect("pending turns"),
    )
    .expect("pending JSON");
    assert_eq!(pending, json!([]));
}

#[tokio::test]
async fn maintenance_rechecks_an_empty_first_pass_before_acknowledging_turns() {
    let _test_guard = TEST_LOCK.lock().await;
    let initialized_dir = initialize_test_core().await;
    let scope_id = uuid::Uuid::now_v7().to_string();
    simple::append_maintenance_turn_json(
        json!({
            "request_id": uuid::Uuid::now_v7().to_string(),
            "scope_id": scope_id,
            "user_content": "Remember that the meeting is at Glass-Archive-f41e9.",
            "assistant_content": "Understood."
        })
        .to_string(),
        true,
        false,
    )
    .await
    .expect("pending memory turn");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("maintenance gateway");
    let address = listener.local_addr().expect("maintenance address");
    let gateway = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept model discovery");
        let request = read_test_http_request(&mut socket).await;
        assert!(request.starts_with("GET /v1/models/memory_distillation "));
        write_test_json_response(
            &mut socket,
            json!({
                "momo": {
                    "context_window": 8192,
                    "max_output_tokens": 2048,
                    "modalities": ["text"]
                }
            }),
        )
        .await;

        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("accept maintenance");
            let request = read_test_http_request(&mut socket).await;
            let request_body = request
                .split_once("\r\n\r\n")
                .expect("maintenance HTTP body")
                .1;
            let request_json: Value =
                serde_json::from_str(request_body).expect("maintenance request JSON");
            let user_content: Value = serde_json::from_str(
                request_json["messages"][1]["content"]
                    .as_str()
                    .expect("maintenance user content"),
            )
            .expect("structured maintenance input");
            if attempt == 0 {
                assert!(user_content.get("previous_output_error").is_none());
            } else {
                assert!(
                    user_content["previous_output_error"]
                        .as_str()
                        .is_some_and(|error| error.contains("empty patch"))
                );
            }
            let patch = if attempt == 0 {
                "patches: []"
            } else {
                "patches:\n  - target_file: events/meeting.md\n    operations:\n      - type: create\n        frontmatter:\n          id: event_meeting\n          type: event\n          importance: 0.8\n          weight: 0.8\n          decay_at: 1\n          relations: {}\n          tags: [meeting]\n          aliases: []\n          status: active\n        content: |-\n          # Meeting\n\n          ## Current commitment\n\n          - Meet at Glass-Archive-f41e9."
            };
            write_test_json_response(
                &mut socket,
                json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": patch},
                        "finish_reason": "stop"
                    }]
                }),
            )
            .await;
        }
    });
    let state = AppState {
        data_dir: initialized_dir,
        momo_api: test_momo_api(format!("http://{address}/v1")),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let result = drain_momo_maintenance(
        State(state),
        Json(SpaceRequest {
            space_id: scope_id.clone(),
        }),
    )
    .await;
    assert!(result.is_ok(), "retry empty memory patch");
    gateway.await.expect("mock gateway task");
    let retrieved: Value = serde_json::from_str(
        &simple::retrieve_memory_json(scope_id, "meeting".to_owned(), 4096)
            .await
            .expect("retrieve repaired memory"),
    )
    .expect("retrieval JSON");
    assert!(
        retrieved
            .as_array()
            .is_some_and(|items| { items.iter().any(|item| item["id"] == "event_meeting") })
    );
}

#[tokio::test]
async fn maintenance_retries_a_length_truncated_patch_before_staging() {
    let _test_guard = TEST_LOCK.lock().await;
    let initialized_dir = initialize_test_core().await;
    let scope_id = uuid::Uuid::now_v7().to_string();
    simple::append_maintenance_turn_json(
        json!({
            "request_id": uuid::Uuid::now_v7().to_string(),
            "scope_id": scope_id,
            "user_content": "Remember that the meeting is at Glass-Archive-f41e9.",
            "assistant_content": "Understood."
        })
        .to_string(),
        true,
        false,
    )
    .await
    .expect("pending memory turn");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("maintenance gateway");
    let address = listener.local_addr().expect("maintenance address");
    let gateway = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept model discovery");
        let request = read_test_http_request(&mut socket).await;
        assert!(request.starts_with("GET /v1/models/memory_distillation "));
        write_test_json_response(
            &mut socket,
            json!({
                "momo": {
                    "context_window": 8192,
                    "max_output_tokens": 2048,
                    "modalities": ["text"]
                }
            }),
        )
        .await;

        let patch = "patches:\n  - target_file: events/meeting.md\n    operations:\n      - type: create\n        frontmatter:\n          id: event_meeting\n          type: event\n          importance: 0.8\n          weight: 0.8\n          decay_at: 1\n          relations: {}\n          tags: [meeting]\n          aliases: []\n          status: active\n        content: |-\n          # Meeting\n\n          ## Current commitment\n\n          - Meet at Glass-Archive-f41e9.";
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("accept maintenance");
            let request = read_test_http_request(&mut socket).await;
            let request_body = request
                .split_once("\r\n\r\n")
                .expect("maintenance HTTP body")
                .1;
            let request_json: Value =
                serde_json::from_str(request_body).expect("maintenance request JSON");
            assert_eq!(request_json["max_tokens"], 2048);
            let user_content: Value = serde_json::from_str(
                request_json["messages"][1]["content"]
                    .as_str()
                    .expect("maintenance user content"),
            )
            .expect("structured maintenance input");
            if attempt == 0 {
                assert!(user_content.get("previous_output_error").is_none());
            } else {
                assert!(
                    user_content["previous_output_error"]
                        .as_str()
                        .is_some_and(|error| error.contains("finish_reason=length"))
                );
                assert!(
                    user_content["required_correction"]
                        .as_str()
                        .is_some_and(|instruction| instruction.contains("complete patch"))
                );
            }
            write_test_json_response(
                &mut socket,
                json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": patch},
                        "finish_reason": if attempt == 0 { "length" } else { "stop" }
                    }]
                }),
            )
            .await;
        }
    });
    let state = AppState {
        data_dir: initialized_dir,
        momo_api: test_momo_api(format!("http://{address}/v1")),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let result = drain_momo_maintenance(
        State(state),
        Json(SpaceRequest {
            space_id: scope_id.clone(),
        }),
    )
    .await;
    assert!(result.is_ok(), "retry truncated memory patch");
    gateway.await.expect("mock gateway task");
    let retrieved: Value = serde_json::from_str(
        &simple::retrieve_memory_json(scope_id, "meeting".to_owned(), 4096)
            .await
            .expect("retrieve repaired memory"),
    )
    .expect("retrieval JSON");
    assert!(
        retrieved
            .as_array()
            .is_some_and(|items| { items.iter().any(|item| item["id"] == "event_meeting") })
    );
}

#[tokio::test]
async fn maintenance_barrier_reports_upstream_failure_and_keeps_pending_work() {
    let _test_guard = TEST_LOCK.lock().await;
    let data_dir = initialize_test_core().await;
    let space_id = uuid::Uuid::now_v7().to_string();
    simple::append_maintenance_turn_json(
        json!({
            "request_id": uuid::Uuid::now_v7().to_string(), "scope_id": space_id,
            "user_content": "Remember this event", "assistant_content": "Acknowledged",
        })
        .to_string(),
        true,
        true,
    )
    .await
    .expect("pending turn");
    let state = AppState {
        data_dir,
        momo_api: test_momo_api("http://127.0.0.1:9/v1"),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(2),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let result = drain_momo_maintenance(
        State(state),
        Json(SpaceRequest {
            space_id: space_id.clone(),
        }),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("failed maintenance must not report completion"),
        Err(error) => error,
    };
    assert!(matches!(
        error.status,
        StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT
    ));
    let pending: Vec<Value> = serde_json::from_str(
        &simple::pending_maintenance_turns_json(space_id, "memory".to_owned(), 32)
            .await
            .expect("pending query"),
    )
    .expect("pending JSON");
    assert_eq!(pending.len(), 1);
}

#[tokio::test]
async fn maintenance_barrier_keeps_memory_progress_when_nsg_fails() {
    let _test_guard = TEST_LOCK.lock().await;
    let data_dir = initialize_test_core().await;
    let space_id = uuid::Uuid::now_v7().to_string();
    simple::append_maintenance_turn_json(
        json!({
            "request_id": uuid::Uuid::now_v7().to_string(),
            "scope_id": space_id,
            "user_content": "Remember this event",
            "assistant_content": "Acknowledged",
        })
        .to_string(),
        true,
        true,
    )
    .await
    .expect("pending turn");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("maintenance gateway");
    let address = listener.local_addr().expect("maintenance address");
    let gateway = tokio::spawn(async move {
        for expected in [
            "GET /v1/models/memory_distillation ",
            "POST /v1/chat/completions ",
            "POST /v1/chat/completions ",
            "GET /v1/models/semantic_graph_governance ",
            "POST /v1/chat/completions ",
        ] {
            let (mut socket, _) = listener.accept().await.expect("accept maintenance");
            let request = read_test_http_request(&mut socket).await;
            assert!(
                request.starts_with(expected),
                "unexpected request: {request}"
            );
            if expected.starts_with("GET") {
                write_test_json_response(
                    &mut socket,
                    json!({
                        "momo": {
                            "context_window": 8192,
                            "max_output_tokens": 1024,
                            "modalities": ["text"]
                        }
                    }),
                )
                .await;
            } else if request.contains("\"model\":\"memory_distillation\"") {
                write_test_json_response(
                    &mut socket,
                    json!({
                        "choices": [{
                            "message": {"role": "assistant", "content": "patches: []"},
                            "finish_reason": "stop"
                        }]
                    }),
                )
                .await;
            } else {
                let body = r#"{"error":{"message":"synthetic NSG failure"}}"#;
                let response = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write failed NSG response");
            }
        }
    });
    let state = AppState {
        data_dir,
        momo_api: test_momo_api(format!("http://{address}/v1")),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let result = drain_momo_maintenance(
        State(state),
        Json(SpaceRequest {
            space_id: space_id.clone(),
        }),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("failed NSG maintenance must not report completion"),
        Err(error) => error,
    };
    assert_eq!(error.status, StatusCode::BAD_GATEWAY);
    gateway.await.expect("mock gateway task");

    let memory_pending: Vec<Value> = serde_json::from_str(
        &simple::pending_maintenance_turns_json(space_id.clone(), "memory".to_owned(), 32)
            .await
            .expect("pending memory query"),
    )
    .expect("pending memory JSON");
    let nsg_pending: Vec<Value> = serde_json::from_str(
        &simple::pending_maintenance_turns_json(space_id, "semantic_graph".to_owned(), 32)
            .await
            .expect("pending NSG query"),
    )
    .expect("pending NSG JSON");
    assert!(memory_pending.is_empty());
    assert_eq!(nsg_pending.len(), 1);
}

#[tokio::test]
async fn structured_controls_switch_clear_and_delete_without_a_model() {
    let _test_guard = TEST_LOCK.lock().await;
    let initialized_dir = initialize_test_core().await;
    let test_space_id = uuid::Uuid::now_v7().to_string();
    let first: Value = serde_json::from_str(
        &simple::stage_character_json(
            test_space_id.clone(),
            "control".to_owned(),
            "First".to_owned(),
            "First prompt".to_owned(),
            String::new(),
        )
        .await
        .expect("first character"),
    )
    .expect("first character JSON");
    let second: Value = serde_json::from_str(
        &simple::stage_character_json(
            test_space_id.clone(),
            "control".to_owned(),
            "Second".to_owned(),
            "Second prompt".to_owned(),
            String::new(),
        )
        .await
        .expect("second character"),
    )
    .expect("second character JSON");
    let conversation: Value = serde_json::from_str(
        &simple::stage_conversation_json(
            None,
            test_space_id.clone(),
            "controlled".to_owned(),
            first["id"].as_str().expect("first id").to_owned(),
        )
        .await
        .expect("conversation"),
    )
    .expect("conversation JSON");
    let conversation_id = conversation["id"].as_str().expect("conversation id");
    let state = AppState {
        data_dir: initialized_dir.clone(),
        momo_api: test_momo_api("http://127.0.0.1:9/v1".to_owned()),
        response_concurrency: Arc::new(Semaphore::new(1)),
        response_timeout: std::time::Duration::from_secs(1),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = build_app(state);
    let invalid_schema = Request::builder()
        .method(Method::POST)
        .uri("/v1/momo/control")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "schema": "momo.control/2.0",
                "request_id": "invalid-schema",
                "actor_space_id": test_space_id,
                "action": {
                    "type": "clear_memory",
                    "target_space_id": test_space_id,
                    "memory": true,
                    "semantic_graph": false,
                },
            })
            .to_string(),
        ))
        .expect("invalid control request");
    assert_eq!(
        app.clone()
            .oneshot(invalid_schema)
            .await
            .expect("invalid schema response")
            .status(),
        StatusCode::BAD_REQUEST
    );
    let unknown_field = Request::builder()
        .method(Method::POST)
        .uri("/v1/momo/control")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "schema": "momo.control/1.0",
                "request_id": "unknown-field",
                "actor_space_id": test_space_id,
                "unexpected": true,
                "action": {
                    "type": "clear_memory",
                    "target_space_id": test_space_id,
                    "memory": true,
                    "semantic_graph": false,
                },
            })
            .to_string(),
        ))
        .expect("unknown-field control request");
    let unknown_field_response = app
        .clone()
        .oneshot(unknown_field)
        .await
        .expect("unknown-field response");
    assert_eq!(
        unknown_field_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let unknown_field_body: Value = serde_json::from_slice(
        &to_bytes(unknown_field_response.into_body(), usize::MAX)
            .await
            .expect("unknown-field response body"),
    )
    .expect("unknown-field response JSON");
    assert_eq!(unknown_field_body["error"]["code"], "invalid_json");
    let control = |request_id: &str, action: Value| {
        Request::builder()
            .method(Method::POST)
            .uri("/v1/momo/control")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "schema": "momo.control/1.0",
                    "request_id": request_id,
                    "actor_space_id": test_space_id,
                    "action": action,
                })
                .to_string(),
            ))
            .expect("control request")
    };

    let response = app
        .clone()
        .oneshot(control(
            "switch-character",
            json!({
                "type": "switch_character",
                "conversation_space_id": test_space_id,
                "conversation_id": conversation_id,
                "character_id": second["id"],
            }),
        ))
        .await
        .expect("switch response");
    assert_eq!(response.status(), StatusCode::OK);
    let first_switch_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("first switch body");
    let replay = app
        .clone()
        .oneshot(control(
            "switch-character",
            json!({
                "type": "switch_character",
                "conversation_space_id": test_space_id,
                "conversation_id": conversation_id,
                "character_id": second["id"],
            }),
        ))
        .await
        .expect("switch replay");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(replay.into_body(), usize::MAX)
            .await
            .expect("replayed switch body"),
        first_switch_body
    );
    let conflict = app
        .clone()
        .oneshot(control(
            "switch-character",
            json!({
                "type": "switch_character",
                "conversation_space_id": test_space_id,
                "conversation_id": conversation_id,
                "character_id": first["id"],
            }),
        ))
        .await
        .expect("switch conflict");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conversations: Value = serde_json::from_str(
        &simple::local_conversations_json(test_space_id.clone())
            .await
            .expect("conversations"),
    )
    .expect("conversations JSON");
    assert_eq!(conversations[0]["character_id"], second["id"]);

    let memory_root = PathBuf::from(&initialized_dir)
        .join("spaces")
        .join(&test_space_id)
        .join("memory");
    std::fs::create_dir_all(memory_root.join("current")).expect("DMW directory");
    std::fs::create_dir_all(memory_root.join("lore")).expect("NSG directory");
    std::fs::write(memory_root.join("current/scene.md"), "scene").expect("DMW file");
    std::fs::write(memory_root.join("lore/world.nsg"), "world").expect("NSG file");
    let response = app
        .clone()
        .oneshot(control(
            "clear-dmw",
            json!({
                "type": "clear_memory",
                "target_space_id": test_space_id,
                "memory": true,
                "semantic_graph": false,
            }),
        ))
        .await
        .expect("clear response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_ne!(
        std::fs::read_to_string(memory_root.join("current/scene.md"))
            .expect("recreated empty scene"),
        "scene"
    );
    assert!(memory_root.join("lore/world.nsg").exists());

    let response = app
        .clone()
        .oneshot(control(
            "clear-nsg",
            json!({
                "type": "clear_memory",
                "target_space_id": test_space_id,
                "memory": false,
                "semantic_graph": true,
            }),
        ))
        .await
        .expect("clear NSG response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!memory_root.join("lore/world.nsg").exists());

    let response = app
        .oneshot(control(
            "delete-conversation",
            json!({
                "type": "delete_conversation",
                "conversation_space_id": test_space_id,
                "conversation_id": conversation_id,
            }),
        ))
        .await
        .expect("delete response");
    assert_eq!(response.status(), StatusCode::OK);
    let conversations: Value = serde_json::from_str(
        &simple::local_conversations_json(test_space_id)
            .await
            .expect("conversations after delete"),
    )
    .expect("conversations JSON");
    assert_eq!(conversations.as_array().map(Vec::len), Some(0));
}
