use super::*;
use sha2::{Digest, Sha256};

impl LocalStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        Self::connect(options).await
    }

    pub async fn in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?
            .foreign_keys(true)
            .shared_cache(true);
        Self::connect(options).await
    }

    async fn connect(options: SqliteConnectOptions) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        // Control actions are idempotent state transitions, but they span
        // SQLite and file-backed memory. A process can therefore stop after
        // claiming an action and before recording its response. Only
        // completed responses are durable replay records; an unfinished claim
        // is released when the single owner of this database starts again.
        sqlx::query("DELETE FROM control_operations WHERE response_json IS NULL")
            .execute(&pool)
            .await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn save_character(&self, card: &CharacterCard) -> Result<(), StorageError> {
        if self.is_tombstoned("character", card.id).await? {
            return Ok(());
        }
        sqlx::query(
            r#"INSERT INTO character_cards
            (id, scope_id, name, version, description, language, tags, author_uid,
             author_display_name, author_name, author_url, character_markdown, user_markdown,
             opening_markdown, created_at, updated_at)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET
              scope_id=excluded.scope_id, name=excluded.name, version=excluded.version,
              author_display_name=excluded.author_display_name,
              author_name=excluded.author_name, author_url=excluded.author_url,
              character_markdown=excluded.character_markdown, user_markdown=excluded.user_markdown,
              opening_markdown=excluded.opening_markdown,
              updated_at=excluded.updated_at"#,
        )
        .bind(card.id.to_string())
        .bind(card.scope_id.to_string())
        .bind(&card.name)
        .bind(&card.version)
        .bind("")
        .bind("")
        .bind("[]")
        .bind("")
        .bind(&card.author_name)
        .bind(&card.author_name)
        .bind(&card.author_url)
        .bind(&card.character_markdown)
        .bind(&card.user_markdown)
        .bind(&card.opening_markdown)
        .bind(card.created_at.to_rfc3339())
        .bind(card.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_characters(&self) -> Result<Vec<CharacterCard>, StorageError> {
        let rows = sqlx::query("SELECT * FROM character_cards ORDER BY updated_at DESC, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(character_from_row).collect()
    }

    pub async fn list_characters_for_scope(
        &self,
        scope_id: Uuid,
    ) -> Result<Vec<CharacterCard>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM character_cards WHERE scope_id=? ORDER BY updated_at DESC, id",
        )
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(character_from_row).collect()
    }

    pub async fn character_by_id(
        &self,
        character_id: Uuid,
    ) -> Result<Option<CharacterCard>, StorageError> {
        let row = sqlx::query("SELECT * FROM character_cards WHERE id=?")
            .bind(character_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(character_from_row).transpose()
    }

    pub async fn character_for_scope(
        &self,
        scope_id: Uuid,
        character_id: Uuid,
    ) -> Result<Option<CharacterCard>, StorageError> {
        let row = sqlx::query("SELECT * FROM character_cards WHERE id=? AND scope_id=?")
            .bind(character_id.to_string())
            .bind(scope_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(character_from_row).transpose()
    }

    pub async fn stage_character(&self, card: &CharacterCard) -> Result<(), StorageError> {
        self.save_character(card).await
    }

    pub async fn stage_character_update(&self, card: &CharacterCard) -> Result<(), StorageError> {
        self.save_character(card).await
    }

    pub async fn stage_character_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("character", "delete_character", id).await
    }

    pub async fn save_conversation(&self, conversation: &Conversation) -> Result<(), StorageError> {
        if self.is_tombstoned("conversation", conversation.id).await? {
            return Ok(());
        }
        sqlx::query(
            r#"INSERT INTO conversations
            (id, scope_id, character_id, title, created_at, updated_at)
            VALUES (?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET scope_id=excluded.scope_id,
              character_id=excluded.character_id, title=excluded.title,
              updated_at=excluded.updated_at"#,
        )
        .bind(conversation.id.to_string())
        .bind(conversation.scope_id.to_string())
        .bind(conversation.character_id.map(|id| id.to_string()))
        .bind(&conversation.title)
        .bind(conversation.created_at.to_rfc3339())
        .bind(conversation.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_conversations(&self) -> Result<Vec<Conversation>, StorageError> {
        let rows = sqlx::query("SELECT * FROM conversations ORDER BY updated_at DESC, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(conversation_from_row).collect()
    }

    pub async fn list_conversations_for_scope(
        &self,
        scope_id: Uuid,
    ) -> Result<Vec<Conversation>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM conversations WHERE scope_id=? ORDER BY updated_at DESC, id",
        )
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(conversation_from_row).collect()
    }

    pub async fn conversation_for_scope(
        &self,
        scope_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<Option<Conversation>, StorageError> {
        let row = sqlx::query("SELECT * FROM conversations WHERE id=? AND scope_id=?")
            .bind(conversation_id.to_string())
            .bind(scope_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(conversation_from_row).transpose()
    }

    pub async fn stage_conversation(
        &self,
        conversation: &Conversation,
    ) -> Result<(), StorageError> {
        self.save_conversation(conversation).await
    }

    pub async fn stage_conversation_update(
        &self,
        conversation: &Conversation,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let character_id: Option<String> =
            sqlx::query_scalar("SELECT character_id FROM conversations WHERE id=? AND scope_id=?")
                .bind(conversation.id.to_string())
                .bind(conversation.scope_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        let mut updated = conversation.clone();
        updated.character_id = character_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()?;
        sqlx::query("UPDATE conversations SET title=?, updated_at=? WHERE id=? AND scope_id=?")
            .bind(&updated.title)
            .bind(updated.updated_at.to_rfc3339())
            .bind(updated.id.to_string())
            .bind(updated.scope_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn stage_conversation_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("conversation", "delete_conversation", id)
            .await
    }

    pub async fn append_message(&self, message: &Message) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        if insert_message_immutable(&mut transaction, message).await? {
            sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                .bind(message.created_at.to_rfc3339())
                .bind(message.conversation_id.to_string())
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn response_operation(
        &self,
        request_id: &str,
    ) -> Result<Option<ResponseOperation>, StorageError> {
        let row = sqlx::query(
            "SELECT request_id, request_fingerprint, conversation_id, user_written, resolved_input_json, response_json \
             FROM response_operations WHERE request_id=?",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ResponseOperation {
                request_id: row.try_get("request_id")?,
                request_fingerprint: row.try_get("request_fingerprint")?,
                conversation_id: row.try_get("conversation_id")?,
                user_written: row.try_get::<i64, _>("user_written")? != 0,
                resolved_input_json: row.try_get("resolved_input_json")?,
                response_json: row.try_get("response_json")?,
            })
        })
        .transpose()
    }

    pub async fn control_operation(
        &self,
        operation_key: &str,
    ) -> Result<Option<ControlOperation>, StorageError> {
        let row = sqlx::query(
            "SELECT operation_key, request_fingerprint, response_json \
             FROM control_operations WHERE operation_key=?",
        )
        .bind(operation_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ControlOperation {
                operation_key: row.try_get("operation_key")?,
                request_fingerprint: row.try_get("request_fingerprint")?,
                response_json: row.try_get("response_json")?,
            })
        })
        .transpose()
    }

    /// Claims a control request. `false` means another attempt already owns
    /// the same operation key; callers must inspect and replay that record.
    pub async fn begin_control_operation(
        &self,
        operation_key: &str,
        request_fingerprint: &str,
    ) -> Result<bool, StorageError> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO control_operations \
             (operation_key, request_fingerprint, created_at, updated_at) \
             VALUES (?, ?, ?, ?) ON CONFLICT(operation_key) DO NOTHING",
        )
        .bind(operation_key)
        .bind(request_fingerprint)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn complete_control_operation(
        &self,
        operation_key: &str,
        response_json: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE control_operations SET response_json=?, updated_at=? WHERE operation_key=?",
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(operation_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn abandon_control_operation(&self, operation_key: &str) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM control_operations WHERE operation_key=? AND response_json IS NULL",
        )
        .bind(operation_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn begin_response_operation(
        &self,
        request_id: &str,
        request_fingerprint: &str,
        conversation_id: &str,
        resolved_input_json: &str,
    ) -> Result<(), StorageError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO response_operations \
             (request_id, request_fingerprint, conversation_id, user_written, resolved_input_json, created_at, updated_at) \
             VALUES (?, ?, ?, 0, ?, ?, ?) ON CONFLICT(request_id) DO UPDATE SET \
             resolved_input_json=COALESCE(response_operations.resolved_input_json, excluded.resolved_input_json) \
             WHERE response_operations.request_fingerprint=excluded.request_fingerprint \
             AND response_operations.conversation_id=excluded.conversation_id",
        )
        .bind(request_id)
        .bind(request_fingerprint)
        .bind(conversation_id)
        .bind(resolved_input_json)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_response_user_written(&self, request_id: &str) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE response_operations SET user_written=1, updated_at=? WHERE request_id=?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Atomically appends the user message for a response operation and marks
    /// that phase complete. A retry after any committed transaction observes
    /// `user_written=1`; a crash before commit observes neither change.
    pub async fn append_response_user_message(
        &self,
        request_id: &str,
        conversation_scope_id: Uuid,
        message: &Message,
    ) -> Result<bool, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT conversation_id, user_written FROM response_operations WHERE request_id=?",
        )
        .bind(request_id)
        .fetch_one(&mut *transaction)
        .await?;
        let conversation_id: String = row.try_get("conversation_id")?;
        if conversation_id != message.conversation_id.to_string() {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response operation conversation does not match user message".to_owned(),
            )));
        }
        let owned: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id=? AND scope_id=?")
                .bind(message.conversation_id.to_string())
                .bind(conversation_scope_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        if owned != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response conversation does not belong to conversation scope".to_owned(),
            )));
        }
        if row.try_get::<i64, _>("user_written")? != 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        if insert_message_immutable(&mut transaction, message).await? {
            sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                .bind(message.created_at.to_rfc3339())
                .bind(message.conversation_id.to_string())
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query(
            "UPDATE response_operations SET user_written=1, updated_at=? WHERE request_id=?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn complete_response_operation(
        &self,
        request_id: &str,
        response_json: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE response_operations SET response_json=?, updated_at=? WHERE request_id=?",
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Atomically commits every durable effect of a completed model response.
    ///
    /// A process exit can therefore leave either the pre-completion state or
    /// the complete assistant message, maintenance turn, and replay record;
    /// it cannot expose only a subset of those effects.
    pub async fn commit_response_completion(
        &self,
        completion: ResponseCompletion<'_>,
    ) -> Result<bool, StorageError> {
        let ResponseCompletion {
            request_id,
            conversation_scope_id,
            assistant_message,
            maintenance_turn,
            memory_enabled,
            nsg_enabled,
            mo_state_operation_id,
            response_json,
        } = completion;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT conversation_id, response_json FROM response_operations WHERE request_id=?",
        )
        .bind(request_id)
        .fetch_one(&mut *transaction)
        .await?;
        let conversation_id: String = row.try_get("conversation_id")?;
        if row.try_get::<Option<String>, _>("response_json")?.is_some() {
            transaction.rollback().await?;
            return Ok(false);
        }
        let owned: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id=? AND scope_id=?")
                .bind(&conversation_id)
                .bind(conversation_scope_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        if owned != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response conversation does not belong to conversation scope".to_owned(),
            )));
        }
        if let Some(message) = assistant_message {
            if message.conversation_id.to_string() != conversation_id
                || message.role != MessageRole::Assistant
            {
                return Err(StorageError::Database(sqlx::Error::Protocol(
                    "response completion contains an invalid assistant message".to_owned(),
                )));
            }
            if insert_message_immutable(&mut transaction, message).await? {
                sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                    .bind(message.created_at.to_rfc3339())
                    .bind(&conversation_id)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        if let Some(turn) = maintenance_turn {
            if turn.request_id != request_id {
                return Err(StorageError::Database(sqlx::Error::Protocol(
                    "maintenance turn does not match response operation".to_owned(),
                )));
            }
            sqlx::query(
                "INSERT INTO maintenance_turns \
                 (request_id, scope_id, user_content, assistant_content, memory_done, nsg_done, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(request_id) DO NOTHING",
            )
            .bind(&turn.request_id)
            .bind(&turn.scope_id)
            .bind(&turn.user_content)
            .bind(&turn.assistant_content)
            .bind(i64::from(!memory_enabled))
            .bind(i64::from(!nsg_enabled))
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *transaction)
            .await?;
        }
        let changed = sqlx::query(
            "UPDATE response_operations SET response_json=?, updated_at=? \
             WHERE request_id=? AND response_json IS NULL",
        )
        .bind(response_json)
        .bind(Utc::now().to_rfc3339())
        .bind(request_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "response completion record changed concurrently".to_owned(),
            )));
        }
        if let Some(operation_id) = mo_state_operation_id {
            let changed = sqlx::query(
                "UPDATE mo_state_operations SET phase='completed', error=NULL, updated_at=? \
                 WHERE operation_id=? AND phase IN ('projected', 'completed')",
            )
            .bind(Utc::now().to_rfc3339())
            .bind(operation_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if changed != 1 {
                return Err(StorageError::Database(sqlx::Error::Protocol(
                    "response completion does not have a projected MO State operation".to_owned(),
                )));
            }
        }
        transaction.commit().await?;
        Ok(true)
    }

    /// Observes authoritative DMW/NSG source identities and stages exactly one
    /// per-request MO State operation. Retrying an existing operation returns
    /// its original base revisions and projected snapshot.
    pub async fn observe_mo_state_operation(
        &self,
        observation: &MoStateObservation,
    ) -> Result<MoStateOperation, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT operation_id, space_id, event_type, event_fingerprint, phase, \
             base_dmw_revision, base_nsg_revision, base_scene_revision, snapshot_json, error \
             FROM mo_state_operations WHERE operation_id=?",
        )
        .bind(&observation.operation_id)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let operation = mo_state_operation_from_row(&row)?;
            if operation.space_id != observation.space_id
                || operation.event_type != observation.event_type
                || operation.event_fingerprint != observation.event_fingerprint
            {
                return Err(StorageError::MoStateOperationConflict(
                    "operation ID was reused with different state-event content".to_owned(),
                ));
            }
            transaction.rollback().await?;
            return Ok(operation);
        }

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO mo_state_spaces \
             (space_id, profile, created_at, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(space_id) DO NOTHING",
        )
        .bind(&observation.space_id)
        .bind(&observation.profile)
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query(
            "SELECT dmw_revision, nsg_revision, scene_revision, dmw_fingerprint, \
             nsg_fingerprint, scene_fingerprint FROM mo_state_spaces WHERE space_id=?",
        )
        .bind(&observation.space_id)
        .fetch_one(&mut *transaction)
        .await?;
        let dmw_revision = next_revision(
            row.try_get("dmw_revision")?,
            row.try_get("dmw_fingerprint")?,
            &observation.dmw_fingerprint,
        );
        let nsg_revision = next_revision(
            row.try_get("nsg_revision")?,
            row.try_get("nsg_fingerprint")?,
            &observation.nsg_fingerprint,
        );
        let scene_revision = next_revision(
            row.try_get("scene_revision")?,
            row.try_get("scene_fingerprint")?,
            &observation.scene_fingerprint,
        );
        sqlx::query(
            "UPDATE mo_state_spaces SET profile=?, dmw_revision=?, nsg_revision=?, \
             scene_revision=?, dmw_fingerprint=?, nsg_fingerprint=?, scene_fingerprint=?, \
             scene_json=?, updated_at=? WHERE space_id=?",
        )
        .bind(&observation.profile)
        .bind(dmw_revision)
        .bind(nsg_revision)
        .bind(scene_revision)
        .bind(&observation.dmw_fingerprint)
        .bind(&observation.nsg_fingerprint)
        .bind(&observation.scene_fingerprint)
        .bind(&observation.scene_json)
        .bind(&now)
        .bind(&observation.space_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO mo_state_operations \
             (operation_id, space_id, event_type, event_fingerprint, phase, \
              base_dmw_revision, base_nsg_revision, base_scene_revision, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'applying', ?, ?, ?, ?, ?)",
        )
        .bind(&observation.operation_id)
        .bind(&observation.space_id)
        .bind(&observation.event_type)
        .bind(&observation.event_fingerprint)
        .bind(dmw_revision)
        .bind(nsg_revision)
        .bind(scene_revision)
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(MoStateOperation {
            operation_id: observation.operation_id.clone(),
            space_id: observation.space_id.clone(),
            event_type: observation.event_type.clone(),
            event_fingerprint: observation.event_fingerprint.clone(),
            phase: "applying".to_owned(),
            base_dmw_revision: to_revision(dmw_revision)?,
            base_nsg_revision: to_revision(nsg_revision)?,
            base_scene_revision: to_revision(scene_revision)?,
            snapshot_json: None,
            error: None,
        })
    }

    /// Publishes one immutable projection for an operation. A retry returns the
    /// previously published value and never advances the snapshot revision.
    pub async fn publish_mo_state_snapshot(
        &self,
        operation_id: &str,
        state_result_json: &str,
        degraded: bool,
        error: Option<&str>,
    ) -> Result<MoStateSnapshot, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let operation_row = sqlx::query(
            "SELECT operation_id, space_id, event_type, event_fingerprint, phase, \
             base_dmw_revision, base_nsg_revision, base_scene_revision, snapshot_json, error \
             FROM mo_state_operations WHERE operation_id=?",
        )
        .bind(operation_id)
        .fetch_one(&mut *transaction)
        .await?;
        let operation = mo_state_operation_from_row(&operation_row)?;
        if let Some(snapshot) = operation.snapshot_json {
            transaction.rollback().await?;
            return serde_json::from_str(&snapshot).map_err(Into::into);
        }
        let state_result: serde_json::Value = serde_json::from_str(state_result_json)?;
        let space_row = sqlx::query(
            "SELECT profile, dmw_revision, nsg_revision, scene_revision, snapshot_revision, \
             dmw_fingerprint, nsg_fingerprint, scene_fingerprint, scene_json \
             FROM mo_state_spaces WHERE space_id=?",
        )
        .bind(&operation.space_id)
        .fetch_one(&mut *transaction)
        .await?;
        let snapshot_revision: i64 = space_row
            .try_get::<i64, _>("snapshot_revision")?
            .checked_add(1)
            .ok_or_else(|| {
                StorageError::MoStateOperationConflict("snapshot revision overflow".to_owned())
            })?;
        let created_at = Utc::now();
        let scene_json: String = space_row.try_get("scene_json")?;
        let mut identity = Sha256::new();
        identity.update(operation_id.as_bytes());
        identity.update(snapshot_revision.to_le_bytes());
        let snapshot = MoStateSnapshot {
            snapshot_id: format!("mos_{}", hex::encode(identity.finalize())),
            space_id: operation.space_id.clone(),
            profile: space_row.try_get("profile")?,
            dmw_revision: to_revision(space_row.try_get("dmw_revision")?)?,
            nsg_revision: to_revision(space_row.try_get("nsg_revision")?)?,
            scene_revision: to_revision(space_row.try_get("scene_revision")?)?,
            snapshot_revision: to_revision(snapshot_revision)?,
            dmw_fingerprint: space_row.try_get("dmw_fingerprint")?,
            nsg_fingerprint: space_row.try_get("nsg_fingerprint")?,
            scene_fingerprint: space_row.try_get("scene_fingerprint")?,
            scene: serde_json::from_str(&scene_json)?,
            state_context: state_result
                .get("context")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            state_audit: state_result.get("audit").cloned().unwrap_or_default(),
            degraded,
            created_at,
        };
        let snapshot_json = serde_json::to_string(&snapshot)?;
        let now = created_at.to_rfc3339();
        let changed = sqlx::query(
            "UPDATE mo_state_operations SET phase='projected', snapshot_json=?, error=?, updated_at=? \
             WHERE operation_id=? AND snapshot_json IS NULL AND phase IN ('applying', 'failed')",
        )
        .bind(&snapshot_json)
        .bind(error)
        .bind(&now)
        .bind(operation_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(StorageError::MoStateOperationConflict(
                "operation cannot publish a second state snapshot".to_owned(),
            ));
        }
        sqlx::query(
            "UPDATE mo_state_spaces SET snapshot_revision=?, current_snapshot_json=?, \
             degraded=?, last_error=?, updated_at=? WHERE space_id=?",
        )
        .bind(snapshot_revision)
        .bind(&snapshot_json)
        .bind(i64::from(degraded))
        .bind(error)
        .bind(&now)
        .bind(&operation.space_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(snapshot)
    }

    pub async fn fail_mo_state_operation(
        &self,
        operation_id: &str,
        error: &str,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let space_id: Option<String> =
            sqlx::query_scalar("SELECT space_id FROM mo_state_operations WHERE operation_id=?")
                .bind(operation_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if let Some(space_id) = space_id {
            let now = Utc::now().to_rfc3339();
            sqlx::query(
                "UPDATE mo_state_operations SET phase='failed', error=?, updated_at=? \
                 WHERE operation_id=? AND phase != 'completed'",
            )
            .bind(error)
            .bind(&now)
            .bind(operation_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE mo_state_spaces SET degraded=1, last_error=?, updated_at=? WHERE space_id=?",
            )
            .bind(error)
            .bind(&now)
            .bind(space_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn mo_state_runtime_status(
        &self,
        space_id: &str,
    ) -> Result<Option<MoStateRuntimeStatus>, StorageError> {
        let row = sqlx::query(
            "SELECT space_id, profile, dmw_revision, nsg_revision, scene_revision, \
             snapshot_revision, degraded, last_error, current_snapshot_json \
             FROM mo_state_spaces WHERE space_id=?",
        )
        .bind(space_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let snapshot: Option<String> = row.try_get("current_snapshot_json")?;
            Ok(MoStateRuntimeStatus {
                space_id: row.try_get("space_id")?,
                profile: row.try_get("profile")?,
                dmw_revision: to_revision(row.try_get("dmw_revision")?)?,
                nsg_revision: to_revision(row.try_get("nsg_revision")?)?,
                scene_revision: to_revision(row.try_get("scene_revision")?)?,
                snapshot_revision: to_revision(row.try_get("snapshot_revision")?)?,
                degraded: row.try_get::<i64, _>("degraded")? != 0,
                last_error: row.try_get("last_error")?,
                current_snapshot: snapshot.as_deref().map(serde_json::from_str).transpose()?,
            })
        })
        .transpose()
    }

    pub async fn append_maintenance_turn(
        &self,
        turn: &MaintenanceTurn,
        memory_enabled: bool,
        nsg_enabled: bool,
    ) -> Result<(), StorageError> {
        let result = sqlx::query(
            "INSERT INTO maintenance_turns \
             (request_id, scope_id, user_content, assistant_content, memory_done, nsg_done, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(request_id) DO NOTHING",
        )
        .bind(&turn.request_id)
        .bind(&turn.scope_id)
        .bind(&turn.user_content)
        .bind(&turn.assistant_content)
        .bind(i64::from(!memory_enabled))
        .bind(i64::from(!nsg_enabled))
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            let existing = sqlx::query(
                "SELECT scope_id, user_content, assistant_content FROM maintenance_turns \
                 WHERE request_id=?",
            )
            .bind(&turn.request_id)
            .fetch_one(&self.pool)
            .await?;
            let same = existing.try_get::<String, _>("scope_id")? == turn.scope_id
                && existing.try_get::<String, _>("user_content")? == turn.user_content
                && existing.try_get::<String, _>("assistant_content")? == turn.assistant_content;
            if !same {
                return Err(StorageError::MaintenanceTurnConflict(
                    turn.request_id.clone(),
                ));
            }
        }
        Ok(())
    }

    pub async fn pending_maintenance_turns(
        &self,
        scope_id: &str,
        kind: MaintenanceKind,
        limit: usize,
    ) -> Result<Vec<MaintenanceTurn>, StorageError> {
        let query = match kind {
            MaintenanceKind::Memory => {
                "SELECT request_id, scope_id, user_content, assistant_content \
                 FROM maintenance_turns WHERE scope_id=? AND memory_done=0 \
                 ORDER BY created_at, request_id LIMIT ?"
            }
            MaintenanceKind::SemanticGraph => {
                "SELECT request_id, scope_id, user_content, assistant_content \
                 FROM maintenance_turns WHERE scope_id=? AND nsg_done=0 \
                 ORDER BY created_at, request_id LIMIT ?"
            }
        };
        let rows = sqlx::query(query)
            .bind(scope_id)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(MaintenanceTurn {
                    request_id: row.try_get("request_id")?,
                    scope_id: row.try_get("scope_id")?,
                    user_content: row.try_get("user_content")?,
                    assistant_content: row.try_get("assistant_content")?,
                })
            })
            .collect()
    }

    pub async fn mark_maintenance_turns_done(
        &self,
        request_ids: &[String],
        kind: MaintenanceKind,
    ) -> Result<(), StorageError> {
        if request_ids.is_empty() {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await?;
        let query = match kind {
            MaintenanceKind::Memory => {
                "UPDATE maintenance_turns SET memory_done=1 WHERE request_id=?"
            }
            MaintenanceKind::SemanticGraph => {
                "UPDATE maintenance_turns SET nsg_done=1 WHERE request_id=?"
            }
        };
        for request_id in request_ids {
            sqlx::query(query)
                .bind(request_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Persists a generated patch before any file mutation. If a previous
    /// attempt crashed, the original patch is returned and must be reused.
    pub async fn maintenance_batch_patch(
        &self,
        batch_key: &str,
    ) -> Result<Option<String>, StorageError> {
        sqlx::query_scalar("SELECT patch_yaml FROM maintenance_batches WHERE batch_key=?")
            .bind(batch_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Removes a staged patch without acknowledging its source turns. This is
    /// used only when validation proves that a persisted model response can
    /// never be applied; the pending turns remain available for regeneration.
    pub async fn discard_maintenance_batch(&self, batch_key: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM maintenance_batches WHERE batch_key=?")
            .bind(batch_key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn stage_maintenance_batch(
        &self,
        batch: &MaintenanceBatch,
    ) -> Result<String, StorageError> {
        let request_ids_json = serde_json::to_string(&batch.request_ids)
            .map_err(|error| StorageError::Database(sqlx::Error::Protocol(error.to_string())))?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO maintenance_batches \
             (batch_key, scope_id, kind, request_ids_json, patch_yaml, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(batch_key) DO NOTHING",
        )
        .bind(&batch.batch_key)
        .bind(&batch.scope_id)
        .bind(&batch.kind)
        .bind(&request_ids_json)
        .bind(&batch.patch_yaml)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query(
            "SELECT scope_id, kind, request_ids_json, patch_yaml \
             FROM maintenance_batches WHERE batch_key=?",
        )
        .bind(&batch.batch_key)
        .fetch_one(&self.pool)
        .await?;
        let stored_scope: String = row.try_get("scope_id")?;
        let stored_kind: String = row.try_get("kind")?;
        let stored_ids: String = row.try_get("request_ids_json")?;
        if stored_scope != batch.scope_id
            || stored_kind != batch.kind
            || stored_ids != request_ids_json
        {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "maintenance batch key was reused with different inputs".to_owned(),
            )));
        }
        row.try_get("patch_yaml").map_err(Into::into)
    }

    /// Atomically acknowledges the source turns and removes the durable patch.
    pub async fn complete_maintenance_batch(
        &self,
        batch_key: &str,
        request_ids: &[String],
        kind: MaintenanceKind,
    ) -> Result<(), StorageError> {
        if request_ids.is_empty() {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await?;
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM maintenance_batches WHERE batch_key=?")
                .bind(batch_key)
                .fetch_one(&mut *transaction)
                .await?;
        if exists != 1 {
            return Err(StorageError::Database(sqlx::Error::Protocol(
                "maintenance batch does not exist".to_owned(),
            )));
        }
        let query = match kind {
            MaintenanceKind::Memory => {
                "UPDATE maintenance_turns SET memory_done=1 WHERE request_id=?"
            }
            MaintenanceKind::SemanticGraph => {
                "UPDATE maintenance_turns SET nsg_done=1 WHERE request_id=?"
            }
        };
        for request_id in request_ids {
            sqlx::query(query)
                .bind(request_id)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query("DELETE FROM maintenance_batches WHERE batch_key=?")
            .bind(batch_key)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn clear_space_memory_state(
        &self,
        space_id: Uuid,
        memory: bool,
        semantic_graph: bool,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let space_id = space_id.to_string();
        if memory {
            sqlx::query("DELETE FROM memory_patch_reviews WHERE scope_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
        }
        if memory || semantic_graph {
            sqlx::query("DELETE FROM mo_state_operations WHERE space_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM mo_state_spaces WHERE space_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
        }
        if memory && semantic_graph {
            sqlx::query("DELETE FROM maintenance_turns WHERE scope_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM maintenance_batches WHERE scope_id=?")
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
        } else {
            if memory {
                sqlx::query("UPDATE maintenance_turns SET memory_done=1 WHERE scope_id=?")
                    .bind(&space_id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query("DELETE FROM maintenance_batches WHERE scope_id=? AND kind='memory'")
                    .bind(&space_id)
                    .execute(&mut *transaction)
                    .await?;
            }
            if semantic_graph {
                sqlx::query("UPDATE maintenance_turns SET nsg_done=1 WHERE scope_id=?")
                    .bind(&space_id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query(
                    "DELETE FROM maintenance_batches WHERE scope_id=? AND kind='semantic_graph'",
                )
                .bind(&space_id)
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                "DELETE FROM maintenance_turns WHERE scope_id=? AND memory_done=1 AND nsg_done=1",
            )
            .bind(&space_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn stage_message(&self, message: &Message) -> Result<(), StorageError> {
        self.append_message(message).await
    }

    pub async fn save_message(&self, message: &Message) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let existing = sqlx::query("SELECT * FROM messages WHERE id=?")
            .bind(message.id.to_string())
            .fetch_optional(&mut *transaction)
            .await?;
        match existing {
            Some(row) => {
                let persisted = message_from_row(&row)?;
                if persisted.conversation_id != message.conversation_id
                    || persisted.role != message.role
                    || persisted.created_at != message.created_at
                {
                    return Err(StorageError::ImmutableMessageConflict(message.id));
                }
                sqlx::query("UPDATE messages SET content=? WHERE id=?")
                    .bind(&message.content)
                    .bind(message.id.to_string())
                    .execute(&mut *transaction)
                    .await?;
            }
            None => {
                insert_message_immutable(&mut transaction, message).await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn message_by_id(&self, id: Uuid) -> Result<Option<Message>, StorageError> {
        let row = sqlx::query("SELECT * FROM messages WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(message_from_row).transpose()
    }

    pub async fn replace_message(&self, message: &Message) -> Result<(), StorageError> {
        sqlx::query(
            r#"INSERT INTO messages (id, conversation_id, role, content, created_at)
            VALUES (?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET
              conversation_id=excluded.conversation_id,
              role=excluded.role,
              content=excluded.content,
              created_at=excluded.created_at"#,
        )
        .bind(message.id.to_string())
        .bind(message.conversation_id.to_string())
        .bind(message.role.as_str())
        .bind(&message.content)
        .bind(message.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn stage_message_update(&self, message: &Message) -> Result<(), StorageError> {
        self.save_message(message).await
    }

    pub async fn stage_message_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("message", "delete_message", id).await
    }

    pub async fn list_messages(&self, conversation_id: Uuid) -> Result<Vec<Message>, StorageError> {
        let rows =
            sqlx::query("SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at, id")
                .bind(conversation_id.to_string())
                .fetch_all(&self.pool)
                .await?;
        rows.iter().map(message_from_row).collect()
    }

    pub async fn list_messages_for_scope(
        &self,
        scope_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<Vec<Message>, StorageError> {
        let rows = sqlx::query(
            r#"SELECT messages.* FROM messages
            INNER JOIN conversations ON conversations.id = messages.conversation_id
            WHERE messages.conversation_id=? AND conversations.scope_id=?
            ORDER BY messages.created_at, messages.id"#,
        )
        .bind(conversation_id.to_string())
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(message_from_row).collect()
    }

    pub async fn message_for_scope(
        &self,
        scope_id: Uuid,
        message_id: Uuid,
    ) -> Result<Option<Message>, StorageError> {
        let row = sqlx::query(
            r#"SELECT messages.* FROM messages
            INNER JOIN conversations ON conversations.id = messages.conversation_id
            WHERE messages.id=? AND conversations.scope_id=?"#,
        )
        .bind(message_id.to_string())
        .bind(scope_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(message_from_row).transpose()
    }

    pub async fn tombstone_ids(&self, object_type: &str) -> Result<Vec<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT object_id FROM local_tombstones WHERE object_type=? ORDER BY object_id",
        )
        .bind(object_type)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn recently_deleted(
        &self,
        limit: u32,
    ) -> Result<Vec<RecentlyDeletedItem>, StorageError> {
        let rows = sqlx::query(
            "SELECT object_type, object_id, deleted_at, payload FROM local_tombstones WHERE hidden=0 ORDER BY deleted_at DESC, object_type, object_id LIMIT ?",
        )
        .bind(i64::from(limit.min(1_000)))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(recently_deleted_from_row).collect()
    }

    pub async fn restore_recently_deleted(
        &self,
        object_type: &str,
        object_id: Uuid,
    ) -> Result<bool, StorageError> {
        let payload: Option<String> = sqlx::query_scalar(
            "SELECT payload FROM local_tombstones WHERE object_type=? AND object_id=?",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        let Some(payload) = payload else {
            return Ok(false);
        };

        let mut transaction = self.pool.begin().await?;
        match object_type {
            "character" => {
                let snapshot: DeletedCharacterSnapshot = serde_json::from_str(&payload)?;
                let card = snapshot.character;
                sqlx::query(
                    r#"INSERT INTO character_cards
                    (id, scope_id, name, version, description, language, tags, author_uid,
                     author_display_name, author_name, author_url, character_markdown, user_markdown,
                     opening_markdown, created_at, updated_at)
                    VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(id) DO UPDATE SET scope_id=excluded.scope_id,
                      name=excluded.name, version=excluded.version,
                      author_display_name=excluded.author_display_name,
                      author_name=excluded.author_name, author_url=excluded.author_url,
                      character_markdown=excluded.character_markdown,
                      user_markdown=excluded.user_markdown,
                      opening_markdown=excluded.opening_markdown,
                      created_at=excluded.created_at, updated_at=excluded.updated_at"#,
                )
                .bind(card.id.to_string())
                .bind(card.scope_id.to_string())
                .bind(&card.name)
                .bind(&card.version)
                .bind("")
                .bind("")
                .bind("[]")
                .bind("")
                .bind(&card.author_name)
                .bind(&card.author_name)
                .bind(&card.author_url)
                .bind(&card.character_markdown)
                .bind(&card.user_markdown)
                .bind(&card.opening_markdown)
                .bind(card.created_at.to_rfc3339())
                .bind(card.updated_at.to_rfc3339())
                .execute(&mut *transaction)
                .await?;
                for conversation_id in snapshot.conversation_ids {
                    sqlx::query(
                        "UPDATE conversations SET character_id=? WHERE id=? AND character_id IS NULL",
                    )
                    .bind(card.id.to_string())
                    .bind(conversation_id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                }
            }
            "conversation" => {
                let snapshot: DeletedConversationSnapshot = serde_json::from_str(&payload)?;
                let conversation = snapshot.conversation;
                sqlx::query(
                    r#"INSERT INTO conversations
                    (id, scope_id, character_id, title, created_at, updated_at)
                    VALUES (?,?,?,?,?,?)
                    ON CONFLICT(id) DO UPDATE SET scope_id=excluded.scope_id,
                      character_id=excluded.character_id, title=excluded.title,
                      created_at=excluded.created_at, updated_at=excluded.updated_at"#,
                )
                .bind(conversation.id.to_string())
                .bind(conversation.scope_id.to_string())
                .bind(conversation.character_id.map(|id| id.to_string()))
                .bind(&conversation.title)
                .bind(conversation.created_at.to_rfc3339())
                .bind(conversation.updated_at.to_rfc3339())
                .execute(&mut *transaction)
                .await?;
                for message in snapshot.messages {
                    insert_message_immutable(&mut transaction, &message).await?;
                }
            }
            "message" => {
                let message: Message = serde_json::from_str(&payload)?;
                if message.id != object_id {
                    return Ok(false);
                }
                insert_message_immutable(&mut transaction, &message).await?;
            }
            _ => return Ok(false),
        }
        sqlx::query("DELETE FROM local_tombstones WHERE object_type=? AND object_id=?")
            .bind(object_type)
            .bind(object_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn purge_recently_deleted(
        &self,
        object_type: &str,
        object_id: Uuid,
    ) -> Result<bool, StorageError> {
        match object_type {
            "character" | "conversation" | "message" => {}
            _ => return Ok(false),
        }
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT 1 FROM local_tombstones WHERE object_type=? AND object_id=? AND hidden=0",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let _ = row;
        let changed = sqlx::query(
            "UPDATE local_tombstones SET payload=NULL, hidden=1 WHERE object_type=? AND object_id=? AND hidden=0",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            > 0;
        transaction.commit().await?;
        Ok(changed)
    }

    pub async fn forget_recently_deleted(
        &self,
        object_type: &str,
        object_id: Uuid,
    ) -> Result<bool, StorageError> {
        if let Some(actual_type) = object_type.strip_prefix("local_only:") {
            let mut transaction = self.pool.begin().await?;
            let changed = sqlx::query(
                "UPDATE local_tombstones SET payload=NULL, hidden=1 WHERE object_type=? AND object_id=? AND hidden=0",
            )
            .bind(actual_type)
            .bind(object_id.to_string())
            .execute(&mut *transaction)
            .await?
            .rows_affected()
                > 0;
            transaction.commit().await?;
            return Ok(changed);
        }
        Ok(sqlx::query(
            "DELETE FROM local_tombstones WHERE object_type=? AND object_id=? AND hidden=0",
        )
        .bind(object_type)
        .bind(object_id.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn save_portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
        document: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"INSERT INTO portable_metadata (kind, object_id, document) VALUES (?,?,?)
            ON CONFLICT(kind, object_id) DO UPDATE SET document=excluded.document"#,
        )
        .bind(kind)
        .bind(object_id)
        .bind(document)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT document FROM portable_metadata WHERE kind=? AND object_id=?",
        )
        .bind(kind)
        .bind(object_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn create_memory_patch_review(
        &self,
        scope_id: Uuid,
        conversation_id: &str,
        patch_yaml: &str,
        targets: &[String],
        operation_count: i64,
        review_mode: &str,
    ) -> Result<MemoryPatchReview, StorageError> {
        let review = MemoryPatchReview {
            id: Uuid::now_v7(),
            scope_id,
            conversation_id: conversation_id.to_owned(),
            patch_yaml: patch_yaml.to_owned(),
            targets: targets.to_vec(),
            operation_count,
            review_mode: review_mode.to_owned(),
            status: MemoryPatchReviewStatus::Pending,
            created_at: Utc::now(),
            resolved_at: None,
            result: None,
            error: None,
        };
        sqlx::query(
            r#"INSERT INTO memory_patch_reviews
            (id, scope_id, conversation_id, patch_yaml, targets, operation_count,
             review_mode, status, created_at)
            VALUES (?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(review.id.to_string())
        .bind(review.scope_id.to_string())
        .bind(&review.conversation_id)
        .bind(&review.patch_yaml)
        .bind(serde_json::to_string(&review.targets)?)
        .bind(review.operation_count)
        .bind(&review.review_mode)
        .bind(review.status.as_str())
        .bind(review.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(review)
    }

    pub async fn memory_patch_review(
        &self,
        scope_id: Uuid,
        review_id: Uuid,
    ) -> Result<Option<MemoryPatchReview>, StorageError> {
        let row = sqlx::query("SELECT * FROM memory_patch_reviews WHERE scope_id=? AND id=?")
            .bind(scope_id.to_string())
            .bind(review_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(memory_patch_review_from_row).transpose()
    }

    pub async fn list_memory_patch_reviews(
        &self,
        scope_id: Uuid,
        include_resolved: bool,
    ) -> Result<Vec<MemoryPatchReview>, StorageError> {
        let rows = if include_resolved {
            sqlx::query(
                r#"SELECT * FROM memory_patch_reviews
                WHERE scope_id=? ORDER BY created_at DESC, id DESC LIMIT 200"#,
            )
            .bind(scope_id.to_string())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT * FROM memory_patch_reviews
                WHERE scope_id=? AND status='pending'
                ORDER BY created_at DESC, id DESC LIMIT 200"#,
            )
            .bind(scope_id.to_string())
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(memory_patch_review_from_row).collect()
    }

    pub async fn resolve_memory_patch_review(
        &self,
        scope_id: Uuid,
        review_id: Uuid,
        status: MemoryPatchReviewStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<Option<MemoryPatchReview>, StorageError> {
        debug_assert!(status != MemoryPatchReviewStatus::Pending);
        let now = Utc::now().to_rfc3339();
        let updated = sqlx::query(
            r#"UPDATE memory_patch_reviews
            SET status=?, resolved_at=?, result=?, error=?
            WHERE scope_id=? AND id=? AND status='pending'"#,
        )
        .bind(status.as_str())
        .bind(now)
        .bind(result)
        .bind(error)
        .bind(scope_id.to_string())
        .bind(review_id.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.memory_patch_review(scope_id, review_id).await
    }

    pub async fn is_tombstoned(&self, object_type: &str, id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM local_tombstones WHERE object_type=? AND object_id=?",
        )
        .bind(object_type)
        .bind(id.to_string())
        .fetch_one(&self.pool)
        .await?
            > 0)
    }

    async fn stage_delete(
        &self,
        object_type: &str,
        _operation: &str,
        id: Uuid,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let payload = match object_type {
            "character" => {
                let row = sqlx::query("SELECT * FROM character_cards WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let character = row.as_ref().map(character_from_row).transpose()?;
                let conversation_ids = sqlx::query_scalar::<_, String>(
                    "SELECT id FROM conversations WHERE character_id=? ORDER BY id",
                )
                .bind(id.to_string())
                .fetch_all(&mut *transaction)
                .await?
                .into_iter()
                .map(|id| Uuid::parse_str(&id))
                .collect::<Result<Vec<_>, _>>()?;
                let payload = character
                    .map(|character| DeletedCharacterSnapshot {
                        character,
                        conversation_ids,
                    })
                    .map(|snapshot| serde_json::to_string(&snapshot))
                    .transpose()?;
                sqlx::query("DELETE FROM character_cards WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                payload
            }
            "conversation" => {
                let row = sqlx::query("SELECT * FROM conversations WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let conversation = row.as_ref().map(conversation_from_row).transpose()?;
                let message_rows = sqlx::query(
                    "SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at, id",
                )
                .bind(id.to_string())
                .fetch_all(&mut *transaction)
                .await?;
                let messages = message_rows
                    .iter()
                    .map(message_from_row)
                    .collect::<Result<Vec<_>, _>>()?;
                let payload = conversation
                    .map(|conversation| DeletedConversationSnapshot {
                        conversation,
                        messages,
                    })
                    .map(|snapshot| serde_json::to_string(&snapshot))
                    .transpose()?;
                sqlx::query("DELETE FROM conversations WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                payload
            }
            "message" => {
                let row = sqlx::query("SELECT * FROM messages WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let message = row.as_ref().map(message_from_row).transpose()?;
                sqlx::query("DELETE FROM messages WHERE id=?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                message
                    .map(|message| serde_json::to_string(&message))
                    .transpose()?
            }
            _ => unreachable!("stage_delete is private and uses known object types"),
        };
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO local_tombstones (object_type, object_id, deleted_at, payload)
            VALUES (?,?,?,?)
            ON CONFLICT(object_type, object_id) DO UPDATE SET
              deleted_at=excluded.deleted_at,
              payload=COALESCE(excluded.payload, local_tombstones.payload),
              hidden=0"#,
        )
        .bind(object_type)
        .bind(id.to_string())
        .bind(&now)
        .bind(payload)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn next_revision(current: i64, previous_fingerprint: String, fingerprint: &str) -> i64 {
    if previous_fingerprint == fingerprint {
        current
    } else {
        current.saturating_add(1)
    }
}

fn to_revision(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| {
        StorageError::MoStateOperationConflict("persisted revision is negative".to_owned())
    })
}

fn mo_state_operation_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<MoStateOperation, StorageError> {
    Ok(MoStateOperation {
        operation_id: row.try_get("operation_id")?,
        space_id: row.try_get("space_id")?,
        event_type: row.try_get("event_type")?,
        event_fingerprint: row.try_get("event_fingerprint")?,
        phase: row.try_get("phase")?,
        base_dmw_revision: to_revision(row.try_get("base_dmw_revision")?)?,
        base_nsg_revision: to_revision(row.try_get("base_nsg_revision")?)?,
        base_scene_revision: to_revision(row.try_get("base_scene_revision")?)?,
        snapshot_json: row.try_get("snapshot_json")?,
        error: row.try_get("error")?,
    })
}
