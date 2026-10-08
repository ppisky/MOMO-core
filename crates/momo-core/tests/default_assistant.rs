use momo_core::{MomoRuntime, api::runtime_api};

#[tokio::test]
async fn bootstrap_is_concurrent_durable_and_preserves_existing_mapping() {
    let directory = tempfile::tempdir().unwrap();
    let scope = uuid::Uuid::now_v7();
    let first_id;
    {
        let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
        assert!(
            runtime_api::default_memory_assistant(&runtime, scope)
                .await
                .unwrap()
                .is_none()
        );
        let create = || {
            runtime_api::ensure_default_memory_assistant(
                &runtime,
                scope,
                "Assistant".into(),
                "Host".into(),
            )
        };
        let (a, b) = tokio::join!(create(), create());
        let a = a.unwrap();
        let b = b.unwrap();
        first_id = a.id;
        assert_eq!(a.id, b.id);
        assert!(a.character_markdown.is_empty());
        assert_eq!(
            runtime
                .core()
                .store()
                .list_characters_for_scope(scope)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    assert_eq!(
        runtime_api::default_memory_assistant(&runtime, scope)
            .await
            .unwrap(),
        Some(first_id)
    );
    let replacement = runtime_api::stage_character(
        &runtime,
        scope.to_string(),
        "Host".into(),
        "Explicit".into(),
        "Persona".into(),
        String::new(),
    )
    .await
    .unwrap();
    runtime_api::set_default_memory_assistant(&runtime, scope, replacement.id)
        .await
        .unwrap();
    let reused = runtime_api::ensure_default_memory_assistant(
        &runtime,
        scope,
        "Ignored".into(),
        "Host".into(),
    )
    .await
    .unwrap();
    assert_eq!(reused.id, replacement.id);
    assert_eq!(reused.character_markdown, "Persona");
    assert!(
        runtime
            .core()
            .store()
            .character_by_id(first_id)
            .await
            .unwrap()
            .is_some()
    );
}
