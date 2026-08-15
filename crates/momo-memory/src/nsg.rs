//! Narrative Semantic Graph parsing, patching, and deterministic retrieval.

use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::{FileMutation, MemoryError, TokenCounter, commit_mutations};

const NSG_DIRECTORIES: &[&str] = &[
    "lore",
    "rules",
    "lore/.pending",
    "rules/.pending",
    "archive/lore",
    "archive/rules",
];
const GENERIC_ANCHOR_DF_RATIO: f64 = 0.25;
const GENERIC_WEIGHT_CAP: f64 = 0.35;
const ZONE3_ABS_MIN: f64 = 0.45;
const ZONE3_REL_RATIO: f64 = 0.75;
const ZONE3_MAX: usize = 2;
const NSG_EXPANSION_SOURCE_LIMIT: usize = 3;
const NSG_NORMAL_EXPANSION_PER_SOURCE: usize = 6;
const NSG_HUB_EXPANSION_PER_SOURCE: usize = 4;
const NSG_MAX_EXPANSION_TOTAL: usize = 12;
const NSG_HUB_THRESHOLD: usize = 12;
const NSG_HUB_EDGE_MIN: f64 = 0.7;
const NSG_EDGE_MIN: f64 = 0.4;
const NSG_HUB_FACTOR: f64 = 0.75;
const NSG_DIRECT_RESERVE_RATIO: f64 = 0.70;
const NSG_EXPANSION_MAX_RATIO: f64 = 0.30;
const RRF_K: f64 = 60.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NsgMode {
    Canon,
    Draft,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NsgStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NsgZone {
    #[serde(rename = "0")]
    Zero,
    #[serde(rename = "2")]
    Two,
    #[serde(rename = "3")]
    Three,
    Auto,
}

impl NsgZone {
    fn parse(value: &str) -> Result<Self, MemoryError> {
        match value.trim() {
            "0" => Ok(Self::Zero),
            "2" => Ok(Self::Two),
            "3" => Ok(Self::Three),
            "auto" => Ok(Self::Auto),
            _ => Err(invalid(format!("unsupported NSG zone: {value}"))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::Two => "2",
            Self::Three => "3",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NsgEdge {
    pub category: String,
    pub relation: String,
    pub weight: f64,
    pub target: String,
}

impl NsgEdge {
    fn validate(&self) -> Result<(), MemoryError> {
        let valid_relation = match self.category.as_str() {
            "structural" => matches!(
                self.relation.as_str(),
                "located_in" | "part_of" | "owned_by" | "contains"
            ),
            "causal" => matches!(
                self.relation.as_str(),
                "causes" | "leads_to" | "trigger" | "prevents"
            ),
            "constraint" => matches!(
                self.relation.as_str(),
                "forbidden_by" | "limited_by" | "requires" | "weak_against"
            ),
            "narrative" => matches!(
                self.relation.as_str(),
                "betrayed_by" | "remembered_with" | "changed_after" | "allied_with"
            ),
            _ => false,
        };
        if !valid_relation || self.target.trim().is_empty() || !(0.0..=1.0).contains(&self.weight) {
            return Err(invalid("invalid NSG edge"));
        }
        Ok(())
    }

    fn key(&self) -> (&str, &str, &str) {
        (&self.category, &self.relation, &self.target)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NsgNode {
    pub id: String,
    #[serde(default = "default_graph_id")]
    pub graph_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub importance: f64,
    pub mode: NsgMode,
    pub status: NsgStatus,
    pub zone: NsgZone,
    pub anchors: Vec<String>,
    pub condition: String,
    pub trigger: String,
    pub consequence: String,
    pub constraint: String,
    #[serde(default)]
    pub source_character_ids: Vec<String>,
    #[serde(default)]
    pub inject_character_ids: Vec<String>,
    pub edges: Vec<NsgEdge>,
}

fn default_graph_id() -> String {
    "default".to_owned()
}

impl NsgNode {
    pub fn parse(text: &str) -> Result<Self, MemoryError> {
        let normalized = text.replace("\r\n", "\n");
        let mut fields = HashMap::new();
        let mut semantic = HashMap::new();
        let mut edges = Vec::new();
        for line in normalized.lines() {
            if let Some(value) = line.strip_prefix("# ") {
                let (key, value) = value
                    .split_once(':')
                    .ok_or_else(|| invalid("invalid NSG metadata line"))?;
                if fields
                    .insert(key.trim().to_owned(), value.trim().to_owned())
                    .is_some()
                {
                    return Err(invalid(format!("duplicate NSG field: {key}")));
                }
            } else if let Some(value) = line.strip_prefix('@') {
                let (key, value) = value
                    .split_once(':')
                    .ok_or_else(|| invalid("invalid NSG semantic line"))?;
                if semantic
                    .insert(key.trim().to_owned(), value.trim().to_owned())
                    .is_some()
                {
                    return Err(invalid(format!("duplicate NSG tag: {key}")));
                }
            } else if let Some(value) = line.strip_prefix("> ") {
                edges.push(parse_edge(value)?);
            } else if !line.trim().is_empty() {
                return Err(invalid("unknown NSG content line"));
            }
        }
        let allowed_fields = [
            "ID",
            "GRAPH",
            "TYPE",
            "IMP",
            "MODE",
            "STATUS",
            "ZONE",
            "SOURCE_CHARACTERS",
            "INJECT_CHARACTERS",
        ];
        if fields
            .keys()
            .any(|key| !allowed_fields.contains(&key.as_str()))
        {
            return Err(invalid("unknown NSG metadata field"));
        }
        let allowed_semantic = [
            "ANCHORS",
            "CONDITION",
            "TRIGGER",
            "CONSEQUENCE",
            "CONSTRAINT",
        ];
        if semantic
            .keys()
            .any(|key| !allowed_semantic.contains(&key.as_str()))
        {
            return Err(invalid("unknown NSG semantic tag"));
        }
        let node = Self {
            id: take_required(&mut fields, "ID")?,
            graph_id: fields.remove("GRAPH").unwrap_or_else(default_graph_id),
            kind: take_required(&mut fields, "TYPE")?,
            importance: fields
                .remove("IMP")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.5),
            mode: match fields.remove("MODE").as_deref().unwrap_or("canon") {
                "canon" => NsgMode::Canon,
                "draft" => NsgMode::Draft,
                _ => NsgMode::Canon,
            },
            status: match fields
                .remove("STATUS")
                .unwrap_or_else(|| "active".to_owned())
                .as_str()
            {
                "active" => NsgStatus::Active,
                "archived" => NsgStatus::Archived,
                _ => NsgStatus::Active,
            },
            zone: fields
                .remove("ZONE")
                .as_deref()
                .map(NsgZone::parse)
                .transpose()?
                .unwrap_or(NsgZone::Auto),
            anchors: semantic
                .remove("ANCHORS")
                .unwrap_or_default()
                .split([',', '，'])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            condition: semantic.remove("CONDITION").unwrap_or_default(),
            trigger: semantic.remove("TRIGGER").unwrap_or_default(),
            consequence: semantic.remove("CONSEQUENCE").unwrap_or_default(),
            constraint: semantic.remove("CONSTRAINT").unwrap_or_default(),
            source_character_ids: split_csv(fields.remove("SOURCE_CHARACTERS")),
            inject_character_ids: split_csv(fields.remove("INJECT_CHARACTERS")),
            edges,
        };
        node.validate()?;
        Ok(node)
    }

    pub fn encode(&self) -> Result<String, MemoryError> {
        self.validate()?;
        let mode = match self.mode {
            NsgMode::Canon => "canon",
            NsgMode::Draft => "draft",
        };
        let status = match self.status {
            NsgStatus::Active => "active",
            NsgStatus::Archived => "archived",
        };
        let mut output = format!(
            "# ID: {}\n# GRAPH: {}\n# TYPE: {}\n# IMP: {}\n# MODE: {mode}\n# STATUS: {status}\n# ZONE: {}\n# SOURCE_CHARACTERS: {}\n# INJECT_CHARACTERS: {}\n\n\
             @ANCHORS: {}\n@CONDITION: {}\n@TRIGGER: {}\n@CONSEQUENCE: {}\n@CONSTRAINT: {}\n",
            self.id,
            self.graph_id,
            self.kind,
            self.importance,
            self.zone.as_str(),
            self.source_character_ids.join(", "),
            self.inject_character_ids.join(", "),
            self.anchors.join(", "),
            self.condition,
            self.trigger,
            self.consequence,
            self.constraint,
        );
        for edge in &self.edges {
            output.push_str(&format!(
                "\n> {}:{} [{}] -> {}",
                edge.category, edge.relation, edge.weight, edge.target
            ));
        }
        output.push('\n');
        Ok(output)
    }

    fn validate(&self) -> Result<(), MemoryError> {
        if self.id.trim().is_empty()
            || self.graph_id.trim().is_empty()
            || self.kind.trim().is_empty()
            || !(0.0..=1.0).contains(&self.importance)
            || self.anchors.iter().any(|value| value.trim().is_empty())
            || self
                .source_character_ids
                .iter()
                .chain(&self.inject_character_ids)
                .any(|value| value.trim().is_empty())
        {
            return Err(invalid("invalid NSG node"));
        }
        let mut edge_keys = HashSet::new();
        for edge in &self.edges {
            edge.validate()?;
            if !edge_keys.insert(edge.key()) {
                return Err(invalid("duplicate NSG edge"));
            }
        }
        Ok(())
    }

    fn graph_context(&self) -> String {
        let mut output = format!(
            "[GRAPH_CONTEXT]\nGRAPH: {}\nID: {}\nCONDITION: {}\nTRIGGER: {}\nCONSEQUENCE: {}\nCONSTRAINT: {}",
            self.graph_id, self.id, self.condition, self.trigger, self.consequence, self.constraint
        );
        for edge in &self.edges {
            output.push_str(&format!(
                "\nEDGE: {}:{} -> {}",
                edge.category, edge.relation, edge.target
            ));
        }
        output.push_str("\n[/GRAPH_CONTEXT]");
        output
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievedNsg {
    pub id: String,
    pub graph_id: String,
    pub path: PathBuf,
    pub zone: u8,
    pub body: String,
    pub score: f64,
    pub estimated_tokens: usize,
    pub source_character_ids: Vec<String>,
    pub inject_character_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NsgCandidateClass {
    Direct,
    Expansion,
}

#[derive(Debug, Clone)]
struct NsgCandidate {
    path: PathBuf,
    node: NsgNode,
    anchor_score: f64,
    retrieval_score: f64,
    class: NsgCandidateClass,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ManagedNsgNode {
    pub path: String,
    #[serde(flatten)]
    pub node: NsgNode,
}

/// Stable embedding input derived from an active `.nsg` node. It is not a
/// second source of truth and is regenerated whenever a node changes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NsgEmbeddingDocument {
    pub node_id: String,
    pub source_hash: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NsgPendingCandidate {
    pub path: String,
    pub node_id: String,
    pub yaml: String,
}

#[derive(Debug, Clone)]
pub struct NsgWorkspace {
    root: PathBuf,
}

impl NsgWorkspace {
    pub fn initialize(root: impl AsRef<Path>) -> Result<Self, MemoryError> {
        fs::create_dir_all(root.as_ref())?;
        let root = fs::canonicalize(root.as_ref())?;
        for relative in NSG_DIRECTORIES {
            fs::create_dir_all(root.join(relative))?;
        }
        Ok(Self { root })
    }

    pub fn apply_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        self.apply_patch_inner(yaml, false)
    }

    pub fn apply_patch_authorized(&self, yaml: &str) -> Result<(), MemoryError> {
        self.apply_patch_inner(yaml, true)
    }

    fn apply_patch_inner(&self, yaml: &str, authorized: bool) -> Result<(), MemoryError> {
        let patch: NsgPatchDocument = yaml_serde::from_str(yaml)?;
        let mut touched = HashSet::new();
        let mut mutations = Vec::new();
        for file_patch in patch.patches {
            if file_patch.operations.is_empty() || !touched.insert(file_patch.target_file.clone()) {
                return Err(invalid("empty or duplicate NSG patch target"));
            }
            let relative = validate_nsg_target(&file_patch.target_file)?;
            let path = self.resolve(&relative)?;
            let create_count = file_patch
                .operations
                .iter()
                .filter(|operation| matches!(operation, NsgOperation::CreateNode { .. }))
                .count();
            if create_count == 1 && file_patch.operations.len() == 1 {
                if path.exists() {
                    return Err(invalid("NSG create target exists"));
                }
                let NsgOperation::CreateNode {
                    metadata,
                    anchors,
                    condition,
                    trigger,
                    consequence,
                    constraint,
                    edges,
                } = file_patch
                    .operations
                    .into_iter()
                    .next()
                    .expect("one operation")
                else {
                    unreachable!()
                };
                if !authorized && metadata.mode != NsgMode::Draft {
                    return Err(invalid("automatic NSG creation must use draft mode"));
                }
                let node = NsgNode {
                    id: metadata.id,
                    graph_id: metadata.graph_id,
                    kind: metadata.kind,
                    importance: metadata.importance,
                    mode: metadata.mode,
                    status: metadata.status,
                    zone: metadata.zone,
                    anchors: split_anchors(&anchors),
                    condition,
                    trigger,
                    consequence,
                    constraint,
                    source_character_ids: metadata.source_character_ids,
                    inject_character_ids: metadata.inject_character_ids,
                    edges,
                };
                mutations.push(FileMutation::Write {
                    path,
                    content: node.encode()?.into_bytes(),
                });
                continue;
            }
            if create_count != 0 {
                return Err(invalid("create_node must be the only operation"));
            }
            let mut node = NsgNode::parse(&fs::read_to_string(&path)?)?;
            for operation in file_patch.operations {
                if matches!(operation, NsgOperation::RevisionCandidate { .. }) {
                    let pending = pending_mutation(&self.root, &relative, &node.id, operation)?;
                    mutations.push(pending);
                    continue;
                }
                if node.mode == NsgMode::Canon && !authorized {
                    let candidate = NsgOperation::RevisionCandidate {
                        reason: "Automatic change was blocked by Canon protection.".to_owned(),
                        suggested_changes: vec![SuggestedChange::try_from(operation)?],
                        source_evidence: "blocked_canon_mutation".to_owned(),
                    };
                    mutations.push(pending_mutation(
                        &self.root, &relative, &node.id, candidate,
                    )?);
                    continue;
                }
                apply_operation(&mut node, operation)?;
            }
            mutations.push(FileMutation::Write {
                path,
                content: node.encode()?.into_bytes(),
            });
        }
        commit_mutations(&mutations)
    }

    pub fn retrieve(
        &self,
        query: &str,
        vector_ranked_ids: &[String],
        max_tokens: usize,
        counter: &impl TokenCounter,
    ) -> Result<Vec<RetrievedNsg>, MemoryError> {
        let nodes = self.active_nodes()?;
        let query_terms = normalized_terms(query);
        let anchor_index = AnchorIndex::build(nodes.iter().map(|(_, node)| node));
        let vector_ranks = vector_ranked_ids
            .iter()
            .enumerate()
            .map(|(rank, id)| (id.as_str(), rank))
            .collect::<HashMap<_, _>>();
        let mut direct = Vec::new();
        let mut by_id = HashMap::new();
        let mut anchor_ranked = Vec::new();
        for (_, node) in &nodes {
            let score = anchor_index.score(&query_terms, &node.anchors);
            if score > 0.0 {
                anchor_ranked.push((node.id.clone(), score));
            }
        }
        anchor_ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        let anchor_ranks = anchor_ranked
            .iter()
            .enumerate()
            .map(|(rank, (id, _))| (id.clone(), rank))
            .collect::<HashMap<_, _>>();
        let max_rrf = nodes
            .iter()
            .map(|(_, node)| rrf_score(node.id.as_str(), &anchor_ranks, &vector_ranks))
            .fold(0.0_f64, f64::max)
            .max(1.0);
        for (path, node) in nodes {
            by_id.insert(node.id.clone(), (path.clone(), node.clone()));
            let score = anchor_index.score(&query_terms, &node.anchors);
            let vector_rank = vector_ranks.get(node.id.as_str()).copied();
            if score > 0.0 || vector_rank.is_some() || node.zone == NsgZone::Zero {
                let fused = if vector_ranked_ids.is_empty() && anchor_ranks.is_empty() {
                    score
                } else {
                    rrf_score(node.id.as_str(), &anchor_ranks, &vector_ranks) / max_rrf
                };
                direct.push(NsgCandidate {
                    path,
                    node,
                    anchor_score: score,
                    retrieval_score: fused,
                    class: NsgCandidateClass::Direct,
                });
            }
        }
        direct.sort_by(compare_candidates);
        let mut candidates = direct.clone();
        let mut included = direct
            .iter()
            .map(|candidate| candidate.node.id.clone())
            .collect::<HashSet<_>>();
        let mut expansion_count = 0_usize;
        for source in direct.iter().take(NSG_EXPANSION_SOURCE_LIMIT) {
            let mut edges = source
                .node
                .edges
                .iter()
                .filter(|edge| edge.weight > 0.0 && by_id.contains_key(&edge.target))
                .collect::<Vec<_>>();
            let is_hub = edges.len() > NSG_HUB_THRESHOLD;
            let min_weight = if is_hub {
                NSG_HUB_EDGE_MIN
            } else {
                NSG_EDGE_MIN
            };
            edges.retain(|edge| edge.weight >= min_weight);
            edges.sort_by(|left, right| {
                right
                    .weight
                    .total_cmp(&left.weight)
                    .then_with(|| left.target.cmp(&right.target))
            });
            let per_source = if is_hub {
                NSG_HUB_EXPANSION_PER_SOURCE
            } else {
                NSG_NORMAL_EXPANSION_PER_SOURCE
            };
            for edge in edges.into_iter().take(per_source) {
                if expansion_count >= NSG_MAX_EXPANSION_TOTAL {
                    break;
                }
                if !included.insert(edge.target.clone()) {
                    continue;
                }
                if let Some((path, node)) = by_id.get(&edge.target) {
                    let hub_factor = if is_hub { NSG_HUB_FACTOR } else { 1.0 };
                    candidates.push(NsgCandidate {
                        path: path.clone(),
                        node: node.clone(),
                        anchor_score: 0.0,
                        retrieval_score: source.retrieval_score * edge.weight * hub_factor,
                        class: NsgCandidateClass::Expansion,
                    });
                    expansion_count += 1;
                }
            }
        }
        candidates.sort_by(compare_candidates);
        let zone3_ids = zone3_candidate_ids(&candidates);

        let mut used = 0;
        let mut direct_used = 0;
        let mut expansion_used = 0;
        let direct_budget = ((max_tokens as f64) * NSG_DIRECT_RESERVE_RATIO) as usize;
        let expansion_budget = ((max_tokens as f64) * NSG_EXPANSION_MAX_RATIO) as usize;
        let mut result = Vec::new();
        for candidate in candidates {
            let body = candidate.node.graph_context();
            let tokens = counter.count(&body);
            if used + tokens > max_tokens {
                continue;
            }
            match candidate.class {
                NsgCandidateClass::Direct => {
                    if direct_used + tokens > direct_budget && direct_used > 0 {
                        continue;
                    }
                    direct_used += tokens;
                }
                NsgCandidateClass::Expansion => {
                    if expansion_used + tokens > expansion_budget {
                        continue;
                    }
                    expansion_used += tokens;
                }
            }
            used += tokens;
            let zone = match candidate.node.zone {
                NsgZone::Zero => 0,
                NsgZone::Three => 3,
                NsgZone::Auto if zone3_ids.contains(&candidate.node.id) => 3,
                NsgZone::Two | NsgZone::Auto => 2,
            };
            result.push(RetrievedNsg {
                id: candidate.node.id,
                graph_id: candidate.node.graph_id,
                path: candidate.path,
                zone,
                body,
                score: candidate.anchor_score,
                estimated_tokens: tokens,
                source_character_ids: candidate.node.source_character_ids,
                inject_character_ids: candidate.node.inject_character_ids,
            });
        }
        result.sort_by_key(|item| item.zone);
        Ok(result)
    }

    pub fn list_nodes(&self, include_archived: bool) -> Result<Vec<ManagedNsgNode>, MemoryError> {
        let mut nodes = self.all_nodes()?;
        nodes.retain(|(_, node)| include_archived || node.status == NsgStatus::Active);
        nodes.sort_by(|left, right| left.1.id.cmp(&right.1.id));
        Ok(nodes
            .into_iter()
            .map(|(path, node)| ManagedNsgNode {
                path: path.to_string_lossy().replace('\\', "/"),
                node,
            })
            .collect())
    }

    pub fn embedding_documents(&self) -> Result<Vec<NsgEmbeddingDocument>, MemoryError> {
        let mut documents = self
            .active_nodes()?
            .into_iter()
            .map(|(_, node)| {
                let content = node.graph_context();
                let source_hash = hex::encode(Sha256::digest(content.as_bytes()));
                NsgEmbeddingDocument {
                    node_id: node.id,
                    source_hash,
                    content,
                }
            })
            .collect::<Vec<_>>();
        documents.sort_by(|left, right| left.node_id.cmp(&right.node_id));
        Ok(documents)
    }

    pub fn list_pending_candidates(&self) -> Result<Vec<NsgPendingCandidate>, MemoryError> {
        let mut candidates = Vec::new();
        for directory in ["lore", "rules"] {
            let pending = self.root.join(directory).join(".pending");
            for entry in fs::read_dir(pending)? {
                let entry = entry?;
                if entry.file_type()?.is_symlink()
                    || entry.path().extension().and_then(|value| value.to_str()) != Some("yaml")
                {
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(&self.root)
                    .map_err(|_| MemoryError::UnsafePath(entry.path()))?
                    .to_path_buf();
                let yaml = fs::read_to_string(entry.path())?;
                if !matches!(
                    yaml_serde::from_str::<NsgOperation>(&yaml)?,
                    NsgOperation::RevisionCandidate { .. }
                ) {
                    return Err(invalid("invalid NSG pending candidate"));
                }
                let file_name = relative
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                let node_id = file_name
                    .rsplit_once('-')
                    .map_or(file_name, |(id, _)| id)
                    .to_owned();
                candidates.push(NsgPendingCandidate {
                    path: relative.to_string_lossy().replace('\\', "/"),
                    node_id,
                    yaml,
                });
            }
        }
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(candidates)
    }

    pub fn approve_pending_candidate(&self, pending_path: &str) -> Result<(), MemoryError> {
        let relative = validate_pending_target(pending_path)?;
        let pending_path = self.resolve(&relative)?;
        let candidate: NsgOperation = yaml_serde::from_str(&fs::read_to_string(&pending_path)?)?;
        let NsgOperation::RevisionCandidate {
            suggested_changes, ..
        } = candidate
        else {
            return Err(invalid("pending file is not an NSG revision candidate"));
        };
        let file_name = relative
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let target_segment = file_name.rsplit_once('-').map_or(file_name, |(id, _)| id);
        let mut matches = self
            .all_nodes()?
            .into_iter()
            .filter(|(_, node)| safe_segment(&node.id) == target_segment)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(invalid("pending candidate target is ambiguous or missing"));
        }
        let (target, mut node) = matches.pop().expect("one target");
        for change in suggested_changes {
            apply_suggested_change(&mut node, change)?;
        }
        commit_mutations(&[
            FileMutation::Write {
                path: self.resolve(&target)?,
                content: node.encode()?.into_bytes(),
            },
            FileMutation::Delete { path: pending_path },
        ])
    }

    pub fn reject_pending_candidate(&self, pending_path: &str) -> Result<(), MemoryError> {
        let relative = validate_pending_target(pending_path)?;
        let path = self.resolve(&relative)?;
        let candidate: NsgOperation = yaml_serde::from_str(&fs::read_to_string(&path)?)?;
        if !matches!(candidate, NsgOperation::RevisionCandidate { .. }) {
            return Err(invalid("pending file is not an NSG revision candidate"));
        }
        commit_mutations(&[FileMutation::Delete { path }])
    }

    pub fn write_node(&self, target_file: &str, node: NsgNode) -> Result<(), MemoryError> {
        node.validate()?;
        let relative = validate_nsg_target(target_file)?;
        let path = self.resolve(&relative)?;
        for (existing_path, existing) in self.all_nodes()? {
            if existing_path != relative && existing.id == node.id {
                return Err(invalid("NSG node ID already exists"));
            }
        }
        commit_mutations(&[FileMutation::Write {
            path,
            content: node.encode()?.into_bytes(),
        }])
    }

    pub fn archive_node(&self, target_file: &str) -> Result<(), MemoryError> {
        let relative = validate_nsg_target(target_file)?;
        let path = self.resolve(&relative)?;
        let mut node = NsgNode::parse(&fs::read_to_string(&path)?)?;
        node.status = NsgStatus::Archived;
        commit_mutations(&[FileMutation::Write {
            path,
            content: node.encode()?.into_bytes(),
        }])
    }

    /// Permanently removes a user-managed semantic node after confirmation.
    pub fn delete_node(&self, target_file: &str) -> Result<(), MemoryError> {
        let relative = validate_nsg_target(target_file)?;
        let path = self.resolve(&relative)?;
        NsgNode::parse(&fs::read_to_string(&path)?)?;
        commit_mutations(&[FileMutation::Delete { path }])
    }

    fn active_nodes(&self) -> Result<Vec<(PathBuf, NsgNode)>, MemoryError> {
        let mut result = self.all_nodes()?;
        result.retain(|(_, node)| node.status == NsgStatus::Active && node.mode == NsgMode::Canon);
        Ok(result)
    }

    fn all_nodes(&self) -> Result<Vec<(PathBuf, NsgNode)>, MemoryError> {
        let mut result = Vec::new();
        for directory in ["lore", "rules"] {
            collect_nsg_files(&self.root, &self.root.join(directory), &mut result)?;
        }
        Ok(result)
    }

    fn resolve(&self, relative: &Path) -> Result<PathBuf, MemoryError> {
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(MemoryError::UnsafePath(relative.to_path_buf()));
        }
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            let parent = fs::canonicalize(parent)?;
            if !parent.starts_with(&self.root) {
                return Err(MemoryError::UnsafePath(relative.to_path_buf()));
            }
        }
        Ok(path)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NsgPatchDocument {
    patches: Vec<NsgFilePatch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NsgFilePatch {
    target_file: String,
    operations: Vec<NsgOperation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum NsgOperation {
    CreateNode {
        metadata: NsgCreateMetadata,
        anchors: String,
        #[serde(default)]
        condition: String,
        #[serde(default)]
        trigger: String,
        #[serde(default)]
        consequence: String,
        #[serde(default)]
        constraint: String,
        #[serde(default)]
        edges: Vec<NsgEdge>,
    },
    UpdateNode {
        fields: NsgNodeUpdate,
    },
    AddEdge {
        edge: NsgEdge,
    },
    RemoveEdge {
        edge: NsgEdgeKey,
    },
    UpdateFrontmatter {
        fields: NsgMetadataUpdate,
    },
    ArchiveNode {
        reason: String,
    },
    RevisionCandidate {
        reason: String,
        suggested_changes: Vec<SuggestedChange>,
        source_evidence: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NsgCreateMetadata {
    id: String,
    #[serde(default = "default_graph_id")]
    graph_id: String,
    #[serde(rename = "type")]
    kind: String,
    importance: f64,
    mode: NsgMode,
    status: NsgStatus,
    #[serde(default = "default_zone")]
    zone: NsgZone,
    #[serde(default)]
    source_character_ids: Vec<String>,
    #[serde(default)]
    inject_character_ids: Vec<String>,
}

fn default_zone() -> NsgZone {
    NsgZone::Auto
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NsgNodeUpdate {
    #[serde(default)]
    anchors: Option<String>,
    #[serde(default)]
    condition: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    consequence: Option<String>,
    #[serde(default)]
    constraint: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NsgEdgeKey {
    category: String,
    relation: String,
    target: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NsgMetadataUpdate {
    #[serde(default)]
    importance: Option<f64>,
    #[serde(default)]
    mode: Option<NsgMode>,
    #[serde(default)]
    status: Option<NsgStatus>,
    #[serde(default)]
    graph_id: Option<String>,
    #[serde(default)]
    source_character_ids: Option<Vec<String>>,
    #[serde(default)]
    inject_character_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum SuggestedChange {
    UpdateNode { fields: NsgNodeUpdate },
    AddEdge { edge: NsgEdge },
    RemoveEdge { edge: NsgEdgeKey },
    UpdateFrontmatter { fields: NsgMetadataUpdate },
    ArchiveNode { reason: String },
}

impl TryFrom<NsgOperation> for SuggestedChange {
    type Error = MemoryError;

    fn try_from(value: NsgOperation) -> Result<Self, Self::Error> {
        match value {
            NsgOperation::UpdateNode { fields } => Ok(Self::UpdateNode { fields }),
            NsgOperation::AddEdge { edge } => Ok(Self::AddEdge { edge }),
            NsgOperation::RemoveEdge { edge } => Ok(Self::RemoveEdge { edge }),
            NsgOperation::UpdateFrontmatter { fields } => Ok(Self::UpdateFrontmatter { fields }),
            NsgOperation::ArchiveNode { reason } => Ok(Self::ArchiveNode { reason }),
            _ => Err(invalid("operation cannot be a revision suggestion")),
        }
    }
}

fn apply_operation(node: &mut NsgNode, operation: NsgOperation) -> Result<(), MemoryError> {
    match operation {
        NsgOperation::UpdateNode { fields } => {
            if let Some(value) = fields.anchors {
                node.anchors = split_anchors(&value);
            }
            if let Some(value) = fields.condition {
                node.condition = value;
            }
            if let Some(value) = fields.trigger {
                node.trigger = value;
            }
            if let Some(value) = fields.consequence {
                node.consequence = value;
            }
            if let Some(value) = fields.constraint {
                node.constraint = value;
            }
        }
        NsgOperation::AddEdge { edge } => {
            edge.validate()?;
            if node
                .edges
                .iter()
                .any(|existing| existing.key() == edge.key())
            {
                return Err(invalid("duplicate NSG edge"));
            }
            node.edges.push(edge);
        }
        NsgOperation::RemoveEdge { edge } => {
            let previous = node.edges.len();
            node.edges.retain(|existing| {
                existing.key() != (&edge.category, &edge.relation, &edge.target)
            });
            if node.edges.len() == previous {
                return Err(invalid("NSG edge was not found"));
            }
        }
        NsgOperation::UpdateFrontmatter { fields } => {
            if let Some(value) = fields.importance {
                node.importance = value;
            }
            if let Some(value) = fields.mode {
                node.mode = value;
            }
            if let Some(value) = fields.status {
                node.status = value;
            }
            if let Some(value) = fields.graph_id {
                node.graph_id = value;
            }
            if let Some(value) = fields.source_character_ids {
                node.source_character_ids = value;
            }
            if let Some(value) = fields.inject_character_ids {
                node.inject_character_ids = value;
            }
        }
        NsgOperation::ArchiveNode { reason } => {
            if reason.trim().is_empty() {
                return Err(invalid("archive reason is empty"));
            }
            node.status = NsgStatus::Archived;
        }
        NsgOperation::CreateNode { .. } | NsgOperation::RevisionCandidate { .. } => {
            return Err(invalid("invalid NSG operation for an existing node"));
        }
    }
    node.validate()
}

fn apply_suggested_change(node: &mut NsgNode, change: SuggestedChange) -> Result<(), MemoryError> {
    let operation = match change {
        SuggestedChange::UpdateNode { fields } => NsgOperation::UpdateNode { fields },
        SuggestedChange::AddEdge { edge } => NsgOperation::AddEdge { edge },
        SuggestedChange::RemoveEdge { edge } => NsgOperation::RemoveEdge { edge },
        SuggestedChange::UpdateFrontmatter { fields } => NsgOperation::UpdateFrontmatter { fields },
        SuggestedChange::ArchiveNode { reason } => NsgOperation::ArchiveNode { reason },
    };
    apply_operation(node, operation)
}

fn pending_mutation(
    root: &Path,
    target: &Path,
    node_id: &str,
    candidate: NsgOperation,
) -> Result<FileMutation, MemoryError> {
    let NsgOperation::RevisionCandidate {
        reason,
        suggested_changes,
        source_evidence,
    } = &candidate
    else {
        return Err(invalid("pending mutation must be a revision candidate"));
    };
    if source_evidence.trim().is_empty() {
        return Err(invalid("revision candidate requires source_evidence"));
    }
    let directory = target
        .components()
        .next()
        .and_then(|value| value.as_os_str().to_str())
        .ok_or_else(|| invalid("invalid NSG target"))?;
    let yaml = yaml_serde::to_string(&candidate)?;
    // Evidence explains why a candidate exists, but it is not its identity.
    // Equivalent suggestions therefore converge on one pending file even when
    // they are rediscovered from a later transcript window.
    let identity = yaml_serde::to_string(&(target.to_string_lossy(), reason, suggested_changes))?;
    let fingerprint = hex::encode(Sha256::digest(identity.as_bytes()));
    let filename = format!("{}-{}.yaml", safe_segment(node_id), &fingerprint[..16]);
    let path = root.join(directory).join(".pending").join(filename);
    Ok(FileMutation::Write {
        path,
        content: yaml.into_bytes(),
    })
}

fn validate_nsg_target(value: &str) -> Result<PathBuf, MemoryError> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path.extension().and_then(|value| value.to_str()) != Some("nsg")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !matches!(
            path.components()
                .next()
                .and_then(|value| value.as_os_str().to_str()),
            Some("lore" | "rules")
        )
    {
        return Err(MemoryError::UnsafePath(path));
    }
    Ok(path)
}

fn validate_pending_target(value: &str) -> Result<PathBuf, MemoryError> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path.extension().and_then(|value| value.to_str()) != Some("yaml")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !matches!(
            path.components()
                .next()
                .and_then(|value| value.as_os_str().to_str()),
            Some("lore" | "rules")
        )
        || path
            .components()
            .nth(1)
            .and_then(|value| value.as_os_str().to_str())
            != Some(".pending")
    {
        return Err(MemoryError::UnsafePath(path));
    }
    Ok(path)
}

fn parse_edge(value: &str) -> Result<NsgEdge, MemoryError> {
    let (relation, target) = value
        .split_once(" -> ")
        .ok_or_else(|| invalid("invalid NSG edge"))?;
    let (typed_relation, weight) = relation
        .rsplit_once(" [")
        .ok_or_else(|| invalid("invalid NSG edge weight"))?;
    let (category, relation) = typed_relation
        .split_once(':')
        .ok_or_else(|| invalid("invalid NSG edge relation"))?;
    let edge = NsgEdge {
        category: category.to_owned(),
        relation: relation.to_owned(),
        weight: weight
            .trim_end_matches(']')
            .parse()
            .map_err(|_| invalid("invalid NSG edge weight"))?,
        target: target.to_owned(),
    };
    edge.validate()?;
    Ok(edge)
}

fn collect_nsg_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(PathBuf, NsgNode)>,
) -> Result<(), MemoryError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            return Err(MemoryError::UnsafePath(entry.path()));
        }
        if entry.file_type()?.is_dir() {
            if entry.file_name() != ".pending" {
                collect_nsg_files(root, &entry.path(), output)?;
            }
        } else if entry.path().extension().and_then(|value| value.to_str()) == Some("nsg") {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| MemoryError::UnsafePath(entry.path()))?
                .to_path_buf();
            let node = NsgNode::parse(&fs::read_to_string(entry.path())?)?;
            output.push((relative, node));
        }
    }
    Ok(())
}

fn compare_candidates(left: &NsgCandidate, right: &NsgCandidate) -> Ordering {
    right
        .retrieval_score
        .total_cmp(&left.retrieval_score)
        .then_with(|| right.anchor_score.total_cmp(&left.anchor_score))
        .then_with(|| right.node.importance.total_cmp(&left.node.importance))
        .then_with(|| left.node.id.cmp(&right.node.id))
}

#[derive(Debug, Default)]
struct AnchorIndex {
    weights: HashMap<String, f64>,
    vocabulary: BTreeSet<String>,
}

impl AnchorIndex {
    fn build<'a>(nodes: impl Iterator<Item = &'a NsgNode>) -> Self {
        let node_anchors = nodes
            .map(|node| {
                node.anchors
                    .iter()
                    .map(|value| normalize(value))
                    .filter(|value| !value.is_empty())
                    .collect::<BTreeSet<_>>()
            })
            .collect::<Vec<_>>();
        let node_count = node_anchors.len().max(1);
        let mut document_frequency = HashMap::<String, usize>::new();
        for anchors in &node_anchors {
            for anchor in anchors {
                *document_frequency.entry(anchor.clone()).or_default() += 1;
            }
        }
        let mut idf = HashMap::new();
        let mut max_idf = 1.0_f64;
        for (term, frequency) in &document_frequency {
            let value = (((node_count + 1) as f64) / ((*frequency + 1) as f64)).ln() + 1.0;
            max_idf = max_idf.max(value);
            idf.insert(term.clone(), value);
        }
        let weights = document_frequency
            .iter()
            .map(|(term, frequency)| {
                let mut weight = idf[term] / max_idf;
                let document_ratio = (*frequency as f64) / (node_count as f64);
                if (node_count >= 8 && document_ratio > GENERIC_ANCHOR_DF_RATIO)
                    || is_generic_anchor(term)
                {
                    weight = weight.min(GENERIC_WEIGHT_CAP);
                }
                (term.clone(), weight)
            })
            .collect::<HashMap<_, _>>();
        Self {
            vocabulary: weights.keys().cloned().collect(),
            weights,
        }
    }

    fn score(&self, raw_query_terms: &BTreeSet<String>, anchors: &[String]) -> f64 {
        let query_terms = raw_query_terms
            .intersection(&self.vocabulary)
            .cloned()
            .collect::<BTreeSet<_>>();
        let anchors = anchors
            .iter()
            .map(|value| normalize(value))
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        let matched = query_terms
            .intersection(&anchors)
            .cloned()
            .collect::<Vec<_>>();
        if query_terms.is_empty() || anchors.is_empty() || matched.is_empty() {
            return 0.0;
        }
        let mass = |terms: &BTreeSet<String>| {
            terms
                .iter()
                .map(|term| self.weights.get(term).copied().unwrap_or(1.0))
                .sum::<f64>()
        };
        let matched_weight = matched
            .iter()
            .map(|term| self.weights.get(term).copied().unwrap_or(1.0))
            .sum::<f64>();
        let query_mass = mass(&query_terms);
        let node_mass = mass(&anchors);
        if query_mass <= f64::EPSILON || node_mass <= f64::EPSILON {
            return 0.0;
        }
        let idf_cosine = matched_weight / (query_mass * node_mass).sqrt();
        let anchor_coverage = matched_weight / node_mass;
        let specificity = matched_weight / (matched.len() as f64);
        (idf_cosine.max(anchor_coverage) * specificity).clamp(0.0, 1.0)
    }
}

fn is_generic_anchor(term: &str) -> bool {
    term.chars().count() <= 1
        || matches!(
            term,
            "the" | "a" | "an" | "and" | "or" | "you" | "me" | "主角" | "角色" | "城市" | "魔法"
        )
}

fn rrf_score(
    id: &str,
    anchor_ranks: &HashMap<String, usize>,
    vector_ranks: &HashMap<&str, usize>,
) -> f64 {
    anchor_ranks
        .get(id)
        .map_or(0.0, |rank| 1.0 / (RRF_K + *rank as f64))
        + vector_ranks
            .get(id)
            .map_or(0.0, |rank| 1.0 / (RRF_K + *rank as f64))
}

fn zone3_candidate_ids(candidates: &[NsgCandidate]) -> HashSet<String> {
    let top_score = candidates
        .iter()
        .filter(|candidate| {
            candidate.class == NsgCandidateClass::Direct
                && candidate.node.zone == NsgZone::Auto
                && candidate.anchor_score >= ZONE3_ABS_MIN
        })
        .map(|candidate| candidate.anchor_score)
        .fold(0.0_f64, f64::max);
    if top_score < ZONE3_ABS_MIN {
        return HashSet::new();
    }
    candidates
        .iter()
        .filter(|candidate| {
            candidate.class == NsgCandidateClass::Direct
                && candidate.node.zone == NsgZone::Auto
                && candidate.anchor_score >= ZONE3_ABS_MIN
                && candidate.anchor_score >= top_score * ZONE3_REL_RATIO
        })
        .take(ZONE3_MAX)
        .map(|candidate| candidate.node.id.clone())
        .collect()
}

fn normalized_terms(value: &str) -> BTreeSet<String> {
    normalize(value)
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

fn normalize(value: &str) -> String {
    value.nfkc().collect::<String>().trim().to_lowercase()
}

fn split_anchors(value: &str) -> Vec<String> {
    value
        .split([',', '，'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn split_csv(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split([',', '，'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn take_required(values: &mut HashMap<String, String>, key: &str) -> Result<String, MemoryError> {
    values
        .remove(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid(format!("missing NSG field: {key}")))
}

fn safe_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn invalid(message: impl Into<String>) -> MemoryError {
    MemoryError::InvalidPatch(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConservativeTokenCounter;

    const CREATE_DRAFT: &str = r#"
patches:
  - target_file: "lore/black_flame.nsg"
    operations:
      - type: "create_node"
        metadata:
          id: "lore_black_flame"
          type: "lore"
          importance: 0.9
          mode: "draft"
          status: "active"
          zone: "auto"
        anchors: "black flame, taboo, magic"
        condition: "The caster lacks a blessing."
        trigger: "The caster uses black flame."
        consequence: "The caster loses vitality."
        constraint: "The spell consumes life."
        edges:
          - category: "constraint"
            relation: "limited_by"
            weight: 0.9
            target: "lore_holy_lake"
"#;

    #[test]
    fn node_round_trip_preserves_semantics_and_edges() {
        let text = r#"# ID: lore_black_flame
# TYPE: lore
# IMP: 0.9
# MODE: canon
# STATUS: active
# ZONE: auto

@ANCHORS: black flame, taboo
@CONDITION: no blessing
@TRIGGER: cast spell
@CONSEQUENCE: life drain
@CONSTRAINT: forbidden

> constraint:limited_by [0.9] -> lore_holy_lake
"#;
        let node = NsgNode::parse(text).expect("parse");
        assert_eq!(
            NsgNode::parse(&node.encode().expect("encode")).expect("parse"),
            node
        );
    }

    #[test]
    fn automatic_creation_requires_draft_and_rejects_unknown_fields() {
        let root = tempfile::tempdir().expect("root");
        let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
        workspace.apply_patch(CREATE_DRAFT).expect("create");
        assert!(root.path().join("lore/black_flame.nsg").exists());

        let unknown = CREATE_DRAFT.replace(
            "        anchors:",
            "        title: \"not allowed\"\n        anchors:",
        );
        assert!(matches!(
            workspace.apply_patch(&unknown),
            Err(MemoryError::Yaml(_))
        ));
    }

    #[test]
    fn manual_management_lists_writes_and_archives_nodes() {
        let root = tempfile::tempdir().expect("root");
        let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
        let node = NsgNode {
            id: "rule_moon_gate".to_owned(),
            graph_id: "moon_world".to_owned(),
            kind: "rule".to_owned(),
            importance: 0.7,
            mode: NsgMode::Canon,
            status: NsgStatus::Active,
            zone: NsgZone::Two,
            anchors: vec!["moon gate".to_owned()],
            condition: "At night.".to_owned(),
            trigger: "A traveler approaches.".to_owned(),
            consequence: "The gate opens.".to_owned(),
            constraint: "A silver key is required.".to_owned(),
            source_character_ids: vec!["character-a".to_owned()],
            inject_character_ids: vec!["character-b".to_owned()],
            edges: Vec::new(),
        };
        workspace
            .write_node("rules/moon_gate.nsg", node.clone())
            .expect("write");
        let listed = workspace.list_nodes(false).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "rules/moon_gate.nsg");
        assert_eq!(listed[0].node, node);

        workspace
            .archive_node("rules/moon_gate.nsg")
            .expect("archive");
        assert!(workspace.list_nodes(false).expect("active list").is_empty());
        assert_eq!(workspace.list_nodes(true).expect("all list").len(), 1);

        workspace
            .delete_node("rules/moon_gate.nsg")
            .expect("permanent delete");
        assert!(workspace.list_nodes(true).expect("empty list").is_empty());
    }

    #[test]
    fn canon_changes_become_pending_candidates() {
        let root = tempfile::tempdir().expect("root");
        let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
        let canon = CREATE_DRAFT.replace("mode: \"draft\"", "mode: \"canon\"");
        workspace
            .apply_patch_authorized(&canon)
            .expect("authorized canon creation");
        workspace
            .apply_patch(
                r#"
patches:
  - target_file: "lore/black_flame.nsg"
    operations:
      - type: "update_node"
        fields:
          constraint: "Changed automatically."
"#,
            )
            .expect("candidate");
        let node = NsgNode::parse(
            &fs::read_to_string(root.path().join("lore/black_flame.nsg")).expect("node"),
        )
        .expect("parse node");
        assert_eq!(node.constraint, "The spell consumes life.");
        assert_eq!(
            fs::read_dir(root.path().join("lore/.pending"))
                .expect("pending")
                .count(),
            1
        );
    }

    #[test]
    fn equivalent_pending_candidates_are_deduplicated_across_evidence_windows() {
        let root = tempfile::tempdir().expect("root");
        let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
        workspace
            .apply_patch_authorized(&CREATE_DRAFT.replace("mode: \"draft\"", "mode: \"canon\""))
            .expect("authorized canon creation");
        let candidate = r#"
patches:
  - target_file: "lore/black_flame.nsg"
    operations:
      - type: "revision_candidate"
        reason: "The established constraint changed."
        suggested_changes:
          - type: "update_node"
            fields:
              constraint: "The spell now consumes memory."
        source_evidence: "first transcript window"
"#;
        workspace.apply_patch(candidate).expect("first candidate");
        workspace
            .apply_patch(&candidate.replace("first transcript window", "later transcript window"))
            .expect("rediscovered candidate");
        assert_eq!(
            fs::read_dir(root.path().join("lore/.pending"))
                .expect("pending")
                .count(),
            1
        );
    }

    #[test]
    fn retrieval_uses_anchor_ranking_one_hop_budget_and_auto_zone() {
        let root = tempfile::tempdir().expect("root");
        let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
        workspace
            .apply_patch_authorized(&CREATE_DRAFT.replace("mode: \"draft\"", "mode: \"canon\""))
            .expect("create");
        workspace
            .apply_patch_authorized(
                r#"
patches:
  - target_file: "lore/holy_lake.nsg"
    operations:
      - type: "create_node"
        metadata:
          id: "lore_holy_lake"
          type: "lore"
          importance: 0.8
          mode: "canon"
          status: "active"
          zone: "2"
        anchors: "holy lake, blessing"
        condition: ""
        trigger: ""
        consequence: "Black flame fails."
        constraint: "The lake purifies dark magic."
"#,
            )
            .expect("create target");
        let retrieved = workspace
            .retrieve("taboo magic", &[], usize::MAX, &ConservativeTokenCounter)
            .expect("retrieve");
        assert!(retrieved.iter().any(|item| item.id == "lore_holy_lake"));
        assert!(
            retrieved.iter().any(|item| {
                item.id == "lore_black_flame" && item.zone == 3 && item.score >= ZONE3_ABS_MIN
            }),
            "{retrieved:#?}"
        );
        assert!(
            retrieved
                .iter()
                .all(|item| item.body.starts_with("[GRAPH_CONTEXT]"))
        );

        workspace
            .apply_patch(&CREATE_DRAFT.replace("black_flame", "draft_flame"))
            .expect("create draft");
        let retrieved = workspace
            .retrieve("draft flame", &[], usize::MAX, &ConservativeTokenCounter)
            .expect("retrieve");
        assert!(retrieved.iter().all(|item| item.id != "lore_draft_flame"));
    }
}
