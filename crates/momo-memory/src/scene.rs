use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{MemoryDocument, MemoryError, MemorySnapshot, MemoryWorkspace};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneStatus {
    Inactive,
    Active,
    Transitioning,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneSnapshot {
    pub scene_id: String,
    pub status: SceneStatus,
    pub location: Option<String>,
    pub timeframe: Option<String>,
    pub participants: Vec<String>,
    pub focus: Option<String>,
    pub open_threads: Vec<String>,
    pub constraints: Vec<String>,
    pub source_refs: Vec<String>,
    pub source_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MoStateSourceFingerprint {
    pub dmw: String,
    pub nsg: String,
    pub scene: String,
    pub scene_snapshot: SceneSnapshot,
}

impl MemoryWorkspace {
    /// Produces stable content identities for the authoritative state sources.
    /// Volatile indexes and audit logs are intentionally excluded from the DMW
    /// identity so a read-only retrieval does not manufacture a new state.
    pub fn mo_state_source_fingerprint(&self) -> Result<MoStateSourceFingerprint, MemoryError> {
        let memory = self.export_memory_partition_snapshot()?;
        let nsg = self.export_semantic_graph_partition_snapshot()?;
        let dmw = digest_snapshot(&memory, is_dmw_state_path);
        let nsg = digest_snapshot(&nsg, |_| true);
        let scene_document = self.read("current/scene.md")?;
        let threads_document = self.read("current/active_threads.md")?;
        let scene_source = format!("{}\n{}", scene_document.body, threads_document.body);
        let scene_hash = digest_text(&scene_source);
        let scene_snapshot = parse_scene(&scene_document.body, &threads_document.body, &scene_hash);
        Ok(MoStateSourceFingerprint {
            dmw,
            nsg,
            scene: scene_hash,
            scene_snapshot,
        })
    }
}

fn is_dmw_state_path(path: &str) -> bool {
    [
        "config/",
        "current/",
        "characters/",
        "relationships/",
        "events/",
        "world/",
        "archive/character/",
        "archive/relationship/",
        "archive/event/",
        "archive/world/",
        "tombstones/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn digest_snapshot(snapshot: &MemorySnapshot, include: impl Fn(&str) -> bool) -> String {
    let mut digest = Sha256::new();
    for (path, content) in snapshot.files.iter().filter(|(path, _)| include(path)) {
        let normalized = normalized_source_content(path, content);
        digest.update(path.len().to_le_bytes());
        digest.update(path.as_bytes());
        digest.update(normalized.len().to_le_bytes());
        digest.update(normalized.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn normalized_source_content(path: &str, content: &str) -> String {
    if !path.ends_with(".md") {
        return content.to_owned();
    }
    let Ok(mut document) = MemoryDocument::parse(content) else {
        return content.to_owned();
    };
    // Retrieval freshness is runtime bookkeeping, not a mutation of the
    // authoritative narrative state. Keeping it out of the identity prevents
    // a read from manufacturing a new DMW revision.
    document.metadata.touch_at = 0;
    document.encode().unwrap_or_else(|_| content.to_owned())
}

fn digest_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn parse_scene(scene: &str, active_threads: &str, source_hash: &str) -> SceneSnapshot {
    let sections = markdown_sections(scene);
    let scene_id = first_value(&sections, &["scene id", "scene_id", "场景 id", "场景id"])
        .unwrap_or_else(|| "scene_current".to_owned());
    let focus = first_value(
        &sections,
        &["focus", "summary", "scene", "焦点", "摘要", "场景"],
    )
    .or_else(|| markdown_preamble(scene));
    let status_text = first_value(&sections, &["status", "状态"]);
    let status = match status_text.as_deref().map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("active") || value == "进行中" => {
            SceneStatus::Active
        }
        Some(value) if value.eq_ignore_ascii_case("transitioning") || value == "切换中" => {
            SceneStatus::Transitioning
        }
        Some(value) if value.eq_ignore_ascii_case("closed") || value == "已结束" => {
            SceneStatus::Closed
        }
        Some(value) if value.eq_ignore_ascii_case("inactive") || value == "未开始" => {
            SceneStatus::Inactive
        }
        _ if focus
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()) =>
        {
            SceneStatus::Active
        }
        _ => SceneStatus::Inactive,
    };
    let mut open_threads = list_values(&sections, &["open threads", "开放线程", "未决事项"]);
    open_threads.extend(markdown_list(active_threads));
    open_threads.sort();
    open_threads.dedup();
    let mut source_refs = wiki_references(scene);
    source_refs.extend(wiki_references(active_threads));
    source_refs.sort();
    source_refs.dedup();
    SceneSnapshot {
        scene_id,
        status,
        location: first_value(&sections, &["location", "地点", "位置"]),
        timeframe: first_value(&sections, &["timeframe", "time", "时间"]),
        participants: list_values(&sections, &["participants", "参与者", "角色"]),
        focus,
        open_threads,
        constraints: list_values(&sections, &["constraints", "约束"]),
        source_refs,
        source_hash: source_hash.to_owned(),
    }
}

fn markdown_sections(markdown: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::<String, String>::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        if let Some(name) = line.trim().strip_prefix("## ") {
            let key = name.trim().to_lowercase();
            sections.entry(key.clone()).or_default();
            current = Some(key);
        } else if let Some(current) = current.as_ref() {
            let value = sections.entry(current.clone()).or_default();
            if !value.is_empty() {
                value.push('\n');
            }
            value.push_str(line);
        }
    }
    sections
}

fn first_value(sections: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        sections.get(*name).and_then(|value| {
            value
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.trim_start_matches(['-', '*', ' ']).trim().to_owned())
        })
    })
}

fn list_values(sections: &BTreeMap<String, String>, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .find_map(|name| sections.get(*name))
        .map_or_else(Vec::new, |value| markdown_list(value))
}

fn markdown_list(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

fn markdown_preamble(markdown: &str) -> Option<String> {
    let mut values = Vec::new();
    for line in markdown.lines().map(str::trim) {
        if line.starts_with("## ") {
            break;
        }
        if !line.is_empty() && !line.starts_with('#') {
            values.push(line);
        }
    }
    let value = values.join(" ");
    (!value.is_empty()).then_some(value)
}

fn wiki_references(markdown: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut rest = markdown;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let value = after[..end].trim();
        if !value.is_empty() {
            references.push(value.to_owned());
        }
        rest = &after[end + 2..];
    }
    references
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_scene_without_inference() {
        let scene = "# 当前场景\n\n## Scene ID\nscene_home\n\n## Status\nactive\n\n## Location\n客厅\n\n## Participants\n- momo\n- user\n\n## Focus\n讨论出行\n\n## Constraints\n- 保持安静 [[rule_quiet]]\n";
        let snapshot = parse_scene(scene, "# 活跃剧情线\n\n- 等待确认目的地\n", "hash");
        assert_eq!(snapshot.scene_id, "scene_home");
        assert_eq!(snapshot.status, SceneStatus::Active);
        assert_eq!(snapshot.location.as_deref(), Some("客厅"));
        assert_eq!(snapshot.participants, vec!["momo", "user"]);
        assert_eq!(snapshot.open_threads, vec!["等待确认目的地"]);
        assert_eq!(snapshot.source_refs, vec!["rule_quiet"]);
    }
}
