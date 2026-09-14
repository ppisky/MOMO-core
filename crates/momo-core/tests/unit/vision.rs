use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn gateway_adapter_discovers_image_capability_and_describes_input() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut discovery, _) = listener.accept().await.expect("discovery connection");
        let discovery_request = read_request(&mut discovery).await;
        assert!(discovery_request.starts_with("GET /v1/models/vision "));
        write_json(
            &mut discovery,
            json!({"id": "vision", "momo": {"modalities": ["text", "image"]}}),
        )
        .await;

        let (mut completion, _) = listener.accept().await.expect("completion connection");
        let completion_request = read_request(&mut completion).await;
        assert!(completion_request.starts_with("POST /v1/chat/completions "));
        let body = completion_request
            .split_once("\r\n\r\n")
            .expect("request body")
            .1;
        let body: serde_json::Value = serde_json::from_str(body).expect("request JSON");
        assert_eq!(body["model"], "vision");
        assert_eq!(body["messages"][1]["content"][1]["type"], "image_url");
        assert_eq!(
            body["messages"][1]["content"][1]["image_url"]["detail"],
            "high"
        );
        write_json(
            &mut completion,
            json!({
                "id": "vision-upstream-1",
                "choices": [{
                    "message": {"role": "assistant", "content": "A red umbrella."},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}
            }),
        )
        .await;
    });
    let adapter = GatewayVisionAdapter::new(
        reqwest::Client::new(),
        format!("http://{address}/v1"),
        None,
        DEFAULT_VISION_ROUTE,
    );
    let batch = adapter
        .describe(VisionDescriptionRequest {
            images: vec![ResponseImageInput {
                image_url: "https://example.test/image.png".to_owned(),
                detail: Some("high".to_owned()),
            }],
            prompt: "Describe visible facts.".to_owned(),
            request_id: "request-vision-test".to_owned(),
        })
        .await
        .expect("vision description");
    server.await.expect("server task");
    assert_eq!(batch.descriptions, ["A red umbrella."]);
    assert_eq!(batch.usage.input_tokens, 10);
    assert_eq!(batch.usage.output_tokens, 4);
    assert_eq!(batch.usage.total_tokens, 14);
    assert_eq!(batch.upstream_request_ids, ["vision-upstream-1"]);
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut expected = None;
    loop {
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).await.expect("read request");
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if expected.is_none()
            && let Some(header_end) = bytes.windows(4).position(|value| value == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            expected = Some(header_end + 4 + content_length);
        }
        if expected.is_some_and(|length| bytes.len() >= length) {
            break;
        }
    }
    String::from_utf8(bytes).expect("UTF-8 request")
}

async fn write_json(stream: &mut tokio::net::TcpStream, value: serde_json::Value) {
    let body = value.to_string();
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("write response");
}
