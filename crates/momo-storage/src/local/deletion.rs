use super::*;

impl LocalStore {
    pub async fn is_tombstoned(&self, object_type: &str, id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM local_tombstones WHERE object_type=? AND object_id=?",
        )
        .bind(object_type)
        .bind(id.to_string())
        .fetch_one(&self.pool)
        .await?
            > 0)
    }

    pub(super) async fn stage_delete(
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
                let context_rows = sqlx::query("SELECT x.message_id,x.payload_json FROM message_context x JOIN messages m ON m.id=x.message_id WHERE m.conversation_id=?")
                    .bind(id.to_string()).fetch_all(&mut *transaction).await?;
                let contexts = context_rows
                    .iter()
                    .map(|row| -> Result<_, StorageError> {
                        Ok(momo_domain::conversation::MessageContext {
                            message_id: Uuid::parse_str(row.try_get("message_id")?)?,
                            payload: serde_json::from_str(row.try_get("payload_json")?)?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let payload = conversation
                    .map(|conversation| DeletedConversationSnapshot {
                        conversation,
                        messages,
                        contexts,
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
                let context: Option<String> = sqlx::query_scalar(
                    "SELECT payload_json FROM message_context WHERE message_id=?",
                )
                .bind(id.to_string())
                .fetch_optional(&mut *transaction)
                .await?;
                let context = context
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?;
                sqlx::query("DELETE FROM messages WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                message
                    .map(|message| {
                        serde_json::to_string(&DeletedMessageSnapshot { message, context })
                    })
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
