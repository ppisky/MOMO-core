//! Model discovery, embeddings, and foreground generation.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{MomoApiError, MomoApiService, MomoResponseEventSink};
use crate::{EmbeddingNormalization, EmbeddingProfile, ResponseUsage, api::simple};

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
    pub(super) messages: Vec<Value>,
    pub(super) temperature: Option<f32>,
    pub(super) request_parameters: Value,
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
    ) -> Result<Value, MomoApiError> {
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
        let gateway_request = json!({
            "base_url": self.gateway_origin,
            "api_key": self.gateway_api_key,
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "request_parameters": request_parameters,
        });

        let generation_gate = self.generation_gate(personal_space_id, "foreground").await;
        let generation_permit = generation_gate
            .acquire_owned()
            .await
            .map_err(|_| MomoApiError::internal("generation gate closed"))?;
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
            let sink = |event_json: String| {
                forward_stream_event(
                    stream,
                    request_id,
                    &message_item_id,
                    &streamed_tools,
                    &event_json,
                )
            };
            let mut stream_request = gateway_request.clone();
            stream_request
                .as_object_mut()
                .expect("gateway request is an object")
                .insert("request_id".to_owned(), json!(operation_key));
            simple::chat_stream_json(stream_request.to_string(), sink)
                .await
                .map_err(MomoApiError::model)?
        } else {
            simple::chat_complete_json(gateway_request.to_string())
                .await
                .map_err(MomoApiError::model)?
        };
        drop(generation_permit);
        serde_json::from_str(&completion)
            .map_err(|error| MomoApiError::model(format!("model returned invalid JSON: {error}")))
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
        let result = simple::generate_embeddings_json(json!({
            "embedding": {"endpoint": {"base_url": self.gateway_origin, "api_key": self.gateway_api_key}, "profile": profile, "timeout_seconds": 120},
            "inputs": [{"id": "query", "text": input, "purpose": "query"}],
        }).to_string()).await?;
        let batch: Value = serde_json::from_str(&result).map_err(|error| error.to_string())?;
        let vector_space_id = batch
            .get("vector_space_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "embedding response lacks vector_space_id".to_owned())?
            .to_owned();
        let vector = batch
            .pointer("/vectors/0/vector")
            .and_then(Value::as_array)
            .ok_or_else(|| "embedding response lacks vectors[0].vector".to_owned())?
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .ok_or_else(|| "embedding vector contains a non-number".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((vector_space_id, vector))
    }
}

fn forward_stream_event(
    stream: &dyn MomoResponseEventSink,
    request_id: &str,
    message_item_id: &str,
    streamed_tools: &Mutex<HashMap<usize, (String, String, bool)>>,
    event_json: &str,
) -> Result<(), String> {
    let event: Value = serde_json::from_str(event_json).map_err(|error| error.to_string())?;
    if event.get("type").and_then(Value::as_str) != Some("delta") {
        return Ok(());
    }
    let delta = event
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !delta.is_empty() {
        stream.send(json!({
            "type": "response.output_text.delta", "request_id": request_id,
            "item_id": message_item_id, "output_index": 0, "content_index": 0, "delta": delta,
        }))?;
    }
    let mut tools = streamed_tools.lock().map_err(|error| error.to_string())?;
    for call in event
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let tool = tools
            .entry(index)
            .or_insert_with(|| (String::new(), String::new(), false));
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            tool.0 = id.to_owned();
        }
        if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
            tool.1 = name.to_owned();
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
            .pointer("/function/arguments")
            .and_then(Value::as_str)
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

pub(super) fn response_usage(completion: &Value) -> ResponseUsage {
    let input_tokens = completion
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let output_tokens = completion
        .pointer("/usage/output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let total_tokens = completion
        .pointer("/usage/total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    ResponseUsage {
        input_tokens,
        output_tokens,
        total_tokens,
    }
}
