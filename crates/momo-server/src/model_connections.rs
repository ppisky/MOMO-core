//! HTTP-host model accounts and durable connection references, outside MomoRuntime.
use momo_core::{
    GovernanceError, ModelConfiguration, ModelConnection, ModelConnectionResolver, ProviderProtocol,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, path::PathBuf, sync::Mutex};

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
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|e| GovernanceError::Persistence(e.to_string()))?
                .join(path)
        };
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
        stored.current.validate()?;
        if stored.snapshots.len() > 4096 {
            return Err(invalid("model snapshot history exceeds limit"));
        }
        for (id, snapshot) in &stored.snapshots {
            ModelConfiguration {
                revision: 0,
                providers: vec![snapshot.provider.clone()],
                models: vec![snapshot.model.clone()],
            }
            .validate()?;
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
        c.validate()?;
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
        std::fs::create_dir_all(self.path.parent().expect("model directory"))
            .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
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

#[derive(Debug)]
pub(crate) struct ModelConnections(Mutex<ModelRegistry>);
impl ModelConnections {
    pub(crate) fn load(path: PathBuf) -> Result<Self, GovernanceError> {
        Ok(Self(Mutex::new(ModelRegistry::load(path)?)))
    }
    fn registry(&self) -> Result<std::sync::MutexGuard<'_, ModelRegistry>, GovernanceError> {
        self.0
            .lock()
            .map_err(|_| GovernanceError::Persistence("host model registry unavailable".into()))
    }
    pub(crate) fn public(&self) -> Result<Value, GovernanceError> {
        Ok(self.registry()?.public())
    }
    pub(crate) fn update(&self, value: ModelConfiguration) -> Result<Value, GovernanceError> {
        self.registry()?.update(value)
    }
    pub(crate) async fn discover(&self, provider_id: uuid::Uuid) -> Result<Value, GovernanceError> {
        let provider = self
            .registry()?
            .stored
            .current
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .cloned()
            .ok_or_else(|| invalid("provider does not exist"))?;
        let mut url = reqwest::Url::parse(&provider.base_url).map_err(invalid)?;
        let path = url.path().trim_end_matches('/');
        let prefix = ["/responses", "/chat/completions"]
            .into_iter()
            .find_map(|suffix| path.strip_suffix(suffix))
            .unwrap_or(path)
            .to_owned();
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
                _ => request.bearer_auth(key),
            };
        }
        if provider.protocol == ProviderProtocol::Anthropic {
            request = request.header("anthropic-version", "2023-06-01");
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| invalid("could not reach model endpoint"))?;
        if !response.status().is_success() {
            return Err(invalid(format!(
                "model listing returned HTTP {}",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| invalid("model listing interrupted"))?
        {
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
}
impl ModelConnectionResolver for ModelConnections {
    fn resolve(&self, reference: &str) -> Result<Option<ModelConnection>, String> {
        Ok(self
            .registry()
            .map_err(|e| e.to_string())?
            .stored
            .snapshots
            .get(reference)
            .cloned())
    }
}

#[cfg(test)]
#[path = "../tests/unit/model_connections.rs"]
mod tests;
