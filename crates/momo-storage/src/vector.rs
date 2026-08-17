use super::*;

const VECTOR_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS nsg_vectors (
    scope_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    vector_space_id TEXT NOT NULL,
    dimension INTEGER NOT NULL,
    vector_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (scope_id, node_id, vector_space_id)
);
CREATE INDEX IF NOT EXISTS nsg_vectors_scope_space_idx
    ON nsg_vectors(scope_id, vector_space_id);
"#;

impl TursoVectorStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let path = path
            .to_str()
            .ok_or_else(|| StorageError::InvalidTursoPath(path.display().to_string()))?;
        Self::connect(path).await
    }

    pub async fn in_memory() -> Result<Self, StorageError> {
        Self::connect(":memory:").await
    }

    async fn connect(path: &str) -> Result<Self, StorageError> {
        let database = turso::Builder::new_local(path).build().await?;
        database.connect()?.execute_batch(VECTOR_SCHEMA).await?;
        Ok(Self { database })
    }

    pub async fn list_nsg_vectors(
        &self,
        scope_id: Uuid,
        vector_space_id: &str,
    ) -> Result<Vec<NsgVectorRecord>, StorageError> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT scope_id, node_id, source_hash, vector_space_id, dimension, vector_json, created_at \
                 FROM nsg_vectors WHERE scope_id=?1 AND vector_space_id=?2",
                (scope_id.to_string(), vector_space_id.to_owned()),
            )
            .await?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await? {
            records.push(nsg_vector_from_row(&row)?);
        }
        Ok(records)
    }

    pub async fn remove_nsg_vectors(
        &self,
        scope_id: Uuid,
        vector_space_id: Option<&str>,
    ) -> Result<u64, StorageError> {
        let connection = self.database.connect()?;
        let removed = if let Some(vector_space_id) = vector_space_id {
            connection
                .execute(
                    "DELETE FROM nsg_vectors WHERE scope_id=?1 AND vector_space_id=?2",
                    (scope_id.to_string(), vector_space_id.to_owned()),
                )
                .await?
        } else {
            connection
                .execute(
                    "DELETE FROM nsg_vectors WHERE scope_id=?1",
                    (scope_id.to_string(),),
                )
                .await?
        };
        Ok(removed)
    }
}

impl NsgVectorStore for TursoVectorStore {
    async fn upsert_nsg_vectors(&self, records: &[NsgVectorRecord]) -> Result<(), StorageError> {
        for record in records {
            validate_nsg_vector(record).map_err(StorageError::InvalidNsgVector)?;
        }
        let connection = self.database.connect()?;
        connection.execute("BEGIN IMMEDIATE", ()).await?;
        for record in records {
            let result = connection
                .execute(
                    r#"INSERT INTO nsg_vectors
                      (scope_id, node_id, source_hash, vector_space_id, dimension, vector_json, created_at)
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                      ON CONFLICT(scope_id, node_id, vector_space_id) DO UPDATE SET
                        source_hash=excluded.source_hash,
                        dimension=excluded.dimension,
                        vector_json=excluded.vector_json,
                        created_at=excluded.created_at"#,
                    turso::params![
                        record.scope_id.to_string(),
                        record.node_id.clone(),
                        record.source_hash.clone(),
                        record.vector_space_id.clone(),
                        record.dimension as i64,
                        serde_json::to_string(&record.vector)?,
                        record.created_at.to_rfc3339(),
                    ],
                )
                .await;
            if let Err(error) = result {
                let _ = connection.execute("ROLLBACK", ()).await;
                return Err(error.into());
            }
        }
        connection.execute("COMMIT", ()).await?;
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

fn validate_nsg_vector(record: &NsgVectorRecord) -> Result<(), String> {
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

fn nsg_vector_from_row(row: &turso::Row) -> Result<NsgVectorRecord, StorageError> {
    let vector: Vec<f64> = serde_json::from_str(&row.get::<String>(5)?)?;
    let dimension = usize::try_from(row.get::<i64>(4)?)
        .map_err(|_| StorageError::InvalidNsgVector("invalid dimension".to_owned()))?;
    let record = NsgVectorRecord {
        scope_id: Uuid::parse_str(&row.get::<String>(0)?)?,
        node_id: row.get(1)?,
        source_hash: row.get(2)?,
        vector_space_id: row.get(3)?,
        dimension,
        vector,
        created_at: DateTime::parse_from_rfc3339(&row.get::<String>(6)?)?.with_timezone(&Utc),
    };
    validate_nsg_vector(&record).map_err(StorageError::InvalidNsgVector)?;
    Ok(record)
}
