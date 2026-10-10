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
    pub(super) fn configured_connection(
        &self,
        alias: &str,
    ) -> Result<Option<crate::model_registry::ModelConnection>, String> {
        self.model_connection(
            self.model_routes
                .get(alias)
                .map_or(alias, |r| r.model.as_str()),
        )
    }
    pub(super) fn validate_model_routes(&self) -> Result<(), MomoApiError> {
        for (alias, route) in &self.model_routes {
            if !matches!(
                alias.as_str(),
                "vision" | "memory_distillation" | "semantic_graph_governance"
            ) || route
                .temperature
                .is_some_and(|v| !v.is_finite() || !(0.0..=2.0).contains(&v))
                || route
                    .top_p
                    .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
                || route.max_output_tokens == Some(0)
            {
                return Err(MomoApiError::bad_request("invalid model route"));
            }
            if let Some(connection) = self
                .model_connection(&route.model)
                .map_err(MomoApiError::bad_request)?
            {
                if connection.model.kind == crate::ModelKind::Embedding
                    || (alias == "vision" && connection.model.kind != crate::ModelKind::Vision)
                {
                    return Err(MomoApiError::bad_request(
                        "model kind is incompatible with route",
                    ));
                }
            } else if route.model != *alias && route.model != "conversation" {
                return Err(MomoApiError::bad_request(
                    "model route requires a configured reference or conversation",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn route_parameters(
        &self,
        alias: &str,
        scope: &str,
    ) -> Result<Option<ChatParameters>, MomoApiError> {
        let Some(route) = self.model_routes.get(alias) else {
            return Ok(None);
        };
        let connection = self
            .configured_connection(alias)
            .map_err(MomoApiError::bad_request)?;
        let (context, output) = connection.as_ref().map_or((8192, 1024), |c| {
            (
                c.model.effective_context_window(),
                c.model.max_output_tokens.unwrap_or(1024),
            )
        });
        let mut values = route.parameters.clone();
        if let Some(p) = route.top_p {
            values.insert("top_p".into(), json!(p));
        }
        let governed = self
            .config_for_space(scope)
            .govern(
                context,
                output,
                crate::RequestedOverrides {
                    context_window: None,
                    max_output_tokens: route.max_output_tokens,
                    temperature: route.temperature,
                    instructions: None,
                    visual_description_prompt: None,
                    parameters: &values,
                    tool_configuration_requested: false,
                },
            )
            .map_err(MomoApiError::bad_request)?;
        let mut parameters = governed.parameters;
        parameters.insert("max_tokens".into(), json!(governed.max_output_tokens));
        Ok(Some(ChatParameters {
            temperature: governed.temperature,
            request_parameters: parameters,
        }))
    }
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
            model: self
                .model_routes
                .get(model)
                .map_or(model, |r| r.model.as_str())
                .to_owned(),
        };
        let connection = self
            .configured_connection(model)
            .map_err(MomoApiError::bad_request)?;
        if connection
            .as_ref()
            .is_some_and(|c| c.model.kind == crate::ModelKind::Embedding)
        {
            return Err(MomoApiError::bad_request(
                "embedding models cannot generate responses",
            ));
        }
        let mut parameters = ChatParameters {
            temperature,
            request_parameters,
        };
        if let Some(route) = self.route_parameters(model, personal_space_id)? {
            parameters.temperature = route.temperature;
            parameters
                .request_parameters
                .extend(route.request_parameters);
        }

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
                connection.as_ref(),
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
            let completion = async {
                if let Some(connection) = connection {
                    connection.complete(&messages, parameters).await
                } else {
                    gateway
                        .complete_messages(&endpoint, &messages, parameters)
                        .await
                }
            };
            tokio::select! {
                result = completion => result.map_err(MomoApiError::model)?,
                () = registration.notified() => return Err(MomoApiError::cancelled()),
            }
        };
        drop(generation_permit);
        Ok(completion)
    }

    pub(super) async fn gateway_response_budget(
        &self,
        model: &str,
    ) -> Result<GatewayGenerationCapability, String> {
        self.gateway_generation_capability(model).await
    }

    pub(super) async fn gateway_generation_capability(
        &self,
        alias: &str,
    ) -> Result<GatewayGenerationCapability, String> {
        if let Some(connection) = self.configured_connection(alias)? {
            if connection.model.kind == crate::ModelKind::Embedding {
                return Err("embedding models cannot generate responses".into());
            }
            return Ok(GatewayGenerationCapability {
                context_window: connection.model.effective_context_window(),
                max_output_tokens: connection
                    .model
                    .max_output_tokens
                    .unwrap_or(1024)
                    .min(connection.model.effective_context_window() - 1),
                supports_images: connection.model.kind == crate::ModelKind::Vision,
            });
        }
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
        let alias = self
            .model_routes
            .get(alias)
            .map_or(alias, |r| r.model.as_str());
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
        configured: Option<&crate::api::runtime_api::EmbeddingRequestConfig>,
    ) -> Result<(String, Vec<f64>), String> {
        if let Some(embedding) = configured {
            let batch = crate::api::runtime_api::generate_embeddings(
                crate::api::runtime_api::GenerateEmbeddingsRequest {
                    embedding: embedding.clone(),
                    inputs: vec![EmbeddingInput {
                        id: "query".into(),
                        text: input.into(),
                        purpose: EmbeddingPurpose::Query,
                    }],
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            let vector = batch
                .vectors
                .into_iter()
                .next()
                .ok_or_else(|| "embedding provider returned no query vector".to_owned())?;
            return Ok((batch.vector_space_id, vector.vector));
        }
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

    pub(super) fn resolve_embedding_selection(
        &self,
        selection: &crate::response::ResponseEmbeddingSelection,
    ) -> Result<Option<crate::api::runtime_api::EmbeddingRequestConfig>, MomoApiError> {
        let Some(reference) = &selection.model else {
            if selection.dimension.is_some() {
                return Err(MomoApiError::bad_request(
                    "disabled embedding cannot declare dimension",
                ));
            }
            return Ok(None);
        };
        let connection = self
            .model_connection(reference)
            .map_err(MomoApiError::bad_request)?
            .ok_or_else(|| {
                MomoApiError::bad_request("embedding requires a configured reference")
            })?;
        if connection.model.kind != crate::ModelKind::Embedding {
            return Err(MomoApiError::bad_request(
                "embedding requires an embedding-kind model",
            ));
        }
        if !matches!(
            connection.provider.protocol,
            crate::ProviderProtocol::Openai | crate::ProviderProtocol::OpenaiResponses
        ) {
            return Err(MomoApiError::bad_request(
                "configured embedding requires an OpenAI-compatible provider",
            ));
        }
        let profile = EmbeddingProfile {
            provider_id: connection.provider.id.to_string(),
            endpoint_id: connection.provider.id.to_string(),
            model: connection.model.model_id,
            dimension: selection
                .dimension
                .ok_or_else(|| MomoApiError::bad_request("embedding dimension is required"))?,
            model_revision: None,
            normalization: EmbeddingNormalization::L2,
            send_dimensions: false,
            query_prefix: String::new(),
            document_prefix: String::new(),
        };
        profile.validate().map_err(MomoApiError::bad_request)?;
        Ok(Some(crate::api::runtime_api::EmbeddingRequestConfig {
            endpoint: EmbeddingEndpoint {
                base_url: connection.provider.base_url,
                api_key: connection.provider.api_key,
            },
            profile,
            timeout_seconds: 120,
        }))
    }
}

async fn stream_gateway_completion<F>(
    runtime: &crate::MomoRuntime,
    operation_key: &str,
    endpoint: &ProviderEndpoint,
    connection: Option<&crate::model_registry::ModelConnection>,
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
    let stream_request = async {
        if let Some(connection) = connection {
            connection.stream(messages, parameters, on_delta).await
        } else {
            gateway
                .stream_messages(endpoint, messages, parameters, on_delta)
                .await
        }
    };
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
