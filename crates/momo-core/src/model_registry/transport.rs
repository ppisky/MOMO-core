//! Protocol adapters for configured endpoints. Routing remains inside Core.
use super::protocol;
use super::{ModelConnection, ProviderProtocol};
use crate::{
    ChatCompletion, ChatParameters, ChatStreamDelta, GatewayError, GatewayMessage, OpenAiGateway,
    ProviderEndpoint,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

fn provider_client() -> Result<reqwest::Client, GatewayError> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(300))
        .build()?)
}

impl ModelConnection {
    pub(crate) fn endpoint(&self) -> ProviderEndpoint {
        ProviderEndpoint {
            base_url: self.provider.base_url.clone(),
            api_key: self.provider.api_key.clone(),
            model: self.model.model_id.clone(),
        }
    }
    pub(crate) fn request_url(&self, stream: bool) -> Result<String, GatewayError> {
        let mut url = url::Url::parse(&self.provider.base_url)?;
        let default = match self.provider.protocol {
            ProviderProtocol::Openai => "chat/completions",
            ProviderProtocol::OpenaiResponses => "responses",
            ProviderProtocol::Anthropic => "messages",
            ProviderProtocol::Gemini => "",
        };
        let path = if !self.provider.api_path.is_empty() {
            self.provider.api_path.clone()
        } else if self.provider.protocol == ProviderProtocol::Gemini {
            format!(
                "models/{}:{}",
                self.model.model_id.trim_start_matches("models/"),
                if stream {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                }
            )
        } else {
            default.to_owned()
        };
        let prefix = url.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{prefix}/{}", path.trim_start_matches('/')));
        if stream && self.provider.protocol == ProviderProtocol::Gemini {
            url.set_query(Some("alt=sse"));
        }
        Ok(url.to_string())
    }
    pub(crate) async fn complete(
        &self,
        messages: &[GatewayMessage],
        mut parameters: ChatParameters,
    ) -> Result<ChatCompletion, GatewayError> {
        parameters
            .request_parameters
            .retain(|key, _| !key.starts_with("momo_"));
        if self.provider.protocol == ProviderProtocol::Openai {
            return OpenAiGateway::from_client(provider_client()?)
                .with_completion_endpoint(self.request_url(false)?)
                .complete_messages(&self.endpoint(), messages, parameters)
                .await;
        }
        self.protocol_request(messages, parameters, false, |_| true)
            .await
    }
    pub(crate) async fn stream<F: FnMut(ChatStreamDelta) -> bool>(
        &self,
        messages: &[GatewayMessage],
        mut parameters: ChatParameters,
        on_delta: F,
    ) -> Result<ChatCompletion, GatewayError> {
        parameters
            .request_parameters
            .retain(|key, _| !key.starts_with("momo_"));
        if self.provider.protocol == ProviderProtocol::Openai {
            return OpenAiGateway::from_client(provider_client()?)
                .with_completion_endpoint(self.request_url(true)?)
                .stream_messages(&self.endpoint(), messages, parameters, on_delta)
                .await;
        }
        self.protocol_request(messages, parameters, true, on_delta)
            .await
    }
    async fn protocol_request<F: FnMut(ChatStreamDelta) -> bool>(
        &self,
        messages: &[GatewayMessage],
        parameters: ChatParameters,
        stream: bool,
        mut on_delta: F,
    ) -> Result<ChatCompletion, GatewayError> {
        let body = self.protocol_body(messages, parameters, stream)?;
        let client = provider_client()?;
        let mut request = client.post(self.request_url(stream)?).json(&body);
        if let Some(key) = self.provider.api_key.as_ref().filter(|k| !k.is_empty()) {
            request = match self.provider.protocol {
                ProviderProtocol::Anthropic => request.header("x-api-key", key),
                ProviderProtocol::Gemini => request.header("x-goog-api-key", key),
                _ => request.bearer_auth(key),
            };
        }
        if self.provider.protocol == ProviderProtocol::Anthropic {
            request = request.header("anthropic-version", "2023-06-01");
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(GatewayError::Http {
                status: response.status().as_u16(),
                body: "configured model endpoint rejected the request".into(),
            });
        }
        let mut bytes = response.bytes_stream();
        let mut size = 0usize;
        let mut decoder = crate::gateway::SseDecoder::new();
        let mut body = Vec::new();
        let mut state = StreamState::new(self.provider.protocol);
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk?;
            size = size.saturating_add(chunk.len());
            if size > crate::response::MAX_RESPONSE_STREAM_BYTES {
                return Err(GatewayError::StreamTooLarge);
            }
            if !stream {
                body.extend_from_slice(&chunk);
                continue;
            }
            for data in decoder.push_bytes(&chunk)? {
                let value: Value = serde_json::from_str(&data)?;
                let delta = state.accept(value)?;
                if !delta.is_empty()
                    && !on_delta(ChatStreamDelta {
                        delta,
                        tool_calls: vec![],
                        finish_reason: None,
                    })
                {
                    return Err(GatewayError::Cancelled);
                }
                if state.done && self.provider.protocol != ProviderProtocol::Gemini {
                    break;
                }
            }
            if state.done && self.provider.protocol != ProviderProtocol::Gemini {
                break;
            }
        }
        let completion = if stream {
            if !state.done {
                return Err(GatewayError::IncompleteStream);
            }
            protocol::completion(self.provider.protocol, &state.value)?
        } else {
            protocol::completion(self.provider.protocol, &serde_json::from_slice(&body)?)?
        };
        if completion
            .tool_calls
            .iter()
            .any(|c| c.function.arguments.len() > crate::response::MAX_RESPONSE_TOOL_ARGUMENT_BYTES)
        {
            return Err(GatewayError::ToolArgumentsTooLarge);
        }
        if stream {
            let calls = completion
                .tool_calls
                .iter()
                .enumerate()
                .map(|(index, c)| crate::ChatStreamToolCallDelta {
                    index,
                    id: Some(c.id.clone()),
                    call_type: Some(c.call_type.clone()),
                    function: Some(crate::ChatStreamFunctionCallDelta {
                        name: Some(c.function.name.clone()),
                        arguments: Some(c.function.arguments.clone()),
                    }),
                })
                .collect();
            if !on_delta(ChatStreamDelta {
                delta: String::new(),
                tool_calls: calls,
                finish_reason: completion.finish_reason.clone(),
            }) {
                return Err(GatewayError::Cancelled);
            }
        }
        Ok(completion)
    }
}

pub(super) struct StreamState {
    protocol: ProviderProtocol,
    value: Value,
    pub(super) done: bool,
    text: String,
    arguments: std::collections::BTreeMap<usize, String>,
}
impl StreamState {
    pub(super) fn new(protocol: ProviderProtocol) -> Self {
        Self {
            protocol,
            value: json!({}),
            done: false,
            text: String::new(),
            arguments: Default::default(),
        }
    }
    pub(super) fn accept(&mut self, v: Value) -> Result<String, GatewayError> {
        let mut delta = String::new();
        if v.get("error").is_some()
            || matches!(
                v["type"].as_str(),
                Some("error" | "response.failed" | "response.incomplete")
            )
        {
            return Err(GatewayError::InvalidMessage(
                "provider stream reported failure".into(),
            ));
        }
        match self.protocol {
            ProviderProtocol::OpenaiResponses => match v["type"].as_str() {
                Some("response.output_text.delta") => {
                    delta = v["delta"].as_str().unwrap_or_default().into()
                }
                Some("response.completed") => {
                    self.value = v["response"].clone();
                    self.done = true;
                }
                _ => {}
            },
            ProviderProtocol::Anthropic => match v["type"].as_str() {
                Some("message_start") => self.value = v["message"].clone(),
                Some("content_block_start") => {
                    let index = v["index"].as_u64().ok_or(GatewayError::MissingChoice)? as usize;
                    let content = self.value["content"]
                        .as_array_mut()
                        .ok_or(GatewayError::MissingChoice)?;
                    if index != content.len() {
                        return Err(GatewayError::MissingChoice);
                    }
                    content.push(v["content_block"].clone());
                }
                Some("content_block_delta") => {
                    let index = v["index"].as_u64().ok_or(GatewayError::MissingChoice)? as usize;
                    if self.value["content"].get(index).is_none() {
                        return Err(GatewayError::MissingChoice);
                    }
                    match v["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            delta = v["delta"]["text"].as_str().unwrap_or_default().into();
                            let old = self.value["content"][index]["text"]
                                .as_str()
                                .unwrap_or_default();
                            self.value["content"][index]["text"] = json!(format!("{old}{delta}"));
                        }
                        Some("input_json_delta") => {
                            let args = self.arguments.entry(index).or_default();
                            args.push_str(v["delta"]["partial_json"].as_str().unwrap_or_default());
                            if args.len() > crate::response::MAX_RESPONSE_TOOL_ARGUMENT_BYTES {
                                return Err(GatewayError::ToolArgumentsTooLarge);
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_stop") => {
                    let index = v["index"].as_u64().ok_or(GatewayError::MissingChoice)? as usize;
                    if let Some(args) = self.arguments.remove(&index) {
                        self.value["content"][index]["input"] = serde_json::from_str(&args)?;
                    }
                }
                Some("message_delta") => {
                    self.value["stop_reason"] = v["delta"]["stop_reason"].clone();
                    if let Some(usage) = v["usage"].as_object() {
                        for (k, val) in usage {
                            self.value["usage"][k] = val.clone();
                        }
                    }
                }
                Some("message_stop") => self.done = true,
                _ => {}
            },
            ProviderProtocol::Gemini => {
                if self.value.get("candidates").is_none() {
                    self.value = json!({"candidates":[{"content":{"parts":[]}}]});
                }
                if let Some(parts) = v
                    .pointer("/candidates/0/content/parts")
                    .and_then(Value::as_array)
                {
                    for part in parts {
                        if part["thought"] != true {
                            if let Some(text) = part["text"].as_str() {
                                delta.push_str(text);
                            }
                            self.value["candidates"][0]["content"]["parts"]
                                .as_array_mut()
                                .expect("initialized parts")
                                .push(part.clone());
                        }
                    }
                }
                if let Some(reason) = v.pointer("/candidates/0/finishReason") {
                    self.value["candidates"][0]["finishReason"] = reason.clone();
                    self.done = true;
                }
                if let Some(usage) = v.get("usageMetadata") {
                    self.value["usageMetadata"] = usage.clone();
                }
                if let Some(id) = v.get("responseId") {
                    self.value["responseId"] = id.clone();
                }
            }
            ProviderProtocol::Openai => unreachable!(),
        }
        self.text.push_str(&delta);
        if self.done && protocol::completion(self.protocol, &self.value)?.content != self.text {
            return Err(GatewayError::InvalidMessage(
                "provider final text differs from streamed text".into(),
            ));
        }
        Ok(delta)
    }
}
