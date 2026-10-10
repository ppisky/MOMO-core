use super::*;
use momo_core::momo_domain;

#[tokio::test]
async fn conversation_archive_routes_are_scoped_idempotent_and_preserve_history() {
    let (_directory, runtime, app) = isolated_app("http://127.0.0.1:9/v1").await;
    let space = uuid::Uuid::now_v7();
    let now = chrono::Utc::now();
    // Missing archived_at remains compatible with existing serialized records.
    let conversation: momo_domain::Conversation = serde_json::from_value(json!({
        "id":uuid::Uuid::now_v7(),"space_id":space,"character_id":null,
        "title":"Archived story","created_at":now,"updated_at":now,
    }))
    .unwrap();
    runtime
        .core()
        .store()
        .save_conversation(&conversation)
        .await
        .unwrap();
    let message = momo_domain::Message {
        id: uuid::Uuid::now_v7(),
        conversation_id: conversation.id,
        role: momo_domain::MessageRole::Assistant,
        content: "Opening evidence".into(),
        created_at: now,
    };
    runtime
        .core()
        .store()
        .append_message(&message)
        .await
        .unwrap();
    let path = format!("/v1/conversations/{}/archive", conversation.id);
    let request = json!({"space_id":space,"archived":true});
    let (status, _, archived) = send_json(&app, Method::POST, &path, request.clone(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(archived["archived_at"].is_string());
    assert_eq!(archived["updated_at"], json!(now));
    assert_eq!(
        send_json(&app, Method::POST, &path, request, None).await.2,
        archived
    );
    let (status, _, _) = send_json(
        &app,
        Method::POST,
        &path,
        json!({"space_id":uuid::Uuid::now_v7(),"archived":false}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for (suffix, expected) in [("", 1), ("&archived=true", 1), ("&archived=false", 0)] {
        let (status, _, items) = send_json(
            &app,
            Method::GET,
            &format!("/v1/conversations?space_id={space}{suffix}"),
            json!({}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(items.as_array().unwrap().len(), expected);
    }
    let mut stale = serde_json::to_value(&conversation).unwrap();
    stale["title"] = json!("Renamed while archived");
    let (status, _, renamed) = send_json(
        &app,
        Method::PUT,
        &format!("/v1/conversations/{}", conversation.id),
        stale,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["archived_at"], archived["archived_at"]);
    let (status, _, restored) = send_json(
        &app,
        Method::POST,
        &path,
        json!({"space_id":space,"archived":false}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restored["id"], archived["id"]);
    assert!(restored["archived_at"].is_null());
    assert_eq!(restored["title"], renamed["title"]);
    assert_eq!(
        runtime
            .core()
            .store()
            .list_messages(conversation.id)
            .await
            .unwrap(),
        vec![message]
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"space_id":space,"archived":true,"title":"Injected"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        runtime
            .core()
            .store()
            .conversation_for_scope(space, conversation.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at
            .is_none()
    );
}

async fn isolated_app(origin: &str) -> (tempfile::TempDir, Arc<MomoRuntime>, Router) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MomoRuntime::initialize(directory.path()).await.unwrap());
    let (_, app) = app_for_runtime(runtime.clone(), origin);
    (directory, runtime, app)
}

fn app_for_runtime(runtime: Arc<MomoRuntime>, origin: &str) -> (Arc<MomoApiService>, Router) {
    let model_connections = Arc::new(
        model_connections::ModelConnections::load(
            runtime.core().data_dir().join("test-host-models.json"),
        )
        .unwrap(),
    );
    let api = Arc::new(
        MomoApiService::new(runtime.clone(), origin, None, reqwest::Client::new())
            .with_model_connections(model_connections.clone()),
    );
    let app = build_app(AppState {
        model_connections,
        runtime: runtime.clone(),
        momo_api: api.clone(),
        response_concurrency: Arc::new(Semaphore::new(8)),
        response_timeout: std::time::Duration::from_secs(120),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    });
    (api, app)
}

async fn send_json(
    app: &Router,
    method: Method,
    uri: &str,
    body: Value,
    revision: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(revision) = revision {
        request = request.header(header::IF_MATCH, revision);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, headers, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn management_routes_expose_patch_etag_space_inheritance_and_state_contract_cas() {
    let (_directory, _runtime, app) = isolated_app("http://127.0.0.1:9/v1").await;
    let (status, headers, _) =
        send_json(&app, Method::GET, "/v1/runtime-settings", Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    let revision = headers[header::ETAG].to_str().unwrap();
    let (status, _, _) = send_json(
        &app,
        Method::PATCH,
        "/v1/runtime-settings",
        json!({"vision":{"enabled":true}}),
        Some(revision),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = send_json(
        &app,
        Method::PATCH,
        "/v1/runtime-settings",
        json!({"vision":{"enabled":false}}),
        Some(revision),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "revision_conflict");
    let space = uuid::Uuid::now_v7();
    let uri = format!("/v1/runtime-settings/{space}");
    let (status, _, body) = send_json(
        &app,
        Method::PATCH,
        &uri,
        json!({"history_window":{"evict_turns":4}}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["vision"]["enabled"], true);
    assert_eq!(body["history_window"]["evict_turns"], 4);
    let (_, _, metadata) = send_json(
        &app,
        Method::GET,
        &format!("/v1/runtime-settings/metadata?space_id={space}"),
        Value::Null,
        None,
    )
    .await;
    assert_eq!(
        metadata["sources"]["history_window.evict_turns"],
        "space_override"
    );
    assert_eq!(metadata["sources"]["vision.enabled"], "instance");
    let (status, _, description) = send_json(
        &app,
        Method::GET,
        "/v1/runtime-settings/description",
        Value::Null,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        description["fields"]["mo_state.ddm.enabled"]["default"],
        false
    );
    assert_eq!(
        description["fields"]["mo_state.max_reconcile_steps"]["maximum"],
        16
    );
    let (status, _, body) = send_json(&app, Method::DELETE, &uri, Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["history_window"]["evict_turns"], 2);
    let uri = format!("/v1/mo-state/contract?space_id={space}");
    let (_, _, original) = send_json(&app, Method::GET, &uri, Value::Null, None).await;
    let (status, _, changed) = send_json(
        &app,
        Method::PUT,
        &uri,
        json!({"content":"version: 1\ndimensions: {}\n"}),
        original["revision"].as_str(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(changed["source"], "override");
    let (status, _, _) = send_json(
        &app,
        Method::DELETE,
        &uri,
        Value::Null,
        original["revision"].as_str(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, reset) = send_json(
        &app,
        Method::DELETE,
        &uri,
        Value::Null,
        changed["revision"].as_str(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reset["source"], "builtin");
}

async fn proposal_gateway(
    invalid: Arc<std::sync::atomic::AtomicBool>,
    delayed: bool,
) -> (
    String,
    Arc<std::sync::atomic::AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_test_http_request(&mut socket).await;
            let response = if request.starts_with("GET ") {
                json!({"id":"conversation","momo":{"context_window":8192,"max_output_tokens":1024,"modalities":["text"]}})
            } else {
                let body: Value =
                    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                let content = if body["model"] == "memory_distillation" {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let source: Value =
                        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap())
                            .unwrap();
                    assert!(source.get("character_definition").is_some());
                    assert!(!body.to_string().contains("USER_PRIVATE_DESCRIPTION"));
                    if delayed {
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                    if invalid.load(Ordering::SeqCst) {
                        "schema: invalid\n".into()
                    } else {
                        format!(
                            "schema: momo.ddm/1\ncharacter_id: {}\nrevision: 1\nprofile: logit_additive\ndispositions:\n  - id: gentle_voice\n    base_activation: 0.8\n    expression:\n      latent: Keep a gentle voice.\n      salient: Speak gently.\n      dominant: Speak gently and preserve user agency.\n",
                            source["character_definition"]["character_id"]
                                .as_str()
                                .unwrap()
                        )
                    }
                } else {
                    "Gentle response".into()
                };
                json!({"id":"mock", "choices":[{"message":{"role":"assistant","content":content},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":5,"completion_tokens":5,"total_tokens":10}})
            };
            write_test_json_response(&mut socket, response).await;
        }
    });
    (format!("http://{address}/v1"), calls, task)
}

#[tokio::test]
async fn automatic_ddm_is_cached_scoped_regenerated_and_does_not_edit_character_or_user() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let invalid = Arc::new(AtomicBool::new(false));
    let (origin, calls, gateway) = proposal_gateway(invalid.clone(), false).await;
    let (_directory, runtime, app) = isolated_app(&origin).await;
    let space = uuid::Uuid::now_v7();
    runtime
        .patch_runtime_settings(
            Some(space),
            json!({"mo_state":{"ddm":{"enabled":true},"max_agent_steps":2,"max_reconcile_steps":2},
        "runtime":{"memory_distillation_enabled":false,"semantic_graph_enabled":false}}),
            None,
        )
        .unwrap();
    let character = runtime_api::stage_character(
        &runtime,
        space.to_string(),
        "Gentle".into(),
        "Creator".into(),
        "The character speaks gently.".into(),
        "USER_PRIVATE_DESCRIPTION".into(),
    )
    .await
    .unwrap();
    let mut request = json!({"model":"conversation","input":"Hello", "momo":{"schema":"momo.responses/1.0",
        "personal_space_id":space,"conversation_space_id":space,"character_id":character.id,"mo_state":true,
        "memory_write_space_id":space,"memory_sources":[{"space_id":space,"label":"personal","weight":100,"memory":true,"semantic_graph":false}]}});
    request["momo"]["request_id"] = json!("generated-first");
    let (status, _, first) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        first["momo"]["state_audit"]["ddm_profile"]["source"],
        "runtime_generated"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first["usage"]["total_tokens"], 20);
    request["momo"]["conversation_id"] = first["momo"]["conversation_id"].clone();
    request["momo"]["request_id"] = json!("generated-second");
    let (_, _, second) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(
        second["momo"]["state_audit"]["ddm_profile"]["source"],
        "runtime_cache"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(second["usage"]["total_tokens"], 10);
    assert!(
        runtime_api::character_ddm_profile(&runtime, character.id.to_string())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runtime
            .core()
            .store()
            .character_by_id(character.id)
            .await
            .unwrap()
            .unwrap()
            .character_markdown,
        character.character_markdown
    );
    let other = uuid::Uuid::now_v7();
    assert!(
        runtime_api::managed_ddm_profile(&runtime, other, character.id)
            .await
            .unwrap()
            .is_none()
    );
    let uri = format!(
        "/v1/mo-state/ddm-profile?space_id={space}&character_id={}",
        character.id
    );
    let (status, _, saved) = send_json(&app, Method::GET, &uri, Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["profile"]["dispositions"][0]["id"], "gentle_voice");
    // One continuation is permitted at max_agent_steps=2; persisted replay bypasses the exhausted limit.
    request["input"] = json!([{"type":"function_call","call_id":"call-1","name":"lookup","arguments":"{}"},
        {"type":"function_call_output","call_id":"call-1","output":"done"}]);
    request["momo"]["request_id"] = json!("tool-first");
    let (status, _, tool) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tool}");
    let (status, _, replay) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tool, replay);
    request["momo"]["request_id"] = json!("tool-over-limit");
    let (status, _, _) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Invalid model proposals exhaust the bounded repair limit without publishing a cache.
    send_json(&app, Method::DELETE, &uri, Value::Null, None).await;
    invalid.store(true, Ordering::SeqCst);
    request["input"] = json!("New ordinary user turn");
    request["momo"]["request_id"] = json!("invalid-proposal");
    let (status, _, failed) =
        send_json(&app, Method::POST, "/v1/momo/responses", request, None).await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    assert_eq!(
        failed["momo"]["state_audit"]["ddm_profile"]["source"],
        "degraded"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(failed["usage"]["total_tokens"], 30);
    assert!(
        runtime_api::managed_ddm_profile(&runtime, space, character.id)
            .await
            .unwrap()
            .is_none()
    );
    gateway.abort();
}

#[tokio::test]
async fn generated_ddm_survives_restart_and_tracks_character_and_prompt_revisions() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (origin, calls, gateway) = proposal_gateway(Arc::new(AtomicBool::new(false)), false).await;
    let directory = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MomoRuntime::initialize(directory.path()).await.unwrap());
    let (api, app) = app_for_runtime(runtime.clone(), &origin);
    let space = uuid::Uuid::now_v7();
    runtime
        .patch_runtime_settings(
            Some(space),
            json!({"mo_state":{"ddm":{"enabled":true}},
        "runtime":{"memory_distillation_enabled":false,"semantic_graph_enabled":false}}),
            None,
        )
        .unwrap();
    let mut character = runtime_api::stage_character(
        &runtime,
        space.to_string(),
        "Gentle".into(),
        "Creator".into(),
        "The character speaks gently.".into(),
        "USER_PRIVATE_DESCRIPTION".into(),
    )
    .await
    .unwrap();
    let mut request = json!({"model":"conversation","input":"Hello","momo":{"schema":"momo.responses/1.0",
        "request_id":"before-restart","personal_space_id":space,"conversation_space_id":space,
        "character_id":character.id,"mo_state":true}});
    let (status, _, body) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    api.wait_for_maintenance().await;
    drop(app);
    drop(api);
    drop(runtime);

    let runtime = Arc::new(MomoRuntime::initialize(directory.path()).await.unwrap());
    let (api, app) = app_for_runtime(runtime.clone(), &origin);
    assert!(
        runtime
            .runtime_settings_for_space(space)
            .mo_state
            .ddm
            .enabled
    );
    request["momo"]["request_id"] = json!("after-restart");
    let (status, _, body) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["momo"]["state_audit"]["ddm_profile"]["source"],
        "runtime_cache"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    character.user_markdown.push_str(" changed user preference");
    runtime_api::stage_character_update(&runtime, space.to_string(), character.clone())
        .await
        .unwrap();
    request["momo"]["request_id"] = json!("user-only-change");
    let (status, _, body) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    character
        .character_markdown
        .push_str(" The character also speaks patiently.");
    runtime_api::stage_character_update(&runtime, space.to_string(), character)
        .await
        .unwrap();
    request["momo"]["request_id"] = json!("character-change");
    let (status, _, body) = send_json(
        &app,
        Method::POST,
        "/v1/momo/responses",
        request.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["momo"]["state_audit"]["ddm_profile"]["source"],
        "runtime_generated"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let prompt = runtime
        .prompt_spaces()
        .get(momo_core::PromptSpaceId::DdmProfileGeneration);
    runtime
        .prompt_spaces()
        .replace(
            momo_core::PromptSpaceId::DdmProfileGeneration,
            format!("{}\nKeep proposals concise.", prompt.content),
        )
        .unwrap();
    request["momo"]["request_id"] = json!("prompt-change");
    let (status, _, body) =
        send_json(&app, Method::POST, "/v1/momo/responses", request, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["momo"]["state_audit"]["ddm_profile"]["source"],
        "runtime_generated"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    api.wait_for_maintenance().await;
    gateway.abort();
}

#[tokio::test]
async fn mo_state_timeout_cancels_generation_without_publishing_a_profile() {
    let (origin, _, gateway) =
        proposal_gateway(Arc::new(std::sync::atomic::AtomicBool::new(false)), true).await;
    let (_directory, runtime, app) = isolated_app(&origin).await;
    let space = uuid::Uuid::now_v7();
    runtime
        .patch_runtime_settings(
            Some(space),
            json!({"mo_state":{"ddm":{"enabled":true},"operation_timeout_ms":1000},
        "runtime":{"memory_distillation_enabled":false,"semantic_graph_enabled":false}}),
            None,
        )
        .unwrap();
    let character = runtime_api::stage_character(
        &runtime,
        space.to_string(),
        "Gentle".into(),
        "Creator".into(),
        "Gentle voice".into(),
        String::new(),
    )
    .await
    .unwrap();
    let request = json!({"model":"conversation","input":"Hello","momo":{"schema":"momo.responses/1.0","request_id":"timeout",
        "personal_space_id":space,"conversation_space_id":space,"character_id":character.id,"mo_state":true}});
    let (status, _, body) =
        send_json(&app, Method::POST, "/v1/momo/responses", request, None).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{body}");
    assert_eq!(body["error"]["code"], "timeout");
    assert!(
        runtime_api::managed_ddm_profile(&runtime, space, character.id)
            .await
            .unwrap()
            .is_none()
    );
    gateway.abort();
}
