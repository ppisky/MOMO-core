use super::*;

impl LocalStore {
    pub async fn stage_message(&self, message: &Message) -> Result<(), StorageError> {
        self.append_message(message).await
    }

    pub async fn save_message(&self, message: &Message) -> Result<(), StorageError> {
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
        self.save_message(message).await
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
