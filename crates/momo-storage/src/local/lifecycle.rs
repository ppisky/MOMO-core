use super::*;

impl LocalStore {
    pub async fn lifecycle_status(&self, space: &str) -> Result<serde_json::Value, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_lifecycle_events WHERE space_id=? AND completed=0",
        )
        .bind(space)
        .fetch_one(&mut *transaction)
        .await?;
        let completed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_lifecycle_events WHERE space_id=? AND completed=1",
        )
        .bind(space)
        .fetch_one(&mut *transaction)
        .await?;
        let report: Option<String> = sqlx::query_scalar("SELECT report_json FROM memory_lifecycle_events WHERE space_id=? AND completed=1 ORDER BY sequence DESC LIMIT 1")
            .bind(space).fetch_optional(&mut *transaction).await?.flatten();
        transaction.commit().await?;
        let report = report
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()?;
        Ok(
            serde_json::json!({"pending_events": pending, "completed_events": completed, "last_report": report}),
        )
    }
    pub async fn pending_lifecycle_events(
        &self,
        space: Option<&str>,
    ) -> Result<Vec<LifecycleEvent>, StorageError> {
        let rows = sqlx::query("SELECT request_id, space_id, activity_json, prepared_commit_json, report_json FROM memory_lifecycle_events WHERE completed=0 AND (? IS NULL OR space_id=?) ORDER BY sequence LIMIT 64")
            .bind(space).bind(space).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                Ok(LifecycleEvent {
                    request_id: row.try_get("request_id")?,
                    space_id: row.try_get("space_id")?,
                    activity_json: row.try_get("activity_json")?,
                    prepared_commit_json: row.try_get("prepared_commit_json")?,
                    report_json: row.try_get("report_json")?,
                })
            })
            .collect()
    }

    pub async fn prepared_lifecycle_spaces(&self) -> Result<Vec<String>, StorageError> {
        Ok(sqlx::query_scalar("SELECT DISTINCT space_id FROM memory_lifecycle_events WHERE completed=0 AND prepared_commit_json IS NOT NULL")
            .fetch_all(&self.pool).await?)
    }

    pub async fn prepare_lifecycle_event(
        &self,
        request: &str,
        plan: &str,
        report: &str,
    ) -> Result<(), StorageError> {
        let changed = sqlx::query("UPDATE memory_lifecycle_events SET prepared_commit_json=?, report_json=? WHERE request_id=? AND completed=0 AND prepared_commit_json IS NULL")
            .bind(plan).bind(report).bind(request).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "lifecycle event already prepared or absent".into(),
            )));
        }
        Ok(())
    }

    pub async fn complete_lifecycle_event(&self, request: &str) -> Result<(), StorageError> {
        let changed = sqlx::query("UPDATE memory_lifecycle_events SET completed=1, prepared_commit_json=NULL WHERE request_id=? AND completed=0 AND prepared_commit_json IS NOT NULL")
            .bind(request).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "lifecycle event not prepared".into(),
            )));
        }
        Ok(())
    }
}
