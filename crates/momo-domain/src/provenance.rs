//! Versioned identity and provenance shared by storage and memory. Host-owned
//! authorization is deliberately separate from model-generated record content.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const PROVENANCE_SCHEMA: &str = "momo.memory-provenance/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: String,
    pub kind: EvidenceKind,
    pub original_space_id: Uuid,
    pub personal_space_id: Uuid,
    pub conversation_id: Option<Uuid>,
    pub character_id: Option<Uuid>,
    pub message_ids: Vec<Uuid>,
    pub configuration: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Conversation,
    Tool,
    Manual,
    Imported,
    Unknown,
}

/// A host-selected continuity. Omitting a mapping uses the conversation UUID,
/// so parallel conversations never share a scene merely by sharing a Space.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryIdentity {
    pub personal_space_id: Uuid,
    pub conversation_id: Uuid,
    pub character_id: Uuid,
    pub continuity_id: Uuid,
    pub function_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    Experience,
    Relationship,
    Commitment,
    Preference,
    Knowledge,
    Rule,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordPolicy {
    pub revision: u64,
    pub local_space_id: Uuid,
    pub personal_space_id: Uuid,
    pub owner_character_id: Option<Uuid>,
    pub readers: Vec<Uuid>,
    pub continuity_ids: Vec<Uuid>,
    pub function_ids: Vec<String>,
    pub fact_kind: FactKind,
    pub subjects: Vec<String>,
    pub participants: Vec<Uuid>,
    pub responsible_characters: Vec<Uuid>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
}

impl RecordPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision == 0
            || self.readers.len() > 256
            || self.continuity_ids.len() > 256
            || self.function_ids.len() > 64
            || self.subjects.len() > 256
            || self.participants.len() > 256
            || self.responsible_characters.len() > 256
            || self
                .function_ids
                .iter()
                .chain(&self.subjects)
                .any(|s| s.is_empty() || s.len() > 512)
            || self
                .valid_from
                .zip(self.valid_until)
                .is_some_and(|(a, b)| a > b)
        {
            return Err("invalid record policy".into());
        }
        if self.fact_kind == FactKind::Rule && self.continuity_ids.is_empty() {
            return Err("rules require an explicit continuity/world mapping".into());
        }
        Ok(())
    }

    pub fn permits(&self, space: Uuid, identity: &MemoryIdentity, now: i64) -> bool {
        self.local_space_id == space
            && self.personal_space_id == identity.personal_space_id
            && (self.owner_character_id == Some(identity.character_id)
                || self.readers.contains(&identity.character_id))
            && (self.continuity_ids.is_empty()
                || self.continuity_ids.contains(&identity.continuity_id))
            && (self.function_ids.is_empty()
                || identity
                    .function_id
                    .as_ref()
                    .is_some_and(|id| self.function_ids.contains(id)))
            && self.valid_from.is_none_or(|t| now >= t)
            && self.valid_until.is_none_or(|t| now <= t)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RevisionContext {
    #[serde(default)]
    pub target_evidence: BTreeMap<String, Vec<String>>,
    pub operation_id: String,
    pub proposer: String,
    pub trigger: String,
    pub coordinator: String,
    pub authorization: String,
    pub configuration: BTreeMap<String, String>,
    pub evidence: Vec<Evidence>,
    pub identity: Option<MemoryIdentity>,
    pub parent_records: Vec<String>,
}

impl RevisionContext {
    pub fn manual(operation: &str) -> Self {
        Self {
            target_evidence: BTreeMap::new(),
            operation_id: Uuid::now_v7().to_string(),
            proposer: "local_author".into(),
            trigger: operation.into(),
            coordinator: "core".into(),
            authorization: "local_author".into(),
            configuration: BTreeMap::new(),
            evidence: Vec::new(),
            identity: None,
            parent_records: Vec::new(),
        }
    }
}
