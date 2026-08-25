//! OpenAI-compatible embedding generation and NSG index orchestration.

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

use super::*;

const DEFAULT_EMBEDDING_TIMEOUT_SECONDS: u64 = 120;
const MAX_EMBEDDING_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_INDEX_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingRequestConfig {
    pub endpoint: EmbeddingEndpoint,
    pub profile: EmbeddingProfile,
    #[serde(default = "default_embedding_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl EmbeddingRequestConfig {
    fn provider(&self) -> Result<OpenAiEmbeddingProvider, EmbeddingError> {
        if self.timeout_seconds == 0 || self.timeout_seconds > MAX_EMBEDDING_TIMEOUT_SECONDS {
            return Err(EmbeddingError::InvalidProfile(format!(
                "embedding timeout must be between 1 and {MAX_EMBEDDING_TIMEOUT_SECONDS} seconds"
            )));
        }
        self.profile.validate()?;
        OpenAiEmbeddingProvider::new(
            self.endpoint.clone(),
            Duration::from_secs(self.timeout_seconds),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateEmbeddingsRequest {
    embedding: EmbeddingRequestConfig,
    inputs: Vec<EmbeddingInput>,
}

#[derive(Debug, thiserror::Error)]
pub enum GenerateEmbeddingsError {
    #[error("invalid embedding request JSON: {0}")]
    InvalidRequest(serde_json::Error),
    #[error(transparent)]
    Provider(#[from] EmbeddingError),
    #[error("failed to serialize embedding response: {0}")]
    Serialize(serde_json::Error),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NsgIndexMode {
    Full,
    Incremental,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RebuildNsgIndexRequest {
    embedding: EmbeddingRequestConfig,
    #[serde(default = "default_index_mode")]
    mode: NsgIndexMode,
    #[serde(default = "default_index_batch_size")]
    batch_size: usize,
}

pub async fn generate_embeddings_json(request_json: String) -> Result<String, String> {
    generate_embeddings_json_typed(request_json)
        .await
        .map_err(|error| error.to_string())
}

pub async fn generate_embeddings_json_typed(
    request_json: String,
) -> Result<String, GenerateEmbeddingsError> {
    let request: GenerateEmbeddingsRequest =
        serde_json::from_str(&request_json).map_err(GenerateEmbeddingsError::InvalidRequest)?;
    let provider = request.embedding.provider()?;
    let batch = provider
        .embed_batch(&request.embedding.profile, &request.inputs)
        .await?;
    serde_json::to_string(&batch).map_err(GenerateEmbeddingsError::Serialize)
}

pub(super) async fn embed_query(
    embedding: &EmbeddingRequestConfig,
    query: &str,
) -> Result<(String, Vec<f64>), String> {
    let provider = embedding.provider().map_err(|error| error.to_string())?;
    let batch = provider
        .embed_batch(
            &embedding.profile,
            &[EmbeddingInput {
                id: "query".to_owned(),
                text: query.to_owned(),
                purpose: EmbeddingPurpose::Query,
            }],
        )
        .await
        .map_err(|error| error.to_string())?;
    let vector = batch
        .vectors
        .into_iter()
        .next()
        .ok_or_else(|| "embedding provider returned no query vector".to_owned())?;
    Ok((batch.vector_space_id, vector.vector))
}

pub async fn rebuild_nsg_vector_index_json(
    scope_id: String,
    request_json: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let request: RebuildNsgIndexRequest =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    if request.batch_size == 0 || request.batch_size > MAX_EMBEDDING_BATCH_SIZE {
        return Err(format!(
            "embedding batch_size must be between 1 and {MAX_EMBEDDING_BATCH_SIZE}"
        ));
    }
    let provider = request
        .embedding
        .provider()
        .map_err(|error| error.to_string())?;
    let vector_space_id = request
        .embedding
        .profile
        .vector_space_id()
        .map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let documents = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?
        .embedding_documents()
        .map_err(|error| error.to_string())?;

    let existing = core()?
        .vector_store()
        .list_nsg_vectors(scope_id, &vector_space_id)
        .await
        .map_err(|error| error.to_string())?;
    let reusable = existing
        .into_iter()
        .filter(|record| record.dimension == request.embedding.profile.dimension)
        .map(|record| ((record.node_id.clone(), record.source_hash.clone()), record))
        .collect::<HashMap<_, _>>();
    let mut records = Vec::with_capacity(documents.len());
    let mut pending = Vec::new();
    let mut reused_count = 0_usize;
    for document in documents {
        if matches!(request.mode, NsgIndexMode::Incremental)
            && let Some(record) =
                reusable.get(&(document.node_id.clone(), document.source_hash.clone()))
        {
            records.push(record.clone());
            reused_count += 1;
        } else {
            pending.push(document);
        }
    }

    let mut batch_count = 0_usize;
    for documents in pending.chunks(request.batch_size) {
        let inputs = documents
            .iter()
            .map(|document| EmbeddingInput {
                id: document.node_id.clone(),
                text: document.content.clone(),
                purpose: EmbeddingPurpose::Document,
            })
            .collect::<Vec<_>>();
        let batch = provider
            .embed_batch(&request.embedding.profile, &inputs)
            .await
            .map_err(|error| error.to_string())?;
        if batch.vector_space_id != vector_space_id {
            return Err("embedding provider returned an unexpected vector space".to_owned());
        }
        for (document, vector) in documents.iter().zip(batch.vectors) {
            if document.node_id != vector.id {
                return Err("embedding response id did not match the NSG document".to_owned());
            }
            records.push(momo_storage::NsgVectorRecord {
                scope_id,
                node_id: document.node_id.clone(),
                source_hash: document.source_hash.clone(),
                vector_space_id: vector_space_id.clone(),
                dimension: request.embedding.profile.dimension,
                vector: vector.vector,
                created_at: chrono::Utc::now(),
            });
        }
        batch_count += 1;
    }
    records.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    core()?
        .vector_store()
        .replace_nsg_vectors(scope_id, &vector_space_id, &records)
        .await
        .map_err(|error| error.to_string())?;

    serde_json::to_string(&serde_json::json!({
        "vector_space_id": vector_space_id,
        "dimension": request.embedding.profile.dimension,
        "node_count": records.len(),
        "embedded_count": pending.len(),
        "reused_count": reused_count,
        "batch_count": batch_count,
        "mode": match request.mode {
            NsgIndexMode::Full => "full",
            NsgIndexMode::Incremental => "incremental",
        },
    }))
    .map_err(|error| error.to_string())
}

const fn default_embedding_timeout_seconds() -> u64 {
    DEFAULT_EMBEDDING_TIMEOUT_SECONDS
}

const fn default_index_batch_size() -> usize {
    DEFAULT_INDEX_BATCH_SIZE
}

const fn default_index_mode() -> NsgIndexMode {
    NsgIndexMode::Incremental
}
