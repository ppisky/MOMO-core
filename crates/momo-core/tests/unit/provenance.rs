use super::*;

fn identity() -> MemoryIdentity {
    let conversation_id = uuid::Uuid::now_v7();
    MemoryIdentity {
        personal_space_id: uuid::Uuid::now_v7(),
        conversation_id,
        character_id: uuid::Uuid::now_v7(),
        continuity_id: conversation_id,
        function_id: Some("chat".into()),
    }
}

fn policy(space: uuid::Uuid, owner: &MemoryIdentity, readers: Vec<uuid::Uuid>) -> RecordPolicy {
    RecordPolicy {
        revision: 1,
        local_space_id: space,
        personal_space_id: owner.personal_space_id,
        owner_character_id: Some(owner.character_id),
        readers,
        continuity_ids: vec![owner.continuity_id],
        function_ids: vec!["chat".into()],
        fact_kind: FactKind::Experience,
        subjects: vec!["user".into()],
        participants: vec![owner.character_id],
        responsible_characters: vec![owner.character_id],
        valid_from: None,
        valid_until: None,
    }
}

fn record(space: uuid::Uuid) -> Value {
    json!({"id":"same_id","path":"events/a.md","body":"A promised to return.","estimated_tokens":12,
        "memory_space":{"id":space,"label":"same label"},"source_character_ids":[],
        "injection_scope":null,"injection_conversation_id":null,"injection_character_id":null,
        "state_signal":{"kind":"relationship","weight":1.0,"touch_at":1,"tags":["promise"],"relations":{}}})
}

#[tokio::test]
async fn scoped_scene_recalls_unmentioned_fact_before_budgeting_without_global_leakage() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = MomoRuntime::initialize(temp.path()).await.unwrap();
    let a = identity();
    let space = a.personal_space_id;
    let ws = runtime.core().memory_for_space(space).unwrap();
    let mut b = a.clone();
    b.character_id = uuid::Uuid::now_v7();
    for (owner, id) in [(&a, "seaside"), (&b, "secret")] {
        let patch = format!(
            "patches:\n  - target_file: events/{id}.md\n    operations:\n      - type: create\n        frontmatter:\n          id: {id}\n          type: event\n          importance: 0.7\n          weight: 0.9\n          decay_at: 0\n          status: active\n        content: '# {id} promise'\n"
        );
        let mut context = momo_domain::provenance::RevisionContext::manual("test");
        context.identity = Some(owner.clone());
        let plan = ws
            .trace_commit(
                ws.prepare_patch_commit(&patch).unwrap(),
                Some(space),
                &context,
            )
            .unwrap();
        ws.apply_prepared_commit(&plan).unwrap();
    }
    // An oversized global scene must neither provide another identity's cues
    // nor consume the actual story's long-term memory budget.
    let mut global = ws.read("current/scene.md").unwrap();
    global.body = format!("# Global\n{}\n[[secret]]", "unrelated ".repeat(100));
    ws.call(move |workspace| {
        std::fs::write(workspace.root().join("current/scene.md"), global.encode()?)?;
        Ok(())
    })
    .unwrap();
    let patch = format!(
        "patches:\n  - target_file: current/scene.md\n    operations:\n      - type: replace\n        section: Location\n        content: '{}'\n      - type: replace\n        section: Source References\n        content: '[[seaside]]'\n",
        "coast ".repeat(100)
    );
    let mut context = momo_domain::provenance::RevisionContext::manual("test");
    context.identity = Some(a.clone());
    let plan = ws
        .trace_commit(
            ws.prepare_identity_patch_commit(&patch, Some(&a)).unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    ws.apply_prepared_commit(&plan).unwrap();
    let request = |who| ScopedMemoryRequest {
        identity: Some(who),
        spaces: vec![MemorySpaceSource {
            space_id: space.to_string(),
            label: "story".into(),
            weight: 100,
            memory: true,
            semantic_graph: false,
        }],
        observe_space_ids: vec![],
        query: "We finally have some time.".into(),
        max_tokens: 128,
        vector_space_id: None,
        query_vector: None,
        embedding: None,
    };
    let snapshot = retrieve_scoped_memory_snapshot(&runtime, request(a.clone()))
        .await
        .unwrap();
    let scene: Value = serde_json::from_str(&snapshot.source_observations[0].scene_json).unwrap();
    assert_eq!(
        scene["location"].as_str().unwrap(),
        "coast ".repeat(100).trim()
    );
    let memories = snapshot.items;
    assert!(
        memories
            .iter()
            .any(|m| m["provenance"]["local_id"] == "seaside")
    );
    assert!(
        !memories
            .iter()
            .any(|m| m["provenance"]["local_id"] == "secret")
    );
    assert!(
        memories
            .iter()
            .map(|m| m["estimated_tokens"].as_u64().unwrap())
            .sum::<u64>()
            <= 128
    );
    assert!(
        retrieve_scoped_memory(&runtime, request(b))
            .await
            .unwrap()
            .is_empty()
    );
    // The same character's independent story must not inherit the cue.
    let mut other_story = a;
    other_story.continuity_id = uuid::Uuid::now_v7();
    assert!(
        retrieve_scoped_memory(&runtime, request(other_story))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn same_space_never_implies_shared_identity_and_shared_experience_is_not_state() {
    let temp = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(temp.path()).await.unwrap();
    let space = uuid::Uuid::now_v7();
    let a = identity();
    let mut b = a.clone();
    b.character_id = uuid::Uuid::now_v7();
    let workspace = core.memory_for_space(space).unwrap();
    workspace
        .set_record_policy(false, "same_id", policy(space, &a, vec![]))
        .unwrap();
    assert!(
        qualify_memory_items(&core, vec![record(space)], &b)
            .await
            .unwrap()
            .is_empty()
    );
    let mut grant = policy(space, &a, vec![b.character_id]);
    grant.revision = 2;
    workspace
        .set_record_policy(false, "same_id", grant)
        .unwrap();
    let shared = qualify_memory_items(&core, vec![record(space)], &b)
        .await
        .unwrap();
    assert_eq!(shared.len(), 1);
    assert_eq!(
        shared[0]["provenance"]["perspective"],
        "external_or_unknown"
    );
    assert!(shared[0]["state_signal"].is_null());
    assert_eq!(
        shared[0]["provenance"]["participants"][0],
        json!(a.character_id)
    );
    b.continuity_id = uuid::Uuid::now_v7();
    assert!(
        qualify_memory_items(&core, vec![record(space)], &b)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn local_ids_are_namespaced_and_unknown_records_do_not_acquire_current_author() {
    let temp = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(temp.path()).await.unwrap();
    let a = identity();
    let first = uuid::Uuid::now_v7();
    let second = uuid::Uuid::now_v7();
    assert!(
        qualify_memory_items(&core, vec![record(first)], &a)
            .await
            .unwrap()
            .is_empty()
    );
    for space in [first, second] {
        core.memory_for_space(space)
            .unwrap()
            .set_record_policy(false, "same_id", policy(space, &a, vec![]))
            .unwrap();
    }
    let records = qualify_memory_items(&core, vec![record(first), record(second)], &a)
        .await
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0]["id"], records[1]["id"]);
    assert!(
        records[0]["provenance"]["source_characters"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn policy_requires_world_mapping_and_valid_intervals() {
    let a = identity();
    let space = uuid::Uuid::now_v7();
    let mut p = policy(space, &a, vec![]);
    p.fact_kind = FactKind::Rule;
    p.continuity_ids.clear();
    assert!(p.validate().is_err());
    p.continuity_ids.push(a.continuity_id);
    assert!(p.validate().is_ok());
    p.valid_from = Some(10);
    p.valid_until = Some(9);
    assert!(p.validate().is_err());
}

#[tokio::test]
async fn imported_history_cannot_reuse_legacy_injection_binding_as_authorization() {
    let temp = tempfile::tempdir().unwrap();
    let core = MomoCore::initialize(temp.path()).await.unwrap();
    let a = identity();
    let space = uuid::Uuid::now_v7();
    let workspace = core.memory_for_space(space).unwrap();
    workspace
        .set_record_policy(false, "same_id", policy(space, &a, vec![]))
        .unwrap();
    let imported = momo_memory::provenance::imported_ledger(
        &serde_json::to_string(&workspace.provenance(false).unwrap()).unwrap(),
        space,
    )
    .unwrap();
    workspace
        .call(move |workspace| {
            std::fs::write(workspace.root().join("config/provenance.json"), imported)?;
            Ok(())
        })
        .unwrap();
    let mut item = record(space);
    item["injection_character_id"] = json!(a.character_id);
    item["injection_conversation_id"] = json!(a.conversation_id);
    assert!(
        qualify_memory_items(&core, vec![item], &a)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn scoped_scene_can_receive_a_new_local_policy_after_import() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = MomoRuntime::initialize(temp.path()).await.unwrap();
    let a = identity();
    let space = a.personal_space_id;
    let workspace = runtime.core().memory_for_space(space).unwrap();
    let patch = "patches:\n  - target_file: current/scene.md\n    operations:\n      - type: append\n        section: Location\n        content: Harbor\n";
    let mut context = momo_domain::provenance::RevisionContext::manual("automatic_threshold");
    context.identity = Some(a.clone());
    let plan = workspace
        .trace_commit(
            workspace
                .prepare_identity_patch_commit(patch, Some(&a))
                .unwrap(),
            Some(space),
            &context,
        )
        .unwrap();
    workspace.apply_prepared_commit(&plan).unwrap();
    let id = workspace.scoped_current(&a).unwrap()[0].id.clone();
    let imported = momo_memory::provenance::imported_ledger(
        &serde_json::to_string(&workspace.provenance(false).unwrap()).unwrap(),
        space,
    )
    .unwrap();
    workspace
        .call(move |workspace| {
            std::fs::write(workspace.root().join("config/provenance.json"), imported)?;
            Ok(())
        })
        .unwrap();
    set_memory_record_policy(
        &runtime,
        space,
        false,
        id.clone(),
        policy(space, &a, vec![]),
    )
    .await
    .unwrap();
    assert!(
        workspace.provenance(false).unwrap().records[&id]
            .policy
            .is_some()
    );
    assert!(
        workspace
            .record_revision_matches(false, &id, "current/scene.md")
            .unwrap()
    );
}

#[tokio::test]
async fn staged_native_maintenance_commits_evidence_and_retrieves_only_for_captured_identity() {
    use momo_domain::provenance::{Evidence, RevisionContext};
    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MomoRuntime::initialize(temp.path()).await.unwrap());
    let identity = identity();
    let space = identity.personal_space_id;
    let now = chrono::Utc::now();
    let store = runtime.core().store();
    store
        .save_conversation(&momo_domain::Conversation {
            id: identity.conversation_id,
            scope_id: space,
            character_id: None,
            title: "evidence".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .begin_response_operation(
            "captured",
            "fp",
            &identity.conversation_id.to_string(),
            "{}",
        )
        .await
        .unwrap();
    let message = momo_domain::Message {
        id: uuid::Uuid::now_v7(),
        conversation_id: identity.conversation_id,
        role: momo_domain::MessageRole::Assistant,
        content: "amberkey confirmed".into(),
        created_at: now,
    };
    let turn = momo_storage::MaintenanceTurn {
        request_id: "captured".into(),
        scope_id: space.to_string(),
        user_content: "amberkey".into(),
        assistant_content: message.content.clone(),
    };
    let response = json!({"momo":{"request_audit":{"memory_identity":identity}}}).to_string();
    store
        .commit_response_completion(momo_storage::ResponseCompletion {
            lifecycle_activity_json: None,
            request_id: "captured",
            conversation_scope_id: space,
            assistant_message: Some(&message),
            maintenance_turn: Some(&turn),
            memory_enabled: true,
            nsg_enabled: false,
            mo_state_operation_id: None,
            response_json: &response,
        })
        .await
        .unwrap();
    let evidence: Evidence = store
        .response_evidence("captured")
        .await
        .unwrap()
        .unwrap()
        .0;
    let patch = "patches:\n  - target_file: events/key.md\n    evidence_refs: [captured]\n    operations:\n      - type: create\n        frontmatter:\n          id: amberkey\n          type: event\n          importance: 0.2\n          weight: 1.0\n          decay_at: 0\n          status: active\n        content: '# Amberkey on the shelf'\n";
    let mut context = RevisionContext::manual("automatic_threshold");
    context.operation_id = "batch".into();
    context.proposer = "dmw_distiller".into();
    context.identity = Some(identity.clone());
    context.evidence = vec![evidence];
    context.target_evidence =
        momo_memory::provenance::patch_evidence(patch, &context.evidence).unwrap();
    store
        .stage_maintenance_batch_with_provenance(
            &momo_storage::MaintenanceBatch {
                batch_key: "batch".into(),
                scope_id: space.to_string(),
                kind: "memory".into(),
                request_ids: vec!["captured".into()],
                patch_yaml: patch.into(),
            },
            Some(&serde_json::to_string(&context).unwrap()),
        )
        .await
        .unwrap();
    let service = crate::MomoApiService::new(
        Arc::clone(&runtime),
        "http://127.0.0.1:9/v1",
        None,
        reqwest::Client::new(),
    );
    assert!(
        service
            .maintain(&space.to_string(), crate::MaintenanceKind::Memory, 12)
            .await
            .unwrap()
    );
    let history = runtime
        .core()
        .memory_for_space(space)
        .unwrap()
        .provenance(false)
        .unwrap();
    assert_eq!(
        history.records["amberkey"].evidence["captured"].character_id,
        Some(identity.character_id)
    );
    let request = |identity| ScopedMemoryRequest {
        identity: Some(identity),
        spaces: vec![MemorySpaceSource {
            space_id: space.to_string(),
            label: "memory".into(),
            weight: 100,
            memory: true,
            semantic_graph: false,
        }],
        observe_space_ids: vec![],
        query: "amberkey".into(),
        max_tokens: 2048,
        vector_space_id: None,
        query_vector: None,
        embedding: None,
    };
    let own = retrieve_scoped_memory(&runtime, request(identity.clone()))
        .await
        .unwrap();
    assert_eq!(own.len(), 1);
    let mut other = identity;
    other.character_id = uuid::Uuid::now_v7();
    assert!(
        retrieve_scoped_memory(&runtime, request(other))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .pending_maintenance_turns(
                &space.to_string(),
                momo_storage::MaintenanceKind::Memory,
                12
            )
            .await
            .unwrap()
            .is_empty()
    );
}
