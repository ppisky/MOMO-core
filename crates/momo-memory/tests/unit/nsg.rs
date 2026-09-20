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
fn invalid_scalar_metadata_uses_conservative_defaults() {
    let node = NsgNode::parse(
        r#"# ID: lore_defaults
# TYPE: lore
# IMP: not-a-number
# MODE: experimental
# STATUS: typo
# ZONE: somewhere

@ANCHORS: defaults
"#,
    )
    .expect("invalid scalar metadata degrades locally");
    assert_eq!(node.importance, 0.5);
    assert_eq!(node.mode, NsgMode::Canon);
    assert_eq!(node.status, NsgStatus::Archived);
    assert_eq!(node.zone, NsgZone::Auto);
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
fn validation_checks_current_graph_without_writing() {
    let root = tempfile::tempdir().expect("root");
    let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");

    workspace.validate_patch(CREATE_DRAFT).expect("valid patch");
    assert!(!root.path().join("lore/black_flame.nsg").exists());

    let unauthorized = CREATE_DRAFT.replace("mode: \"draft\"", "mode: \"canon\"");
    assert!(matches!(
        workspace.validate_patch(&unauthorized),
        Err(MemoryError::InvalidPatch(_))
    ));
    assert!(!root.path().join("lore/black_flame.nsg").exists());
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
fn direct_candidates_can_use_budget_left_by_an_empty_expansion_pool() {
    let root = tempfile::tempdir().expect("root");
    let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
    let node = |id: &str, consequence: &str| NsgNode {
        id: id.to_owned(),
        graph_id: "budget-test".to_owned(),
        kind: "lore".to_owned(),
        importance: 0.8,
        mode: NsgMode::Canon,
        status: NsgStatus::Active,
        zone: NsgZone::Two,
        anchors: vec!["budget marker".to_owned()],
        condition: "The condition is known.".to_owned(),
        trigger: "The marker is mentioned.".to_owned(),
        consequence: consequence.to_owned(),
        constraint: "No graph edges are present.".to_owned(),
        source_character_ids: Vec::new(),
        inject_character_ids: Vec::new(),
        edges: Vec::new(),
    };
    let first = node("budget_first", "The first independent fact applies.");
    let second = node("budget_second", "The second independent fact applies.");
    let max_tokens = ConservativeTokenCounter.count(&first.graph_context())
        + ConservativeTokenCounter.count(&second.graph_context());
    workspace
        .write_node("lore/budget_first.nsg", first)
        .expect("first node");
    workspace
        .write_node("lore/budget_second.nsg", second)
        .expect("second node");

    let retrieved = workspace
        .retrieve("budget marker", &[], max_tokens, &ConservativeTokenCounter)
        .expect("retrieve");
    assert!(retrieved.iter().any(|item| item.id == "budget_first"));
    assert!(retrieved.iter().any(|item| item.id == "budget_second"));
}

#[test]
fn retrieval_excludes_one_invalid_file_without_losing_valid_nodes() {
    let root = tempfile::tempdir().expect("root");
    let workspace = NsgWorkspace::initialize(root.path()).expect("workspace");
    workspace
        .apply_patch_authorized(&CREATE_DRAFT.replace("mode: \"draft\"", "mode: \"canon\""))
        .expect("create Canon fixture");
    fs::write(
        root.path().join("lore/invalid.nsg"),
        "this is not an NSG metadata document\n",
    )
    .expect("invalid node fixture");

    let retrieved = workspace
        .retrieve("black flame", &[], 512, &ConservativeTokenCounter)
        .expect("one invalid node must not abort retrieval");

    assert!(retrieved.iter().any(|item| item.id == "lore_black_flame"));
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
