//! Background DMW and NSG maintenance batches.

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::{MaintenanceKind, MomoApiError, MomoApiService};
use crate::{
    ChatParameters, GatewayMessage, GatewayMessageContent, GatewayMessageRole, OpenAiGateway,
    PromptSpaceId, ProviderEndpoint,
};

fn maintenance_batch_key(scope_id: &str, kind: &str, request_ids: &[String]) -> String {
    let mut hasher = Sha256::new();
    for value in std::iter::once(scope_id)
        .chain(std::iter::once(kind))
        .chain(request_ids.iter().map(String::as_str))
    {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    format!("maintenance:{}", hex::encode(hasher.finalize()))
}

fn opaque_hyphenated_identifiers(value: &str) -> std::collections::HashSet<&str> {
    value
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        })
        .filter(|token| {
            token.len() >= 8
                && token.matches('-').count() >= 2
                && token
                    .chars()
                    .any(|character| character.is_ascii_alphabetic())
                && token.chars().any(|character| character.is_ascii_digit())
        })
        .collect()
}

pub(super) fn validate_generated_opaque_identifiers(
    source: &str,
    generated: &str,
) -> Result<(), String> {
    let allowed = opaque_hyphenated_identifiers(source);
    if let Some(identifier) = opaque_hyphenated_identifiers(generated)
        .into_iter()
        .find(|identifier| !allowed.contains(identifier))
    {
        return Err(format!(
            "generated patch altered or invented opaque identifier {identifier:?}; copy identifiers from evidence character-for-character"
        ));
    }
    Ok(())
}

pub(super) fn memory_patch_is_noop(patch: &str) -> bool {
    patch.split_whitespace().collect::<String>() == "patches:[]"
}

pub(super) fn maintenance_finish_error(completion: &Value) -> Option<String> {
    let finish_reason = completion.get("finish_reason").and_then(Value::as_str);
    (finish_reason != Some("stop")).then(|| {
        format!(
            "maintenance model did not finish normally (finish_reason={})",
            finish_reason.unwrap_or("missing")
        )
    })
}

impl MomoApiService {
    pub(super) async fn recover_due_maintenance(&self, scope_id: &str) -> Vec<String> {
        let config = self.config_snapshot();
        let mut warnings = Vec::new();
        for (kind, enabled, threshold) in [
            (
                MaintenanceKind::Memory,
                config.runtime.memory_distillation_enabled,
                config.runtime.memory_distill_every_turns,
            ),
            (
                MaintenanceKind::SemanticGraph,
                config.runtime.semantic_graph_enabled,
                config.runtime.nsg_govern_every_turns,
            ),
        ] {
            if !enabled {
                continue;
            }
            if let Err(error) = self.maintain(scope_id, kind, threshold).await {
                warnings.push(format!(
                    "{} maintenance recovery degraded: {error}",
                    kind.storage_name()
                ));
            }
        }
        warnings
    }

    pub(super) fn schedule_maintenance(&self, scope_id: String) {
        let config = self.config_snapshot();
        for (kind, enabled, threshold) in [
            (
                MaintenanceKind::Memory,
                config.runtime.memory_distillation_enabled,
                config.runtime.memory_distill_every_turns,
            ),
            (
                MaintenanceKind::SemanticGraph,
                config.runtime.semantic_graph_enabled,
                config.runtime.nsg_govern_every_turns,
            ),
        ] {
            if !enabled {
                continue;
            }
            let service = self.clone();
            let scope_id = scope_id.clone();
            let task = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if service.coordination.has_active_responses(&scope_id) {
                    return;
                }
                if let Err(error) = service.maintain(&scope_id, kind, threshold).await {
                    tracing::warn!(?kind, %error, "background response maintenance failed; turns remain pending");
                }
            });
            let mut tasks = self
                .coordination
                .maintenance_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.retain(|task| !task.is_finished());
            tasks.push(task);
        }
    }

    /// Waits until all background maintenance spawned by this service has
    /// finished. Transports should call this after they stop accepting work.
    pub async fn wait_for_maintenance(&self) {
        loop {
            let tasks = {
                let mut tasks = self
                    .coordination
                    .maintenance_tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *tasks)
            };
            if tasks.is_empty() {
                self.runtime().core().wait_for_commits().await;
                return;
            }
            for task in tasks {
                if let Err(error) = task.await {
                    tracing::warn!(%error, "background maintenance task did not shut down cleanly");
                }
            }
        }
    }

    /// Runs one governed Core maintenance batch. Callers only choose when to schedule it.
    pub async fn maintain(
        &self,
        scope_id: &str,
        kind: MaintenanceKind,
        threshold: usize,
    ) -> Result<bool, MomoApiError> {
        let normalized_scope = uuid::Uuid::parse_str(scope_id)
            .map_err(MomoApiError::bad_request)?
            .to_string();
        let scope_id = normalized_scope.as_str();
        let config = self.config_snapshot();
        if threshold == 0 {
            return Err(MomoApiError::bad_request(
                "maintenance threshold must be positive",
            ));
        }
        let storage_kind = kind.storage_name();
        let lock_key = format!("{scope_id}:{storage_kind}");
        let task_lock = {
            let mut locks = self.coordination.maintenance_locks.lock().await;
            if locks.len() >= 1_024 {
                locks.retain(|_, lock| lock.strong_count() > 0);
            }
            if let Some(lock) = locks.get(&lock_key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(lock_key, Arc::downgrade(&lock));
                lock
            }
        };
        let _guard = task_lock.lock_owned().await;
        let storage_kind_value = match kind {
            MaintenanceKind::Memory => momo_storage::MaintenanceKind::Memory,
            MaintenanceKind::SemanticGraph => momo_storage::MaintenanceKind::SemanticGraph,
        };
        let pending = self
            .runtime()
            .core()
            .store()
            .pending_maintenance_batch(scope_id, storage_kind)
            .await
            .map_err(MomoApiError::internal)?;
        if let Some(pending) = pending
            .as_ref()
            .filter(|pending| pending.prepared_commit_json.is_some())
        {
            self.finish_maintenance_commit(pending.clone(), _guard)
                .await?;
            return Ok(true);
        }
        let turns = if let Some(pending) = &pending {
            self.runtime()
                .core()
                .store()
                .maintenance_turns_for_batch(&pending.batch)
                .await
                .map_err(MomoApiError::internal)?
        } else {
            let turns = self
                .runtime()
                .core()
                .store()
                .pending_maintenance_turns(scope_id, storage_kind_value, threshold)
                .await
                .map_err(MomoApiError::internal)?;
            if turns.len() < threshold {
                return Ok(false);
            }
            turns
        };
        let request_ids = turns
            .iter()
            .map(|turn| turn.request_id.clone())
            .collect::<Vec<_>>();
        let batch_key = pending
            .as_ref()
            .map(|pending| pending.batch.batch_key.clone())
            .unwrap_or_else(|| maintenance_batch_key(scope_id, storage_kind, &request_ids));
        let mut repair_error: Option<String> = None;
        let mut staged = self
            .runtime()
            .core()
            .store()
            .maintenance_batch_patch(&batch_key)
            .await
            .map_err(MomoApiError::internal)?;
        if let Some(patch) = staged.clone() {
            let validation =
                validate_maintenance_patch(self.runtime(), scope_id, kind, patch).await;
            if let Err(error) = validation {
                if !self
                    .runtime()
                    .core()
                    .store()
                    .discard_maintenance_batch(&batch_key)
                    .await
                    .map_err(MomoApiError::internal)?
                {
                    return Err(MomoApiError::internal(
                        "invalid staged maintenance batch disappeared before regeneration",
                    ));
                }
                tracing::warn!(%batch_key, %error, ?kind, "discarded invalid staged maintenance patch");
                repair_error = Some(error.to_string());
                staged = None;
            }
        }
        let patch = if let Some(staged) = staged {
            staged
        } else {
            let transcript = turns
                .iter()
                .enumerate()
                .map(|(index, turn)| {
                    format!(
                        "Turn {}\nUser:\n{}\nAssistant:\n{}",
                        index + 1,
                        turn.user_content,
                        turn.assistant_content,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let parsed_scope_id =
                uuid::Uuid::parse_str(scope_id).map_err(MomoApiError::bad_request)?;
            let existing_context = crate::api::runtime_api::retrieve_memory_items(
                self.runtime(),
                parsed_scope_id,
                transcript.clone(),
                4_096,
            )
            .await
            .map_err(|error| {
                MomoApiError::internal(format!(
                    "maintenance context retrieval failed; refusing transcript-only write: {error}"
                ))
            })?;
            let maintenance_input = json!({
                "maintenance_kind": storage_kind,
                "mo_state_profile": config.mo_state.profile.as_str(),
                "scene_management": config.mo_state.scene_management,
                "current_unix_timestamp": Utc::now().timestamp(),
                "existing_context": existing_context,
                "pending_turns": turns,
            });
            let maintenance_source = maintenance_input.to_string();
            let (route, prompt_space_id) = match kind {
                MaintenanceKind::Memory => {
                    ("memory_distillation", PromptSpaceId::MemoryDistillation)
                }
                MaintenanceKind::SemanticGraph => (
                    "semantic_graph_governance",
                    PromptSpaceId::SemanticGraphGovernance,
                ),
            };
            let system = self.runtime.prompt_spaces().get(prompt_space_id).content;
            let maintenance_max_tokens = self
                .gateway_generation_capability(route)
                .await
                .ok()
                .map(|capability| capability.max_output_tokens);
            let mut maintenance_parameters = json!({"momo_hop": 1});
            if let Some(max_tokens) = maintenance_max_tokens {
                maintenance_parameters["max_tokens"] = json!(max_tokens);
            }
            let generated_patch = loop {
                let user_content = repair_error.as_ref().map_or_else(
                    || maintenance_input.to_string(),
                    |error| {
                        let required_correction = match kind {
                            MaintenanceKind::Memory => "Regenerate the complete patch. Obey every field and target constraint; do not repeat the invalid output. For create operations, frontmatter type must be exactly character under characters/, relationship under relationships/, event under events/, or world under world/.",
                            MaintenanceKind::SemanticGraph => "Regenerate the complete semantic-graph patch. Obey every field, target, node, and automatic-authority constraint; do not repeat the invalid output.",
                        };
                        json!({
                            "maintenance_input": &maintenance_input,
                            "previous_output_error": error,
                            "required_correction": required_correction,
                        })
                        .to_string()
                    },
                );
                let generation_gate = self
                    .coordination
                    .generation_gate(scope_id, "maintenance")
                    .await;
                let generation_permit = generation_gate
                    .acquire_owned()
                    .await
                    .map_err(|_| MomoApiError::internal("generation gate closed"))?;
                let messages = vec![
                    GatewayMessage {
                        role: GatewayMessageRole::System,
                        content: Some(GatewayMessageContent::Text(system.clone())),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    GatewayMessage {
                        role: GatewayMessageRole::User,
                        content: Some(GatewayMessageContent::Text(user_content)),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                ];
                let completion = OpenAiGateway::default()
                    .complete_messages(
                        &ProviderEndpoint {
                            base_url: self.gateway_origin.clone(),
                            api_key: self.gateway_api_key.clone(),
                            model: route.to_owned(),
                        },
                        &messages,
                        ChatParameters {
                            temperature: None,
                            request_parameters: maintenance_parameters
                                .as_object()
                                .cloned()
                                .expect("maintenance parameters are an object"),
                        },
                    )
                    .await
                    .map_err(MomoApiError::model)?;
                drop(generation_permit);
                let completion =
                    serde_json::to_value(completion).map_err(MomoApiError::internal)?;
                if let Some(error) = maintenance_finish_error(&completion) {
                    if repair_error.is_none() {
                        repair_error = Some(format!(
                            "{error}. Regenerate a smaller complete patch. Prefer fewer complete targets and concise sections; never continue or complete the truncated YAML fragment."
                        ));
                        continue;
                    }
                    return Err(MomoApiError::model(error));
                }
                let generated = completion
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| MomoApiError::model("maintenance model returned no text"))?
                    .to_owned();
                let validation = if kind == MaintenanceKind::Memory {
                    validate_generated_opaque_identifiers(&maintenance_source, &generated)
                } else {
                    Ok(())
                };
                let validation = match validation {
                    Ok(()) => {
                        validate_maintenance_patch(
                            self.runtime(),
                            scope_id,
                            kind,
                            generated.clone(),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                match validation {
                    Ok(_)
                        if kind == MaintenanceKind::Memory
                            && memory_patch_is_noop(&generated)
                            && repair_error.is_none() =>
                    {
                        repair_error = Some(
                            "The first pass returned an empty patch for a non-empty maintenance batch. Recheck every pending turn for explicit durable facts, corrections, commitments, boundaries, and activated outcomes. Return patches: [] again only if none are supported."
                                .to_owned(),
                        );
                    }
                    Ok(_) => break generated,
                    Err(error) if repair_error.is_none() => {
                        repair_error = Some(error.to_string());
                    }
                    Err(error) => {
                        return Err(MomoApiError::model(format!(
                            "{storage_kind} maintenance model returned an invalid patch after repair: {error}"
                        )));
                    }
                }
            };
            self.runtime()
                .core()
                .store()
                .stage_maintenance_batch(&momo_storage::MaintenanceBatch {
                    batch_key: batch_key.clone(),
                    scope_id: scope_id.to_owned(),
                    kind: storage_kind.to_owned(),
                    request_ids: request_ids.clone(),
                    patch_yaml: generated_patch,
                })
                .await
                .map_err(MomoApiError::internal)?
        };
        // Once preparation starts, the owned task holds the Space lock through
        // file application and SQL acknowledgement, even if its caller is cancelled.
        let runtime = Arc::clone(&self.runtime);
        let scope_id = scope_id.to_owned();
        let batch = momo_storage::MaintenanceBatch {
            batch_key,
            scope_id: scope_id.clone(),
            kind: storage_kind.to_owned(),
            request_ids,
            patch_yaml: patch.clone(),
        };
        self.runtime
            .finish_commit(async move {
                let _maintenance_guard = _guard;
                let guard = runtime
                    .lock_space(
                        uuid::Uuid::parse_str(&scope_id).map_err(MomoApiError::bad_request)?,
                    )
                    .await
                    .map_err(MomoApiError::internal)?;
                let parsed_scope_id =
                    uuid::Uuid::parse_str(&scope_id).map_err(MomoApiError::bad_request)?;
                let core = runtime.core_handle();
                let plan = tokio::task::spawn_blocking(move || {
                    let memory = core.memory_for_space(parsed_scope_id)?;
                    match kind {
                        MaintenanceKind::Memory => memory.prepare_patch_commit(&patch),
                        MaintenanceKind::SemanticGraph => {
                            momo_memory::nsg::NsgWorkspace::initialize(memory.root())?
                                .prepare_patch_commit(&patch)
                        }
                    }
                })
                .await
                .map_err(MomoApiError::internal)?
                .map_err(MomoApiError::internal)?;
                let json = serde_json::to_string(&plan).map_err(MomoApiError::internal)?;
                runtime.core().mark_memory_commit_pending(parsed_scope_id);
                let prepared = runtime
                    .core()
                    .store()
                    .prepare_maintenance_commit(&batch.batch_key, &json)
                    .await
                    .map_err(MomoApiError::internal)?;
                runtime
                    .core()
                    .commit_prepared_maintenance(&momo_storage::PendingMaintenanceBatch {
                        batch,
                        prepared_commit_json: Some(prepared),
                    })
                    .await
                    .map_err(MomoApiError::internal)?;
                drop(guard);
                Ok::<_, MomoApiError>(())
            })
            .await
            .map_err(MomoApiError::internal)??;
        Ok(true)
    }

    async fn finish_maintenance_commit(
        &self,
        pending: momo_storage::PendingMaintenanceBatch,
        maintenance_guard: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<(), MomoApiError> {
        let runtime = Arc::clone(&self.runtime);
        self.runtime
            .finish_commit(async move {
                let _maintenance_guard = maintenance_guard;
                runtime.core().mark_memory_commit_pending(
                    uuid::Uuid::parse_str(&pending.batch.scope_id)
                        .map_err(MomoApiError::bad_request)?,
                );
                let _guard = runtime
                    .lock_space(
                        uuid::Uuid::parse_str(&pending.batch.scope_id)
                            .map_err(MomoApiError::bad_request)?,
                    )
                    .await
                    .map_err(MomoApiError::internal)?;
                Ok(())
            })
            .await
            .map_err(MomoApiError::internal)?
    }
}

async fn validate_maintenance_patch(
    runtime: &crate::MomoRuntime,
    scope_id: &str,
    kind: MaintenanceKind,
    patch: String,
) -> Result<(), String> {
    let parsed_scope_id = uuid::Uuid::parse_str(scope_id).map_err(|error| error.to_string())?;
    let _state_guard = runtime
        .lock_space(parsed_scope_id)
        .await
        .map_err(|error| error.to_string())?;
    let core = runtime.core_handle();
    tokio::task::spawn_blocking(move || {
        let memory = core
            .memory_for_space(parsed_scope_id)
            .map_err(|error| error.to_string())?;
        match kind {
            MaintenanceKind::Memory => memory
                .validate_patch(&patch)
                .map_err(|error| error.to_string()),
            MaintenanceKind::SemanticGraph => {
                momo_memory::nsg::NsgWorkspace::initialize(memory.root())
                    .map_err(|error| error.to_string())?
                    .validate_patch(&patch)
                    .map_err(|error| error.to_string())
            }
        }
    })
    .await
    .map_err(|error| format!("maintenance validation task failed: {error}"))?
}
