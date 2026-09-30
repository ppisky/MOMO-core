use super::*;

const CONVERSATION: &str = "00000000-0000-4000-8000-000000000001";
const OTHER: &str = "00000000-0000-4000-8000-000000000002";
const CHARACTER: &str = "00000000-0000-4000-8000-000000000003";

fn setup(bound: bool) -> (tempfile::TempDir, MemoryWorkspace) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = MemoryWorkspace::initialize(temp.path()).unwrap();
    let doc = MemoryDocument {
        metadata: Metadata {
            id: "event_amber_key".into(),
            kind: "event".into(),
            importance: Some(0.1),
            weight: Some(0.25),
            touch_at: 1,
            decay_at: Some(1),
            archived_at: None,
            relations: BTreeMap::new(),
            tags: vec![],
            aliases: vec!["amberkey".into()],
            injection_scope: None,
            injection_conversation_id: bound.then(|| CONVERSATION.into()),
            injection_character_id: bound.then(|| CHARACTER.into()),
            status: "active".into(),
        },
        body: "A forgotten amberkey lies on a shelf.".into(),
    };
    fs::write(temp.path().join("events/key.md"), doc.encode().unwrap()).unwrap();
    workspace.rebuild_index().unwrap();
    (temp, workspace)
}

fn tick(
    workspace: &MemoryWorkspace,
    conversation: &str,
    query: &str,
    now: i64,
) -> MaintenanceReport {
    let activity = LifecycleActivity {
        identity: None,
        conversation_id: conversation.into(),
        character_id: CHARACTER.into(),
        query: query.into(),
        settings: LifecycleSettings {
            decay_after_turns: 2,
            forget_after_turns: 3,
            decay_factor: 0.1,
            ..LifecycleSettings::default()
        },
    };
    let (plan, report) = workspace
        .prepare_lifecycle_activity(&activity, now)
        .unwrap();
    workspace.apply_prepared_commit(&plan).unwrap();
    // Durable recovery re-applies exact after-images, including the clock.
    workspace.apply_prepared_commit(&plan).unwrap();
    report
}

#[test]
fn activity_not_calendar_drives_archive_and_forgetting() {
    let (temp, workspace) = setup(true);
    assert!(
        tick(&workspace, CONVERSATION, "hello", 1)
            .decayed_ids
            .is_empty()
    );
    assert!(
        tick(&workspace, CONVERSATION, "hello", 9_000_000_000)
            .decayed_ids
            .is_empty()
    );
    let report = tick(&workspace, CONVERSATION, "hello", 2);
    assert_eq!(report.archived_ids, ["event_amber_key"]);
    assert!(temp.path().join("archive/event/key.md").exists());
    for _ in 0..10 {
        tick(&workspace, OTHER, "hello", 9_000_000_000);
    }
    assert!(temp.path().join("archive/event/key.md").exists());
    assert!(
        tick(&workspace, CONVERSATION, "hello", 3)
            .forgotten_ids
            .is_empty()
    );
    assert!(
        tick(&workspace, CONVERSATION, "hello", 4)
            .forgotten_ids
            .is_empty()
    );
    assert_eq!(
        tick(&workspace, CONVERSATION, "hello", 5).forgotten_ids,
        ["event_amber_key"]
    );
    assert!(!temp.path().join("archive/event/key.md").exists());
    assert!(
        workspace
            .load_tombstones()
            .unwrap()
            .contains_key("event_amber_key")
    );
}

#[test]
fn unknown_legacy_records_do_not_inherit_current_context_or_calendar_age() {
    let (temp, workspace) = setup(false);
    for _ in 0..20 {
        tick(&workspace, CONVERSATION, "hello", 9_000_000_000);
    }
    assert!(temp.path().join("events/key.md").exists());
    assert!(
        tick(&workspace, CONVERSATION, "amberkey", 1)
            .decayed_ids
            .is_empty()
    );
    assert!(
        tick(&workspace, CONVERSATION, "hello", 2)
            .decayed_ids
            .is_empty()
    );
    assert_eq!(
        tick(&workspace, CONVERSATION, "hello", 3).archived_ids,
        ["event_amber_key"]
    );
}

#[test]
fn shared_record_waits_for_all_contexts_and_substantive_hits_reset_age() {
    let (temp, workspace) = setup(false);
    tick(&workspace, CONVERSATION, "amberkey", 1);
    tick(&workspace, OTHER, "amberkey", 2);
    for _ in 0..8 {
        tick(&workspace, OTHER, "hello", 3);
    }
    assert!(temp.path().join("events/key.md").exists());
    tick(&workspace, CONVERSATION, "hello", 4);
    assert_eq!(
        tick(&workspace, CONVERSATION, "hello", 5).archived_ids,
        ["event_amber_key"]
    );
}

#[test]
fn open_commitments_and_explicit_references_are_protected() {
    let (temp, workspace) = setup(true);
    fs::write(temp.path().join("current/active_threads.md"), "---\nid: current_active_threads\ntype: current\nstatus: active\n---\n[[event_amber_key]]\n").unwrap();
    for _ in 0..10 {
        tick(&workspace, CONVERSATION, "hello", 100);
    }
    assert!(temp.path().join("events/key.md").exists());
    let mut doc = workspace.read("events/key.md").unwrap();
    doc.metadata.tags.push("pending".into());
    fs::write(temp.path().join("events/key.md"), doc.encode().unwrap()).unwrap();
    fs::write(
        temp.path().join("current/active_threads.md"),
        "---\nid: current_active_threads\ntype: current\nstatus: active\n---\nNothing pending.\n",
    )
    .unwrap();
    for _ in 0..10 {
        tick(&workspace, CONVERSATION, "hello", 100);
    }
    assert!(temp.path().join("events/key.md").exists());
}

#[test]
fn disabling_physical_forgetting_keeps_archive_and_explicit_restore_resets_age() {
    let (temp, workspace) = setup(true);
    for n in 0..12 {
        let activity = LifecycleActivity {
            identity: None,
            conversation_id: CONVERSATION.into(),
            character_id: CHARACTER.into(),
            query: "hello".into(),
            settings: LifecycleSettings {
                decay_after_turns: 1,
                forget_after_turns: 1,
                decay_factor: 0.1,
                auto_forget: false,
                ..Default::default()
            },
        };
        let (plan, report) = workspace.prepare_lifecycle_activity(&activity, n).unwrap();
        workspace.apply_prepared_commit(&plan).unwrap();
        assert!(report.forgotten_ids.is_empty());
    }
    assert!(temp.path().join("archive/event/key.md").exists());
    workspace
        .restore_archived_authorized("event_amber_key")
        .unwrap();
    assert!(
        tick(&workspace, CONVERSATION, "hello", 100)
            .decayed_ids
            .is_empty()
    );
    assert!(temp.path().join("events/key.md").exists());
    let before = workspace.export_snapshot().unwrap();
    let (plan, _) = workspace
        .prepare_lifecycle_activity(
            &LifecycleActivity {
                identity: None,
                conversation_id: CONVERSATION.into(),
                character_id: CHARACTER.into(),
                query: "hello".into(),
                settings: LifecycleSettings {
                    enabled: false,
                    ..Default::default()
                },
            },
            9_000_000_000,
        )
        .unwrap();
    workspace.apply_prepared_commit(&plan).unwrap();
    assert_eq!(workspace.export_snapshot().unwrap().files, before.files);
}

#[test]
fn relevant_fact_without_presentation_budget_does_not_age_as_unused() {
    let (_temp, workspace) = setup(true);
    tick(&workspace, CONVERSATION, "hello", 1);
    let scope = MemoryRetrievalScope {
        current: Vec::new(),
        eligible_ids: ["event_amber_key".into()].into(),
    };
    for now in 2..8 {
        assert!(
            workspace
                .retrieve_in_scope("amberkey", 1, &ConservativeTokenCounter, Some(&scope))
                .unwrap()
                .is_empty()
        );
        let report = tick(&workspace, CONVERSATION, "amberkey", now);
        assert!(report.decayed_ids.is_empty());
        assert!(report.archived_ids.is_empty());
    }
    assert_eq!(
        workspace.read("events/key.md").unwrap().metadata.weight,
        Some(0.25)
    );
}

#[test]
fn completed_event_keeps_character_significance_while_low_value_details_age() {
    let (_temp, workspace) = setup(true);
    let mut significant = workspace.read("events/key.md").unwrap();
    significant.metadata.id = "meaningful_key".into();
    significant.metadata.importance = Some(0.9);
    significant.metadata.weight = Some(0.9);
    significant.metadata.tags = vec!["completed".into()];
    significant.body =
        "The task is completed. The gift remains deeply significant to this character.".into();
    fs::write(
        workspace.root().join("events/meaningful.md"),
        significant.encode().unwrap(),
    )
    .unwrap();
    workspace.rebuild_index().unwrap();
    for now in 1..=7 {
        tick(&workspace, CONVERSATION, "hello", now);
    }
    assert_eq!(
        workspace
            .read("events/meaningful.md")
            .unwrap()
            .metadata
            .weight,
        Some(0.9)
    );
    assert!(workspace.read("events/key.md").is_err());
}

#[test]
fn mentioning_archived_record_does_not_silently_restore_or_refresh_it() {
    let (_temp, workspace) = setup(true);
    for now in 1..=3 {
        tick(&workspace, CONVERSATION, "hello", now);
    }
    assert!(workspace.root().join("archive/event/key.md").exists());
    for now in 4..=6 {
        tick(&workspace, CONVERSATION, "amberkey", now);
    }
    assert!(!workspace.root().join("archive/event/key.md").exists());
}
