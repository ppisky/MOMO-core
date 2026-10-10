//! Conversion between Core's message contract and native provider protocols.
use super::{ModelConnection, ProviderProtocol as Protocol};
use crate::{
    ChatCompletion, ChatFunctionCall, ChatParameters, ChatToolCall, ChatUsage,
    GatewayContentPart as Part, GatewayError, GatewayMessage, GatewayMessageContent as Content,
    GatewayMessageRole as Role,
};
use serde_json::{Value, json};

fn invalid(message: &str) -> GatewayError {
    GatewayError::InvalidMessage(message.into())
}
fn text_parts(content: &Option<Content>) -> Vec<Part> {
    match content {
        Some(Content::Text(text)) => vec![Part::Text { text: text.clone() }],
        Some(Content::Parts(parts)) => parts.clone(),
        None => vec![],
    }
}
fn image_source(url: &str) -> Result<Value, GatewayError> {
    if let Some(data) = url.strip_prefix("data:") {
        let (mime, bytes) = data
            .split_once(";base64,")
            .ok_or_else(|| invalid("image must use a base64 data URL"))?;
        Ok(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":bytes}}))
    } else {
        Ok(json!({"type":"image","source":{"type":"url","url":url}}))
    }
}
impl ModelConnection {
    pub(super) fn protocol_body(
        &self,
        messages: &[GatewayMessage],
        parameters: ChatParameters,
        stream: bool,
    ) -> Result<Value, GatewayError> {
        crate::gateway::validate_gateway_messages(messages)?;
        let protocol = self.effective_protocol();
        let mut body = json!({"model":self.model.model_id,"stream":stream});
        let mut items = Vec::new();
        let mut system = Vec::new();
        for message in messages {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            if message.role == Role::Tool {
                let output = match &message.content {
                    Some(Content::Text(t)) => t.clone(),
                    _ => return Err(invalid("tool output must be text")),
                };
                let id = message.tool_call_id.as_deref().unwrap_or_default();
                items.push(match protocol {
                    Protocol::OpenaiResponses=>json!({"type":"function_call_output","call_id":id,"output":output}),
                    Protocol::Anthropic=>json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":output}]}),
                    Protocol::Openai=>unreachable!()
                });
                continue;
            }
            let mut parts = Vec::new();
            for part in text_parts(&message.content) {
                parts.push(match part {
                    Part::Text{text}=>match protocol {Protocol::OpenaiResponses=>json!({"type":if role=="assistant"{"output_text"}else{"input_text"},"text":text}),_=>json!({"type":"text","text":text})},
                    Part::ImageUrl{image_url}=>if protocol==Protocol::OpenaiResponses {json!({"type":"input_image","image_url":image_url.url,"detail":image_url.detail.unwrap_or_else(||"auto".into())})} else {image_source(&image_url.url)?}
                });
            }
            let mut response_calls = Vec::new();
            for call in &message.tool_calls {
                match protocol {
                    Protocol::OpenaiResponses=>response_calls.push(json!({"type":"function_call","call_id":call.id,"name":call.function.name,"arguments":call.function.arguments})),
                    Protocol::Anthropic=>parts.push(json!({"type":"tool_use","id":call.id,"name":call.function.name,"input":serde_json::from_str::<Value>(&call.function.arguments)?})),
                    Protocol::Openai=>unreachable!()
                }
            }
            if parts.is_empty() {
                items.extend(response_calls);
                continue;
            }
            if role == "system" && protocol != Protocol::OpenaiResponses {
                system.extend(parts);
            } else {
                items.push(json!({"role":role,"content":parts}));
            }
            items.extend(response_calls);
        }
        match protocol {
            Protocol::OpenaiResponses => {
                body["input"] = json!(items);
                body["store"] = json!(false);
            }
            Protocol::Anthropic => {
                body["messages"] = json!(items);
                if !system.is_empty() {
                    body["system"] = json!(system);
                }
            }

            Protocol::Openai => unreachable!(),
        }
        if let Some(t) = parameters.temperature {
            body["temperature"] = json!(t);
        }
        for (key, value) in parameters.request_parameters {
            // These are gateway control metadata, never provider payload fields.
            if key == "momo_hop" {
                continue;
            }
            if matches!(
                key.as_str(),
                "model"
                    | "messages"
                    | "input"
                    | "contents"
                    | "system"
                    | "systemInstruction"
                    | "stream"
                    | "store"
                    | "temperature"
            ) {
                return Err(invalid("reserved provider request parameter"));
            }
            if key == "max_tokens" || key == "max_completion_tokens" || key == "max_output_tokens" {
                match protocol {
                    Protocol::OpenaiResponses => body["max_output_tokens"] = value,
                    _ => body["max_tokens"] = value,
                };
            } else if key == "tools" {
                let functions = value
                    .as_array()
                    .ok_or_else(|| invalid("tools must be an array"))?
                    .iter()
                    .map(|v| {
                        v.get("function")
                            .cloned()
                            .ok_or_else(|| invalid("only function tools are supported"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                body["tools"]=match protocol {
                    Protocol::OpenaiResponses=>json!(functions.into_iter().map(|mut f|{f["type"]=json!("function");f}).collect::<Vec<_>>()),
                    Protocol::Anthropic=>json!(functions.into_iter().map(|f|json!({"name":f["name"],"description":f.get("description").unwrap_or(&json!("")),"input_schema":f["parameters"]})).collect::<Vec<_>>()),
                    Protocol::Openai=>unreachable!()
                };
            } else if key == "tool_choice" {
                let named = value.pointer("/function/name").and_then(Value::as_str);
                body["tool_choice"] = match protocol {
                    Protocol::OpenaiResponses => {
                        if let Some(name) = named {
                            json!({"type":"function","name":name})
                        } else {
                            value
                        }
                    }
                    Protocol::Anthropic => {
                        if let Some(name) = named {
                            json!({"type":"tool","name":name})
                        } else {
                            json!({"type":match value.as_str(){Some("auto")=>"auto",Some("required")=>"any",Some("none")=>"none",_=>return Err(invalid("invalid tool choice"))}})
                        }
                    }

                    Protocol::Openai => unreachable!(),
                };
            } else {
                body[key] = value;
            }
        }
        // Anthropic requires max_tokens even if the host left its override blank.
        if protocol == Protocol::Anthropic && body.get("max_tokens").is_none() {
            body["max_tokens"] = json!(self.model.max_output_tokens.unwrap_or(1024));
        }
        Ok(body)
    }
}
fn call(id: &str, name: &str, arguments: String) -> ChatToolCall {
    ChatToolCall {
        id: id.into(),
        call_type: "function".into(),
        function: ChatFunctionCall {
            name: name.into(),
            arguments,
        },
    }
}
pub(super) fn completion(
    protocol: Protocol,
    value: &Value,
) -> Result<ChatCompletion, GatewayError> {
    let mut result = ChatCompletion {
        content: String::new(),
        tool_calls: vec![],
        finish_reason: None,
        usage: None,
        upstream_request_id: value
            .get("id")
            .or_else(|| value.get("responseId"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    match protocol {
        Protocol::OpenaiResponses => {
            if value["status"] != "completed" {
                return Err(invalid("Responses request did not complete"));
            }
            for item in value["output"]
                .as_array()
                .ok_or(GatewayError::MissingChoice)?
            {
                if item["type"] == "function_call" {
                    result.tool_calls.push(call(
                        item["call_id"].as_str().unwrap_or_default(),
                        item["name"].as_str().unwrap_or_default(),
                        item["arguments"].as_str().unwrap_or_default().into(),
                    ));
                }
                if let Some(parts) = item["content"].as_array() {
                    for part in parts {
                        if part["type"] == "output_text" {
                            result
                                .content
                                .push_str(part["text"].as_str().unwrap_or_default());
                        }
                    }
                }
            }
            result.finish_reason = Some(
                if result.tool_calls.is_empty() {
                    "stop"
                } else {
                    "tool_calls"
                }
                .into(),
            );
            result.usage = Some(usage(&value["usage"], "input_tokens", "output_tokens"));
        }
        Protocol::Anthropic => {
            for part in value["content"]
                .as_array()
                .ok_or(GatewayError::MissingChoice)?
            {
                if part["type"] == "text" {
                    result
                        .content
                        .push_str(part["text"].as_str().unwrap_or_default());
                }
                if part["type"] == "tool_use" {
                    result.tool_calls.push(call(
                        part["id"].as_str().unwrap_or_default(),
                        part["name"].as_str().unwrap_or_default(),
                        part["input"].to_string(),
                    ));
                }
            }
            result.finish_reason = Some(
                match value["stop_reason"].as_str() {
                    Some("tool_use") => "tool_calls",
                    Some("max_tokens") => "length",
                    Some("end_turn" | "stop_sequence") => "stop",
                    _ => return Err(GatewayError::IncompleteStream),
                }
                .into(),
            );
            result.usage = Some(usage(&value["usage"], "input_tokens", "output_tokens"));
        }

        Protocol::Openai => unreachable!(),
    }
    if result
        .tool_calls
        .iter()
        .any(|t| t.id.is_empty() || t.function.name.is_empty())
    {
        return Err(invalid("provider returned an incomplete tool call"));
    }
    Ok(result)
}
pub(super) fn usage(value: &Value, input: &str, output: &str) -> ChatUsage {
    let input_tokens = value[input].as_u64().unwrap_or(0);
    let output_tokens = value[output].as_u64().unwrap_or(0);
    ChatUsage {
        input_tokens,
        output_tokens,
        total_tokens: value["total_tokens"]
            .as_u64()
            .or_else(|| value["totalTokenCount"].as_u64())
            .unwrap_or(input_tokens.saturating_add(output_tokens)),
    }
}
