use std::{collections::BTreeMap, fmt, time::Duration};

use futures_util::StreamExt;
use momo_domain::MessageRole;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::response::{
    MAX_RESPONSE_SSE_EVENT_BYTES, MAX_RESPONSE_STREAM_BYTES, MAX_RESPONSE_TOOL_ARGUMENT_BYTES,
};

const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_UPSTREAM_ERROR_BYTES: usize = 8 * 1024;
const RESERVED_REQUEST_FIELDS: [&str; 4] = ["model", "messages", "temperature", "stream"];

#[derive(Clone)]
pub struct ProviderEndpoint {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

impl fmt::Debug for ProviderEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderEndpoint")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatInput {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GatewayMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatewayMessage {
    pub role: GatewayMessageRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<GatewayMessageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum GatewayMessageContent {
    Text(String),
    Parts(Vec<GatewayContentPart>),
}

impl GatewayMessageContent {
    fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Parts(parts) => parts.is_empty(),
        }
    }

    const fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }
}

impl From<String> for GatewayMessageContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for GatewayMessageContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayContentPart {
    Text { text: String },
    ImageUrl { image_url: GatewayImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatewayImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl From<&ChatInput> for GatewayMessage {
    fn from(message: &ChatInput) -> Self {
        let role = match message.role {
            MessageRole::System => GatewayMessageRole::System,
            MessageRole::User => GatewayMessageRole::User,
            MessageRole::Assistant => GatewayMessageRole::Assistant,
        };
        Self {
            role,
            content: Some(message.content.clone().into()),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatParameters {
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub request_parameters: serde_json::Map<String, serde_json::Value>,
}

impl Default for ChatParameters {
    fn default() -> Self {
        Self {
            temperature: None,
            request_parameters: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatCompletion {
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatToolCall>,
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ChatFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatUsage {
    #[serde(default, alias = "prompt_tokens")]
    pub input_tokens: u64,
    #[serde(default, alias = "completion_tokens")]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatStreamDelta {
    pub delta: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatStreamToolCallDelta>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatStreamToolCallDelta {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<ChatStreamFunctionCallDelta>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatStreamFunctionCallDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("invalid provider base URL: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("invalid authorization header")]
    InvalidAuthorization,
    #[error("invalid gateway message: {0}")]
    InvalidMessage(String),
    #[error("model request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("model endpoint returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("model response did not contain an assistant choice")]
    MissingChoice,
    #[error("model stream is invalid: {0}")]
    InvalidStream(#[from] SseDecodeError),
    #[error("model stream contained invalid JSON: {0}")]
    InvalidStreamJson(#[from] serde_json::Error),
    #[error("model request was cancelled")]
    Cancelled,
    #[error("model stream ended before the [DONE] event")]
    IncompleteStream,
    #[error("model response exceeded the {MAX_RESPONSE_BODY_BYTES} byte limit")]
    ResponseTooLarge,
    #[error("model stream exceeded the {MAX_RESPONSE_STREAM_BYTES} byte response limit")]
    StreamTooLarge,
    #[error(
        "model stream tool arguments exceeded the {MAX_RESPONSE_TOOL_ARGUMENT_BYTES} byte limit"
    )]
    ToolArgumentsTooLarge,
    #[error("model stream tool call {index} is missing {field}")]
    IncompleteToolCall { index: usize, field: &'static str },
}

#[derive(Debug, Error)]
pub enum SseDecodeError {
    #[error("event contains invalid UTF-8: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    #[error("event exceeded the {MAX_RESPONSE_SSE_EVENT_BYTES} byte limit")]
    EventTooLarge,
}

#[derive(Debug, Clone)]
pub struct OpenAiGateway {
    client: Client,
}

impl Default for OpenAiGateway {
    fn default() -> Self {
        Self::new(Duration::from_secs(120)).expect("valid HTTP client configuration")
    }
}

impl OpenAiGateway {
    pub fn new(timeout: Duration) -> Result<Self, GatewayError> {
        Ok(Self {
            client: Client::builder().timeout(timeout).build()?,
        })
    }

    #[must_use]
    pub const fn from_client(client: Client) -> Self {
        Self { client }
    }

    pub async fn complete(
        &self,
        endpoint: &ProviderEndpoint,
        messages: &[ChatInput],
        parameters: ChatParameters,
    ) -> Result<ChatCompletion, GatewayError> {
        let messages = messages
            .iter()
            .map(GatewayMessage::from)
            .collect::<Vec<_>>();
        self.complete_messages(endpoint, &messages, parameters)
            .await
    }

    pub async fn complete_messages(
        &self,
        endpoint: &ProviderEndpoint,
        messages: &[GatewayMessage],
        parameters: ChatParameters,
    ) -> Result<ChatCompletion, GatewayError> {
        validate_gateway_messages(messages)?;
        let url = completion_url(&endpoint.base_url)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(api_key) = endpoint.api_key.as_deref() {
            let value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                .map_err(|_| GatewayError::InvalidAuthorization)?;
            headers.insert(AUTHORIZATION, value);
        }
        let request = completion_request(endpoint, messages, &parameters, false);
        let response = self
            .client
            .post(url)
            .headers(headers)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_bounded_text(response, MAX_UPSTREAM_ERROR_BYTES).await?;
            return Err(GatewayError::Http {
                status: status.as_u16(),
                body,
            });
        }
        let header_request_id = upstream_request_id(response.headers());
        let body = read_bounded_bytes(response, MAX_RESPONSE_BODY_BYTES).await?;
        let response: CompletionResponse = serde_json::from_slice(&body)?;
        let upstream_request_id = header_request_id.or(response.id.clone());
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or(GatewayError::MissingChoice)?;
        Ok(ChatCompletion {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice.message.tool_calls,
            finish_reason: choice.finish_reason,
            usage: response.usage,
            upstream_request_id,
        })
    }

    pub async fn stream<F>(
        &self,
        endpoint: &ProviderEndpoint,
        messages: &[ChatInput],
        parameters: ChatParameters,
        on_delta: F,
    ) -> Result<ChatCompletion, GatewayError>
    where
        F: FnMut(ChatStreamDelta) -> bool,
    {
        let messages = messages
            .iter()
            .map(GatewayMessage::from)
            .collect::<Vec<_>>();
        self.stream_messages(endpoint, &messages, parameters, on_delta)
            .await
    }

    pub async fn stream_messages<F>(
        &self,
        endpoint: &ProviderEndpoint,
        messages: &[GatewayMessage],
        parameters: ChatParameters,
        mut on_delta: F,
    ) -> Result<ChatCompletion, GatewayError>
    where
        F: FnMut(ChatStreamDelta) -> bool,
    {
        validate_gateway_messages(messages)?;
        let url = completion_url(&endpoint.base_url)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(api_key) = endpoint.api_key.as_deref() {
            let value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                .map_err(|_| GatewayError::InvalidAuthorization)?;
            headers.insert(AUTHORIZATION, value);
        }
        let request = completion_request(endpoint, messages, &parameters, true);
        let response = self
            .client
            .post(url)
            .headers(headers)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_bounded_text(response, MAX_UPSTREAM_ERROR_BYTES).await?;
            return Err(GatewayError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let mut upstream_request_id = upstream_request_id(response.headers());
        let mut bytes = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut content = String::new();
        let mut tool_calls = BTreeMap::<usize, PendingToolCall>::new();
        let mut stream_content_bytes = 0_usize;
        let mut finish_reason = None;
        let mut usage = None;
        while let Some(chunk) = bytes.next().await {
            for event in decoder.push_bytes(&chunk?)? {
                if event == "[DONE]" {
                    let tool_calls = complete_tool_calls(tool_calls)?;
                    return Ok(ChatCompletion {
                        content,
                        tool_calls,
                        finish_reason,
                        usage,
                        upstream_request_id,
                    });
                }
                let response: StreamResponse = serde_json::from_str(&event)?;
                if upstream_request_id.is_none() {
                    upstream_request_id = response.id;
                }
                if response.usage.is_some() {
                    usage = response.usage;
                }
                let Some(choice) = response.choices.into_iter().next() else {
                    continue;
                };
                let delta = choice.delta.content.unwrap_or_default();
                let tool_deltas = choice.delta.tool_calls;
                let added_bytes = delta.len().saturating_add(
                    tool_deltas
                        .iter()
                        .map(|call| {
                            call.id.as_deref().map_or(0, str::len)
                                + call.call_type.as_deref().map_or(0, str::len)
                                + call.function.as_ref().map_or(0, |function| {
                                    function.name.as_deref().map_or(0, str::len)
                                        + function.arguments.as_deref().map_or(0, str::len)
                                })
                        })
                        .sum::<usize>(),
                );
                stream_content_bytes = stream_content_bytes.saturating_add(added_bytes);
                if stream_content_bytes > MAX_RESPONSE_STREAM_BYTES {
                    return Err(GatewayError::StreamTooLarge);
                }
                if !delta.is_empty() {
                    content.push_str(&delta);
                }
                for call in &tool_deltas {
                    let pending = tool_calls.entry(call.index).or_default();
                    if let Some(id) = &call.id {
                        pending.id.get_or_insert_with(|| id.clone());
                    }
                    if let Some(call_type) = &call.call_type {
                        pending.call_type.get_or_insert_with(|| call_type.clone());
                    }
                    if let Some(function) = &call.function {
                        if let Some(name) = &function.name {
                            pending.name.get_or_insert_with(|| name.clone());
                        }
                        if let Some(arguments) = &function.arguments {
                            if pending.arguments.len().saturating_add(arguments.len())
                                > MAX_RESPONSE_TOOL_ARGUMENT_BYTES
                            {
                                return Err(GatewayError::ToolArgumentsTooLarge);
                            }
                            pending.arguments.push_str(arguments);
                        }
                    }
                }
                if choice.finish_reason.is_some() {
                    finish_reason.clone_from(&choice.finish_reason);
                }
                if (!delta.is_empty() || !tool_deltas.is_empty() || choice.finish_reason.is_some())
                    && !on_delta(ChatStreamDelta {
                        delta,
                        tool_calls: tool_deltas,
                        finish_reason: choice.finish_reason,
                    })
                {
                    return Err(GatewayError::Cancelled);
                }
            }
        }
        Err(GatewayError::IncompleteStream)
    }
}

fn validate_gateway_messages(messages: &[GatewayMessage]) -> Result<(), GatewayError> {
    if messages.is_empty() {
        return Err(GatewayError::InvalidMessage(
            "at least one message is required".to_owned(),
        ));
    }
    for message in messages {
        let has_content = message
            .content
            .as_ref()
            .is_some_and(|value| !value.is_empty());
        match message.role {
            GatewayMessageRole::System => {
                if !has_content
                    || message
                        .content
                        .as_ref()
                        .is_some_and(|content| !content.is_text())
                    || message.tool_call_id.is_some()
                    || !message.tool_calls.is_empty()
                {
                    return Err(GatewayError::InvalidMessage(
                        "system messages require text content and cannot carry tool fields"
                            .to_owned(),
                    ));
                }
            }
            GatewayMessageRole::User => {
                if !has_content || message.tool_call_id.is_some() || !message.tool_calls.is_empty()
                {
                    return Err(GatewayError::InvalidMessage(
                        "user messages require content and cannot carry tool fields".to_owned(),
                    ));
                }
                if let Some(GatewayMessageContent::Parts(parts)) = &message.content
                    && parts.iter().any(|part| match part {
                        GatewayContentPart::Text { text } => text.is_empty(),
                        GatewayContentPart::ImageUrl { image_url } => image_url.url.is_empty(),
                    })
                {
                    return Err(GatewayError::InvalidMessage(
                        "user content parts must not be empty".to_owned(),
                    ));
                }
            }
            GatewayMessageRole::Assistant => {
                if (!has_content && message.tool_calls.is_empty())
                    || message
                        .content
                        .as_ref()
                        .is_some_and(|content| !content.is_text())
                    || message.tool_call_id.is_some()
                {
                    return Err(GatewayError::InvalidMessage(
                        "assistant messages require content or tool_calls".to_owned(),
                    ));
                }
                for call in &message.tool_calls {
                    if call.call_type != "function"
                        || call.id.is_empty()
                        || call.id.len() > 256
                        || call.function.name.is_empty()
                        || call.function.arguments.len() > MAX_RESPONSE_TOOL_ARGUMENT_BYTES
                    {
                        return Err(GatewayError::InvalidMessage(
                            "assistant function call is invalid".to_owned(),
                        ));
                    }
                }
            }
            GatewayMessageRole::Tool => {
                if !has_content
                    || message
                        .content
                        .as_ref()
                        .is_some_and(|content| !content.is_text())
                    || message
                        .tool_call_id
                        .as_deref()
                        .is_none_or(|value| value.is_empty() || value.len() > 256)
                    || !message.tool_calls.is_empty()
                {
                    return Err(GatewayError::InvalidMessage(
                        "tool messages require content and tool_call_id".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn completion_request(
    endpoint: &ProviderEndpoint,
    messages: &[GatewayMessage],
    parameters: &ChatParameters,
    stream: bool,
) -> serde_json::Value {
    let mut request = serde_json::json!({
        "model": endpoint.model,
        "messages": messages,
        "stream": stream,
    });
    let object = request
        .as_object_mut()
        .expect("completion request is always a JSON object");
    if let Some(temperature) = parameters.temperature {
        object.insert("temperature".to_owned(), serde_json::json!(temperature));
    }
    for (key, value) in &parameters.request_parameters {
        // These fields define MOMO's protocol and request lifecycle. Keeping
        // the guard here as well as at the API boundary prevents another
        // caller from silently replacing the selected model or chat history.
        if !RESERVED_REQUEST_FIELDS.contains(&key.as_str()) {
            object.insert(key.clone(), value.clone());
        }
    }
    request
}

#[derive(Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    id: Option<String>,
    choices: Vec<CompletionChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompletionMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Deserialize)]
struct StreamResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

fn upstream_request_id(headers: &HeaderMap) -> Option<String> {
    ["x-request-id", "request-id", "openai-request-id"]
        .into_iter()
        .find_map(|name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.len() <= 256)
                .map(str::to_owned)
        })
}

async fn read_bounded_bytes(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GatewayError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(GatewayError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(GatewayError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_bounded_text(
    response: reqwest::Response,
    limit: usize,
) -> Result<String, GatewayError> {
    let bytes = read_bounded_bytes(response, limit).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatStreamToolCallDelta>,
}

#[derive(Default)]
struct PendingToolCall {
    id: Option<String>,
    call_type: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn complete_tool_calls(
    pending: BTreeMap<usize, PendingToolCall>,
) -> Result<Vec<ChatToolCall>, GatewayError> {
    pending
        .into_iter()
        .map(|(index, call)| {
            Ok(ChatToolCall {
                id: call
                    .id
                    .ok_or(GatewayError::IncompleteToolCall { index, field: "id" })?,
                call_type: call.call_type.unwrap_or_else(|| "function".to_owned()),
                function: ChatFunctionCall {
                    name: call.name.ok_or(GatewayError::IncompleteToolCall {
                        index,
                        field: "function.name",
                    })?,
                    arguments: call.arguments,
                },
            })
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.push_bytes(chunk.as_bytes())
            .expect("a string chunk always contains valid UTF-8")
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) -> Result<Vec<String>, SseDecodeError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((boundary, delimiter_len)) = find_event_boundary(&self.buffer) {
            if boundary > MAX_RESPONSE_SSE_EVENT_BYTES {
                return Err(SseDecodeError::EventTooLarge);
            }
            let raw = std::str::from_utf8(&self.buffer[..boundary])?;
            let data = raw
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            self.buffer.drain(..boundary + delimiter_len);
            if !data.is_empty() {
                events.push(data);
            }
        }
        if self.buffer.len() > MAX_RESPONSE_SSE_EVENT_BYTES {
            return Err(SseDecodeError::EventTooLarge);
        }
        Ok(events)
    }
}

fn find_event_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2))
        .or_else(|| {
            bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| (position, 4))
        })
}

fn completion_url(base_url: &str) -> Result<Url, url::ParseError> {
    let mut base = Url::parse(base_url)?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join("chat/completions")
}

#[cfg(test)]
#[path = "../tests/unit/gateway.rs"]
mod tests;
