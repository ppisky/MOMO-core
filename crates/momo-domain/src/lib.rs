//! Shared MOMO domain types without storage, network, or UI dependencies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Current in-repository domain schema generation.
pub const SCHEMA_GENERATION: u32 = 2;

/// Generates a time-ordered UUIDv7 suitable for persisted MOMO entities.
#[must_use]
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub public_uid: i64,
    pub email: String,
    pub display_name: String,
    pub is_official: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterCard {
    pub id: Uuid,
    pub scope_id: Uuid,
    pub name: String,
    pub version: String,
    #[serde(alias = "author_display_name")]
    pub author_name: String,
    #[serde(default)]
    pub author_url: Option<String>,
    pub character_markdown: String,
    pub user_markdown: String,
    #[serde(default)]
    pub opening_markdown: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl MessageRole {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

impl TryFrom<&str> for MessageRole {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            _ => Err("unsupported message role"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conversation {
    pub id: Uuid,
    pub scope_id: Uuid,
    pub character_id: Option<Uuid>,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncObject {
    pub object_id: String,
    pub category: String,
    pub schema_version: i32,
    pub revision: i64,
    pub change_sequence: i64,
    pub updated_at: DateTime<Utc>,
    pub device_id: String,
    pub deleted: bool,
    pub payload: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_uuid_v7() {
        let id = new_id();
        assert_eq!(id.get_version_num(), 7);
    }

    #[test]
    fn roles_use_wire_values() {
        assert_eq!(MessageRole::Assistant.as_str(), "assistant");
        assert_eq!(MessageRole::try_from("user"), Ok(MessageRole::User));
    }
}
