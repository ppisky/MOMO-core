use super::*;

const TEST_BODY: &str = "# Test\n\n## 关键变化\n\n旧内容。\n";

#[test]
fn body_only_identifiers_are_searchable_and_removed_when_corrected() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    write_fixture(
        root.path(),
        &event_fixture("events/owner.md", "owner", &["ownership"]),
    );
    let mut document = workspace.read("events/owner.md").expect("read");
    document.body = "# Mara\n\nMara owns instrument S17.".to_owned();
    fs::write(
        root.path().join("events/owner.md"),
        document.encode().expect("encode"),
    )
    .expect("write");
    let query = |q| {
        workspace
            .retrieve(q, 4096, &ConservativeTokenCounter)
            .expect("retrieve")
    };
    assert!(query("S17").iter().any(|r| r.id == "owner"));
    assert!(!query("S71").iter().any(|r| r.id == "owner"));
    assert!(!query("S170").iter().any(|r| r.id == "owner"));
    document.body = "# Mara\n\nCorrection: Mara owns instrument S71.".to_owned();
    fs::write(
        root.path().join("events/owner.md"),
        document.encode().expect("encode"),
    )
    .expect("write");
    assert!(!query("S17").iter().any(|r| r.id == "owner"));
    assert!(query("S71").iter().any(|r| r.id == "owner"));
}

#[test]
fn structured_tags_and_ids_do_not_match_shared_prefix_tokens() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    for index in 0..3 {
        let id = format!("event_{index:05}");
        let tag = format!("unique_{index:05}");
        let relative = format!("events/{id}.md");
        let tags = [tag.as_str()];
        write_fixture(root.path(), &event_fixture(&relative, &id, &tags));
    }
    workspace.rebuild_index().expect("index");

    let retrieved = workspace
        .retrieve("unique_00002", 4096, &ConservativeTokenCounter)
        .expect("retrieve");
    let event_ids = retrieved
        .iter()
        .filter(|item| item.path.starts_with("events"))
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(event_ids, vec!["event_00002"]);
}

#[test]
fn body_terms_retrieve_specific_english_and_chinese_facts() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    write_fixture(
        root.path(),
        &event_fixture("events/english.md", "english_record", &[]),
    );
    write_fixture(
        root.path(),
        &event_fixture("events/chinese.md", "chinese_record", &[]),
    );
    let mut english = workspace.read("events/english.md").expect("english");
    english.body =
        "# Record Twenty Four\n\nMara's silver astrolabe is stored in Glass-Archive-52636."
            .to_owned();
    fs::write(
        root.path().join("events/english.md"),
        english.encode().expect("encode english"),
    )
    .expect("write english");
    let mut chinese = workspace.read("events/chinese.md").expect("chinese");
    chinese.body = "# 第二十四条记录\n\n玛拉的银色星盘存放在Glass-Archive-52636。".to_owned();
    fs::write(
        root.path().join("events/chinese.md"),
        chinese.encode().expect("encode chinese"),
    )
    .expect("write chinese");

    let english_results = workspace
        .retrieve(
            "Where is Mara's silver astrolabe?",
            4096,
            &ConservativeTokenCounter,
        )
        .expect("retrieve english");
    assert!(
        english_results
            .iter()
            .any(|item| item.id == "english_record")
    );
    let chinese_results = workspace
        .retrieve(
            "现在应该去哪里找玛拉的银色星盘？",
            4096,
            &ConservativeTokenCounter,
        )
        .expect("retrieve chinese");
    assert!(
        chinese_results
            .iter()
            .any(|item| item.id == "chinese_record")
    );
}

#[test]
fn cross_language_fallback_is_bounded_and_preserves_same_language_negatives() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    for (index, weight) in [(1, 0.9), (2, 0.8), (3, 0.7)] {
        let id = format!("english_record_{index}");
        let relative = format!("events/{id}.md");
        write_fixture(root.path(), &event_fixture(&relative, &id, &[]));
        let mut document = workspace.read(&relative).expect("read fixture");
        document.metadata.weight = Some(weight);
        document.body = format!("# Archive Record {index}\n\nMara stored an instrument safely.");
        fs::write(
            root.path().join(&relative),
            document.encode().expect("encode fixture"),
        )
        .expect("write fixture");
    }
    workspace.rebuild_index().expect("index");

    let cross_language = workspace
        .retrieve("玛拉把星盘存放在哪里？", 4096, &ConservativeTokenCounter)
        .expect("cross-language retrieval");
    let ids = cross_language
        .iter()
        .filter(|item| item.path.starts_with("events"))
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["english_record_1", "english_record_2"]);

    let same_language_negative = workspace
        .retrieve("Where is Ivo's telescope?", 4096, &ConservativeTokenCounter)
        .expect("same-language negative retrieval");
    assert!(
        same_language_negative
            .iter()
            .all(|item| !item.path.starts_with("events"))
    );
}

#[test]
fn retrieval_prefers_specific_entity_binding_over_popular_partial_matches() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let mut distractor = event_fixture("events/decoy.md", "decoy", &["silver", "silver", "silver"]);
    distractor.importance = 1.0;
    write_fixture(root.path(), &distractor);
    let mut target = event_fixture("events/target.md", "target", &["Mara silver astrolabe"]);
    target.importance = 0.1;
    write_fixture(root.path(), &target);
    workspace.rebuild_index().expect("index");
    let retrieved = workspace
        .retrieve("Mara silver astrolabe", 4096, &ConservativeTokenCounter)
        .expect("retrieve");
    let ids: Vec<_> = retrieved
        .iter()
        .filter(|r| r.id == "target" || r.id == "decoy")
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(ids, vec!["target", "decoy"]);
}

#[test]
fn initialize_upgrades_an_empty_legacy_scene_without_overwriting_content() {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let mut scene = workspace.read("current/scene.md").expect("scene");
    scene.body = "# 当前场景\n\n".to_owned();
    fs::write(
        workspace.root().join("current/scene.md"),
        scene.encode().expect("encode"),
    )
    .expect("write legacy scene");

    let workspace = MemoryWorkspace::initialize(root.path()).expect("reinitialize");
    let upgraded = workspace.read("current/scene.md").expect("upgraded scene");
    assert!(upgraded.body.contains("## Scene ID\nscene_initial"));
    assert!(upgraded.body.contains("## Source References"));

    let mut customized = upgraded;
    customized.body = "# 当前场景\n\n用户自己写的场景。\n".to_owned();
    fs::write(
        workspace.root().join("current/scene.md"),
        customized.encode().expect("encode customized"),
    )
    .expect("write customized scene");
    MemoryWorkspace::initialize(root.path()).expect("reinitialize customized");
    assert_eq!(
        workspace.read("current/scene.md").expect("preserved").body,
        customized.body
    );
}

#[test]
fn mo_state_source_identity_ignores_audit_but_tracks_scene_and_graph() {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        workspace.root(),
        &event_fixture("events/retrieval.md", "event_retrieval", &["retrievable"]),
    );
    workspace.rebuild_index().expect("rebuild index");
    let initial = workspace
        .mo_state_source_fingerprint()
        .expect("initial fingerprint");
    workspace
        .retrieve("retrievable", 2_048, &ConservativeTokenCounter)
        .expect("retrieve");
    let after_retrieval = workspace
        .mo_state_source_fingerprint()
        .expect("retrieval fingerprint");
    assert_eq!(initial, after_retrieval);
    fs::write(
        workspace.root().join("audit/memory.log"),
        "volatile read audit\n",
    )
    .expect("audit");
    let after_audit = workspace
        .mo_state_source_fingerprint()
        .expect("audit fingerprint");
    assert_eq!(initial, after_audit);

    let mut scene = workspace.read("current/scene.md").expect("scene");
    scene.body = scene.body.replace("## Focus\n", "## Focus\n在客厅交谈\n");
    fs::write(
        workspace.root().join("current/scene.md"),
        scene.encode().expect("encode"),
    )
    .expect("write scene");
    let changed = workspace
        .mo_state_source_fingerprint()
        .expect("changed fingerprint");
    assert_ne!(changed.dmw, initial.dmw);
    assert_ne!(changed.scene, initial.scene);
    assert_eq!(changed.nsg, initial.nsg);
    assert_eq!(changed.scene_snapshot.focus.as_deref(), Some("在客厅交谈"));
}

struct Fixture<'a> {
    relative: &'a str,
    id: &'a str,
    kind: &'a str,
    importance: f64,
    weight: f64,
    touch_at: i64,
    decay_at: i64,
    tags: &'a [&'a str],
    status: &'a str,
}

fn write_fixture(root: &Path, fixture: &Fixture<'_>) {
    let document = MemoryDocument {
        metadata: Metadata {
            id: fixture.id.to_owned(),
            kind: fixture.kind.to_owned(),
            importance: Some(fixture.importance),
            weight: Some(fixture.weight),
            touch_at: fixture.touch_at,
            decay_at: Some(fixture.decay_at),
            archived_at: (fixture.status == "archived").then_some(fixture.touch_at),
            relations: BTreeMap::new(),
            injection_scope: None,
            injection_conversation_id: None,
            injection_character_id: None,
            tags: fixture
                .tags
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            aliases: Vec::new(),
            status: fixture.status.to_owned(),
        },
        body: TEST_BODY.to_owned(),
    };
    atomic_write(
        &root.join(fixture.relative),
        &document.encode().expect("encode fixture"),
    )
    .expect("write fixture");
}

fn event_fixture<'a>(relative: &'a str, id: &'a str, tags: &'a [&'a str]) -> Fixture<'a> {
    Fixture {
        relative,
        id,
        kind: "event",
        importance: 0.5,
        weight: 0.5,
        touch_at: 1,
        decay_at: 1,
        tags,
        status: "active",
    }
}

fn write_access(root: &Path, read: &[&str], write: &[&str], allow_archive_restore: bool) {
    let access = AccessConfig {
        version: 1,
        read: read.iter().map(|value| (*value).to_owned()).collect(),
        write: write.iter().map(|value| (*value).to_owned()).collect(),
        allow_archive_restore,
    };
    atomic_write(
        &root.join("config/access.yaml"),
        &yaml_serde::to_string(&access).expect("encode access"),
    )
    .expect("write access");
}

#[test]
fn initializes_required_workspace() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let scene = workspace.read("current/scene.md").expect("scene");
    assert_eq!(scene.metadata.kind, "current");
    assert!(root.path().join("indexes/memory_index.yaml").exists());
}

#[test]
fn snapshots_round_trip_workspace_files() {
    let source_root = tempfile::tempdir().expect("source");
    let source = MemoryWorkspace::initialize(source_root.path()).expect("source workspace");
    fs::write(source.root().join("current/scene.md"), TEST_BODY).expect("scene");
    write_fixture(
        source.root(),
        &event_fixture("events/safe.md", "event_safe", &["safe"]),
    );

    let snapshot = source.export_snapshot().expect("snapshot");
    assert_eq!(snapshot.version, 1);
    assert!(snapshot.files.contains_key("current/scene.md"));
    assert!(snapshot.files.contains_key("events/safe.md"));

    let destination_root = tempfile::tempdir().expect("destination");
    let destination =
        MemoryWorkspace::initialize(destination_root.path()).expect("destination workspace");
    destination
        .import_snapshot(&snapshot)
        .expect("import snapshot");

    assert_eq!(
        fs::read_to_string(destination.root().join("current/scene.md")).expect("scene"),
        TEST_BODY
    );
    assert!(
        fs::read_to_string(destination.root().join("events/safe.md"))
            .expect("event")
            .contains("event_safe")
    );
}

#[test]
fn memory_partition_import_does_not_import_semantic_web_files() {
    let source_root = tempfile::tempdir().expect("source root");
    let source = MemoryWorkspace::initialize(source_root.path()).expect("source workspace");
    fs::write(source.root().join("current/scene.md"), "# Local memory\n").expect("write memory");
    fs::write(source.root().join("lore/world.nsg"), "# Semantic web\n").expect("write graph");
    let memory_snapshot = source
        .export_memory_partition_snapshot()
        .expect("memory snapshot");
    let graph_snapshot = source
        .export_semantic_graph_partition_snapshot()
        .expect("semantic web snapshot");

    let fresh_root = tempfile::tempdir().expect("fresh root");
    let fresh = MemoryWorkspace::initialize(fresh_root.path()).expect("fresh workspace");
    fresh
        .import_memory_partition_snapshot(&memory_snapshot)
        .expect("import memory partition snapshot");
    assert!(
        fs::read_to_string(fresh.root().join("current/scene.md"))
            .expect("fresh memory")
            .contains("Local memory")
    );
    assert!(!fresh.root().join("lore/world.nsg").exists());

    let graph_root = tempfile::tempdir().expect("graph root");
    let graph = MemoryWorkspace::initialize(graph_root.path()).expect("graph workspace");
    graph
        .import_semantic_graph_partition_snapshot(&graph_snapshot)
        .expect("import semantic web partition snapshot");
    assert!(
        !fs::read_to_string(graph.root().join("current/scene.md"))
            .expect("default scene")
            .contains("Local memory")
    );
    assert!(
        fs::read_to_string(graph.root().join("lore/world.nsg"))
            .expect("semantic web")
            .contains("Semantic web")
    );
}

#[test]
fn snapshot_import_rejects_unsafe_paths() {
    let root = tempfile::tempdir().expect("root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
    let snapshot = MemorySnapshot {
        version: 1,
        files: BTreeMap::from([("../escape.md".to_owned(), "bad".to_owned())]),
    };
    assert!(matches!(
        workspace.import_snapshot(&snapshot),
        Err(MemoryError::UnsafePath(_))
    ));
}

#[test]
fn applies_append_and_frontmatter_update() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/test.md", "event_test", &["test"]),
    );
    workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/test.md
    operations:
      - type: append
        section: 关键变化
        content: 新内容。
      - type: update_frontmatter
        fields:
          weight: 0.9
          tags: [test, updated]
"#,
        )
        .expect("apply patch");
    let document = workspace.read("events/test.md").expect("updated");
    assert_eq!(document.metadata.weight, Some(0.9));
    assert!(document.body.contains("旧内容。\n新内容。"));
    let index = workspace.load_index().expect("index");
    assert_eq!(
        index.entries["event_test"].tags,
        vec!["test".to_owned(), "updated".to_owned()]
    );
}

#[test]
fn append_rejects_a_missing_section() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/test.md", "event_test", &["test"]),
    );

    let error = workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/test.md
    operations:
      - type: append
        section: 新章节
        content: 新内容。
"#,
        )
        .expect_err("append must target an existing section");
    assert!(matches!(error, MemoryError::MissingSection(_)));
}

#[test]
fn replaces_complete_document_body_without_requiring_a_section() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/test.md", "event_test", &["test"]),
    );

    workspace
        .replace_document_body("event_test", "# 艾琳\n\n角色正文。")
        .expect("replace body");
    let document = workspace
        .read_document_by_id("event_test")
        .expect("read updated document");
    assert_eq!(document.body, "# 艾琳\n\n角色正文。\n");
}

#[test]
fn patch_rejects_unknown_operation_fields() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let error = workspace
        .validate_patch(
            r#"
patches:
  - target_file: events/invalid.md
    operations:
      - type: create
        title: Invalid
        frontmatter:
          id: event_invalid
          type: event
          importance: 0.5
          weight: 0.5
          decay_at: 1
          relations: {}
          tags: []
          status: active
        content: Invalid.
"#,
        )
        .expect_err("unknown fields must be rejected");
    assert!(matches!(error, MemoryError::Yaml(_)));
}

#[test]
fn repairs_latex_backslashes_in_double_quoted_content() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/test.md", "event_test", &["test"]),
    );
    workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/test.md
    operations:
      - type: append
        section: 关键变化
        content: "保留公式 \text{哈基米} \quad x"
"#,
        )
        .expect("unknown quoted escapes should be repaired");
    let document = workspace.read("events/test.md").expect("updated");
    assert!(document.body.contains(r"\text{哈基米} \quad x"));
}

#[test]
fn empty_patch_is_a_valid_noop() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let index_before = fs::read(workspace.index_path()).expect("index");

    workspace.apply_patch("patches: []").expect("empty patch");

    assert_eq!(
        fs::read(workspace.index_path()).expect("index after"),
        index_before
    );
}

#[test]
fn retrieve_updates_touch_at_only_after_selecting_memory() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/retrieved.md", "event_retrieved", &["needle"]),
    );
    workspace.rebuild_index().expect("rebuild index");

    let retrieved = workspace
        .retrieve("needle", usize::MAX, &ConservativeTokenCounter)
        .expect("retrieve");

    assert_eq!(
        retrieved
            .iter()
            .map(|memory| memory.id.as_str())
            .collect::<Vec<_>>(),
        vec!["current_scene", "current_active_threads", "event_retrieved"]
    );
    let updated = workspace.read("events/retrieved.md").expect("updated");
    assert!(updated.metadata.touch_at > 1);
    assert_eq!(updated.metadata.weight, Some(0.5));
}

#[test]
fn whole_patch_is_prevalidated_before_any_write() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/first.md", "event_first", &["first"]),
    );
    write_fixture(
        root.path(),
        &event_fixture("events/second.md", "event_second", &["second"]),
    );
    workspace.rebuild_index().expect("rebuild index");
    let original_first = fs::read(root.path().join("events/first.md")).expect("first");
    let original_index = fs::read(workspace.index_path()).expect("index");

    let error = workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/first.md
    operations:
      - type: append
        section: 关键变化
        content: 不应提交。
  - target_file: events/second.md
    operations:
      - type: replace
        section: 不存在
        content: 无效。
"#,
        )
        .expect_err("whole patch must fail");

    assert!(matches!(error, MemoryError::MissingSection(_)));
    assert_eq!(
        fs::read(root.path().join("events/first.md")).expect("first"),
        original_first
    );
    assert_eq!(
        fs::read(workspace.index_path()).expect("index"),
        original_index
    );
}

#[test]
fn validate_patch_runs_full_validation_without_writing() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/approval.md", "event_approval", &["approval"]),
    );
    workspace.rebuild_index().expect("rebuild index");
    let document_path = root.path().join("events/approval.md");
    let original_document = fs::read(&document_path).expect("document");
    let original_index = fs::read(workspace.index_path()).expect("index");
    let patch = r#"
patches:
  - target_file: events/approval.md
    operations:
      - type: append
        section: 关键变化
        content: 审批后才可写入。
      - type: update_frontmatter
        fields:
          weight: 0.8
"#;

    workspace.validate_patch(patch).expect("validate");
    assert_eq!(
        fs::read(&document_path).expect("document"),
        original_document
    );
    assert_eq!(
        fs::read(workspace.index_path()).expect("index"),
        original_index
    );

    workspace.apply_patch(patch).expect("apply");
    assert_ne!(fs::read(document_path).expect("updated"), original_document);
}

#[test]
fn transaction_failure_rolls_back_document_and_index() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/rollback.md", "event_rollback", &["before"]),
    );
    workspace.rebuild_index().expect("rebuild index");
    let document_path = root.path().join("events/rollback.md");
    let index_path = workspace.index_path();
    let original_document = fs::read(&document_path).expect("document");
    let original_index = fs::read(&index_path).expect("index");
    let mut failed_once = false;
    let mut writer = |path: &Path, content: &[u8]| {
        if path == index_path && !failed_once {
            failed_once = true;
            return Err(io::Error::other("injected index failure").into());
        }
        atomic_write_bytes(path, content)
    };

    workspace
        .apply_patch_with_writer(
            r#"
patches:
  - target_file: events/rollback.md
    operations:
      - type: update_frontmatter
        fields:
          tags: [after]
"#,
            &mut writer,
        )
        .expect_err("injected write failure");

    assert_eq!(
        fs::read(document_path).expect("document"),
        original_document
    );
    assert_eq!(fs::read(index_path).expect("index"), original_index);
}

#[test]
fn failed_index_write_removes_a_newly_created_document() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let index_path = workspace.index_path();
    let original_index = fs::read(&index_path).expect("index");
    let mut failed_once = false;
    let mut writer = |path: &Path, content: &[u8]| {
        if path == index_path && !failed_once {
            failed_once = true;
            return Err(io::Error::other("injected index failure").into());
        }
        atomic_write_bytes(path, content)
    };

    workspace
        .apply_patch_with_writer(
            r#"
patches:
  - target_file: events/new.md
    operations:
      - type: create
        frontmatter:
          id: event_new
          type: event
          importance: 0.5
          weight: 0.5
          decay_at: 1
          relations: {}
          tags: [new]
          status: active
        content: New memory.
"#,
            &mut writer,
        )
        .expect_err("injected write failure");

    assert!(!root.path().join("events/new.md").exists());
    assert_eq!(fs::read(index_path).expect("index"), original_index);
}

#[test]
fn create_and_tag_updates_are_immediately_searchable() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/created.md
    operations:
      - type: create
        frontmatter:
          id: event_record
          type: event
          importance: 0.7
          weight: 0.8
          decay_at: 1
          relations: {}
          tags: [created]
          status: active
        content: |-
          # Recorded Memory

          Recorded memory.
"#,
        )
        .expect("create");
    workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/created.md
    operations:
      - type: update_frontmatter
        fields:
          tags: [renamed]
"#,
        )
        .expect("update tags");

    assert!(
        workspace
            .retrieve("created", usize::MAX, &ConservativeTokenCounter)
            .expect("old tag query")
            .iter()
            .all(|memory| memory.id != "event_record")
    );
    let retrieved = workspace
        .retrieve("renamed", usize::MAX, &ConservativeTokenCounter)
        .expect("new tag query");
    assert!(retrieved.iter().any(|memory| memory.id == "event_record"));
    let by_title = workspace
        .retrieve("recorded memory", usize::MAX, &ConservativeTokenCounter)
        .expect("title alias query");
    assert!(by_title.iter().any(|memory| memory.id == "event_record"));
}

#[test]
fn multiword_title_alias_is_searchable_by_a_specific_term() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    workspace
        .apply_patch(
            r#"
patches:
  - target_file: world/astrolabe_provenance.md
    operations:
      - type: create
        frontmatter:
          id: world_navigation_record
          type: world
          importance: 0.8
          weight: 0.8
          decay_at: 2000000000
          relations: {}
          tags: []
          aliases: []
          status: active
        content: |-
          # Astrolabe Provenance and Winter Storage

          ## Confirmed Facts

          - Mara's silver astrolabe is stored in the Glass Archive.
"#,
        )
        .expect("create memory");

    let retrieved = workspace
        .retrieve(
            "Where is Mara's silver astrolabe?",
            576,
            &ConservativeTokenCounter,
        )
        .expect("retrieve");
    let memory = retrieved
        .iter()
        .find(|item| item.id == "world_navigation_record")
        .expect("specific title term retrieves cropped document");
    assert!(memory.body.contains("Glass Archive"));
}

#[test]
fn rebuild_index_recovers_from_corrupt_index_and_removes_stale_entries() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/rebuild.md", "event_rebuild", &["rebuilt"]),
    );
    atomic_write(&workspace.index_path(), "not: [valid").expect("corrupt index");

    assert_eq!(workspace.rebuild_index().expect("rebuild"), 1);
    let index = workspace.load_index().expect("valid index");
    assert_eq!(index.entries.len(), 1);
    assert_eq!(index.entries["event_rebuild"].path, "events/rebuild.md");
}

#[test]
fn maintenance_decays_archives_and_explicitly_restores_memory() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let now = DECAY_INTERVAL_SECONDS * 3;
    write_fixture(
        root.path(),
        &Fixture {
            relative: "events/fading.md",
            id: "event_fading",
            kind: "event",
            importance: 0.5,
            weight: 0.21,
            touch_at: now - DECAY_INTERVAL_SECONDS - 1,
            decay_at: now - DECAY_INTERVAL_SECONDS,
            tags: &["fading"],
            status: "active",
        },
    );
    write_fixture(
        root.path(),
        &Fixture {
            relative: "events/core.md",
            id: "event_core",
            kind: "event",
            importance: 0.8,
            weight: 0.1,
            touch_at: now - DECAY_INTERVAL_SECONDS - 1,
            decay_at: now - DECAY_INTERVAL_SECONDS,
            tags: &["core"],
            status: "active",
        },
    );
    workspace.rebuild_index().expect("rebuild index");

    let report = workspace.run_maintenance_at(now).expect("maintenance");

    assert_eq!(report.decayed_ids, vec!["event_core", "event_fading"]);
    assert_eq!(report.archived_ids, vec!["event_fading"]);
    assert!(!root.path().join("events/fading.md").exists());
    let archived = workspace
        .read("archive/event/fading.md")
        .expect("archived document");
    assert_eq!(archived.metadata.status, "archived");
    assert!((archived.metadata.weight.expect("weight") - 0.189).abs() < f64::EPSILON);
    let core = workspace.read("events/core.md").expect("core memory");
    assert_eq!(core.metadata.status, "active");
    assert!((core.metadata.weight.expect("weight") - 0.09).abs() < f64::EPSILON);
    assert_eq!(
        workspace.load_index().expect("index").entries["event_fading"].path,
        "archive/event/fading.md"
    );

    assert!(matches!(
        workspace.restore_archived("event_fading"),
        Err(MemoryError::AccessDenied {
            operation: "archive_restore",
            ..
        })
    ));
    let restored_path = workspace
        .restore_archived_authorized("event_fading")
        .expect("explicitly authorized restore");
    assert_eq!(restored_path, PathBuf::from("events/fading.md"));
    assert!(!root.path().join("archive/event/fading.md").exists());
    assert_eq!(
        workspace
            .read("events/fading.md")
            .expect("restored")
            .metadata
            .status,
        "active"
    );
    assert_eq!(
        workspace.load_index().expect("index").entries["event_fading"].path,
        "events/fading.md"
    );

    workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/fading.md
    operations:
      - type: update_frontmatter
        fields:
          status: archived
"#,
        )
        .expect("mark archived again");
    workspace.run_maintenance().expect("archive again");
    write_access(
        root.path(),
        &["current", "character", "relationship", "event", "world"],
        &["current", "character", "relationship", "event", "world"],
        true,
    );
    let restored_path = workspace.restore_archived("event_fading").expect("restore");
    assert_eq!(restored_path, PathBuf::from("events/fading.md"));
    assert!(!root.path().join("archive/event/fading.md").exists());
    assert_eq!(
        workspace
            .read("events/fading.md")
            .expect("restored")
            .metadata
            .status,
        "active"
    );
    assert_eq!(
        workspace.load_index().expect("index").entries["event_fading"].path,
        "events/fading.md"
    );
}

#[test]
fn maintenance_guards_decay_when_clock_moves_back_more_than_tolerance() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let now = DECAY_INTERVAL_SECONDS * 3;
    write_fixture(
        root.path(),
        &Fixture {
            relative: "events/future.md",
            id: "event_future",
            kind: "event",
            importance: 0.5,
            weight: 0.5,
            touch_at: now + CLOCK_SKEW_TOLERANCE_SECONDS + 1,
            decay_at: now + CLOCK_SKEW_TOLERANCE_SECONDS + 1,
            tags: &["future"],
            status: "active",
        },
    );
    workspace.rebuild_index().expect("rebuild index");

    let report = workspace.run_maintenance_at(now).expect("maintenance");

    assert!(report.decayed_ids.is_empty());
    assert_eq!(report.clock_skew_guarded_ids, vec!["event_future"]);
    let document = workspace.read("events/future.md").expect("memory");
    assert_eq!(document.metadata.weight, Some(0.5));
    let audit = fs::read_to_string(root.path().join("audit/memory.log")).expect("audit");
    assert!(audit.contains("\tclock_skew_guard\tevent_future\t"));
}

#[test]
fn explicitly_authorized_delete_removes_document_and_index_entry() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &Fixture {
            relative: "events/delete_me.md",
            id: "event_delete_me",
            kind: "event",
            importance: 0.5,
            weight: 0.5,
            touch_at: 1,
            decay_at: 1,
            tags: &[],
            status: "active",
        },
    );
    workspace.rebuild_index().expect("rebuild index");

    workspace
        .delete_document_authorized("event_delete_me")
        .expect("permanent delete");

    assert!(!root.path().join("events/delete_me.md").exists());
    assert!(
        !workspace
            .load_index()
            .expect("index")
            .entries
            .contains_key("event_delete_me")
    );
}

#[test]
fn maintenance_forgets_only_expired_unreferenced_archived_events() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let now = FORGET_AFTER_SECONDS + DECAY_INTERVAL_SECONDS + 10;
    write_fixture(
        root.path(),
        &Fixture {
            relative: "archive/event/forgettable.md",
            id: "event_forgettable",
            kind: "event",
            importance: 0.1,
            weight: 0.04,
            touch_at: 1,
            decay_at: now,
            tags: &["forgettable"],
            status: "archived",
        },
    );
    workspace.rebuild_index().expect("rebuild index");

    let report = workspace.run_maintenance_at(now).expect("maintenance");

    assert_eq!(report.forgotten_ids, vec!["event_forgettable"]);
    assert!(!root.path().join("archive/event/forgettable.md").exists());
    assert!(
        !workspace
            .load_index()
            .expect("index")
            .entries
            .contains_key("event_forgettable")
    );
    let tombstones = workspace.load_tombstones().expect("tombstones");
    assert_eq!(
        tombstones["event_forgettable"],
        ForgottenTombstone {
            kind: "event".to_owned(),
            forgotten_at: now,
            reason: "low_narrative_value".to_owned(),
        }
    );
    let audit = fs::read_to_string(root.path().join("audit/memory.log")).expect("audit");
    assert!(audit.contains("\tforget\tevent_forgettable\t"));
}

#[test]
fn distiller_patch_cannot_write_runtime_managed_fields() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    for field in ["touch_at", "archived_at"] {
        let patch = format!(
            r#"
patches:
  - target_file: "events/runtime_field.md"
    operations:
      - type: "create"
        frontmatter:
          id: "event_runtime_field"
          type: "event"
          importance: 0.5
          weight: 0.5
          decay_at: 1
          {field}: 1
          status: "active"
        content: |-
          # Runtime Field
"#
        );
        assert!(matches!(
            workspace.validate_patch(&patch),
            Err(MemoryError::Yaml(_))
        ));
    }
}

#[test]
fn access_configuration_denies_undeclared_reads_and_writes() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/restricted.md", "event_restricted", &["needle"]),
    );
    write_access(root.path(), &["current"], &["current"], false);

    assert!(matches!(
        workspace.read("events/restricted.md"),
        Err(MemoryError::AccessDenied {
            operation: "read",
            ref kind,
        }) if kind == "event"
    ));
    let retrieved = workspace
        .retrieve("needle", usize::MAX, &ConservativeTokenCounter)
        .expect("retrieve permitted hot memory");
    assert_eq!(retrieved.len(), 2);
    assert!(
        retrieved
            .iter()
            .all(|memory| memory.id.starts_with("current_"))
    );

    let error = workspace
        .apply_patch(
            r#"
patches:
  - target_file: events/restricted.md
    operations:
      - type: update_frontmatter
        fields:
          weight: 0.9
"#,
        )
        .expect_err("event write must be denied");
    assert!(matches!(
        error,
        MemoryError::AccessDenied {
            operation: "write",
            ref kind,
        } if kind == "event"
    ));
}

#[test]
fn hot_memory_is_loaded_first_and_cropped_at_paragraph_boundaries() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let mut scene = workspace.read("current/scene.md").expect("scene");
    scene.body = "# Scene\n\nalpha\n\nbeta\n\n".to_owned();
    atomic_write(
        &root.path().join("current/scene.md"),
        &scene.encode().expect("encode scene"),
    )
    .expect("write scene");
    let mut threads = workspace
        .read("current/active_threads.md")
        .expect("threads");
    threads.body = "# Threads\n\none\n\ntwo\n\n".to_owned();
    atomic_write(
        &root.path().join("current/active_threads.md"),
        &threads.encode().expect("encode threads"),
    )
    .expect("write threads");
    write_fixture(
        root.path(),
        &event_fixture("events/long_term.md", "event_long_term", &["needle"]),
    );

    let counter = ConservativeTokenCounter;
    let active_prefix = "# Threads\n\none\n\n";
    let max_tokens = counter.count(&scene.body) + counter.count(active_prefix);
    let retrieved = workspace
        .retrieve("needle", max_tokens, &counter)
        .expect("retrieve");

    assert_eq!(
        retrieved
            .iter()
            .map(|memory| memory.id.as_str())
            .collect::<Vec<_>>(),
        vec!["current_scene", "current_active_threads"]
    );
    assert_eq!(retrieved[0].body, scene.body);
    assert_eq!(retrieved[1].body, active_prefix);
    assert!(
        retrieved
            .iter()
            .map(|memory| memory.estimated_tokens)
            .sum::<usize>()
            <= max_tokens
    );
}

#[test]
fn retrieval_rebuilds_corrupt_and_stale_indexes_from_memory_files() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    write_fixture(
        root.path(),
        &event_fixture("events/recover.md", "event_recover", &["original"]),
    );
    atomic_write(&workspace.index_path(), "not: [valid").expect("corrupt index");

    assert!(
        workspace
            .retrieve("original", usize::MAX, &ConservativeTokenCounter)
            .expect("repair corrupt index")
            .iter()
            .any(|memory| memory.id == "event_recover")
    );

    write_fixture(
        root.path(),
        &event_fixture("events/recover.md", "event_recover", &["changed"]),
    );
    assert!(
        workspace
            .retrieve("changed", usize::MAX, &ConservativeTokenCounter)
            .expect("repair stale index")
            .iter()
            .any(|memory| memory.id == "event_recover")
    );
    assert_eq!(
        workspace.load_index().expect("index").entries["event_recover"].tags,
        vec!["changed".to_owned()]
    );
}

#[test]
fn rejects_path_traversal_and_unknown_patch_fields_without_writing() {
    let root = tempfile::tempdir().expect("memory root");
    let memory_root = root.path().join("memory");
    let workspace = MemoryWorkspace::initialize(&memory_root).expect("initialize");
    write_fixture(
        &memory_root,
        &event_fixture("events/safe.md", "event_safe", &["safe"]),
    );
    let original = fs::read(memory_root.join("events/safe.md")).expect("fixture");

    let traversal = workspace.apply_patch(
        r#"
patches:
  - target_file: ../escape.md
    operations:
      - type: create
        frontmatter:
          id: event_escape
          type: event
          importance: 0.5
          weight: 0.5
          decay_at: 1
          relations: {}
          tags: []
          status: active
        content: Escape memory.
"#,
    );
    assert!(matches!(traversal, Err(MemoryError::UnsafePath(_))));

    let unknown = workspace.apply_patch(
        r#"
patches:
  - target_file: events/safe.md
    operations:
      - type: update_frontmatter
        fields:
          tags: [changed]
        unexpected: true
"#,
    );
    assert!(matches!(unknown, Err(MemoryError::Yaml(_))));
    assert_eq!(
        fs::read(memory_root.join("events/safe.md")).expect("safe"),
        original
    );
    assert!(!root.path().join("escape.md").exists());
}

#[test]
fn exact_memory_patch_replay_is_idempotent() {
    let root = tempfile::tempdir().expect("memory root");
    let workspace = MemoryWorkspace::initialize(root.path()).expect("initialize");
    let create = r#"
patches:
  - target_file: events/replay.md
    operations:
      - type: create
        frontmatter:
          id: event_replay
          type: event
          importance: 0.5
          weight: 0.5
          decay_at: 1
          status: active
        content: |-
          # Replay

          ## Facts
          original
"#;
    workspace.apply_patch(create).expect("first create");
    workspace.apply_patch(create).expect("replayed create");

    let append = r#"
patches:
  - target_file: events/replay.md
    operations:
      - type: append
        section: Facts
        content: durable addition
"#;
    workspace.apply_patch(append).expect("first append");
    workspace.apply_patch(append).expect("replayed append");
    let document = workspace
        .read("events/replay.md")
        .expect("replayed document");
    assert_eq!(document.body.matches("durable addition").count(), 1);
}

#[test]
fn rejects_symbolic_link_targets() {
    let root = tempfile::tempdir().expect("test root");
    let memory_root = root.path().join("memory");
    let workspace = MemoryWorkspace::initialize(&memory_root).expect("initialize");
    let outside = root.path().join("outside.md");
    atomic_write(
        &outside,
        &MemoryDocument {
            metadata: Metadata {
                id: "event_outside".to_owned(),
                kind: "event".to_owned(),
                importance: Some(0.5),
                weight: Some(0.5),
                touch_at: 1,
                decay_at: Some(1),
                archived_at: None,
                relations: BTreeMap::new(),
                injection_scope: None,
                injection_conversation_id: None,
                injection_character_id: None,
                tags: vec!["outside".to_owned()],
                aliases: Vec::new(),
                status: "active".to_owned(),
            },
            body: TEST_BODY.to_owned(),
        }
        .encode()
        .expect("encode"),
    )
    .expect("outside fixture");
    let link = memory_root.join("events/link.md");
    if let Err(error) = create_file_symlink(&outside, &link) {
        if error.kind() == io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("create symlink: {error}");
    }

    assert!(matches!(
        workspace.read("events/link.md"),
        Err(MemoryError::UnsafePath(_))
    ));
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
