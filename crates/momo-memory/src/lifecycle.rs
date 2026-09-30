//! Activity-driven lifecycle. Wall time is audit data, never the aging clock.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LifecycleSettings {
    pub enabled: bool,
    pub decay_after_turns: u64,
    pub forget_after_turns: u64,
    pub decay_factor: f64,
    pub auto_forget: bool,
}

impl Default for LifecycleSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            decay_after_turns: 48,
            forget_after_turns: 240,
            decay_factor: 0.9,
            auto_forget: true,
        }
    }
}

impl LifecycleSettings {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if !(1..=1_000_000).contains(&self.decay_after_turns)
            || !(1..=1_000_000).contains(&self.forget_after_turns)
            || !self.decay_factor.is_finite()
            || !(0.0..1.0).contains(&self.decay_factor)
            || self.decay_factor == 0.0
        {
            return Err(MemoryError::InvalidPatch("lifecycle requires turn intervals 1..=1000000 and decay_factor strictly between 0 and 1".into()));
        }
        Ok(())
    }
}

/// Captured by the trusted runtime for a completed response, not model output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleActivity {
    #[serde(default)]
    pub identity: Option<momo_domain::provenance::MemoryIdentity>,
    pub conversation_id: String,
    pub character_id: String,
    pub query: String,
    pub settings: LifecycleSettings,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ActivityLedger {
    #[serde(default)]
    contexts: BTreeMap<String, u64>,
    #[serde(default)]
    records: BTreeMap<String, RecordClock>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecordClock {
    fingerprint: String,
    contexts: BTreeMap<String, ContextClock>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ContextClock {
    last_hit: u64,
    last_decay: u64,
    archived_at: Option<u64>,
}

fn content_fingerprint(doc: &MemoryDocument) -> Result<String, MemoryError> {
    let meta = &doc.metadata;
    let bytes = serde_json::to_vec(&(
        &doc.body,
        &meta.kind,
        &meta.importance,
        &meta.weight,
        &meta.status,
        &meta.relations,
        &meta.tags,
        &meta.aliases,
        &meta.injection_conversation_id,
        &meta.injection_character_id,
    ))
    .map_err(|error| MemoryError::InvalidIndex(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

impl MemoryWorkspace {
    /// Freeze activity, content changes, index, tombstones and audit together.
    /// The caller must journal this plan before applying it, then acknowledge
    /// the response event. Replaying the prepared plan cannot age memory twice.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn prepare_lifecycle_activity(
        &self,
        activity: &LifecycleActivity,
        now: i64,
    ) -> Result<(PreparedMemoryCommit, MaintenanceReport), MemoryError> {
        activity.settings.validate()?;
        let conversation = uuid::Uuid::parse_str(&activity.conversation_id)
            .map_err(|e| MemoryError::InvalidPatch(e.to_string()))?;
        let character = uuid::Uuid::parse_str(&activity.character_id)
            .map_err(|e| MemoryError::InvalidPatch(e.to_string()))?;
        let context = format!("{conversation}:{character}");
        let mut report = MaintenanceReport::default();
        if !activity.settings.enabled {
            return Ok((PreparedMemoryCommit::prepare(&self.root, &[])?, report));
        }
        let ledger_path = self.resolve(Path::new("indexes/lifecycle_activity.json"))?;
        let mut ledger: ActivityLedger = if ledger_path.exists() {
            serde_json::from_slice(&fs::read(&ledger_path)?)
                .map_err(|e| MemoryError::InvalidIndex(e.to_string()))?
        } else {
            ActivityLedger::default()
        };
        let turn = ledger.contexts.entry(context.clone()).or_default();
        *turn = turn
            .checked_add(1)
            .ok_or_else(|| MemoryError::InvalidIndex("activity counter overflow".into()))?;
        let turn = *turn;
        let access = self.load_access()?;
        let provenance = self.provenance(false)?;
        let previous = self.load_index_for_rebuild()?;
        let mut index = self.build_index_data(Some(&previous))?;
        let paths = self.all_long_term_memory_paths()?;
        let mut documents = Vec::new();
        let mut references = HashSet::new();
        let mut current_references = HashSet::new();
        for path in paths {
            let doc = self.read_unchecked(&path)?;
            references.extend(doc.metadata.relations.values().flatten().cloned());
            references.extend(explicit_memory_references(&doc.body));
            documents.push((path, doc));
        }
        if let Some(identity) = &activity.identity {
            for (key, scene) in &provenance.scenes {
                for text in scene.documents.values() {
                    let ids = explicit_memory_references(text);
                    if key == &provenance::scene_key(identity) {
                        current_references.extend(ids.clone());
                    }
                    // An unresolved reference protects its target, but is not
                    // evidence that another character just remembered it.
                    references.extend(ids);
                }
            }
        } else {
            for path in ["current/scene.md", "current/active_threads.md"] {
                current_references.extend(explicit_memory_references(
                    &self.read_unchecked(Path::new(path))?.body,
                ));
            }
        }
        references.extend(current_references.iter().cloned());
        let normalized = normalize(&activity.query);
        let terms = searchable_terms(&normalized);
        let mut mutations = Vec::new();
        let mut tombstones = self.load_tombstones()?;
        let mut audit_events = Vec::new();
        let existing_ids: HashSet<_> = documents
            .iter()
            .map(|(_, d)| d.metadata.id.clone())
            .collect();
        ledger.records.retain(|id, _| existing_ids.contains(id));
        for (relative, mut doc) in documents {
            let meta = &doc.metadata;
            if let Some(policy) = provenance
                .records
                .get(&meta.id)
                .and_then(|r| r.policy.as_ref())
            {
                let Some(identity) = &activity.identity else {
                    continue;
                };
                if !policy.permits(policy.local_space_id, identity, now) {
                    continue;
                }
            }
            // No inference of ownership from the current reader. Explicit
            // bindings restrict aging; unbound legacy records enroll only on
            // a substantive match, never merely because they were injected.
            if !access.can_read(&meta.kind)
                || !access.can_write(&meta.kind)
                || meta.kind != "event"
                || meta.importance.unwrap_or(1.0) >= 0.8
                || meta
                    .injection_conversation_id
                    .as_ref()
                    .is_some_and(|id| id != &activity.conversation_id)
                || meta
                    .injection_character_id
                    .as_ref()
                    .is_some_and(|id| id != &activity.character_id)
            {
                continue;
            }
            let id = meta.id.clone();
            let archived = relative.starts_with("archive");
            let hit = !archived
                && (current_references.contains(&id)
                    || index.entries.get(&id).is_some_and(|entry| {
                        query_hit(entry, &id, &normalized, &terms).substantive
                    }));
            let bound = meta.injection_conversation_id.as_deref()
                == Some(activity.conversation_id.as_str())
                || activity.identity.as_ref().is_some_and(|identity| {
                    provenance
                        .records
                        .get(&id)
                        .and_then(|r| r.policy.as_ref())
                        .is_some_and(|p| {
                            p.owner_character_id == Some(identity.character_id)
                                && p.permits(p.local_space_id, identity, now)
                        })
                });
            let fingerprint = content_fingerprint(&doc)?;
            let record = ledger.records.entry(id.clone()).or_default();
            if record.fingerprint != fingerprint {
                record.fingerprint = fingerprint;
                for (key, clock) in &mut record.contexts {
                    let current = ledger.contexts.get(key).copied().unwrap_or(0);
                    *clock = ContextClock {
                        last_hit: current,
                        last_decay: current,
                        archived_at: archived.then_some(current),
                    };
                }
            }
            if !record.contexts.contains_key(&context) && (hit || bound) {
                record.contexts.insert(
                    context.clone(),
                    ContextClock {
                        last_hit: turn,
                        last_decay: turn,
                        archived_at: archived.then_some(turn),
                    },
                );
            }
            let Some(clock) = record.contexts.get_mut(&context) else {
                continue;
            };
            if hit {
                clock.last_hit = turn;
                clock.last_decay = turn;
                clock.archived_at = archived.then_some(turn);
            } else if archived && clock.archived_at.is_none() {
                clock.archived_at = Some(turn);
            } else if !archived {
                clock.archived_at = None;
            }
            let protected = references.contains(&id)
                || doc.metadata.tags.iter().any(|tag| {
                    matches!(
                        tag.to_ascii_lowercase().as_str(),
                        "commitment" | "promise" | "open" | "pending" | "unresolved"
                    )
                });
            // Shared memories age only when ALL enrolled contexts have
            // advanced enough. Activity in B cannot spend A's retention.
            let decay = !protected
                && record.contexts.iter().all(|(key, clock)| {
                    let current = ledger.contexts.get(key).copied().unwrap_or(0);
                    current.saturating_sub(clock.last_hit) >= activity.settings.decay_after_turns
                        && current.saturating_sub(clock.last_decay)
                            >= activity.settings.decay_after_turns
                });
            if decay {
                doc.metadata.weight =
                    Some(doc.metadata.weight.unwrap_or(1.0) * activity.settings.decay_factor);
                for (key, clock) in &mut record.contexts {
                    clock.last_decay = ledger.contexts.get(key).copied().unwrap_or(0);
                }
                report.decayed_ids.push(id.clone());
                audit_events.push(format!("{now}\tactivity_decay\t{id}\t{context}\t{turn}"));
            }
            let forget = archived
                && !protected
                && activity.settings.auto_forget
                && doc.metadata.importance.unwrap_or(1.0) < 0.2
                && doc.metadata.weight.unwrap_or(1.0) < 0.05
                && record.contexts.iter().all(|(key, clock)| {
                    clock.archived_at.is_some_and(|start| {
                        ledger
                            .contexts
                            .get(key)
                            .copied()
                            .unwrap_or(0)
                            .saturating_sub(start)
                            >= activity.settings.forget_after_turns
                    })
                });
            let source = self.resolve(&relative)?;
            if forget {
                tombstones.insert(
                    id.clone(),
                    ForgottenTombstone {
                        kind: "event".into(),
                        forgotten_at: now,
                        reason: "inactive_completed_turns".into(),
                    },
                );
                mutations.push(FileMutation::Delete { path: source });
                index.entries.remove(&id);
                report.forgotten_ids.push(id.clone());
                audit_events.push(format!("{now}\tactivity_forget\t{id}\t{context}\t{turn}"));
            } else if !archived && decay && !protected && doc.metadata.weight.unwrap_or(1.0) < 0.2 {
                let destination = archive_path_for(&relative, "event")?;
                let destination_path = self.resolve(&destination)?;
                if destination_path.exists() {
                    return Err(MemoryError::InvalidPatch("archive target exists".into()));
                }
                doc.metadata.status = "archived".into();
                doc.metadata.archived_at = Some(now);
                for (key, clock) in &mut record.contexts {
                    clock.archived_at = Some(ledger.contexts.get(key).copied().unwrap_or(0));
                }
                mutations.push(FileMutation::Write {
                    path: destination_path,
                    content: doc.encode()?.into_bytes(),
                });
                mutations.push(FileMutation::Delete { path: source });
                update_index_entry(&mut index, &destination, &doc)?;
                report.archived_ids.push(id.clone());
                audit_events.push(format!("{now}\tactivity_archive\t{id}\t{context}\t{turn}"));
            } else if decay {
                mutations.push(FileMutation::Write {
                    path: source,
                    content: doc.encode()?.into_bytes(),
                });
                update_index_entry(&mut index, &relative, &doc)?;
            }
            record.fingerprint = content_fingerprint(&doc)?;
        }
        for id in &report.forgotten_ids {
            ledger.records.remove(id);
        }
        mutations.push(FileMutation::Write {
            path: ledger_path,
            content: serde_json::to_vec(&ledger)
                .map_err(|e| MemoryError::InvalidIndex(e.to_string()))?,
        });
        mutations.push(FileMutation::Write {
            path: self.checked_index_path()?,
            content: encode_index(&index)?.into_bytes(),
        });
        if !report.forgotten_ids.is_empty() {
            mutations.push(FileMutation::Write {
                path: self.resolve(Path::new("tombstones/forgotten.yaml"))?,
                content: yaml_serde::to_string(&tombstones)?.into_bytes(),
            });
        }
        let audit_path = self.resolve(Path::new("audit/memory.log"))?;
        let mut audit = fs::read_to_string(&audit_path)?;
        audit.push_str(&format!(
            "{now}\tactivity_turn\t{context}\t{turn}\t{}\n",
            serde_json::to_string(&activity.settings)
                .map_err(|e| MemoryError::InvalidIndex(e.to_string()))?
        ));
        for event in audit_events {
            audit.push_str(&event);
            audit.push('\n');
        }
        mutations.push(FileMutation::Write {
            path: audit_path,
            content: audit.into_bytes(),
        });
        let mut context = momo_domain::provenance::RevisionContext::manual("activity_lifecycle");
        context.proposer = "deterministic_lifecycle".into();
        context.authorization = "captured_lifecycle_policy/1".into();
        context.configuration.insert(
            "settings".into(),
            serde_json::to_string(&activity.settings)
                .map_err(|e| MemoryError::InvalidIndex(e.to_string()))?,
        );
        let plan = self.trace_commit(
            PreparedMemoryCommit::prepare(&self.root, &mutations)?,
            None,
            &context,
        )?;
        Ok((plan, report))
    }

    /// Explicit archive of one record must not run calendar aging of the
    /// entire Space as a side effect.
    pub fn move_archived_document(&self, path: &str) -> Result<(), MemoryError> {
        let relative = Path::new(path);
        let document = self.read(relative)?;
        if document.metadata.status != "archived" || relative.starts_with("archive") {
            return Err(MemoryError::InvalidPatch(
                "expected an active-path document marked archived".into(),
            ));
        }
        self.load_access()?.require_write(&document.metadata.kind)?;
        let mut document = document;
        document
            .metadata
            .archived_at
            .get_or_insert(Utc::now().timestamp());
        let destination = archive_path_for(relative, &document.metadata.kind)?;
        let target = self.resolve(&destination)?;
        if target.exists() {
            return Err(MemoryError::InvalidPatch("archive target exists".into()));
        }
        let previous = self.load_index_for_rebuild()?;
        let mut index = self.build_index_data(Some(&previous))?;
        update_index_entry(&mut index, &destination, &document)?;
        provenance::commit_traced(
            &self.root,
            &[
                FileMutation::Write {
                    path: target,
                    content: document.encode()?.into_bytes(),
                },
                FileMutation::Delete {
                    path: self.resolve(relative)?,
                },
                FileMutation::Write {
                    path: self.checked_index_path()?,
                    content: encode_index(&index)?.into_bytes(),
                },
            ],
            &momo_domain::provenance::RevisionContext::manual("archive"),
        )
    }
}

#[cfg(test)]
#[path = "../tests/unit/lifecycle.rs"]
mod tests;
