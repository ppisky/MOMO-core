use super::*;

impl LocalStore {
    pub async fn stage_message(&self, message: &Message) -> Result<(), StorageError> {
        self.append_message(message).await
    }

    pub async fn save_message(&self, message: &Message) -> Result<(), StorageError> {
        self.save_message_record(message, None).await
    }

    pub async fn save_message_with_context(
        &self,
        message: &Message,
        context: Option<&momo_domain::conversation::MessageContextPayload>,
    ) -> Result<(), StorageError> {
        self.save_message_record(message, Some(context)).await
    }

    async fn save_message_record(
        &self,
        message: &Message,
        context: Option<Option<&momo_domain::conversation::MessageContextPayload>>,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let existing = sqlx::query("SELECT * FROM messages WHERE id=?")
            .bind(message.id.to_string())
            .fetch_optional(&mut *transaction)
            .await?;
        match existing {
            Some(row) => {
                let persisted = message_from_row(&row)?;
                if persisted.conversation_id != message.conversation_id
                    || persisted.role != message.role
                    || persisted.created_at != message.created_at
                {
                    return Err(StorageError::ImmutableMessageConflict(message.id));
                }
                sqlx::query("UPDATE messages SET content=? WHERE id=?")
                    .bind(&message.content)
                    .bind(message.id.to_string())
                    .execute(&mut *transaction)
                    .await?;
            }
            None => {
                insert_message_immutable(&mut transaction, message).await?;
            }
        }
        if let Some(context) = context {
            if let Some(context) = context {
                super::message_context::insert_message_context(
                    &mut transaction,
                    message.id,
                    context,
                    true,
                )
                .await?;
            } else {
                sqlx::query("DELETE FROM message_context WHERE message_id=?")
                    .bind(message.id.to_string())
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn message_by_id(&self, id: Uuid) -> Result<Option<Message>, StorageError> {
        let row = sqlx::query("SELECT * FROM messages WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(message_from_row).transpose()
    }

    pub async fn replace_message(&self, message: &Message) -> Result<(), StorageError> {
        sqlx::query(
            r#"INSERT INTO messages (id, conversation_id, role, content, created_at)
            VALUES (?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET
              conversation_id=excluded.conversation_id,
              role=excluded.role,
              content=excluded.content,
              created_at=excluded.created_at"#,
        )
        .bind(message.id.to_string())
        .bind(message.conversation_id.to_string())
        .bind(message.role.as_str())
        .bind(&message.content)
        .bind(message.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn stage_message_update(&self, message: &Message) -> Result<(), StorageError> {
        self.update_existing_message(message, None).await
    }

    pub async fn stage_message_update_for_scope(
        &self,
        scope_id: Uuid,
        message: &Message,
    ) -> Result<(), StorageError> {
        self.update_existing_message(message, Some(scope_id)).await
    }

    async fn update_existing_message(
        &self,
        message: &Message,
        scope_id: Option<Uuid>,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT messages.* FROM messages JOIN conversations ON conversations.id=messages.conversation_id WHERE messages.id=? AND (? IS NULL OR conversations.scope_id=?)",
        )
        .bind(message.id.to_string())
        .bind(scope_id.map(|id| id.to_string()))
        .bind(scope_id.map(|id| id.to_string()))
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::MessageNotFound(message.id))?;
        let persisted = message_from_row(&row)?;
        if persisted.conversation_id != message.conversation_id
            || persisted.role != message.role
            || persisted.created_at != message.created_at
        {
            return Err(StorageError::ImmutableMessageConflict(message.id));
        }
        sqlx::query("UPDATE messages SET content=? WHERE id=?")
            .bind(&message.content)
            .bind(message.id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn stage_message_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("message", "delete_message", id).await
    }

    pub async fn list_messages(&self, conversation_id: Uuid) -> Result<Vec<Message>, StorageError> {
        let rows =
            sqlx::query("SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at, id")
                .bind(conversation_id.to_string())
                .fetch_all(&self.pool)
                .await?;
        rows.iter().map(message_from_row).collect()
    }

    pub async fn list_messages_for_scope(
        &self,
        scope_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<Vec<Message>, StorageError> {
        let rows = sqlx::query(
            r#"SELECT messages.* FROM messages
            INNER JOIN conversations ON conversations.id = messages.conversation_id
            WHERE messages.conversation_id=? AND conversations.scope_id=?
            ORDER BY messages.created_at, messages.id"#,
        )
        .bind(conversation_id.to_string())
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(message_from_row).collect()
    }

    /// Determine the retained complete-turn window before loading message bodies.
    pub async fn list_message_window_for_scope(
        &self,
        scope_id: Uuid,
        conversation_id: Uuid,
        window: Option<(usize, usize)>,
    ) -> Result<MessageWindow, StorageError> {
        let Some((trigger, evict)) = window else {
            return Ok(MessageWindow {
                messages: self
                    .list_messages_for_scope(scope_id, conversation_id)
                    .await?,
                evicted_messages: 0,
            });
        };
        if trigger < 2 || evict == 0 || evict >= trigger || trigger > 1000 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "invalid history window".into(),
            )));
        }
        let mut transaction = self.pool.begin().await?;
        let cutoff = sqlx::query(
            r#"WITH ordered AS (
                SELECT m.id, m.created_at, m.role,
                    SUM(CASE WHEN m.role='user' THEN 1 ELSE 0 END) OVER (
                        ORDER BY m.created_at, m.id
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                    ) AS turn_number
                FROM messages m JOIN conversations c ON c.id=m.conversation_id
                WHERE m.conversation_id=? AND c.scope_id=?
            ), turns AS (
                SELECT id, created_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY turn_number ORDER BY created_at DESC, id DESC
                    ) AS reverse_ordinal,
                    MAX(CASE WHEN role='assistant' THEN 1 ELSE 0 END) OVER (
                        PARTITION BY turn_number
                    ) AS completed
                FROM ordered WHERE turn_number > 0
            ), boundaries AS (
                SELECT id, created_at,
                    ROW_NUMBER() OVER (ORDER BY created_at, id) AS ordinal,
                    COUNT(*) OVER () AS total
                FROM turns WHERE reverse_ordinal=1 AND completed=1
            )
            SELECT id, created_at FROM boundaries
            WHERE total >= ? AND ordinal <= ((total-?)/?+1)*?
            ORDER BY ordinal DESC LIMIT 1"#,
        )
        .bind(conversation_id.to_string())
        .bind(scope_id.to_string())
        .bind(trigger as i64)
        .bind(trigger as i64)
        .bind(evict as i64)
        .bind(evict as i64)
        .fetch_optional(&mut *transaction)
        .await?;
        let (rows, evicted_messages) = if let Some(cutoff) = cutoff {
            let date: String = cutoff.try_get("created_at")?;
            let id: String = cutoff.try_get("id")?;
            let evicted: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM messages WHERE conversation_id=? AND (created_at,id) <= (?,?)",
            ).bind(conversation_id.to_string()).bind(&date).bind(&id)
                .fetch_one(&mut *transaction).await?;
            let rows = sqlx::query(
                "SELECT * FROM messages WHERE conversation_id=? AND (created_at,id) > (?,?) ORDER BY created_at,id",
            ).bind(conversation_id.to_string()).bind(&date).bind(&id)
                .fetch_all(&mut *transaction).await?;
            (rows, evicted as usize)
        } else {
            let rows = sqlx::query(
                "SELECT m.* FROM messages m JOIN conversations c ON c.id=m.conversation_id WHERE m.conversation_id=? AND c.scope_id=? ORDER BY m.created_at,m.id",
            ).bind(conversation_id.to_string()).bind(scope_id.to_string())
                .fetch_all(&mut *transaction).await?;
            (rows, 0)
        };
        let messages = rows
            .iter()
            .map(message_from_row)
            .collect::<Result<_, _>>()?;
        transaction.commit().await?;
        Ok(MessageWindow {
            messages,
            evicted_messages,
        })
    }

    pub async fn message_for_scope(
        &self,
        scope_id: Uuid,
        message_id: Uuid,
    ) -> Result<Option<Message>, StorageError> {
        let row = sqlx::query(
            r#"SELECT messages.* FROM messages
            INNER JOIN conversations ON conversations.id = messages.conversation_id
            WHERE messages.id=? AND conversations.scope_id=?"#,
        )
        .bind(message_id.to_string())
        .bind(scope_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(message_from_row).transpose()
    }
}
