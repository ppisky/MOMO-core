//! Typed process-wide runtime settings and request governance.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

pub const RUNTIME_SETTINGS_SCHEMA_VERSION: u32 = 1;

mod ddm;

pub use ddm::DdmRuntimeConfig;

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisionDescriptionConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoleplayRuntimeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for RoleplayRuntimeConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceRuntimeSettings {
    #[serde(default = "default_true")]
    pub memory_distillation_enabled: bool,
    #[serde(default = "default_maintenance_turns")]
    pub memory_distill_every_turns: usize,
    #[serde(default = "default_true")]
    pub semantic_graph_enabled: bool,
    #[serde(default = "default_maintenance_turns")]
    pub nsg_govern_every_turns: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoStateProfile {
    #[default]
    ClosedAutonomous,
    V1Projection,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoStateInjectionMode {
    #[default]
    Active,
    Shadow,
}

impl MoStateInjectionMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Shadow => "shadow",
        }
    }
}

impl MoStateProfile {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClosedAutonomous => "closed_autonomous",
            Self::V1Projection => "v1_projection",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MoStateRuntimeConfig {
    #[serde(default)]
    pub profile: MoStateProfile,
    #[serde(default = "default_true")]
    pub scene_management: bool,
    #[serde(default = "default_max_reconcile_steps")]
    pub max_reconcile_steps: usize,
    #[serde(default = "default_max_agent_steps")]
    pub max_agent_steps: usize,
    #[serde(default = "default_mo_state_operation_timeout_ms")]
    pub operation_timeout_ms: u64,
    #[serde(default)]
    pub injection_mode: MoStateInjectionMode,
    #[serde(default)]
    pub ddm: DdmRuntimeConfig,
}

impl Default for MoStateRuntimeConfig {
    fn default() -> Self {
        Self {
            profile: MoStateProfile::ClosedAutonomous,
            scene_management: true,
            max_reconcile_steps: default_max_reconcile_steps(),
            max_agent_steps: default_max_agent_steps(),
            operation_timeout_ms: default_mo_state_operation_timeout_ms(),
            injection_mode: MoStateInjectionMode::Active,
            ddm: DdmRuntimeConfig::default(),
        }
    }
}

impl Default for MaintenanceRuntimeSettings {
    fn default() -> Self {
        Self {
            memory_distillation_enabled: true,
            memory_distill_every_turns: default_maintenance_turns(),
            semantic_graph_enabled: true,
            nsg_govern_every_turns: default_maintenance_turns(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MomoRuntimeSettings {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub request_overrides: RequestOverridePolicy,
    #[serde(default)]
    pub runtime: MaintenanceRuntimeSettings,
    #[serde(default)]
    pub mo_state: MoStateRuntimeConfig,
    #[serde(default)]
    pub roleplay: RoleplayRuntimeConfig,
    #[serde(default)]
    pub vision: VisionDescriptionConfig,
}

impl Default for MomoRuntimeSettings {
    fn default() -> Self {
        Self {
            schema_version: RUNTIME_SETTINGS_SCHEMA_VERSION,
            request_overrides: RequestOverridePolicy::default(),
            runtime: MaintenanceRuntimeSettings::default(),
            mo_state: MoStateRuntimeConfig::default(),
            roleplay: RoleplayRuntimeConfig::default(),
            vision: VisionDescriptionConfig::default(),
        }
    }
}

impl MomoRuntimeSettings {
    pub fn validate(&self) -> Result<(), GovernanceError> {
        self.validate_portable_fields()?;
        Ok(())
    }

    fn validate_portable_fields(&self) -> Result<(), GovernanceError> {
        if self.schema_version != RUNTIME_SETTINGS_SCHEMA_VERSION {
            return Err(GovernanceError::UnsupportedSchema(self.schema_version));
        }
        if !(1..=200).contains(&self.runtime.memory_distill_every_turns)
            || !(1..=200).contains(&self.runtime.nsg_govern_every_turns)
        {
            return Err(GovernanceError::Invalid(
                "runtime maintenance intervals must be between 1 and 200 turns".to_owned(),
            ));
        }
        if !(1..=16).contains(&self.mo_state.max_reconcile_steps)
            || !(1..=32).contains(&self.mo_state.max_agent_steps)
            || !(1_000..=300_000).contains(&self.mo_state.operation_timeout_ms)
        {
            return Err(GovernanceError::Invalid(
                "MO State limits must use 1..=16 reconcile steps, 1..=32 agent steps, and a 1000..=300000 ms timeout"
                    .to_owned(),
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
            "prompt_space"
        };
        governed.audit["visual_description_prompt_source"] = json!(visual_source);
        Ok(governed)
    }
}

const fn default_true() -> bool {
    true
}

const fn default_maintenance_turns() -> usize {
    12
}

const fn default_max_reconcile_steps() -> usize {
    4
}

const fn default_max_agent_steps() -> usize {
    8
}

const fn default_mo_state_operation_timeout_ms() -> u64 {
    30_000
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
    RUNTIME_SETTINGS_SCHEMA_VERSION
}

#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("unsupported runtime settings schema_version {0}")]
    UnsupportedSchema(u32),
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
#[path = "../tests/unit/governance.rs"]
mod tests;
