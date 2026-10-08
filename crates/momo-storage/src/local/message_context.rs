use super::*;
use momo_domain::conversation::{MessageContext, MessageContextPayload};

impl LocalStore {
    /// Completed tool continuation responses since the latest ordinary user input.
    pub async fn tool_continuation_count(
        &self,
        scope_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<usize, StorageError> {
        let count: i64 = sqlx::query_scalar(r#"
            WITH recent_user AS (
                SELECT m.created_at,m.id FROM messages m JOIN conversations c ON c.id=m.conversation_id
                WHERE m.conversation_id=? AND c.scope_id=? AND m.role='user'
                ORDER BY m.created_at DESC,m.id DESC LIMIT 1
            )
            SELECT COUNT(*) FROM message_context x JOIN messages m ON m.id=x.message_id
            JOIN conversations c ON c.id=m.conversation_id
            WHERE m.conversation_id=? AND c.scope_id=? AND m.role='assistant'
                AND json_extract(x.payload_json,'$.type')='tool_exchanges'
                AND (NOT EXISTS(SELECT 1 FROM recent_user) OR (m.created_at,m.id)>(SELECT created_at,id FROM recent_user))
        "#).bind(conversation_id.to_string()).bind(scope_id.to_string())
            .bind(conversation_id.to_string()).bind(scope_id.to_string()).fetch_one(&self.pool).await?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }
    /// The selected IDs come from the bounded history window. Never load older
    /// image data or tool outputs merely to discard them in the application.
    pub async fn message_contexts_for_scope(
        &self,
        scope_id: Uuid,
        message_ids: &[Uuid],
    ) -> Result<Vec<MessageContext>, StorageError> {
        let mut contexts = Vec::new();
        for chunk in message_ids.chunks(400) {
            if chunk.is_empty() {
                continue;
            }
            let mut query = sqlx::QueryBuilder::new(
                "SELECT x.message_id,x.payload_json,m.role FROM message_context x JOIN messages m ON m.id=x.message_id JOIN conversations c ON c.id=m.conversation_id WHERE c.scope_id=",
            );
            query
                .push_bind(scope_id.to_string())
                .push(" AND x.message_id IN (");
            let mut ids = query.separated(",");
            for id in chunk {
                ids.push_bind(id.to_string());
            }
            ids.push_unseparated(")");
            for row in query.build().fetch_all(&self.pool).await? {
                let payload: MessageContextPayload =
                    serde_json::from_str(row.try_get("payload_json")?)?;
                let role = MessageRole::try_from(row.try_get::<&str, _>("role")?)
                    .map_err(|e| StorageError::Database(sqlx::Error::Protocol(e.into())))?;
                payload
                    .validate(role)
                    .map_err(|e| StorageError::Database(sqlx::Error::Protocol(e.into())))?;
                contexts.push(MessageContext {
                    message_id: Uuid::parse_str(row.try_get("message_id")?)?,
                    payload,
                });
            }
        }
        contexts.sort_by_key(|context| context.message_id);
        Ok(contexts)
    }
}

pub(super) async fn insert_message_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    message_id: Uuid,
    payload: &MessageContextPayload,
    replace: bool,
) -> Result<(), StorageError> {
    let role: String = sqlx::query_scalar("SELECT role FROM messages WHERE id=?")
        .bind(message_id.to_string())
        .fetch_one(&mut **transaction)
        .await?;
    payload
        .validate(
            MessageRole::try_from(role.as_str())
                .map_err(|e| StorageError::Database(sqlx::Error::Protocol(e.into())))?,
        )
        .map_err(|e| StorageError::Database(sqlx::Error::Protocol(e.into())))?;
    let json = serde_json::to_string(payload)?;
    if !replace {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT payload_json FROM message_context WHERE message_id=?")
                .bind(message_id.to_string())
                .fetch_optional(&mut **transaction)
                .await?;
        if existing.as_ref().is_some_and(|value| value != &json) {
            return Err(StorageError::ImmutableMessageConflict(message_id));
        }
    }
    sqlx::query("INSERT INTO message_context(message_id,payload_json) VALUES (?,?) ON CONFLICT(message_id) DO UPDATE SET payload_json=excluded.payload_json")
        .bind(message_id.to_string()).bind(json).execute(&mut **transaction).await?;
    Ok(())
}
