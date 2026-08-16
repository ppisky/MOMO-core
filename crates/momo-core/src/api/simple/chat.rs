//! Non-streaming and streaming chat gateway operations.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatJsonRequest {
    base_url: String,
    api_key: Option<String>,
    model: String,
    messages: Vec<ChatInput>,
    temperature: f32,
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
    if !(0.0..=2.0).contains(&request.temperature) {
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

pub async fn chat_complete_json(request_json: String) -> Result<String, String> {
    let request: ChatJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    validate_chat_request(&request)?;
    let completion = OpenAiGateway::default()
        .complete(&endpoint(&request), &request.messages, parameters(&request))
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&completion).map_err(|error| error.to_string())
}

pub fn prepare_context_json(request_json: String) -> Result<String, String> {
    let request: PrepareContextJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    let prepared = prepare_context(ContextRequest {
        sections: ContextSections {
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

pub async fn chat_stream_json(
    request_json: String,
    sink: impl ChatEventSink,
) -> Result<(), String> {
    let request: ChatStreamJsonRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    validate_chat_request(&request.chat)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    CANCELLATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(request.request_id.clone(), Arc::clone(&cancelled));

    let mut sequence = 0_u64;
    let result = OpenAiGateway::default()
        .stream(
            &endpoint(&request.chat),
            &request.chat.messages,
            parameters(&request.chat),
            |event| {
                if cancelled.load(Ordering::Acquire) {
                    return false;
                }
                sequence += 1;
                sink.add(
                    json!({
                        "type": "delta",
                        "request_id": request.request_id,
                        "sequence": sequence,
                        "delta": event.delta,
                        "finish_reason": event.finish_reason,
                    })
                    .to_string(),
                )
                .is_ok()
            },
        )
        .await;

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
                })
                .to_string(),
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        }
        Err(GatewayError::Cancelled) if cancelled.load(Ordering::Acquire) => {
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
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}
