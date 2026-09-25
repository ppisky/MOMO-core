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
