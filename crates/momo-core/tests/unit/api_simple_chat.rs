use super::*;
use crate::MomoRuntime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn cancellation_also_interrupts_non_streaming_generation() {
    let directory = tempfile::tempdir().expect("runtime directory");
    let runtime = Arc::new(
        MomoRuntime::initialize(directory.path())
            .await
            .expect("runtime"),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let (ready, received) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut bytes = [0_u8; 8192];
        assert!(socket.read(&mut bytes).await.expect("request") > 0);
        ready.send(()).expect("signal request arrival");
        std::future::pending::<()>().await;
    });
    let service = {
        let runtime = runtime.clone();
        runtime
            .update_runtime_settings(crate::MomoRuntimeSettings::default())
            .expect("runtime settings");
        MomoApiService::new(
            runtime,
            format!("http://{address}/v1"),
            None,
            reqwest::Client::new(),
        )
    };
    let task_service = service.clone();
    let key = super::super::execution::scoped_operation_key(
        "00000000-0000-4000-8000-000000000002",
        "non-stream",
    );
    let task_key = key.clone();
    let task = tokio::spawn(async move {
        let _operation = task_service
            .coordination
            .enter_operation(&task_key, "00000000-0000-4000-8000-000000000002");
        task_service
            .generate_response_completion(GatewayResponseAttempt {
                request_id: "non-stream",
                operation_key: &task_key,
                personal_space_id: "00000000-0000-4000-8000-000000000002",
                model: "conversation",
                messages: vec![GatewayMessage {
                    role: crate::GatewayMessageRole::User,
                    content: Some(crate::GatewayMessageContent::Text("wait".to_owned())),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                }],
                temperature: None,
                request_parameters: Default::default(),
                stream: None,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .expect("request reached upstream")
        .expect("ready");
    assert!(service.cancel("00000000-0000-4000-8000-000000000002", "non-stream"));
    let result = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("prompt cancellation")
        .expect("task");
    assert_eq!(
        result.expect_err("cancelled").kind,
        super::super::MomoApiErrorKind::Cancelled
    );
    assert!(
        !runtime.cancel_chat(&key),
        "registration must leave with the request"
    );
    upstream.abort();
}

#[tokio::test]
async fn cancellation_drops_a_stalled_upstream_stream_immediately() {
    let directory = tempfile::tempdir().expect("runtime directory");
    let runtime = Arc::new(
        MomoRuntime::initialize(directory.path())
            .await
            .expect("initialize runtime"),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("request");
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("headers");
        socket.flush().await.expect("flush");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    });

    let request_id = format!("cancel-{}", momo_domain::new_id());
    let request_id_for_task = request_id.clone();
    let task_runtime = Arc::clone(&runtime);
    let task = tokio::spawn(async move {
        stream_gateway_completion(
            task_runtime.as_ref(),
            &request_id_for_task,
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            None,
            &[GatewayMessage {
                role: crate::GatewayMessageRole::User,
                content: Some(crate::GatewayMessageContent::Text("wait".to_owned())),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            ChatParameters::default(),
            |_| true,
        )
        .await
    });

    let mut registered = false;
    for _ in 0..100 {
        if runtime.cancel_chat(&request_id) {
            registered = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        registered,
        "request must register cancellation before streaming"
    );
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), task)
        .await
        .expect("cancellation must not wait for the next upstream chunk")
        .expect("task");
    assert!(matches!(
        result.expect_err("cancelled result"),
        GatewayError::Cancelled
    ));
}

#[derive(Debug)]
struct InjectedConnections(crate::ModelConnection);
impl crate::ModelConnectionResolver for InjectedConnections {
    fn resolve(&self, reference: &str) -> Result<Option<crate::ModelConnection>, String> {
        Ok((reference == "configured_fixture").then(|| self.0.clone()))
    }
}

#[tokio::test]
async fn model_resolver_is_explicit_service_scoped_and_never_falls_back_for_unknown_references() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MomoRuntime::initialize(directory.path()).await.unwrap());
    let service = MomoApiService::new(
        runtime.clone(),
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
    );
    assert!(
        service
            .configured_connection("conversation")
            .unwrap()
            .is_none()
    );
    assert!(service.configured_connection("configured_fixture").is_err());
    let provider = crate::ModelProvider {
        id: uuid::Uuid::now_v7(),
        name: "Injected".into(),
        protocol: crate::ProviderProtocol::Openai,
        base_url: "http://127.0.0.1:9/v1".into(),
        api_path: String::new(),
        api_key: None,
    };
    let connection = crate::ModelConnection {
        model: crate::ConfiguredModel {
            id: uuid::Uuid::now_v7(),
            provider_id: provider.id,
            model_id: "host-model".into(),
            name: "Host model".into(),
            kind: crate::ModelKind::Chat,
            context_window: Some(8192),
            max_output_tokens: None,
        },
        provider,
    };
    let injected = service
        .clone()
        .with_model_connections(Arc::new(InjectedConnections(connection.clone())));
    assert_eq!(
        injected
            .configured_connection("configured_fixture")
            .unwrap()
            .unwrap()
            .model
            .model_id,
        "host-model"
    );
    assert!(
        injected
            .configured_connection("configured_missing")
            .is_err()
    );
    assert!(
        injected
            .configured_connection("conversation")
            .unwrap()
            .is_none()
    );
    assert!(service.configured_connection("configured_fixture").is_err());
    let mut invalid = connection;
    invalid.provider.base_url = "file:///private".into();
    let bad = service.with_model_connections(Arc::new(InjectedConnections(invalid)));
    assert!(bad.configured_connection("configured_fixture").is_err());
    assert!(!directory.path().join("model-connections.json").exists());
}
