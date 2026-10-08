//! Host-authorized Space lifecycle. UI names and the host's last-Space rule
//! remain outside Core. Core owns the durable cross-store asset transition.
use crate::{
    CoreError, MomoCore,
    api::runtime_api::{RuntimeApiError, RuntimeApiResult},
};
use momo_domain::provenance::RevisionContext;
use momo_memory::{
    MemorySnapshot,
    provenance::{DMW_LEDGER, NSG_LEDGER, ProvenanceLedger, RecordRevision},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpaceAssets {
    pub space_id: Uuid,
    pub revision: String,
    pub characters: usize,
    pub conversations: usize,
    pub messages: usize,
    pub memories: usize,
    pub nsg_nodes: usize,
    pub nsg_candidates: usize,
    pub deleted_items: usize,
    pub pending_reviews: usize,
    pub pending_turns: usize,
    pub settings_overrides: usize,
    pub external_conversations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpaceDisposition {
    Migrate { target_id: Uuid },
    DeleteAssets,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpaceRemovalRequest {
    pub operation_id: Uuid,
    pub source_id: Uuid,
    pub expected_revision: String,
    pub disposition: SpaceDisposition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpaceRemovalReceipt {
    pub operation_id: Uuid,
    pub source_id: Uuid,
    pub disposition: SpaceDisposition,
    pub assets: SpaceAssets,
}

#[derive(Debug, Serialize, Deserialize)]
struct RemovalPlan {
    target: Option<MemorySnapshot>,
    settings: Option<String>,
}

fn internal(error: impl ToString) -> RuntimeApiError {
    RuntimeApiError::internal(error.to_string())
}
fn conflict(error: impl ToString) -> RuntimeApiError {
    RuntimeApiError::conflict(error.to_string())
}

impl MomoCore {
    /// Transports hold this guard through work spawned for an admitted request,
    /// including SSE producers. Removal takes exclusive ownership of the gate.
    pub async fn admit_space_operation(&self) -> tokio::sync::OwnedRwLockReadGuard<()> {
        Arc::clone(&self.lifecycle_gate).read_owned().await
    }

    pub fn space_transition_pending(&self) -> bool {
        self.pending_space_transition
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn require_live_space(&self, id: Uuid) -> Result<(), momo_memory::MemoryError> {
        if self.retired_spaces.call(move |spaces| spaces.contains(&id)) {
            return Err(momo_memory::MemoryError::InvalidPatch(format!(
                "Space {id} retired or removal pending"
            )));
        }
        Ok(())
    }

    pub(crate) fn admit_settings_operation(
        &self,
    ) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, crate::GovernanceError> {
        let guard = Arc::clone(&self.lifecycle_gate)
            .try_read_owned()
            .map_err(|_| crate::GovernanceError::Invalid("Space transition is active".into()))?;
        if self
            .pending_space_transition
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(crate::GovernanceError::Invalid(
                "Space transition requires recovery before settings can change".into(),
            ));
        }
        Ok(guard)
    }

    pub(crate) fn try_admit_space_operation(
        &self,
    ) -> RuntimeApiResult<tokio::sync::OwnedRwLockReadGuard<()>> {
        Arc::clone(&self.lifecycle_gate)
            .try_read_owned()
            .map_err(|_| conflict("Space transition is active"))
    }

    pub async fn space_assets(&self, id: Uuid) -> RuntimeApiResult<SpaceAssets> {
        let _gate = self.admit_space_operation().await;
        let _guards = self.reserve_spaces([id]).await.map_err(internal)?;
        self.inventory_locked(id).await
    }

    async fn inventory_locked(&self, id: Uuid) -> RuntimeApiResult<SpaceAssets> {
        self.require_live_space(id).map_err(conflict)?;
        let memory = self.memory_for_space(id).map_err(internal)?;
        let documents = memory.list_documents().map_err(internal)?;
        let nsg = memory.nsg().map_err(internal)?;
        let nodes = nsg.list_nodes(true).map_err(internal)?;
        let candidates = nsg.list_pending_candidates().map_err(internal)?;
        let snapshot = memory.export_snapshot().map_err(internal)?;
        let settings = crate::runtime::settings::SettingsStore::load(
            &self.data_dir.join("runtime-settings.json"),
        )
        .map_err(internal)?
        .space_override(id);
        let assistant: Option<String> = sqlx::query_scalar(
            "SELECT character_id FROM default_assistants WHERE personal_space_id=?",
        )
        .bind(id.to_string())
        .fetch_optional(self.store.pool())
        .await
        .map_err(internal)?;
        let characters = self
            .store
            .list_characters_for_scope(id)
            .await
            .map_err(internal)?;
        let conversations = self
            .store
            .list_conversations_for_scope(id)
            .await
            .map_err(internal)?;
        let mut messages = Vec::new();
        let mut contexts = Vec::new();
        for conversation in &conversations {
            let history = self
                .store
                .list_messages(conversation.id)
                .await
                .map_err(internal)?;
            let ids = history.iter().map(|message| message.id).collect::<Vec<_>>();
            contexts.extend(
                self.store
                    .message_contexts_for_scope(id, &ids)
                    .await
                    .map_err(internal)?,
            );
            messages.extend(history);
        }
        let deleted = self.owned_deleted_items(id).await?;
        let reviews: String = sqlx::query_scalar("SELECT json_group_array(json_object('id',id,'status',status,'patch',patch_yaml,'prepared',prepared_commit_json)) FROM memory_patch_reviews WHERE scope_id=? ORDER BY id")
            .bind(id.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
        let turns: String = sqlx::query_scalar("SELECT json_group_array(json_object('id',request_id,'memory',memory_done,'nsg',nsg_done)) FROM maintenance_turns WHERE scope_id=? ORDER BY request_id")
            .bind(id.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
        let metadata: String = sqlx::query_scalar("SELECT json_group_array(json_object('kind',kind,'id',object_id,'document',document)) FROM portable_metadata WHERE object_id IN (SELECT id FROM character_cards WHERE scope_id=?) ORDER BY kind,object_id")
            .bind(id.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
        let external_conversations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE scope_id<>? AND character_id IN (SELECT id FROM character_cards WHERE scope_id=?)")
            .bind(id.to_string()).bind(id.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
        let pending_reviews: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_patch_reviews WHERE scope_id=? AND status='pending'",
        )
        .bind(id.to_string())
        .fetch_one(self.store.pool())
        .await
        .map_err(internal)?;
        let pending_turns: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maintenance_turns WHERE scope_id=? AND (memory_done=0 OR nsg_done=0)")
            .bind(id.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
        let bytes = serde_json::to_vec(&(
            &characters,
            &conversations,
            &messages,
            &contexts,
            &snapshot,
            &deleted,
            &reviews,
            &turns,
            &metadata,
            &settings,
            &assistant,
            external_conversations,
        ))
        .map_err(internal)?;
        Ok(SpaceAssets {
            space_id: id,
            revision: hex::encode(Sha256::digest(bytes)),
            characters: characters.len(),
            conversations: conversations.len(),
            messages: messages.len(),
            memories: documents
                .iter()
                .filter(|document| document.kind != "current")
                .count(),
            nsg_nodes: nodes.len(),
            nsg_candidates: candidates.len(),
            deleted_items: deleted.len(),
            pending_reviews: pending_reviews as usize,
            pending_turns: pending_turns as usize,
            settings_overrides: usize::from(settings.is_some()) + usize::from(assistant.is_some()),
            external_conversations: external_conversations as usize,
        })
    }

    async fn owned_deleted_items(
        &self,
        id: Uuid,
    ) -> RuntimeApiResult<Vec<(String, String, String)>> {
        let rows = sqlx::query("SELECT object_type,object_id,payload FROM local_tombstones WHERE COALESCE(json_extract(payload,'$.character.owner_space_id'),json_extract(payload,'$.conversation.space_id'))=? OR (object_type='message' AND json_extract(payload,'$.conversation_id') IN (SELECT id FROM conversations WHERE scope_id=? UNION SELECT json_extract(payload,'$.conversation.id') FROM local_tombstones WHERE json_extract(payload,'$.conversation.space_id')=?)) ORDER BY object_type,object_id")
            .bind(id.to_string()).bind(id.to_string()).bind(id.to_string()).fetch_all(self.store.pool()).await.map_err(internal)?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("object_type").map_err(internal)?,
                    row.try_get("object_id").map_err(internal)?,
                    row.try_get("payload").map_err(internal)?,
                ))
            })
            .collect()
    }

    pub async fn remove_space_assets(
        &self,
        request: SpaceRemovalRequest,
    ) -> RuntimeApiResult<SpaceRemovalReceipt> {
        self.remove_space_assets_with_publish(request, || Ok(()))
            .await
    }

    pub(crate) async fn remove_space_assets_with_publish(
        &self,
        request: SpaceRemovalRequest,
        publish: impl FnOnce() -> RuntimeApiResult<()> + Send + 'static,
    ) -> RuntimeApiResult<SpaceRemovalReceipt> {
        if request.expected_revision.len() != 64
            || !request
                .expected_revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(RuntimeApiError::invalid(
                "expected asset revision is required",
            ));
        }
        let core = self.clone();
        // A disconnected caller cannot abandon a prepared transition.
        self.finish_commit(async move {
            let _gate = Arc::clone(&core.lifecycle_gate).write_owned().await;
            let receipt = core.remove_space_locked(request).await?;
            let pending: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM space_removals WHERE completed=0")
                    .fetch_one(core.store.pool())
                    .await
                    .map_err(internal)?;
            // An old receipt can be replayed while a later transition needs
            // recovery. Do not publish its partial settings or unfreeze writes.
            if pending == 0 {
                publish()?;
                core.pending_space_transition
                    .store(false, std::sync::atomic::Ordering::Release);
            }
            Ok(receipt)
        })
        .await
        .map_err(internal)?
    }

    async fn remove_space_locked(
        &self,
        request: SpaceRemovalRequest,
    ) -> RuntimeApiResult<SpaceRemovalReceipt> {
        if let Some(row) = sqlx::query(
            "SELECT request_json,receipt_json,completed FROM space_removals WHERE operation_id=?",
        )
        .bind(request.operation_id.to_string())
        .fetch_optional(self.store.pool())
        .await
        .map_err(internal)?
        {
            let previous: SpaceRemovalRequest =
                serde_json::from_str(row.try_get("request_json").map_err(internal)?)
                    .map_err(internal)?;
            if previous != request {
                return Err(conflict("operation ID reused with a different removal"));
            }
            if row.try_get::<i64, _>("completed").map_err(internal)? == 0 {
                self.finish_space_removal(request.operation_id).await?;
            }
            return serde_json::from_str(row.try_get("receipt_json").map_err(internal)?)
                .map_err(internal);
        }
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM space_removals WHERE completed=0")
                .fetch_one(self.store.pool())
                .await
                .map_err(internal)?;
        if pending > 0 {
            return Err(conflict("Another Space transition needs recovery").with_code("space_busy"));
        }
        let mut spaces = vec![request.source_id];
        if let SpaceDisposition::Migrate { target_id } = request.disposition {
            if target_id == request.source_id {
                return Err(RuntimeApiError::invalid(
                    "migration target must be another Space",
                ));
            }
            self.require_live_space(target_id).map_err(conflict)?;
            let source_assistant: Option<String> = sqlx::query_scalar(
                "SELECT character_id FROM default_assistants WHERE personal_space_id=?",
            )
            .bind(request.source_id.to_string())
            .fetch_optional(self.store.pool())
            .await
            .map_err(internal)?;
            let target_assistant: Option<String> = sqlx::query_scalar(
                "SELECT character_id FROM default_assistants WHERE personal_space_id=?",
            )
            .bind(target_id.to_string())
            .fetch_optional(self.store.pool())
            .await
            .map_err(internal)?;
            if source_assistant.is_some()
                && target_assistant.is_some()
                && source_assistant != target_assistant
            {
                return Err(conflict("Default assistant settings conflict")
                    .with_code("space_migration_conflict"));
            }
            spaces.push(target_id);
        }
        let _guards = self
            .reserve_spaces(spaces.clone())
            .await
            .map_err(internal)?;
        let assets = self.inventory_locked(request.source_id).await?;
        if assets.revision != request.expected_revision {
            return Err(
                conflict("Refresh assets and confirm again").with_code("space_assets_changed")
            );
        }
        for space in spaces {
            let unfinished: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM response_operations WHERE response_json IS NULL AND conversation_id IN (SELECT id FROM conversations WHERE scope_id=?)")
                .bind(space.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
            if unfinished > 0 {
                return Err(conflict("Resolve unfinished responses first")
                    .with_code("space_response_pending"));
            }
            let prepared:i64=sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM maintenance_batches WHERE scope_id=? AND prepared_commit_json IS NOT NULL)+(SELECT COUNT(*) FROM memory_patch_reviews WHERE scope_id=? AND prepared_commit_json IS NOT NULL)+(SELECT COUNT(*) FROM memory_lifecycle_events WHERE space_id=? AND prepared_commit_json IS NOT NULL)")
                .bind(space.to_string()).bind(space.to_string()).bind(space.to_string()).fetch_one(self.store.pool()).await.map_err(internal)?;
            if prepared > 0 {
                return Err(conflict("Finish prepared memory commits before removal")
                    .with_code("space_response_pending"));
            }
        }
        let settings = crate::runtime::settings::SettingsStore::load(
            &self.data_dir.join("runtime-settings.json"),
        )
        .map_err(internal)?
        .removal_image(
            request.source_id,
            match request.disposition {
                SpaceDisposition::Migrate { target_id } => Some(target_id),
                SpaceDisposition::DeleteAssets => None,
            },
        )
        .map_err(|error| conflict(error).with_code("space_migration_conflict"))?;
        let plan = match request.disposition {
            SpaceDisposition::DeleteAssets => RemovalPlan {
                target: None,
                settings,
            },
            SpaceDisposition::Migrate { target_id } => {
                let source = self
                    .memory_for_space(request.source_id)
                    .map_err(internal)?
                    .export_snapshot()
                    .map_err(internal)?;
                let destination = self
                    .memory_for_space(target_id)
                    .map_err(internal)?
                    .export_snapshot()
                    .map_err(internal)?;
                RemovalPlan {
                    target: Some(
                        merge_snapshots(
                            source,
                            destination,
                            request.source_id,
                            target_id,
                            request.operation_id,
                            &self.data_dir,
                        )
                        .map_err(|error| error.with_code("space_migration_conflict"))?,
                    ),
                    settings,
                }
            }
        };
        let receipt = SpaceRemovalReceipt {
            operation_id: request.operation_id,
            source_id: request.source_id,
            disposition: request.disposition.clone(),
            assets,
        };
        sqlx::query("INSERT INTO space_removals(operation_id,source_id,request_json,plan_json,receipt_json) VALUES(?,?,?,?,?)")
            .bind(request.operation_id.to_string()).bind(request.source_id.to_string()).bind(serde_json::to_string(&request).map_err(internal)?)
            .bind(serde_json::to_string(&plan).map_err(internal)?).bind(serde_json::to_string(&receipt).map_err(internal)?)
            .execute(self.store.pool()).await.map_err(conflict)?;
        let disposition = request.disposition.clone();
        self.retired_spaces.call(move |spaces| {
            spaces.insert(request.source_id);
            if let SpaceDisposition::Migrate { target_id } = disposition {
                spaces.insert(target_id);
            }
        });
        self.pending_space_transition
            .store(true, std::sync::atomic::Ordering::Release);
        self.finish_space_removal(request.operation_id).await?;
        Ok(receipt)
    }

    /// Startup replays exact after-images before publishing any API or worker.
    pub(crate) async fn recover_space_removals(&self) -> Result<(), CoreError> {
        let rows =
            sqlx::query("SELECT operation_id,source_id,request_json,completed FROM space_removals")
                .fetch_all(self.store.pool())
                .await
                .map_err(momo_storage::StorageError::from)?;
        for row in rows {
            let source = Uuid::parse_str(
                row.try_get("source_id")
                    .map_err(momo_storage::StorageError::from)?,
            )
            .map_err(|error| momo_memory::MemoryError::InvalidPatch(error.to_string()))?;
            self.retired_spaces.call(move |spaces| {
                spaces.insert(source);
            });
            if row
                .try_get::<i64, _>("completed")
                .map_err(momo_storage::StorageError::from)?
                == 0
            {
                let request: SpaceRemovalRequest = serde_json::from_str(
                    row.try_get("request_json")
                        .map_err(momo_storage::StorageError::from)?,
                )
                .map_err(momo_storage::StorageError::from)?;
                if let SpaceDisposition::Migrate { target_id } = request.disposition {
                    self.retired_spaces.call(move |spaces| {
                        spaces.insert(target_id);
                    });
                }
                let operation = Uuid::parse_str(
                    row.try_get("operation_id")
                        .map_err(momo_storage::StorageError::from)?,
                )
                .map_err(|error| momo_memory::MemoryError::InvalidPatch(error.to_string()))?;
                self.finish_space_removal(operation)
                    .await
                    .map_err(|error| CoreError::Recovery {
                        space_id: source,
                        message: error.to_string(),
                    })?;
            }
        }
        Ok(())
    }

    async fn finish_space_removal(&self, operation: Uuid) -> RuntimeApiResult<()> {
        let row = sqlx::query(
            "SELECT request_json,plan_json,completed FROM space_removals WHERE operation_id=?",
        )
        .bind(operation.to_string())
        .fetch_one(self.store.pool())
        .await
        .map_err(internal)?;
        if row.try_get::<i64, _>("completed").map_err(internal)? != 0 {
            return Ok(());
        }
        let request: SpaceRemovalRequest =
            serde_json::from_str(row.try_get("request_json").map_err(internal)?)
                .map_err(internal)?;
        let plan: RemovalPlan =
            serde_json::from_str(row.try_get("plan_json").map_err(internal)?).map_err(internal)?;
        if let SpaceDisposition::Migrate { target_id } = request.disposition {
            let snapshot = plan
                .target
                .ok_or_else(|| internal("missing target after-image"))?;
            self.memory_for_space_unchecked(target_id)
                .map_err(internal)?
                .call(move |workspace| workspace.import_snapshot(&snapshot))
                .map_err(internal)?;
            self.vector_store
                .remove_nsg_vectors(target_id, None)
                .await
                .map_err(internal)?;
        }
        if let Some(settings) = plan.settings {
            crate::runtime::settings::SettingsStore::apply_removal_image(
                &self.data_dir.join("runtime-settings.json"),
                &settings,
            )
            .map_err(internal)?;
        }
        self.memory_for_space_unchecked(request.source_id)
            .map_err(internal)?
            .call(|workspace| {
                workspace.import_snapshot(&MemorySnapshot {
                    version: 1,
                    files: BTreeMap::new(),
                })
            })
            .map_err(internal)?;
        self.vector_store
            .remove_nsg_vectors(request.source_id, None)
            .await
            .map_err(internal)?;
        self.finish_space_sql(&request).await?;
        if let SpaceDisposition::Migrate { target_id } = request.disposition {
            self.retired_spaces.call(move |spaces| {
                spaces.remove(&target_id);
            });
        }
        Ok(())
    }

    async fn finish_space_sql(&self, request: &SpaceRemovalRequest) -> RuntimeApiResult<()> {
        let source = request.source_id.to_string();
        let deleted = self.owned_deleted_items(request.source_id).await?;
        let mut transaction = self.store.pool().begin().await.map_err(internal)?;
        // This acknowledgement is invisible until the SQL transaction commits;
        // it permits the operation's own writes into the fenced destination.
        sqlx::query("UPDATE space_removals SET completed=1 WHERE operation_id=?")
            .bind(request.operation_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(internal)?;
        if let SpaceDisposition::Migrate { target_id } = request.disposition {
            let target = target_id.to_string();
            for statement in [
                "UPDATE character_cards SET scope_id=? WHERE scope_id=?",
                "UPDATE conversations SET scope_id=? WHERE scope_id=?",
                "UPDATE memory_patch_reviews SET scope_id=? WHERE scope_id=?",
                "UPDATE maintenance_turns SET scope_id=? WHERE scope_id=?",
                "UPDATE maintenance_batches SET scope_id=? WHERE scope_id=?",
            ] {
                sqlx::query(statement)
                    .bind(&target)
                    .bind(&source)
                    .execute(&mut *transaction)
                    .await
                    .map_err(internal)?;
            }
            sqlx::query("UPDATE memory_lifecycle_events SET space_id=? WHERE space_id=?")
                .bind(&target)
                .bind(&source)
                .execute(&mut *transaction)
                .await
                .map_err(internal)?;
            sqlx::query("UPDATE memory_lifecycle_events SET activity_json=json_set(activity_json,'$.identity.personal_space_id',?) WHERE space_id=? AND json_extract(activity_json,'$.identity.personal_space_id')=?")
                .bind(&target).bind(&target).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("UPDATE maintenance_batches SET provenance_json=json_set(provenance_json,'$.identity.personal_space_id',?) WHERE scope_id=? AND json_extract(provenance_json,'$.identity.personal_space_id')=?")
                .bind(&target).bind(&target).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("UPDATE response_evidence SET identity_json=json_set(identity_json,'$.personal_space_id',?) WHERE json_extract(identity_json,'$.personal_space_id')=?")
                .bind(&target).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("UPDATE local_tombstones SET payload=CASE WHEN json_extract(payload,'$.character.owner_space_id')=? THEN json_set(payload,'$.character.owner_space_id',?) ELSE json_set(payload,'$.conversation.space_id',?) END WHERE COALESCE(json_extract(payload,'$.character.owner_space_id'),json_extract(payload,'$.conversation.space_id'))=?")
                .bind(&source).bind(&target).bind(&target).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("INSERT OR IGNORE INTO default_assistants(personal_space_id,character_id) SELECT ?,character_id FROM default_assistants WHERE personal_space_id=?")
                .bind(&target).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
        } else {
            // Stop future distillation; immutable provenance on records owned by
            // other Spaces is deliberately untouched.
            sqlx::query("INSERT INTO memory_evidence_controls(conversation_id,stopped,revoked,updated_at) SELECT id,1,0,? FROM conversations WHERE scope_id=? ON CONFLICT(conversation_id) DO UPDATE SET stopped=1")
                .bind(UtcNow::text()).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("DELETE FROM portable_metadata WHERE object_id IN (SELECT id FROM character_cards WHERE scope_id=?)").bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("DELETE FROM default_assistants WHERE character_id IN (SELECT id FROM character_cards WHERE scope_id=?)").bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("DELETE FROM ddm_projection_states WHERE character_id IN (SELECT id FROM character_cards WHERE scope_id=?)").bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("DELETE FROM memory_identity_bindings WHERE conversation_id IN (SELECT id FROM conversations WHERE scope_id=?)").bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("DELETE FROM response_operations WHERE conversation_id IN (SELECT id FROM conversations WHERE scope_id=?)").bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            sqlx::query("UPDATE response_evidence SET maintenance_stopped=1 WHERE conversation_id IN (SELECT id FROM conversations WHERE scope_id=?)").bind(&source).execute(&mut *transaction).await.map_err(internal)?;
            for (kind, id, _) in deleted {
                sqlx::query("DELETE FROM portable_metadata WHERE object_id=?")
                    .bind(&id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(internal)?;
                sqlx::query("UPDATE local_tombstones SET payload=NULL,hidden=1 WHERE object_type=? AND object_id=?").bind(kind).bind(id).execute(&mut *transaction).await.map_err(internal)?;
            }
            for kind in ["character", "conversation"] {
                let statement = if kind == "character" {
                    "INSERT OR REPLACE INTO local_tombstones(object_type,object_id,deleted_at,payload,hidden) SELECT 'character',id,?,NULL,1 FROM character_cards WHERE scope_id=?"
                } else {
                    "INSERT OR REPLACE INTO local_tombstones(object_type,object_id,deleted_at,payload,hidden) SELECT 'conversation',id,?,NULL,1 FROM conversations WHERE scope_id=?"
                };
                sqlx::query(statement)
                    .bind(UtcNow::text())
                    .bind(&source)
                    .execute(&mut *transaction)
                    .await
                    .map_err(internal)?;
            }
            for statement in [
                "DELETE FROM conversations WHERE scope_id=?",
                "DELETE FROM character_cards WHERE scope_id=?",
                "DELETE FROM memory_patch_reviews WHERE scope_id=?",
                "DELETE FROM maintenance_turns WHERE scope_id=?",
                "DELETE FROM maintenance_batches WHERE scope_id=?",
            ] {
                sqlx::query(statement)
                    .bind(&source)
                    .execute(&mut *transaction)
                    .await
                    .map_err(internal)?;
            }
            sqlx::query("DELETE FROM memory_lifecycle_events WHERE space_id=?")
                .bind(&source)
                .execute(&mut *transaction)
                .await
                .map_err(internal)?;
        }
        sqlx::query("DELETE FROM default_assistants WHERE personal_space_id=?")
            .bind(&source)
            .execute(&mut *transaction)
            .await
            .map_err(internal)?;
        // MO State and vectors are projections. Invalidate dependencies instead
        // of merging revisions from unrelated worlds or leaving stale grants.
        sqlx::query("DELETE FROM mo_state_spaces WHERE space_id=? OR space_id IN (SELECT managed_space_id FROM mo_state_source_versions WHERE source_space_id=?)")
            .bind(&source).bind(&source).execute(&mut *transaction).await.map_err(internal)?;
        if let SpaceDisposition::Migrate { target_id } = request.disposition {
            sqlx::query("DELETE FROM mo_state_spaces WHERE space_id=?")
                .bind(target_id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(internal)?;
        }
        sqlx::query("UPDATE space_removals SET completed=1,plan_json=NULL WHERE operation_id=?")
            .bind(request.operation_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(internal)?;
        transaction.commit().await.map_err(internal)?;
        Ok(())
    }
}

struct UtcNow;
impl UtcNow {
    fn text() -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

fn merge_snapshots(
    source: MemorySnapshot,
    mut target: MemorySnapshot,
    from: Uuid,
    to: Uuid,
    operation: Uuid,
    root: &std::path::Path,
) -> RuntimeApiResult<MemorySnapshot> {
    for (path, mut content) in source.files {
        if path == "indexes/memory_activity.yaml" {
            let mut activity: Value = yaml_serde::from_str(&content).map_err(internal)?;
            rebind_activity(&mut activity, &format!("{from}:"), &format!("{to}:"))?;
            content = yaml_serde::to_string(&activity).map_err(internal)?;
        }
        if matches!(path.as_str(), DMW_LEDGER | NSG_LEDGER) {
            let mut incoming = ProvenanceLedger::parse(&content).map_err(internal)?;
            let mut existing = target
                .files
                .get(&path)
                .map(|text| ProvenanceLedger::parse(text))
                .transpose()
                .map_err(internal)?
                .unwrap_or_default();
            for (id, mut history) in std::mem::take(&mut incoming.records) {
                if existing.records.contains_key(&id) {
                    return Err(conflict(format!("migration_record_conflict: {id}")));
                }
                if let Some(policy) = &mut history.policy {
                    if policy.local_space_id != from {
                        return Err(conflict("record ownership does not match source Space"));
                    }
                    policy.local_space_id = to;
                    if policy.personal_space_id == from {
                        policy.personal_space_id = to;
                    }
                    policy.revision += 1;
                }
                let revision = history.revisions.len() as u64 + 1;
                let mut context = RevisionContext::manual("space_migration");
                context.operation_id = operation.to_string();
                context
                    .configuration
                    .insert("source_space".into(), from.to_string());
                context
                    .configuration
                    .insert("destination_space".into(), to.to_string());
                history.revisions.push(RecordRevision {
                    revision,
                    parent_revision: (revision > 1).then_some(revision - 1),
                    content_hash: history
                        .revisions
                        .last()
                        .map(|record| record.content_hash.clone())
                        .unwrap_or_default(),
                    context,
                });
                existing.records.insert(id, history);
            }
            for (_, mut scene) in incoming.scenes {
                if scene.identity.personal_space_id == from {
                    scene.identity.personal_space_id = to;
                }
                let key = momo_memory::provenance::scene_key(&scene.identity);
                if existing.scenes.insert(key.clone(), scene).is_some() {
                    return Err(conflict(format!("migration_scene_conflict: {key}")));
                }
            }
            target
                .files
                .insert(path, serde_json::to_string(&existing).map_err(internal)?);
            continue;
        }
        if path == "audit/memory.log" {
            target.files.entry(path).or_default().push_str(&content);
            continue;
        }
        if let Some(previous) = target.files.get(&path) {
            if previous == &content {
                continue;
            }
            // Initial current documents contain creation timestamps. Empty,
            // identical bodies can retain the destination bootstrap timestamp.
            if path.starts_with("current/") {
                let left = momo_memory::MemoryDocument::parse(previous).map_err(internal)?;
                let right = momo_memory::MemoryDocument::parse(&content).map_err(internal)?;
                let mut left_metadata = left.metadata.clone();
                left_metadata.touch_at = right.metadata.touch_at;
                if left.body == right.body
                    && serde_json::to_value(left_metadata).map_err(internal)?
                        == serde_json::to_value(&right.metadata).map_err(internal)?
                {
                    continue;
                }
            }
            // Indexes/activity/tombstones are semantic maps: merge only disjoint
            // or identical keys. Never replace the target's retrieval history.
            if path.starts_with("indexes/") || path == "tombstones/forgotten.yaml" {
                let mut left: Value = yaml_serde::from_str(previous).map_err(internal)?;
                let right: Value = yaml_serde::from_str(&content).map_err(internal)?;
                merge_maps(&mut left, right, &path)?;
                target
                    .files
                    .insert(path, yaml_serde::to_string(&left).map_err(internal)?);
                continue;
            }
            return Err(conflict(format!("migration_file_conflict: {path}")));
        }
        target.files.insert(path, content);
    }
    momo_memory::provenance::validate_snapshot_provenance(&target).map_err(internal)?;
    // Validate the complete result before durable intent, including duplicate
    // record IDs hidden behind different filenames.
    let staging = tempfile::Builder::new()
        .prefix(".space-removal-")
        .tempdir_in(root)
        .map_err(internal)?;
    let workspace = momo_memory::MemoryWorkspace::initialize(staging.path()).map_err(internal)?;
    workspace.import_snapshot(&target).map_err(internal)?;
    workspace.rebuild_index().map_err(internal)?;
    workspace.list_documents().map_err(internal)?;
    momo_memory::nsg::NsgWorkspace::initialize(workspace.root())
        .map_err(internal)?
        .list_nodes(true)
        .map_err(internal)?;
    Ok(target)
}

pub(crate) fn merge_maps(left: &mut Value, right: Value, path: &str) -> RuntimeApiResult<()> {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            for (key, value) in right {
                if let Some(previous) = left.get_mut(&key) {
                    merge_maps(previous, value, path)?;
                } else {
                    left.insert(key, value);
                }
            }
        }
        (left, right) if *left == right => {}
        _ => return Err(conflict(format!("migration_metadata_conflict: {path}"))),
    }
    Ok(())
}

fn rebind_activity(value: &mut Value, from: &str, to: &str) -> RuntimeApiResult<()> {
    if let Value::Object(fields) = value {
        let mut result = serde_json::Map::new();
        for (key, mut value) in std::mem::take(fields) {
            rebind_activity(&mut value, from, to)?;
            let key = key
                .strip_prefix(from)
                .map_or_else(|| key.clone(), |suffix| format!("{to}{suffix}"));
            if result.insert(key, value).is_some() {
                return Err(conflict("migration activity identity collision"));
            }
        }
        *fields = result;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/space_lifecycle.rs"]
mod tests;
