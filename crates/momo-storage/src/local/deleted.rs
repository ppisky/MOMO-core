use super::*;

impl LocalStore {
    pub async fn tombstone_ids(&self, object_type: &str) -> Result<Vec<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT object_id FROM local_tombstones WHERE object_type=? ORDER BY object_id",
        )
        .bind(object_type)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn recently_deleted(
        &self,
        limit: u32,
    ) -> Result<Vec<RecentlyDeletedItem>, StorageError> {
        let rows = sqlx::query(
            "SELECT object_type, object_id, deleted_at, payload FROM local_tombstones WHERE hidden=0 ORDER BY deleted_at DESC, object_type, object_id LIMIT ?",
        )
        .bind(i64::from(limit.min(1_000)))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(recently_deleted_from_row).collect()
    }

    pub async fn restore_recently_deleted(
        &self,
        object_type: &str,
        object_id: Uuid,
    ) -> Result<bool, StorageError> {
        let payload: Option<String> = sqlx::query_scalar(
            "SELECT payload FROM local_tombstones WHERE object_type=? AND object_id=?",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        let Some(payload) = payload else {
            return Ok(false);
        };

        let mut transaction = self.pool.begin().await?;
        match object_type {
            "character" => {
                let snapshot: DeletedCharacterSnapshot = serde_json::from_str(&payload)?;
                let card = snapshot.character;
                sqlx::query(
                    r#"INSERT INTO character_cards
                    (id, scope_id, name, version, description, language, tags, author_uid,
                     author_display_name, author_name, author_url, character_markdown, user_markdown,
                     opening_markdown, created_at, updated_at)
                    VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(id) DO UPDATE SET scope_id=excluded.scope_id,
                      name=excluded.name, version=excluded.version,
                      author_display_name=excluded.author_display_name,
                      author_name=excluded.author_name, author_url=excluded.author_url,
                      character_markdown=excluded.character_markdown,
                      user_markdown=excluded.user_markdown,
                      opening_markdown=excluded.opening_markdown,
                      created_at=excluded.created_at, updated_at=excluded.updated_at"#,
                )
                .bind(card.id.to_string())
                .bind(card.scope_id.to_string())
                .bind(&card.name)
                .bind(&card.version)
                .bind("")
                .bind("")
                .bind("[]")
                .bind("")
                .bind(&card.author_name)
                .bind(&card.author_name)
                .bind(&card.author_url)
                .bind(&card.character_markdown)
                .bind(&card.user_markdown)
                .bind(&card.opening_markdown)
                .bind(card.created_at.to_rfc3339())
                .bind(card.updated_at.to_rfc3339())
                .execute(&mut *transaction)
                .await?;
                for conversation_id in snapshot.conversation_ids {
                    sqlx::query(
                        "UPDATE conversations SET character_id=? WHERE id=? AND character_id IS NULL",
                    )
                    .bind(card.id.to_string())
                    .bind(conversation_id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                }
            }
            "conversation" => {
                let snapshot: DeletedConversationSnapshot = serde_json::from_str(&payload)?;
                let conversation = snapshot.conversation;
                sqlx::query(
                    r#"INSERT INTO conversations
                    (id, scope_id, character_id, title, created_at, updated_at)
                    VALUES (?,?,?,?,?,?)
                    ON CONFLICT(id) DO UPDATE SET scope_id=excluded.scope_id,
                      character_id=excluded.character_id, title=excluded.title,
                      created_at=excluded.created_at, updated_at=excluded.updated_at"#,
                )
                .bind(conversation.id.to_string())
                .bind(conversation.scope_id.to_string())
                .bind(conversation.character_id.map(|id| id.to_string()))
                .bind(&conversation.title)
                .bind(conversation.created_at.to_rfc3339())
                .bind(conversation.updated_at.to_rfc3339())
                .execute(&mut *transaction)
                .await?;
                for message in snapshot.messages {
                    insert_message_immutable(&mut transaction, &message).await?;
                }
            }
            "message" => {
                let message: Message = serde_json::from_str(&payload)?;
                if message.id != object_id {
                    return Ok(false);
                }
                insert_message_immutable(&mut transaction, &message).await?;
            }
            _ => return Ok(false),
        }
        sqlx::query("DELETE FROM local_tombstones WHERE object_type=? AND object_id=?")
            .bind(object_type)
            .bind(object_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn purge_recently_deleted(
        &self,
        object_type: &str,
        object_id: Uuid,
    ) -> Result<bool, StorageError> {
        match object_type {
            "character" | "conversation" | "message" => {}
            _ => return Ok(false),
        }
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT 1 FROM local_tombstones WHERE object_type=? AND object_id=? AND hidden=0",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let _ = row;
        let changed = sqlx::query(
            "UPDATE local_tombstones SET payload=NULL, hidden=1 WHERE object_type=? AND object_id=? AND hidden=0",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            > 0;
        transaction.commit().await?;
        Ok(changed)
    }

    pub async fn forget_recently_deleted(
        &self,
        object_type: &str,
        object_id: Uuid,
    ) -> Result<bool, StorageError> {
        if let Some(actual_type) = object_type.strip_prefix("local_only:") {
            let mut transaction = self.pool.begin().await?;
            let changed = sqlx::query(
                "UPDATE local_tombstones SET payload=NULL, hidden=1 WHERE object_type=? AND object_id=? AND hidden=0",
            )
            .bind(actual_type)
            .bind(object_id.to_string())
            .execute(&mut *transaction)
            .await?
            .rows_affected()
                > 0;
            transaction.commit().await?;
            return Ok(changed);
        }
        Ok(sqlx::query(
            "DELETE FROM local_tombstones WHERE object_type=? AND object_id=? AND hidden=0",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }
}
