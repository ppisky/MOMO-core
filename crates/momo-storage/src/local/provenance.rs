use super::*;
use momo_domain::provenance::{Evidence, MemoryIdentity};

impl LocalStore {
    pub async fn capture_memory_character_profile(
        &self,
        character_id: Uuid,
        revision: &str,
        profile: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO memory_character_profiles(character_id,revision,profile) VALUES(?,?,?) ON CONFLICT(character_id,revision) DO NOTHING")
            .bind(character_id.to_string()).bind(revision).bind(profile).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn memory_character_profile(
        &self,
        character_id: Uuid,
        revision: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT profile FROM memory_character_profiles WHERE character_id=? AND revision=?",
        )
        .bind(character_id.to_string())
        .bind(revision)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn response_evidence(
        &self,
        request_id: &str,
    ) -> Result<Option<(Evidence, MemoryIdentity)>, StorageError> {
        let row = sqlx::query("SELECT evidence_json, identity_json FROM response_evidence WHERE request_id=? AND revoked=0")
            .bind(request_id).fetch_optional(&self.pool).await?;
        row.map(|r| {
            Ok((
                serde_json::from_str(r.try_get("evidence_json")?)?,
                serde_json::from_str(r.try_get("identity_json")?)?,
            ))
        })
        .transpose()
    }

    /// Availability refers to active messages, never the deleted-item snapshot.
    pub async fn evidence_status(&self, evidence: &Evidence) -> Result<&'static str, StorageError> {
        let row =
            sqlx::query("SELECT evidence_json, revoked FROM response_evidence WHERE request_id=?")
                .bind(&evidence.id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Ok("external_unresolved");
        };
        if row.try_get::<i64, _>("revoked")? != 0 {
            return Ok("revoked");
        }
        let original: Evidence = serde_json::from_str(row.try_get("evidence_json")?)?;
        if original != *evidence {
            return Ok("external_unresolved");
        }
        for id in &evidence.message_ids {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM messages WHERE id=? AND conversation_id=?",
            )
            .bind(id.to_string())
            .bind(evidence.conversation_id.map(|v| v.to_string()))
            .fetch_one(&self.pool)
            .await?;
            if exists == 0 {
                return Ok("source_deleted");
            }
        }
        Ok("available")
    }

    pub async fn memory_identity(
        &self,
        personal_space_id: Uuid,
        conversation_id: Uuid,
        character_id: Uuid,
    ) -> Result<MemoryIdentity, StorageError> {
        let row = sqlx::query("SELECT continuity_id, function_id FROM memory_identity_bindings WHERE conversation_id=?")
            .bind(conversation_id.to_string()).fetch_optional(&self.pool).await?;
        let (continuity_id, function_id) = if let Some(r) = row {
            (
                Uuid::parse_str(r.try_get("continuity_id")?)?,
                r.try_get("function_id")?,
            )
        } else {
            (conversation_id, None)
        };
        Ok(MemoryIdentity {
            personal_space_id,
            conversation_id,
            character_id,
            continuity_id,
            function_id,
        })
    }

    pub async fn set_memory_identity_binding(
        &self,
        conversation_id: Uuid,
        continuity_id: Uuid,
        function_id: Option<&str>,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO memory_identity_bindings(conversation_id,continuity_id,function_id) VALUES(?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET continuity_id=excluded.continuity_id,function_id=excluded.function_id")
            .bind(conversation_id.to_string()).bind(continuity_id.to_string()).bind(function_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Stopping extraction and revoking evidence are explicit controls. Neither
    /// is implied by deleting a conversation or erasing its active messages.
    pub async fn control_evidence(
        &self,
        conversation_id: Uuid,
        stop_maintenance: bool,
        revoke: bool,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let prepared: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maintenance_batches b, json_each(b.request_ids_json) j JOIN response_evidence e ON e.request_id=j.value WHERE e.conversation_id=? AND b.prepared_commit_json IS NOT NULL")
            .bind(conversation_id.to_string()).fetch_one(&mut *tx).await?;
        if prepared > 0 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "recover prepared commits before controlling evidence".into(),
            )));
        }
        sqlx::query("INSERT INTO memory_evidence_controls(conversation_id,stopped,revoked,updated_at) VALUES(?,?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET stopped=MAX(stopped,excluded.stopped),revoked=MAX(revoked,excluded.revoked),updated_at=excluded.updated_at")
            .bind(conversation_id.to_string()).bind(i64::from(stop_maintenance)).bind(i64::from(revoke)).bind(Utc::now().to_rfc3339()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO memory_evidence_control_audit(id,conversation_id,stopped,revoked,actor,created_at) VALUES(?,?,?,?,?,?)")
            .bind(Uuid::now_v7().to_string()).bind(conversation_id.to_string()).bind(i64::from(stop_maintenance)).bind(i64::from(revoke)).bind("local_author").bind(Utc::now().to_rfc3339()).execute(&mut *tx).await?;
        sqlx::query("UPDATE response_evidence SET revoked=MAX(revoked,?), maintenance_stopped=MAX(maintenance_stopped,?) WHERE conversation_id=?")
            .bind(i64::from(revoke)).bind(i64::from(stop_maintenance)).bind(conversation_id.to_string()).execute(&mut *tx).await?;
        if stop_maintenance || revoke {
            sqlx::query("DELETE FROM maintenance_batches WHERE batch_key IN (SELECT b.batch_key FROM maintenance_batches b, json_each(b.request_ids_json) j JOIN response_evidence e ON e.request_id=j.value WHERE e.conversation_id=?)")
                .bind(conversation_id.to_string()).execute(&mut *tx).await?;
            sqlx::query("UPDATE maintenance_turns SET memory_done=1,nsg_done=1 WHERE request_id IN (SELECT request_id FROM response_evidence WHERE conversation_id=?)")
                .bind(conversation_id.to_string()).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn evidence_write_spaces(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<Uuid>, StorageError> {
        let spaces: Vec<String> = sqlx::query_scalar("SELECT DISTINCT t.scope_id FROM maintenance_turns t JOIN response_evidence e ON e.request_id=t.request_id WHERE e.conversation_id=?")
            .bind(conversation_id.to_string()).fetch_all(&self.pool).await?;
        spaces
            .into_iter()
            .map(|s| Uuid::parse_str(&s).map_err(StorageError::from))
            .collect()
    }

    pub async fn default_assistant(
        &self,
        personal_space_id: Uuid,
    ) -> Result<Option<Uuid>, StorageError> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT character_id FROM default_assistants WHERE personal_space_id=?",
        )
        .bind(personal_space_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        id.map(|id| Uuid::parse_str(&id).map_err(StorageError::from))
            .transpose()
    }

    pub async fn set_default_assistant(
        &self,
        personal_space_id: Uuid,
        character_id: Uuid,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO default_assistants(personal_space_id,character_id) VALUES(?,?) ON CONFLICT(personal_space_id) DO UPDATE SET character_id=excluded.character_id")
            .bind(personal_space_id.to_string()).bind(character_id.to_string()).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn turn(
        store: &LocalStore,
        space: Uuid,
        conversation: Uuid,
        character: Uuid,
        id: &str,
    ) -> Evidence {
        let now = Utc::now();
        store
            .begin_response_operation(id, id, &conversation.to_string(), "{}")
            .await
            .unwrap();
        let user = Message {
            id: Uuid::now_v7(),
            conversation_id: conversation,
            role: MessageRole::User,
            content: "user fact".into(),
            created_at: now,
        };
        store
            .append_response_user_message(id, space, &user)
            .await
            .unwrap();
        let assistant = Message {
            id: Uuid::now_v7(),
            conversation_id: conversation,
            role: MessageRole::Assistant,
            content: "reply".into(),
            created_at: now,
        };
        let identity = MemoryIdentity {
            personal_space_id: space,
            conversation_id: conversation,
            character_id: character,
            continuity_id: conversation,
            function_id: None,
        };
        let response=serde_json::json!({"momo":{"request_audit":{"memory_identity":identity,"evidence_configuration":{"assistant_revision":"revision-a"}}}}).to_string();
        let maintenance = MaintenanceTurn {
            request_id: id.into(),
            scope_id: space.to_string(),
            user_content: user.content.clone(),
            assistant_content: assistant.content.clone(),
        };
        for expected in [true, false] {
            assert_eq!(
                store
                    .commit_response_completion(ResponseCompletion {
                        lifecycle_activity_json: None,
                        request_id: id,
                        conversation_scope_id: space,
                        assistant_message: Some(&assistant),
                        maintenance_turn: Some(&maintenance),
                        memory_enabled: true,
                        nsg_enabled: true,
                        mo_state_operation_id: None,
                        response_json: &response
                    })
                    .await
                    .unwrap(),
                expected
            );
        }
        let evidence = store.response_evidence(id).await.unwrap().unwrap().0;
        assert_eq!(evidence.message_ids, vec![user.id, assistant.id]);
        evidence
    }

    #[tokio::test]
    async fn evidence_is_atomic_historical_and_deletion_does_not_revoke_derived_memory() {
        let store = LocalStore::in_memory().await.unwrap();
        let space = Uuid::now_v7();
        let conversation = Uuid::now_v7();
        let now = Utc::now();
        store
            .save_conversation(&Conversation {
                id: conversation,
                scope_id: space,
                character_id: None,
                title: "history".into(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let first = turn(&store, space, conversation, a, "first").await;
        let second = turn(&store, space, conversation, b, "second").await;
        assert_eq!(
            store
                .response_evidence("first")
                .await
                .unwrap()
                .unwrap()
                .0
                .character_id,
            Some(a)
        );
        assert_eq!(store.evidence_status(&first).await.unwrap(), "available");
        let batch = store
            .pending_maintenance_turns(&space.to_string(), MaintenanceKind::Memory, 12)
            .await
            .unwrap();
        assert_eq!(
            batch.len(),
            1,
            "different historical characters form separate batches"
        );
        store.stage_conversation_delete(conversation).await.unwrap();
        assert_eq!(
            store.evidence_status(&first).await.unwrap(),
            "source_deleted"
        );
        assert_eq!(
            store
                .pending_maintenance_turns(&space.to_string(), MaintenanceKind::Memory, 12)
                .await
                .unwrap()
                .len(),
            1
        );
        store
            .control_evidence(conversation, true, false)
            .await
            .unwrap();
        assert!(
            store
                .pending_maintenance_turns(&space.to_string(), MaintenanceKind::Memory, 12)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.evidence_status(&second).await.unwrap(),
            "source_deleted"
        );
        store
            .control_evidence(conversation, false, true)
            .await
            .unwrap();
        assert_eq!(store.evidence_status(&first).await.unwrap(), "revoked");
        let attempted = MaintenanceBatch {
            batch_key: "late".into(),
            scope_id: space.to_string(),
            kind: "memory".into(),
            request_ids: vec!["first".into()],
            patch_yaml: "patches: []".into(),
        };
        assert!(store.stage_maintenance_batch(&attempted).await.is_err());
    }
}
