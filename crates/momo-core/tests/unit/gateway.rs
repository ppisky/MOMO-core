use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn normalizes_completion_url_once() {
    assert_eq!(
        completion_url("https://example.com/v1")
            .expect("URL")
            .as_str(),
        "https://example.com/v1/chat/completions"
    );
}

#[test]
fn merges_user_defined_request_parameters_without_replacing_protocol_fields() {
    let request = completion_request(
        &ProviderEndpoint {
            base_url: "https://example.com/v1".to_owned(),
            api_key: None,
            model: "default-model".to_owned(),
        },
        &[GatewayMessage {
            role: GatewayMessageRole::User,
            content: Some("hello".into()),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        &ChatParameters {
            temperature: Some(0.7),
            request_parameters: serde_json::from_value(serde_json::json!({
                "temperature": 1.25,
                "model": "attacker-selected-model",
                "messages": [{"role": "user", "content": "replaced"}],
                "stream": true,
                "top_k": 40,
                "stop": ["END"]
            }))
            .expect("request parameters"),
        },
        false,
    );
    assert!(
        (request["temperature"].as_f64().expect("temperature") - 0.7).abs()
            < f64::from(f32::EPSILON)
    );
    assert_eq!(request["model"], "default-model");
    assert_eq!(request["top_k"], 40);
    assert_eq!(request["stop"], serde_json::json!(["END"]));
    assert_eq!(request["messages"][0]["content"], "hello");
    assert_eq!(request["stream"], false);
}

#[test]
fn serializes_openai_multimodal_user_content_without_changing_text_messages() {
    let request = completion_request(
        &ProviderEndpoint {
            base_url: "https://example.com/v1".to_owned(),
            api_key: None,
            model: "vision".to_owned(),
        },
        &[
            GatewayMessage {
                role: GatewayMessageRole::System,
                content: Some("Describe visible facts.".into()),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            GatewayMessage {
                role: GatewayMessageRole::User,
                content: Some(GatewayMessageContent::Parts(vec![
                    GatewayContentPart::Text {
                        text: "Describe this image.".to_owned(),
                    },
                    GatewayContentPart::ImageUrl {
                        image_url: GatewayImageUrl {
                            url: "https://example.test/image.png".to_owned(),
                            detail: Some("high".to_owned()),
                        },
                    },
                ])),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
        ],
        &ChatParameters::default(),
        false,
    );
    assert_eq!(request["messages"][0]["content"], "Describe visible facts.");
    assert_eq!(request["messages"][1]["content"][0]["type"], "text");
    assert_eq!(
        request["messages"][1]["content"][1]["image_url"]["url"],
        "https://example.test/image.png"
    );
    let messages: Vec<GatewayMessage> =
        serde_json::from_value(request["messages"].clone()).expect("messages");
    validate_gateway_messages(&messages).expect("multimodal user message");
}

#[test]
fn parses_non_text_function_call_completions() {
    let response: CompletionResponse = serde_json::from_value(serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_weather_1",
                    "type": "function",
                    "function": {"name": "lookup_weather", "arguments": "{\"city\":\"Shanghai\"}"}
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }))
    .expect("tool completion");
    let choice = response.choices.into_iter().next().expect("choice");
    assert_eq!(choice.message.content, None);
    assert_eq!(choice.message.tool_calls[0].id, "call_weather_1");
    assert_eq!(choice.message.tool_calls[0].function.name, "lookup_weather");
}

#[test]
fn rejects_an_unbounded_sse_event() {
    let mut decoder = SseDecoder::new();
    let error = decoder
        .push_bytes(&vec![b'a'; MAX_RESPONSE_SSE_EVENT_BYTES + 1])
        .expect_err("oversized event");
    assert!(matches!(error, SseDecodeError::EventTooLarge));
}

#[test]
fn decodes_sse_across_chunk_boundaries() {
    let mut decoder = SseDecoder::new();
    assert!(decoder.push("data: {\"a\":").is_empty());
    assert_eq!(
        decoder.push("1}\n\ndata: [DONE]\n\n"),
        vec!["{\"a\":1}", "[DONE]"]
    );
}

#[test]
fn decodes_utf8_split_across_network_chunks() {
    let mut decoder = SseDecoder::new();
    let encoded = "data: 你好\n\n".as_bytes();
    let split = encoded
        .windows(2)
        .position(|window| window[0] >= 0x80 && window[1] >= 0x80)
        .expect("multibyte content")
        + 1;
    assert!(
        decoder
            .push_bytes(&encoded[..split])
            .expect("partial")
            .is_empty()
    );
    assert_eq!(
        decoder.push_bytes(&encoded[split..]).expect("complete"),
        vec!["你好"]
    );
}

#[test]
fn decodes_sse_one_byte_at_a_time() {
    let mut decoder = SseDecoder::new();
    let payload = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}\n\n",
        "data: [DONE]\n\n"
    );
    let mut events = Vec::new();
    for byte in payload.as_bytes() {
        events.extend(decoder.push_bytes(&[*byte]).expect("byte chunk"));
    }
    assert_eq!(
        events,
        vec![
            "{\"choices\":[{\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}",
            "[DONE]"
        ]
    );
}

#[test]
fn parses_cross_repository_chat_stream_fixture() {
    let fixture = include_str!("../../../../contracts/1.0/chat_stream.sse");
    let mut decoder = SseDecoder::new();
    let mut events = Vec::new();
    for byte in fixture.as_bytes() {
        events.extend(decoder.push_bytes(&[*byte]).expect("fixture byte"));
    }
    assert_eq!(events.last().map(String::as_str), Some("[DONE]"));
    assert!(events.iter().any(|event| event.contains("Hello")));
    assert!(events.iter().any(|event| event.contains("total_tokens")));
}

#[test]
fn rejects_oversized_sse_event_when_delimiter_arrives() {
    let mut decoder = SseDecoder::new();
    let mut payload = Vec::with_capacity(MAX_RESPONSE_SSE_EVENT_BYTES + 16);
    payload.extend_from_slice(b"data: ");
    payload.extend(vec![b'a'; MAX_RESPONSE_SSE_EVENT_BYTES]);
    payload.extend_from_slice(b"\n\n");
    let error = decoder
        .push_bytes(&payload)
        .expect_err("oversized complete event");
    assert!(matches!(error, SseDecodeError::EventTooLarge));
}

#[tokio::test]
async fn streams_openai_compatible_deltas_over_http() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("read request");
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"你\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"好\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Request-ID: upstream-stream-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let gateway = OpenAiGateway {
        client: Client::builder().no_proxy().build().expect("test client"),
    };
    let mut deltas = Vec::new();
    let completion = gateway
        .stream(
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "hello".to_owned(),
            }],
            ChatParameters::default(),
            |event| {
                deltas.push(event.delta);
                true
            },
        )
        .await
        .expect("stream completion");
    assert_eq!(deltas.concat(), "你好");
    assert_eq!(completion.content, "你好");
    assert_eq!(completion.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        completion.upstream_request_id.as_deref(),
        Some("upstream-stream-1")
    );
}

#[tokio::test]
async fn rejects_oversized_non_streaming_response_before_reading_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("request");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_RESPONSE_BODY_BYTES + 1
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("response");
    });
    let gateway = OpenAiGateway {
        client: Client::builder().no_proxy().build().expect("client"),
    };
    let error = gateway
        .complete(
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "hello".to_owned(),
            }],
            ChatParameters::default(),
        )
        .await
        .expect_err("oversized response");
    assert!(matches!(error, GatewayError::ResponseTooLarge));
}

#[tokio::test]
async fn streams_tool_calls_without_buffering_argument_deltas() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("read request");
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Shanghai\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":4,\"total_tokens\":12}}\n\n",
            "data: [DONE]\n\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let gateway = OpenAiGateway {
        client: Client::builder().no_proxy().build().expect("test client"),
    };
    let mut argument_deltas = Vec::new();
    let completion = gateway
        .stream(
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "weather".to_owned(),
            }],
            ChatParameters::default(),
            |event| {
                argument_deltas.extend(event.tool_calls.into_iter().filter_map(|call| {
                    call.function
                        .and_then(|function| function.arguments)
                        .filter(|arguments| !arguments.is_empty())
                }));
                true
            },
        )
        .await
        .expect("tool stream");
    assert_eq!(argument_deltas, ["{\"city\":", "\"Shanghai\"}"]);
    assert!(completion.content.is_empty());
    assert_eq!(completion.tool_calls[0].id, "call_1");
    assert_eq!(completion.tool_calls[0].function.name, "weather");
    assert_eq!(
        completion.tool_calls[0].function.arguments,
        "{\"city\":\"Shanghai\"}"
    );
    assert_eq!(completion.usage.expect("usage").total_tokens, 12);
}

#[tokio::test]
async fn streams_when_http_body_arrives_one_byte_at_a_time() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("read request");
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"slow\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket
            .write_all(header.as_bytes())
            .await
            .expect("write response header");
        for byte in body.as_bytes() {
            socket.write_all(&[*byte]).await.expect("write byte");
            socket.flush().await.expect("flush byte");
            tokio::task::yield_now().await;
        }
    });

    let gateway = OpenAiGateway {
        client: Client::builder().no_proxy().build().expect("test client"),
    };
    let completion = gateway
        .stream(
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "hello".to_owned(),
            }],
            ChatParameters::default(),
            |_| true,
        )
        .await
        .expect("slow stream");
    assert_eq!(completion.content, "slow");
}

#[tokio::test]
async fn rejects_a_stream_that_closes_without_done() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("read request");
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let gateway = OpenAiGateway {
        client: Client::builder().no_proxy().build().expect("test client"),
    };
    let error = gateway
        .stream(
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "hello".to_owned(),
            }],
            ChatParameters::default(),
            |_| true,
        )
        .await
        .expect_err("missing done must fail");
    assert!(matches!(error, GatewayError::IncompleteStream));
}

#[tokio::test]
async fn stops_stream_when_consumer_rejects_more_deltas() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 8_192];
        let _ = socket.read(&mut request).await.expect("read request");
        let mut body = String::new();
        for index in 0..128 {
            body.push_str(&format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{index},\"}},\"finish_reason\":null}}]}}\n\n"
            ));
        }
        body.push_str("data: [DONE]\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let gateway = OpenAiGateway {
        client: Client::builder().no_proxy().build().expect("test client"),
    };
    let mut accepted = 0_usize;
    let error = gateway
        .stream(
            &ProviderEndpoint {
                base_url: format!("http://{address}/v1"),
                api_key: None,
                model: "test-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "hello".to_owned(),
            }],
            ChatParameters::default(),
            |_| {
                accepted += 1;
                accepted < 4
            },
        )
        .await
        .expect_err("consumer rejection must cancel stream");
    assert!(matches!(error, GatewayError::Cancelled));
    assert_eq!(accepted, 4);
}
