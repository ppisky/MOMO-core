//! Non-streaming and streaming chat gateway operations.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatJsonRequest {
    base_url: String,
    api_key: Option<String>,
    model: String,
    messages: Vec<crate::GatewayMessage>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    request_parameters: serde_json::Map<String, serde_json::Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatStreamJsonRequest {
    request_id: String,
    #[serde(flatten)]
    chat: ChatJsonRequest,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareContextJsonRequest {
    #[serde(default)]
    runtime_instructions: String,
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
    messages: Vec<ChatInput>,
    context_window: usize,
    reserve_output_tokens: usize,
}

fn validate_chat_request(request: &ChatJsonRequest) -> Result<(), String> {
    if request
        .temperature
        .is_some_and(|temperature| !(0.0..=2.0).contains(&temperature))
    {
        return Err("temperature must be between 0 and 2".to_owned());
    }
    if request.request_parameters.contains_key("messages")
        || request.request_parameters.contains_key("stream")
    {
        return Err("messages and stream are managed by MOMO".to_owned());
    }
    Ok(())
}

fn endpoint(request: &ChatJsonRequest) -> ProviderEndpoint {
    ProviderEndpoint {
        base_url: request.base_url.clone(),
        api_key: request.api_key.clone(),
        model: request.model.clone(),
    }
}

fn parameters(request: &ChatJsonRequest) -> ChatParameters {
    ChatParameters {
        temperature: request.temperature,
        request_parameters: request.request_parameters.clone(),
    }
}

pub(crate) async fn chat_complete_json(request_json: String) -> Result<String, String> {
    let request: ChatJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    validate_chat_request(&request)?;
    let completion = OpenAiGateway::default()
        .complete_messages(&endpoint(&request), &request.messages, parameters(&request))
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&completion).map_err(|error| error.to_string())
}

pub fn prepare_context_json(request_json: String) -> Result<String, String> {
    let request: PrepareContextJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let prepared = prepare_context(ContextRequest {
        sections: ContextSections {
            runtime_instructions: &request.runtime_instructions,
            character: &request.character_markdown,
            user: &request.user_markdown,
            memory: &request.memory_markdown,
            state: &request.state_context,
            semantic_graph: &request.nsg_markdown,
        },
        messages: &request.messages,
        budget: ContextBudget {
            context_window: request.context_window,
            reserve_output_tokens: request.reserve_output_tokens,
        },
    });
    serde_json::to_string(&prepared).map_err(|error| error.to_string())
}

pub(crate) async fn chat_stream_json(
    request_json: String,
    sink: impl ChatEventSink,
) -> Result<String, String> {
    let request: ChatStreamJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    validate_chat_request(&request.chat)?;
    let cancelled = Arc::new(CancellationSignal {
        cancelled: AtomicBool::new(false),
        notify: Notify::new(),
    });
    CANCELLATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(request.request_id.clone(), Arc::clone(&cancelled));

    let mut sequence = 0_u64;
    let result = {
        let gateway = OpenAiGateway::default();
        let endpoint = endpoint(&request.chat);
        let stream = gateway.stream_messages(
            &endpoint,
            &request.chat.messages,
            parameters(&request.chat),
            |event| {
                if cancelled.cancelled.load(Ordering::Acquire) {
                    return false;
                }
                sequence += 1;
                sink.add(
                    json!({
                        "type": "delta",
                        "request_id": request.request_id,
                        "sequence": sequence,
                        "delta": event.delta,
                        "tool_calls": event.tool_calls,
                        "finish_reason": event.finish_reason,
                    })
                    .to_string(),
                )
                .is_ok()
            },
        );
        tokio::pin!(stream);
        let cancellation = async {
            if !cancelled.cancelled.load(Ordering::Acquire) {
                cancelled.notify.notified().await;
            }
        };
        tokio::pin!(cancellation);
        tokio::select! {
            result = &mut stream => result,
            () = &mut cancellation => Err(GatewayError::Cancelled),
        }
    };

    CANCELLATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&request.request_id);
    match result {
        Ok(completion) => {
            sequence += 1;
            sink.add(
                json!({
                    "type": "done",
                    "request_id": request.request_id,
                    "sequence": sequence,
                    "finish_reason": completion.finish_reason,
                    "usage": completion.usage,
                })
                .to_string(),
            )
            .map_err(|error| error.to_string())?;
            serde_json::to_string(&completion).map_err(|error| error.to_string())
        }
        Err(GatewayError::Cancelled) if cancelled.cancelled.load(Ordering::Acquire) => {
            sequence += 1;
            sink.add(
                json!({
                    "type": "cancelled",
                    "request_id": request.request_id,
                    "sequence": sequence,
                })
                .to_string(),
            )
            .map_err(|error| error.to_string())?;
            Err("model request was cancelled".to_owned())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
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
}
