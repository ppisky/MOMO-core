//! SQLite-backed local persistence for the local MOMO Rust core.

use std::{collections::HashMap, path::Path, str::FromStr};

use chrono::{DateTime, Utc};
use momo_domain::{CharacterCard, Conversation, Message, MessageRole};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RecentlyDeletedItem {
    pub object_type: String,
    pub object_id: String,
    pub display_name: Option<String>,
    pub deleted_at: DateTime<Utc>,
    pub can_restore: bool,
}

/// A disposable semantic-graph vector cache entry. The source graph remains
/// authoritative; callers must compare both hashes and vector space identity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct NsgVectorRecord {
    pub owner_id: Uuid,
    pub node_id: String,
    pub source_hash: String,
    pub vector_space_id: String,
    pub dimension: usize,
    pub vector: Vec<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NsgVectorStatus {
    pub vector_space_id: String,
    pub dimension: Option<usize>,
    pub node_count: usize,
    pub indexed_count: usize,
    pub stale_count: usize,
    pub missing_count: usize,
}

pub const DEFAULT_NSG_VECTOR_TOP_K: usize = 64;
pub const MAX_NSG_VECTOR_TOP_K: usize = 512;

#[allow(async_fn_in_trait)]
pub trait NsgVectorStore {
    async fn upsert_nsg_vectors(&self, records: &[NsgVectorRecord]) -> Result<(), StorageError>;

    async fn rank_nsg_vectors(
        &self,
        owner_id: Uuid,
        vector_space_id: &str,
        query_vector: &[f64],
        current_hashes: &HashMap<String, String>,
        limit: usize,
    ) -> Result<Vec<String>, StorageError>;

    async fn nsg_vector_status(
        &self,
        owner_id: Uuid,
        vector_space_id: &str,
        current_hashes: &HashMap<String, String>,
    ) -> Result<NsgVectorStatus, StorageError>;
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPatchReviewStatus {
    Pending,
    Approved,
    Rejected,
    Failed,
}

impl MemoryPatchReviewStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct MemoryPatchReview {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub conversation_id: String,
    pub patch_yaml: String,
    pub targets: Vec<String>,
    pub operation_count: i64,
    pub review_mode: String,
    pub status: MemoryPatchReviewStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
    pub error: Option<String>,
}

pub use momo_domain::SCHEMA_GENERATION;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("local database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("local database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("invalid persisted UUID: {0}")]
    Uuid(#[from] uuid::Error),
    #[error("invalid persisted timestamp: {0}")]
    Timestamp(#[from] chrono::ParseError),
    #[error("invalid persisted JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid persisted message role")]
    MessageRole,
    #[error("message {0} already exists with different immutable content")]
    ImmutableMessageConflict(Uuid),
    #[error("message was not found: {0}")]
    MessageNotFound(Uuid),
    #[error("invalid persisted memory patch review status: {0}")]
    MemoryPatchReviewStatus(String),
    #[error("invalid semantic-graph vector: {0}")]
    InvalidNsgVector(String),
}

#[derive(Debug, Clone)]
pub struct LocalStore {
    pool: SqlitePool,
}

mod local;

mod vector;

fn recently_deleted_from_row(row: &SqliteRow) -> Result<RecentlyDeletedItem, StorageError> {
    let object_type: String = row.try_get("object_type")?;
    let payload: Option<String> = row.try_get("payload")?;
    let display_name = payload
        .as_deref()
        .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
        .and_then(|payload| match object_type.as_str() {
            "character" => payload
                .get("character")
                .and_then(|value| value.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            "conversation" => payload
                .get("conversation")
                .and_then(|value| value.get("title"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            _ => None,
        });
    Ok(RecentlyDeletedItem {
        object_type,
        object_id: row.try_get("object_id")?,
        display_name,
        deleted_at: parse_timestamp(row.try_get("deleted_at")?)?,
        can_restore: payload.is_some(),
    })
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DeletedConversationSnapshot {
    conversation: Conversation,
    messages: Vec<Message>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DeletedCharacterSnapshot {
    character: CharacterCard,
    conversation_ids: Vec<Uuid>,
}

fn memory_patch_review_from_row(row: &SqliteRow) -> Result<MemoryPatchReview, StorageError> {
    let status: String = row.try_get("status")?;
    let status = match status.as_str() {
        "pending" => MemoryPatchReviewStatus::Pending,
        "approved" => MemoryPatchReviewStatus::Approved,
        "rejected" => MemoryPatchReviewStatus::Rejected,
        "failed" => MemoryPatchReviewStatus::Failed,
        other => return Err(StorageError::MemoryPatchReviewStatus(other.to_owned())),
    };
    let resolved_at: Option<String> = row.try_get("resolved_at")?;
    Ok(MemoryPatchReview {
        id: Uuid::parse_str(row.try_get("id")?)?,
        owner_id: Uuid::parse_str(row.try_get("owner_id")?)?,
        conversation_id: row.try_get("conversation_id")?,
        patch_yaml: row.try_get("patch_yaml")?,
        targets: serde_json::from_str(row.try_get("targets")?)?,
        operation_count: row.try_get("operation_count")?,
        review_mode: row.try_get("review_mode")?,
        status,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        resolved_at: resolved_at.as_deref().map(parse_timestamp).transpose()?,
        result: row.try_get("result")?,
        error: row.try_get("error")?,
    })
}

async fn insert_message_immutable(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    message: &Message,
) -> Result<bool, StorageError> {
    let inserted = sqlx::query(
        r#"INSERT INTO messages (id, conversation_id, role, content, created_at)
        VALUES (?,?,?,?,?) ON CONFLICT(id) DO NOTHING"#,
    )
    .bind(message.id.to_string())
    .bind(message.conversation_id.to_string())
    .bind(message.role.as_str())
    .bind(&message.content)
    .bind(message.created_at.to_rfc3339())
    .execute(&mut **transaction)
    .await?
    .rows_affected()
        > 0;
    if inserted {
        return Ok(true);
    }

    let row = sqlx::query("SELECT * FROM messages WHERE id=?")
        .bind(message.id.to_string())
        .fetch_one(&mut **transaction)
        .await?;
    if message_from_row(&row)? != *message {
        return Err(StorageError::ImmutableMessageConflict(message.id));
    }
    Ok(false)
}

fn character_from_row(row: &SqliteRow) -> Result<CharacterCard, StorageError> {
    Ok(CharacterCard {
        id: Uuid::parse_str(row.try_get("id")?)?,
        owner_id: Uuid::parse_str(row.try_get("owner_id")?)?,
        name: row.try_get("name")?,
        version: row.try_get("version")?,
        author_name: row.try_get("author_name")?,
        author_url: row.try_get("author_url")?,
        character_markdown: row.try_get("character_markdown")?,
        user_markdown: row.try_get("user_markdown")?,
        opening_markdown: row.try_get("opening_markdown")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        updated_at: parse_timestamp(row.try_get("updated_at")?)?,
    })
}

fn conversation_from_row(row: &SqliteRow) -> Result<Conversation, StorageError> {
    let character_id: Option<String> = row.try_get("character_id")?;
    Ok(Conversation {
        id: Uuid::parse_str(row.try_get("id")?)?,
        owner_id: Uuid::parse_str(row.try_get("owner_id")?)?,
        character_id: character_id.map(|id| Uuid::parse_str(&id)).transpose()?,
        title: row.try_get("title")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        updated_at: parse_timestamp(row.try_get("updated_at")?)?,
    })
}

fn message_from_row(row: &SqliteRow) -> Result<Message, StorageError> {
    let role: String = row.try_get("role")?;
    Ok(Message {
        id: Uuid::parse_str(row.try_get("id")?)?,
        conversation_id: Uuid::parse_str(row.try_get("conversation_id")?)?,
        role: MessageRole::try_from(role.as_str()).map_err(|_| StorageError::MessageRole)?,
        content: row.try_get("content")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
    })
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

#[cfg(test)]
mod tests;
