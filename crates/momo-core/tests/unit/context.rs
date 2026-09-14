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
fn roleplay_direction_follows_all_evidence_sections() {
    let prepared = prepare_context(ContextRequest {
        sections: ContextSections {
            roleplay_director: "Do not invent unsupported history.",
            character: "A careful engineer.",
            memory: "The lamp failed once.",
            state: "# Current State\nThe lamp is dark.",
            semantic_graph: "The lighthouse faces the sea.",
            ..ContextSections::default()
        },
        messages: &[message("Should we cancel?")],
        budget: ContextBudget::default(),
    });
    let system = &prepared.messages[0].content;
    let direction = system.find("# Roleplay Direction").expect("direction");
    assert!(direction > system.find("# Character").expect("character"));
    assert!(direction > system.find("# Relevant Memory").expect("memory"));
    assert!(direction > system.find("# Current State").expect("state"));
    assert!(direction > system.find("# Active Lore Context").expect("lore"));
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
fn non_ascii_estimate_is_conservative_across_scripts() {
    assert_eq!(estimate_text_tokens("abcd"), 1);
    assert_eq!(estimate_text_tokens("你好"), 2);
    assert_eq!(estimate_text_tokens("日本"), 2);
    assert_eq!(estimate_text_tokens("عربي"), 4);
}

#[test]
fn context_preserves_multilingual_narrative_values_verbatim() {
    for value in ["quiet room", "安静的房间", "静かな部屋", "غرفة هادئة"] {
        let messages = [message(value)];
        let prepared = prepare_context(request(
            value,
            value,
            value,
            &messages,
            ContextBudget::default(),
        ));
        assert!(prepared.messages[0].content.contains(value));
        assert_eq!(prepared.messages.last().expect("message").content, value);
    }
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
                    roleplay_director: "Stay embodied in the current scene.",
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
        assert!(system.contains("Stay embodied in the current scene."));
        assert!(
            prepared
                .section_audit
                .iter()
                .any(|section| section.section == "roleplay_director" && !section.omitted)
        );
        assert_eq!(prepared.section_audit.len(), 7);
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
