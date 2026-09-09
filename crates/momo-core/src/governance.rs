//! Portable request-governance and visual-description configuration.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

pub const MOMO_CONFIG_SCHEMA_VERSION: u32 = 1;
const MAX_MAINTENANCE_PROMPT_BYTES: u64 = 256 * 1024;
const DEFAULT_MEMORY_PROMPT_FILE: &str = "prompts/dmw_distiller.md";
const DEFAULT_NSG_PROMPT_FILE: &str = "prompts/nsg_governor.md";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MaintenancePromptConfig {
    pub memory_distillation_file: PathBuf,
    pub semantic_graph_governance_file: PathBuf,
    /// Optional external override. Configurations created before the role-play
    /// director existed use the embedded audited default.
    #[serde(default)]
    pub roleplay_director_file: Option<PathBuf>,
    #[serde(skip)]
    pub memory_distillation: String,
    #[serde(skip)]
    pub semantic_graph_governance: String,
    #[serde(skip, default = "default_roleplay_director_prompt")]
    pub roleplay_director: String,
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
pub struct MomoRuntimeConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        }
    }
}

impl Default for MomoRuntimeConfig {
    fn default() -> Self {
        Self {
            memory_distillation_enabled: true,
            memory_distill_every_turns: default_maintenance_turns(),
            semantic_graph_enabled: true,
            nsg_govern_every_turns: default_maintenance_turns(),
        }
    }
}

impl Default for VisionDescriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prompt: default_visual_description_prompt(),
        }
    }
}

impl Default for MaintenancePromptConfig {
    fn default() -> Self {
        Self {
            memory_distillation_file: PathBuf::from(DEFAULT_MEMORY_PROMPT_FILE),
            semantic_graph_governance_file: PathBuf::from(DEFAULT_NSG_PROMPT_FILE),
            roleplay_director_file: None,
            memory_distillation: include_str!("../../../prompts/dmw_distiller.md").to_owned(),
            semantic_graph_governance: include_str!("../../../prompts/nsg_governor.md").to_owned(),
            roleplay_director: include_str!("../../../prompts/roleplay_director.md").to_owned(),
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
    pub runtime: MomoRuntimeConfig,
    #[serde(default)]
    pub mo_state: MoStateRuntimeConfig,
    #[serde(default)]
    pub roleplay: RoleplayRuntimeConfig,
    #[serde(default)]
    pub vision: VisionDescriptionConfig,
    #[serde(default)]
    pub prompts: MaintenancePromptConfig,
}

impl Default for MomoConfig {
    fn default() -> Self {
        Self {
            schema_version: MOMO_CONFIG_SCHEMA_VERSION,
            request_overrides: RequestOverridePolicy::default(),
            runtime: MomoRuntimeConfig::default(),
            mo_state: MoStateRuntimeConfig::default(),
            roleplay: RoleplayRuntimeConfig::default(),
            vision: VisionDescriptionConfig::default(),
            prompts: MaintenancePromptConfig::default(),
        }
    }
}

impl MomoConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GovernanceError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)?;
        validate_document_ownership(&text)?;
        let mut config: Self = toml::from_str(&text)?;
        config.load_maintenance_prompts(path)?;
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
        self.validate_portable_fields()?;
        for (name, prompt) in [
            (
                "prompts.memory_distillation_file",
                &self.prompts.memory_distillation,
            ),
            (
                "prompts.semantic_graph_governance_file",
                &self.prompts.semantic_graph_governance,
            ),
            (
                "prompts.roleplay_director_file",
                &self.prompts.roleplay_director,
            ),
        ] {
            if prompt.trim().is_empty()
                || u64::try_from(prompt.len()).unwrap_or(u64::MAX) > MAX_MAINTENANCE_PROMPT_BYTES
            {
                return Err(GovernanceError::Invalid(format!(
                    "the file referenced by {name} must contain 1 to {MAX_MAINTENANCE_PROMPT_BYTES} UTF-8 bytes"
                )));
            }
        }
        Ok(())
    }

    fn validate_portable_fields(&self) -> Result<(), GovernanceError> {
        if self.schema_version != MOMO_CONFIG_SCHEMA_VERSION {
            return Err(GovernanceError::UnsupportedSchema(self.schema_version));
        }
        if self.vision.prompt.trim().is_empty() || self.vision.prompt.len() > 64 * 1024 {
            return Err(GovernanceError::Invalid(
                "vision.prompt must contain 1 to 65536 bytes".to_owned(),
            ));
        }
        for (name, path) in [
            (
                "prompts.memory_distillation_file",
                &self.prompts.memory_distillation_file,
            ),
            (
                "prompts.semantic_graph_governance_file",
                &self.prompts.semantic_graph_governance_file,
            ),
        ] {
            validate_prompt_reference(name, path)?;
        }
        if let Some(path) = self.prompts.roleplay_director_file.as_deref() {
            validate_prompt_reference("prompts.roleplay_director_file", path)?;
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

    fn load_maintenance_prompts(&mut self, config_path: &Path) -> Result<(), GovernanceError> {
        self.validate_portable_fields()?;
        self.prompts.memory_distillation = read_prompt_file(
            config_path,
            "prompts.memory_distillation_file",
            &self.prompts.memory_distillation_file,
        )?;
        self.prompts.semantic_graph_governance = read_prompt_file(
            config_path,
            "prompts.semantic_graph_governance_file",
            &self.prompts.semantic_graph_governance_file,
        )?;
        self.prompts.roleplay_director =
            if let Some(path) = self.prompts.roleplay_director_file.as_deref() {
                read_prompt_file(config_path, "prompts.roleplay_director_file", path)?
            } else {
                include_str!("../../../prompts/roleplay_director.md").to_owned()
            };
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

pub fn validate_momo_document(text: &str) -> Result<(), GovernanceError> {
    validate_document_ownership(text)?;
    let config: MomoConfig = toml::from_str(text)?;
    config.validate_portable_fields()
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

fn default_roleplay_director_prompt() -> String {
    include_str!("../../../prompts/roleplay_director.md").to_owned()
}

fn validate_prompt_reference(name: &str, path: &Path) -> Result<(), GovernanceError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        return Err(GovernanceError::Invalid(format!(
            "{name} must be a safe relative .md path beneath momo.toml"
        )));
    }
    Ok(())
}

fn read_prompt_file(
    config_path: &Path,
    name: &str,
    relative: &Path,
) -> Result<String, GovernanceError> {
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let canonical_base = fs::canonicalize(base)?;
    let path = base.join(relative);
    let canonical_path = fs::canonicalize(&path)?;
    if !canonical_path.starts_with(&canonical_base) {
        return Err(GovernanceError::Invalid(format!(
            "{name} resolves outside the momo.toml directory"
        )));
    }
    let metadata = fs::metadata(&canonical_path)?;
    if !metadata.is_file() || metadata.len() > MAX_MAINTENANCE_PROMPT_BYTES {
        return Err(GovernanceError::Invalid(format!(
            "the file referenced by {name} must be a regular file no larger than {MAX_MAINTENANCE_PROMPT_BYTES} bytes"
        )));
    }
    let prompt = fs::read_to_string(&canonical_path)?;
    if prompt.trim().is_empty() || prompt.as_bytes().contains(&0) {
        return Err(GovernanceError::Invalid(format!(
            "the file referenced by {name} must be non-empty UTF-8 text without NUL bytes"
        )));
    }
    Ok(prompt)
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
    fn default_distiller_preserves_confirmed_rule_outcomes_in_dmw() {
        let config = MomoConfig::default();
        assert!(
            config
                .prompts
                .memory_distillation
                .contains("preserve the concrete outcome in DMW")
        );
        assert!(config.roleplay.enabled);
        assert!(
            config
                .prompts
                .roleplay_director
                .contains("performer of the character")
        );
        assert!(config.prompts.roleplay_director_file.is_none());
    }

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

[prompts]
memory_distillation_file = "prompts/dmw_distiller.md"
semantic_graph_governance_file = "prompts/nsg_governor.md"
"#;
        let mut config: MomoConfig = toml::from_str(document).expect("portable config");
        config.prompts.memory_distillation = "full memory prompt".to_owned();
        config.prompts.semantic_graph_governance = "full graph prompt".to_owned();
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

    #[test]
    fn maintenance_runtime_is_portable_and_bounded() {
        let config: MomoConfig = toml::from_str(
            r#"
schema_version = 1

[runtime]
memory_distillation_enabled = false
memory_distill_every_turns = 7
semantic_graph_enabled = true
nsg_govern_every_turns = 19
max_concurrent_chats = 4

[mo_state]
profile = "closed_autonomous"
scene_management = true
max_reconcile_steps = 4
max_agent_steps = 8
operation_timeout_ms = 30000
injection_mode = "shadow"

[prompts]
memory_distillation_file = "prompts/dmw_distiller.md"
semantic_graph_governance_file = "prompts/nsg_governor.md"
"#,
        )
        .expect("portable runtime");
        let mut config = config;
        config.prompts.memory_distillation = "full memory prompt".to_owned();
        config.prompts.semantic_graph_governance = "full graph prompt".to_owned();
        config.validate().expect("valid runtime");
        assert!(!config.runtime.memory_distillation_enabled);
        assert_eq!(config.runtime.memory_distill_every_turns, 7);
        assert!(config.runtime.semantic_graph_enabled);
        assert_eq!(config.runtime.nsg_govern_every_turns, 19);
        assert_eq!(config.mo_state.profile, MoStateProfile::ClosedAutonomous);
        assert!(config.mo_state.scene_management);
        assert_eq!(config.mo_state.injection_mode, MoStateInjectionMode::Shadow);

        let mut invalid = config;
        invalid.runtime.nsg_govern_every_turns = 0;
        assert!(matches!(
            invalid.validate(),
            Err(GovernanceError::Invalid(_))
        ));
    }

    #[test]
    fn omitted_prompt_table_uses_standard_file_references() {
        let config: MomoConfig = toml::from_str("schema_version = 1\n").expect("portable config");
        assert_eq!(
            config.prompts.memory_distillation_file,
            PathBuf::from(DEFAULT_MEMORY_PROMPT_FILE)
        );
        assert_eq!(
            config.prompts.semantic_graph_governance_file,
            PathBuf::from(DEFAULT_NSG_PROMPT_FILE)
        );
        assert!(config.roleplay.enabled);
        assert!(config.prompts.roleplay_director_file.is_none());
        assert!(
            config
                .prompts
                .roleplay_director
                .contains("Stay inside the fiction")
        );
    }

    #[test]
    fn loads_safe_relative_prompt_files_and_rejects_traversal() {
        let directory = tempfile::tempdir().expect("directory");
        fs::create_dir(directory.path().join("prompts")).expect("prompt directory");
        fs::write(
            directory.path().join(DEFAULT_MEMORY_PROMPT_FILE),
            "memory rules",
        )
        .expect("memory prompt");
        fs::write(
            directory.path().join(DEFAULT_NSG_PROMPT_FILE),
            "graph rules",
        )
        .expect("graph prompt");
        let path = directory.path().join("momo.toml");
        fs::write(
            &path,
            "schema_version = 1\n[prompts]\nmemory_distillation_file = 'prompts/dmw_distiller.md'\nsemantic_graph_governance_file = 'prompts/nsg_governor.md'\n",
        )
        .expect("config");
        let config = MomoConfig::load(&path).expect("load references");
        assert_eq!(config.prompts.memory_distillation, "memory rules");
        assert_eq!(config.prompts.semantic_graph_governance, "graph rules");

        let unsafe_document = "schema_version = 1\n[prompts]\nmemory_distillation_file = '../outside.md'\nsemantic_graph_governance_file = 'prompts/nsg_governor.md'\n";
        assert!(matches!(
            validate_momo_document(unsafe_document),
            Err(GovernanceError::Invalid(_))
        ));
    }
}
