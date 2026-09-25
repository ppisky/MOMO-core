use super::*;
use sha2::{Digest, Sha256};

impl LocalStore {
    /// Observes source identities and preserves the original per-request projection on replay.
    pub async fn observe_mo_state_operation(
        &self,
        observation: &MoStateObservation,
    ) -> Result<MoStateOperation, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT operation_id, space_id, event_type, event_fingerprint, phase, \
             base_dmw_revision, base_nsg_revision, base_scene_revision, source_versions_json, \
             observed_scene_json, snapshot_json, error \
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

        let managed_source = MoStateSourceObservation {
            space_id: observation.space_id.clone(),
            dmw_fingerprint: observation.dmw_fingerprint.clone(),
            nsg_fingerprint: observation.nsg_fingerprint.clone(),
            scene_fingerprint: observation.scene_fingerprint.clone(),
            scene_json: observation.scene_json.clone(),
        };
        let mut source_observations =
            std::collections::BTreeMap::from([(managed_source.space_id.clone(), managed_source)]);
        for source in &observation.source_observations {
            if let Some(existing) =
                source_observations.insert(source.space_id.clone(), source.clone())
                && existing != *source
            {
                return Err(StorageError::MoStateOperationConflict(format!(
                    "Space {} has conflicting source fingerprints",
                    source.space_id
                )));
            }
        }
        let mut source_versions = Vec::with_capacity(source_observations.len());
        for source in source_observations.values() {
            let is_managed_source = source.space_id == observation.space_id;
            let initial_dmw_revision = if is_managed_source { dmw_revision } else { 0 };
            let initial_nsg_revision = if is_managed_source { nsg_revision } else { 0 };
            let initial_scene_revision = if is_managed_source { scene_revision } else { 0 };
            let initial_dmw_fingerprint = if is_managed_source {
                source.dmw_fingerprint.as_str()
            } else {
                ""
            };
            let initial_nsg_fingerprint = if is_managed_source {
                source.nsg_fingerprint.as_str()
            } else {
                ""
            };
            let initial_scene_fingerprint = if is_managed_source {
                source.scene_fingerprint.as_str()
            } else {
                ""
            };
            sqlx::query(
                "INSERT INTO mo_state_source_versions \
                 (managed_space_id, source_space_id, dmw_revision, nsg_revision, scene_revision, \
                  dmw_fingerprint, nsg_fingerprint, scene_fingerprint, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(managed_space_id, source_space_id) DO NOTHING",
            )
            .bind(&observation.space_id)
            .bind(&source.space_id)
            .bind(initial_dmw_revision)
            .bind(initial_nsg_revision)
            .bind(initial_scene_revision)
            .bind(initial_dmw_fingerprint)
            .bind(initial_nsg_fingerprint)
            .bind(initial_scene_fingerprint)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
            let row = sqlx::query(
                "SELECT dmw_revision, nsg_revision, scene_revision, dmw_fingerprint, \
                 nsg_fingerprint, scene_fingerprint FROM mo_state_source_versions \
                 WHERE managed_space_id=? AND source_space_id=?",
            )
            .bind(&observation.space_id)
            .bind(&source.space_id)
            .fetch_one(&mut *transaction)
            .await?;
            let source_dmw_revision = next_revision(
                row.try_get("dmw_revision")?,
                row.try_get("dmw_fingerprint")?,
                &source.dmw_fingerprint,
            );
            let source_nsg_revision = next_revision(
                row.try_get("nsg_revision")?,
                row.try_get("nsg_fingerprint")?,
                &source.nsg_fingerprint,
            );
            let source_scene_revision = next_revision(
                row.try_get("scene_revision")?,
                row.try_get("scene_fingerprint")?,
                &source.scene_fingerprint,
            );
            sqlx::query(
                "UPDATE mo_state_source_versions SET dmw_revision=?, nsg_revision=?, \
                 scene_revision=?, dmw_fingerprint=?, nsg_fingerprint=?, scene_fingerprint=?, \
                 updated_at=? WHERE managed_space_id=? AND source_space_id=?",
            )
            .bind(source_dmw_revision)
            .bind(source_nsg_revision)
            .bind(source_scene_revision)
            .bind(&source.dmw_fingerprint)
            .bind(&source.nsg_fingerprint)
            .bind(&source.scene_fingerprint)
            .bind(&now)
            .bind(&observation.space_id)
            .bind(&source.space_id)
            .execute(&mut *transaction)
            .await?;
            source_versions.push(MoStateSourceVersion {
                space_id: source.space_id.clone(),
                dmw_revision: to_revision(source_dmw_revision)?,
                nsg_revision: to_revision(source_nsg_revision)?,
                scene_revision: to_revision(source_scene_revision)?,
                dmw_fingerprint: source.dmw_fingerprint.clone(),
                nsg_fingerprint: source.nsg_fingerprint.clone(),
                scene_fingerprint: source.scene_fingerprint.clone(),
            });
        }
        let source_versions_json = serde_json::to_string(&source_versions)?;
        sqlx::query(
            "INSERT INTO mo_state_operations \
             (operation_id, space_id, event_type, event_fingerprint, phase, \
              base_dmw_revision, base_nsg_revision, base_scene_revision, source_versions_json, \
              observed_scene_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'applying', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&observation.operation_id)
        .bind(&observation.space_id)
        .bind(&observation.event_type)
        .bind(&observation.event_fingerprint)
        .bind(dmw_revision)
        .bind(nsg_revision)
        .bind(scene_revision)
        .bind(&source_versions_json)
        .bind(&observation.scene_json)
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
            source_versions,
            observed_scene: serde_json::from_str(&observation.scene_json)?,
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
        ddm_update: Option<&DdmProjectionUpdate>,
    ) -> Result<MoStateSnapshot, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let operation_row = sqlx::query(
            "SELECT operation_id, space_id, event_type, event_fingerprint, phase, \
             base_dmw_revision, base_nsg_revision, base_scene_revision, source_versions_json, \
             observed_scene_json, snapshot_json, error \
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
        validate_ddm_projection_update(&operation, &state_result, ddm_update)?;
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
            source_versions: operation.source_versions,
            scene: operation.observed_scene.clone(),
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
        if let Some(update) = ddm_update {
            if update.managed_space_id != operation.space_id || update.profile_revision == 0 {
                return Err(StorageError::MoStateOperationConflict(
                    "DDM projection update does not match the MO State operation".to_owned(),
                ));
            }
            let bands_json = serde_json::to_string(&update.bands)?;
            sqlx::query(
                "INSERT INTO ddm_projection_states \
                 (managed_space_id, conversation_id, character_id, profile_revision, \
                  profile_fingerprint, source_fingerprint, bands_json, updated_at) \
                  VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(managed_space_id, conversation_id, character_id) DO UPDATE SET \
                  profile_revision=excluded.profile_revision, \
                  profile_fingerprint=excluded.profile_fingerprint, \
                  source_fingerprint=excluded.source_fingerprint, \
                  bands_json=excluded.bands_json, updated_at=excluded.updated_at",
            )
            .bind(&update.managed_space_id)
            .bind(&update.conversation_id)
            .bind(&update.character_id)
            .bind(i64::try_from(update.profile_revision).map_err(|_| {
                StorageError::MoStateOperationConflict(
                    "DDM profile revision exceeds SQLite range".to_owned(),
                )
            })?)
            .bind(&update.profile_fingerprint)
            .bind(&update.source_fingerprint)
            .bind(&bands_json)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(snapshot)
    }

    pub async fn ddm_projection_state(
        &self,
        managed_space_id: &str,
        conversation_id: &str,
        character_id: &str,
    ) -> Result<Option<DdmProjectionState>, StorageError> {
        let row = sqlx::query(
            "SELECT managed_space_id, conversation_id, character_id, profile_revision, \
             profile_fingerprint, source_fingerprint, bands_json, updated_at \
             FROM ddm_projection_states \
             WHERE managed_space_id=? AND conversation_id=? AND character_id=?",
        )
        .bind(managed_space_id)
        .bind(conversation_id)
        .bind(character_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(DdmProjectionState {
                managed_space_id: row.try_get("managed_space_id")?,
                conversation_id: row.try_get("conversation_id")?,
                character_id: row.try_get("character_id")?,
                profile_revision: to_revision(row.try_get("profile_revision")?)?,
                profile_fingerprint: row.try_get("profile_fingerprint")?,
                source_fingerprint: row.try_get("source_fingerprint")?,
                bands: serde_json::from_str(&row.try_get::<String, _>("bands_json")?)?,
                updated_at: parse_timestamp(row.try_get("updated_at")?)?,
            })
        })
        .transpose()
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
}

fn validate_ddm_projection_update(
    operation: &MoStateOperation,
    state_result: &serde_json::Value,
    update: Option<&DdmProjectionUpdate>,
) -> Result<(), StorageError> {
    let audit = state_result
        .pointer("/audit/ddm")
        .filter(|value| !value.is_null());
    let Some(update) = update else {
        return if audit.is_none() {
            Ok(())
        } else {
            Err(StorageError::MoStateOperationConflict(
                "DDM audit is missing its atomic projection update".to_owned(),
            ))
        };
    };
    let Some(audit) = audit else {
        return Err(StorageError::MoStateOperationConflict(
            "DDM projection update has no matching audit".to_owned(),
        ));
    };
    let bands = serde_json::to_value(&update.bands)?;
    let matches_audit = update.managed_space_id == operation.space_id
        && update.profile_revision > 0
        && valid_sha256_fingerprint(&update.profile_fingerprint)
        && valid_sha256_fingerprint(&update.source_fingerprint)
        && update.bands.iter().all(|(id, band)| {
            !id.trim().is_empty() && matches!(band.as_str(), "latent" | "salient" | "dominant")
        })
        && audit
            .get("profile_revision")
            .and_then(serde_json::Value::as_u64)
            == Some(update.profile_revision)
        && audit
            .get("profile_fingerprint")
            .and_then(serde_json::Value::as_str)
            == Some(update.profile_fingerprint.as_str())
        && audit
            .get("source_fingerprint")
            .and_then(serde_json::Value::as_str)
            == Some(update.source_fingerprint.as_str())
        && audit.get("next_bands") == Some(&bands);
    if matches_audit {
        Ok(())
    } else {
        Err(StorageError::MoStateOperationConflict(
            "DDM projection update does not match the MO State audit".to_owned(),
        ))
    }
}

fn valid_sha256_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
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
        source_versions: serde_json::from_str(&row.try_get::<String, _>("source_versions_json")?)?,
        observed_scene: serde_json::from_str(&row.try_get::<String, _>("observed_scene_json")?)?,
        snapshot_json: row.try_get("snapshot_json")?,
        error: row.try_get("error")?,
    })
}
