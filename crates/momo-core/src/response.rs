//! Stable wire types for MOMO's high-level response operation.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MOMO_RESPONSE_SCHEMA: &str = "momo.responses/1.0";
pub const MAX_RESPONSE_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE_INSTRUCTIONS_BYTES: usize = 256 * 1024;
pub const MAX_RESPONSE_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_TOOL_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_TOOL_SCHEMA_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_TOOLS: usize = 128;
pub const MAX_RESPONSE_ID_BYTES: usize = 256;
pub const MAX_RESPONSE_IMAGE_REFERENCE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE_IMAGES: usize = 8;
pub const MAX_RESPONSE_MEMORY_SPACES: usize = 16;
pub const MAX_RESPONSE_SSE_EVENT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_STREAM_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_GATEWAY_HOPS: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ResponseInput {
    Text(String),
    Items(Vec<ResponseInputItem>),
}

impl ResponseInput {
    pub fn text(&self) -> Result<String, ResponseContractError> {
        self.validate()?;
        let text = match self {
            Self::Text(text) => text.clone(),
            Self::Items(items) => items
                .iter()
                .flat_map(|item| match item {
                    ResponseInputItem::Message { content, .. } => content.texts(),
                    ResponseInputItem::InputText { text } => vec![text.as_str()],
                    ResponseInputItem::InputImage { .. }
                    | ResponseInputItem::FunctionCall { .. } => Vec::new(),
                    ResponseInputItem::FunctionCallOutput { output, .. } => vec![output.as_str()],
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        let text = text.trim().to_owned();
        if text.is_empty() && !self.has_images() {
            return Err(ResponseContractError::EmptyInput);
        }
        if text.len() > MAX_RESPONSE_INPUT_BYTES {
            return Err(ResponseContractError::InputTooLarge);
        }
        Ok(text)
    }

    #[must_use]
    pub fn image_inputs(&self) -> Vec<ResponseImageInput> {
        let mut images = Vec::new();
        if let Self::Items(items) = self {
            for item in items {
                match item {
                    ResponseInputItem::Message { content, .. } => {
                        content.collect_images(&mut images);
                    }
                    ResponseInputItem::InputImage { image_url, detail } => {
                        images.push(ResponseImageInput {
                            image_url: image_url.clone(),
                            detail: detail.clone(),
                        });
                    }
                    ResponseInputItem::InputText { .. }
                    | ResponseInputItem::FunctionCall { .. }
                    | ResponseInputItem::FunctionCallOutput { .. } => {}
                }
            }
        }
        images
    }

    #[must_use]
    pub fn has_images(&self) -> bool {
        match self {
            Self::Text(_) => false,
            Self::Items(items) => items.iter().any(|item| match item {
                ResponseInputItem::Message { content, .. } => content.has_images(),
                ResponseInputItem::InputImage { .. } => true,
                ResponseInputItem::InputText { .. }
                | ResponseInputItem::FunctionCall { .. }
                | ResponseInputItem::FunctionCallOutput { .. } => false,
            }),
        }
    }

    pub fn text_with_image_descriptions(
        &self,
        descriptions: &[String],
    ) -> Result<String, ResponseContractError> {
        self.validate()?;
        if self.image_inputs().len() != descriptions.len() {
            return Err(ResponseContractError::ImageDescriptionCount);
        }
        if descriptions.iter().any(|value| value.trim().is_empty()) {
            return Err(ResponseContractError::EmptyImageDescription);
        }
        let mut description_index = 0_usize;
        let mut parts = Vec::new();
        match self {
            Self::Text(text) => parts.push(text.clone()),
            Self::Items(items) => {
                for item in items {
                    match item {
                        ResponseInputItem::Message { content, .. } => {
                            content.resolved_texts(descriptions, &mut description_index, &mut parts)
                        }
                        ResponseInputItem::InputText { text } => parts.push(text.clone()),
                        ResponseInputItem::InputImage { .. } => {
                            push_image_description(
                                descriptions,
                                &mut description_index,
                                &mut parts,
                            );
                        }
                        ResponseInputItem::FunctionCall { .. } => {}
                        ResponseInputItem::FunctionCallOutput { output, .. } => {
                            parts.push(output.clone());
                        }
                    }
                }
            }
        }
        let text = parts
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned();
        if text.is_empty() {
            return Err(ResponseContractError::EmptyInput);
        }
        if text.len() > MAX_RESPONSE_INPUT_BYTES {
            return Err(ResponseContractError::InputTooLarge);
        }
        Ok(text)
    }

    fn validate(&self) -> Result<(), ResponseContractError> {
        match self {
            Self::Text(text) => validate_len(text, MAX_RESPONSE_INPUT_BYTES, "input text")?,
            Self::Items(items) => {
                if items.is_empty() {
                    return Err(ResponseContractError::EmptyInput);
                }
                let mut text_bytes = 0_usize;
                for item in items {
                    item.validate(&mut text_bytes)?;
                }
                let image_count = self.image_inputs().len();
                if image_count > MAX_RESPONSE_IMAGES {
                    return Err(ResponseContractError::TooManyImages);
                }
                if image_count > 0
                    && items.iter().any(|item| {
                        matches!(
                            item,
                            ResponseInputItem::FunctionCall { .. }
                                | ResponseInputItem::FunctionCallOutput { .. }
                        )
                    })
                {
                    return Err(ResponseContractError::ImageInputWithTools);
                }
                if text_bytes > MAX_RESPONSE_INPUT_BYTES {
                    return Err(ResponseContractError::InputTooLarge);
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn is_structured(&self) -> bool {
        matches!(self, Self::Items(_))
    }

    #[must_use]
    pub fn has_function_outputs(&self) -> bool {
        matches!(self, Self::Items(items) if items.iter().any(|item| {
            matches!(item, ResponseInputItem::FunctionCallOutput { .. })
        }))
    }

    /// Returns only user-authored conversational text. Tool results are useful
    /// to the current model call but must not be persisted later as if the user
    /// had said them.
    pub fn user_text(&self) -> Result<String, ResponseContractError> {
        self.validate()?;
        let text = match self {
            Self::Text(text) => text.clone(),
            Self::Items(items) => items
                .iter()
                .flat_map(|item| match item {
                    ResponseInputItem::Message { role, content } if role == "user" => {
                        content.texts()
                    }
                    ResponseInputItem::InputText { text } => vec![text.as_str()],
                    _ => Vec::new(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        Ok(text.trim().to_owned())
    }

    fn validate_tool_continuation(&self) -> Result<bool, ResponseContractError> {
        let Self::Items(items) = self else {
            return Ok(false);
        };
        let mut calls = HashSet::new();
        let mut outputs = HashSet::new();
        for item in items {
            match item {
                ResponseInputItem::FunctionCall { call_id, .. } => {
                    if !calls.insert(call_id.as_str()) {
                        return Err(ResponseContractError::InvalidToolContinuation);
                    }
                }
                ResponseInputItem::FunctionCallOutput { call_id, .. }
                    if !calls.contains(call_id.as_str()) || !outputs.insert(call_id.as_str()) =>
                {
                    return Err(ResponseContractError::InvalidToolContinuation);
                }
                _ => {}
            }
        }
        if calls.is_empty() && outputs.is_empty() {
            return Ok(false);
        }
        if calls != outputs {
            return Err(ResponseContractError::InvalidToolContinuation);
        }
        Ok(true)
    }

    pub fn gateway_messages(&self) -> Result<Vec<crate::GatewayMessage>, ResponseContractError> {
        self.validate()?;
        let mut messages = Vec::new();
        match self {
            Self::Text(text) => messages.push(crate::GatewayMessage {
                role: crate::GatewayMessageRole::User,
                content: Some(text.clone().into()),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }),
            Self::Items(items) => {
                for item in items {
                    let message = match item {
                        ResponseInputItem::Message { role, content } => crate::GatewayMessage {
                            role: match role.as_str() {
                                "system" => crate::GatewayMessageRole::System,
                                "user" => crate::GatewayMessageRole::User,
                                "assistant" => crate::GatewayMessageRole::Assistant,
                                _ => {
                                    return Err(ResponseContractError::InvalidRole(role.clone()));
                                }
                            },
                            content: Some(content.gateway_content()),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        },
                        ResponseInputItem::InputText { text } => crate::GatewayMessage {
                            role: crate::GatewayMessageRole::User,
                            content: Some(text.clone().into()),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        },
                        ResponseInputItem::FunctionCall {
                            call_id,
                            name,
                            arguments,
                        } => crate::GatewayMessage {
                            role: crate::GatewayMessageRole::Assistant,
                            content: None,
                            tool_call_id: None,
                            tool_calls: vec![crate::ChatToolCall {
                                id: call_id.clone(),
                                call_type: "function".to_owned(),
                                function: crate::ChatFunctionCall {
                                    name: name.clone(),
                                    arguments: arguments.clone(),
                                },
                            }],
                        },
                        ResponseInputItem::FunctionCallOutput { call_id, output } => {
                            crate::GatewayMessage {
                                role: crate::GatewayMessageRole::Tool,
                                content: Some(output.clone().into()),
                                tool_call_id: Some(call_id.clone()),
                                tool_calls: Vec::new(),
                            }
                        }
                        ResponseInputItem::InputImage { image_url, detail } => {
                            crate::GatewayMessage {
                                role: crate::GatewayMessageRole::User,
                                content: Some(crate::GatewayMessageContent::Parts(vec![
                                    crate::GatewayContentPart::ImageUrl {
                                        image_url: crate::GatewayImageUrl {
                                            url: image_url.clone(),
                                            detail: detail.clone(),
                                        },
                                    },
                                ])),
                                tool_call_id: None,
                                tool_calls: Vec::new(),
                            }
                        }
                    };
                    messages.push(message);
                }
            }
        }
        Ok(messages)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ResponseMessageContent {
    Text(String),
    Blocks(Vec<ResponseContentBlock>),
}

impl ResponseMessageContent {
    fn texts(&self) -> Vec<&str> {
        match self {
            Self::Text(text) => vec![text.as_str()],
            Self::Blocks(blocks) => blocks
                .iter()
                .filter_map(ResponseContentBlock::text)
                .collect(),
        }
    }

    fn validate(&self, text_bytes: &mut usize) -> Result<(), ResponseContractError> {
        match self {
            Self::Text(text) => *text_bytes = text_bytes.saturating_add(text.len()),
            Self::Blocks(blocks) => {
                if blocks.is_empty() {
                    return Err(ResponseContractError::EmptyContentBlocks);
                }
                for block in blocks {
                    block.validate(text_bytes)?;
                }
            }
        }
        Ok(())
    }

    fn has_images(&self) -> bool {
        matches!(self, Self::Blocks(blocks) if blocks.iter().any(|block| matches!(block, ResponseContentBlock::InputImage { .. })))
    }

    fn collect_images(&self, output: &mut Vec<ResponseImageInput>) {
        if let Self::Blocks(blocks) = self {
            for block in blocks {
                if let ResponseContentBlock::InputImage { image_url, detail } = block {
                    output.push(ResponseImageInput {
                        image_url: image_url.clone(),
                        detail: detail.clone(),
                    });
                }
            }
        }
    }

    fn gateway_content(&self) -> crate::GatewayMessageContent {
        match self {
            Self::Text(text) => text.clone().into(),
            Self::Blocks(blocks)
                if blocks
                    .iter()
                    .any(|block| matches!(block, ResponseContentBlock::InputImage { .. })) =>
            {
                crate::GatewayMessageContent::Parts(
                    blocks
                        .iter()
                        .map(|block| match block {
                            ResponseContentBlock::InputText { text }
                            | ResponseContentBlock::OutputText { text, .. } => {
                                crate::GatewayContentPart::Text { text: text.clone() }
                            }
                            ResponseContentBlock::Refusal { refusal } => {
                                crate::GatewayContentPart::Text {
                                    text: refusal.clone(),
                                }
                            }
                            ResponseContentBlock::InputImage { image_url, detail } => {
                                crate::GatewayContentPart::ImageUrl {
                                    image_url: crate::GatewayImageUrl {
                                        url: image_url.clone(),
                                        detail: detail.clone(),
                                    },
                                }
                            }
                        })
                        .collect(),
                )
            }
            Self::Blocks(_) => self.texts().join("\n").into(),
        }
    }

    fn resolved_texts(
        &self,
        descriptions: &[String],
        description_index: &mut usize,
        output: &mut Vec<String>,
    ) {
        match self {
            Self::Text(text) => output.push(text.clone()),
            Self::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ResponseContentBlock::InputText { text }
                        | ResponseContentBlock::OutputText { text, .. } => {
                            output.push(text.clone());
                        }
                        ResponseContentBlock::Refusal { refusal } => {
                            output.push(refusal.clone());
                        }
                        ResponseContentBlock::InputImage { .. } => {
                            push_image_description(descriptions, description_index, output)
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResponseImageInput {
    pub image_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn push_image_description(
    descriptions: &[String],
    description_index: &mut usize,
    output: &mut Vec<String>,
) {
    let index = *description_index;
    output.push(format!(
        "[Visual description for image {}]\n{}",
        index + 1,
        descriptions[index].trim()
    ));
    *description_index += 1;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseContentBlock {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
        #[serde(default)]
        detail: Option<String>,
    },
    OutputText {
        text: String,
        #[serde(default)]
        annotations: Vec<Value>,
    },
    Refusal {
        refusal: String,
    },
}

impl ResponseContentBlock {
    fn text(&self) -> Option<&str> {
        match self {
            Self::InputText { text } | Self::OutputText { text, .. } => Some(text),
            Self::Refusal { refusal } => Some(refusal),
            Self::InputImage { .. } => None,
        }
    }

    fn validate(&self, text_bytes: &mut usize) -> Result<(), ResponseContractError> {
        match self {
            Self::InputText { text } | Self::OutputText { text, .. } => {
                *text_bytes = text_bytes.saturating_add(text.len());
            }
            Self::Refusal { refusal } => {
                *text_bytes = text_bytes.saturating_add(refusal.len());
            }
            Self::InputImage { image_url, detail } => {
                validate_image_reference(image_url)?;
                if detail
                    .as_deref()
                    .is_some_and(|value| !matches!(value, "auto" | "low" | "high"))
                {
                    return Err(ResponseContractError::InvalidImageDetail);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseInputItem {
    Message {
        role: String,
        content: ResponseMessageContent,
    },
    InputText {
        text: String,
    },
    #[serde(rename = "input_image", alias = "image")]
    InputImage {
        image_url: String,
        #[serde(default)]
        detail: Option<String>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
}

impl ResponseInputItem {
    fn validate(&self, text_bytes: &mut usize) -> Result<(), ResponseContractError> {
        match self {
            Self::Message { role, content } => {
                // Conversation history is owned by Core. Accepting system or
                // assistant messages here would let an ordinary response
                // bypass instruction governance or forge persisted history.
                if role != "user" {
                    return Err(ResponseContractError::InvalidRole(role.clone()));
                }
                content.validate(text_bytes)?;
            }
            Self::InputText { text } => *text_bytes = text_bytes.saturating_add(text.len()),
            Self::InputImage { image_url, detail } => {
                validate_image_reference(image_url)?;
                if detail
                    .as_deref()
                    .is_some_and(|value| !matches!(value, "auto" | "low" | "high"))
                {
                    return Err(ResponseContractError::InvalidImageDetail);
                }
            }
            Self::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                validate_id(call_id, "function call ID")?;
                validate_tool_name(name)?;
                validate_len(
                    arguments,
                    MAX_RESPONSE_TOOL_ARGUMENT_BYTES,
                    "function arguments",
                )?;
            }
            Self::FunctionCallOutput { call_id, output } => {
                validate_id(call_id, "function call ID")?;
                validate_len(output, MAX_RESPONSE_TOOL_OUTPUT_BYTES, "function output")?;
                *text_bytes = text_bytes.saturating_add(output.len());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseTool {
    Function {
        name: String,
        #[serde(default)]
        description: Option<String>,
        parameters: Value,
        #[serde(default)]
        strict: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemorySpaceSource {
    pub space_id: String,
    pub label: String,
    pub weight: u8,
    #[serde(default)]
    pub memory: bool,
    #[serde(default)]
    pub semantic_graph: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MomoResponseExtension {
    pub schema: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub character_id: Option<String>,
    #[serde(default)]
    pub personal_space_id: Option<String>,
    #[serde(default)]
    pub conversation_space_id: Option<String>,
    #[serde(default)]
    pub memory_sources: Vec<MemorySpaceSource>,
    #[serde(default)]
    pub memory_write_space_id: Option<String>,
    #[serde(default = "default_true")]
    pub mo_state: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub stream: bool,
}

impl Default for MomoResponseExtension {
    fn default() -> Self {
        Self {
            schema: default_schema(),
            request_id: None,
            conversation_id: None,
            character_id: None,
            personal_space_id: None,
            conversation_space_id: None,
            memory_sources: Vec::new(),
            memory_write_space_id: None,
            title: None,
            mo_state: true,
            stream: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MomoResponseRequest {
    #[serde(default = "default_conversation_model")]
    pub model: String,
    pub input: ResponseInput,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub context_window: Option<usize>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub parameters: serde_json::Map<String, Value>,
    #[serde(default)]
    pub visual_description_prompt: Option<String>,
    #[serde(default)]
    pub tools: Vec<ResponseTool>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub momo: MomoResponseExtension,
}

impl MomoResponseRequest {
    pub fn validate(&self) -> Result<String, ResponseContractError> {
        if self.momo.schema != MOMO_RESPONSE_SCHEMA {
            return Err(ResponseContractError::UnsupportedSchema(
                self.momo.schema.clone(),
            ));
        }
        if self.model.trim().is_empty() {
            return Err(ResponseContractError::EmptyModel);
        }
        validate_id(&self.model, "model route")?;
        if serde_json::to_vec(self)
            .map_err(|error| ResponseContractError::InvalidJson(error.to_string()))?
            .len()
            > MAX_RESPONSE_REQUEST_BYTES
        {
            return Err(ResponseContractError::RequestTooLarge);
        }
        let input_text = self.input.text()?;
        if self.input.validate_tool_continuation()? && self.momo.conversation_id.is_none() {
            return Err(ResponseContractError::ToolContinuationNeedsConversation);
        }
        if let Some(instructions) = &self.instructions {
            validate_len(
                instructions,
                MAX_RESPONSE_INSTRUCTIONS_BYTES,
                "instructions",
            )?;
        }
        if self
            .visual_description_prompt
            .as_ref()
            .is_some_and(|value| value.len() > MAX_RESPONSE_INSTRUCTIONS_BYTES)
        {
            return Err(ResponseContractError::FieldTooLarge(
                "visual description prompt",
            ));
        }
        if self
            .temperature
            .is_some_and(|temperature| !temperature.is_finite())
        {
            return Err(ResponseContractError::InvalidTemperature);
        }
        if self.tools.len() > MAX_RESPONSE_TOOLS {
            return Err(ResponseContractError::TooManyTools);
        }
        for tool in &self.tools {
            match tool {
                ResponseTool::Function {
                    name, parameters, ..
                } => {
                    validate_tool_name(name)?;
                    if serde_json::to_vec(parameters)
                        .map_err(|error| ResponseContractError::InvalidJson(error.to_string()))?
                        .len()
                        > MAX_RESPONSE_TOOL_SCHEMA_BYTES
                    {
                        return Err(ResponseContractError::FieldTooLarge("tool schema"));
                    }
                }
            }
        }
        if let Some(value) = self.momo.request_id.as_deref() {
            validate_id(value, "request ID")?;
        }
        if let Some(value) = self.momo.conversation_id.as_deref() {
            validate_uuid(value, "conversation ID")?;
        }
        for (value, field) in [(self.momo.character_id.as_deref(), "character ID")] {
            if let Some(value) = value {
                validate_uuid(value, field)?;
            }
        }
        for (value, field) in [
            (self.momo.personal_space_id.as_deref(), "personal space ID"),
            (
                self.momo.conversation_space_id.as_deref(),
                "conversation space ID",
            ),
        ] {
            validate_required_uuid(value, field)?;
        }
        if self.momo.memory_sources.len() > MAX_RESPONSE_MEMORY_SPACES {
            return Err(ResponseContractError::TooManyMemorySpaces);
        }
        let mut source_ids = HashSet::new();
        for source in &self.momo.memory_sources {
            validate_uuid(&source.space_id, "memory source space ID")?;
            validate_id(&source.label, "memory source label")?;
            if !(1..=100).contains(&source.weight)
                || (!source.memory && !source.semantic_graph)
                || !source_ids.insert(source.space_id.as_str())
            {
                return Err(ResponseContractError::InvalidMemorySpace);
            }
        }
        if let Some(write_space_id) = self.momo.memory_write_space_id.as_deref() {
            validate_uuid(write_space_id, "memory write space ID")?;
            if !source_ids.contains(write_space_id) {
                return Err(ResponseContractError::InvalidMemoryWriteSpace);
            }
        }
        Ok(input_text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResponseUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MomoResponse {
    pub id: String,
    pub object: String,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    pub output_text: String,
    pub usage: ResponseUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    pub momo: MomoResponseMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseOutputItem {
    Message {
        id: String,
        role: String,
        status: String,
        content: Vec<ResponseOutputContent>,
    },
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        status: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseOutputContent {
    OutputText {
        text: String,
        annotations: Vec<Value>,
    },
    Refusal {
        refusal: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MomoResponseMetadata {
    pub schema: String,
    pub request_id: String,
    pub conversation_id: String,
    pub route: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_request_id: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub state_audit: Value,
    #[serde(default)]
    pub request_audit: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResponseContractError {
    #[error("response input must contain text, an image, or function output")]
    EmptyInput,
    #[error("response input exceeds the {MAX_RESPONSE_INPUT_BYTES} byte limit")]
    InputTooLarge,
    #[error("response request exceeds the {MAX_RESPONSE_REQUEST_BYTES} byte limit")]
    RequestTooLarge,
    #[error("model route must not be empty")]
    EmptyModel,
    #[error("{0} must not be empty and may not exceed {MAX_RESPONSE_ID_BYTES} bytes")]
    InvalidId(&'static str),
    #[error("{0} is required and must be a UUID")]
    InvalidUuid(&'static str),
    #[error(
        "input message role {0:?} is not supported; conversation input messages must use role user"
    )]
    InvalidRole(String),
    #[error("structured message content must not be empty")]
    EmptyContentBlocks,
    #[error("image detail must be auto, low, or high")]
    InvalidImageDetail,
    #[error("temperature must be finite")]
    InvalidTemperature,
    #[error("image input cannot be combined with function-call continuation items")]
    ImageInputWithTools,
    #[error(
        "tool continuation must pair each function_call with one following function_call_output using the same call_id"
    )]
    InvalidToolContinuation,
    #[error("tool continuation requires momo.conversation_id")]
    ToolContinuationNeedsConversation,
    #[error("response request may contain at most {MAX_RESPONSE_IMAGES} images")]
    TooManyImages,
    #[error("the visual-description adapter returned the wrong number of descriptions")]
    ImageDescriptionCount,
    #[error("the visual-description adapter returned an empty description")]
    EmptyImageDescription,
    #[error(
        "image reference is invalid or exceeds the {MAX_RESPONSE_IMAGE_REFERENCE_BYTES} byte limit"
    )]
    InvalidImageReference,
    #[error("function name must use 1-128 ASCII letters, digits, underscores, dots, or hyphens")]
    InvalidToolName,
    #[error("response request may contain at most {MAX_RESPONSE_TOOLS} tools")]
    TooManyTools,
    #[error("response request may contain at most {MAX_RESPONSE_MEMORY_SPACES} memory Spaces")]
    TooManyMemorySpaces,
    #[error("memory Spaces must be unique, enable DMW or NSG, and have weight 1 through 100")]
    InvalidMemorySpace,
    #[error("memory_write_space_id must identify one declared memory source")]
    InvalidMemoryWriteSpace,
    #[error("{0} exceeds its byte limit")]
    FieldTooLarge(&'static str),
    #[error("response contract could not be encoded: {0}")]
    InvalidJson(String),
    #[error("unsupported MOMO response schema {0:?}")]
    UnsupportedSchema(String),
}

fn validate_len(
    value: &str,
    limit: usize,
    field: &'static str,
) -> Result<(), ResponseContractError> {
    if value.len() > limit {
        Err(ResponseContractError::FieldTooLarge(field))
    } else {
        Ok(())
    }
}

fn validate_id(value: &str, field: &'static str) -> Result<(), ResponseContractError> {
    if value.trim().is_empty() || value.len() > MAX_RESPONSE_ID_BYTES {
        Err(ResponseContractError::InvalidId(field))
    } else {
        Ok(())
    }
}

fn validate_required_uuid(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), ResponseContractError> {
    let Some(value) = value else {
        return Err(ResponseContractError::InvalidUuid(field));
    };
    validate_uuid(value, field)
}

fn validate_uuid(value: &str, field: &'static str) -> Result<(), ResponseContractError> {
    validate_id(value, field)?;
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ResponseContractError::InvalidUuid(field))
}

fn validate_tool_name(value: &str) -> Result<(), ResponseContractError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        Err(ResponseContractError::InvalidToolName)
    } else {
        Ok(())
    }
}

fn validate_image_reference(value: &str) -> Result<(), ResponseContractError> {
    let supported = value.starts_with("https://")
        || value.starts_with("http://")
        || value.starts_with("data:image/");
    if !supported || value.len() > MAX_RESPONSE_IMAGE_REFERENCE_BYTES {
        Err(ResponseContractError::InvalidImageReference)
    } else {
        Ok(())
    }
}

fn default_schema() -> String {
    MOMO_RESPONSE_SCHEMA.to_owned()
}

fn default_conversation_model() -> String {
    "conversation".to_owned()
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_string_and_structured_text_input() {
        let string: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": "hello",
            "momo": {
                "schema": MOMO_RESPONSE_SCHEMA,
                "personal_space_id": "00000000-0000-4000-8000-000000000011",
                "conversation_space_id": "00000000-0000-4000-8000-000000000012",
                "character_id": "00000000-0000-4000-8000-000000000014"
            }
        }))
        .expect("string request");
        assert_eq!(string.validate().expect("text"), "hello");

        let structured: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{"type": "input_text", "text": "hello"}],
            "momo": {
                "schema": MOMO_RESPONSE_SCHEMA,
                "personal_space_id": "00000000-0000-4000-8000-000000000011",
                "conversation_space_id": "00000000-0000-4000-8000-000000000012",
                "character_id": "00000000-0000-4000-8000-000000000014"
            }
        }))
        .expect("structured request");
        assert_eq!(structured.validate().expect("text"), "hello");
    }

    #[test]
    fn rejects_unknown_contract_generation() {
        let request: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": "hello",
            "momo": {"schema": "momo.responses/9.9"}
        }))
        .expect("request");
        assert!(matches!(
            request.validate(),
            Err(ResponseContractError::UnsupportedSchema(_))
        ));
    }

    #[test]
    fn requires_an_explicit_contract_generation() {
        let request = serde_json::from_value::<MomoResponseRequest>(serde_json::json!({
            "input": "hello",
            "momo": {
                "personal_space_id": "00000000-0000-4000-8000-000000000011",
                "conversation_space_id": "00000000-0000-4000-8000-000000000012",
                "character_id": "00000000-0000-4000-8000-000000000014"
            }
        }));
        assert!(request.is_err());
    }

    #[test]
    fn validates_and_resolves_governed_image_input() {
        let structured: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }],
            "momo": {
                "schema": MOMO_RESPONSE_SCHEMA,
                "personal_space_id": "00000000-0000-4000-8000-000000000011",
                "conversation_space_id": "00000000-0000-4000-8000-000000000012",
                "character_id": "00000000-0000-4000-8000-000000000014"
            }
        }))
        .expect("content blocks");
        assert_eq!(structured.validate().expect("text"), "hello");

        let image: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [
                {"type": "input_text", "text": "What is shown?"},
                {"type": "input_image", "image_url": "https://example.test/image.png", "detail": "high"}
            ],
            "momo": {
                "schema": MOMO_RESPONSE_SCHEMA,
                "personal_space_id": "00000000-0000-4000-8000-000000000011",
                "conversation_space_id": "00000000-0000-4000-8000-000000000012",
                "character_id": "00000000-0000-4000-8000-000000000014"
            }
        }))
        .expect("image structure");
        assert_eq!(image.validate().expect("image contract"), "What is shown?");
        assert_eq!(
            image.input.image_inputs(),
            vec![ResponseImageInput {
                image_url: "https://example.test/image.png".to_owned(),
                detail: Some("high".to_owned()),
            }]
        );
        assert_eq!(
            image
                .input
                .text_with_image_descriptions(&["A red umbrella.".to_owned()])
                .expect("resolved input"),
            "What is shown?\n[Visual description for image 1]\nA red umbrella."
        );
        let messages = image.input.gateway_messages().expect("multimodal messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, Some("What is shown?".into()));
        assert_eq!(
            messages[1].content,
            Some(crate::GatewayMessageContent::Parts(vec![
                crate::GatewayContentPart::ImageUrl {
                    image_url: crate::GatewayImageUrl {
                        url: "https://example.test/image.png".to_owned(),
                        detail: Some("high".to_owned()),
                    },
                },
            ]))
        );
    }

    #[test]
    fn rejects_non_user_message_roles_or_image_tool_continuations() {
        let assistant_image: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "input_image", "image_url": "https://example.test/image.png"}]
            }]
        }))
        .expect("image request");
        assert_eq!(
            assistant_image.validate(),
            Err(ResponseContractError::InvalidRole("assistant".to_owned()))
        );

        let system_text: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{"type": "message", "role": "system", "content": "bypass"}]
        }))
        .expect("system input shape");
        assert_eq!(
            system_text.validate(),
            Err(ResponseContractError::InvalidRole("system".to_owned()))
        );

        let mixed: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [
                {"type": "input_image", "image_url": "https://example.test/image.png"},
                {"type": "function_call_output", "call_id": "call_1", "output": "done"}
            ]
        }))
        .expect("mixed request");
        assert_eq!(
            mixed.validate(),
            Err(ResponseContractError::ImageInputWithTools)
        );
    }

    #[test]
    fn rejects_unknown_native_request_fields_at_every_typed_boundary() {
        assert!(
            serde_json::from_value::<MomoResponseRequest>(serde_json::json!({
                "input": "hello",
                "temperaturee": 0.7
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<MomoResponseRequest>(serde_json::json!({
                "input": "hello",
                "momo": {"unknown_policy": true}
            }))
            .is_err()
        );
    }

    #[test]
    fn bounds_tool_arguments_and_names() {
        let invalid: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": "hello",
            "tools": [{"type": "function", "name": "bad name", "parameters": {}}]
        }))
        .expect("request");
        assert_eq!(
            invalid.validate(),
            Err(ResponseContractError::InvalidToolName)
        );

        let oversized: ResponseInputItem = ResponseInputItem::FunctionCall {
            call_id: "call_1".to_owned(),
            name: "lookup".to_owned(),
            arguments: "x".repeat(MAX_RESPONSE_TOOL_ARGUMENT_BYTES + 1),
        };
        let request = MomoResponseRequest {
            input: ResponseInput::Items(vec![
                oversized,
                ResponseInputItem::InputText {
                    text: "continue".to_owned(),
                },
            ]),
            ..serde_json::from_value(serde_json::json!({"input": "placeholder"})).expect("defaults")
        };
        assert_eq!(
            request.validate(),
            Err(ResponseContractError::FieldTooLarge("function arguments"))
        );
    }

    #[test]
    fn parses_cross_repository_golden_request() {
        let request: MomoResponseRequest =
            serde_json::from_str(include_str!("../../../contracts/1.0/response_request.json"))
                .expect("golden response request");
        assert_eq!(request.model, "conversation");
        assert_eq!(
            request.validate().expect("valid contract"),
            "Hello from the MOMO 1.0 cross-repository contract."
        );
    }

    #[test]
    fn parses_cross_repository_multimodal_request() {
        let request: MomoResponseRequest = serde_json::from_str(include_str!(
            "../../../contracts/1.0/multimodal_request.json"
        ))
        .expect("golden multimodal request");
        request.validate().expect("valid multimodal contract");
        assert_eq!(request.input.image_inputs().len(), 1);
        assert_eq!(request.momo.schema, MOMO_RESPONSE_SCHEMA);
    }

    #[test]
    fn parses_cross_repository_tool_contract() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../contracts/1.0/tool_turn.json"))
                .expect("tool fixture");
        let tools: Vec<ResponseTool> =
            serde_json::from_value(fixture["tools"].clone()).expect("tools");
        assert!(matches!(
            &tools[0],
            ResponseTool::Function { name, .. } if name == "lookup_weather"
        ));
        let call: ResponseOutputItem =
            serde_json::from_value(fixture["assistant_call"].clone()).expect("function call");
        assert!(matches!(
            call,
            ResponseOutputItem::FunctionCall { call_id, .. } if call_id == "call_weather_1"
        ));
        let result: ResponseInputItem =
            serde_json::from_value(fixture["tool_result"].clone()).expect("tool result");
        assert!(matches!(
            result,
            ResponseInputItem::FunctionCallOutput { call_id, .. } if call_id == "call_weather_1"
        ));
    }

    #[test]
    fn tool_continuation_keeps_call_identity_at_the_gateway_boundary() {
        let input = ResponseInput::Items(vec![
            ResponseInputItem::FunctionCall {
                call_id: "call_weather_1".to_owned(),
                name: "lookup_weather".to_owned(),
                arguments: r#"{"city":"Shanghai"}"#.to_owned(),
            },
            ResponseInputItem::FunctionCallOutput {
                call_id: "call_weather_1".to_owned(),
                output: r#"{"temperature":28}"#.to_owned(),
            },
        ]);
        let messages = input.gateway_messages().expect("gateway messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, crate::GatewayMessageRole::Assistant);
        assert_eq!(messages[0].tool_calls[0].id, "call_weather_1");
        assert_eq!(messages[1].role, crate::GatewayMessageRole::Tool);
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_weather_1"));
        assert_eq!(input.user_text().expect("user text"), "");
    }

    #[test]
    fn tool_continuation_requires_a_conversation_and_exact_call_pairs() {
        let base = serde_json::json!({
            "input": [
                {"type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "done"}
            ],
            "momo": {
                "schema": MOMO_RESPONSE_SCHEMA,
                "personal_space_id": "00000000-0000-4000-8000-000000000011",
                "conversation_space_id": "00000000-0000-4000-8000-000000000012",
                "character_id": "00000000-0000-4000-8000-000000000014"
            }
        });
        let missing_conversation: MomoResponseRequest =
            serde_json::from_value(base.clone()).expect("request");
        assert_eq!(
            missing_conversation.validate(),
            Err(ResponseContractError::ToolContinuationNeedsConversation)
        );

        let mut mismatched = base;
        mismatched["momo"]["conversation_id"] =
            serde_json::json!("00000000-0000-4000-8000-000000000013");
        mismatched["input"][1]["call_id"] = serde_json::json!("call_2");
        let mismatched: MomoResponseRequest = serde_json::from_value(mismatched).expect("request");
        assert_eq!(
            mismatched.validate(),
            Err(ResponseContractError::InvalidToolContinuation)
        );
    }
}
