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
    /// Per-section losses, measured with the same counter as the final prompt.
    #[serde(default)]
    pub section_audit: Vec<ContextSectionAudit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextSectionAudit {
    pub section: String,
    /// Includes the section heading, excludes shared message overhead.
    pub original_tokens: usize,
    /// Includes any truncation marker; not a count of surviving source tokens.
    pub injected_tokens: usize,
    pub truncated: bool,
    pub omitted: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ContextSections<'a> {
    /// Governed runtime instructions supplied by the trusted host. These stay
    /// in the system section so history truncation cannot silently discard an
    /// override that the request audit reports as applied.
    pub runtime_instructions: &'a str,
    pub character: &'a str,
    pub user: &'a str,
    pub memory: &'a str,
    pub state: &'a str,
    pub semantic_graph: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct ContextRequest<'a> {
    pub sections: ContextSections<'a>,
    pub messages: &'a [ChatInput],
    pub budget: ContextBudget,
}

/// Builds a deterministic prompt and retains the newest complete messages that
/// fit the configured input budget. The estimator deliberately over-counts CJK
/// text so an unknown tokenizer is less likely to overflow the provider limit.
#[must_use]
pub fn prepare_context(request: ContextRequest<'_>) -> PreparedContext {
    prepare_context_with_counter(request, &estimate_text_tokens)
}

/// Equivalent to [`prepare_context`] but uses the endpoint's declared exact
/// tokenizer when available. Unknown model mappings safely use the conservative
/// estimator defined by [`TokenizerProfile::count_or_conservative`].
#[must_use]
pub fn prepare_context_with_tokenizer(
    request: ContextRequest<'_>,
    tokenizer: &TokenizerProfile,
) -> PreparedContext {
    prepare_context_with_counter(request, &|text| tokenizer.count_or_conservative(text))
}

fn prepare_context_with_counter(
    request: ContextRequest<'_>,
    counter: &dyn Fn(&str) -> usize,
) -> PreparedContext {
    let input_limit = request
        .budget
        .context_window
        .saturating_sub(request.budget.reserve_output_tokens)
        .saturating_sub(CONTEXT_SAFETY_MARGIN)
        .max(1);
    let raw_sections = system_sections(request.sections);
    let mut selected = Vec::new();
    let mut used = 0;
    let mut truncated_messages = 0;
    let mut section_audit = Vec::new();

    if !raw_sections.is_empty() {
        // A huge character card must not crowd the newest user turn out of the
        // request entirely.  Reserve its full size when possible and otherwise
        // split the available budget between system context and the newest turn.
        let newest_reserve = request
            .messages
            .last()
            .map(|message| estimate_message_tokens_with(message, counter))
            .unwrap_or_default()
            .min(input_limit.div_ceil(2));
        let system_budget = input_limit.saturating_sub(newest_reserve);
        let (system_content, audit) = fit_system_sections(&raw_sections, system_budget, counter);
        if audit.iter().any(|section| section.truncated) {
            truncated_messages += 1;
        }
        section_audit = audit;
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
    for message in request.messages.iter().rev() {
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
    let omitted_messages = request
        .messages
        .len()
        .saturating_sub(reverse_messages.len());
    selected.extend(reverse_messages);

    PreparedContext {
        messages: selected,
        estimated_input_tokens: used.saturating_add(history_tokens),
        omitted_messages,
        truncated_messages,
        section_audit,
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

fn system_sections(sections: ContextSections<'_>) -> Vec<(&'static str, String, usize)> {
    let mut output = Vec::new();
    if !sections.runtime_instructions.trim().is_empty() {
        output.push((
            "runtime_instructions",
            format!(
                "# Runtime Instructions\n{}",
                sections.runtime_instructions.trim()
            ),
            4,
        ));
    }
    if !sections.character.trim().is_empty() {
        output.push((
            "character",
            format!("# Character\n{}", sections.character.trim()),
            4,
        ));
    }
    if !sections.user.trim().is_empty() {
        output.push(("user", format!("# User\n{}", sections.user.trim()), 2));
    }
    if !sections.memory.trim().is_empty() {
        output.push((
            "memory",
            format!("# Relevant Memory\n{}", sections.memory.trim()),
            3,
        ));
    }
    if !sections.state.trim().is_empty() {
        output.push(("state", sections.state.trim().to_owned(), 3));
    }
    if !sections.semantic_graph.trim().is_empty() {
        output.push((
            "semantic_graph",
            format!("# Active Lore Context\n{}", sections.semantic_graph.trim()),
            2,
        ));
    }
    output
}

fn fit_system_sections(
    sections: &[(&str, String, usize)],
    message_budget: usize,
    counter: &dyn Fn(&str) -> usize,
) -> (String, Vec<ContextSectionAudit>) {
    let join = |values: &[String]| {
        values
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let originals = sections
        .iter()
        .map(|(_, text, _)| text.clone())
        .collect::<Vec<_>>();
    let full = join(&originals);
    let content_budget = message_budget.saturating_sub(6);
    let mut kept = originals.clone();
    if message_budget <= 6 || counter(&full) > content_budget {
        // Water-fill capped weighted shares. A giant card cannot erase all
        // relationship, state or lore sections by occupying their position.
        // Small sections finish early and return unused tokens to larger ones.
        let separator_reserve = sections.len().saturating_sub(1) * counter("\n\n");
        let mut remaining = content_budget.saturating_sub(separator_reserve);
        let costs = originals.iter().map(|s| counter(s)).collect::<Vec<_>>();
        let mut quotas = vec![0_usize; sections.len()];
        while remaining > 0 {
            let mut advanced = false;
            for (index, (_, _, weight)) in sections.iter().enumerate() {
                let grant = (*weight)
                    .min(costs[index].saturating_sub(quotas[index]))
                    .min(remaining);
                quotas[index] += grant;
                remaining -= grant;
                advanced |= grant > 0;
            }
            if !advanced {
                break;
            }
        }
        kept = originals
            .iter()
            .zip(&quotas)
            .map(|(text, quota)| {
                if *quota == 0 {
                    String::new()
                } else {
                    truncate_message_content_with(text, quota.saturating_add(6), counter).0
                }
            })
            .collect();
        // Exact tokenizers can merge tokens across section boundaries. Verify
        // the combined prompt, not just the sum of independently counted parts.
        // Very small budgets drop the lowest-priority nonempty section first.
        while counter(&join(&kept)) > content_budget {
            let Some(index) = (0..kept.len()).rev().find(|&i| !kept[i].is_empty()) else {
                break;
            };
            kept[index].pop();
        }
    }
    let audit = sections
        .iter()
        .zip(&kept)
        .map(|((name, original, _), injected)| ContextSectionAudit {
            section: (*name).to_owned(),
            original_tokens: counter(original),
            injected_tokens: counter(injected),
            truncated: original != injected,
            omitted: injected.is_empty(),
        })
        .collect();
    (join(&kept), audit)
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

    fn request<'a>(
        character: &'a str,
        user: &'a str,
        memory: &'a str,
        messages: &'a [ChatInput],
        budget: ContextBudget,
    ) -> ContextRequest<'a> {
        ContextRequest {
            sections: ContextSections {
                character,
                user,
                memory,
                ..ContextSections::default()
            },
            messages,
            budget,
        }
    }

    #[test]
    fn injects_character_and_user_before_history() {
        let prepared = prepare_context(request(
            "温柔的向导",
            "用户喜欢简洁回答",
            "曾经去过海边",
            &[message("你好")],
            ContextBudget::default(),
        ));
        assert_eq!(prepared.messages[0].role, MessageRole::System);
        assert!(prepared.messages[0].content.contains("# Character"));
        assert!(prepared.messages[0].content.contains("# User"));
        assert!(prepared.messages[0].content.contains("# Relevant Memory"));
        assert_eq!(prepared.omitted_messages, 0);
    }

    #[test]
    fn governed_runtime_instructions_are_system_context_not_trimmable_history() {
        let messages = [message(&"old history ".repeat(200)), message("latest turn")];
        let prepared = prepare_context(ContextRequest {
            sections: ContextSections {
                runtime_instructions: "Always answer in character.",
                character: "A concise guide.",
                ..ContextSections::default()
            },
            messages: &messages,
            budget: ContextBudget {
                context_window: 256,
                reserve_output_tokens: 32,
            },
        });

        let system = &prepared.messages[0].content;
        assert!(system.starts_with("# Runtime Instructions"));
        assert!(system.contains("Always answer in character."));
        assert_eq!(
            prepared.messages.last().expect("latest").content,
            "latest turn"
        );
    }

    #[test]
    fn drops_oldest_messages_when_budget_is_full() {
        let messages = vec![message(&"a".repeat(80)), message("newest")];
        let prepared = prepare_context(request(
            "",
            "",
            "",
            &messages,
            ContextBudget {
                context_window: 160,
                reserve_output_tokens: 8,
            },
        ));
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
            request("", "", "", &messages, ContextBudget::default()),
            &TokenizerProfile::Cl100kBase,
        );
        assert_eq!(prepared.estimated_input_tokens, 8);
    }

    #[test]
    fn oversized_system_context_cannot_evict_the_newest_turn() {
        let prepared = prepare_context(request(
            &"角色设定".repeat(200),
            &"用户设定".repeat(100),
            &"记忆".repeat(200),
            &[message("这是必须保留的最新问题")],
            ContextBudget {
                context_window: 256,
                reserve_output_tokens: 32,
            },
        ));
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
        let prepared = prepare_context(request(
            "",
            "",
            "",
            &[message(&format!("start{}end", "中".repeat(100)))],
            ContextBudget {
                context_window: 180,
                reserve_output_tokens: 16,
            },
        ));
        assert_eq!(prepared.messages.len(), 1);
        assert!(prepared.messages[0].content.starts_with("start"));
        assert!(prepared.messages[0].content.ends_with("end"));
        assert_eq!(prepared.omitted_messages, 0);
        assert_eq!(prepared.truncated_messages, 1);
        assert!(prepared.estimated_input_tokens <= 36);
    }

    #[test]
    fn huge_character_preserves_small_relationship_and_state_sections() {
        let messages = [message("Where do we go next?")];
        let character = "Biography detail. ".repeat(4_000);
        for tokenizer in [TokenizerProfile::Cl100kBase, TokenizerProfile::Conservative] {
            let prepared = prepare_context_with_tokenizer(
                ContextRequest {
                    sections: ContextSections {
                        runtime_instructions: "Respect the traveler's choices.",
                        character: &character,
                        user: "The traveler trusts Mira.",
                        memory: "Mira promised to meet at dawn.",
                        state: "# Scene\nOnly Eren and the traveler are in the harbor.",
                        semantic_graph: "Mira is a cartographer, not the harbor keeper.",
                    },
                    messages: &messages,
                    budget: ContextBudget {
                        context_window: 512,
                        reserve_output_tokens: 64,
                    },
                },
                &tokenizer,
            );
            let system = &prepared.messages[0].content;
            assert!(system.contains("Mira promised to meet at dawn."));
            assert!(system.contains("Only Eren and the traveler are in the harbor."));
            assert!(system.contains("Respect the traveler's choices."));
            assert_eq!(prepared.section_audit.len(), 6);
            assert!(
                prepared
                    .section_audit
                    .iter()
                    .find(|s| s.section == "character")
                    .unwrap()
                    .truncated
            );
            assert!(
                !prepared
                    .section_audit
                    .iter()
                    .find(|s| s.section == "state")
                    .unwrap()
                    .truncated
            );
            assert!(prepared.estimated_input_tokens <= 320);
        }
    }
}
