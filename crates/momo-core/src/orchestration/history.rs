//! Admit durable message evidence through the same text budget as its anchor.
//! A tool exchange is expanded only when its complete budget placeholder fits.
use crate::{
    ChatFunctionCall, ChatInput, ChatToolCall, GatewayContentPart, GatewayImageUrl, GatewayMessage,
    GatewayMessageRole,
};
use momo_domain::{
    Message,
    conversation::{MessageContext, MessageContextPayload},
};
use std::collections::HashMap;

struct Entry {
    messages: Vec<GatewayMessage>,
    estimated_input_tokens: usize,
    image_count: usize,
    tool_exchanges: usize,
}

impl Entry {
    fn new(message: &Message, payload: &MessageContextPayload) -> Self {
        let original = ChatInput {
            role: message.role.clone(),
            content: message.content.clone(),
        };
        let (messages, image_count, tool_exchanges) = match payload {
            MessageContextPayload::UserImages { images } => {
                let mut parts = vec![GatewayContentPart::Text {
                    text: message.content.clone(),
                }];
                parts.extend(images.iter().map(|image| GatewayContentPart::ImageUrl {
                    image_url: GatewayImageUrl {
                        url: image.image_url.clone(),
                        detail: image.detail.clone(),
                    },
                }));
                let mut gateway = GatewayMessage::from(&original);
                gateway.content = Some(crate::GatewayMessageContent::Parts(parts));
                (vec![gateway], images.len(), 0)
            }
            MessageContextPayload::ToolExchanges { exchanges } => {
                let mut messages = vec![GatewayMessage {
                    role: GatewayMessageRole::Assistant,
                    content: None,
                    tool_call_id: None,
                    tool_calls: exchanges
                        .iter()
                        .map(|call| ChatToolCall {
                            id: call.call_id.clone(),
                            call_type: "function".into(),
                            function: ChatFunctionCall {
                                name: call.name.clone(),
                                arguments: call.arguments.clone(),
                            },
                        })
                        .collect(),
                }];
                messages.extend(exchanges.iter().map(|call| GatewayMessage {
                    role: GatewayMessageRole::Tool,
                    content: Some(call.output.clone().into()),
                    tool_call_id: Some(call.call_id.clone()),
                    tool_calls: vec![],
                }));
                if !message.content.is_empty() {
                    messages.push(GatewayMessage::from(&original));
                }
                (messages, 0, exchanges.len())
            }
        };
        let estimated_input_tokens = messages
            .iter()
            .map(crate::context::estimate_gateway_message_tokens)
            .sum();
        Self {
            messages,
            estimated_input_tokens,
            image_count,
            tool_exchanges,
        }
    }
}

pub(super) struct HistoryContext {
    pub(super) messages: Vec<ChatInput>,
    entries: HashMap<String, Entry>,
    omitted_blocks: usize,
}

impl HistoryContext {
    pub(super) fn new(
        messages: Vec<Message>,
        contexts: Vec<MessageContext>,
        supports_images: bool,
        available_images: usize,
    ) -> Self {
        let mut contexts: HashMap<_, _> = contexts
            .into_iter()
            .map(|c| (c.message_id, c.payload))
            .collect();
        let mut remaining = available_images;
        let mut omitted_blocks = 0;
        // Keep the newest image groups whole; current input gets first claim.
        for message in messages.iter().rev() {
            if let Some(MessageContextPayload::UserImages { images }) = contexts.get(&message.id) {
                if !supports_images || images.len() > remaining {
                    contexts.remove(&message.id);
                    omitted_blocks += 1;
                } else {
                    remaining -= images.len();
                }
            }
        }
        let mut entries = HashMap::new();
        let messages = messages
            .into_iter()
            .map(|message| {
                let mut content = message.content.clone();
                if let Some(payload) = contexts.remove(&message.id) {
                    let marker = format!("\n<momo-history-context id=\"{}\">\n", message.id);
                    let estimate = match &payload {
                        MessageContextPayload::UserImages { images } => {
                            format!("Previous user images: {}", images.len())
                        }
                        MessageContextPayload::ToolExchanges { exchanges } => {
                            serde_json::to_string(exchanges).expect("serializable tool evidence")
                        }
                    };
                    content.push_str(&marker);
                    content.push_str(&estimate);
                    content.push_str("\n</momo-history-context>");
                    entries.insert(content.clone(), Entry::new(&message, &payload));
                }
                ChatInput {
                    role: message.role,
                    content,
                }
            })
            .collect();
        Self {
            messages,
            entries,
            omitted_blocks,
        }
    }

    pub(super) fn prepare_context(
        &self,
        request: crate::ContextRequest<'_>,
    ) -> crate::PreparedContext {
        crate::context::prepare_context_with_message_tokens(request, &|message| {
            self.entries.get(&message.content).map_or_else(
                || crate::estimate_text_tokens(&message.content).saturating_add(6),
                |entry| entry.estimated_input_tokens,
            )
        })
    }

    pub(super) fn expand(
        &self,
        prepared: &crate::PreparedContext,
    ) -> (Vec<GatewayMessage>, serde_json::Value) {
        let mut output = Vec::new();
        let mut image_count = 0;
        let mut tool_exchanges = 0;
        let mut admitted_blocks = 0;
        // Preparation retains one suffix of the source history, possibly with
        // a generated system message in front. Match by source position, never
        // by a fragment of truncated text: trimming can preserve both ends.
        let retained = self
            .messages
            .len()
            .saturating_sub(prepared.omitted_messages);
        let leading = prepared.messages.len().saturating_sub(retained);
        let first = self.messages.len() - retained;
        for (index, message) in prepared.messages.iter().enumerate() {
            let source = index
                .checked_sub(leading)
                .and_then(|index| self.messages.get(first + index));
            let Some(entry) = source.and_then(|source| self.entries.get(&source.content)) else {
                output.push(GatewayMessage::from(message));
                continue;
            };
            if source.is_some_and(|source| source.content != message.content) {
                // Drop the whole shortened anchor/evidence block. A retained
                // JSON tail must never leak as unpaired tool evidence.
                continue;
            }
            admitted_blocks += 1;
            output.extend(entry.messages.iter().cloned());
            image_count += entry.image_count;
            tool_exchanges += entry.tool_exchanges;
        }
        (
            output,
            serde_json::json!({"image_count":image_count,"tool_exchanges":tool_exchanges,
            "omitted_blocks":self.omitted_blocks + self.entries.len() - admitted_blocks,
            "excludes_image_tokens":image_count>0}),
        )
    }
}

pub(super) fn tool_context(input: &crate::ResponseInput) -> Option<MessageContextPayload> {
    let crate::ResponseInput::Items(items) = input else {
        return None;
    };
    let calls: HashMap<_, _> = items
        .iter()
        .filter_map(|item| {
            if let crate::ResponseInputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } = item
            {
                Some((call_id, (name, arguments)))
            } else {
                None
            }
        })
        .collect();
    let exchanges: Vec<_> = items
        .iter()
        .filter_map(|item| {
            if let crate::ResponseInputItem::FunctionCallOutput { call_id, output } = item {
                calls.get(call_id).map(|(name, arguments)| {
                    momo_domain::conversation::StoredToolExchange {
                        call_id: call_id.clone(),
                        name: (*name).clone(),
                        arguments: (*arguments).clone(),
                        output: output.clone(),
                    }
                })
            } else {
                None
            }
        })
        .collect();
    (!exchanges.is_empty()).then_some(MessageContextPayload::ToolExchanges { exchanges })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContextBudget, ContextRequest, ContextSections};
    use momo_domain::{
        MessageRole,
        conversation::{StoredImage, StoredToolExchange},
    };

    #[test]
    fn tool_budget_counts_wire_messages_even_when_the_placeholder_is_smaller() {
        let anchor = Message {
            id: uuid::Uuid::now_v7(),
            conversation_id: uuid::Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: "result".into(),
            created_at: chrono::Utc::now(),
        };
        let context = MessageContext {
            message_id: anchor.id,
            payload: MessageContextPayload::ToolExchanges {
                exchanges: (0..14)
                    .map(|index| StoredToolExchange {
                        call_id: format!("call_{index}"),
                        name: "inspect".into(),
                        arguments: "{}".into(),
                        output: "ok".into(),
                    })
                    .collect(),
            },
        };
        let history = HistoryContext::new(vec![anchor], vec![context], false, 0);
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget::default(),
        });
        let (gateway, audit) = history.expand(&prepared);
        let actual: usize = gateway
            .iter()
            .map(crate::context::estimate_gateway_message_tokens)
            .sum();
        assert_eq!(prepared.estimated_input_tokens, actual);
        assert_eq!(audit["tool_exchanges"], 14);

        // The textual placeholder fits, but its complete native tool group does not.
        // A newest evidence anchor must still be omitted atomically in this case.
        let placeholder_tokens = crate::estimate_text_tokens(&history.messages[0].content) + 6;
        assert!(placeholder_tokens < actual);
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget {
                context_window: placeholder_tokens + crate::context::CONTEXT_SAFETY_MARGIN + 32,
                reserve_output_tokens: 32,
            },
        });
        let (gateway, audit) = history.expand(&prepared);
        assert!(gateway.is_empty());
        assert_eq!(audit["omitted_blocks"], 1);
        assert!(prepared.estimated_input_tokens <= placeholder_tokens);
    }

    #[test]
    fn oversized_tool_evidence_is_omitted_as_a_whole_through_the_real_budget() {
        let anchor = Message {
            id: uuid::Uuid::now_v7(),
            conversation_id: uuid::Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: "result".into(),
            created_at: chrono::Utc::now(),
        };
        let context = MessageContext {
            message_id: anchor.id,
            payload: MessageContextPayload::ToolExchanges {
                exchanges: vec![StoredToolExchange {
                    call_id: "large".into(),
                    name: "inspect".into(),
                    arguments: "{}".into(),
                    output: "long evidence ".repeat(2000),
                }],
            },
        };
        let followup = Message {
            id: uuid::Uuid::now_v7(),
            conversation_id: anchor.conversation_id,
            role: MessageRole::User,
            content: "What happened?".into(),
            created_at: chrono::Utc::now(),
        };
        let history = HistoryContext::new(
            vec![anchor.clone(), followup],
            vec![context.clone()],
            false,
            0,
        );
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget {
                context_window: 512,
                reserve_output_tokens: 64,
            },
        });
        let (gateway, audit) = history.expand(&prepared);
        assert_eq!(audit["omitted_blocks"], 1);
        assert_eq!(audit["tool_exchanges"], 0);
        assert!(gateway.iter().all(
            |message| message.role != GatewayMessageRole::Tool && message.tool_calls.is_empty()
        ));
        assert!(prepared.estimated_input_tokens <= 320);
        assert_eq!(gateway.last().unwrap().role, GatewayMessageRole::User);

        // A truncated newest anchor must not leak its placeholder either.
        let history = HistoryContext::new(vec![anchor.clone()], vec![context.clone()], false, 0);
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget {
                context_window: 512,
                reserve_output_tokens: 64,
            },
        });
        let (gateway, audit) = history.expand(&prepared);
        assert_eq!(audit["omitted_blocks"], 1);
        let json = serde_json::to_string(&gateway).unwrap();
        assert!(!json.contains("momo-history-context"));
        assert!(!json.contains("long evidence"));
        assert!(gateway.is_empty());

        // Head-and-tail truncation can remove the opening placeholder marker
        // entirely while keeping tool JSON at the end. Match the source anchor.
        let anchor = Message {
            content: "long assistant anchor ".repeat(2000),
            ..anchor
        };
        let history = HistoryContext::new(vec![anchor], vec![context], false, 0);
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget {
                context_window: 512,
                reserve_output_tokens: 64,
            },
        });
        let (gateway, audit) = history.expand(&prepared);
        assert!(gateway.is_empty());
        assert_eq!(audit["omitted_blocks"], 1);
    }

    #[test]
    fn history_images_obey_capability_and_reserve_quota_for_current_input() {
        let conversation_id = uuid::Uuid::now_v7();
        let messages: Vec<_> = (0..2)
            .map(|index| Message {
                id: uuid::Uuid::now_v7(),
                conversation_id,
                role: MessageRole::User,
                content: format!("image-{index}"),
                created_at: chrono::Utc::now(),
            })
            .collect();
        let contexts: Vec<_> = messages
            .iter()
            .map(|message| MessageContext {
                message_id: message.id,
                payload: MessageContextPayload::UserImages {
                    images: vec![StoredImage {
                        image_url: format!("https://example.test/{}.png", message.content),
                        detail: None,
                    }],
                },
            })
            .collect();
        let history = HistoryContext::new(messages.clone(), contexts.clone(), true, 1);
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget::default(),
        });
        let (gateway, audit) = history.expand(&prepared);
        let json = serde_json::to_string(&gateway).unwrap();
        assert!(json.contains("image-1.png"));
        assert!(!json.contains("image-0.png"));
        assert_eq!(audit["image_count"], 1);
        assert_eq!(audit["omitted_blocks"], 1);
        let history = HistoryContext::new(messages, contexts, false, 8);
        let prepared = history.prepare_context(ContextRequest {
            sections: ContextSections::default(),
            messages: &history.messages,
            budget: ContextBudget::default(),
        });
        let (_, audit) = history.expand(&prepared);
        assert_eq!(audit["image_count"], 0);
        assert_eq!(audit["omitted_blocks"], 2);
    }
}
