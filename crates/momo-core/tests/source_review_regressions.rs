//! Public API regressions for the source review of 2026-10-02.
#[cfg(test)]
mod tests {
    use axum::{
        Json, Router,
        extract::{Path, State},
        routing::{get, post},
    };
    use momo_core::{MomoApiService, MomoResponseRequest, MomoRuntime, api::runtime_api};
    use serde_json::{Value, json};
    use std::{future::Future, sync::Arc, task::Poll, time::Duration};
    use tokio::sync::{Mutex, Semaphore, mpsc};

    struct Gateway {
        discoveries: Mutex<Vec<String>>,
        calls: Mutex<Vec<Value>>,
        started: mpsc::UnboundedSender<Value>,
        gate: Semaphore,
    }

    async fn discover(State(state): State<Arc<Gateway>>, Path(alias): Path<String>) -> Json<Value> {
        state.discoveries.lock().await.push(alias.clone());
        Json(if alias == "small" {
            json!({"momo":{"context_window":1024,"max_output_tokens":32,"modalities":["text"]}})
        } else {
            json!({"momo":{"context_window":2048,"max_output_tokens":128,"modalities":["text","image"]}})
        })
    }

    async fn complete(State(state): State<Arc<Gateway>>, Json(body): Json<Value>) -> Json<Value> {
        state.calls.lock().await.push(body.clone());
        let _ = state.started.send(body);
        state.gate.acquire().await.unwrap().forget();
        Json(
            json!({"id":"review-local-stub","choices":[{"message":{"role":"assistant","content":"review reply"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}),
        )
    }

    struct Fixture {
        _directory: tempfile::TempDir,
        runtime: Arc<MomoRuntime>,
        service: MomoApiService,
        gateway: Arc<Gateway>,
        events: mpsc::UnboundedReceiver<Value>,
        server: tokio::task::JoinHandle<()>,
        scope: uuid::Uuid,
        character: uuid::Uuid,
        conversation: uuid::Uuid,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    impl Fixture {
        async fn new(permits: usize) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let runtime = Arc::new(MomoRuntime::initialize(directory.path()).await.unwrap());
            let mut settings = runtime.runtime_settings();
            settings.runtime.memory_distillation_enabled = false;
            settings.runtime.semantic_graph_enabled = false;
            settings.roleplay.enabled = false;
            runtime.update_runtime_settings(settings).unwrap();
            let scope = uuid::Uuid::now_v7();
            let character = runtime_api::stage_character(
                &runtime,
                scope.to_string(),
                "review".into(),
                "Review character".into(),
                String::new(),
                String::new(),
            )
            .await
            .unwrap();
            let conversation = runtime_api::stage_conversation(
                &runtime,
                None,
                scope.to_string(),
                "Review conversation".into(),
                character.id.to_string(),
            )
            .await
            .unwrap();
            let (started, events) = mpsc::unbounded_channel();
            let gateway = Arc::new(Gateway {
                discoveries: Mutex::new(vec![]),
                calls: Mutex::new(vec![]),
                started,
                gate: Semaphore::new(permits),
            });
            let app = Router::new()
                .route("/v1/models/:alias", get(discover))
                .route("/v1/chat/completions", post(complete))
                .with_state(gateway.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let service = MomoApiService::new(
                runtime.clone(),
                format!("http://{address}/v1"),
                None,
                reqwest::Client::new(),
            );
            Self {
                _directory: directory,
                runtime,
                service,
                gateway,
                events,
                server,
                scope,
                character: character.id,
                conversation: conversation.id,
            }
        }

        fn request(&self, input: Value) -> MomoResponseRequest {
            serde_json::from_value(json!({"input":input,"momo":{"schema":"momo.responses/1.0","personal_space_id":self.scope,"conversation_space_id":self.scope,"conversation_id":self.conversation,"character_id":self.character,"mo_state":false}})).unwrap()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn historical_tool_messages_must_fit_before_expansion() {
        let fixture = Fixture::new(3).await;
        let mut character = fixture
            .runtime
            .core()
            .store()
            .character_by_id(fixture.character)
            .await
            .unwrap()
            .unwrap();
        character.character_markdown = "Character traits. ".repeat(110);
        runtime_api::stage_character_update(&fixture.runtime, fixture.scope.to_string(), character)
            .await
            .unwrap();
        let mut initial = fixture.request(json!("Run inspections"));
        initial.model = "small".into();
        fixture
            .service
            .execute(&initial, "tool-budget-initial", None)
            .await
            .unwrap();
        let items: Vec<_> = (0..14)
            .flat_map(|index| {
                let id = format!("call_{index}");
                [
                    json!({"type":"function_call","call_id":id,"name":"inspect","arguments":"{}"}),
                    json!({"type":"function_call_output","call_id":id,"output":"ok"}),
                ]
            })
            .collect();
        let mut tool_input = fixture.request(json!(items));
        tool_input.model = "small".into();
        fixture
            .service
            .execute(&tool_input, "tool-budget-continuation", None)
            .await
            .unwrap();
        let mut followup = fixture.request(json!("What happened?"));
        followup.model = "small".into();
        let response = fixture
            .service
            .execute(&followup, "tool-budget-followup", None)
            .await
            .expect("oversized history should be omitted, allowing the current request");
        let calls = fixture.gateway.calls.lock().await;
        assert_eq!(calls.len(), 3);
        let messages = calls[2]["messages"].as_array().unwrap();
        assert_eq!(messages.last().unwrap()["content"], "What happened?");
        assert!(messages.iter().all(|message| message["role"] != "tool"));
        assert_eq!(
            response.momo.request_audit["history_context"]["tool_exchanges"],
            0
        );
        assert!(
            response.momo.request_audit["history_context"]["omitted_blocks"]
                .as_u64()
                .unwrap()
                >= 1
        );
        assert!(
            response.momo.request_audit["text_context"]["estimated_input_tokens"]
                .as_u64()
                .unwrap()
                <= 864
        );
    }

    fn prompt_tokens(body: &Value) -> usize {
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| {
                momo_core::estimate_text_tokens(message["content"].as_str().unwrap_or_default())
            })
            .sum()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn structured_current_input_must_obey_the_same_budget_as_text() {
        let fixture = Fixture::new(2).await;
        let input = "budget_probe ".repeat(2000);
        fixture
            .service
            .execute(&fixture.request(json!(input)), "plain-budget", None)
            .await
            .unwrap();
        let response = fixture
            .service
            .execute(
                &fixture.request(json!([{"type":"input_text","text":input}])),
                "structured-budget",
                None,
            )
            .await
            .unwrap();
        let calls = fixture.gateway.calls.lock().await;
        let plain = prompt_tokens(&calls[0]);
        let structured = prompt_tokens(&calls[1]);
        println!(
            "plain={plain}, structured={structured}, input_budget={}",
            2048 - 128
        );
        assert!(plain <= 2048 - 128, "plain control should fit");
        assert!(
            structured <= 2048 - 128,
            "structured input bypassed context budget: {structured} > 1920"
        );
        let estimated = response.momo.request_audit["text_context"]["estimated_input_tokens"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(
            estimated,
            structured + calls[1]["messages"].as_array().unwrap().len() * 6
        );
        assert_eq!(
            response.momo.request_audit["text_context"]["excludes_appended_structured_input"],
            false
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn selected_model_must_use_its_own_discovery_and_output_limit() {
        let fixture = Fixture::new(1).await;
        let mut request = fixture.request(json!("hello"));
        request.model = "small".into();
        fixture
            .service
            .execute(&request, "small-route", None)
            .await
            .unwrap();
        let aliases = fixture.gateway.discoveries.lock().await.clone();
        let calls = fixture.gateway.calls.lock().await;
        println!(
            "discovered={aliases:?}, actual_model={}, actual_max_tokens={}, selected_model_limit=32",
            calls[0]["model"], calls[0]["max_tokens"]
        );
        assert!(
            calls[0]["max_tokens"].as_u64().unwrap() <= 32,
            "selected model received another model's token limit"
        );
        assert_eq!(aliases, vec!["small"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_edit_must_not_resurrect_a_deleted_message() {
        let fixture = Fixture::new(0).await;
        let original = runtime_api::stage_message(
            &fixture.runtime,
            fixture.scope.to_string(),
            fixture.conversation.to_string(),
            "user".into(),
            "original".into(),
        )
        .await
        .unwrap();
        let mut edited = original.clone();
        edited.content = "edited after deletion".into();
        let held = fixture
            .runtime
            .core()
            .store()
            .pool()
            .acquire()
            .await
            .unwrap();
        // Both workflows use the public application API. Queue deletion's
        // scope check first, then editing's scope check. The pool's FIFO
        // admission orders both checks before the delete and update writes.
        let mut deletion = Box::pin(runtime_api::stage_message_delete(
            &fixture.runtime,
            fixture.scope.to_string(),
            original.id.to_string(),
        ));
        std::future::poll_fn(|cx| {
            assert!(deletion.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let mut update = Box::pin(runtime_api::stage_message_update(
            &fixture.runtime,
            fixture.scope.to_string(),
            edited,
        ));
        std::future::poll_fn(|cx| {
            assert!(update.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(held);
        let (updated, deleted) = tokio::join!(update, deletion);
        deleted.unwrap();
        let active = fixture
            .runtime
            .core()
            .store()
            .message_by_id(original.id)
            .await
            .unwrap();
        let tombstone = fixture
            .runtime
            .core()
            .store()
            .is_tombstoned("message", original.id)
            .await
            .unwrap();
        println!(
            "update_succeeded={}, active_after_delete={}, tombstone={tombstone}",
            updated.is_ok(),
            active.is_some()
        );
        assert!(
            active.is_none(),
            "the editor recreated a message already deleted by a concurrent transaction"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn new_conversation_response_must_hold_the_conversation_lock() {
        let mut fixture = Fixture::new(0).await;
        let mut first_request = fixture.request(json!("first user turn"));
        first_request.momo.conversation_id = None;
        let service = fixture.service.clone();
        let first = tokio::spawn(async move {
            service
                .execute(&first_request, "create-conversation", None)
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        let conversations =
            runtime_api::local_conversations(&fixture.runtime, fixture.scope.to_string())
                .await
                .unwrap();
        let new_conversation = conversations
            .iter()
            .find(|conversation| conversation.id != fixture.conversation)
            .unwrap()
            .id;
        let mut second_request = fixture.request(json!("second user turn"));
        second_request.momo.conversation_id = Some(new_conversation.to_string());
        second_request.momo.personal_space_id = Some(uuid::Uuid::now_v7().to_string());
        let service = fixture.service.clone();
        let second = tokio::spawn(async move {
            service
                .execute(&second_request, "existing-conversation", None)
                .await
        });
        let overlap = tokio::time::timeout(Duration::from_secs(1), fixture.events.recv())
            .await
            .is_ok();
        fixture.gateway.gate.add_permits(2);
        let first_result = first.await.unwrap();
        let second_result = second.await.unwrap();
        println!(
            "second_generation_started_before_first_completed={overlap}, first_ok={}, second_ok={}",
            first_result.is_ok(),
            second_result.is_ok()
        );
        assert!(first_result.is_ok() && second_result.is_ok());
        assert!(
            !overlap,
            "a request without explicit conversation_id never took the resolved conversation's lock"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn control_existing_conversation_is_serialized_across_personal_spaces() {
        let mut fixture = Fixture::new(0).await;
        let first_request = fixture.request(json!("first existing turn"));
        let service = fixture.service.clone();
        let first = tokio::spawn(async move {
            service
                .execute(&first_request, "existing-first", None)
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        let mut second_request = fixture.request(json!("second existing turn"));
        second_request.momo.personal_space_id = Some(uuid::Uuid::now_v7().to_string());
        let service = fixture.service.clone();
        let second = tokio::spawn(async move {
            service
                .execute(&second_request, "existing-second", None)
                .await
        });
        let overlap = tokio::time::timeout(Duration::from_secs(1), fixture.events.recv())
            .await
            .is_ok();
        fixture.gateway.gate.add_permits(2);
        assert!(first.await.unwrap().is_ok());
        assert!(second.await.unwrap().is_ok());
        println!("explicit-conversation control overlap={overlap}");
        assert!(!overlap);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interrupted_control_claim_must_be_retryable_without_restart() {
        let fixture = Fixture::new(0).await;
        let replacement = runtime_api::stage_character(
            &fixture.runtime,
            fixture.scope.to_string(),
            "replacement".into(),
            "Replacement".into(),
            String::new(),
            String::new(),
        )
        .await
        .unwrap();
        let request = momo_core::MomoControlRequest {
            schema: momo_core::MOMO_CONTROL_SCHEMA.into(),
            request_id: "interrupted-control".into(),
            actor_space_id: fixture.scope,
            action: momo_core::MomoControlAction::SwitchCharacter {
                conversation_space_id: fixture.scope,
                conversation_id: fixture.conversation,
                character_id: replacement.id,
            },
        };
        let held_initial = fixture
            .runtime
            .core()
            .store()
            .pool()
            .acquire()
            .await
            .unwrap();
        let runtime = fixture.runtime.clone();
        let original = request.clone();
        let mut pending_control =
            Box::pin(async move { runtime_api::execute_control(&runtime, original).await });
        std::future::poll_fn(|cx| {
            assert!(pending_control.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(held_initial);
        let held_after_claim = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                // Observe the durable claim using the held connection itself.
                // This works whether the operation is caller-owned or spawned.
                let mut connection = tokio::select! {
                    connection = fixture.runtime.core().store().pool().acquire() => connection.unwrap(),
                    result = &mut pending_control => panic!("control completed before observing its claim: {result:?}"),
                };
                let claimed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM control_operations WHERE response_json IS NULL")
                    .fetch_one(&mut *connection).await.unwrap();
                if claimed == 1 { break connection; }
                drop(connection);
                tokio::task::yield_now().await;
            }
        }).await.expect("unfinished claim must be observed");
        // Cancel the transport while SQLite progress is paused after claiming.
        drop(pending_control);
        drop(held_after_claim);
        let retry = runtime_api::execute_control(&fixture.runtime, request.clone()).await;
        println!("same-runtime retry after cancellation={retry:?}");
        assert!(
            retry.is_ok(),
            "an abandoned control claim permanently blocks identical retries until process restart"
        );
        let replay = runtime_api::execute_control(&fixture.runtime, request.clone())
            .await
            .unwrap();
        assert_eq!(retry.unwrap(), replay);
        assert_eq!(
            fixture
                .runtime
                .core()
                .store()
                .conversation_for_scope(fixture.scope, fixture.conversation)
                .await
                .unwrap()
                .unwrap()
                .character_id,
            Some(replacement.id)
        );
    }

    #[tokio::test]
    async fn oversized_tool_arguments_and_results_are_rejected_before_generation() {
        for large_arguments in [true, false] {
            let fixture = Fixture::new(1).await;
            let large = "tool_payload ".repeat(2000);
            let request = fixture.request(json!([
                {"type":"function_call","call_id":"probe-call","name":"inspect","arguments":if large_arguments { large.clone() } else { "{}".into() }},
                {"type":"function_call_output","call_id":"probe-call","output":if large_arguments { "ok".into() } else { large }},
            ]));
            let error = fixture
                .service
                .execute(&request, "tool-budget", None)
                .await
                .unwrap_err();
            assert_eq!(error.kind, momo_core::MomoApiErrorKind::BadRequest);
            assert!(error.message.contains("input token budget"));
            assert!(fixture.gateway.calls.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn oversized_tool_schema_is_rejected_before_generation() {
        let fixture = Fixture::new(1).await;
        let mut request = fixture.request(json!("hello"));
        request.tools = serde_json::from_value(json!([{
            "type":"function", "name":"inspect", "parameters":{
                "type":"object", "description":"schema_payload ".repeat(2000), "properties":{}
            }
        }]))
        .unwrap();
        let error = fixture
            .service
            .execute(&request, "schema-budget", None)
            .await
            .unwrap_err();
        assert_eq!(error.kind, momo_core::MomoApiErrorKind::BadRequest);
        assert!(fixture.gateway.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn admitted_tool_exchange_keeps_calls_and_results_whole() {
        let fixture = Fixture::new(1).await;
        let request = fixture.request(json!([
            {"type":"function_call","call_id":"probe-call","name":"inspect","arguments":"{\"key\":\"value\"}"},
            {"type":"function_call_output","call_id":"probe-call","output":"a complete tool result"},
        ]));
        let response = fixture
            .service
            .execute(&request, "tool-whole", None)
            .await
            .unwrap();
        let calls = fixture.gateway.calls.lock().await;
        let messages = calls[0]["messages"].as_array().unwrap();
        assert!(
            messages
                .iter()
                .any(|message| message["tool_calls"][0]["function"]["arguments"]
                    == "{\"key\":\"value\"}")
        );
        assert!(messages.iter().any(|message| message["role"] == "tool"
            && message["tool_call_id"] == "probe-call"
            && message["content"] == "a complete tool result"));
        assert!(
            response.momo.request_audit["text_context"]["estimated_input_tokens"]
                .as_u64()
                .unwrap()
                >= 20
        );
    }

    #[tokio::test]
    async fn selected_text_model_uses_vision_fallback_instead_of_direct_images() {
        let fixture = Fixture::new(2).await;
        let mut settings = fixture.runtime.runtime_settings();
        settings.vision.enabled = true;
        fixture.runtime.update_runtime_settings(settings).unwrap();
        let mut request = fixture.request(json!([
            {"type":"input_text", "text":"Describe this image"},
            {"type":"input_image", "image_url":"https://example.test/image.png"},
        ]));
        request.model = "small".into();
        let response = fixture
            .service
            .execute(&request, "small-vision", None)
            .await
            .unwrap();
        assert_eq!(
            response.momo.request_audit["vision"]["mode"],
            "description_fallback"
        );
        let calls = fixture.gateway.calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1]["model"], "small");
        assert!(
            calls[1]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|message| message["content"].is_string())
        );
        assert!(calls[1]["max_tokens"].as_u64().unwrap() <= 32);
    }
}
