//! Model discovery, embeddings, and foreground generation.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{MomoApiError, MomoApiService, MomoResponseEventSink};
use crate::{
    ChatParameters, ChatStreamDelta, EmbeddingEndpoint, EmbeddingInput, EmbeddingNormalization,
    EmbeddingProfile, EmbeddingProvider, EmbeddingPurpose, GatewayError, GatewayMessage,
    OpenAiEmbeddingProvider, OpenAiGateway, ProviderEndpoint, ResponseUsage,
};

pub(super) struct GatewayGenerationCapability {
    pub(super) context_window: usize,
    pub(super) max_output_tokens: usize,
    pub(super) supports_images: bool,
}

pub(super) struct GatewayResponseAttempt<'a> {
    pub(super) request_id: &'a str,
    pub(super) operation_key: &'a str,
    pub(super) personal_space_id: &'a str,
    pub(super) model: &'a str,
    pub(super) messages: Vec<GatewayMessage>,
    pub(super) temperature: Option<f32>,
    pub(super) request_parameters: serde_json::Map<String, Value>,
    pub(super) stream: Option<&'a dyn MomoResponseEventSink>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayEmbeddingProfile {
    dimension: usize,
    normalization: EmbeddingNormalization,
    #[serde(default)]
    send_dimensions: bool,
    #[serde(default)]
    model_revision: Option<String>,
    #[serde(default)]
    query_prefix: String,
    #[serde(default)]
    document_prefix: String,
}

impl MomoApiService {
    pub(super) async fn generate_response_completion(
        &self,
        attempt: GatewayResponseAttempt<'_>,
    ) -> Result<crate::ChatCompletion, MomoApiError> {
        let GatewayResponseAttempt {
            request_id,
            operation_key,
            personal_space_id,
            model,
            messages,
            temperature,
            request_parameters,
            stream,
        } = attempt;
        if temperature.is_some_and(|value| !(0.0..=2.0).contains(&value)) {
            return Err(MomoApiError::bad_request(
                "temperature must be between 0 and 2",
            ));
        }
        if request_parameters.contains_key("messages") || request_parameters.contains_key("stream")
        {
            return Err(MomoApiError::bad_request(
                "messages and stream are managed by MOMO",
            ));
        }
        let endpoint = ProviderEndpoint {
            base_url: self.gateway_origin.clone(),
            api_key: self.gateway_api_key.clone(),
            model: model.to_owned(),
        };
        let parameters = ChatParameters {
            temperature,
            request_parameters,
        };

        let generation_gate = self
            .coordination
            .generation_gate(personal_space_id, "foreground")
            .await;
        let generation_permit = generation_gate
            .acquire_owned()
            .await
            .map_err(|_| MomoApiError::internal("generation gate closed"))?;
        self.coordination.ensure_active(operation_key)?;
        let completion = if let Some(stream) = stream {
            let message_item_id = format!("msg_{request_id}");
            stream.send(json!({
                "type": "response.output_item.added",
                "request_id": request_id,
                "output_index": 0,
                "item": {"id": message_item_id, "type": "message", "role": "assistant", "status": "in_progress", "content": []},
            })).map_err(MomoApiError::internal)?;
            stream
                .send(json!({
                    "type": "response.content_part.added",
                    "request_id": request_id,
                    "item_id": message_item_id,
                    "output_index": 0,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": "", "annotations": []},
                }))
                .map_err(MomoApiError::internal)?;
            let streamed_tools = Arc::new(std::sync::Mutex::new(HashMap::<
                usize,
                (String, String, bool),
            >::new()));
            let result = stream_gateway_completion(
                self.runtime(),
                operation_key,
                &endpoint,
                &messages,
                parameters,
                |event| {
                    forward_stream_delta(
                        stream,
                        request_id,
                        &message_item_id,
                        &streamed_tools,
                        event,
                    )
                    .is_ok()
                },
            )
            .await;
            match result {
                Ok(completion) => completion,
                Err(GatewayError::Cancelled) => return Err(MomoApiError::cancelled()),
                Err(error) => return Err(MomoApiError::model(error)),
            }
        } else {
            let registration = self
                .runtime()
                .register_cancellation(operation_key.to_owned());
            self.coordination.ensure_active(operation_key)?;
            let gateway = OpenAiGateway::default();
            tokio::select! {
                result = gateway.complete_messages(&endpoint, &messages, parameters) => result.map_err(MomoApiError::model)?,
                () = registration.notified() => return Err(MomoApiError::cancelled()),
            }
        };
        drop(generation_permit);
        Ok(completion)
    }

    pub(super) async fn gateway_response_budget(
        &self,
    ) -> Result<GatewayGenerationCapability, String> {
        self.gateway_generation_capability("conversation").await
    }

    pub(super) async fn gateway_generation_capability(
        &self,
        alias: &str,
    ) -> Result<GatewayGenerationCapability, String> {
        let discovery = self.gateway_model_detail(alias).await?;
        let context_window = discovery
            .pointer("/momo/context_window")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(8_192);
        let max_output_tokens = discovery
            .pointer("/momo/max_output_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(1_024);
        if context_window < 1_024 || max_output_tokens == 0 || max_output_tokens >= context_window {
            return Err(format!(
                "gateway returned an invalid token budget for route {alias}"
            ));
        }
        let supports_images = discovery
            .pointer("/momo/modalities")
            .and_then(Value::as_array)
            .is_some_and(|modalities| {
                modalities
                    .iter()
                    .any(|modality| modality.as_str() == Some("image"))
            });
        Ok(GatewayGenerationCapability {
            context_window,
            max_output_tokens,
            supports_images,
        })
    }

    async fn gateway_model_detail(&self, alias: &str) -> Result<Value, String> {
        let url = format!(
            "{}/models/{alias}",
            self.gateway_origin.trim_end_matches('/')
        );
        let mut request = self
            .gateway_client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(api_key) = self.gateway_api_key.as_deref() {
            request = request.bearer_auth(api_key);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "gateway model discovery returned HTTP {}",
                status.as_u16()
            ));
        }
        response.json().await.map_err(|error| error.to_string())
    }

    pub(super) async fn generate_query_embedding(
        &self,
        input: &str,
    ) -> Result<(String, Vec<f64>), String> {
        let discovery = self.gateway_model_detail("embedding").await?;
        let metadata: GatewayEmbeddingProfile = serde_json::from_value(
            discovery
                .pointer("/momo/embedding_profile")
                .cloned()
                .ok_or_else(|| "gateway embedding route has no embedding_profile".to_owned())?,
        )
        .map_err(|error| error.to_string())?;
        let profile = EmbeddingProfile {
            provider_id: "mobot-gateway".to_owned(),
            endpoint_id: self.gateway_origin.clone(),
            model: "embedding".to_owned(),
            dimension: metadata.dimension,
            model_revision: metadata.model_revision,
            normalization: metadata.normalization,
            send_dimensions: metadata.send_dimensions,
            query_prefix: metadata.query_prefix,
            document_prefix: metadata.document_prefix,
        };
        profile.validate().map_err(|error| error.to_string())?;
        let provider = OpenAiEmbeddingProvider::new(
            EmbeddingEndpoint {
                base_url: self.gateway_origin.clone(),
                api_key: self.gateway_api_key.clone(),
            },
            Duration::from_secs(120),
        )
        .map_err(|error| error.to_string())?;
        let batch = provider
            .embed_batch(
                &profile,
                &[EmbeddingInput {
                    id: "query".to_owned(),
                    text: input.to_owned(),
                    purpose: EmbeddingPurpose::Query,
                }],
            )
            .await
            .map_err(|error| error.to_string())?;
        let vector = batch
            .vectors
            .into_iter()
            .next()
            .ok_or_else(|| "embedding provider returned no query vector".to_owned())?;
        Ok((batch.vector_space_id, vector.vector))
    }
}

async fn stream_gateway_completion<F>(
    runtime: &crate::MomoRuntime,
    operation_key: &str,
    endpoint: &ProviderEndpoint,
    messages: &[GatewayMessage],
    parameters: ChatParameters,
    on_delta: F,
) -> Result<crate::ChatCompletion, GatewayError>
where
    F: FnMut(ChatStreamDelta) -> bool,
{
    let cancelled = runtime.register_cancellation(operation_key.to_owned());
    runtime
        .response_coordination()
        .ensure_active(operation_key)
        .map_err(|_| GatewayError::Cancelled)?;
    let gateway = OpenAiGateway::default();
    let stream_request = gateway.stream_messages(endpoint, messages, parameters, on_delta);
    tokio::pin!(stream_request);
    let cancellation = cancelled.notified();
    tokio::pin!(cancellation);
    let result = tokio::select! {
        result = &mut stream_request => result,
        () = &mut cancellation => Err(GatewayError::Cancelled),
    };
    result
}

fn forward_stream_delta(
    stream: &dyn MomoResponseEventSink,
    request_id: &str,
    message_item_id: &str,
    streamed_tools: &Mutex<HashMap<usize, (String, String, bool)>>,
    event: ChatStreamDelta,
) -> Result<(), String> {
    if !event.delta.is_empty() {
        stream.send(json!({
            "type": "response.output_text.delta", "request_id": request_id,
            "item_id": message_item_id, "output_index": 0, "content_index": 0, "delta": event.delta,
        }))?;
    }
    let mut tools = streamed_tools.lock().map_err(|error| error.to_string())?;
    for call in event.tool_calls {
        let index = call.index;
        let tool = tools
            .entry(index)
            .or_insert_with(|| (String::new(), String::new(), false));
        if let Some(id) = call.id {
            tool.0 = id;
        }
        if let Some(name) = call
            .function
            .as_ref()
            .and_then(|function| function.name.as_ref())
        {
            tool.1.clone_from(name);
        }
        if !tool.2 && !tool.0.is_empty() && !tool.1.is_empty() {
            stream.send(json!({
                "type": "response.output_item.added", "request_id": request_id,
                "output_index": index + 1,
                "item": {"id": tool.0, "call_id": tool.0, "type": "function_call", "name": tool.1, "arguments": "", "status": "in_progress"},
            }))?;
            tool.2 = true;
        }
        if let Some(arguments) = call
            .function
            .and_then(|function| function.arguments)
            .filter(|value| !value.is_empty())
        {
            stream.send(json!({
                "type": "response.function_call_arguments.delta", "request_id": request_id,
                "item_id": tool.0, "output_index": index + 1, "delta": arguments,
            }))?;
        }
    }
    Ok(())
}

pub(super) fn tools_to_chat(tools: &[crate::ResponseTool]) -> Result<Value, MomoApiError> {
    let tools = serde_json::to_value(tools)
        .map_err(|error| MomoApiError::bad_request(error.to_string()))?;
    tools
        .as_array()
        .ok_or_else(|| MomoApiError::bad_request("tools must be an array"))?
        .iter()
        .map(|tool| {
            let mut function = tool
                .as_object()
                .cloned()
                .ok_or_else(|| MomoApiError::bad_request("tool must be an object"))?;
            function.remove("type");
            Ok(json!({"type": "function", "function": function}))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

pub(super) fn tool_choice_to_chat(choice: &Value) -> Result<Value, MomoApiError> {
    if choice.is_string() {
        return Ok(choice.clone());
    }
    let name = choice
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| MomoApiError::bad_request("function tool_choice requires name"))?;
    Ok(json!({"type": "function", "function": {"name": name}}))
}

pub(super) fn response_usage(completion: &crate::ChatCompletion) -> ResponseUsage {
    let usage = completion.usage.clone().unwrap_or_default();
    ResponseUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/api_simple_chat.rs"]
mod tests;
