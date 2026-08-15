use momo_domain::MessageRole;
use serde::{Deserialize, Serialize};

use crate::{ChatInput, TokenizerProfile};

const CONTEXT_SAFETY_MARGIN: usize = 128;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_window: usize,
    pub reserve_output_tokens: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            context_window: 8_192,
            reserve_output_tokens: 1_024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedContext {
    pub messages: Vec<ChatInput>,
    pub estimated_input_tokens: usize,
    pub omitted_messages: usize,
    /// Number of retained messages whose content had to be shortened.  This is
    /// separate from `omitted_messages` so the UI can describe the actual loss.
    pub truncated_messages: usize,
}

/// Builds a deterministic prompt and retains the newest complete messages that
/// fit the configured input budget. The estimator deliberately over-counts CJK
/// text so an unknown tokenizer is less likely to overflow the provider limit.
#[must_use]
pub fn prepare_context(
    character_markdown: &str,
    user_markdown: &str,
    memory_markdown: &str,
    messages: &[ChatInput],
    budget: ContextBudget,
) -> PreparedContext {
    prepare_context_with_counter(
        character_markdown,
        user_markdown,
        memory_markdown,
        messages,
        budget,
        &estimate_text_tokens,
    )
}

#[must_use]
pub fn prepare_context_with_state(
    character_markdown: &str,
    user_markdown: &str,
    memory_markdown: &str,
    state_context: &str,
    nsg_markdown: &str,
    messages: &[ChatInput],
    budget: ContextBudget,
) -> PreparedContext {
    prepare_context_with_counter_and_state(
        character_markdown,
        user_markdown,
        memory_markdown,
        state_context,
        nsg_markdown,
        messages,
        budget,
        &estimate_text_tokens,
    )
}

/// Equivalent to [`prepare_context`] but uses the endpoint's declared exact
/// tokenizer when available. Unknown model mappings safely use the conservative
/// estimator defined by [`TokenizerProfile::count_or_conservative`].
#[must_use]
pub fn prepare_context_with_tokenizer(
    character_markdown: &str,
    user_markdown: &str,
    memory_markdown: &str,
    messages: &[ChatInput],
    budget: ContextBudget,
    tokenizer: &TokenizerProfile,
) -> PreparedContext {
    prepare_context_with_counter(
        character_markdown,
        user_markdown,
        memory_markdown,
        messages,
        budget,
        &|text| tokenizer.count_or_conservative(text),
    )
}

fn prepare_context_with_counter(
    character_markdown: &str,
    user_markdown: &str,
    memory_markdown: &str,
    messages: &[ChatInput],
    budget: ContextBudget,
    counter: &dyn Fn(&str) -> usize,
) -> PreparedContext {
    prepare_context_with_counter_and_state(
        character_markdown,
        user_markdown,
        memory_markdown,
        "",
        "",
        messages,
        budget,
        counter,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_context_with_counter_and_state(
    character_markdown: &str,
    user_markdown: &str,
    memory_markdown: &str,
    state_context: &str,
    nsg_markdown: &str,
    messages: &[ChatInput],
    budget: ContextBudget,
    counter: &dyn Fn(&str) -> usize,
) -> PreparedContext {
    let input_limit = budget
        .context_window
        .saturating_sub(budget.reserve_output_tokens)
        .saturating_sub(CONTEXT_SAFETY_MARGIN)
        .max(1);
    let raw_system_content = system_prompt(
        character_markdown,
        user_markdown,
        memory_markdown,
        state_context,
        nsg_markdown,
    );
    let mut selected = Vec::new();
    let mut used = 0;
    let mut truncated_messages = 0;

    if !raw_system_content.is_empty() {
        // A huge character card must not crowd the newest user turn out of the
        // request entirely.  Reserve its full size when possible and otherwise
        // split the available budget between system context and the newest turn.
        let newest_reserve = messages
            .last()
            .map(|message| estimate_message_tokens_with(message, counter))
            .unwrap_or_default()
            .min(input_limit.div_ceil(2));
        let system_budget = input_limit.saturating_sub(newest_reserve);
        let (system_content, was_truncated) =
            truncate_message_content_with(&raw_system_content, system_budget, counter);
        if was_truncated {
            truncated_messages += 1;
        }
        if !system_content.is_empty() {
            let system = ChatInput {
                role: MessageRole::System,
                content: system_content,
            };
            used = estimate_message_tokens_with(&system, counter);
            selected.push(system);
        }
    }

    let available = input_limit.saturating_sub(used);
    let mut reverse_messages = Vec::new();
    let mut history_tokens = 0_usize;
    for message in messages.iter().rev() {
        let tokens = estimate_message_tokens_with(message, counter);
        if history_tokens.saturating_add(tokens) > available {
            // Always retain a meaningful part of the newest turn.  Older
            // oversized messages remain all-or-nothing so chronology stays
            // deterministic and no hole is introduced into recent history.
            if reverse_messages.is_empty() {
                let remaining = available.saturating_sub(history_tokens);
                let (content, was_truncated) =
                    truncate_message_content_with(&message.content, remaining, counter);
                if !content.is_empty() {
                    let mut shortened = message.clone();
                    shortened.content = content;
                    history_tokens = history_tokens
                        .saturating_add(estimate_message_tokens_with(&shortened, counter));
                    reverse_messages.push(shortened);
                    if was_truncated {
                        truncated_messages += 1;
                    }
                }
            }
            break;
        }
        history_tokens += tokens;
        reverse_messages.push(message.clone());
    }
    reverse_messages.reverse();
    let omitted_messages = messages.len().saturating_sub(reverse_messages.len());
    selected.extend(reverse_messages);

    PreparedContext {
        messages: selected,
        estimated_input_tokens: used.saturating_add(history_tokens),
        omitted_messages,
        truncated_messages,
    }
}

#[must_use]
pub fn estimate_text_tokens(value: &str) -> usize {
    let mut ascii_units = 0usize;
    let mut non_ascii = 0usize;
    for character in value.chars() {
        if character.is_ascii() {
            ascii_units += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii_units.div_ceil(4).saturating_add(non_ascii)
}

fn estimate_message_tokens_with(message: &ChatInput, counter: &dyn Fn(&str) -> usize) -> usize {
    counter(&message.content).saturating_add(6)
}

/// Fits message content into a total message budget (including the fixed wire
/// overhead) while preserving both the beginning and ending of the text.
fn truncate_message_content_with(
    value: &str,
    message_budget: usize,
    counter: &dyn Fn(&str) -> usize,
) -> (String, bool) {
    const MESSAGE_OVERHEAD: usize = 6;
    if value.is_empty() || message_budget <= MESSAGE_OVERHEAD {
        return (String::new(), !value.is_empty());
    }
    let content_budget = message_budget - MESSAGE_OVERHEAD;
    if counter(value) <= content_budget {
        return (value.to_owned(), false);
    }

    let marker = "…";
    if counter(marker) > content_budget {
        return (String::new(), true);
    }
    let characters = value.chars().collect::<Vec<_>>();
    let mut low = 0_usize;
    let mut high = characters.len();
    let mut best = marker.to_owned();
    while low <= high {
        let keep = low + (high - low) / 2;
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let mut candidate = String::with_capacity(keep.saturating_add(marker.len()));
        candidate.extend(characters[..head].iter());
        candidate.push_str(marker);
        if tail > 0 {
            candidate.extend(characters[characters.len() - tail..].iter());
        }
        if counter(&candidate) <= content_budget {
            best = candidate;
            low = keep.saturating_add(1);
        } else if keep == 0 {
            break;
        } else {
            high = keep - 1;
        }
    }
    (best, true)
}

fn system_prompt(
    character_markdown: &str,
    user_markdown: &str,
    memory_markdown: &str,
    state_context: &str,
    nsg_markdown: &str,
) -> String {
    let mut sections = Vec::new();
    if !character_markdown.trim().is_empty() {
        sections.push(format!("# Character\n{}", character_markdown.trim()));
    }
    if !user_markdown.trim().is_empty() {
        sections.push(format!("# User\n{}", user_markdown.trim()));
    }
    if !memory_markdown.trim().is_empty() {
        sections.push(format!("# Relevant Memory\n{}", memory_markdown.trim()));
    }
    if !state_context.trim().is_empty() {
        sections.push(state_context.trim().to_owned());
    }
    if !nsg_markdown.trim().is_empty() {
        sections.push(format!("# Active Lore Context\n{}", nsg_markdown.trim()));
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(content: &str) -> ChatInput {
        ChatInput {
            role: MessageRole::User,
            content: content.to_owned(),
        }
    }

    #[test]
    fn injects_character_and_user_before_history() {
        let prepared = prepare_context(
            "温柔的向导",
            "用户喜欢简洁回答",
            "曾经去过海边",
            &[message("你好")],
            ContextBudget::default(),
        );
        assert_eq!(prepared.messages[0].role, MessageRole::System);
        assert!(prepared.messages[0].content.contains("# Character"));
        assert!(prepared.messages[0].content.contains("# User"));
        assert!(prepared.messages[0].content.contains("# Relevant Memory"));
        assert_eq!(prepared.omitted_messages, 0);
    }

    #[test]
    fn drops_oldest_messages_when_budget_is_full() {
        let messages = vec![message(&"a".repeat(80)), message("newest")];
        let prepared = prepare_context(
            "",
            "",
            "",
            &messages,
            ContextBudget {
                context_window: 160,
                reserve_output_tokens: 8,
            },
        );
        assert_eq!(prepared.messages, vec![message("newest")]);
        assert_eq!(prepared.omitted_messages, 1);
    }

    #[test]
    fn cjk_estimate_is_conservative() {
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("你好"), 2);
    }

    #[test]
    fn exact_tokenizer_profile_drives_reported_budget() {
        let messages = [message("hello world")];
        let prepared = prepare_context_with_tokenizer(
            "",
            "",
            "",
            &messages,
            ContextBudget::default(),
            &TokenizerProfile::Cl100kBase,
        );
        assert_eq!(prepared.estimated_input_tokens, 8);
    }

    #[test]
    fn oversized_system_context_cannot_evict_the_newest_turn() {
        let prepared = prepare_context(
            &"角色设定".repeat(200),
            &"用户设定".repeat(100),
            &"记忆".repeat(200),
            &[message("这是必须保留的最新问题")],
            ContextBudget {
                context_window: 256,
                reserve_output_tokens: 32,
            },
        );
        assert_eq!(
            prepared.messages.last().expect("latest").role,
            MessageRole::User
        );
        assert!(
            prepared
                .messages
                .last()
                .expect("latest")
                .content
                .contains("最新问题")
        );
        assert!(prepared.estimated_input_tokens <= 96);
        assert!(prepared.truncated_messages >= 1);
    }

    #[test]
    fn oversized_latest_turn_is_truncated_instead_of_silently_dropped() {
        let prepared = prepare_context(
            "",
            "",
            "",
            &[message(&format!("start{}end", "中".repeat(100)))],
            ContextBudget {
                context_window: 180,
                reserve_output_tokens: 16,
            },
        );
        assert_eq!(prepared.messages.len(), 1);
        assert!(prepared.messages[0].content.starts_with("start"));
        assert!(prepared.messages[0].content.ends_with("end"));
        assert_eq!(prepared.omitted_messages, 0);
        assert_eq!(prepared.truncated_messages, 1);
        assert!(prepared.estimated_input_tokens <= 36);
    }
}
