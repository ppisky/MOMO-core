//! Provider capability discovery and registration.

use super::*;

pub async fn discover_capabilities_json(
    provider_id: String,
    discovery_url: String,
    bearer_token: Option<String>,
) -> Result<String, String> {
    let url = reqwest::Url::parse(&discovery_url).map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let mut document = fetch_capability_document(&client, &url, bearer_token.as_deref())
        .await
        .map_err(|error| error.to_string())?;
    document.ttl_seconds = document.ttl_seconds.clamp(1, MAX_DISCOVERY_TTL_SECONDS);
    let now = chrono::Utc::now().timestamp();
    CAPABILITIES
        .write()
        .await
        .register_document(&provider_id, document.clone(), now)
        .map_err(|error| error.to_string())?;
    let expires_at_unix = now.saturating_add(document.ttl_seconds as i64);
    Ok(json!({
        "document": document,
        "expires_at_unix": expires_at_unix,
    })
    .to_string())
}

pub async fn register_capabilities_json(
    provider_id: String,
    document_json: String,
    fetched_at_unix: i64,
) -> Result<(), String> {
    let document: CapabilityDiscoveryDocument =
        serde_json::from_str(&document_json).map_err(|error| error.to_string())?;
    CAPABILITIES
        .write()
        .await
        .register_document(&provider_id, document, fetched_at_unix)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub async fn resolve_capability_json(provider_id: String, model: String) -> Result<String, String> {
    let resolved =
        CAPABILITIES
            .read()
            .await
            .resolve(&provider_id, &model, chrono::Utc::now().timestamp());
    serde_json::to_string(&json!({
        "profile": resolved.profile,
        "source": match resolved.source {
            crate::CapabilitySource::CachedDiscovery => "cached_discovery",
            crate::CapabilitySource::ConservativeFallback => "conservative_fallback",
        },
    }))
    .map_err(|error| error.to_string())
}
