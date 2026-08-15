//! Dual-Mem Wiki workspace, deterministic retrieval, and YAML patch execution.

pub mod nsg;
pub mod state;

mod filesystem;
mod patch;
mod retrieval;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

pub use momo_domain::SCHEMA_GENERATION;
pub use state::{MoStateAudit, MoStateContext};

use filesystem::*;
use patch::*;
use retrieval::*;

const MEMORY_DIRECTORIES: &[&str] = &[
    "config",
    "current",
    "characters",
    "relationships",
    "events",
    "world",
    "archive/character",
    "archive/relationship",
    "archive/event",
    "archive/world",
    "indexes",
    "tombstones",
    "audit",
    "lore",
    "rules",
    "lore/.pending",
    "rules/.pending",
    "archive/lore",
    "archive/rules",
];

const ACTIVE_MEMORY_DIRECTORIES: &[(&str, &str)] = &[
    ("characters", "character"),
    ("relationships", "relationship"),
    ("events", "event"),
    ("world", "world"),
];
const DECAY_INTERVAL_SECONDS: i64 = 7 * 24 * 60 * 60;
const FORGET_AFTER_SECONDS: i64 = 180 * 24 * 60 * 60;
pub const CLOCK_SKEW_TOLERANCE_SECONDS: i64 = 300;
const EXPANSION_SOURCE_LIMIT: usize = 3;
const NORMAL_EXPANSION_PER_SOURCE: usize = 8;
const HUB_EXPANSION_PER_SOURCE: usize = 3;
const MAX_EXPANSION_TOTAL: usize = 15;
const HUB_THRESHOLD: usize = 15;
const DIRECT_RESERVE_RATIO_NUMERATOR: usize = 60;
const EXPANSION_MAX_RATIO_NUMERATOR: usize = 35;
const HIT_REFRESH_LIMIT: usize = 5;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("memory I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid memory YAML: {0}")]
    Yaml(#[from] yaml_serde::Error),
    #[error("memory path is unsafe: {0}")]
    UnsafePath(PathBuf),
    #[error("memory document has invalid frontmatter")]
    InvalidFrontmatter,
    #[error("memory document is missing section: {0}")]
    MissingSection(String),
    #[error("memory patch is invalid: {0}")]
    InvalidPatch(String),
    #[error("memory index is invalid: {0}")]
    InvalidIndex(String),
    #[error("memory access configuration is invalid: {0}")]
    InvalidAccess(String),
    #[error("memory access denied for {operation} capability: {kind}")]
    AccessDenied {
        operation: &'static str,
        kind: String,
    },
    #[error("memory entry was not found: {0}")]
    NotFound(String),
    #[error("failed to persist memory atomically: {0}")]
    Persist(#[from] tempfile::PersistError),
    #[error("memory transaction failed: {commit}; rollback also failed: {rollback}")]
    TransactionRollback { commit: String, rollback: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub importance: Option<f64>,
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub touch_at: i64,
    #[serde(default)]
    pub decay_at: Option<i64>,
    #[serde(default)]
    pub archived_at: Option<i64>,
    #[serde(default)]
    pub relations: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub injection_scope: Option<String>,
    #[serde(default)]
    pub injection_conversation_id: Option<String>,
    #[serde(default)]
    pub injection_character_id: Option<String>,
    pub status: String,
}

fn clock_skew_exceeds_tolerance(metadata: &Metadata, now: i64) -> bool {
    metadata.touch_at.saturating_sub(now) > CLOCK_SKEW_TOLERANCE_SECONDS
        || metadata
            .decay_at
            .is_some_and(|decay_at| decay_at.saturating_sub(now) > CLOCK_SKEW_TOLERANCE_SECONDS)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryDocument {
    pub metadata: Metadata,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPatchSummary {
    pub targets: Vec<String>,
    pub operation_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemorySnapshot {
    pub version: u32,
    pub files: BTreeMap<String, String>,
}

fn is_semantic_graph_snapshot_path(path: &str) -> bool {
    matches!(
        path,
        value if value.starts_with("lore/")
            || value.starts_with("rules/")
            || value.starts_with("archive/lore/")
            || value.starts_with("archive/rules/")
    )
}

impl MemoryDocument {
    pub fn parse(text: &str) -> Result<Self, MemoryError> {
        let normalized = text.replace("\r\n", "\n");
        let remainder = normalized
            .strip_prefix("---\n")
            .ok_or(MemoryError::InvalidFrontmatter)?;
        let (frontmatter, body) = remainder
            .split_once("\n---\n")
            .ok_or(MemoryError::InvalidFrontmatter)?;
        let metadata = yaml_serde::from_str(frontmatter)?;
        validate_metadata(&metadata)?;
        Ok(Self {
            metadata,
            body: body.to_owned(),
        })
    }

    pub fn encode(&self) -> Result<String, MemoryError> {
        validate_metadata(&self.metadata)?;
        let frontmatter = yaml_serde::to_string(&self.metadata)?;
        Ok(format!(
            "---\n{}---\n{}",
            frontmatter.trim_start_matches("---\n"),
            self.body
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct MemoryIndex {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct MemoryActivity {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, ActivityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct ActivityEntry {
    last_injected_at: i64,
    injection_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct IndexEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AccessConfig {
    version: u32,
    #[serde(default)]
    read: Vec<String>,
    #[serde(default)]
    write: Vec<String>,
    #[serde(default)]
    allow_archive_restore: bool,
}

impl AccessConfig {
    fn validate(&self) -> Result<(), MemoryError> {
        if self.version != 1 {
            return Err(MemoryError::InvalidAccess(format!(
                "unsupported version: {}",
                self.version
            )));
        }
        for capability in self.read.iter().chain(&self.write) {
            if !matches!(
                capability.as_str(),
                "current" | "character" | "relationship" | "event" | "world"
            ) {
                return Err(MemoryError::InvalidAccess(format!(
                    "unsupported memory type: {capability}"
                )));
            }
        }
        Ok(())
    }

    fn can_read(&self, kind: &str) -> bool {
        self.read.iter().any(|value| value == kind)
    }

    fn can_write(&self, kind: &str) -> bool {
        self.write.iter().any(|value| value == kind)
    }

    fn require_read(&self, kind: &str) -> Result<(), MemoryError> {
        if self.can_read(kind) {
            Ok(())
        } else {
            Err(MemoryError::AccessDenied {
                operation: "read",
                kind: kind.to_owned(),
            })
        }
    }

    fn require_write(&self, kind: &str) -> Result<(), MemoryError> {
        if self.can_write(kind) {
            Ok(())
        } else {
            Err(MemoryError::AccessDenied {
                operation: "write",
                kind: kind.to_owned(),
            })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetrievedMemory {
    pub id: String,
    pub path: PathBuf,
    pub body: String,
    pub estimated_tokens: usize,
    pub source_character_ids: Vec<String>,
    pub injection_scope: Option<String>,
    pub injection_conversation_id: Option<String>,
    pub injection_character_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MaintenanceReport {
    pub decayed_ids: Vec<String>,
    pub archived_ids: Vec<String>,
    pub forgotten_ids: Vec<String>,
    pub clock_skew_guarded_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ForgottenTombstone {
    #[serde(rename = "type")]
    kind: String,
    forgotten_at: i64,
    reason: String,
}

pub trait TokenCounter {
    fn count(&self, text: &str) -> usize;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConservativeTokenCounter;

impl TokenCounter for ConservativeTokenCounter {
    fn count(&self, text: &str) -> usize {
        text.chars().count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub status: String,
    pub importance: Option<f64>,
    pub weight: Option<f64>,
    pub touch_at: i64,
    pub source_character_ids: Vec<String>,
    pub injection_scope: Option<String>,
    pub injection_conversation_id: Option<String>,
    pub injection_character_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryWorkspace {
    root: PathBuf,
}

mod workspace;

fn snapshot_partition(
    snapshot: &MemorySnapshot,
    includes: impl Fn(&str) -> bool,
) -> MemorySnapshot {
    MemorySnapshot {
        version: snapshot.version,
        files: snapshot
            .files
            .iter()
            .filter(|(path, _)| includes(path))
            .map(|(path, content)| (path.clone(), content.clone()))
            .collect(),
    }
}

#[derive(Debug)]
enum FileMutation {
    Write { path: PathBuf, content: Vec<u8> },
    Delete { path: PathBuf },
}

impl FileMutation {
    fn path(&self) -> &Path {
        match self {
            Self::Write { path, .. } | Self::Delete { path } => path,
        }
    }
}

fn commit_mutations(mutations: &[FileMutation]) -> Result<(), MemoryError> {
    let mut writer = atomic_write_bytes;
    commit_mutations_with(mutations, &mut writer)
}

fn commit_mutations_with<F>(mutations: &[FileMutation], writer: &mut F) -> Result<(), MemoryError>
where
    F: FnMut(&Path, &[u8]) -> Result<(), MemoryError>,
{
    let mut unique_paths = HashSet::new();
    let mut snapshots = Vec::with_capacity(mutations.len());
    for mutation in mutations {
        if !unique_paths.insert(mutation.path().to_path_buf()) {
            return Err(MemoryError::InvalidPatch(format!(
                "transaction contains duplicate path: {}",
                mutation.path().display()
            )));
        }
        snapshots.push(snapshot_file(mutation.path())?);
    }

    for (index, mutation) in mutations.iter().enumerate() {
        let result = match mutation {
            FileMutation::Write { path, content } => writer(path, content),
            FileMutation::Delete { path } => fs::remove_file(path).map_err(MemoryError::from),
        };
        if let Err(commit_error) = result {
            let mut rollback_errors = Vec::new();
            for rollback_index in (0..=index).rev() {
                if let Err(error) = restore_snapshot(
                    mutations[rollback_index].path(),
                    &snapshots[rollback_index],
                    writer,
                ) {
                    rollback_errors.push(format!(
                        "{}: {error}",
                        mutations[rollback_index].path().display()
                    ));
                }
            }
            if rollback_errors.is_empty() {
                return Err(commit_error);
            }
            return Err(MemoryError::TransactionRollback {
                commit: commit_error.to_string(),
                rollback: rollback_errors.join("; "),
            });
        }
    }
    Ok(())
}

fn snapshot_file(path: &Path) -> Result<Option<Vec<u8>>, MemoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(MemoryError::UnsafePath(path.to_path_buf()))
        }
        Ok(_) => Ok(Some(fs::read(path)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn restore_snapshot<F>(
    path: &Path,
    snapshot: &Option<Vec<u8>>,
    writer: &mut F,
) -> Result<(), MemoryError>
where
    F: FnMut(&Path, &[u8]) -> Result<(), MemoryError>,
{
    if let Some(content) = snapshot {
        return writer(path, content);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path)?;
        }
        Ok(_) => return Err(MemoryError::UnsafePath(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn update_index_entry(
    index: &mut MemoryIndex,
    relative: &Path,
    document: &MemoryDocument,
) -> Result<(), MemoryError> {
    let metadata = &document.metadata;
    if metadata.kind == "current" {
        return Ok(());
    }
    validate_document_location(relative, metadata)?;
    let mut aliases = if metadata.aliases.is_empty() {
        index
            .entries
            .get(&metadata.id)
            .map(|entry| entry.aliases.clone())
            .unwrap_or_default()
    } else {
        metadata.aliases.clone()
    };
    add_title_alias(&mut aliases, &metadata.id, &document.body);
    index.entries.insert(
        metadata.id.clone(),
        IndexEntry {
            path: portable_path(relative),
            kind: metadata.kind.clone(),
            aliases,
            tags: metadata.tags.clone(),
        },
    );
    Ok(())
}

fn add_title_alias(aliases: &mut Vec<String>, id: &str, body: &str) {
    let Some(title) = body
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
    else {
        return;
    };
    let normalized_title = normalize(title);
    if normalized_title == normalize(id)
        || aliases
            .iter()
            .any(|alias| normalize(alias) == normalized_title)
    {
        return;
    }
    aliases.push(title.to_owned());
}

fn encode_index(index: &MemoryIndex) -> Result<String, MemoryError> {
    if index.version != 1 {
        return Err(MemoryError::InvalidIndex(format!(
            "unsupported version: {}",
            index.version
        )));
    }
    Ok(yaml_serde::to_string(index)?)
}

fn encode_activity(activity: &MemoryActivity) -> Result<String, MemoryError> {
    if activity.version != 1 {
        return Err(MemoryError::InvalidIndex(format!(
            "unsupported activity version: {}",
            activity.version
        )));
    }
    Ok(yaml_serde::to_string(activity)?)
}

#[cfg(test)]
mod tests;
