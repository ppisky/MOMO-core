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
fn exact_nsg_create_patch_replay_is_idempotent() {
    let root = tempfile::tempdir().expect("root");
    let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
    workspace.apply_patch(CREATE_DRAFT).expect("first create");
    workspace
        .apply_patch(CREATE_DRAFT)
        .expect("replayed create");
    assert_eq!(workspace.list_nodes(false).expect("nodes").len(), 1);
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
fn retrieval_matches_terms_inside_multiword_anchors() {
    let root = tempfile::tempdir().expect("root");
    let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
    workspace
        .apply_patch_authorized(&CREATE_DRAFT.replace("mode: \"draft\"", "mode: \"canon\""))
        .expect("create Canon fixture");

    let retrieved = workspace
        .retrieve(
            "What happens when someone uses black flame?",
            &[],
            512,
            &ConservativeTokenCounter,
        )
        .expect("retrieve");
    assert!(
        retrieved.iter().any(|item| item.id == "lore_black_flame"),
        "multiword anchor was not searchable: {retrieved:?}"
    );
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
