use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn profile() -> EmbeddingProfile {
    EmbeddingProfile {
        provider_id: "openai-compatible".to_owned(),
        endpoint_id: "primary".to_owned(),
        model: "embedding-model".to_owned(),
        dimension: 3,
        model_revision: Some("2026-08".to_owned()),
        normalization: EmbeddingNormalization::L2,
        send_dimensions: true,
        query_prefix: "query: ".to_owned(),
        document_prefix: "passage: ".to_owned(),
    }
}

#[test]
fn vector_space_identity_is_deterministic_and_structured() {
    let profile = profile();
    let first = profile.vector_space_id().expect("space id");
    let second = profile.vector_space_id().expect("space id");
    assert_eq!(first, second);
    assert!(first.starts_with("momo-embedding-v1:"));
    let mut changed = profile;
    changed.dimension = 4;
    assert_ne!(first, changed.vector_space_id().expect("changed space id"));
}

#[test]
fn rejects_duplicate_input_ids() {
    let inputs = vec![
        EmbeddingInput {
            id: "same".to_owned(),
            text: "one".to_owned(),
            purpose: EmbeddingPurpose::Document,
        },
        EmbeddingInput {
            id: "same".to_owned(),
            text: "two".to_owned(),
            purpose: EmbeddingPurpose::Document,
        },
    ];
    assert!(matches!(
        validate_inputs(&inputs),
        Err(EmbeddingError::InvalidInput(_))
    ));
}

#[test]
fn endpoint_debug_output_redacts_api_keys() {
    let endpoint = EmbeddingEndpoint {
        base_url: "http://127.0.0.1/v1".to_owned(),
        api_key: Some("top-secret-key".to_owned()),
    };
    let debug = format!("{endpoint:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("top-secret-key"));
}

#[tokio::test]
async fn calls_openai_compatible_endpoint_and_restores_response_order() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = vec![0_u8; 16_384];
        let read = socket.read(&mut request).await.expect("read request");
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.starts_with("POST /v1/embeddings "));
        assert!(request.contains("query: hello"));
        assert!(request.contains("passage: world"));
        assert!(request.contains("\"dimensions\":3"));
        let body = serde_json::json!({
            "model": "embedding-model",
            "object": "list",
            "usage": {"prompt_tokens": 4, "total_tokens": 4},
            "data": [
                {"object": "embedding", "index": 1, "embedding": [0.0, 3.0, 4.0]},
                {"object": "embedding", "index": 0, "embedding": [2.0, 0.0, 0.0]}
            ]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let provider = OpenAiEmbeddingProvider::with_client(
        EmbeddingEndpoint {
            base_url: format!("http://{address}/v1"),
            api_key: None,
        },
        Client::builder().no_proxy().build().expect("client"),
    );
    let batch = provider
        .embed_batch(
            &profile(),
            &[
                EmbeddingInput {
                    id: "query".to_owned(),
                    text: "hello".to_owned(),
                    purpose: EmbeddingPurpose::Query,
                },
                EmbeddingInput {
                    id: "document".to_owned(),
                    text: "world".to_owned(),
                    purpose: EmbeddingPurpose::Document,
                },
            ],
        )
        .await
        .expect("embedding batch");
    assert_eq!(batch.vectors[0].id, "query");
    assert_eq!(batch.vectors[0].vector, vec![1.0, 0.0, 0.0]);
    assert_eq!(batch.vectors[1].id, "document");
    assert_eq!(batch.vectors[1].vector, vec![0.0, 0.6, 0.8]);
    assert_eq!(
        batch.usage,
        Some(EmbeddingUsage {
            prompt_tokens: 4,
            total_tokens: 4
        })
    );
}

#[test]
fn rejects_wrong_response_dimension() {
    let error = validate_response(
        &profile(),
        &[EmbeddingInput {
            id: "query".to_owned(),
            text: "hello".to_owned(),
            purpose: EmbeddingPurpose::Query,
        }],
        EmbeddingResponse {
            data: vec![EmbeddingResponseItem {
                index: 0,
                embedding: vec![1.0, 2.0],
                object: None,
            }],
            model: "embedding-model".to_owned(),
            object: None,
            usage: None,
        },
    )
    .expect_err("wrong dimension");
    assert!(matches!(error, EmbeddingError::InvalidResponse(_)));
}

#[test]
fn rejects_unexpected_response_model() {
    let error = validate_response(
        &profile(),
        &[EmbeddingInput {
            id: "query".to_owned(),
            text: "hello".to_owned(),
            purpose: EmbeddingPurpose::Query,
        }],
        EmbeddingResponse {
            data: vec![EmbeddingResponseItem {
                index: 0,
                embedding: vec![1.0, 0.0, 0.0],
                object: None,
            }],
            model: "different-model".to_owned(),
            object: None,
            usage: None,
        },
    )
    .expect_err("unexpected model");
    assert!(matches!(error, EmbeddingError::InvalidResponse(_)));
}

#[test]
fn rejects_invalid_openai_response_metadata() {
    let input = [EmbeddingInput {
        id: "query".to_owned(),
        text: "hello".to_owned(),
        purpose: EmbeddingPurpose::Query,
    }];
    let response = EmbeddingResponse {
        data: vec![EmbeddingResponseItem {
            index: 0,
            embedding: vec![1.0, 0.0, 0.0],
            object: Some("not-an-embedding".to_owned()),
        }],
        model: "embedding-model".to_owned(),
        object: Some("list".to_owned()),
        usage: Some(EmbeddingUsage {
            prompt_tokens: 2,
            total_tokens: 1,
        }),
    };
    assert!(matches!(
        validate_response(&profile(), &input, response),
        Err(EmbeddingError::InvalidResponse(_))
    ));
}
