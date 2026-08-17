use super::*;

impl LocalStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        Self::connect(options).await
    }

    pub async fn in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?
            .foreign_keys(true)
            .shared_cache(true);
        Self::connect(options).await
    }

    async fn connect(options: SqliteConnectOptions) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn save_character(&self, card: &CharacterCard) -> Result<(), StorageError> {
        if self.is_tombstoned("character", card.id).await? {
            return Ok(());
        }
        sqlx::query(
            r#"INSERT INTO character_cards
            (id, scope_id, name, version, description, language, tags, author_uid,
             author_display_name, author_name, author_url, character_markdown, user_markdown,
             opening_markdown, created_at, updated_at)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET
              scope_id=excluded.scope_id, name=excluded.name, version=excluded.version,
              author_display_name=excluded.author_display_name,
              author_name=excluded.author_name, author_url=excluded.author_url,
              character_markdown=excluded.character_markdown, user_markdown=excluded.user_markdown,
              opening_markdown=excluded.opening_markdown,
              updated_at=excluded.updated_at"#,
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_characters(&self) -> Result<Vec<CharacterCard>, StorageError> {
        let rows = sqlx::query("SELECT * FROM character_cards ORDER BY updated_at DESC, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(character_from_row).collect()
    }

    pub async fn list_characters_for_scope(
        &self,
        scope_id: Uuid,
    ) -> Result<Vec<CharacterCard>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM character_cards WHERE scope_id=? ORDER BY updated_at DESC, id",
        )
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(character_from_row).collect()
    }

    pub async fn stage_character(&self, card: &CharacterCard) -> Result<(), StorageError> {
        self.save_character(card).await
    }

    pub async fn stage_character_update(&self, card: &CharacterCard) -> Result<(), StorageError> {
        self.save_character(card).await
    }

    pub async fn stage_character_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("character", "delete_character", id).await
    }

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

    pub async fn save_portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
        document: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"INSERT INTO portable_metadata (kind, object_id, document) VALUES (?,?,?)
            ON CONFLICT(kind, object_id) DO UPDATE SET document=excluded.document"#,
        )
        .bind(kind)
        .bind(object_id)
        .bind(document)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT document FROM portable_metadata WHERE kind=? AND object_id=?",
        )
        .bind(kind)
        .bind(object_id)
        .fetch_optional(&self.pool)
        .await?)
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
            SET status=?, resolved_at=?, result=?, error=?
            WHERE scope_id=? AND id=? AND status='pending'"#,
        )
        .bind(status.as_str())
        .bind(now)
        .bind(result)
        .bind(error)
        .bind(scope_id.to_string())
        .bind(review_id.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.memory_patch_review(scope_id, review_id).await
    }

    async fn is_tombstoned(&self, object_type: &str, id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM local_tombstones WHERE object_type=? AND object_id=?",
        )
        .bind(object_type)
        .bind(id.to_string())
        .fetch_one(&self.pool)
        .await?
            > 0)
    }

    async fn stage_delete(
        &self,
        object_type: &str,
        _operation: &str,
        id: Uuid,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let payload = match object_type {
            "character" => {
                let row = sqlx::query("SELECT * FROM character_cards WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let character = row.as_ref().map(character_from_row).transpose()?;
                let conversation_ids = sqlx::query_scalar::<_, String>(
                    "SELECT id FROM conversations WHERE character_id=? ORDER BY id",
                )
                .bind(id.to_string())
                .fetch_all(&mut *transaction)
                .await?
                .into_iter()
                .map(|id| Uuid::parse_str(&id))
                .collect::<Result<Vec<_>, _>>()?;
                let payload = character
                    .map(|character| DeletedCharacterSnapshot {
                        character,
                        conversation_ids,
                    })
                    .map(|snapshot| serde_json::to_string(&snapshot))
                    .transpose()?;
                sqlx::query("DELETE FROM character_cards WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                payload
            }
            "conversation" => {
                let row = sqlx::query("SELECT * FROM conversations WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let conversation = row.as_ref().map(conversation_from_row).transpose()?;
                let message_rows = sqlx::query(
                    "SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at, id",
                )
                .bind(id.to_string())
                .fetch_all(&mut *transaction)
                .await?;
                let messages = message_rows
                    .iter()
                    .map(message_from_row)
                    .collect::<Result<Vec<_>, _>>()?;
                let payload = conversation
                    .map(|conversation| DeletedConversationSnapshot {
                        conversation,
                        messages,
                    })
                    .map(|snapshot| serde_json::to_string(&snapshot))
                    .transpose()?;
                sqlx::query("DELETE FROM conversations WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                payload
            }
            "message" => {
                let row = sqlx::query("SELECT * FROM messages WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let message = row.as_ref().map(message_from_row).transpose()?;
                sqlx::query("DELETE FROM messages WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                message
                    .map(|message| serde_json::to_string(&message))
                    .transpose()?
            }
            _ => unreachable!("stage_delete is private and uses known object types"),
        };
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO local_tombstones (object_type, object_id, deleted_at, payload)
            VALUES (?,?,?,?)
            ON CONFLICT(object_type, object_id) DO UPDATE SET
              deleted_at=excluded.deleted_at,
              payload=COALESCE(excluded.payload, local_tombstones.payload),
              hidden=0"#,
        )
        .bind(object_type)
        .bind(id.to_string())
        .bind(&now)
        .bind(payload)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}
