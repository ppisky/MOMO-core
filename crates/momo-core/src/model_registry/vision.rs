use super::ModelConnection;
use crate::{
    ChatParameters, GatewayContentPart, GatewayImageUrl, GatewayMessage, GatewayMessageContent,
    GatewayMessageRole, VisionDescriptionAdapter, VisionDescriptionBatch, VisionDescriptionRequest,
    VisionError,
};
use futures_util::{FutureExt, future::BoxFuture};
#[derive(Debug, Clone)]
pub(crate) struct ConfiguredVisionAdapter {
    pub connection: ModelConnection,
    pub parameters: ChatParameters,
}
impl VisionDescriptionAdapter for ConfiguredVisionAdapter {
    fn describe(
        &self,
        request: VisionDescriptionRequest,
    ) -> BoxFuture<'static, Result<VisionDescriptionBatch, VisionError>> {
        let connection = self.connection.clone();
        let parameters = self.parameters.clone();
        async move {
            if connection.model.kind != super::ModelKind::Vision {
                return Err(VisionError::Capability(
                    "the selected model does not support vision; choose a vision parameter group"
                        .into(),
                ));
            }
            let mut batch = VisionDescriptionBatch::default();
            for (index, image) in request.images.into_iter().enumerate() {
                let messages = [
                    GatewayMessage {
                        role: GatewayMessageRole::System,
                        content: Some(request.prompt.clone().into()),
                        tool_calls: vec![],
                        tool_call_id: None,
                    },
                    GatewayMessage {
                        role: GatewayMessageRole::User,
                        content: Some(GatewayMessageContent::Parts(vec![
                            GatewayContentPart::Text {
                                text: format!("Describe image {}.", index + 1),
                            },
                            GatewayContentPart::ImageUrl {
                                image_url: GatewayImageUrl {
                                    url: image.image_url,
                                    detail: image.detail,
                                },
                            },
                        ])),
                        tool_calls: vec![],
                        tool_call_id: None,
                    },
                ];
                let result = connection.complete(&messages, parameters.clone()).await?;
                if !result.tool_calls.is_empty() {
                    return Err(VisionError::UnexpectedToolCall { index });
                }
                if result.content.trim().is_empty() {
                    return Err(VisionError::EmptyDescription { index });
                }
                if result.content.len() > crate::vision::MAX_VISUAL_DESCRIPTION_BYTES {
                    return Err(VisionError::DescriptionTooLarge { index });
                }
                batch.descriptions.push(result.content);
                if let Some(id) = result.upstream_request_id {
                    batch.upstream_request_ids.push(id);
                }
                if let Some(usage) = result.usage {
                    batch.usage.input_tokens =
                        batch.usage.input_tokens.saturating_add(usage.input_tokens);
                    batch.usage.output_tokens = batch
                        .usage
                        .output_tokens
                        .saturating_add(usage.output_tokens);
                    batch.usage.total_tokens =
                        batch.usage.total_tokens.saturating_add(usage.total_tokens);
                }
            }
            Ok(batch)
        }
        .boxed()
    }
}
