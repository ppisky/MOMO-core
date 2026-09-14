use super::*;
use crate::{ConservativeTokenCounter, RetrievedStateSignal};
use std::path::PathBuf;

fn retrieved_signal(
    id: &str,
    kind: &str,
    weight: f64,
    touch_at: i64,
    tags: &[&str],
    relations: BTreeMap<String, Vec<String>>,
    body: &str,
) -> RetrievedMemory {
    RetrievedMemory {
        id: id.to_owned(),
        path: PathBuf::from(format!("missing/{id}.md")),
        body: body.to_owned(),
        estimated_tokens: body.chars().count(),
        source_character_ids: Vec::new(),
        injection_scope: None,
        injection_conversation_id: None,
        injection_character_id: None,
        state_signal: Some(RetrievedStateSignal {
            kind: kind.to_owned(),
            weight,
            touch_at,
            tags: tags.iter().map(|value| (*value).to_owned()).collect(),
            relations,
        }),
    }
}

#[test]
fn formats_empty_context_when_no_signals_match() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let context = workspace
        .compile_mo_state(&[], &[], 1024, &ConservativeTokenCounter)
        .expect("compile");
    assert!(context.context.is_empty());
    assert!(!context.audit.degraded);
}

#[test]
fn generic_tag_guard_depends_only_on_scalar_count() {
    assert!(!is_generic_tag("tag_00"));
    assert!(is_generic_tag("x"));
}

#[test]
fn ddm_is_projected_after_state_without_exposing_numeric_audit() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let memory = retrieved_signal(
        "danger_event",
        "event",
        0.9,
        1,
        &["danger"],
        BTreeMap::new(),
        "body",
    );
    let profile = DdmProfile::parse_yaml(
        r#"
schema: momo.ddm/1
character_id: 018f0000-0000-7000-8000-000000000001
revision: 7
profile: logit_additive
dispositions:
  - id: protect_companion
    base_activation: 0.72
    modulation:
      context:
        - id: immediate_danger
          signal: dmw.tag.danger
          when: true
          delta: 0.90
    expression:
      latent: cue_0
      salient: cue_1
      dominant: cue_2
    constraints:
      - constraint_0
"#,
    )
    .expect("profile");
    let context = workspace
        .compile_mo_state_with_ddm(
            &[memory],
            &[],
            100_000,
            &ConservativeTokenCounter,
            Some(&profile),
            None,
        )
        .expect("compile");
    assert!(context.context.contains("cue_2"));
    assert!(context.context.contains("constraint_0"));
    assert!(!context.context.contains("0.72"));
    let ddm = context.audit.ddm.expect("DDM audit");
    assert_eq!(ddm.character_id, "018f0000-0000-7000-8000-000000000001");
    assert_eq!(ddm.profile_revision, 7);
    assert_eq!(
        ddm.effective_dispositions[0].matched_rule_ids,
        ["immediate_danger"]
    );
}

#[test]
fn ddm_uses_governed_scene_request_and_previous_band_inputs() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let profile = DdmProfile::parse_yaml(
        r#"
schema: momo.ddm/1
character_id: 018f0000-0000-7000-8000-000000000001
revision: 8
profile: logit_additive
selection:
  salient_threshold: 0.65
  dominant_threshold: 0.85
  hysteresis_margin: 0.05
dispositions:
  - id: scene_awareness
    base_activation: 0.64
    modulation:
      context:
        - id: active_scene
          signal: scene.status
          when: active
          delta: 0.01
        - id: image_request
          signal: request.has_image
          when: true
          delta: 0.01
    expression:
      latent: cue_0
      salient: cue_1
      dominant: cue_2
"#,
    )
    .expect("profile");
    let runtime = DdmRuntimeInput {
        previous_bands: [("scene_awareness".to_owned(), DdmBand::Salient)]
            .into_iter()
            .collect(),
        scene: Some(SceneSnapshot {
            scene_id: "scene-1".to_owned(),
            status: crate::SceneStatus::Active,
            location: None,
            timeframe: None,
            participants: vec!["character-1".to_owned()],
            focus: None,
            open_threads: Vec::new(),
            constraints: Vec::new(),
            source_refs: vec!["event-1".to_owned()],
            source_hash: "scene-hash".to_owned(),
        }),
        request_event_type: "user_message".to_owned(),
        request_has_image: true,
        request_evidence_id: "request-1".to_owned(),
    };
    let output = workspace
        .compile_mo_state_with_ddm(
            &[],
            &[],
            100_000,
            &ConservativeTokenCounter,
            Some(&profile),
            Some(&runtime),
        )
        .expect("compile");
    let ddm = output.audit.ddm.expect("DDM audit");
    assert_eq!(ddm.effective_dispositions[0].band, DdmBand::Salient);
    assert_eq!(
        ddm.effective_dispositions[0].matched_rule_ids,
        ["active_scene", "image_request"]
    );
    assert!(
        ddm.effective_dispositions[0]
            .evidence_ids
            .contains(&"scene:scene-hash".to_owned())
    );
    assert!(
        ddm.effective_dispositions[0]
            .evidence_ids
            .contains(&"request:request-1".to_owned())
    );
}

#[test]
fn compiles_only_from_the_retrieval_snapshot() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let memory = retrieved_signal(
        "relationship_test",
        "relationship",
        0.8,
        1,
        &["conflict"],
        BTreeMap::new(),
        "payload_0",
    );
    let context = workspace
        .compile_mo_state(&[memory], &[], 100_000, &ConservativeTokenCounter)
        .expect("compile");
    assert_eq!(context.audit.matched_rules, ["stance_conflict"]);
    assert!(!context.audit.degraded);
}

#[test]
fn invalid_override_rules_degrade_locally_and_priority_must_be_exact() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    fs::write(
        root.path().join("config/state_contract.yaml"),
        r#"
version: 1
dimensions:
  relational_stance:
    signal_source: dmw
    match_mode: first
    rules:
      - id: invalid_condition
        condition:
          unknown_filter: [conflict]
        directives: ["directive_0"]
      - id: valid_condition
        condition:
          tags_any: [conflict]
        directives: ["directive_1"]
conflict_priority:
  - scene_constraint
  - scene_constraint
  - physiological_state
  - epistemic_state
  - relational_stance
  - emotional_tone
"#,
    )
    .expect("override");
    let memory = retrieved_signal(
        "relationship_test",
        "relationship",
        0.8,
        1,
        &["conflict"],
        BTreeMap::new(),
        "body",
    );
    let context = workspace
        .compile_mo_state(&[memory], &[], 100_000, &ConservativeTokenCounter)
        .expect("compile");
    assert!(context.context.contains("directive_1"));
    assert!(!context.context.contains("directive_0"));
    assert!(context.audit.degraded);
    assert!(
        context
            .audit
            .warnings
            .iter()
            .any(|warning| warning.contains("exact five-dimension permutation"))
    );
}

#[test]
fn state_context_never_exceeds_its_hard_budget() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let scene = retrieved_signal(
        "current_scene",
        "current",
        0.0,
        1,
        &[],
        BTreeMap::new(),
        "## Environment\npayload_0 payload_1 payload_2 payload_3 payload_4.",
    );
    let context = workspace
        .compile_mo_state(&[scene], &[], 10, &ConservativeTokenCounter)
        .expect("compile");
    assert!(context.context.is_empty());
    assert_eq!(context.audit.token_count, 0);
    assert!(context.audit.degraded);
}

#[test]
fn epistemic_absence_is_scoped_to_the_target_character() {
    let mut relations = BTreeMap::new();
    relations.insert("characters".to_owned(), vec!["char_other".to_owned()]);
    let event = DmwSignal {
        id: "secret".to_owned(),
        kind: "event".to_owned(),
        weight: 1.0,
        touch_at: 1,
        tags: ["secret", "witness"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        relations,
        body: String::new(),
    };
    let condition = RuleCondition {
        mode: Some("absence".to_owned()),
        event_tag: Some("secret".to_owned()),
        required_witness_tag: Some("witness".to_owned()),
        character_ref: Some("char_target".to_owned()),
        ..RuleCondition::default()
    };
    assert!(epistemic_match(&condition, &[event]));
}

#[test]
fn epistemic_absence_excludes_secret_event_and_linked_projection_from_prompt() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let event = retrieved_signal(
        "secret_vote",
        "event",
        1.0,
        1,
        &["secret"],
        BTreeMap::new(),
        "payload_0 (source: e0018)",
    );
    let scene = retrieved_signal(
        "current_scene",
        "current",
        0.0,
        2,
        &[],
        BTreeMap::new(),
        "## Open Threads\n- [[secret_vote]] payload_1",
    );
    let character = retrieved_signal(
        "char_sera",
        "character",
        1.0,
        3,
        &[],
        BTreeMap::new(),
        "payload_2 (source: e0002)",
    );
    let contaminated_character = retrieved_signal(
        "char_mara",
        "character",
        1.0,
        4,
        &[],
        BTreeMap::new(),
        "payload_3 (source: e0018)",
    );

    let compiled = workspace
        .compile_mo_state(
            &[event, scene, character, contaminated_character],
            &[],
            100_000,
            &ConservativeTokenCounter,
        )
        .expect("compile");

    assert_eq!(
        compiled.audit.prompt_excluded_memory_ids,
        ["char_mara", "current_scene", "secret_vote"]
    );
    assert_eq!(compiled.audit.matched_rules, ["epistemic_secret_absence"]);
}

#[test]
fn epistemic_absence_does_not_assume_an_unspecified_character_is_a_witness() {
    let event = DmwSignal {
        id: "secret".to_owned(),
        kind: "event".to_owned(),
        weight: 1.0,
        touch_at: 1,
        tags: ["secret", "witness"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        relations: BTreeMap::from([(
            "characters".to_owned(),
            vec!["char_someone_else".to_owned()],
        )]),
        body: "payload_0 (source: e0009)".to_owned(),
    };
    let condition = RuleCondition {
        mode: Some("absence".to_owned()),
        event_tag: Some("secret".to_owned()),
        required_witness_tag: Some("witness".to_owned()),
        ..RuleCondition::default()
    };

    assert_eq!(
        unwitnessed_event_ids(&condition, &[event]),
        BTreeSet::from(["secret".to_owned()])
    );
}

#[test]
fn a_later_related_correction_clears_a_misconception() {
    let character = "char_target".to_owned();
    let misconception = DmwSignal {
        id: "misconception_1".to_owned(),
        kind: "event".to_owned(),
        weight: 1.0,
        touch_at: 10,
        tags: ["misconception"].into_iter().map(str::to_owned).collect(),
        relations: BTreeMap::from([("characters".to_owned(), vec![character.clone()])]),
        body: String::new(),
    };
    let correction = DmwSignal {
        id: "correction_1".to_owned(),
        kind: "event".to_owned(),
        weight: 1.0,
        touch_at: 11,
        tags: ["corrected"].into_iter().map(str::to_owned).collect(),
        relations: BTreeMap::from([("events".to_owned(), vec![misconception.id.clone()])]),
        body: String::new(),
    };
    let condition = RuleCondition {
        mode: Some("misconception".to_owned()),
        character_ref: Some(character),
        ..RuleCondition::default()
    };
    assert!(!epistemic_match(&condition, &[misconception, correction]));
}

#[test]
fn duplicate_priority_entries_are_rejected() {
    let mut priority = default_priority();
    priority.push("scene_constraint".to_owned());
    assert!(!valid_priority(&priority));
}

#[test]
fn explicit_rule_conflicts_are_resolved_by_dimension_priority() {
    let mut contract: StateContract =
        yaml_serde::from_str(DEFAULT_CONTRACT).expect("default contract");
    let emotional = contract
        .dimensions
        .get_mut("emotional_tone")
        .expect("emotional dimension");
    let lower_rule = emotional
        .rules
        .iter_mut()
        .find(|rule| rule.id == "tone_loss")
        .expect("loss rule");
    lower_rule.conflicts_with = vec!["stance_conflict".to_owned()];
    lower_rule.conflict_resolution = vec!["resolution_0".to_owned()];
    let directives = BTreeMap::from([
        (
            "relational_stance".to_owned(),
            vec![("stance_conflict".to_owned(), "directive_0".to_owned())],
        ),
        (
            "emotional_tone".to_owned(),
            vec![("tone_loss".to_owned(), "directive_1".to_owned())],
        ),
    ]);
    let mut audit = MoStateAudit::default();
    let ordered = order_directives(&contract, directives, &mut audit);
    let context = format_state_context(&ordered);
    assert!(context.contains("directive_0"));
    assert!(context.contains("resolution_0"));
    assert!(!context.contains("directive_1"));
    assert_eq!(audit.conflicts_resolved, 1);
}
