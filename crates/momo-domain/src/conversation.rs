//! Durable evidence attached to an actual conversation message.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MessageContext {
    pub message_id: Uuid,
    pub payload: MessageContextPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageContextPayload {
    UserImages { images: Vec<StoredImage> },
    ToolExchanges { exchanges: Vec<StoredToolExchange> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoredImage {
    pub image_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoredToolExchange {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub output: String,
}

impl MessageContextPayload {
    pub fn validate(&self, role: crate::MessageRole) -> Result<(), &'static str> {
        match self {
            Self::UserImages { images } => {
                if role != crate::MessageRole::User || images.is_empty() || images.len() > 8 {
                    return Err(
                        "image context must belong to a user message and contain 1..=8 images",
                    );
                }
                for image in images {
                    if image.image_url.len() > 2 * 1024 * 1024
                        || !(image.image_url.starts_with("https://")
                            || image.image_url.starts_with("http://")
                            || image.image_url.starts_with("data:image/"))
                        || image
                            .detail
                            .as_deref()
                            .is_some_and(|v| !matches!(v, "auto" | "low" | "high"))
                    {
                        return Err("invalid stored image reference");
                    }
                }
            }
            Self::ToolExchanges { exchanges } => {
                if role != crate::MessageRole::Assistant || exchanges.is_empty() {
                    return Err(
                        "tool context must belong to an assistant message and contain exchanges",
                    );
                }
                let mut ids = std::collections::HashSet::new();
                let mut bytes = 0usize;
                for exchange in exchanges {
                    bytes = bytes
                        .saturating_add(exchange.arguments.len())
                        .saturating_add(exchange.output.len())
                        .saturating_add(exchange.call_id.len())
                        .saturating_add(exchange.name.len());
                    if exchange.call_id.trim().is_empty()
                        || exchange.call_id.len() > 256
                        || exchange.name.is_empty()
                        || exchange.name.len() > 128
                        || !exchange.name.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
                        })
                        || !ids.insert(&exchange.call_id)
                        || exchange.arguments.len() > 1024 * 1024
                        || exchange.output.len() > 1024 * 1024
                        || bytes > 2 * 1024 * 1024
                    {
                        return Err("invalid stored tool exchange");
                    }
                }
            }
        }
        Ok(())
    }
}
