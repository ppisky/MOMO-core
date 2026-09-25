//! Non-streaming and streaming chat gateway operations.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareContextRequest {
    #[serde(default)]
    pub runtime_instructions: String,
    #[serde(default)]
    pub roleplay_director: String,
    #[serde(default)]
    pub character_markdown: String,
    #[serde(default)]
    pub user_markdown: String,
    #[serde(default)]
    pub memory_markdown: String,
    #[serde(default)]
    pub state_context: String,
    #[serde(default)]
    pub nsg_markdown: String,
    #[serde(default)]
    pub messages: Vec<ChatInput>,
    pub context_window: usize,
    pub reserve_output_tokens: usize,
}

pub fn prepare_context_request(
    request: PrepareContextRequest,
) -> Result<crate::PreparedContext, RuntimeApiError> {
    let prepared = prepare_context(ContextRequest {
        sections: ContextSections {
            runtime_instructions: &request.runtime_instructions,
            roleplay_director: &request.roleplay_director,
            character: &request.character_markdown,
            user: &request.user_markdown,
            memory: &request.memory_markdown,
            state: &request.state_context,
            semantic_graph: &request.nsg_markdown,
        },
        messages: &request.messages,
        budget: ContextBudget {
            context_window: request.context_window,
            reserve_output_tokens: request.reserve_output_tokens,
        },
    });
    Ok(prepared)
}
