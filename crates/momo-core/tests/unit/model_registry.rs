use super::*;
use crate::{ChatParameters, GatewayMessage, GatewayMessageRole};
use axum::{
    Json, Router,
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::post,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn connection(protocol: ProviderProtocol, base_url: String) -> ModelConnection {
    let provider = ModelProvider {
        id: uuid::Uuid::now_v7(),
        name: "Fixture".into(),
        protocol,
        base_url,
        api_path: String::new(),
        api_key: Some("fixture-secret".into()),
    };
    let model = ConfiguredModel {
        id: uuid::Uuid::now_v7(),
        provider_id: provider.id,
        model_id: "example".into(),
        name: "Example".into(),
        kind: ModelKind::Chat,
        context_window: Some(8192),
        max_output_tokens: None,
    };
    ModelConnection { provider, model }
}

#[test]
fn optional_context_is_preserved_without_accepting_zero_or_invalid_output() {
    let mut connection = connection(ProviderProtocol::Openai, "http://127.0.0.1:9/v1".into());
    connection.model.context_window = None;
    let mut configuration = ModelConfiguration {
        revision: 0,
        providers: vec![connection.provider],
        models: vec![connection.model],
    };
    configuration.validate().unwrap();
    assert_eq!(configuration.models[0].effective_context_window(), 8192);
    let encoded = serde_json::to_value(&configuration.models[0]).unwrap();
    assert!(encoded["context_window"].is_null());
    let mut omitted = encoded.clone();
    omitted.as_object_mut().unwrap().remove("context_window");
    let decoded: ConfiguredModel = serde_json::from_value(omitted).unwrap();
    assert_eq!(decoded.context_window, None);
    configuration.models[0].max_output_tokens = Some(10000);
    configuration.validate().unwrap();
    assert_eq!(configuration.models[0].effective_context_window(), 10001);
    configuration.models[0].context_window = Some(0);
    assert!(configuration.validate().is_err());
    configuration.models[0].context_window = Some(8192);
    assert!(configuration.validate().is_err());
    configuration.models[0].max_output_tokens = Some(0);
    assert!(configuration.validate().is_err());
}

#[test]
fn chat_is_default_and_google_protocol_is_rejected() {
    assert_eq!(ProviderProtocol::default(), ProviderProtocol::Openai);
    assert!(serde_json::from_value::<ProviderProtocol>(json!("gemini")).is_err());
}

#[tokio::test]
async fn compatible_and_explicit_responses_connections_use_responses_http_and_sse() {
    for mode in 0..3 {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app=Router::new().route("/v1/responses",post(move|Json(body):Json<Value>|{
            captured.lock().unwrap().push(body.clone()); async move {
                let final_value=json!({"id":"responses-direct","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"完整回复"}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}});
                if body["stream"]!=true { return Json(final_value).into_response(); }
                ([(header::CONTENT_TYPE,"text/event-stream")],format!("data: {}\n\ndata: {}\n\n",
                    json!({"type":"response.output_text.delta","delta":"完整回复"}),
                    json!({"type":"response.completed","response":final_value}))).into_response()
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut c = connection(
            if mode == 2 {
                ProviderProtocol::OpenaiResponses
            } else {
                ProviderProtocol::Openai
            },
            format!("http://{address}/v1"),
        );
        if mode == 0 {
            c.provider.api_path = "/responses".into();
        } else {
            c.provider.base_url.push_str("/responses");
        }
        assert_eq!(c.effective_protocol(), ProviderProtocol::OpenaiResponses);
        let text = "引号\"、换行\n与反斜杠\\";
        let messages = [GatewayMessage {
            role: GatewayMessageRole::User,
            content: Some(text.into()),
            tool_call_id: None,
            tool_calls: vec![],
        }];
        let parameters = ChatParameters {
            temperature: None,
            request_parameters: serde_json::Map::from_iter([("max_tokens".into(), json!(300))]),
        };
        assert_eq!(
            c.complete(&messages, parameters.clone())
                .await
                .unwrap()
                .content,
            "完整回复"
        );
        let mut output = String::new();
        c.stream(&messages, parameters, |event| {
            output.push_str(&event.delta);
            true
        })
        .await
        .unwrap();
        assert_eq!(output, "完整回复");
        for body in seen.lock().unwrap().iter() {
            assert_eq!(body["input"][0]["content"][0]["text"], text);
            assert_eq!(body["max_output_tokens"], 300);
            assert!(body.get("max_tokens").is_none() && body.get("messages").is_none());
        }
        server.abort();
    }
}
#[tokio::test]
async fn native_protocols_convert_requests_and_stream_text_with_terminal_checks() {
    for protocol in [
        ProviderProtocol::Openai,
        ProviderProtocol::OpenaiResponses,
        ProviderProtocol::Anthropic,
    ] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app=Router::new().route("/*path",post(move|headers:HeaderMap,Json(body):Json<Value>|{
            captured.lock().unwrap().push((headers,body.clone()));async move {
                let stream=body["stream"]==true;
                let final_value=match protocol{
                    ProviderProtocol::Openai=>json!({"id":"upstream","choices":[{"message":{"content":"你好"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}),
                    ProviderProtocol::OpenaiResponses=>json!({"id":"upstream","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"你好"}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}),
                    ProviderProtocol::Anthropic=>json!({"id":"upstream","content":[{"type":"text","text":"你好"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}),
                };
                if !stream{return Json(final_value).into_response();}
                let values=match protocol{
                    ProviderProtocol::Openai=>vec![json!({"id":"upstream","choices":[{"delta":{"content":"你"}}]}),json!({"choices":[{"delta":{"content":"好"},"finish_reason":"stop"}]}),json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})],
                    ProviderProtocol::OpenaiResponses=>vec![json!({"type":"response.output_text.delta","delta":"你"}),json!({"type":"response.output_text.delta","delta":"好"}),json!({"type":"response.completed","response":final_value})],
                    ProviderProtocol::Anthropic=>vec![json!({"type":"message_start","message":{"id":"upstream","content":[],"usage":{"input_tokens":3}}}),json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你"}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"好"}}),json!({"type":"content_block_stop","index":0}),json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),json!({"type":"message_stop"})],
                };
                let mut data=values.iter().map(|v|format!("data: {v}\n\n")).collect::<String>();if protocol==ProviderProtocol::Openai{data.push_str("data: [DONE]\n\n");}
                ([(header::CONTENT_TYPE,"text/event-stream")],data).into_response()
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = connection(protocol, format!("http://{address}/v1"));
        let messages = [
            GatewayMessage {
                role: GatewayMessageRole::System,
                content: Some("system".into()),
                tool_call_id: None,
                tool_calls: vec![],
            },
            GatewayMessage {
                role: GatewayMessageRole::User,
                content: Some("hello".into()),
                tool_call_id: None,
                tool_calls: vec![],
            },
        ];
        let parameters = ChatParameters {
            temperature: None,
            request_parameters: serde_json::Map::from_iter([
                ("max_tokens".into(), json!(300)),
                ("top_p".into(), json!(0.7)),
            ]),
        };
        let mut deltas = String::new();
        let completed = c
            .stream(&messages, parameters, |event| {
                deltas.push_str(&event.delta);
                true
            })
            .await
            .unwrap();
        assert_eq!(deltas, "你好");
        assert_eq!(completed.content, "你好");
        assert_eq!(completed.usage.unwrap().total_tokens, 5);
        let captured = seen.lock().unwrap();
        let (headers, body) = &captured[0];
        match protocol {
            ProviderProtocol::Anthropic => {
                assert_eq!(headers["x-api-key"], "fixture-secret");
                assert_eq!(body["max_tokens"], 300);
                assert_eq!(body["system"][0]["text"], "system");
            }
            _ => assert_eq!(headers["authorization"], "Bearer fixture-secret"),
        }
        assert!(body.get("temperature").is_none());
        server.abort();
    }
}
#[test]
fn rejects_unterminated_or_inconsistent_native_streams_and_preserves_function_calls() {
    let mut s = transport::StreamState::new(ProviderProtocol::OpenaiResponses);
    s.accept(json!({"type":"response.output_text.delta","delta":"partial"}))
        .unwrap();
    assert!(!s.done);
    assert!(s.accept(json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"changed"}]}]}})).is_err());
    for (protocol, value) in [
        (
            ProviderProtocol::OpenaiResponses,
            json!({"status":"completed","output":[{"type":"function_call","call_id":"call1","name":"weather","arguments":"{\"city\":\"Paris\"}"}]}),
        ),
        (
            ProviderProtocol::Anthropic,
            json!({"stop_reason":"tool_use","content":[{"type":"tool_use","id":"call1","name":"weather","input":{"city":"Paris"}}]}),
        ),
    ] {
        let result = protocol::completion(protocol, &value).unwrap();
        assert_eq!(result.tool_calls[0].function.name, "weather");
        assert_eq!(
            serde_json::from_str::<Value>(&result.tool_calls[0].function.arguments).unwrap()["city"],
            "Paris"
        );
        assert_eq!(result.finish_reason.as_deref(), Some("tool_calls"));
    }
}
