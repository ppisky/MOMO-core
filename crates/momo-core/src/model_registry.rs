//! Outbound connection contracts and protocol adapters; storage belongs to the host.
use crate::GovernanceError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
mod protocol;
mod transport;
mod vision;
pub(crate) use vision::ConfiguredVisionAdapter;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    #[default]
    Openai,
    OpenaiResponses,
    Anthropic,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Chat,
    Vision,
    Embedding,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProvider {
    pub id: uuid::Uuid,
    pub name: String,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    pub base_url: String,
    #[serde(default)]
    pub api_path: String,
    #[serde(default)]
    pub api_key: Option<String>,
}
impl std::fmt::Debug for ModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelProvider")
            .field("id", &self.id)
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredModel {
    pub id: uuid::Uuid,
    pub provider_id: uuid::Uuid,
    pub model_id: String,
    pub name: String,
    pub kind: ModelKind,
    #[serde(default)]
    pub context_window: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
}
impl ConfiguredModel {
    /// A protective budget for undeclared capabilities, never a persisted declaration.
    pub fn effective_context_window(&self) -> usize {
        self.context_window
            .unwrap_or_else(|| 8192.max(self.max_output_tokens.unwrap_or(1024).saturating_add(1)))
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfiguration {
    pub revision: u64,
    pub providers: Vec<ModelProvider>,
    pub models: Vec<ConfiguredModel>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConnection {
    pub provider: ModelProvider,
    pub model: ConfiguredModel,
}

impl ModelConnection {
    pub(crate) fn effective_protocol(&self) -> ProviderProtocol {
        if self.provider.protocol == ProviderProtocol::Openai {
            let path = if self.provider.api_path.is_empty() {
                url::Url::parse(&self.provider.base_url)
                    .ok()
                    .map(|url| url.path().to_owned())
                    .unwrap_or_default()
            } else {
                self.provider.api_path.clone()
            };
            if path.trim_end_matches('/').ends_with("/responses")
                || path.trim_matches('/') == "responses"
            {
                return ProviderProtocol::OpenaiResponses;
            }
        }
        self.provider.protocol
    }
}

/// Host-owned connection lookup. Resolvers must retain immutable references for replay.
/// Core does not prescribe a configuration file, credential store or account catalog.
pub trait ModelConnectionResolver: std::fmt::Debug + Send + Sync {
    fn resolve(&self, reference: &str) -> Result<Option<ModelConnection>, String>;
}

impl ModelConfiguration {
    /// Validate connection contracts before the host stores or injects them.
    pub fn validate(&self) -> Result<(), GovernanceError> {
        validate(self)
    }
}
fn invalid(message: impl ToString) -> GovernanceError {
    GovernanceError::Invalid(message.to_string())
}
fn validate(c: &ModelConfiguration) -> Result<(), GovernanceError> {
    if c.providers.len() > 64 || c.models.len() > 1024 {
        return Err(invalid("too many model connections"));
    }
    let mut ids = BTreeSet::new();
    for p in &c.providers {
        if !ids.insert(p.id) || p.name.trim().is_empty() || p.name.len() > 200 {
            return Err(invalid("invalid or duplicate provider"));
        }
        let url = url::Url::parse(&p.base_url).map_err(|_| invalid("invalid provider URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "provider URL must be an HTTP(S) endpoint without credentials or query",
            ));
        }
        if p.api_path.len() > 300
            || p.api_path.contains(['?', '#', '\\'])
            || p.api_path.split('/').any(|s| s == "..")
            || p.api_path.contains("://")
        {
            return Err(invalid("invalid API path"));
        }
        if p.api_key
            .as_ref()
            .is_some_and(|key| key.len() > 8192 || key.contains(['\r', '\n']))
        {
            return Err(invalid("invalid API key"));
        }
    }
    let mut models = BTreeSet::new();
    for m in &c.models {
        if !models.insert(m.id)
            || !ids.contains(&m.provider_id)
            || m.model_id.trim().is_empty()
            || m.model_id.len() > 256
            || m.model_id.chars().any(char::is_control)
            || m.name.trim().is_empty()
            || m.name.len() > 200
            || m.context_window
                .is_some_and(|n| !(1024..=16_000_000).contains(&n))
            || m.max_output_tokens.is_some_and(|n| {
                n == 0 || n >= 16_000_000 || m.context_window.is_some_and(|context| n >= context)
            })
        {
            return Err(invalid("invalid model definition"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/model_registry.rs"]
mod tests;
