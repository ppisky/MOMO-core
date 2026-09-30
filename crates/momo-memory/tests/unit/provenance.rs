use super::*;

fn context() -> (uuid::Uuid, RevisionContext) {
    let space = uuid::Uuid::now_v7();
    let conversation_id = uuid::Uuid::now_v7();
    let character_id = uuid::Uuid::now_v7();
    let identity = MemoryIdentity {
        personal_space_id: space,
        conversation_id,
        character_id,
        continuity_id: conversation_id,
        function_id: None,
    };
    let mut context = RevisionContext::manual("automatic_batch");
    context.proposer = "dmw_distiller".into();
    context.identity = Some(identity.clone());
    context.evidence.push(Evidence {
        id: "response1".into(),
        kind: EvidenceKind::Conversation,
        original_space_id: space,
        personal_space_id: space,
        conversation_id: Some(conversation_id),
        character_id: Some(character_id),
        message_ids: vec![uuid::Uuid::now_v7()],
        configuration: BTreeMap::new(),
    });
    (space, context)
}

fn patch() -> &'static str {
    "patches:\n  - target_file: events/a.md\n    evidence_refs: [response1]\n    operations:\n      - type: create\n        frontmatter:\n          id: fact_a\n          type: event\n          importance: 0.3\n          weight: 1.0\n          decay_at: 0\n          status: active\n        content: |\n          # Event\n          ## Facts\n          A promised to return.\n"
}

#[test]
fn revision_and_evidence_survive_partial_commit_replay_and_import_without_grants() {
    let temp = tempfile::tempdir().unwrap();
    let ws = MemoryWorkspace::initialize(temp.path()).unwrap();
    let (space, mut context) = context();
    context.target_evidence = patch_evidence(patch(), &context.evidence).unwrap();
    let plan = ws
        .trace_commit(
            ws.prepare_patch_commit(patch()).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    let event = plan.files.iter().find(|f| f.path == "events/a.md").unwrap();
    fs::write(ws.root().join(&event.path), event.after.as_ref().unwrap()).unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    let ledger = ws.provenance(false).unwrap();
    let history = &ledger.records["fact_a"];
    assert_eq!(history.revisions.len(), 1);
    assert_eq!(
        history.evidence["response1"].character_id,
        context.identity.as_ref().map(|i| i.character_id)
    );
    assert!(history.exact_evidence);
    assert!(
        ws.record_revision_matches(false, "fact_a", "events/a.md")
            .unwrap()
    );
    let path = ws.root().join("events/a.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("promised", "refused")).unwrap();
    assert!(
        !ws.record_revision_matches(false, "fact_a", "events/a.md")
            .unwrap()
    );
    let imported = imported_ledger(
        &serde_json::to_string(&ledger).unwrap(),
        uuid::Uuid::now_v7(),
    )
    .unwrap();
    let imported = ProvenanceLedger::parse(std::str::from_utf8(&imported).unwrap()).unwrap();
    assert!(imported.records["fact_a"].policy.is_none());
    assert_eq!(imported.records["fact_a"].evidence, history.evidence);
    assert_eq!(imported.records["fact_a"].revisions.len(), 2);
}

#[test]
fn automatic_write_cannot_mutate_other_identity_and_fabricated_evidence_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let ws = MemoryWorkspace::initialize(temp.path()).unwrap();
    let (space, context) = context();
    let plan = ws
        .trace_commit(
            ws.prepare_patch_commit(patch()).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    let mut other = context.clone();
    other.identity.as_mut().unwrap().character_id = uuid::Uuid::now_v7();
    let edit = "patches:\n  - target_file: events/a.md\n    operations:\n      - type: append\n        section: Facts\n        content: B was there.\n";
    assert!(
        ws.trace_commit(ws.prepare_patch_commit(edit).unwrap(), Some(space), &other)
            .is_err()
    );
    assert!(patch_evidence(&patch().replace("response1", "forged"), &context.evidence).is_err());
}

#[test]
fn first_legacy_policy_pins_content_against_untracked_edits() {
    let temp = tempfile::tempdir().unwrap();
    let ws = MemoryWorkspace::initialize(temp.path()).unwrap();
    let (space, context) = context();
    let plan = ws
        .trace_commit(
            ws.prepare_patch_commit(patch()).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    let policy = ws.provenance(false).unwrap().records["fact_a"]
        .policy
        .clone()
        .unwrap();
    fs::write(
        ws.root().join(DMW_LEDGER),
        serde_json::to_vec(&ProvenanceLedger::default()).unwrap(),
    )
    .unwrap();
    ws.set_record_policy(false, "fact_a", policy).unwrap();
    assert!(
        ws.record_revision_matches(false, "fact_a", "events/a.md")
            .unwrap()
    );
    let path = ws.root().join("events/a.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("promised", "refused")).unwrap();
    assert!(
        !ws.record_revision_matches(false, "fact_a", "events/a.md")
            .unwrap()
    );
}

#[test]
fn parallel_scenes_are_isolated_and_same_continuity_retains_scene() {
    let temp = tempfile::tempdir().unwrap();
    let ws = MemoryWorkspace::initialize(temp.path()).unwrap();
    let (space, context) = context();
    let update = "patches:\n  - target_file: current/scene.md\n    operations:\n      - type: append\n        section: Current Scene\n        content: Harbor\n";
    // Replace the whole current document via a prepared mutation so this test
    // is independent of the scene Markdown's section vocabulary.
    let mut doc = ws.read("current/scene.md").unwrap();
    doc.body = "# Current Scene\n\nHarbor".into();
    let plan = PreparedMemoryCommit::prepare(
        ws.root(),
        &[FileMutation::Write {
            path: ws.root().join("current/scene.md"),
            content: doc.encode().unwrap().into_bytes(),
        }],
    )
    .unwrap();
    let plan = ws.trace_commit(plan, Some(space), &context).unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    assert!(!ws.read("current/scene.md").unwrap().body.contains("Harbor"));
    let a = context.identity.unwrap();
    assert!(ws.scoped_current(&a).unwrap()[0].body.contains("Harbor"));
    let mut b = a.clone();
    b.conversation_id = uuid::Uuid::now_v7();
    b.continuity_id = b.conversation_id;
    assert!(ws.scoped_current(&b).unwrap().is_empty());
    b.continuity_id = a.continuity_id;
    assert_eq!(ws.scoped_current(&b).unwrap().len(), 1);
    let _ = update;
}

#[test]
fn unknown_provenance_versions_fail_before_snapshot_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let ws = MemoryWorkspace::initialize(temp.path()).unwrap();
    let before = ws.export_snapshot().unwrap();
    let mut snapshot = before.clone();
    snapshot.files.insert(
        DMW_LEDGER.into(),
        "{\"schema\":\"future\",\"records\":{},\"scenes\":{}}".into(),
    );
    assert!(ws.import_snapshot(&snapshot).is_err());
    assert_eq!(ws.export_snapshot().unwrap(), before);
}

#[test]
fn qualified_parent_reference_retains_evidence_from_the_correct_module() {
    let temp = tempfile::tempdir().unwrap();
    let ws = MemoryWorkspace::initialize(temp.path()).unwrap();
    let (space, mut context) = context();
    let plan = ws
        .trace_commit(
            ws.prepare_patch_commit(patch()).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    let mut graph = ws.provenance(false).unwrap();
    let parent = graph.records.get_mut("fact_a").unwrap();
    let mut evidence = parent.evidence.remove("response1").unwrap();
    evidence.id = "graph-source".into();
    parent.evidence.insert(evidence.id.clone(), evidence);
    std::fs::create_dir_all(ws.root().join("rules")).unwrap();
    std::fs::write(
        ws.root().join(NSG_LEDGER),
        serde_json::to_vec(&graph).unwrap(),
    )
    .unwrap();
    let child = patch()
        .replace("events/a.md", "events/b.md")
        .replace("fact_a", "fact_b");
    context.parent_records = vec![format!("{space}/nsg/fact_a")];
    context.target_evidence = patch_evidence(&child, &context.evidence).unwrap();
    let plan = ws
        .trace_commit(
            ws.prepare_patch_commit(&child).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    let ledger = ws.provenance(false).unwrap();
    let child = &ledger.records["fact_b"];
    assert!(child.evidence.contains_key("graph-source"));
    assert!(!child.exact_evidence);
    assert_eq!(
        child.revisions[0].context.configuration[&format!("parent:{space}/nsg/fact_a")],
        "1"
    );
}
