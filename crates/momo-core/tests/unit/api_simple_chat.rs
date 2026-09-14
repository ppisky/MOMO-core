use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn cancellation_drops_a_stalled_upstream_stream_immediately() {
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

    let request_id = format!("cancel-{}", new_request_id());
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink_events = Arc::clone(&events);
    let request_id_for_task = request_id.clone();
    let task = tokio::spawn(async move {
        chat_stream_json(
            json!({
                "request_id": request_id_for_task,
                "base_url": format!("http://{address}/v1"),
                "api_key": null,
                "model": "test-model",
                "messages": [{"role": "user", "content": "wait"}],
                "request_parameters": {},
            })
            .to_string(),
            move |event| {
                sink_events
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(event);
                Ok(())
            },
        )
        .await
    });

    let mut registered = false;
    for _ in 0..100 {
        if cancel_chat(request_id.clone()) {
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
    assert_eq!(
        result.expect_err("cancelled result"),
        "model request was cancelled"
    );
    let events = events.lock().unwrap_or_else(|error| error.into_inner());
    assert!(
        events
            .iter()
            .any(|event| event.contains("\"type\":\"cancelled\""))
    );
}
