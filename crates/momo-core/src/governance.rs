//! Portable request-governance and visual-description configuration.

use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

pub const MOMO_CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverrideMode {
    Allow,
    #[default]
    Ignore,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestOverridePolicy {
    #[serde(default)]
    pub context_window: OverrideMode,
    #[serde(default = "allow")]
    pub max_output_tokens: OverrideMode,
    #[serde(default)]
    pub sampling: OverrideMode,
    #[serde(default = "allow")]
    pub instructions: OverrideMode,
    #[serde(default)]
    pub visual_description_prompt: OverrideMode,
    #[serde(default = "allow")]
    pub tools: OverrideMode,
    #[serde(default)]
    pub allowed_parameters: BTreeSet<String>,
}

impl Default for RequestOverridePolicy {
    fn default() -> Self {
        Self {
            context_window: OverrideMode::Ignore,
            max_output_tokens: OverrideMode::Allow,
            sampling: OverrideMode::Ignore,
            instructions: OverrideMode::Allow,
            visual_description_prompt: OverrideMode::Ignore,
            tools: OverrideMode::Allow,
            allowed_parameters: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisionDescriptionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_visual_description_prompt")]
    pub prompt: String,
}

impl Default for VisionDescriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prompt: default_visual_description_prompt(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MomoConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub request_overrides: RequestOverridePolicy,
    #[serde(default)]
    pub vision: VisionDescriptionConfig,
}

impl Default for MomoConfig {
    fn default() -> Self {
        Self {
            schema_version: MOMO_CONFIG_SCHEMA_VERSION,
            request_overrides: RequestOverridePolicy::default(),
            vision: VisionDescriptionConfig::default(),
        }
    }
}

impl MomoConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GovernanceError> {
        let text = fs::read_to_string(path)?;
        validate_document_ownership(&text)?;
        let config: Self = toml::from_str(&text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self, GovernanceError> {
        let path = path.as_ref();
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn validate(&self) -> Result<(), GovernanceError> {
        if self.schema_version != MOMO_CONFIG_SCHEMA_VERSION {
            return Err(GovernanceError::UnsupportedSchema(self.schema_version));
        }
        if self.vision.prompt.trim().is_empty() || self.vision.prompt.len() > 64 * 1024 {
            return Err(GovernanceError::Invalid(
                "vision.prompt must contain 1 to 65536 bytes".to_owned(),
            ));
        }
        const RESERVED: [&str; 8] = [
            "model",
            "messages",
            "input",
            "stream",
            "context_window",
            "max_tokens",
            "max_output_tokens",
            "temperature",
        ];
        if let Some(field) = self
            .request_overrides
            .allowed_parameters
            .iter()
            .find(|field| RESERVED.contains(&field.as_str()))
        {
            return Err(GovernanceError::Invalid(format!(
                "allowed_parameters contains reserved field {field:?}"
            )));
        }
        Ok(())
    }

    pub fn govern(
        &self,
        capability_context_window: usize,
        route_max_output_tokens: usize,
        requested: RequestedOverrides<'_>,
    ) -> Result<GovernedOverrides, GovernanceError> {
        let visual_override_requested = requested
            .visual_description_prompt
            .is_some_and(|value| !value.trim().is_empty());
        let mut governed = self.request_overrides.apply(
            capability_context_window,
            route_max_output_tokens,
            requested,
        )?;
        let visual_source = if !self.vision.enabled {
            governed.visual_description_prompt = None;
            if visual_override_requested {
                "disabled_request_ignored"
            } else {
                "disabled"
            }
        } else if governed.visual_description_prompt.is_some() {
            "request"
        } else {
            governed.visual_description_prompt = Some(self.vision.prompt.clone());
            "momo"
        };
        governed.audit["visual_description_prompt_source"] = json!(visual_source);
        Ok(governed)
    }
}

pub fn validate_momo_document(text: &str) -> Result<(), GovernanceError> {
    validate_document_ownership(text)?;
    let config: MomoConfig = toml::from_str(text)?;
    config.validate()
}

fn validate_document_ownership(text: &str) -> Result<(), GovernanceError> {
    let table: toml::Table = toml::from_str(text)?;
    const HOST_ONLY: [&str; 10] = [
        "providers",
        "models",
        "model",
        "active_model_profile",
        "modules",
        "core",
        "server",
        "gateway",
        "discord",
        "auth",
    ];
    if let Some(field) = HOST_ONLY.iter().find(|field| table.contains_key(**field)) {
        return Err(GovernanceError::HostField((*field).to_owned()));
    }
    Ok(())
}

pub struct RequestedOverrides<'a> {
    pub context_window: Option<usize>,
    pub max_output_tokens: Option<usize>,
    pub temperature: Option<f32>,
    pub instructions: Option<&'a str>,
    pub visual_description_prompt: Option<&'a str>,
    pub parameters: &'a Map<String, Value>,
    pub tool_configuration_requested: bool,
}

#[derive(Debug, Clone)]
pub struct GovernedOverrides {
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: Option<f32>,
    pub instructions: Option<String>,
    pub visual_description_prompt: Option<String>,
    pub parameters: Map<String, Value>,
    pub allow_tools: bool,
    pub audit: Value,
}

impl RequestOverridePolicy {
    pub fn apply(
        &self,
        capability_context_window: usize,
        route_max_output_tokens: usize,
        requested: RequestedOverrides<'_>,
    ) -> Result<GovernedOverrides, GovernanceError> {
        let mut applied = Vec::new();
        let mut ignored = Vec::new();
        let context_window = governed_usize(
            "context_window",
            self.context_window,
            requested.context_window,
            capability_context_window,
            256,
            capability_context_window,
            &mut applied,
            &mut ignored,
        )?;
        let max_allowed_output = route_max_output_tokens.min(context_window.saturating_sub(1));
        let max_output_tokens = governed_usize(
            "max_output_tokens",
            self.max_output_tokens,
            requested.max_output_tokens,
            max_allowed_output,
            1,
            max_allowed_output,
            &mut applied,
            &mut ignored,
        )?;
        let temperature = match (self.sampling, requested.temperature) {
            (_, None) => None,
            (OverrideMode::Allow, Some(value)) if (0.0..=2.0).contains(&value) => {
                applied.push("temperature");
                Some(value)
            }
            (OverrideMode::Allow, Some(_)) => {
                return Err(GovernanceError::Invalid(
                    "temperature must be between 0 and 2".to_owned(),
                ));
            }
            (OverrideMode::Ignore, Some(_)) => {
                ignored.push("temperature");
                None
            }
            (OverrideMode::Reject, Some(_)) => {
                return Err(GovernanceError::Rejected("temperature"));
            }
        };
        let instructions = governed_text(
            "instructions",
            self.instructions,
            requested.instructions,
            &mut applied,
            &mut ignored,
        )?;
        let visual_description_prompt = governed_text(
            "visual_description_prompt",
            self.visual_description_prompt,
            requested.visual_description_prompt,
            &mut applied,
            &mut ignored,
        )?;
        let allow_tools = match (self.tools, requested.tool_configuration_requested) {
            (_, false) | (OverrideMode::Allow, true) => true,
            (OverrideMode::Ignore, true) => {
                ignored.push("tools");
                false
            }
            (OverrideMode::Reject, true) => return Err(GovernanceError::Rejected("tools")),
        };
        if requested.tool_configuration_requested && allow_tools {
            applied.push("tools");
        }
        let mut parameters = Map::new();
        for (key, value) in requested.parameters {
            if self.allowed_parameters.contains(key) {
                applied.push("parameter");
                parameters.insert(key.clone(), value.clone());
            } else {
                return Err(GovernanceError::ParameterNotAllowed(key.clone()));
            }
        }
        let instructions_source = if instructions.is_some() {
            "request"
        } else {
            "omitted"
        };
        let parameter_names = parameters.keys().cloned().collect::<Vec<_>>();
        Ok(GovernedOverrides {
            context_window,
            max_output_tokens,
            temperature,
            instructions,
            visual_description_prompt,
            parameters,
            allow_tools,
            audit: json!({
                "capability_context_window": capability_context_window,
                "route_max_output_tokens": route_max_output_tokens,
                "effective_context_window": context_window,
                "effective_max_output_tokens": max_output_tokens,
                "context_window_source": if requested.context_window.is_some() && self.context_window == OverrideMode::Allow { "request" } else { "capability" },
                "max_output_tokens_source": if requested.max_output_tokens.is_some() && self.max_output_tokens == OverrideMode::Allow { "request" } else { "route" },
                "temperature_source": if temperature.is_some() { "request" } else { "omitted" },
                "instructions_source": instructions_source,
                "parameter_names": parameter_names,
                "applied": applied,
                "ignored": ignored,
            }),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn governed_usize(
    field: &'static str,
    mode: OverrideMode,
    requested: Option<usize>,
    default: usize,
    min: usize,
    max: usize,
    applied: &mut Vec<&'static str>,
    ignored: &mut Vec<&'static str>,
) -> Result<usize, GovernanceError> {
    match (mode, requested) {
        (_, None) => Ok(default),
        (OverrideMode::Allow, Some(value)) if (min..=max).contains(&value) => {
            applied.push(field);
            Ok(value)
        }
        (OverrideMode::Allow, Some(value)) => Err(GovernanceError::OutOfRange {
            field,
            value,
            min,
            max,
        }),
        (OverrideMode::Ignore, Some(_)) => {
            ignored.push(field);
            Ok(default)
        }
        (OverrideMode::Reject, Some(_)) => Err(GovernanceError::Rejected(field)),
    }
}

fn governed_text(
    field: &'static str,
    mode: OverrideMode,
    requested: Option<&str>,
    applied: &mut Vec<&'static str>,
    ignored: &mut Vec<&'static str>,
) -> Result<Option<String>, GovernanceError> {
    let requested = requested.filter(|value| !value.trim().is_empty());
    match (mode, requested) {
        (_, None) => Ok(None),
        (OverrideMode::Allow, Some(value)) => {
            applied.push(field);
            Ok(Some(value.to_owned()))
        }
        (OverrideMode::Ignore, Some(_)) => {
            ignored.push(field);
            Ok(None)
        }
        (OverrideMode::Reject, Some(_)) => Err(GovernanceError::Rejected(field)),
    }
}

const fn allow() -> OverrideMode {
    OverrideMode::Allow
}

const fn schema_version() -> u32 {
    MOMO_CONFIG_SCHEMA_VERSION
}

fn default_visual_description_prompt() -> String {
    "Describe only visible facts that are relevant to the conversation. Do not infer identity, intent, private attributes, or text that is not legible.".to_owned()
}

#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("MOMO configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("MOMO configuration TOML is invalid: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("unsupported momo.toml schema_version {0}")]
    UnsupportedSchema(u32),
    #[error("host-local field {0:?} belongs in config.toml, not momo.toml")]
    HostField(String),
    #[error("request override {0} is rejected by policy")]
    Rejected(&'static str),
    #[error("request parameter {0:?} is not allowed by policy")]
    ParameterNotAllowed(String),
    #[error("request override {field}={value} must be between {min} and {max}")]
    OutOfRange {
        field: &'static str,
        value: usize,
        min: usize,
        max: usize,
    },
    #[error("invalid MOMO configuration: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_ignored_and_allowed_overrides_are_distinct() {
        let policy = RequestOverridePolicy {
            context_window: OverrideMode::Ignore,
            max_output_tokens: OverrideMode::Allow,
            sampling: OverrideMode::Reject,
            instructions: OverrideMode::Allow,
            visual_description_prompt: OverrideMode::Ignore,
            tools: OverrideMode::Allow,
            allowed_parameters: BTreeSet::from(["seed".to_owned()]),
        };
        let parameters = Map::from_iter([("seed".to_owned(), json!(42))]);
        let effective = policy
            .apply(
                8_192,
                1_024,
                RequestedOverrides {
                    context_window: Some(4_096),
                    max_output_tokens: Some(512),
                    temperature: None,
                    instructions: Some("request instructions"),
                    visual_description_prompt: Some("request vision prompt"),
                    parameters: &parameters,
                    tool_configuration_requested: false,
                },
            )
            .expect("governed request");
        assert_eq!(effective.context_window, 8_192);
        assert_eq!(effective.max_output_tokens, 512);
        assert_eq!(
            effective.instructions.as_deref(),
            Some("request instructions")
        );
        assert!(effective.visual_description_prompt.is_none());
        assert_eq!(effective.parameters["seed"], 42);
    }

    #[test]
    fn full_portable_document_keeps_unowned_sections_and_uses_momo_vision_prompt() {
        let document = r#"
schema_version = 1

[model_use.chat]
route = "primary"

[vision]
enabled = true
prompt = "Describe the visible scene."
"#;
        let config: MomoConfig = toml::from_str(document).expect("portable config");
        config.validate().expect("valid config");
        let parameters = Map::new();
        let effective = config
            .govern(
                8_192,
                1_024,
                RequestedOverrides {
                    context_window: None,
                    max_output_tokens: None,
                    temperature: None,
                    instructions: None,
                    visual_description_prompt: None,
                    parameters: &parameters,
                    tool_configuration_requested: false,
                },
            )
            .expect("governed request");
        assert_eq!(
            effective.visual_description_prompt.as_deref(),
            Some("Describe the visible scene.")
        );
        assert_eq!(effective.audit["visual_description_prompt_source"], "momo");
    }

    #[test]
    fn rejects_host_adapter_wiring_in_portable_document() {
        let error =
            validate_momo_document("schema_version = 1\n[[providers]]\nprovider_id = 'unsafe'\n")
                .expect_err("host field");
        assert!(matches!(error, GovernanceError::HostField(field) if field == "providers"));
    }
}
