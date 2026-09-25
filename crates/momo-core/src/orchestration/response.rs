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
    ChatInput, ContextBudget, ContextRequest, ContextSections, MoStateProfile, MomoResponse,
    MomoResponseMetadata, MomoResponseRequest, PromptSpaceId, ResponseOutputContent,
    ResponseOutputItem, api::runtime_api, prepare_context,
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
        if request.momo.mo_state && config.mo_state.profile == MoStateProfile::ClosedAutonomous {
            warnings.extend(self.recover_due_maintenance(&managed_space_id).await);
        }
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
        let (retrieved, source_observations, retrieval_status) = if retrieval_enabled
            || request.momo.mo_state
        {
            let retrieval_request = runtime_api::ScopedMemoryRequest {
                spaces: request
                    .momo
                    .memory_sources
                    .iter()
                    .map(|source| runtime_api::MemorySpaceSource {
                        space_id: source.space_id.clone(),
                        label: source.label.clone(),
                        weight: source.weight,
                        memory: source.memory,
                        semantic_graph: source.semantic_graph,
                    })
                    .collect(),
                observe_space_ids: vec![managed_space_id.clone()],
                query: input.to_owned(),
                max_tokens: context_window
                    .saturating_sub(reserve_output_tokens)
                    .saturating_div(8)
                    .clamp(128, 2_048),
                vector_space_id: query_embedding.as_ref().map(|value| value.0.clone()),
                query_vector: query_embedding.as_ref().map(|value| value.1.clone()),
                embedding: None,
            };
            match runtime_api::retrieve_scoped_memory_snapshot(self.runtime(), retrieval_request)
                .await
            {
                Ok(snapshot) => (
                    Value::Array(snapshot.items),
                    snapshot.source_observations,
                    if retrieval_enabled { "ok" } else { "disabled" },
                ),
                Err(error) => {
                    warnings.push(format!("retrieval degraded: {error}"));
                    (json!([]), Vec::new(), "degraded")
                }
            }
        } else {
            (json!([]), Vec::new(), "disabled")
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

        let conversation_scope_uuid =
            uuid::Uuid::parse_str(&conversation_space_id).map_err(MomoApiError::internal)?;
        let conversation_uuid =
            uuid::Uuid::parse_str(&conversation_id).map_err(MomoApiError::internal)?;
        let history = self
            .runtime()
            .core()
            .store()
            .list_messages_for_scope(conversation_scope_uuid, conversation_uuid)
            .await
            .map_err(MomoApiError::internal)?;
        let mut messages = history
            .into_iter()
            .map(|message| ChatInput {
                role: message.role,
                content: message.content,
            })
            .collect::<Vec<_>>();
        if request.input.is_structured()
            && (direct_multimodal || !request.input.has_images())
            && messages.last().is_some_and(|message| {
                message.role == momo_domain::MessageRole::User && message.content == input
            })
        {
            messages.pop();
        }
        let assistant_prompt = self.runtime.prompt_spaces().get(PromptSpaceId::Assistant);
        let roleplay_director = self
            .runtime
            .prompt_spaces()
            .get(PromptSpaceId::RoleplayDirector);
        let character_markdown = character.character_markdown.as_str();
        let has_character = !character_markdown.trim().is_empty();
        let assistant_enabled = governed.instructions.is_none() && !has_character;
        let roleplay_director_enabled = config.roleplay.enabled && has_character;
        if governed.instructions.is_none() {
            governed.audit["instructions_source"] = if assistant_enabled {
                json!("prompt_space")
            } else {
                json!("omitted_for_character")
            };
        }
        governed.audit["prompt_spaces"] = json!({
            "assistant_revision": assistant_enabled.then_some(&assistant_prompt.revision),
            "roleplay_director_revision": roleplay_director_enabled.then_some(&roleplay_director.revision),
        });
        let memory_markdown = if include_memory {
            joined_bodies(&prompt_memory)
        } else {
            String::new()
        };
        let nsg_markdown = if include_semantic_graph {
            joined_bodies(&nsg)
        } else {
            String::new()
        };
        let prepared = prepare_context(ContextRequest {
            sections: ContextSections {
                runtime_instructions: governed.instructions.as_deref().unwrap_or({
                    if assistant_enabled {
                        assistant_prompt.content.as_str()
                    } else {
                        ""
                    }
                }),
                roleplay_director: if roleplay_director_enabled {
                    roleplay_director.content.as_str()
                } else {
                    ""
                },
                character: character_markdown,
                user: &character.user_markdown,
                memory: &memory_markdown,
                state: &state_context_for_prompt,
                semantic_graph: &nsg_markdown,
            },
            messages: &messages,
            budget: ContextBudget {
                context_window,
                reserve_output_tokens,
            },
        });
        let mut gateway_messages = prepared
            .messages
            .iter()
            .map(crate::GatewayMessage::from)
            .collect::<Vec<_>>();
        governed.audit["text_context"] = json!({
            "estimated_input_tokens": prepared.estimated_input_tokens,
            "omitted_messages": prepared.omitted_messages,
            "truncated_messages": prepared.truncated_messages,
            "sections": prepared.section_audit,
            "excludes_appended_structured_input": request.input.is_structured()
                && (direct_multimodal || !request.input.has_images()),
        });
        if prepared
            .section_audit
            .iter()
            .any(|section| section.truncated)
        {
            warnings.push("system context sections were truncated; inspect request_audit.text_context.sections".to_owned());
        }
        if request.input.is_structured() && (direct_multimodal || !request.input.has_images()) {
            gateway_messages.extend(
                request
                    .input
                    .gateway_messages()
                    .map_err(|error| MomoApiError::bad_request(error.to_string()))?,
            );
        }
        let mut request_parameters = serde_json::Map::from_iter([
            ("max_tokens".to_owned(), json!(reserve_output_tokens)),
            ("momo_hop".to_owned(), json!(1)),
            ("momo_request_id".to_owned(), json!(operation_key)),
        ]);
        let parameter_object = &mut request_parameters;
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
        let content = completion.content.trim().to_owned();
        let tool_calls = &completion.tool_calls;
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
            let id = call.id.clone();
            let function = &call.function;
            output.push(ResponseOutputItem::FunctionCall {
                id: id.clone(),
                call_id: id,
                name: function.name.clone(),
                arguments: function.arguments.clone(),
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
                finish_reason: completion.finish_reason.clone(),
                momo: MomoResponseMetadata {
                    schema: crate::MOMO_RESPONSE_SCHEMA.to_owned(),
                    request_id: request_id.to_owned(),
                    conversation_id,
                    route: request.model.clone(),
                    upstream_request_id: completion.upstream_request_id.clone(),
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
    character: momo_domain::CharacterCard,
}

impl MomoApiService {
    async fn prepare_response_conversation(
        &self,
        request: &MomoResponseRequest,
        operation_key: &str,
        request_fingerprint: &str,
        persisted: Option<&momo_storage::ResponseOperation>,
        resolved_input: &ResolvedResponseInput,
    ) -> Result<ResponseConversation, MomoApiError> {
        let conversation_space_id = request
            .momo
            .conversation_space_id
            .clone()
            .expect("validated conversation Space");
        let conversation_scope_uuid =
            uuid::Uuid::parse_str(&conversation_space_id).map_err(MomoApiError::bad_request)?;
        let requested_character_id = request.momo.character_id.as_deref();
        let attempted_conversation = persisted
            .map(|operation| operation.conversation_id.clone())
            .or(self
                .coordination
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
            let conversation_uuid =
                uuid::Uuid::parse_str(&id).map_err(MomoApiError::bad_request)?;
            let conversation = self
                .runtime()
                .core()
                .store()
                .conversation_for_scope(conversation_scope_uuid, conversation_uuid)
                .await
                .map_err(MomoApiError::internal)?
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "conversation_id does not belong to conversation_space_id",
                    )
                })?;
            let stored_character_id = conversation
                .character_id
                .ok_or_else(|| {
                    MomoApiError::bad_request(
                        "conversation has no character; use switch_character control first",
                    )
                })?
                .to_string();
            if requested_character_id.is_some_and(|value| value != stored_character_id) {
                return Err(MomoApiError::bad_request(
                    "character_id differs from the conversation; use switch_character control",
                ));
            }
            (id, stored_character_id)
        } else {
            let character_id = requested_character_id.ok_or_else(|| {
                MomoApiError::bad_request("character_id is required for a new conversation")
            })?;
            let character_uuid =
                uuid::Uuid::parse_str(character_id).map_err(MomoApiError::bad_request)?;
            if self
                .runtime()
                .core()
                .store()
                .character_by_id(character_uuid)
                .await
                .map_err(MomoApiError::internal)?
                .is_none()
            {
                return Err(MomoApiError::bad_request("character_id does not exist"));
            }
            let now = chrono::Utc::now();
            let conversation = momo_domain::Conversation {
                id: momo_domain::new_id(),
                scope_id: conversation_scope_uuid,
                character_id: Some(character_uuid),
                title: request
                    .momo
                    .title
                    .clone()
                    .unwrap_or_else(|| "MOMO response".to_owned()),
                created_at: now,
                updated_at: now,
            };
            self.runtime()
                .core()
                .store()
                .save_conversation(&conversation)
                .await
                .map_err(MomoApiError::internal)?;
            let id = conversation.id.to_string();
            self.coordination
                .response_attempts
                .lock()
                .await
                .insert(operation_key.to_owned(), id.clone());
            (id, character_id.to_owned())
        };
        let character_uuid =
            uuid::Uuid::parse_str(&character_id).map_err(MomoApiError::bad_request)?;
        let character = self
            .runtime()
            .core()
            .store()
            .character_by_id(character_uuid)
            .await
            .map_err(MomoApiError::internal)?
            .ok_or_else(|| MomoApiError::bad_request("character_id does not exist"))?;
        self.coordination
            .response_attempts
            .lock()
            .await
            .entry(operation_key.to_owned())
            .or_insert_with(|| conversation_id.clone());
        let resolved_input_json =
            serde_json::to_string(resolved_input).map_err(MomoApiError::internal)?;
        self.runtime()
            .core()
            .store()
            .begin_response_operation(
                operation_key,
                request_fingerprint,
                &conversation_id,
                &resolved_input_json,
            )
            .await
            .map_err(MomoApiError::internal)?;
        self.coordination
            .response_attempts
            .lock()
            .await
            .remove(operation_key);

        let user_already_written = persisted
            .map(|operation| operation.user_written)
            .unwrap_or(false);
        if !user_already_written {
            let persisted_user_text = resolved_input.persisted_user_text.as_deref().or_else(|| {
                (!request.input.has_function_outputs()).then_some(resolved_input.text.as_str())
            });
            if let Some(user_text) = persisted_user_text {
                let conversation_uuid =
                    uuid::Uuid::parse_str(&conversation_id).map_err(MomoApiError::internal)?;
                let message = momo_domain::Message {
                    id: momo_domain::new_id(),
                    conversation_id: conversation_uuid,
                    role: momo_domain::MessageRole::User,
                    content: user_text.to_owned(),
                    created_at: chrono::Utc::now(),
                };
                self.runtime()
                    .core()
                    .store()
                    .append_response_user_message(operation_key, conversation_scope_uuid, &message)
                    .await
                    .map_err(MomoApiError::internal)?;
            } else {
                self.runtime()
                    .core()
                    .store()
                    .mark_response_user_written(operation_key)
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
