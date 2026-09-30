//! Portable revision history and host-owned record grants. Model patches never
//! address these files. They are committed with the corresponding after-images.
use super::*;
use momo_domain::provenance::*;
use sha2::{Digest, Sha256};

pub const DMW_LEDGER: &str = "config/provenance.json";
pub const NSG_LEDGER: &str = "rules/provenance.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordRevision {
    pub revision: u64,
    pub parent_revision: Option<u64>,
    pub content_hash: String,
    pub context: RevisionContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RecordHistory {
    #[serde(default)]
    pub exact_evidence: bool,
    pub policy: Option<RecordPolicy>,
    pub revisions: Vec<RecordRevision>,
    pub evidence: BTreeMap<String, Evidence>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedCurrent {
    pub identity: MemoryIdentity,
    pub documents: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceLedger {
    pub schema: String,
    pub records: BTreeMap<String, RecordHistory>,
    pub scenes: BTreeMap<String, ScopedCurrent>,
}

impl Default for ProvenanceLedger {
    fn default() -> Self {
        Self {
            schema: PROVENANCE_SCHEMA.into(),
            records: BTreeMap::new(),
            scenes: BTreeMap::new(),
        }
    }
}

fn invalid(error: impl ToString) -> MemoryError {
    MemoryError::InvalidPatch(error.to_string())
}

impl ProvenanceLedger {
    pub fn parse(text: &str) -> Result<Self, MemoryError> {
        let ledger: Self = serde_json::from_str(text).map_err(invalid)?;
        if ledger.schema != PROVENANCE_SCHEMA {
            return Err(invalid("unsupported provenance schema"));
        }
        for history in ledger.records.values() {
            if let Some(policy) = &history.policy {
                policy.validate().map_err(invalid)?;
            }
            for (index, revision) in history.revisions.iter().enumerate() {
                if revision.revision != index as u64 + 1
                    || revision.parent_revision != (index > 0).then_some(index as u64)
                {
                    return Err(invalid("broken provenance revision chain"));
                }
            }
            for (id, evidence) in &history.evidence {
                if id != &evidence.id {
                    return Err(invalid("evidence identity mismatch"));
                }
            }
        }
        Ok(ledger)
    }
}

pub fn scene_key(identity: &MemoryIdentity) -> String {
    format!(
        "{}:{}:{}",
        identity.personal_space_id, identity.character_id, identity.continuity_id
    )
}

fn record_from_file(path: &str, bytes: &[u8]) -> Option<(String, FactKind)> {
    let text = std::str::from_utf8(bytes).ok()?;
    if path.ends_with(".md") {
        let doc = MemoryDocument::parse(text).ok()?;
        let kind = match doc.metadata.kind.as_str() {
            "relationship" => FactKind::Relationship,
            "event" => FactKind::Experience,
            "world" => FactKind::Knowledge,
            "current" => FactKind::Experience,
            _ => FactKind::Unknown,
        };
        Some((doc.metadata.id, kind))
    } else if path.ends_with(".nsg") {
        Some((nsg::NsgNode::parse(text).ok()?.id, FactKind::Rule))
    } else if path.contains("/.pending/") {
        Some((path.into(), FactKind::Rule))
    } else {
        None
    }
}

fn semantic_hash(path: &str, bytes: &[u8]) -> String {
    let normalized = if path.ends_with(".md") {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| MemoryDocument::parse(text).ok())
            .map(|mut doc| {
                doc.metadata.touch_at = 0;
                doc.metadata.decay_at = None;
                serde_json::to_vec(&(doc.metadata, doc.body)).expect("memory fields serialize")
            })
    } else {
        None
    };
    hex::encode(Sha256::digest(normalized.as_deref().unwrap_or(bytes)))
}

impl MemoryWorkspace {
    pub fn record_revision_matches(
        &self,
        graph: bool,
        id: &str,
        path: &str,
    ) -> Result<bool, MemoryError> {
        let ledger = self.provenance(graph)?;
        self.record_revision_matches_in_ledger(&ledger, id, path)
    }

    /// Reuse the caller's locked ledger snapshot when qualifying many records.
    pub fn record_revision_matches_in_ledger(
        &self,
        ledger: &ProvenanceLedger,
        id: &str,
        path: &str,
    ) -> Result<bool, MemoryError> {
        let Some(revision) = ledger.records.get(id).and_then(|h| h.revisions.last()) else {
            return Ok(true);
        };
        if revision.content_hash.is_empty() {
            return Ok(true);
        }
        let bytes = if path.starts_with("current/")
            && let Some((_, key)) = id.split_once('@')
        {
            ledger
                .scenes
                .get(key)
                .and_then(|s| s.documents.get(path))
                .map(|s| s.as_bytes().to_vec())
        } else {
            let target = self.resolve(Path::new(path))?;
            if target.exists() {
                Some(fs::read(target)?)
            } else {
                None
            }
        };
        Ok(bytes.is_some_and(|bytes| semantic_hash(path, &bytes) == revision.content_hash))
    }
    pub fn provenance(&self, semantic_graph: bool) -> Result<ProvenanceLedger, MemoryError> {
        if !self
            .root
            .join(if semantic_graph { "rules" } else { "config" })
            .exists()
        {
            return Ok(ProvenanceLedger::default());
        }
        let path = self.resolve(Path::new(if semantic_graph {
            NSG_LEDGER
        } else {
            DMW_LEDGER
        }))?;
        if !path.exists() {
            return Ok(ProvenanceLedger::default());
        }
        ProvenanceLedger::parse(&fs::read_to_string(path)?)
    }

    /// Enrich a not-yet-journaled plan. No effect is applied until the caller
    /// durably stores the resulting plan and applies it under the Space lock.
    pub fn trace_commit(
        &self,
        mut plan: PreparedMemoryCommit,
        space: Option<uuid::Uuid>,
        context: &RevisionContext,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        let mut dmw = self.provenance(false)?;
        let mut nsg = self.provenance(true)?;
        let mut changed = [false; 2];
        let mut removed_current = HashSet::new();
        // A move may contain a write followed by a delete of the same ID.
        let written: HashSet<_> = plan
            .files
            .iter()
            .filter_map(|f| {
                f.after
                    .as_deref()
                    .and_then(|bytes| record_from_file(&f.path, bytes))
                    .map(|(id, _)| (f.path.ends_with(".nsg"), id))
            })
            .collect();
        for file in &plan.files {
            let Some(bytes) = file.after.as_deref().or(file.before.as_deref()) else {
                continue;
            };
            let Some((mut id, kind)) = record_from_file(&file.path, bytes) else {
                continue;
            };
            if file
                .before
                .as_ref()
                .zip(file.after.as_ref())
                .is_some_and(|(before, after)| {
                    semantic_hash(&file.path, before) == semantic_hash(&file.path, after)
                })
            {
                continue;
            }
            let graph = file.path.ends_with(".nsg") || file.path.contains("/.pending/");
            if file.after.is_none() && written.contains(&(graph, id.clone())) {
                continue;
            }
            let ledger = if graph { &mut nsg } else { &mut dmw };
            if file.path.starts_with("current/")
                && let Some(identity) = &context.identity
            {
                let key = scene_key(identity);
                let scene = ledger
                    .scenes
                    .entry(key.clone())
                    .or_insert_with(|| ScopedCurrent {
                        identity: identity.clone(),
                        documents: BTreeMap::new(),
                    });
                if let Some(after) = &file.after {
                    scene.documents.insert(
                        file.path.clone(),
                        String::from_utf8(after.clone()).map_err(invalid)?,
                    );
                }
                id = format!("{id}@{key}");
                removed_current.insert(file.path.clone());
            }
            let history = ledger.records.entry(id).or_default();
            if let Some(identity) = &context.identity
                && !file.path.starts_with("current/")
                && (file.before.is_some() || !history.revisions.is_empty())
                && !file.path.contains("/.pending/")
                && !history.policy.as_ref().is_some_and(|policy| {
                    policy.owner_character_id == Some(identity.character_id)
                        && space.is_some_and(|space| {
                            policy.permits(space, identity, Utc::now().timestamp())
                        })
                })
            {
                return Err(invalid(
                    "automatic maintenance cannot change another identity's or unbound legacy record",
                ));
            }
            let mut revision_context = context.clone();
            let mut parents_exact = true;
            // Approving a candidate inherits the proposal's original evidence,
            // never relabels its model author as the approving user.
            for parent in &context.parent_records {
                let mut parts = parent.splitn(3, '/');
                let first = parts.next().unwrap_or_default();
                let (parent_graph, parent_id) =
                    if let Ok(parent_space) = uuid::Uuid::parse_str(first) {
                        if Some(parent_space) != space {
                            return Err(invalid("parent record belongs to a different Space"));
                        }
                        let parent_graph = match parts.next() {
                            Some("dmw") => false,
                            Some("nsg") => true,
                            _ => return Err(invalid("invalid parent record namespace")),
                        };
                        (
                            parent_graph,
                            parts
                                .next()
                                .ok_or_else(|| invalid("parent record ID missing"))?,
                        )
                    } else {
                        (graph, parent.as_str())
                    };
                if let Some(parent_history) = self.provenance(parent_graph)?.records.get(parent_id)
                {
                    parents_exact &= parent_history.exact_evidence;
                    for (id, evidence) in &parent_history.evidence {
                        history.evidence.insert(id.clone(), evidence.clone());
                    }
                    if let Some(last) = parent_history.revisions.last() {
                        revision_context
                            .configuration
                            .insert(format!("parent:{parent}"), last.revision.to_string());
                    }
                }
            }
            let selected = context.target_evidence.get(&file.path);
            history.exact_evidence = selected.is_some()
                && parents_exact
                && (history.revisions.is_empty() || history.exact_evidence);
            for evidence in context
                .evidence
                .iter()
                .filter(|e| selected.is_none_or(|ids| ids.contains(&e.id)))
            {
                if history
                    .evidence
                    .get(&evidence.id)
                    .is_some_and(|old| old != evidence)
                {
                    return Err(invalid("evidence ID reused with different origin"));
                }
                history
                    .evidence
                    .insert(evidence.id.clone(), evidence.clone());
            }
            if history.policy.is_none()
                && let (Some(space), Some(identity)) = (space, &context.identity)
            {
                history.policy = Some(RecordPolicy {
                    revision: 1,
                    local_space_id: space,
                    personal_space_id: identity.personal_space_id,
                    owner_character_id: Some(identity.character_id),
                    readers: Vec::new(),
                    continuity_ids: vec![identity.continuity_id],
                    function_ids: identity.function_id.iter().cloned().collect(),
                    fact_kind: kind,
                    subjects: Vec::new(),
                    participants: Vec::new(),
                    responsible_characters: Vec::new(),
                    valid_from: None,
                    valid_until: None,
                });
            }
            let revision = history.revisions.len() as u64 + 1;
            history.revisions.push(RecordRevision {
                revision,
                parent_revision: (revision > 1).then_some(revision - 1),
                content_hash: file
                    .after
                    .as_ref()
                    .map(|v| semantic_hash(&file.path, v))
                    .unwrap_or_default(),
                context: revision_context,
            });
            history.deleted = file.after.is_none();
            changed[usize::from(graph)] = true;
        }
        plan.files.retain(|f| !removed_current.contains(&f.path));
        for (graph, ledger) in [(false, dmw), (true, nsg)] {
            if changed[usize::from(graph)] {
                self.add_ledger_to_plan(&mut plan, graph, &ledger)?;
            }
        }
        Ok(plan)
    }

    fn add_ledger_to_plan(
        &self,
        plan: &mut PreparedMemoryCommit,
        graph: bool,
        ledger: &ProvenanceLedger,
    ) -> Result<(), MemoryError> {
        let path = if graph { NSG_LEDGER } else { DMW_LEDGER };
        let target = self.resolve(Path::new(path))?;
        if plan.files.iter().any(|f| f.path == path) {
            return Err(invalid("model patch cannot modify provenance"));
        }
        plan.files.push(crate::commit::PreparedFile {
            path: path.into(),
            before: snapshot_file(&target)?,
            after: Some(serde_json::to_vec(ledger).map_err(invalid)?),
        });
        Ok(())
    }

    pub fn set_record_policy(
        &self,
        graph: bool,
        id: &str,
        policy: RecordPolicy,
    ) -> Result<(), MemoryError> {
        policy.validate().map_err(invalid)?;
        let mut ledger = self.provenance(graph)?;
        let history = ledger.records.entry(id.into()).or_default();
        if history
            .policy
            .as_ref()
            .is_some_and(|p| p.revision.checked_add(1) != Some(policy.revision))
            || (history.policy.is_none() && policy.revision != 1)
        {
            return Err(invalid("policy revision must advance exactly once"));
        }
        let revision = history.revisions.len() as u64 + 1;
        let mut content_hash = history
            .revisions
            .last()
            .map(|r| r.content_hash.clone())
            .unwrap_or_default();
        // Granting an untracked local record also pins its current content.
        // Later filesystem edits cannot silently retain the old authorization.
        if content_hash.is_empty() {
            if graph {
                if let Some(record) = nsg::NsgWorkspace::initialize(&self.root)?
                    .list_nodes(true)?
                    .into_iter()
                    .find(|r| r.node.id == id)
                {
                    let bytes = fs::read(self.resolve(Path::new(&record.path))?)?;
                    content_hash = semantic_hash(&record.path, &bytes);
                }
            } else if let Ok(document) = self.read_document_by_id(id) {
                content_hash = semantic_hash("record.md", document.encode()?.as_bytes());
            }
        }
        let mut context = RevisionContext::manual("record_policy");
        context.configuration.insert(
            "policy".into(),
            serde_json::to_string(&policy).map_err(invalid)?,
        );
        history.revisions.push(RecordRevision {
            revision,
            parent_revision: (revision > 1).then_some(revision - 1),
            content_hash,
            context,
        });
        history.policy = Some(policy);
        let mut plan = PreparedMemoryCommit::prepare(&self.root, &[])?;
        self.add_ledger_to_plan(&mut plan, graph, &ledger)?;
        self.apply_prepared_commit(&plan)
    }

    pub fn scoped_current(
        &self,
        identity: &MemoryIdentity,
    ) -> Result<Vec<RetrievedMemory>, MemoryError> {
        let ledger = self.provenance(false)?;
        let Some(scene) = ledger.scenes.get(&scene_key(identity)) else {
            return Ok(Vec::new());
        };
        scene
            .documents
            .iter()
            .map(|(path, text)| {
                let doc = MemoryDocument::parse(text)?;
                Ok(RetrievedMemory {
                    id: format!("{}@{}", doc.metadata.id, scene_key(identity)),
                    path: path.into(),
                    body: doc.body.clone(),
                    estimated_tokens: doc.body.chars().count(),
                    source_character_ids: Vec::new(),
                    injection_scope: Some("conversation".into()),
                    injection_conversation_id: Some(identity.conversation_id.to_string()),
                    injection_character_id: Some(identity.character_id.to_string()),
                    state_signal: Some(RetrievedStateSignal {
                        kind: "current".into(),
                        weight: 1.0,
                        touch_at: doc.metadata.touch_at,
                        tags: doc.metadata.tags,
                        relations: doc.metadata.relations,
                    }),
                })
            })
            .collect()
    }
}

/// Manual low-level mutations retain honest unknown origin when the host has
/// not supplied evidence. No current reader is inferred to be their author.
pub(crate) fn commit_traced(
    root: &Path,
    mutations: &[FileMutation],
    context: &RevisionContext,
) -> Result<(), MemoryError> {
    let workspace = MemoryWorkspace {
        root: root.to_path_buf(),
        index_cache: Arc::new(RwLock::new(None)),
    };
    let plan = PreparedMemoryCommit::prepare(root, mutations)?;
    let plan = workspace.trace_commit(plan, None, context)?;
    workspace.apply_prepared_commit(&plan)
}

pub fn validate_snapshot_provenance(snapshot: &MemorySnapshot) -> Result<(), MemoryError> {
    for path in [DMW_LEDGER, NSG_LEDGER] {
        if let Some(text) = snapshot.files.get(path) {
            ProvenanceLedger::parse(text)?;
        }
    }
    Ok(())
}

pub fn patch_evidence(
    yaml: &str,
    allowed: &[Evidence],
) -> Result<BTreeMap<String, Vec<String>>, MemoryError> {
    let patch: serde_json::Value = yaml_serde::from_str(yaml)?;
    let mut result = BTreeMap::new();
    for item in patch["patches"].as_array().into_iter().flatten() {
        if let Some(refs) = item.get("evidence_refs") {
            let refs: Vec<String> = serde_json::from_value(refs.clone()).map_err(invalid)?;
            if refs.is_empty() || refs.iter().any(|id| !allowed.iter().any(|e| &e.id == id)) {
                return Err(invalid(
                    "evidence_refs must reference captured evidence from this batch",
                ));
            }
            let target = item["target_file"]
                .as_str()
                .ok_or_else(|| invalid("evidence target missing"))?;
            result.insert(target.into(), refs);
        }
    }
    Ok(result)
}

/// Imported history remains attributable, but an artifact cannot grant local
/// read access or attest that its original conversations are locally verified.
pub fn imported_ledger(text: &str, destination: uuid::Uuid) -> Result<Vec<u8>, MemoryError> {
    let mut ledger = ProvenanceLedger::parse(text)?;
    for history in ledger.records.values_mut() {
        let mut context = RevisionContext::manual("import");
        context.proposer = "importer".into();
        context
            .configuration
            .insert("destination_space".into(), destination.to_string());
        if let Some(policy) = history.policy.take() {
            context.configuration.insert(
                "untrusted_imported_policy".into(),
                serde_json::to_string(&policy).map_err(invalid)?,
            );
        }
        let revision = history.revisions.len() as u64 + 1;
        let content_hash = history
            .revisions
            .last()
            .map(|r| r.content_hash.clone())
            .unwrap_or_default();
        history.revisions.push(RecordRevision {
            revision,
            parent_revision: (revision > 1).then_some(revision - 1),
            content_hash,
            context,
        });
    }
    serde_json::to_vec(&ledger).map_err(invalid)
}

#[cfg(test)]
#[path = "../tests/unit/provenance.rs"]
mod tests;
