//! Non-streaming and streaming chat gateway operations.

use super::*;

fn parse_request_parameters(
    value: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let parameters: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(value).map_err(|error| error.to_string())?;
    if parameters.contains_key("messages") || parameters.contains_key("stream") {
        return Err("messages and stream are managed by MOMO".to_owned());
    }
    Ok(parameters)
}

pub async fn chat_complete_json(
    base_url: String,
    api_key: Option<String>,
    model: String,
    messages_json: String,
    temperature: f32,
    request_parameters_json: String,
) -> Result<String, String> {
    if !(0.0..=2.0).contains(&temperature) {
        return Err("temperature must be between 0 and 2".to_owned());
    }
    let request_parameters = parse_request_parameters(&request_parameters_json)?;
    let messages: Vec<ChatInput> =
        serde_json::from_str(&messages_json).map_err(|error| error.to_string())?;
    let completion = OpenAiGateway::default()
        .complete(
            &ProviderEndpoint {
                base_url,
                api_key,
                model,
            },
            &messages,
            ChatParameters {
                temperature,
                request_parameters,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&completion).map_err(|error| error.to_string())
}

pub fn prepare_context_json(
    character_markdown: String,
    user_markdown: String,
    memory_markdown: String,
    messages_json: String,
    context_window: usize,
    reserve_output_tokens: usize,
) -> Result<String, String> {
    let messages: Vec<ChatInput> =
        serde_json::from_str(&messages_json).map_err(|error| error.to_string())?;
    let prepared = prepare_context(
        &character_markdown,
        &user_markdown,
        &memory_markdown,
        &messages,
        ContextBudget {
            context_window,
            reserve_output_tokens,
        },
    );
    serde_json::to_string(&prepared).map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_context_with_state_json(
    character_markdown: String,
    user_markdown: String,
    memory_markdown: String,
    state_context: String,
    nsg_markdown: String,
    messages_json: String,
    context_window: usize,
    reserve_output_tokens: usize,
) -> Result<String, String> {
    let messages: Vec<ChatInput> =
        serde_json::from_str(&messages_json).map_err(|error| error.to_string())?;
    let prepared = prepare_context_with_state(
        &character_markdown,
        &user_markdown,
        &memory_markdown,
        &state_context,
        &nsg_markdown,
        &messages,
        ContextBudget {
            context_window,
            reserve_output_tokens,
        },
    );
    serde_json::to_string(&prepared).map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
pub async fn chat_stream_json(
    request_id: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
    messages_json: String,
    temperature: f32,
    request_parameters_json: String,
    sink: impl ChatEventSink,
) -> Result<(), String> {
    if !(0.0..=2.0).contains(&temperature) {
        return Err("temperature must be between 0 and 2".to_owned());
    }
    let request_parameters = parse_request_parameters(&request_parameters_json)?;
    let messages: Vec<ChatInput> =
        serde_json::from_str(&messages_json).map_err(|error| error.to_string())?;
    let cancelled = Arc::new(AtomicBool::new(false));
    CANCELLATIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(request_id.clone(), Arc::clone(&cancelled));

    let mut sequence = 0_u64;
    let result = OpenAiGateway::default()
        .stream(
            &ProviderEndpoint {
                base_url,
                api_key,
                model,
            },
            &messages,
            ChatParameters {
                temperature,
                request_parameters,
            },
            |event| {
                if cancelled.load(Ordering::Acquire) {
                    return false;
                }
                sequence += 1;
                sink.add(
                    json!({
                        "type": "delta",
                        "request_id": request_id,
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
        .remove(&request_id);
    match result {
        Ok(completion) => {
            sequence += 1;
            sink.add(
                json!({
                    "type": "done",
                    "request_id": request_id,
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
                    "request_id": request_id,
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
