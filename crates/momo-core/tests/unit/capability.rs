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
