use std::collections::{BTreeMap, BTreeSet};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tiktoken_rs::{
    bpe_for_model, cl100k_base_singleton, o200k_base_singleton, p50k_base_singleton,
};

use crate::estimate_text_tokens;

const CAPABILITY_SCHEMA_VERSION: u32 = 1;
const MAX_DISCOVERY_BYTES: usize = 256 * 1024;
pub const MAX_DISCOVERY_TTL_SECONDS: u64 = 24 * 60 * 60;
const RESERVED_PARAMETERS: [&str; 4] = ["model", "messages", "temperature", "stream"];

/// Tokenizer selected by an endpoint capability profile.  Explicit encodings
/// are stable across provider model aliases; `Model` follows tiktoken's audited
/// OpenAI model mapping.  Unknown third-party tokenizers must use the clearly
/// labelled conservative fallback rather than pretending to be exact.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "model", rename_all = "snake_case")]
pub enum TokenizerProfile {
    #[default]
    Conservative,
    Cl100kBase,
    O200kBase,
    P50kBase,
    Model(String),
}

impl TokenizerProfile {
    pub fn count(&self, text: &str) -> Result<usize, CapabilityError> {
        let tokens = match self {
            Self::Conservative => return Ok(estimate_text_tokens(text)),
            Self::Cl100kBase => cl100k_base_singleton().encode_ordinary(text),
            Self::O200kBase => o200k_base_singleton().encode_ordinary(text),
            Self::P50kBase => p50k_base_singleton().encode_ordinary(text),
            Self::Model(model) => bpe_for_model(model)
                .map_err(|_| CapabilityError::UnknownTokenizerModel(model.clone()))?
                .encode_ordinary(text),
        };
        Ok(tokens.len())
    }

    /// Counts without making an unsafe compatibility claim. A model mapping
    /// failure deliberately falls back to MOMO's conservative estimator.
    #[must_use]
    pub fn count_or_conservative(&self, text: &str) -> usize {
        self.count(text)
            .unwrap_or_else(|_| estimate_text_tokens(text))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityProfile {
    #[serde(default = "capability_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub tokenizer: TokenizerProfile,
    pub context_window: usize,
    pub max_output_tokens: usize,
    #[serde(default = "enabled")]
    pub streaming: bool,
    #[serde(default = "default_parameters")]
    pub parameters: BTreeSet<String>,
    #[serde(default)]
    pub allow_unknown_parameters: bool,
}

impl CapabilityProfile {
    pub fn validate(&self) -> Result<(), CapabilityError> {
        if self.schema_version != CAPABILITY_SCHEMA_VERSION {
            return Err(CapabilityError::UnsupportedSchema(self.schema_version));
        }
        if self.context_window < 256 {
            return Err(CapabilityError::InvalidContextWindow);
        }
        if self.max_output_tokens == 0 || self.max_output_tokens >= self.context_window {
            return Err(CapabilityError::InvalidOutputLimit);
        }
        if self
            .parameters
            .iter()
            .any(|name| name.trim().is_empty() || RESERVED_PARAMETERS.contains(&name.as_str()))
        {
            return Err(CapabilityError::InvalidParameterName);
        }
        Ok(())
    }

    pub fn validate_request_parameters<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), CapabilityError> {
        self.validate()?;
        for name in names {
            if RESERVED_PARAMETERS.contains(&name) {
                return Err(CapabilityError::ReservedParameter(name.to_owned()));
            }
            if !self.allow_unknown_parameters && !self.parameters.contains(name) {
                return Err(CapabilityError::UnsupportedParameter(name.to_owned()));
            }
        }
        Ok(())
    }
}

impl Default for CapabilityProfile {
    fn default() -> Self {
        Self {
            schema_version: CAPABILITY_SCHEMA_VERSION,
            tokenizer: TokenizerProfile::Conservative,
            context_window: 8_192,
            max_output_tokens: 1_024,
            streaming: true,
            parameters: default_parameters(),
            allow_unknown_parameters: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityDiscoveryDocument {
    pub schema_version: u32,
    #[serde(default = "default_discovery_ttl")]
    pub ttl_seconds: u64,
    pub models: BTreeMap<String, CapabilityProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilitySource {
    CachedDiscovery,
    ConservativeFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCapability {
    pub profile: CapabilityProfile,
    pub source: CapabilitySource,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    providers: BTreeMap<String, CachedProviderCapabilities>,
}

#[derive(Debug, Clone)]
struct CachedProviderCapabilities {
    expires_at_unix: i64,
    models: BTreeMap<String, CapabilityProfile>,
}

impl CapabilityRegistry {
    pub fn register_document(
        &mut self,
        provider_id: &str,
        document: CapabilityDiscoveryDocument,
        now_unix: i64,
    ) -> Result<usize, CapabilityError> {
        validate_identifier(provider_id)?;
        if document.schema_version != CAPABILITY_SCHEMA_VERSION {
            return Err(CapabilityError::UnsupportedSchema(document.schema_version));
        }
        if document.models.is_empty() {
            return Err(CapabilityError::EmptyDiscovery);
        }
        for (model, profile) in &document.models {
            validate_identifier(model)?;
            profile.validate()?;
        }
        let ttl = document.ttl_seconds.clamp(1, MAX_DISCOVERY_TTL_SECONDS);
        let count = document.models.len();
        self.providers.insert(
            provider_id.to_owned(),
            CachedProviderCapabilities {
                expires_at_unix: now_unix.saturating_add(ttl as i64),
                models: document.models,
            },
        );
        Ok(count)
    }

    pub fn resolve(&self, provider_id: &str, model: &str, now_unix: i64) -> ResolvedCapability {
        if let Some(provider) = self.providers.get(provider_id)
            && provider.expires_at_unix > now_unix
            && let Some(profile) = provider.models.get(model)
        {
            return ResolvedCapability {
                profile: profile.clone(),
                source: CapabilitySource::CachedDiscovery,
            };
        }
        ResolvedCapability {
            profile: CapabilityProfile::default(),
            source: CapabilitySource::ConservativeFallback,
        }
    }

    pub fn remove_expired(&mut self, now_unix: i64) -> usize {
        let before = self.providers.len();
        self.providers
            .retain(|_, provider| provider.expires_at_unix > now_unix);
        before - self.providers.len()
    }

    pub async fn discover_and_cache(
        &mut self,
        client: &reqwest::Client,
        provider_id: &str,
        discovery_url: &Url,
        bearer_token: Option<&str>,
        now_unix: i64,
    ) -> Result<usize, CapabilityError> {
        validate_identifier(provider_id)?;
        let document = fetch_capability_document(client, discovery_url, bearer_token).await?;
        self.register_document(provider_id, document, now_unix)
    }
}

pub async fn fetch_capability_document(
    client: &reqwest::Client,
    discovery_url: &Url,
    bearer_token: Option<&str>,
) -> Result<CapabilityDiscoveryDocument, CapabilityError> {
    if !matches!(discovery_url.scheme(), "http" | "https") {
        return Err(CapabilityError::InvalidDiscoveryUrl);
    }
    let mut request = client.get(discovery_url.clone());
    if let Some(token) = bearer_token.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| CapabilityError::DiscoveryRequest(error.to_string()))?
        .error_for_status()
        .map_err(|error| CapabilityError::DiscoveryRequest(error.to_string()))?;
    if response.content_length().unwrap_or(0) > MAX_DISCOVERY_BYTES as u64 {
        return Err(CapabilityError::DiscoveryTooLarge);
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| CapabilityError::DiscoveryRequest(error.to_string()))?;
    if body.len() > MAX_DISCOVERY_BYTES {
        return Err(CapabilityError::DiscoveryTooLarge);
    }
    serde_json::from_slice(&body)
        .map_err(|error| CapabilityError::InvalidDiscovery(error.to_string()))
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapabilityError {
    #[error("unsupported capability profile schema version {0}")]
    UnsupportedSchema(u32),
    #[error("context_window must be at least 256")]
    InvalidContextWindow,
    #[error("max_output_tokens must be positive and smaller than context_window")]
    InvalidOutputLimit,
    #[error("capability parameter names must be non-empty and not protocol-reserved")]
    InvalidParameterName,
    #[error("request parameter {0} is managed by MOMO")]
    ReservedParameter(String),
    #[error("request parameter {0} is not enabled by this endpoint capability profile")]
    UnsupportedParameter(String),
    #[error("no exact tokenizer mapping is known for model {0}")]
    UnknownTokenizerModel(String),
    #[error("provider and model identifiers must be non-empty and at most 256 characters")]
    InvalidIdentifier,
    #[error("capability discovery document contains no models")]
    EmptyDiscovery,
    #[error("capability discovery URL must use HTTP or HTTPS")]
    InvalidDiscoveryUrl,
    #[error("capability discovery request failed: {0}")]
    DiscoveryRequest(String),
    #[error("capability discovery response exceeds 256 KiB")]
    DiscoveryTooLarge,
    #[error("invalid capability discovery response: {0}")]
    InvalidDiscovery(String),
}

const fn capability_schema_version() -> u32 {
    CAPABILITY_SCHEMA_VERSION
}

const fn enabled() -> bool {
    true
}

const fn default_discovery_ttl() -> u64 {
    3600
}

fn validate_identifier(value: &str) -> Result<(), CapabilityError> {
    if value.trim().is_empty() || value.len() > 256 {
        return Err(CapabilityError::InvalidIdentifier);
    }
    Ok(())
}

fn default_parameters() -> BTreeSet<String> {
    ["top_p", "max_tokens", "max_completion_tokens", "stop"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn exact_profiles_use_bpe_instead_of_the_character_heuristic() {
        let text = "hello world";
        assert_eq!(TokenizerProfile::Cl100kBase.count(text).expect("count"), 2);
        assert_eq!(TokenizerProfile::O200kBase.count(text).expect("count"), 2);
        assert_eq!(
            TokenizerProfile::Model("gpt-4o".to_owned())
                .count(text)
                .expect("count"),
            2
        );
        assert_eq!(
            TokenizerProfile::Conservative.count(text).expect("count"),
            3
        );
    }

    #[test]
    fn profile_rejects_reserved_and_unadvertised_parameters() {
        let profile = CapabilityProfile::default();
        assert!(matches!(
            profile.validate_request_parameters(["model"]),
            Err(CapabilityError::ReservedParameter(_))
        ));
        assert!(matches!(
            profile.validate_request_parameters(["top_k"]),
            Err(CapabilityError::UnsupportedParameter(_))
        ));
        profile
            .validate_request_parameters(["top_p", "stop"])
            .expect("advertised parameters");
    }

    #[test]
    fn unknown_model_mapping_has_an_explicit_safe_fallback() {
        let tokenizer = TokenizerProfile::Model("third-party-unknown".to_owned());
        assert!(matches!(
            tokenizer.count("你好"),
            Err(CapabilityError::UnknownTokenizerModel(_))
        ));
        assert_eq!(tokenizer.count_or_conservative("你好"), 2);
    }

    #[test]
    fn registry_uses_valid_cache_then_expires_to_safe_fallback() {
        let mut registry = CapabilityRegistry::default();
        let exact = CapabilityProfile {
            tokenizer: TokenizerProfile::O200kBase,
            context_window: 128_000,
            max_output_tokens: 16_384,
            ..CapabilityProfile::default()
        };
        registry
            .register_document(
                "provider-a",
                CapabilityDiscoveryDocument {
                    schema_version: CAPABILITY_SCHEMA_VERSION,
                    ttl_seconds: 60,
                    models: BTreeMap::from([("model-a".to_owned(), exact.clone())]),
                },
                1_000,
            )
            .expect("register");

        let cached = registry.resolve("provider-a", "model-a", 1_059);
        assert_eq!(cached.source, CapabilitySource::CachedDiscovery);
        assert_eq!(cached.profile, exact);

        let expired = registry.resolve("provider-a", "model-a", 1_060);
        assert_eq!(expired.source, CapabilitySource::ConservativeFallback);
        assert_eq!(expired.profile.tokenizer, TokenizerProfile::Conservative);
    }

    #[test]
    fn registry_rejects_invalid_profiles_without_replacing_cache() {
        let mut registry = CapabilityRegistry::default();
        let result = registry.register_document(
            "provider-a",
            CapabilityDiscoveryDocument {
                schema_version: CAPABILITY_SCHEMA_VERSION,
                ttl_seconds: 0,
                models: BTreeMap::from([(
                    "model-a".to_owned(),
                    CapabilityProfile {
                        max_output_tokens: 0,
                        ..CapabilityProfile::default()
                    },
                )]),
            },
            1_000,
        );
        assert_eq!(result, Err(CapabilityError::InvalidOutputLimit));
        assert_eq!(
            registry.resolve("provider-a", "model-a", 1_000).source,
            CapabilitySource::ConservativeFallback
        );
    }

    #[tokio::test]
    async fn online_discovery_validates_and_caches_http_document() {
        let document = CapabilityDiscoveryDocument {
            schema_version: CAPABILITY_SCHEMA_VERSION,
            ttl_seconds: 120,
            models: BTreeMap::from([(
                "online-model".to_owned(),
                CapabilityProfile {
                    tokenizer: TokenizerProfile::Cl100kBase,
                    ..CapabilityProfile::default()
                },
            )]),
        };
        let body = serde_json::to_vec(&document).expect("serialize document");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 2048];
            let read = socket.read(&mut request).await.expect("read");
            assert!(
                String::from_utf8_lossy(&request[..read])
                    .starts_with("GET /.well-known/momo-capabilities.json ")
            );
            let headers = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.expect("headers");
            socket.write_all(&body).await.expect("body");
        });

        let mut registry = CapabilityRegistry::default();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .no_proxy()
            .build()
            .expect("client");
        let url = Url::parse(&format!(
            "http://{address}/.well-known/momo-capabilities.json"
        ))
        .expect("url");
        assert_eq!(
            registry
                .discover_and_cache(&client, "online-provider", &url, None, 2_000)
                .await
                .expect("discover"),
            1
        );
        server.await.expect("server");
        assert_eq!(
            registry
                .resolve("online-provider", "online-model", 2_001)
                .source,
            CapabilitySource::CachedDiscovery
        );
    }
}
