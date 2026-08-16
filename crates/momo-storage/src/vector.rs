use super::*;

impl NsgVectorStore for LocalStore {
    async fn upsert_nsg_vectors(&self, records: &[NsgVectorRecord]) -> Result<(), StorageError> {
        for record in records {
            validate_nsg_vector(record).map_err(StorageError::InvalidNsgVector)?;
        }
        for record in records {
            self.upsert_nsg_vector(record).await?;
        }
        Ok(())
    }

    async fn rank_nsg_vectors(
        &self,
        scope_id: Uuid,
        vector_space_id: &str,
        query_vector: &[f64],
        current_hashes: &HashMap<String, String>,
        limit: usize,
    ) -> Result<Vec<String>, StorageError> {
        validate_query_vector(vector_space_id, query_vector)?;
        let limit = normalize_top_k(limit);
        let mut ranked = self
            .list_nsg_vectors(scope_id, vector_space_id)
            .await?
            .into_iter()
            .filter_map(|record| {
                (record.dimension == query_vector.len()
                    && current_hashes.get(record.node_id.as_str()) == Some(&record.source_hash))
                .then(|| {
                    cosine_similarity(query_vector, &record.vector)
                        .map(|score| (record.node_id, score))
                })
                .flatten()
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        Ok(ranked
            .into_iter()
            .take(limit)
            .map(|(node_id, _)| node_id)
            .collect())
    }

    async fn nsg_vector_status(
        &self,
        scope_id: Uuid,
        vector_space_id: &str,
        current_hashes: &HashMap<String, String>,
    ) -> Result<NsgVectorStatus, StorageError> {
        let vectors = if vector_space_id.trim().is_empty() {
            Vec::new()
        } else {
            self.list_nsg_vectors(scope_id, vector_space_id).await?
        };
        let indexed = vectors
            .iter()
            .filter(|record| {
                current_hashes.get(record.node_id.as_str()) == Some(&record.source_hash)
            })
            .count();
        let dimension = vectors.first().map(|record| record.dimension);
        Ok(NsgVectorStatus {
            vector_space_id: vector_space_id.to_owned(),
            dimension,
            node_count: current_hashes.len(),
            indexed_count: indexed,
            stale_count: vectors.len().saturating_sub(indexed),
            missing_count: current_hashes.len().saturating_sub(indexed),
        })
    }
}

pub(super) fn validate_nsg_vector(record: &NsgVectorRecord) -> Result<(), String> {
    if record.node_id.trim().is_empty()
        || record.source_hash.len() != 64
        || !record
            .source_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || record.vector_space_id.trim().is_empty()
        || record.dimension == 0
        || record.dimension > 8192
        || record.vector.len() != record.dimension
        || record.vector.iter().any(|value| !value.is_finite())
        || record.vector.iter().all(|value| *value == 0.0)
    {
        return Err(
            "scope, node, hash, space, dimension, and finite vector values are required".to_owned(),
        );
    }
    Ok(())
}

fn validate_query_vector(vector_space_id: &str, vector: &[f64]) -> Result<(), StorageError> {
    if vector_space_id.trim().is_empty()
        || vector.is_empty()
        || vector.len() > 8192
        || vector.iter().any(|value| !value.is_finite())
        || vector.iter().all(|value| *value == 0.0)
    {
        return Err(StorageError::InvalidNsgVector(
            "invalid semantic-graph query vector".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_top_k(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_NSG_VECTOR_TOP_K
    } else {
        limit.min(MAX_NSG_VECTOR_TOP_K)
    }
}

fn cosine_similarity(left: &[f64], right: &[f64]) -> Option<f64> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let (mut dot, mut left_norm, mut right_norm) = (0.0, 0.0, 0.0);
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    (left_norm > 0.0 && right_norm > 0.0).then(|| dot / (left_norm.sqrt() * right_norm.sqrt()))
}

pub(super) fn nsg_vector_from_row(row: &SqliteRow) -> Result<NsgVectorRecord, StorageError> {
    let scope_id = Uuid::parse_str(row.try_get("scope_id")?)?;
    let vector: Vec<f64> = serde_json::from_str(row.try_get("vector_json")?)?;
    let record = NsgVectorRecord {
        scope_id,
        node_id: row.try_get("node_id")?,
        source_hash: row.try_get("source_hash")?,
        vector_space_id: row.try_get("vector_space_id")?,
        dimension: usize::try_from(row.try_get::<i64, _>("dimension")?)
            .map_err(|_| StorageError::InvalidNsgVector("invalid dimension".to_owned()))?,
        vector,
        created_at: DateTime::parse_from_rfc3339(&row.try_get::<String, _>("created_at")?)
            .map_err(StorageError::Timestamp)?
            .with_timezone(&Utc),
    };
    validate_nsg_vector(&record).map_err(StorageError::InvalidNsgVector)?;
    Ok(record)
}
