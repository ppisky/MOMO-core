use super::*;

impl LocalStore {
    pub async fn save_conversation(&self, conversation: &Conversation) -> Result<(), StorageError> {
        if self.is_tombstoned("conversation", conversation.id).await? {
            return Ok(());
        }
        sqlx::query(
            r#"INSERT INTO conversations
            (id, scope_id, character_id, title, created_at, updated_at)
            VALUES (?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET scope_id=excluded.scope_id,
              character_id=excluded.character_id, title=excluded.title,
              updated_at=excluded.updated_at"#,
        )
        .bind(conversation.id.to_string())
        .bind(conversation.scope_id.to_string())
        .bind(conversation.character_id.map(|id| id.to_string()))
        .bind(&conversation.title)
        .bind(conversation.created_at.to_rfc3339())
        .bind(conversation.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_conversations(&self) -> Result<Vec<Conversation>, StorageError> {
        let rows = sqlx::query("SELECT * FROM conversations ORDER BY updated_at DESC, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(conversation_from_row).collect()
    }

    pub async fn list_conversations_for_scope(
        &self,
        scope_id: Uuid,
    ) -> Result<Vec<Conversation>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM conversations WHERE scope_id=? ORDER BY updated_at DESC, id",
        )
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(conversation_from_row).collect()
    }

    pub async fn conversation_for_scope(
        &self,
        scope_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<Option<Conversation>, StorageError> {
        let row = sqlx::query("SELECT * FROM conversations WHERE id=? AND scope_id=?")
            .bind(conversation_id.to_string())
            .bind(scope_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(conversation_from_row).transpose()
    }

    pub async fn stage_conversation(
        &self,
        conversation: &Conversation,
    ) -> Result<(), StorageError> {
        self.save_conversation(conversation).await
    }

    pub async fn stage_conversation_update(
        &self,
        conversation: &Conversation,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let character_id: Option<String> =
            sqlx::query_scalar("SELECT character_id FROM conversations WHERE id=? AND scope_id=?")
                .bind(conversation.id.to_string())
                .bind(conversation.scope_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        let mut updated = conversation.clone();
        updated.character_id = character_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()?;
        sqlx::query("UPDATE conversations SET title=?, updated_at=? WHERE id=? AND scope_id=?")
            .bind(&updated.title)
            .bind(updated.updated_at.to_rfc3339())
            .bind(updated.id.to_string())
            .bind(updated.scope_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn stage_conversation_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("conversation", "delete_conversation", id)
            .await
    }

    pub async fn append_message(&self, message: &Message) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        if insert_message_immutable(&mut transaction, message).await? {
            sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                .bind(message.created_at.to_rfc3339())
                .bind(message.conversation_id.to_string())
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
