use momo_core::{
    MomoRuntime,
    api::runtime_api::{self, AssetEdit, RuntimeApiErrorKind},
};
use serde_json::{Value, json};
use uuid::Uuid;

fn edit(space: Uuid, owner: Uuid, revision: Option<u64>, policy_revision: u64) -> AssetEdit {
    serde_json::from_value(json!({
        "space_id":space,"expected_revision":revision,
        "content":{"kind":"memory","metadata":{"id":"editable","type":"event","status":"active",
            "importance":0.8,"weight":0.8,"decay_at":0,"aliases":["Title"],
            "injection_scope":"character","injection_character_id":owner},"body":"Original"},
        "policy":{"revision":policy_revision,"local_space_id":space,"personal_space_id":space,
            "owner_character_id":owner,"readers":[],"continuity_ids":[],"function_ids":[],
            "fact_kind":"experience","subjects":[],"participants":[],"responsible_characters":[],"valid_from":null,"valid_until":null}
    })).unwrap()
}
async fn document(runtime: &MomoRuntime, space: Uuid) -> Value {
    runtime_api::read_memory_document(runtime, space.to_string(), "editable".into())
        .await
        .unwrap()
}
#[tokio::test]
async fn asset_configuration_is_atomic_conflict_checked_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let space = Uuid::now_v7();
    let owner = Uuid::now_v7();
    {
        let runtime = MomoRuntime::initialize(root.path()).await.unwrap();
        runtime_api::configure_asset(&runtime, edit(space, owner, None, 1))
            .await
            .unwrap();
        // A shared storage Space does not become the owner's personal identity.
        let mut shared = edit(space, owner, None, 1);
        shared.policy.personal_space_id = Uuid::now_v7();
        if let runtime_api::AssetContent::Memory {
            ref mut metadata, ..
        } = shared.content
        {
            metadata.id = "shared-memory".into();
        }
        runtime_api::configure_asset(&runtime, shared)
            .await
            .unwrap();
        let ledger = runtime_api::memory_provenance(&runtime, space, false)
            .await
            .unwrap();
        let before = ledger.records["editable"].clone();
        let revision = before.revisions.len() as u64;
        let mut invalid = edit(space, owner, Some(revision), 1); // stale policy after content staged
        if let runtime_api::AssetContent::Memory { ref mut body, .. } = invalid.content {
            *body = "Must rollback".into();
        }
        assert!(
            runtime_api::configure_asset(&runtime, invalid)
                .await
                .is_err()
        );
        assert_eq!(document(&runtime, space).await["body"], "Original");
        let stale = runtime_api::configure_asset(&runtime, edit(space, owner, Some(0), 2))
            .await
            .unwrap_err();
        assert_eq!(stale.kind, RuntimeApiErrorKind::Conflict);
        let mut updated = edit(space, owner, Some(revision), 2);
        if let runtime_api::AssetContent::Memory {
            ref mut body,
            ref mut metadata,
        } = updated.content
        {
            *body = "Updated".into();
            metadata.injection_conversation_id = None;
        }
        runtime_api::configure_asset(&runtime, updated)
            .await
            .unwrap();
        assert!(document(&runtime, space).await["metadata"]["injection_conversation_id"].is_null());
        let after = runtime_api::memory_provenance(&runtime, space, false)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&after.records["editable"].revisions[0]).unwrap(),
            serde_json::to_value(&before.revisions[0]).unwrap()
        );
        // NSG rejects dangling edges without publishing a partially configured node.
        let mut graph = serde_json::to_value(edit(space, owner, None, 1)).unwrap();
        graph["content"] = json!({"kind":"semantic_graph","node":{"id":"rule","graph_id":"default","type":"rule","importance":0.8,"mode":"canon","status":"active","zone":"auto","anchors":["rule"],"condition":"","trigger":"rule","constraint":"stay","consequence":"","source_character_ids":[],"inject_character_ids":[owner],"edges":[{"category":"structural","relation":"contains","weight":0.8,"target":"missing"}]}});
        graph["policy"]["fact_kind"] = json!("rule");
        graph["policy"]["continuity_ids"] = json!([Uuid::now_v7()]);
        assert!(
            runtime_api::configure_asset(&runtime, serde_json::from_value(graph.clone()).unwrap())
                .await
                .is_err()
        );
        graph["content"]["node"]["edges"] = json!([]);
        runtime_api::configure_asset(&runtime, serde_json::from_value(graph).unwrap())
            .await
            .unwrap();
        assert!(
            runtime_api::memory_provenance(&runtime, space, true)
                .await
                .unwrap()
                .records
                .contains_key("rule")
        );
    }
    let runtime = MomoRuntime::initialize(root.path()).await.unwrap();
    assert_eq!(document(&runtime, space).await["body"], "Updated");
}

#[tokio::test]
async fn full_character_opening_is_a_single_durable_assistant_message() {
    let root = tempfile::tempdir().unwrap();
    let runtime = MomoRuntime::initialize(root.path()).await.unwrap();
    let space = Uuid::now_v7();
    let now = chrono::Utc::now();
    let card = momo_core::momo_domain::CharacterCard {
        id: Uuid::now_v7(),
        scope_id: space,
        name: "Character".into(),
        version: "2.1.0".into(),
        author_name: "Author".into(),
        author_url: Some("https://example.com".into()),
        character_markdown: "Persona".into(),
        user_markdown: "User".into(),
        opening_markdown: Some("Opening".into()),
        created_at: now,
        updated_at: now,
    };
    let card = runtime_api::stage_character_card(&runtime, card)
        .await
        .unwrap();
    let id = Uuid::now_v7().to_string();
    for _ in 0..2 {
        runtime_api::stage_conversation(
            &runtime,
            Some(id.clone()),
            space.to_string(),
            "Chat".into(),
            card.id.to_string(),
        )
        .await
        .unwrap();
    }
    let messages = runtime_api::local_messages(&runtime, space.to_string(), id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Opening");
    assert_eq!(
        messages[0].role,
        momo_core::momo_domain::MessageRole::Assistant
    );
    let stored = runtime_api::local_character(&runtime, card.id.to_string())
        .await
        .unwrap();
    assert_eq!(stored.user_markdown, "User");
    assert_eq!(stored.author_name, "Author");
}
