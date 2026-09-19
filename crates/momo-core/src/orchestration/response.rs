//! End-to-end response assembly and its local stages.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    GovernedResponseAttempt, ImageInputHandling, MomoApiError, MomoApiService,
    ResolvedResponseInput,
    generation::{GatewayResponseAttempt, response_usage, tool_choice_to_chat, tools_to_chat},
    state,
};
use crate::{
    MomoResponse, MomoResponseMetadata, MomoResponseRequest, ResponseOutputContent,
    ResponseOutputItem, api::simple, product_prompts,
};

impl MomoApiService {
    pub(super) async fn respond(
        &self,
        request: &MomoResponseRequest,
        attempt: GovernedResponseAttempt<'_>,
    ) -> Result<(MomoResponse, Option<String>), MomoApiError> {
        let config = self.config_snapshot();
        let GovernedResponseAttempt {
            request_id,
            operation_key,
            request_fingerprint,
            persisted,
            resolved_input,
            mut governed,
            mut warnings,
            stream,
        } = attempt;
        let input = resolved_input.text.as_str();
        let direct_multimodal =
            resolved_input.image_handling == ImageInputHandling::DirectMultimodal;
        let personal_space_id = request
            .momo
            .personal_space_id
            .clone()
            .expect("validated personal Space");
        let conversation = self
            .prepare_response_conversation(
                request,
                operation_key,
                request_fingerprint,
                persisted,
                resolved_input,
            )
            .await?;
        let conversation_space_id = conversation.conversation_space_id;
        let conversation_id = conversation.conversation_id;
        let character_id = conversation.character_id;
        let character = conversation.character;

        let context_window = governed.context_window;
        let reserve_output_tokens = governed.max_output_tokens;
        let managed_space_id = request
            .momo
            .memory_write_space_id
            .clone()
            .unwrap_or_else(|| personal_space_id.clone());
        let include_memory = request
            .momo
            .memory_sources
            .iter()
            .any(|source| source.memory);
        let include_semantic_graph = request
            .momo
            .memory_sources
            .iter()
            .any(|source| source.semantic_graph);
        let query_embedding = if include_semantic_graph {
            match self.generate_query_embedding(input).await {
                Ok(value) => Some(value),
                Err(error) => {
                    warnings.push(format!("embedding degraded: {error}"));
                    None
                }
            }
        } else {
            None
        };
        let retrieval_enabled = !request.momo.memory_sources.is_empty();
        let (retrieved, source_observations, retrieval_status) =
            if retrieval_enabled || request.momo.mo_state {
                let payload = json!({
                    "spaces": request.momo.memory_sources,
                    "observe_space_ids": [managed_space_id],
                    "query": input,
                    "max_tokens": context_window
                        .saturating_sub(reserve_output_tokens)
                        .saturating_div(8)
                        .clamp(128, 2_048),
                    "vector_space_id": query_embedding.as_ref().map(|value| &value.0),
                    "query_vector": query_embedding.as_ref().map(|value| &value.1),
                });
                match simple::retrieve_scoped_memory_snapshot_json(payload.to_string()).await {
                    Ok(value) => {
                        let snapshot = serde_json::from_str::<Value>(&value)
                            .map_err(|error| MomoApiError::internal(error.to_string()))?;
                        (
                            snapshot.get("items").cloned().unwrap_or_else(|| json!([])),
                            snapshot
                                .get("source_observations")
                                .cloned()
                                .unwrap_or_else(|| json!([])),
                            if retrieval_enabled { "ok" } else { "disabled" },
                        )
                    }
                    Err(error) => {
                        warnings.push(format!("retrieval degraded: {error}"));
                        (json!([]), json!([]), "degraded")
                    }
                }
            } else {
                (json!([]), json!([]), "disabled")
            };
        let items = retrieved
            .as_array()
            .cloned()
            .ok_or_else(|| MomoApiError::internal("retrieval result must be an array"))?;
        governed.audit["memory_retrieval"] =
            retrieval_audit(&items, retrieval_enabled, retrieval_status);
        let (nsg, memory): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|item| item.get("graph_id").is_some());
        let state_input_audit = state_input_audit(&memory, &nsg, retrieval_status)?;
        let state = self
            .compile_response_state(state::StateRequest {
                request,
                config: &config,
                operation_key,
                request_fingerprint,
                managed_space_id: &managed_space_id,
                conversation_id: &conversation_id,
                character_id: &character_id,
                source_observations: &source_observations,
                memory: &memory,
                nsg: &nsg,
                state_input_audit,
                context_window,
                resolved_input,
            })
            .await?;
        let state_result = state.result;
        let mo_state_operation_id = state.operation_id;
        let state_context_for_prompt = state.context_for_prompt;
        let state_injection_status = state.injection_status;
        let _state_guard = state.guard;
        warnings.extend(state.warnings);
        let (prompt_memory, memory_prompt_filter) =
            filter_memory_for_prompt(&memory, &state_result, state_injection_status == "active");
        governed.audit["memory_prompt_filter"] = memory_prompt_filter;

        let history = parse_json(
            simple::local_messages_json(conversation_space_id, conversation_id.clone()).await,
        )?;
        let mut messages = history
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|message| {
                Some(json!({
                    "role": message.get("role")?.as_str()?,
                    "content": message.get("content")?.as_str()?,
                }))
            })
            .collect::<Vec<_>>();
        if request.input.is_structured()
            && (direct_multimodal || !request.input.has_images())
            && messages.last().is_some_and(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && message.get("content").and_then(Value::as_str) == Some(input)
            })
        {
            messages.pop();
        }
        let prepared = parse_json(simple::prepare_context_json(
            json!({
                "runtime_instructions": governed.instructions.as_deref().unwrap_or_default(),
                "roleplay_director": if config.roleplay.enabled
                    && character.get("character_markdown").and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                {
                    product_prompts::ROLEPLAY_DIRECTOR
                } else {
                    ""
                },
                "character_markdown": character.get("character_markdown").and_then(Value::as_str).unwrap_or_default(),
                "user_markdown": character.get("user_markdown").and_then(Value::as_str).unwrap_or_default(),
                "memory_markdown": if include_memory { joined_bodies(&prompt_memory) } else { String::new() },
                "state_context": state_context_for_prompt,
                "nsg_markdown": if include_semantic_graph { joined_bodies(&nsg) } else { String::new() },
                "messages": messages,
                "context_window": context_window,
                "reserve_output_tokens": reserve_output_tokens,
            })
            .to_string(),
        ))?;
        let mut gateway_messages = prepared
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        governed.audit["text_context"] = json!({
            "estimated_input_tokens": prepared.get("estimated_input_tokens"),
            "omitted_messages": prepared.get("omitted_messages"),
            "truncated_messages": prepared.get("truncated_messages"),
            "sections": prepared.get("section_audit"),
            "excludes_appended_structured_input": request.input.is_structured()
                && (direct_multimodal || !request.input.has_images()),
        });
        if prepared
            .get("section_audit")
            .and_then(Value::as_array)
            .is_some_and(|sections| {
                sections
                    .iter()
                    .any(|section| section.get("truncated").and_then(Value::as_bool) == Some(true))
            })
        {
            warnings.push("system context sections were truncated; inspect request_audit.text_context.sections".to_owned());
        }
        if request.input.is_structured() && (direct_multimodal || !request.input.has_images()) {
            gateway_messages.extend(
                request
                    .input
                    .gateway_messages()
                    .map_err(|error| MomoApiError::bad_request(error.to_string()))?
                    .into_iter()
                    .map(|message| serde_json::to_value(message).expect("message serializes")),
            );
        }
        let mut request_parameters = json!({
            "max_tokens": reserve_output_tokens,
            "momo_hop": 1,
            "momo_request_id": operation_key,
        });
        let parameter_object = request_parameters
            .as_object_mut()
            .expect("request parameters are an object");
        parameter_object.extend(governed.parameters.clone());
        if governed.allow_tools && !request.tools.is_empty() {
            parameter_object.insert("tools".to_owned(), tools_to_chat(&request.tools)?);
        }
        if governed.allow_tools
            && let Some(choice) = &request.tool_choice
        {
            parameter_object.insert("tool_choice".to_owned(), tool_choice_to_chat(choice)?);
        }
        let completion = self
            .generate_response_completion(GatewayResponseAttempt {
                request_id,
                operation_key,
                personal_space_id: &personal_space_id,
                model: &request.model,
                messages: gateway_messages,
                temperature: governed.temperature,
                request_parameters,
                stream,
            })
            .await?;
        let content = required_str(&completion, "content")?.trim().to_owned();
        let tool_calls = completion
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if content.is_empty() && tool_calls.is_empty() {
            return Err(MomoApiError::model(
                "model gateway returned neither text nor tool calls",
            ));
        }
        let mut output = vec![ResponseOutputItem::Message {
            id: format!("msg_{request_id}"),
            role: "assistant".to_owned(),
            status: "completed".to_owned(),
            content: vec![ResponseOutputContent::OutputText {
                text: content.clone(),
                annotations: Vec::new(),
            }],
        }];
        for call in tool_calls {
            let id = required_str(&call, "id")?.to_owned();
            let function = call
                .get("function")
                .ok_or_else(|| MomoApiError::model("tool call is missing function"))?;
            output.push(ResponseOutputItem::FunctionCall {
                id: id.clone(),
                call_id: id,
                name: required_str(function, "name")?.to_owned(),
                arguments: required_str(function, "arguments")?.to_owned(),
                status: "completed".to_owned(),
            });
        }
        Ok((
            MomoResponse {
                id: format!("resp_{request_id}"),
                object: "response".to_owned(),
                status: "completed".to_owned(),
                model: request.model.clone(),
                output,
                output_text: content,
                usage: response_usage(&completion),
                finish_reason: completion
                    .get("finish_reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                momo: MomoResponseMetadata {
                    schema: crate::MOMO_RESPONSE_SCHEMA.to_owned(),
                    request_id: request_id.to_owned(),
                    conversation_id,
                    route: request.model.clone(),
                    upstream_request_id: completion
                        .get("upstream_request_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    warnings,
                    state_audit: state_result
                        .get("audit")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                    request_audit: governed.audit,
                },
            },
            mo_state_operation_id,
        ))
    }
}

fn parse_json(result: Result<String, String>) -> Result<Value, MomoApiError> {
    let value = result.map_err(MomoApiError::internal)?;
    serde_json::from_str(&value).map_err(|error| MomoApiError::internal(error.to_string()))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, MomoApiError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| MomoApiError::internal(format!("response is missing string field {field}")))
}

pub(super) fn joined_bodies(values: &[Value]) -> String {
    values
        .iter()
        .filter_map(|value| {
            let body = value.get("body").and_then(Value::as_str)?.trim();
            if body.is_empty() {
                return None;
            }
            let source = value.get("memory_space");
            let label = source
                .and_then(|source| source.get("label"))
                .and_then(Value::as_str);
            let record_id = value
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("graph_id").and_then(Value::as_str));
            Some(match (label, record_id) {
                (Some(label), Some(record_id)) => format!(
                    "[Memory record: {} | source: {}]\n{body}",
                    serde_json::to_string(record_id).expect("string serializes"),
                    serde_json::to_string(label).expect("string serializes")
                ),
                (None, Some(record_id)) => format!(
                    "[Memory record: {}]\n{body}",
                    serde_json::to_string(record_id).expect("string serializes")
                ),
                _ => body.to_owned(),
            })
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn filter_memory_for_prompt(
    values: &[Value],
    state_result: &Value,
    state_injection_active: bool,
) -> (Vec<Value>, Value) {
    let requested = if state_injection_active {
        state_result
            .pointer("/audit/prompt_excluded_memory_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };
    let mut excluded = Vec::new();
    let retained = values
        .iter()
        .filter_map(|value| {
            let id = value.get("id").and_then(Value::as_str);
            if id.is_some_and(|id| requested.contains(id)) {
                excluded.push(id.expect("checked id").to_owned());
                None
            } else {
                Some(value.clone())
            }
        })
        .collect::<Vec<_>>();
    excluded.sort();
    (
        retained,
        json!({
            "applied": !excluded.is_empty(),
            "excluded_count": excluded.len(),
            "excluded_ids": excluded,
        }),
    )
}

pub(super) fn retrieval_audit(items: &[Value], enabled: bool, status: &str) -> Value {
    let entries = items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("graph_id").and_then(Value::as_str))?;
            Some(json!({
                "id": id,
                "kind": if item.get("graph_id").is_some() { "nsg" } else { "dmw" },
                "space_id": item.pointer("/memory_space/id").and_then(Value::as_str),
                "space_label": item.pointer("/memory_space/label").and_then(Value::as_str),
                "estimated_tokens": item.get("estimated_tokens").and_then(Value::as_u64),
            }))
        })
        .collect::<Vec<_>>();
    json!({"enabled": enabled, "status": status, "count": entries.len(), "entries": entries})
}

pub(super) fn state_input_audit(
    memory: &[Value],
    nsg: &[Value],
    retrieval_status: &str,
) -> Result<Value, MomoApiError> {
    let encoded = serde_json::to_vec(&json!({"memory": memory, "nsg": nsg}))
        .map_err(|error| MomoApiError::internal(error.to_string()))?;
    let source_spaces = memory
        .iter()
        .chain(nsg)
        .filter_map(|item| item.pointer("/memory_space/id").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    Ok(json!({
        "scope": "retrieved_subset",
        "retrieval_status": retrieval_status,
        "fingerprint_sha256": hex::encode(Sha256::digest(encoded)),
        "memory_count": memory.len(),
        "nsg_count": nsg.len(),
        "source_space_ids": source_spaces,
    }))
}

struct ResponseConversation {
    conversation_space_id: String,
    conversation_id: String,
    character_id: String,
    character: Value,
}

impl MomoApiService {
    async fn prepare_response_conversation(
        &self,
        request: &MomoResponseRequest,
        operation_key: &str,
        request_fingerprint: &str,
        persisted: Option<&Value>,
        resolved_input: &ResolvedResponseInput,
    ) -> Result<ResponseConversation, MomoApiError> {
        let conversation_space_id = request
            .momo
            .conversation_space_id
            .clone()
            .expect("validated conversation Space");
        let requested_character_id = request.momo.character_id.as_deref();
        let attempted_conversation = persisted
            .and_then(|operation| operation["conversation_id"].as_str())
            .map(str::to_owned)
            .or(self
                .response_attempts
                .lock()
                .await
                .get(operation_key)
                .cloned());
        let existing_conversation_id = request
            .momo
            .conversation_id
            .as_deref()
            .map(str::to_owned)
            .or(attempted_conversation);
        let (conversation_id, character_id) = if let Some(id) = existing_conversation_id {
            let conversations =
                parse_json(simple::local_conversations_json(conversation_space_id.clone()).await)?;
            let conversation = conversations
                .as_array()
                .and_then(|items| {
                    items.iter().find(|conversation| {
                        conversation.get("id").and_then(Value::as_str) == Some(id.as_str())
                    })
                })
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "conversation_id does not belong to conversation_space_id",
                    )
                })?;
            let stored_character_id = conversation
                .get("character_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "conversation has no character; use switch_character control first",
                    )
                })?;
            if requested_character_id.is_some_and(|value| value != stored_character_id) {
                return Err(MomoApiError::bad_request(
                    "character_id differs from the conversation; use switch_character control",
                ));
            }
            (id, stored_character_id.to_owned())
        } else {
            let character_id = requested_character_id.ok_or_else(|| {
                MomoApiError::bad_request("character_id is required for a new conversation")
            })?;
            let created = parse_json(
                simple::stage_conversation_json(
                    None,
                    conversation_space_id.clone(),
                    request
                        .momo
                        .title
                        .clone()
                        .unwrap_or_else(|| "MOMO response".to_owned()),
                    character_id.to_owned(),
                )
                .await,
            )?;
            let id = required_str(&created, "id")?.to_owned();
            self.response_attempts
                .lock()
                .await
                .insert(operation_key.to_owned(), id.clone());
            (id, character_id.to_owned())
        };
        let character = parse_json(simple::local_character_json(character_id.clone()).await)
            .map_err(|_| MomoApiError::bad_request("character_id does not exist"))?;
        self.response_attempts
            .lock()
            .await
            .entry(operation_key.to_owned())
            .or_insert_with(|| conversation_id.clone());
        simple::begin_response_operation(
            operation_key.to_owned(),
            request_fingerprint.to_owned(),
            conversation_id.clone(),
            serde_json::to_string(resolved_input)
                .map_err(|error| MomoApiError::internal(error.to_string()))?,
        )
        .await
        .map_err(MomoApiError::internal)?;
        self.response_attempts.lock().await.remove(operation_key);

        let user_already_written = persisted
            .and_then(|operation| operation["user_written"].as_bool())
            .unwrap_or(false);
        if !user_already_written {
            let persisted_user_text = resolved_input.persisted_user_text.as_deref().or_else(|| {
                (!request.input.has_function_outputs()).then_some(resolved_input.text.as_str())
            });
            if let Some(user_text) = persisted_user_text {
                simple::append_response_user_message_json(
                    operation_key.to_owned(),
                    conversation_space_id.clone(),
                    conversation_id.clone(),
                    user_text.to_owned(),
                )
                .await
                .map_err(MomoApiError::internal)?;
            } else {
                simple::mark_response_user_written(operation_key.to_owned())
                    .await
                    .map_err(MomoApiError::internal)?;
            }
        }

        Ok(ResponseConversation {
            conversation_space_id,
            conversation_id,
            character_id,
            character,
        })
    }
}
