//! Structured embedding profiles and OpenAI-compatible vector generation.

use std::{collections::HashSet, fmt, future::Future, time::Duration};

use futures_util::StreamExt;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_EMBEDDING_BATCH_SIZE: usize = 128;
pub const MAX_EMBEDDING_DIMENSION: usize = 8_192;
const MAX_EMBEDDING_INPUT_BYTES: usize = 1024 * 1024;
const MAX_EMBEDDING_BATCH_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 2_000;
const MAX_EMBEDDING_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingEndpoint {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl fmt::Debug for EmbeddingEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingEndpoint")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingNormalization {
    None,
    #[default]
    L2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingProfile {
    pub provider_id: String,
    pub endpoint_id: String,
    pub model: String,
    pub dimension: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_revision: Option<String>,
    #[serde(default)]
    pub normalization: EmbeddingNormalization,
    #[serde(default)]
    pub send_dimensions: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query_prefix: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub document_prefix: String,
}

impl EmbeddingProfile {
    pub fn validate(&self) -> Result<(), EmbeddingError> {
        for (label, value) in [
            ("provider_id", self.provider_id.as_str()),
            ("endpoint_id", self.endpoint_id.as_str()),
            ("model", self.model.as_str()),
        ] {
            if value.trim().is_empty() || value.chars().count() > 256 {
                return Err(EmbeddingError::InvalidProfile(format!(
                    "{label} must contain 1 to 256 characters"
                )));
            }
        }
        if self.dimension == 0 || self.dimension > MAX_EMBEDDING_DIMENSION {
            return Err(EmbeddingError::InvalidProfile(format!(
                "dimension must be between 1 and {MAX_EMBEDDING_DIMENSION}"
            )));
        }
        if self
            .model_revision
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.chars().count() > 256)
        {
            return Err(EmbeddingError::InvalidProfile(
                "model_revision must be omitted or contain 1 to 256 characters".to_owned(),
            ));
        }
        if self.query_prefix.len() > 4_096 || self.document_prefix.len() > 4_096 {
            return Err(EmbeddingError::InvalidProfile(
                "embedding prefixes must not exceed 4096 bytes".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn vector_space_id(&self) -> Result<String, EmbeddingError> {
        self.validate()?;
        let canonical = serde_json::to_vec(self).map_err(EmbeddingError::SerializeProfile)?;
        Ok(format!(
            "momo-embedding-v1:{}",
            hex::encode(Sha256::digest(canonical))
        ))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingPurpose {
    Query,
    Document,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingInput {
    pub id: String,
    pub text: String,
    pub purpose: EmbeddingPurpose,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingVector {
    pub id: String,
    pub vector: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingBatch {
    pub vector_space_id: String,
    pub model: String,
    pub dimension: usize,
    pub vectors: Vec<EmbeddingVector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("invalid embedding profile: {0}")]
    InvalidProfile(String),
    #[error("invalid embedding input: {0}")]
    InvalidInput(String),
    #[error("invalid embedding provider base URL: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("invalid embedding authorization header")]
    InvalidAuthorization,
    #[error("embedding request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("embedding endpoint returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("invalid embedding response: {0}")]
    InvalidResponse(String),
    #[error("embedding response exceeded the {MAX_EMBEDDING_RESPONSE_BYTES} byte limit")]
    ResponseTooLarge,
    #[error("failed to decode embedding response: {0}")]
    DecodeResponse(serde_json::Error),
    #[error("failed to serialize embedding profile: {0}")]
    SerializeProfile(serde_json::Error),
}

pub trait EmbeddingProvider {
    fn embed_batch(
        &self,
        profile: &EmbeddingProfile,
        inputs: &[EmbeddingInput],
    ) -> impl Future<Output = Result<EmbeddingBatch, EmbeddingError>> + Send;
}

#[derive(Debug, Clone)]
pub struct OpenAiEmbeddingProvider {
    client: Client,
    endpoint: EmbeddingEndpoint,
}

impl OpenAiEmbeddingProvider {
    pub fn new(endpoint: EmbeddingEndpoint, timeout: Duration) -> Result<Self, EmbeddingError> {
        embedding_url(&endpoint.base_url)?;
        Ok(Self {
            client: Client::builder().timeout(timeout).build()?,
            endpoint,
        })
    }

    #[cfg(test)]
    fn with_client(endpoint: EmbeddingEndpoint, client: Client) -> Self {
        Self { client, endpoint }
    }
}

impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed_batch(
        &self,
        profile: &EmbeddingProfile,
        inputs: &[EmbeddingInput],
    ) -> Result<EmbeddingBatch, EmbeddingError> {
        profile.validate()?;
        validate_inputs(inputs)?;
        let url = embedding_url(&self.endpoint.base_url)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(api_key) = self.endpoint.api_key.as_deref() {
            let value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                .map_err(|_| EmbeddingError::InvalidAuthorization)?;
            headers.insert(AUTHORIZATION, value);
        }
        let prepared = inputs
            .iter()
            .map(|input| match input.purpose {
                EmbeddingPurpose::Query => format!("{}{}", profile.query_prefix, input.text),
                EmbeddingPurpose::Document => {
                    format!("{}{}", profile.document_prefix, input.text)
                }
            })
            .collect::<Vec<_>>();
        let mut request = serde_json::json!({
            "model": profile.model,
            "input": prepared,
            "encoding_format": "float",
        });
        if profile.send_dimensions {
            request["dimensions"] = serde_json::json!(profile.dimension);
        }
        let response = self
            .client
            .post(url)
            .headers(headers)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_error_body(response).await?;
            return Err(EmbeddingError::Http {
                status: status.as_u16(),
                body,
            });
        }
        let body = read_response_body(response).await?;
        let response: EmbeddingResponse =
            serde_json::from_slice(&body).map_err(EmbeddingError::DecodeResponse)?;
        validate_response(profile, inputs, response)
    }
}

async fn read_error_body(response: reqwest::Response) -> Result<String, EmbeddingError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(MAX_ERROR_BODY_BYTES);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

async fn read_response_body(response: reqwest::Response) -> Result<Vec<u8>, EmbeddingError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_EMBEDDING_RESPONSE_BYTES as u64)
    {
        return Err(EmbeddingError::ResponseTooLarge);
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > MAX_EMBEDDING_RESPONSE_BYTES)
        {
            return Err(EmbeddingError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn validate_inputs(inputs: &[EmbeddingInput]) -> Result<(), EmbeddingError> {
    if inputs.is_empty() || inputs.len() > MAX_EMBEDDING_BATCH_SIZE {
        return Err(EmbeddingError::InvalidInput(format!(
            "a batch must contain 1 to {MAX_EMBEDDING_BATCH_SIZE} inputs"
        )));
    }
    let mut ids = HashSet::new();
    let mut total_bytes = 0_usize;
    for input in inputs {
        if input.id.trim().is_empty() || input.id.chars().count() > 512 || !ids.insert(&input.id) {
            return Err(EmbeddingError::InvalidInput(
                "input ids must be unique and contain 1 to 512 characters".to_owned(),
            ));
        }
        if input.text.trim().is_empty() || input.text.len() > MAX_EMBEDDING_INPUT_BYTES {
            return Err(EmbeddingError::InvalidInput(format!(
                "each input must contain 1 to {MAX_EMBEDDING_INPUT_BYTES} bytes"
            )));
        }
        total_bytes = total_bytes.saturating_add(input.text.len());
    }
    if total_bytes > MAX_EMBEDDING_BATCH_BYTES {
        return Err(EmbeddingError::InvalidInput(format!(
            "batch input exceeds {MAX_EMBEDDING_BATCH_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_response(
    profile: &EmbeddingProfile,
    inputs: &[EmbeddingInput],
    response: EmbeddingResponse,
) -> Result<EmbeddingBatch, EmbeddingError> {
    if response.model != profile.model {
        return Err(EmbeddingError::InvalidResponse(format!(
            "provider reported model {:?}, expected {:?}",
            response.model, profile.model
        )));
    }
    if response
        .object
        .as_deref()
        .is_some_and(|object| object != "list")
    {
        return Err(EmbeddingError::InvalidResponse(
            "response object must be \"list\" when present".to_owned(),
        ));
    }
    if response
        .usage
        .as_ref()
        .is_some_and(|usage| usage.total_tokens < usage.prompt_tokens)
    {
        return Err(EmbeddingError::InvalidResponse(
            "response usage total_tokens must not be lower than prompt_tokens".to_owned(),
        ));
    }
    if response.data.len() != inputs.len() {
        return Err(EmbeddingError::InvalidResponse(format!(
            "expected {} vectors, received {}",
            inputs.len(),
            response.data.len()
        )));
    }
    let mut ordered = vec![None; inputs.len()];
    for item in response.data {
        if item.index >= inputs.len() || ordered[item.index].is_some() {
            return Err(EmbeddingError::InvalidResponse(
                "response indexes must be unique and cover the request batch".to_owned(),
            ));
        }
        if item
            .object
            .as_deref()
            .is_some_and(|object| object != "embedding")
        {
            return Err(EmbeddingError::InvalidResponse(format!(
                "vector {} object must be \"embedding\" when present",
                item.index
            )));
        }
        let mut vector = item.embedding;
        if vector.len() != profile.dimension
            || vector.iter().any(|value| !value.is_finite())
            || vector.iter().all(|value| *value == 0.0)
        {
            return Err(EmbeddingError::InvalidResponse(format!(
                "vector {} does not match dimension {} or contains invalid values",
                item.index, profile.dimension
            )));
        }
        if profile.normalization == EmbeddingNormalization::L2 {
            normalize_l2(&mut vector)?;
        }
        ordered[item.index] = Some(vector);
    }
    let vectors = inputs
        .iter()
        .zip(ordered)
        .map(|(input, vector)| EmbeddingVector {
            id: input.id.clone(),
            vector: vector.expect("validated response covers every input index"),
        })
        .collect();
    Ok(EmbeddingBatch {
        vector_space_id: profile.vector_space_id()?,
        model: response.model,
        dimension: profile.dimension,
        vectors,
        usage: response.usage,
    })
}

fn normalize_l2(vector: &mut [f64]) -> Result<(), EmbeddingError> {
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err(EmbeddingError::InvalidResponse(
            "vector norm is zero or non-finite".to_owned(),
        ));
    }
    for value in vector {
        *value /= norm;
    }
    Ok(())
}

fn embedding_url(base_url: &str) -> Result<Url, url::ParseError> {
    let mut base = Url::parse(base_url)?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join("embeddings")
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingResponseItem>,
    model: String,
    object: Option<String>,
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponseItem {
    index: usize,
    embedding: Vec<f64>,
    object: Option<String>,
}

#[cfg(test)]
mod tests {
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
}
