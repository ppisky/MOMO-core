//! Core-owned background request defaults and explicit, governed overrides.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{MomoApiError, MomoApiService};
use crate::{ChatParameters, OverrideMode, RequestModelRoute, ResponseEmbeddingSelection};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestConfigurationRequest {
    pub personal_space_id: uuid::Uuid,
    #[serde(default)]
    pub model_routes: BTreeMap<String, RequestModelRoute>,
    #[serde(default)]
    pub embedding: Option<ResponseEmbeddingSelection>,
}

#[derive(Debug, Serialize)]
pub struct ModelRequestConfiguration {
    pub defaults: Map<String, Value>,
    pub effective: Map<String, Value>,
    pub overrides: Map<String, Value>,
    pub constraints: Value,
    pub ignored: Vec<String>,
}

pub(crate) fn generation_defaults(alias: &str, context: usize, output: usize) -> ChatParameters {
    let purpose_limit = if alias == "vision" { 1024 } else { 4096 };
    ChatParameters {
        temperature: Some(0.0),
        request_parameters: Map::from_iter([(
            "max_tokens".into(),
            json!(
                purpose_limit
                    .min(output)
                    .min(context.saturating_sub(1))
                    .max(1)
            ),
        )]),
    }
}

pub(super) fn parameter_json(parameters: &ChatParameters) -> Map<String, Value> {
    let mut values = parameters.request_parameters.clone();
    if let Some(output) = values.remove("max_tokens") {
        values.insert("max_output_tokens".into(), output);
    }
    if let Some(temperature) = parameters.temperature {
        values.insert("temperature".into(), json!(temperature));
    }
    values
}

impl MomoApiService {
    pub(super) fn purpose_configuration(
        &self,
        alias: &str,
        scope: &str,
        context: usize,
        output: usize,
    ) -> Result<ModelRequestConfiguration, MomoApiError> {
        let defaults = parameter_json(&generation_defaults(alias, context, output));
        let route = self.model_routes.get(alias);
        let mut overrides = route.map_or_else(Map::new, |route| route.parameters.clone());
        if let Some(route) = route {
            for (key, value) in [
                ("temperature", route.temperature.map(|v| json!(v))),
                ("top_p", route.top_p.map(|v| json!(v))),
                (
                    "max_output_tokens",
                    route.max_output_tokens.map(|v| json!(v)),
                ),
            ] {
                if let Some(value) = value {
                    overrides.insert(key.into(), value);
                }
            }
        }
        overrides.retain(|key, value| defaults.get(key) != Some(value));
        let temperature = overrides
            .remove("temperature")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);
        let max_output = overrides
            .remove("max_output_tokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let policy = self.config_for_space(scope);
        let governed = policy
            .govern(
                context,
                output,
                crate::RequestedOverrides {
                    context_window: None,
                    max_output_tokens: max_output,
                    temperature,
                    instructions: None,
                    visual_description_prompt: None,
                    parameters: &overrides,
                    tool_configuration_requested: false,
                },
            )
            .map_err(MomoApiError::bad_request)?;
        let mut effective = defaults.clone();
        effective.extend(governed.parameters);
        if let Some(value) = governed.temperature {
            effective.insert("temperature".into(), json!(value));
        }
        if max_output.is_some() && policy.request_overrides.max_output_tokens == OverrideMode::Allow
        {
            effective.insert(
                "max_output_tokens".into(),
                json!(governed.max_output_tokens),
            );
        }
        if let Some(value) = temperature {
            overrides.insert("temperature".into(), json!(value));
        }
        if let Some(value) = max_output {
            overrides.insert("max_output_tokens".into(), json!(value));
        }
        Ok(ModelRequestConfiguration {
            defaults,
            effective,
            overrides,
            constraints: json!({
                "temperature": {"min":0,"max":2,"mode":policy.request_overrides.sampling},
                "top_p": {"min":0,"max":1},
                "max_output_tokens": {"min":1,"max":output.min(context.saturating_sub(1)),"mode":policy.request_overrides.max_output_tokens},
                "allowed_parameters": policy.request_overrides.allowed_parameters,
            }),
            ignored: governed.audit["ignored"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        })
    }

    pub(super) fn purpose_chat_parameters(
        &self,
        alias: &str,
        scope: &str,
        context: usize,
        output: usize,
    ) -> Result<ChatParameters, MomoApiError> {
        if let Some(captured) = self.purpose_parameters.get(alias) {
            return Ok(captured.clone());
        }
        let mut values = self
            .purpose_configuration(alias, scope, context, output)?
            .effective;
        let temperature = values
            .remove("temperature")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);
        if let Some(output) = values.remove("max_output_tokens") {
            values.insert("max_tokens".into(), output);
        }
        Ok(ChatParameters {
            temperature,
            request_parameters: values,
        })
    }

    /// Read/validate purpose requests without making a provider request or persisting data.
    pub fn resolve_model_requests(
        &self,
        request: ModelRequestConfigurationRequest,
    ) -> Result<Value, MomoApiError> {
        let mut routed = self.clone();
        routed.model_routes = request.model_routes;
        routed.validate_model_routes()?;
        let scope = request.personal_space_id.to_string();
        let mut purposes = BTreeMap::new();
        for (alias, route) in &routed.model_routes {
            let connection = routed
                .model_connection(&route.model)
                .map_err(MomoApiError::bad_request)?
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "configuration resolution requires a configured model",
                    )
                })?;
            purposes.insert(
                alias.clone(),
                routed.purpose_configuration(
                    alias,
                    &scope,
                    connection.model.effective_context_window(),
                    connection.model.max_output_tokens.unwrap_or(1024),
                )?,
            );
        }
        if let Some(selection) = request.embedding
            && let Some(config) = routed.resolve_embedding_selection(&selection)?
        {
            let defaults = json!({"encoding_format":"float"})
                .as_object()
                .expect("object")
                .clone();
            let mut effective = defaults.clone();
            if let Some(dimension) = config.profile.dimension {
                effective.insert("dimensions".into(), json!(dimension));
            }
            let overrides = effective
                .iter()
                .filter(|(key, value)| defaults.get(*key) != Some(*value))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            purposes.insert("embedding".into(), ModelRequestConfiguration {
                    defaults, effective, overrides,
                    constraints: json!({"dimensions":{"min":1,"max":crate::embedding::MAX_EMBEDDING_DIMENSION,"default":"model","requires":"provider/model support"},"encoding_format":["float"]}),
                    ignored: Vec::new(),
                });
        }
        Ok(json!({"version":1,"purposes":purposes}))
    }
}
