use super::*;
use momo_core::{ConfiguredModel, ModelKind, ModelProvider};

fn connection(protocol: ProviderProtocol, base_url: String) -> ModelConnection {
    let provider = ModelProvider {
        id: uuid::Uuid::now_v7(),
        name: "Fixture".into(),
        protocol,
        base_url,
        api_path: String::new(),
        api_key: Some("fixture-secret".into()),
    };
    let model = ConfiguredModel {
        id: uuid::Uuid::now_v7(),
        provider_id: provider.id,
        model_id: "example".into(),
        name: "Example".into(),
        kind: ModelKind::Chat,
        context_window: Some(8192),
        max_output_tokens: None,
    };
    ModelConnection { provider, model }
}
#[test]
fn persistence_redacts_secrets_uses_cas_and_keeps_frozen_references() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    let mut registry = ModelRegistry::load(path.clone()).unwrap();
    let c = connection(ProviderProtocol::Openai, "http://127.0.0.1/v1".into());
    let reference = reference(&c);
    let config = ModelConfiguration {
        revision: 0,
        providers: vec![c.provider.clone()],
        models: vec![c.model.clone()],
    };
    let public = registry.update(config.clone()).unwrap();
    assert!(!public.to_string().contains("fixture-secret"));
    assert_eq!(public["providers"][0]["has_api_key"], true);
    assert!(matches!(
        registry.update(config.clone()),
        Err(GovernanceError::Conflict { .. })
    ));
    let mut next = config;
    next.revision = 1;
    next.providers[0].api_key = None;
    next.models[0].model_id = "next".into();
    registry.update(next).unwrap();
    let reopened = ModelRegistry::load(path).unwrap();
    assert_eq!(
        reopened.stored.current.providers[0].api_key.as_deref(),
        Some("fixture-secret")
    );
    assert_eq!(
        reopened.stored.snapshots[&reference].model.model_id,
        "example"
    );
    assert_eq!(reopened.stored.snapshots.len(), 2);
}

#[test]
fn host_store_can_live_outside_core_and_resolver_retains_removed_models() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private-host").join("accounts.json");
    let registry = ModelConnections::load(path.clone()).unwrap();
    let c = connection(ProviderProtocol::Openai, "http://127.0.0.1/v1".into());
    let public = registry
        .update(ModelConfiguration {
            revision: 0,
            providers: vec![c.provider],
            models: vec![c.model],
        })
        .unwrap();
    let id = public["models"][0]["reference"].as_str().unwrap();
    assert_eq!(
        registry.resolve(id).unwrap().unwrap().model.model_id,
        "example"
    );
    registry
        .update(ModelConfiguration {
            revision: 1,
            ..Default::default()
        })
        .unwrap();
    let reopened = ModelConnections::load(path).unwrap();
    assert_eq!(reopened.public().unwrap()["models"], json!([]));
    assert_eq!(
        reopened
            .resolve(id)
            .unwrap()
            .unwrap()
            .provider
            .api_key
            .as_deref(),
        Some("fixture-secret")
    );
    assert!(reopened.resolve("configured_unknown").unwrap().is_none());
}
