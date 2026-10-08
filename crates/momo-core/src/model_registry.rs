//! Host-configured model connections. Secrets never appear in the read resource.
use crate::{GovernanceError, MomoRuntime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
};
mod protocol;
mod transport;
mod vision;
pub(crate) use vision::ConfiguredVisionAdapter;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    Openai,
    OpenaiResponses,
    Anthropic,
    Gemini,
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
    #[serde(default = "default_context")]
    pub context_window: usize,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
}
fn default_context() -> usize {
    8192
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfiguration {
    pub revision: u64,
    pub providers: Vec<ModelProvider>,
    pub models: Vec<ConfiguredModel>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ModelConnection {
    pub provider: ModelProvider,
    pub model: ConfiguredModel,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    current: ModelConfiguration,
    snapshots: BTreeMap<String, ModelConnection>,
}
#[derive(Debug)]
pub(crate) struct ModelRegistry {
    path: PathBuf,
    stored: Stored,
}
fn invalid(message: impl ToString) -> GovernanceError {
    GovernanceError::Invalid(message.to_string())
}
fn reference(connection: &ModelConnection) -> String {
    format!(
        "configured_{}",
        hex::encode(Sha256::digest(
            serde_json::to_vec(connection).expect("model connection is serializable")
        ))
    )
}
impl ModelRegistry {
    pub(crate) fn load(path: PathBuf) -> Result<Self, GovernanceError> {
        let stored: Stored = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 32 * 1024 * 1024 {
                    return Err(invalid("model configuration exceeds limit"));
                }
                serde_json::from_slice(&bytes).map_err(invalid)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(GovernanceError::Persistence(e.to_string())),
        };
        validate(&stored.current)?;
        if stored.snapshots.len() > 4096 {
            return Err(invalid("model snapshot history exceeds limit"));
        }
        for (id, snapshot) in &stored.snapshots {
            validate(&ModelConfiguration {
                revision: 0,
                providers: vec![snapshot.provider.clone()],
                models: vec![snapshot.model.clone()],
            })?;
            if reference(snapshot) != *id {
                return Err(invalid("model snapshot identity mismatch"));
            }
        }
        Ok(Self { path, stored })
    }
    fn public(&self) -> Value {
        let c = &self.stored.current;
        json!({"revision":c.revision,"providers":c.providers.iter().map(|p|json!({"id":p.id,"name":p.name,"protocol":p.protocol,"base_url":p.base_url,"api_path":p.api_path,"has_api_key":p.api_key.as_ref().is_some_and(|k|!k.is_empty())})).collect::<Vec<_>>(),"models":c.models.iter().map(|m|{let p=c.providers.iter().find(|p|p.id==m.provider_id).expect("validated provider");let mut v=serde_json::to_value(m).expect("serializable model");v["reference"]=json!(reference(&ModelConnection{provider:p.clone(),model:m.clone()}));v}).collect::<Vec<_>>()})
    }
    fn update(&mut self, mut c: ModelConfiguration) -> Result<Value, GovernanceError> {
        if c.revision != self.stored.current.revision {
            return Err(GovernanceError::Conflict {
                expected: c.revision,
                actual: self.stored.current.revision,
            });
        }
        for p in &mut c.providers {
            if p.api_key.is_none() {
                p.api_key = self
                    .stored
                    .current
                    .providers
                    .iter()
                    .find(|old| old.id == p.id)
                    .and_then(|old| old.api_key.clone());
            }
        }
        validate(&c)?;
        c.revision = c
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("model revision exhausted"))?;
        let mut next = self.stored.clone();
        next.current = c;
        for m in &next.current.models {
            let provider = next
                .current
                .providers
                .iter()
                .find(|p| p.id == m.provider_id)
                .expect("validated")
                .clone();
            let connection = ModelConnection {
                provider,
                model: m.clone(),
            };
            next.snapshots.insert(reference(&connection), connection);
        }
        let bytes = serde_json::to_vec(&next).map_err(invalid)?;
        if bytes.len() > 32 * 1024 * 1024 || next.snapshots.len() > 4096 {
            return Err(invalid("model snapshot history exceeds limit"));
        }
        let mut temp =
            tempfile::NamedTempFile::new_in(self.path.parent().expect("model directory"))
                .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
        temp.write_all(&bytes)
            .and_then(|()| temp.as_file().sync_all())
            .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
        temp.persist(&self.path)
            .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
        self.stored = next;
        Ok(self.public())
    }
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
            || !(1024..=16_000_000).contains(&m.context_window)
            || m.max_output_tokens
                .is_some_and(|n| n == 0 || n >= m.context_window)
        {
            return Err(invalid("invalid model definition"));
        }
        if c.providers
            .iter()
            .any(|p| p.id == m.provider_id && p.protocol == ProviderProtocol::Gemini)
        {
            let name = m.model_id.strip_prefix("models/").unwrap_or(&m.model_id);
            if name.is_empty()
                || name.contains(['/', '\\', '?', '#', '%'])
                || matches!(name, "." | "..")
            {
                return Err(invalid("invalid Gemini model name"));
            }
        }
    }
    Ok(())
}
impl MomoRuntime {
    pub async fn discover_models(&self, provider_id: uuid::Uuid) -> Result<Value, GovernanceError> {
        use futures_util::StreamExt;
        let provider = self
            .models
            .call(move |m| {
                m.stored
                    .current
                    .providers
                    .iter()
                    .find(|p| p.id == provider_id)
                    .cloned()
            })
            .ok_or_else(|| invalid("provider does not exist"))?;
        let mut url = url::Url::parse(&provider.base_url).map_err(invalid)?;
        let prefix = url.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{prefix}/models"));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(invalid)?;
        let mut request = client.get(url);
        if let Some(key) = provider.api_key.filter(|k| !k.is_empty()) {
            request = match provider.protocol {
                ProviderProtocol::Anthropic => request.header("x-api-key", key),
                ProviderProtocol::Gemini => request.header("x-goog-api-key", key),
                _ => request.bearer_auth(key),
            };
        }
        if provider.protocol == ProviderProtocol::Anthropic {
            request = request.header("anthropic-version", "2023-06-01");
        }
        let response = request
            .send()
            .await
            .map_err(|_| invalid("could not reach model endpoint"))?;
        if !response.status().is_success() {
            return Err(invalid(format!(
                "model listing returned HTTP {}",
                response.status().as_u16()
            )));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| invalid("model listing interrupted"))?;
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(invalid("model listing exceeds limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid model listing"))?;
        let models = value
            .get("data")
            .or_else(|| value.get("models"))
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("model listing has no models"))?;
        let mut result = BTreeMap::new();
        for model in models.iter().take(1024) {
            let Some(id) = model
                .get("id")
                .or_else(|| model.get("name"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let id = id.trim_start_matches("models/");
            if id.is_empty() || id.len() > 256 {
                continue;
            }
            let name = model
                .get("displayName")
                .or_else(|| model.get("display_name"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            result.insert(id.to_owned(), json!({"model_id":id,"name":name}));
        }
        Ok(
            json!({"models":result.into_values().collect::<Vec<_>>(),"has_more":value.get("nextPageToken").is_some()||value["has_more"]==true||models.len()>1024}),
        )
    }
    pub fn model_configuration(&self) -> Value {
        self.models.call(|m| m.public())
    }
    pub fn update_model_configuration(
        &self,
        value: ModelConfiguration,
    ) -> Result<Value, GovernanceError> {
        self.models.call(move |m| m.update(value))
    }
    pub(crate) fn model_connection(
        &self,
        reference: &str,
    ) -> Result<Option<ModelConnection>, String> {
        if !reference.starts_with("configured_") {
            return Ok(None);
        }
        let id = reference.to_owned();
        self.models.call(move |m| {
            m.stored
                .snapshots
                .get(&id)
                .cloned()
                .ok_or_else(|| "configured model reference does not exist".to_owned())
                .map(Some)
        })
    }
}

#[cfg(test)]
#[path = "../tests/unit/model_registry.rs"]
mod tests;
