use super::*;

impl LocalStore {
    pub async fn pending_space_memory_patch_commits(
        &self,
        scope_id: Uuid,
    ) -> Result<Vec<PendingMemoryPatchReview>, StorageError> {
        let rows = sqlx::query("SELECT * FROM memory_patch_reviews WHERE scope_id=? AND status='pending' AND prepared_commit_json IS NOT NULL ORDER BY created_at, id")
            .bind(scope_id.to_string()).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                Ok(PendingMemoryPatchReview {
                    review: memory_patch_review_from_row(row)?,
                    prepared_commit_json: row.try_get("prepared_commit_json")?,
                })
            })
            .collect()
    }
    pub async fn prepared_memory_patch_review(
        &self,
        scope_id: Uuid,
        id: Uuid,
    ) -> Result<Option<PendingMemoryPatchReview>, StorageError> {
        let row = sqlx::query("SELECT * FROM memory_patch_reviews WHERE scope_id=? AND id=? AND status='pending' AND prepared_commit_json IS NOT NULL")
            .bind(scope_id.to_string()).bind(id.to_string()).fetch_optional(&self.pool).await?;
        row.as_ref()
            .map(|row| {
                Ok(PendingMemoryPatchReview {
                    review: memory_patch_review_from_row(row)?,
                    prepared_commit_json: row.try_get("prepared_commit_json")?,
                })
            })
            .transpose()
    }
    pub async fn pending_memory_patch_commits(
        &self,
    ) -> Result<Vec<PendingMemoryPatchReview>, StorageError> {
        let rows = sqlx::query("SELECT * FROM memory_patch_reviews WHERE status='pending' AND prepared_commit_json IS NOT NULL ORDER BY created_at, id")
            .fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                Ok(PendingMemoryPatchReview {
                    review: memory_patch_review_from_row(row)?,
                    prepared_commit_json: row.try_get("prepared_commit_json")?,
                })
            })
            .collect()
    }

    pub async fn prepare_memory_patch_review_commit(
        &self,
        scope_id: Uuid,
        id: Uuid,
        plan: &str,
    ) -> Result<String, StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE memory_patch_reviews SET prepared_commit_json=? WHERE scope_id=? AND id=? AND status='pending' AND prepared_commit_json IS NULL")
            .bind(plan).bind(scope_id.to_string()).bind(id.to_string()).execute(&mut *tx).await?;
        let plan = sqlx::query_scalar("SELECT prepared_commit_json FROM memory_patch_reviews WHERE scope_id=? AND id=? AND status='pending' AND prepared_commit_json IS NOT NULL")
            .bind(scope_id.to_string()).bind(id.to_string()).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(plan)
    }

    pub async fn create_memory_patch_review(
        &self,
        scope_id: Uuid,
        conversation_id: &str,
        patch_yaml: &str,
        targets: &[String],
        operation_count: i64,
        review_mode: &str,
    ) -> Result<MemoryPatchReview, StorageError> {
        let review = MemoryPatchReview {
            id: Uuid::now_v7(),
            scope_id,
            conversation_id: conversation_id.to_owned(),
            patch_yaml: patch_yaml.to_owned(),
            targets: targets.to_vec(),
            operation_count,
            review_mode: review_mode.to_owned(),
            status: MemoryPatchReviewStatus::Pending,
            created_at: Utc::now(),
            resolved_at: None,
            result: None,
            error: None,
        };
        sqlx::query(
            r#"INSERT INTO memory_patch_reviews
            (id, scope_id, conversation_id, patch_yaml, targets, operation_count,
             review_mode, status, created_at)
            VALUES (?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(review.id.to_string())
        .bind(review.scope_id.to_string())
        .bind(&review.conversation_id)
        .bind(&review.patch_yaml)
        .bind(serde_json::to_string(&review.targets)?)
        .bind(review.operation_count)
        .bind(&review.review_mode)
        .bind(review.status.as_str())
        .bind(review.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(review)
    }

    pub async fn memory_patch_review(
        &self,
        scope_id: Uuid,
        review_id: Uuid,
    ) -> Result<Option<MemoryPatchReview>, StorageError> {
        let row = sqlx::query("SELECT * FROM memory_patch_reviews WHERE scope_id=? AND id=?")
            .bind(scope_id.to_string())
            .bind(review_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(memory_patch_review_from_row).transpose()
    }

    pub async fn list_memory_patch_reviews(
        &self,
        scope_id: Uuid,
        include_resolved: bool,
    ) -> Result<Vec<MemoryPatchReview>, StorageError> {
        let rows = if include_resolved {
            sqlx::query(
                r#"SELECT * FROM memory_patch_reviews
                WHERE scope_id=? ORDER BY created_at DESC, id DESC LIMIT 200"#,
            )
            .bind(scope_id.to_string())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT * FROM memory_patch_reviews
                WHERE scope_id=? AND status='pending'
                ORDER BY created_at DESC, id DESC LIMIT 200"#,
            )
            .bind(scope_id.to_string())
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(memory_patch_review_from_row).collect()
    }

    pub async fn resolve_memory_patch_review(
        &self,
        scope_id: Uuid,
        review_id: Uuid,
        status: MemoryPatchReviewStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<Option<MemoryPatchReview>, StorageError> {
        debug_assert!(status != MemoryPatchReviewStatus::Pending);
        let now = Utc::now().to_rfc3339();
        let updated = sqlx::query(
            r#"UPDATE memory_patch_reviews
            SET status=?, resolved_at=?, result=?, error=?, prepared_commit_json=NULL
            WHERE scope_id=? AND id=? AND status='pending'
            AND (prepared_commit_json IS NULL OR ?='approved')"#,
        )
        .bind(status.as_str())
        .bind(now)
        .bind(result)
        .bind(error)
        .bind(scope_id.to_string())
        .bind(review_id.to_string())
        .bind(status.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.memory_patch_review(scope_id, review_id).await
    }
}
