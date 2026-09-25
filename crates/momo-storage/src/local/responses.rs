use super::*;

impl LocalStore {
    pub async fn response_operation(
        &self,
        request_id: &str,
    ) -> Result<Option<ResponseOperation>, StorageError> {
        let row = sqlx::query(
            "SELECT request_id, request_fingerprint, conversation_id, user_written, resolved_input_json, response_json \
             FROM response_operations WHERE request_id=?",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ResponseOperation {
                request_id: row.try_get("request_id")?,
                request_fingerprint: row.try_get("request_fingerprint")?,
                conversation_id: row.try_get("conversation_id")?,
                user_written: row.try_get::<i64, _>("user_written")? != 0,
                resolved_input_json: row.try_get("resolved_input_json")?,
                response_json: row.try_get("response_json")?,
            })
        })
        .transpose()
    }

    pub async fn control_operation(
        &self,
        operation_key: &str,
    ) -> Result<Option<ControlOperation>, StorageError> {
        let row = sqlx::query(
            "SELECT operation_key, request_fingerprint, response_json \
             FROM control_operations WHERE operation_key=?",
        )
        .bind(operation_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ControlOperation {
                operation_key: row.try_get("operation_key")?,
                request_fingerprint: row.try_get("request_fingerprint")?,
                response_json: row.try_get("response_json")?,
            })
        })
        .transpose()
    }

    /// Claims a control request. `false` means another attempt already owns
    /// the same operation key; callers must inspect and replay that record.
    pub async fn begin_control_operation(
        &self,
        operation_key: &str,
        request_fingerprint: &str,
    ) -> Result<bool, StorageError> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO control_operations \
             (operation_key, request_fingerprint, created_at, updated_at) \
             VALUES (?, ?, ?, ?) ON CONFLICT(operation_key) DO NOTHING",
        )
        .bind(operation_key)
        .bind(request_fingerprint)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn complete_control_operation(
        &self,
        operation_key: &str,
        response_json: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE control_operations SET response_json=?, updated_at=? WHERE operation_key=?",
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(operation_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn abandon_control_operation(&self, operation_key: &str) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM control_operations WHERE operation_key=? AND response_json IS NULL",
        )
        .bind(operation_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn begin_response_operation(
        &self,
        request_id: &str,
        request_fingerprint: &str,
        conversation_id: &str,
        resolved_input_json: &str,
    ) -> Result<(), StorageError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO response_operations \
             (request_id, request_fingerprint, conversation_id, user_written, resolved_input_json, created_at, updated_at) \
             VALUES (?, ?, ?, 0, ?, ?, ?) ON CONFLICT(request_id) DO UPDATE SET \
             resolved_input_json=COALESCE(response_operations.resolved_input_json, excluded.resolved_input_json) \
             WHERE response_operations.request_fingerprint=excluded.request_fingerprint \
             AND response_operations.conversation_id=excluded.conversation_id",
        )
        .bind(request_id)
        .bind(request_fingerprint)
        .bind(conversation_id)
        .bind(resolved_input_json)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_response_user_written(&self, request_id: &str) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE response_operations SET user_written=1, updated_at=? WHERE request_id=?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Atomically appends the user message for a response operation and marks
    /// that phase complete. A retry after any committed transaction observes
    /// `user_written=1`; a crash before commit observes neither change.
    pub async fn append_response_user_message(
        &self,
        request_id: &str,
        conversation_scope_id: Uuid,
        message: &Message,
    ) -> Result<bool, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT conversation_id, user_written FROM response_operations WHERE request_id=?",
        )
        .bind(request_id)
        .fetch_one(&mut *transaction)
        .await?;
        let conversation_id: String = row.try_get("conversation_id")?;
        if conversation_id != message.conversation_id.to_string() {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response operation conversation does not match user message".to_owned(),
            )));
        }
        let owned: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id=? AND scope_id=?")
                .bind(message.conversation_id.to_string())
                .bind(conversation_scope_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        if owned != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response conversation does not belong to conversation scope".to_owned(),
            )));
        }
        if row.try_get::<i64, _>("user_written")? != 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        if insert_message_immutable(&mut transaction, message).await? {
            sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                .bind(message.created_at.to_rfc3339())
                .bind(message.conversation_id.to_string())
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query(
            "UPDATE response_operations SET user_written=1, updated_at=? WHERE request_id=?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn complete_response_operation(
        &self,
        request_id: &str,
        response_json: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE response_operations SET response_json=?, updated_at=? WHERE request_id=?",
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Atomically commits every durable effect of a completed model response.
    ///
    /// A process exit can therefore leave either the pre-completion state or
    /// the complete assistant message, maintenance turn, and replay record;
    /// it cannot expose only a subset of those effects.
    pub async fn commit_response_completion(
        &self,
        completion: ResponseCompletion<'_>,
    ) -> Result<bool, StorageError> {
        let ResponseCompletion {
            request_id,
            conversation_scope_id,
            assistant_message,
            maintenance_turn,
            memory_enabled,
            nsg_enabled,
            mo_state_operation_id,
            response_json,
        } = completion;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT conversation_id, response_json FROM response_operations WHERE request_id=?",
        )
        .bind(request_id)
        .fetch_one(&mut *transaction)
        .await?;
        let conversation_id: String = row.try_get("conversation_id")?;
        if row.try_get::<Option<String>, _>("response_json")?.is_some() {
            transaction.rollback().await?;
            return Ok(false);
        }
        let owned: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id=? AND scope_id=?")
                .bind(&conversation_id)
                .bind(conversation_scope_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        if owned != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response conversation does not belong to conversation scope".to_owned(),
            )));
        }
        if let Some(message) = assistant_message {
            if message.conversation_id.to_string() != conversation_id
                || message.role != MessageRole::Assistant
            {
                return Err(StorageError::Database(sqlx::Error::Protocol(
                    "response completion contains an invalid assistant message".to_owned(),
                )));
            }
            if insert_message_immutable(&mut transaction, message).await? {
                sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                    .bind(message.created_at.to_rfc3339())
                    .bind(&conversation_id)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        if let Some(turn) = maintenance_turn {
            if turn.request_id != request_id {
                return Err(StorageError::Database(sqlx::Error::Protocol(
                    "maintenance turn does not match response operation".to_owned(),
                )));
            }
            sqlx::query(
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
            .execute(&mut *transaction)
            .await?;
        }
        let changed = sqlx::query(
            "UPDATE response_operations SET response_json=?, updated_at=? \
             WHERE request_id=? AND response_json IS NULL",
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response completion record changed concurrently".to_owned(),
            )));
        }
        if let Some(operation_id) = mo_state_operation_id {
            let changed = sqlx::query(
                "UPDATE mo_state_operations SET phase='completed', error=NULL, updated_at=? \
                 WHERE operation_id=? AND phase IN ('projected', 'completed')",
            )
            .bind(Utc::now().to_rfc3339())
            .bind(operation_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if changed != 1 {
                return Err(StorageError::Database(sqlx::Error::Protocol(
                    "response completion does not have a projected MO State operation".to_owned(),
                )));
            }
        }
        transaction.commit().await?;
        Ok(true)
    }
}
