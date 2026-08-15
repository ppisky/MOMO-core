use std::time::Duration;

use futures_util::StreamExt;
use momo_domain::MessageRole;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;
const MAX_STREAM_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const RESERVED_REQUEST_FIELDS: [&str; 4] = ["model", "messages", "temperature", "stream"];

#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatInput {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatParameters {
    pub temperature: f32,
    #[serde(default)]
    pub request_parameters: serde_json::Map<String, serde_json::Value>,
}

impl Default for ChatParameters {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            request_parameters: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatCompletion {
    pub content: String,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatStreamDelta {
    pub delta: String,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("invalid provider base URL: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("invalid authorization header")]
    InvalidAuthorization,
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
    #[error("model stream exceeded the {MAX_STREAM_CONTENT_BYTES} byte response limit")]
    StreamTooLarge,
}

#[derive(Debug, Error)]
pub enum SseDecodeError {
    #[error("event contains invalid UTF-8: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    #[error("event exceeded the {MAX_SSE_EVENT_BYTES} byte limit")]
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

    pub async fn complete(
        &self,
        endpoint: &ProviderEndpoint,
        messages: &[ChatInput],
        parameters: ChatParameters,
    ) -> Result<ChatCompletion, GatewayError> {
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
            let body = response.text().await.unwrap_or_default();
            return Err(GatewayError::Http {
                status: status.as_u16(),
                body: body.chars().take(2_000).collect(),
            });
        }
        let response: CompletionResponse = response.json().await?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or(GatewayError::MissingChoice)?;
        Ok(ChatCompletion {
            content: choice.message.content,
            finish_reason: choice.finish_reason,
        })
    }

    pub async fn stream<F>(
        &self,
        endpoint: &ProviderEndpoint,
        messages: &[ChatInput],
        parameters: ChatParameters,
        mut on_delta: F,
    ) -> Result<ChatCompletion, GatewayError>
    where
        F: FnMut(ChatStreamDelta) -> bool,
    {
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
            let body = response.text().await.unwrap_or_default();
            return Err(GatewayError::Http {
                status: status.as_u16(),
                body: body.chars().take(2_000).collect(),
            });
        }

        let mut bytes = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut content = String::new();
        let mut finish_reason = None;
        while let Some(chunk) = bytes.next().await {
            for event in decoder.push_bytes(&chunk?)? {
                if event == "[DONE]" {
                    return Ok(ChatCompletion {
                        content,
                        finish_reason,
                    });
                }
                let response: StreamResponse = serde_json::from_str(&event)?;
                let Some(choice) = response.choices.into_iter().next() else {
                    continue;
                };
                let delta = choice.delta.content.unwrap_or_default();
                if !delta.is_empty() {
                    if content.len().saturating_add(delta.len()) > MAX_STREAM_CONTENT_BYTES {
                        return Err(GatewayError::StreamTooLarge);
                    }
                    content.push_str(&delta);
                }
                if choice.finish_reason.is_some() {
                    finish_reason.clone_from(&choice.finish_reason);
                }
                if (!delta.is_empty() || choice.finish_reason.is_some())
                    && !on_delta(ChatStreamDelta {
                        delta,
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

fn completion_request(
    endpoint: &ProviderEndpoint,
    messages: &[ChatInput],
    parameters: &ChatParameters,
    stream: bool,
) -> serde_json::Value {
    let mut request = serde_json::json!({
        "model": endpoint.model,
        "messages": messages
            .iter()
            .map(|message| WireMessage {
                role: message.role.as_str(),
                content: &message.content,
            })
            .collect::<Vec<_>>(),
        "temperature": parameters.temperature,
        "stream": stream,
    });
    let object = request
        .as_object_mut()
        .expect("completion request is always a JSON object");
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

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<CompletionChoice>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompletionMessage {
    content: String,
}

#[derive(Deserialize)]
struct StreamResponse {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamMessage {
    content: Option<String>,
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
            if boundary > MAX_SSE_EVENT_BYTES {
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
        if self.buffer.len() > MAX_SSE_EVENT_BYTES {
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
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn normalizes_completion_url_once() {
        assert_eq!(
            completion_url("https://example.com/v1")
                .expect("URL")
                .as_str(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn merges_user_defined_request_parameters_without_replacing_protocol_fields() {
        let request = completion_request(
            &ProviderEndpoint {
                base_url: "https://example.com/v1".to_owned(),
                api_key: None,
                model: "default-model".to_owned(),
            },
            &[ChatInput {
                role: MessageRole::User,
                content: "hello".to_owned(),
            }],
            &ChatParameters {
                temperature: 0.7,
                request_parameters: serde_json::from_value(serde_json::json!({
                    "temperature": 1.25,
                    "model": "attacker-selected-model",
                    "messages": [{"role": "user", "content": "replaced"}],
                    "stream": true,
                    "top_k": 40,
                    "stop": ["END"]
                }))
                .expect("request parameters"),
            },
            false,
        );
        assert!(
            (request["temperature"].as_f64().expect("temperature") - 0.7).abs()
                < f64::from(f32::EPSILON)
        );
        assert_eq!(request["model"], "default-model");
        assert_eq!(request["top_k"], 40);
        assert_eq!(request["stop"], serde_json::json!(["END"]));
        assert_eq!(request["messages"][0]["content"], "hello");
        assert_eq!(request["stream"], false);
    }

    #[test]
    fn rejects_an_unbounded_sse_event() {
        let mut decoder = SseDecoder::new();
        let error = decoder
            .push_bytes(&vec![b'a'; MAX_SSE_EVENT_BYTES + 1])
            .expect_err("oversized event");
        assert!(matches!(error, SseDecodeError::EventTooLarge));
    }

    #[test]
    fn decodes_sse_across_chunk_boundaries() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push("data: {\"a\":").is_empty());
        assert_eq!(
            decoder.push("1}\n\ndata: [DONE]\n\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
    }

    #[test]
    fn decodes_utf8_split_across_network_chunks() {
        let mut decoder = SseDecoder::new();
        let encoded = "data: 你好\n\n".as_bytes();
        let split = encoded
            .windows(2)
            .position(|window| window[0] >= 0x80 && window[1] >= 0x80)
            .expect("multibyte content")
            + 1;
        assert!(
            decoder
                .push_bytes(&encoded[..split])
                .expect("partial")
                .is_empty()
        );
        assert_eq!(
            decoder.push_bytes(&encoded[split..]).expect("complete"),
            vec!["你好"]
        );
    }

    #[test]
    fn decodes_sse_one_byte_at_a_time() {
        let mut decoder = SseDecoder::new();
        let payload = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut events = Vec::new();
        for byte in payload.as_bytes() {
            events.extend(decoder.push_bytes(&[*byte]).expect("byte chunk"));
        }
        assert_eq!(
            events,
            vec![
                "{\"choices\":[{\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}",
                "[DONE]"
            ]
        );
    }

    #[test]
    fn rejects_oversized_sse_event_when_delimiter_arrives() {
        let mut decoder = SseDecoder::new();
        let mut payload = Vec::with_capacity(MAX_SSE_EVENT_BYTES + 16);
        payload.extend_from_slice(b"data: ");
        payload.extend(vec![b'a'; MAX_SSE_EVENT_BYTES]);
        payload.extend_from_slice(b"\n\n");
        let error = decoder
            .push_bytes(&payload)
            .expect_err("oversized complete event");
        assert!(matches!(error, SseDecodeError::EventTooLarge));
    }

    #[tokio::test]
    async fn streams_openai_compatible_deltas_over_http() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = vec![0_u8; 8_192];
            let _ = socket.read(&mut request).await.expect("read request");
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"你\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"好\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let gateway = OpenAiGateway {
            client: Client::builder().no_proxy().build().expect("test client"),
        };
        let mut deltas = Vec::new();
        let completion = gateway
            .stream(
                &ProviderEndpoint {
                    base_url: format!("http://{address}/v1"),
                    api_key: None,
                    model: "test-model".to_owned(),
                },
                &[ChatInput {
                    role: MessageRole::User,
                    content: "hello".to_owned(),
                }],
                ChatParameters::default(),
                |event| {
                    deltas.push(event.delta);
                    true
                },
            )
            .await
            .expect("stream completion");
        assert_eq!(deltas.concat(), "你好");
        assert_eq!(completion.content, "你好");
        assert_eq!(completion.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn streams_when_http_body_arrives_one_byte_at_a_time() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = vec![0_u8; 8_192];
            let _ = socket.read(&mut request).await.expect("read request");
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"slow\"},\"finish_reason\":null}]}\n\n",
                "data: [DONE]\n\n"
            );
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket
                .write_all(header.as_bytes())
                .await
                .expect("write response header");
            for byte in body.as_bytes() {
                socket.write_all(&[*byte]).await.expect("write byte");
                socket.flush().await.expect("flush byte");
                tokio::task::yield_now().await;
            }
        });

        let gateway = OpenAiGateway {
            client: Client::builder().no_proxy().build().expect("test client"),
        };
        let completion = gateway
            .stream(
                &ProviderEndpoint {
                    base_url: format!("http://{address}/v1"),
                    api_key: None,
                    model: "test-model".to_owned(),
                },
                &[ChatInput {
                    role: MessageRole::User,
                    content: "hello".to_owned(),
                }],
                ChatParameters::default(),
                |_| true,
            )
            .await
            .expect("slow stream");
        assert_eq!(completion.content, "slow");
    }

    #[tokio::test]
    async fn rejects_a_stream_that_closes_without_done() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = vec![0_u8; 8_192];
            let _ = socket.read(&mut request).await.expect("read request");
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let gateway = OpenAiGateway {
            client: Client::builder().no_proxy().build().expect("test client"),
        };
        let error = gateway
            .stream(
                &ProviderEndpoint {
                    base_url: format!("http://{address}/v1"),
                    api_key: None,
                    model: "test-model".to_owned(),
                },
                &[ChatInput {
                    role: MessageRole::User,
                    content: "hello".to_owned(),
                }],
                ChatParameters::default(),
                |_| true,
            )
            .await
            .expect_err("missing done must fail");
        assert!(matches!(error, GatewayError::IncompleteStream));
    }

    #[tokio::test]
    async fn stops_stream_when_consumer_rejects_more_deltas() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = vec![0_u8; 8_192];
            let _ = socket.read(&mut request).await.expect("read request");
            let mut body = String::new();
            for index in 0..128 {
                body.push_str(&format!(
                    "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{index},\"}},\"finish_reason\":null}}]}}\n\n"
                ));
            }
            body.push_str("data: [DONE]\n\n");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let gateway = OpenAiGateway {
            client: Client::builder().no_proxy().build().expect("test client"),
        };
        let mut accepted = 0_usize;
        let error = gateway
            .stream(
                &ProviderEndpoint {
                    base_url: format!("http://{address}/v1"),
                    api_key: None,
                    model: "test-model".to_owned(),
                },
                &[ChatInput {
                    role: MessageRole::User,
                    content: "hello".to_owned(),
                }],
                ChatParameters::default(),
                |_| {
                    accepted += 1;
                    accepted < 4
                },
            )
            .await
            .expect_err("consumer rejection must cancel stream");
        assert!(matches!(error, GatewayError::Cancelled));
        assert_eq!(accepted, 4);
    }
}
