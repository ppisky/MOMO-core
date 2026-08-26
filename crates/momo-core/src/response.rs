//! Stable wire types for MOMO's high-level response operation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MOMO_RESPONSE_SCHEMA: &str = "momo.responses/0.5";
pub const MAX_RESPONSE_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE_INSTRUCTIONS_BYTES: usize = 256 * 1024;
pub const MAX_RESPONSE_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_TOOL_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_TOOL_SCHEMA_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_TOOLS: usize = 128;
pub const MAX_RESPONSE_ID_BYTES: usize = 256;
pub const MAX_RESPONSE_IMAGE_REFERENCE_BYTES: usize = 2 * 1024 * 1024;
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

    pub fn gateway_messages(&self) -> Result<Vec<crate::GatewayMessage>, ResponseContractError> {
        self.validate()?;
        let mut messages = Vec::new();
        match self {
            Self::Text(text) => messages.push(crate::GatewayMessage {
                role: crate::GatewayMessageRole::User,
                content: Some(text.clone()),
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
                            content: Some(content.texts().join("\n")),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        },
                        ResponseInputItem::InputText { text } => crate::GatewayMessage {
                            role: crate::GatewayMessageRole::User,
                            content: Some(text.clone()),
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
                                content: Some(output.clone()),
                                tool_call_id: Some(call_id.clone()),
                                tool_calls: Vec::new(),
                            }
                        }
                        ResponseInputItem::InputImage { .. } => unreachable!("validated above"),
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
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
                return Err(ResponseContractError::UnsupportedInputModality("image"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
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
                if !matches!(role.as_str(), "system" | "user" | "assistant") {
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
                return Err(ResponseContractError::UnsupportedInputModality("image"));
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
#[serde(tag = "type", rename_all = "snake_case")]
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
pub struct MomoResponseExtension {
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub character_id: Option<String>,
    #[serde(default)]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "default_true")]
    pub memory: bool,
    #[serde(default = "default_true")]
    pub semantic_graph: bool,
    #[serde(default = "default_true")]
    pub mo_state: bool,
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
            scope_id: None,
            title: None,
            memory: true,
            semantic_graph: true,
            mo_state: true,
            stream: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
        if let Some(instructions) = &self.instructions {
            validate_len(
                instructions,
                MAX_RESPONSE_INSTRUCTIONS_BYTES,
                "instructions",
            )?;
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
        for (value, field) in [
            (self.momo.request_id.as_deref(), "request ID"),
            (self.momo.conversation_id.as_deref(), "conversation ID"),
            (self.momo.character_id.as_deref(), "character ID"),
            (self.momo.scope_id.as_deref(), "scope ID"),
        ] {
            if let Some(value) = value {
                validate_id(value, field)?;
            }
        }
        self.input.text()
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MomoResponseEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub request_id: String,
    pub sequence: u64,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub response: Option<MomoResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
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
    #[error("response input must contain text")]
    EmptyInput,
    #[error("response input exceeds the {MAX_RESPONSE_INPUT_BYTES} byte limit")]
    InputTooLarge,
    #[error("response request exceeds the {MAX_RESPONSE_REQUEST_BYTES} byte limit")]
    RequestTooLarge,
    #[error("model route must not be empty")]
    EmptyModel,
    #[error("{0} must not be empty and may not exceed {MAX_RESPONSE_ID_BYTES} bytes")]
    InvalidId(&'static str),
    #[error("message role {0:?} is not supported")]
    InvalidRole(String),
    #[error("structured message content must not be empty")]
    EmptyContentBlocks,
    #[error("image detail must be auto, low, or high")]
    InvalidImageDetail,
    #[error("{0} input is reserved for a future capability and is not enabled in 0.5")]
    UnsupportedInputModality(&'static str),
    #[error(
        "image reference is invalid or exceeds the {MAX_RESPONSE_IMAGE_REFERENCE_BYTES} byte limit"
    )]
    InvalidImageReference,
    #[error("function name must use 1-128 ASCII letters, digits, underscores, dots, or hyphens")]
    InvalidToolName,
    #[error("response request may contain at most {MAX_RESPONSE_TOOLS} tools")]
    TooManyTools,
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
            "momo": {"schema": MOMO_RESPONSE_SCHEMA}
        }))
        .expect("string request");
        assert_eq!(string.validate().expect("text"), "hello");

        let structured: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{"type": "input_text", "text": "hello"}],
            "momo": {"schema": MOMO_RESPONSE_SCHEMA}
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
    fn validates_content_blocks_and_rejects_deferred_images() {
        let structured: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }]
        }))
        .expect("content blocks");
        assert_eq!(structured.validate().expect("text"), "hello");

        let image: MomoResponseRequest = serde_json::from_value(serde_json::json!({
            "input": [{"type": "input_image", "image_url": "https://example.test/image.png"}]
        }))
        .expect("image structure");
        assert_eq!(
            image.validate(),
            Err(ResponseContractError::UnsupportedInputModality("image"))
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
            serde_json::from_str(include_str!("../../../contracts/0.5/response_request.json"))
                .expect("golden response request");
        assert_eq!(request.model, "conversation");
        assert_eq!(
            request.validate().expect("valid contract"),
            "Hello from the cross-repository contract."
        );
    }

    #[test]
    fn parses_cross_repository_tool_contract() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../contracts/0.5/tool_turn.json"))
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
    }
}
