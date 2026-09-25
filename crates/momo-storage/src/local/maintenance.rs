use super::*;

fn pending_batch_from_row(row: &SqliteRow) -> Result<PendingMaintenanceBatch, StorageError> {
    Ok(PendingMaintenanceBatch {
        batch: MaintenanceBatch {
            batch_key: row.try_get("batch_key")?,
            scope_id: row.try_get("scope_id")?,
            kind: row.try_get("kind")?,
            request_ids: serde_json::from_str(row.try_get("request_ids_json")?)?,
            patch_yaml: row.try_get("patch_yaml")?,
        },
        prepared_commit_json: row.try_get("prepared_commit_json")?,
    })
}

impl LocalStore {
    pub async fn pending_maintenance_batch(
        &self,
        scope_id: &str,
        kind: &str,
    ) -> Result<Option<PendingMaintenanceBatch>, StorageError> {
        let row = sqlx::query("SELECT batch_key, scope_id, kind, request_ids_json, patch_yaml, prepared_commit_json FROM maintenance_batches WHERE scope_id=? AND kind=? ORDER BY created_at, batch_key LIMIT 1")
            .bind(scope_id).bind(kind).fetch_optional(&self.pool).await?;
        row.as_ref().map(pending_batch_from_row).transpose()
    }
    /// Lists staged operations in durable order, independently of today's batch threshold.
    pub async fn pending_maintenance_batches(
        &self,
    ) -> Result<Vec<PendingMaintenanceBatch>, StorageError> {
        let rows = sqlx::query("SELECT batch_key, scope_id, kind, request_ids_json, patch_yaml, prepared_commit_json FROM maintenance_batches ORDER BY created_at, batch_key")
            .fetch_all(&self.pool).await?;
        rows.iter().map(pending_batch_from_row).collect()
    }

    /// All prepared commits must be acknowledged before a Space is admitted again.
    pub async fn pending_space_maintenance_commits(
        &self,
        scope_id: uuid::Uuid,
    ) -> Result<Vec<PendingMaintenanceBatch>, StorageError> {
        let rows = sqlx::query("SELECT batch_key, scope_id, kind, request_ids_json, patch_yaml, prepared_commit_json FROM maintenance_batches WHERE scope_id=? AND prepared_commit_json IS NOT NULL ORDER BY created_at, batch_key")
            .bind(scope_id.to_string()).fetch_all(&self.pool).await?;
        rows.iter().map(pending_batch_from_row).collect()
    }

    /// Returns the original evidence window when resuming a staged batch.
    pub async fn maintenance_turns_for_batch(
        &self,
        batch: &MaintenanceBatch,
    ) -> Result<Vec<MaintenanceTurn>, StorageError> {
        let mut turns = Vec::with_capacity(batch.request_ids.len());
        for id in &batch.request_ids {
            let row = sqlx::query("SELECT request_id, scope_id, user_content, assistant_content FROM maintenance_turns WHERE request_id=? AND scope_id=?")
                .bind(id).bind(&batch.scope_id).fetch_one(&self.pool).await?;
            turns.push(MaintenanceTurn {
                request_id: row.try_get("request_id")?,
                scope_id: row.try_get("scope_id")?,
                user_content: row.try_get("user_content")?,
                assistant_content: row.try_get("assistant_content")?,
            });
        }
        Ok(turns)
    }

    /// First prepared plan wins. Replays must reuse it, even if the source files changed.
    pub async fn prepare_maintenance_commit(
        &self,
        batch_key: &str,
        plan_json: &str,
    ) -> Result<String, StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE maintenance_batches SET prepared_commit_json=? WHERE batch_key=? AND prepared_commit_json IS NULL")
            .bind(plan_json).bind(batch_key).execute(&mut *tx).await?;
        let plan = sqlx::query_scalar(
            "SELECT prepared_commit_json FROM maintenance_batches WHERE batch_key=?",
        )
        .bind(batch_key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(plan)
    }

    pub async fn append_maintenance_turn(
        &self,
        turn: &MaintenanceTurn,
        memory_enabled: bool,
        nsg_enabled: bool,
    ) -> Result<(), StorageError> {
        let result = sqlx::query(
            "INSERT INTO maintenance_turns \
             (request_id, scope_id, user_content, assistant_content, memory_done, nsg_done, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(request_id) DO NOTHING",
        )
        .bind(&turn.request_id)
        .bind(&turn.scope_id)
        .bind(&turn.user_content)
        .bind(&turn.assistant_content)
        .bind(i64::from(!memory_enabled))
        .bind(i64::from(!nsg_enabled))
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            let existing = sqlx::query(
                "SELECT scope_id, user_content, assistant_content FROM maintenance_turns \
                 WHERE request_id=?",
            )
            .bind(&turn.request_id)
            .fetch_one(&self.pool)
            .await?;
            let same = existing.try_get::<String, _>("scope_id")? == turn.scope_id
                && existing.try_get::<String, _>("user_content")? == turn.user_content
                && existing.try_get::<String, _>("assistant_content")? == turn.assistant_content;
            if !same {
                return Err(StorageError::MaintenanceTurnConflict(
                    turn.request_id.clone(),
                ));
            }
        }
        Ok(())
    }

    pub async fn pending_maintenance_turns(
        &self,
        scope_id: &str,
        kind: MaintenanceKind,
        limit: usize,
    ) -> Result<Vec<MaintenanceTurn>, StorageError> {
        let query = match kind {
            MaintenanceKind::Memory => {
                "SELECT request_id, scope_id, user_content, assistant_content \
                 FROM maintenance_turns WHERE scope_id=? AND memory_done=0 \
                 ORDER BY created_at, request_id LIMIT ?"
            }
            MaintenanceKind::SemanticGraph => {
                "SELECT request_id, scope_id, user_content, assistant_content \
                 FROM maintenance_turns WHERE scope_id=? AND nsg_done=0 \
                 ORDER BY created_at, request_id LIMIT ?"
            }
        };
        let rows = sqlx::query(query)
            .bind(scope_id)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(MaintenanceTurn {
                    request_id: row.try_get("request_id")?,
                    scope_id: row.try_get("scope_id")?,
                    user_content: row.try_get("user_content")?,
                    assistant_content: row.try_get("assistant_content")?,
                })
            })
            .collect()
    }

    pub async fn mark_maintenance_turns_done(
        &self,
        request_ids: &[String],
        kind: MaintenanceKind,
    ) -> Result<(), StorageError> {
        if request_ids.is_empty() {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await?;
        let query = match kind {
            MaintenanceKind::Memory => {
                "UPDATE maintenance_turns SET memory_done=1 WHERE request_id=?"
            }
            MaintenanceKind::SemanticGraph => {
                "UPDATE maintenance_turns SET nsg_done=1 WHERE request_id=?"
            }
        };
        for request_id in request_ids {
            sqlx::query(query)
                .bind(request_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Persists a generated patch before any file mutation. If a previous
    /// attempt crashed, the original patch is returned and must be reused.
    pub async fn maintenance_batch_patch(
        &self,
        batch_key: &str,
    ) -> Result<Option<String>, StorageError> {
        sqlx::query_scalar("SELECT patch_yaml FROM maintenance_batches WHERE batch_key=?")
            .bind(batch_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Removes a staged patch without acknowledging its source turns. This is
    /// used only when validation proves that a persisted model response can
    /// never be applied; the pending turns remain available for regeneration.
    pub async fn discard_maintenance_batch(&self, batch_key: &str) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "DELETE FROM maintenance_batches WHERE batch_key=? AND prepared_commit_json IS NULL",
        )
        .bind(batch_key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn stage_maintenance_batch(
        &self,
        batch: &MaintenanceBatch,
    ) -> Result<String, StorageError> {
        let request_ids_json = serde_json::to_string(&batch.request_ids)
            .map_err(|error| StorageError::Database(sqlx::Error::Protocol(error.to_string())))?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO maintenance_batches \
             (batch_key, scope_id, kind, request_ids_json, patch_yaml, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(batch_key) DO NOTHING",
        )
        .bind(&batch.batch_key)
        .bind(&batch.scope_id)
        .bind(&batch.kind)
        .bind(&request_ids_json)
        .bind(&batch.patch_yaml)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query(
            "SELECT scope_id, kind, request_ids_json, patch_yaml \
             FROM maintenance_batches WHERE batch_key=?",
        )
        .bind(&batch.batch_key)
        .fetch_one(&self.pool)
        .await?;
        let stored_scope: String = row.try_get("scope_id")?;
        let stored_kind: String = row.try_get("kind")?;
        let stored_ids: String = row.try_get("request_ids_json")?;
        if stored_scope != batch.scope_id
            || stored_kind != batch.kind
            || stored_ids != request_ids_json
        {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "maintenance batch key was reused with different inputs".to_owned(),
            )));
        }
        row.try_get("patch_yaml").map_err(Into::into)
    }

    /// Atomically acknowledges the source turns and removes the durable patch.
    pub async fn complete_maintenance_batch(
        &self,
        batch_key: &str,
        request_ids: &[String],
        kind: MaintenanceKind,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT request_ids_json, kind FROM maintenance_batches WHERE batch_key=?")
                .bind(batch_key)
                .fetch_optional(&mut *transaction)
                .await?;
        let expected_kind = match kind {
            MaintenanceKind::Memory => "memory",
            MaintenanceKind::SemanticGraph => "semantic_graph",
        };
        let matching = row
            .as_ref()
            .map(|row| -> Result<bool, StorageError> {
                let stored_ids: Vec<String> =
                    serde_json::from_str(row.try_get("request_ids_json")?)?;
                Ok(stored_ids == request_ids && row.try_get::<String, _>("kind")? == expected_kind)
            })
            .transpose()?
            .unwrap_or(false);
        if !matching {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "maintenance acknowledgement does not match the staged batch".to_owned(),
            )));
        }
        let query = match kind {
            MaintenanceKind::Memory => {
                "UPDATE maintenance_turns SET memory_done=1 WHERE request_id=?"
            }
            MaintenanceKind::SemanticGraph => {
                "UPDATE maintenance_turns SET nsg_done=1 WHERE request_id=?"
            }
        };
        for request_id in request_ids {
            sqlx::query(query)
                .bind(request_id)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query("DELETE FROM maintenance_batches WHERE batch_key=?")
            .bind(batch_key)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn clear_space_memory_state(
        &self,
        space_id: Uuid,
        memory: bool,
        semantic_graph: bool,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let space_id = space_id.to_string();
        if memory {
            sqlx::query("DELETE FROM memory_patch_reviews WHERE scope_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
        }
        if memory || semantic_graph {
            sqlx::query("DELETE FROM mo_state_operations WHERE space_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM mo_state_spaces WHERE space_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
        }
        if memory && semantic_graph {
            sqlx::query("DELETE FROM maintenance_turns WHERE scope_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM maintenance_batches WHERE scope_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
        } else {
            if memory {
                sqlx::query("UPDATE maintenance_turns SET memory_done=1 WHERE scope_id=?")
                    .bind(&space_id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query("DELETE FROM maintenance_batches WHERE scope_id=? AND kind='memory'")
                    .bind(&space_id)
                    .execute(&mut *transaction)
                    .await?;
            }
            if semantic_graph {
                sqlx::query("UPDATE maintenance_turns SET nsg_done=1 WHERE scope_id=?")
                    .bind(&space_id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query(
                    "DELETE FROM maintenance_batches WHERE scope_id=? AND kind='semantic_graph'",
                )
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                "DELETE FROM maintenance_turns WHERE scope_id=? AND memory_done=1 AND nsg_done=1",
            )
            .bind(&space_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
