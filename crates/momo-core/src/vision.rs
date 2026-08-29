//! Governed image-to-text adapter used before the native conversation pipeline.

use std::fmt;

use futures_util::{FutureExt, StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ChatParameters, ChatUsage, GatewayContentPart, GatewayImageUrl, GatewayMessage,
    GatewayMessageContent, GatewayMessageRole, OpenAiGateway, ProviderEndpoint, ResponseImageInput,
};

pub const DEFAULT_VISION_ROUTE: &str = "vision";
pub const MAX_VISUAL_DESCRIPTION_BYTES: usize = 256 * 1024;
const MAX_VISION_DISCOVERY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct VisionDescriptionRequest {
    pub images: Vec<ResponseImageInput>,
    pub prompt: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VisionDescriptionBatch {
    pub descriptions: Vec<String>,
    #[serde(default)]
    pub usage: ChatUsage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_request_ids: Vec<String>,
}

pub trait VisionDescriptionAdapter: fmt::Debug + Send + Sync {
    fn describe(
        &self,
        request: VisionDescriptionRequest,
    ) -> BoxFuture<'static, Result<VisionDescriptionBatch, VisionError>>;
}

#[derive(Debug, Clone)]
pub struct GatewayVisionAdapter {
    client: reqwest::Client,
    gateway: OpenAiGateway,
    endpoint: ProviderEndpoint,
}

impl GatewayVisionAdapter {
    #[must_use]
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        api_key: Option<String>,
        route: impl Into<String>,
    ) -> Self {
        Self {
            client: client.clone(),
            gateway: OpenAiGateway::from_client(client),
            endpoint: ProviderEndpoint {
                base_url: base_url.into(),
                api_key,
                model: route.into(),
            },
        }
    }
}

impl VisionDescriptionAdapter for GatewayVisionAdapter {
    fn describe(
        &self,
        request: VisionDescriptionRequest,
    ) -> BoxFuture<'static, Result<VisionDescriptionBatch, VisionError>> {
        let gateway = self.gateway.clone();
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        async move {
            ensure_gateway_vision_capability(&client, &endpoint).await?;
            let VisionDescriptionRequest {
                images,
                prompt,
                request_id,
            } = request;
            let image_count = images.len();
            let requests = images.into_iter().enumerate().map(|(index, image)| {
                let gateway = gateway.clone();
                let endpoint = endpoint.clone();
                let prompt = prompt.clone();
                let request_id = request_id.clone();
                async move {
                    describe_one(
                        &gateway,
                        &endpoint,
                        image,
                        &prompt,
                        &request_id,
                        index,
                        image_count,
                    )
                    .await
                }
            });
            let results = futures_util::future::try_join_all(requests).await?;
            let mut batch = VisionDescriptionBatch::default();
            for result in results {
                batch.descriptions.push(result.description);
                batch.usage.input_tokens = batch
                    .usage
                    .input_tokens
                    .saturating_add(result.usage.input_tokens);
                batch.usage.output_tokens = batch
                    .usage
                    .output_tokens
                    .saturating_add(result.usage.output_tokens);
                batch.usage.total_tokens = batch
                    .usage
                    .total_tokens
                    .saturating_add(result.usage.total_tokens);
                if let Some(request_id) = result.upstream_request_id {
                    batch.upstream_request_ids.push(request_id);
                }
            }
            Ok(batch)
        }
        .boxed()
    }
}

struct SingleDescription {
    description: String,
    usage: ChatUsage,
    upstream_request_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn describe_one(
    gateway: &OpenAiGateway,
    endpoint: &ProviderEndpoint,
    image: ResponseImageInput,
    prompt: &str,
    request_id: &str,
    index: usize,
    image_count: usize,
) -> Result<SingleDescription, VisionError> {
    let messages = [
        GatewayMessage {
            role: GatewayMessageRole::System,
            content: Some(prompt.into()),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        GatewayMessage {
            role: GatewayMessageRole::User,
            content: Some(GatewayMessageContent::Parts(vec![
                GatewayContentPart::Text {
                    text: format!("Describe image {} of {image_count}.", index + 1),
                },
                GatewayContentPart::ImageUrl {
                    image_url: GatewayImageUrl {
                        url: image.image_url,
                        detail: image.detail,
                    },
                },
            ])),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let mut request_parameters = Map::new();
    request_parameters.insert("max_tokens".to_owned(), json!(1024));
    request_parameters.insert("momo_hop".to_owned(), json!(1));
    request_parameters.insert(
        "momo_request_id".to_owned(),
        json!(format!(
            "vision_{}",
            hex::encode(Sha256::digest(format!("{request_id}:{index}")))
        )),
    );
    let completion = gateway
        .complete_messages(
            endpoint,
            &messages,
            ChatParameters {
                temperature: Some(0.0),
                request_parameters,
            },
        )
        .await?;
    if !completion.tool_calls.is_empty() {
        return Err(VisionError::UnexpectedToolCall { index });
    }
    let description = completion.content.trim().to_owned();
    if description.is_empty() {
        return Err(VisionError::EmptyDescription { index });
    }
    if description.len() > MAX_VISUAL_DESCRIPTION_BYTES {
        return Err(VisionError::DescriptionTooLarge { index });
    }
    let mut usage = completion.usage.unwrap_or_default();
    if usage.total_tokens == 0 {
        usage.total_tokens = usage.input_tokens.saturating_add(usage.output_tokens);
    }
    Ok(SingleDescription {
        description,
        usage,
        upstream_request_id: completion.upstream_request_id,
    })
}

async fn ensure_gateway_vision_capability(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
) -> Result<(), VisionError> {
    let mut url = reqwest::Url::parse(&endpoint.base_url)
        .map_err(|error| VisionError::Capability(error.to_string()))?;
    {
        let mut segments = url.path_segments_mut().map_err(|()| {
            VisionError::Capability("gateway base URL cannot be a base".to_owned())
        })?;
        segments.pop_if_empty();
        segments.push("models");
        segments.push(&endpoint.model);
    }
    let mut request = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(api_key) = endpoint.api_key.as_deref() {
        request = request.bearer_auth(api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| VisionError::Capability(error.to_string()))?;
    if !response.status().is_success() {
        return Err(VisionError::Capability(format!(
            "vision route discovery returned HTTP {}",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_VISION_DISCOVERY_BYTES as u64)
    {
        return Err(VisionError::Capability(
            "vision route discovery response is too large".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| VisionError::Capability(error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_VISION_DISCOVERY_BYTES {
            return Err(VisionError::Capability(
                "vision route discovery response is too large".to_owned(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| VisionError::Capability(error.to_string()))?;
    let supports_images = document
        .pointer("/momo/modalities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|modalities| {
            modalities
                .iter()
                .any(|modality| modality.as_str() == Some("image"))
        });
    if !supports_images {
        return Err(VisionError::Capability(
            "vision route does not advertise the image modality".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum VisionError {
    #[error("visual-description capability discovery failed: {0}")]
    Capability(String),
    #[error("visual-description gateway failed: {0}")]
    Gateway(#[from] crate::GatewayError),
    #[error("visual-description adapter returned no text for image {index}")]
    EmptyDescription { index: usize },
    #[error(
        "visual-description adapter exceeded the {MAX_VISUAL_DESCRIPTION_BYTES} byte limit for image {index}"
    )]
    DescriptionTooLarge { index: usize },
    #[error("visual-description adapter returned a tool call for image {index}")]
    UnexpectedToolCall { index: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn gateway_adapter_discovers_image_capability_and_describes_input() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut discovery, _) = listener.accept().await.expect("discovery connection");
            let discovery_request = read_request(&mut discovery).await;
            assert!(discovery_request.starts_with("GET /v1/models/vision "));
            write_json(
                &mut discovery,
                json!({"id": "vision", "momo": {"modalities": ["text", "image"]}}),
            )
            .await;

            let (mut completion, _) = listener.accept().await.expect("completion connection");
            let completion_request = read_request(&mut completion).await;
            assert!(completion_request.starts_with("POST /v1/chat/completions "));
            let body = completion_request
                .split_once("\r\n\r\n")
                .expect("request body")
                .1;
            let body: serde_json::Value = serde_json::from_str(body).expect("request JSON");
            assert_eq!(body["model"], "vision");
            assert_eq!(body["messages"][1]["content"][1]["type"], "image_url");
            assert_eq!(
                body["messages"][1]["content"][1]["image_url"]["detail"],
                "high"
            );
            write_json(
                &mut completion,
                json!({
                    "id": "vision-upstream-1",
                    "choices": [{
                        "message": {"role": "assistant", "content": "A red umbrella."},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}
                }),
            )
            .await;
        });
        let adapter = GatewayVisionAdapter::new(
            reqwest::Client::new(),
            format!("http://{address}/v1"),
            None,
            DEFAULT_VISION_ROUTE,
        );
        let batch = adapter
            .describe(VisionDescriptionRequest {
                images: vec![ResponseImageInput {
                    image_url: "https://example.test/image.png".to_owned(),
                    detail: Some("high".to_owned()),
                }],
                prompt: "Describe visible facts.".to_owned(),
                request_id: "request-vision-test".to_owned(),
            })
            .await
            .expect("vision description");
        server.await.expect("server task");
        assert_eq!(batch.descriptions, ["A red umbrella."]);
        assert_eq!(batch.usage.input_tokens, 10);
        assert_eq!(batch.usage.output_tokens, 4);
        assert_eq!(batch.usage.total_tokens, 14);
        assert_eq!(batch.upstream_request_ids, ["vision-upstream-1"]);
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut expected = None;
        loop {
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).await.expect("read request");
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if expected.is_none()
                && let Some(header_end) = bytes.windows(4).position(|value| value == b"\r\n\r\n")
            {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                    })
                    .unwrap_or(0);
                expected = Some(header_end + 4 + content_length);
            }
            if expected.is_some_and(|length| bytes.len() >= length) {
                break;
            }
        }
        String::from_utf8(bytes).expect("UTF-8 request")
    }

    async fn write_json(stream: &mut tokio::net::TcpStream, value: serde_json::Value) {
        let body = value.to_string();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("write response");
    }
}
