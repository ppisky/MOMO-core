use super::*;

#[tokio::test]
async fn settings_survive_restart_and_sparse_space_updates_preserve_inheritance() {
    let directory = tempfile::tempdir().unwrap();
    let space = uuid::Uuid::now_v7();
    let revision;
    {
        let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
        runtime
            .patch_runtime_settings(
                None,
                serde_json::json!({"history_window":{"evict_turns":4}}),
                Some(0),
            )
            .unwrap();
        runtime
            .patch_runtime_settings(
                Some(space),
                serde_json::json!({"vision":{"enabled":true}}),
                Some(1),
            )
            .unwrap();
        runtime
            .patch_runtime_settings(
                None,
                serde_json::json!({"mo_state":{"max_agent_steps":5}}),
                Some(2),
            )
            .unwrap();
        let effective = runtime.runtime_settings_for_space(space);
        assert!(effective.vision.enabled);
        assert_eq!(effective.history_window.evict_turns, 4);
        assert_eq!(effective.mo_state.max_agent_steps, 5);
        assert!(matches!(
            runtime.patch_runtime_settings(
                None,
                serde_json::json!({"vision":{"enabled":true}}),
                Some(1)
            ),
            Err(crate::GovernanceError::Conflict { .. })
        ));
        assert!(
            runtime
                .patch_runtime_settings(
                    Some(space),
                    serde_json::json!({"mo_state":{"unknown":1}}),
                    None
                )
                .is_err()
        );
        assert_eq!(runtime.runtime_settings_resource(None).revision, 3);
        // A new instance default must not invalidate an existing sparse Space policy.
        runtime
            .patch_runtime_settings(
                Some(space),
                serde_json::json!({"history_window":{"evict_turns":6}}),
                None,
            )
            .unwrap();
        let before = runtime.runtime_settings_resource(None).revision;
        assert!(
            runtime
                .patch_runtime_settings(
                    None,
                    serde_json::json!({"history_window":{"trigger_turns":5}}),
                    None
                )
                .is_err()
        );
        assert_eq!(runtime.runtime_settings_resource(None).revision, before);
        runtime
            .patch_runtime_settings(
                Some(space),
                serde_json::json!({"history_window":null}),
                None,
            )
            .unwrap();
        revision = runtime.runtime_settings_resource(None).revision;
    }
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    assert_eq!(runtime.runtime_settings_resource(None).revision, revision);
    assert!(runtime.runtime_settings_for_space(space).vision.enabled);
    runtime
        .patch_runtime_settings(
            Some(space),
            serde_json::json!({"vision":null}),
            Some(revision),
        )
        .unwrap();
    assert!(!runtime.runtime_settings_for_space(space).vision.enabled);
    runtime.reset_runtime_settings(Some(space), None).unwrap();
    assert_eq!(
        runtime.runtime_settings_for_space(space),
        runtime.runtime_settings()
    );
}

#[tokio::test]
async fn clearing_last_space_override_restores_instance_source() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    let space = uuid::Uuid::now_v7();
    runtime
        .patch_runtime_settings(
            Some(space),
            serde_json::json!({
                "vision":{"enabled":true},
                "roleplay":{"enabled":false},
                "request_overrides":{"allowed_parameters":[]},
            }),
            None,
        )
        .unwrap();
    let remaining = runtime
        .patch_runtime_settings(
            Some(space),
            serde_json::json!({"vision":{"enabled":null}}),
            None,
        )
        .unwrap();
    assert_eq!(remaining.source, "space_override");
    assert_eq!(remaining.sources["vision.enabled"], "instance");
    assert_eq!(remaining.sources["roleplay.enabled"], "space_override");
    assert!(!remaining.settings.roleplay.enabled);
    assert!(
        remaining
            .settings
            .request_overrides
            .allowed_parameters
            .is_empty()
    );
    let cleared = runtime
        .patch_runtime_settings(
            Some(space),
            serde_json::json!({
                "roleplay":{"enabled":null}, "request_overrides":{"allowed_parameters":null},
            }),
            None,
        )
        .unwrap();
    assert_eq!(cleared.source, "instance");
    assert!(cleared.sources.values().all(|source| source == "instance"));
    assert!(
        runtime
            .patch_runtime_settings(Some(space), serde_json::json!({"unknown":{}}), None,)
            .is_err(),
        "normalization must not hide invalid fields"
    );
    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("runtime-settings.json")).unwrap(),
    )
    .unwrap();
    assert!(persisted["spaces"].as_object().unwrap().is_empty());
    drop(runtime);
    let restarted = MomoRuntime::initialize(directory.path()).await.unwrap();
    assert_eq!(
        restarted.runtime_settings_resource(Some(space)).source,
        "instance"
    );
}

#[tokio::test]
async fn empty_space_overrides_do_not_consume_capacity_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let empty_space = uuid::Uuid::now_v7();
    let occupied: Vec<_> = (0..1023).map(|_| uuid::Uuid::now_v7()).collect();
    let mut spaces: std::collections::BTreeMap<_, _> = occupied
        .iter()
        .map(|id| (*id, serde_json::json!({"vision":{"enabled":true}})))
        .collect();
    spaces.insert(
        empty_space,
        serde_json::json!({"vision":{},"mo_state":{"ddm":{}}}),
    );
    std::fs::write(
        directory.path().join("runtime-settings.json"),
        serde_json::to_vec(
            &serde_json::json!({"schema":"momo.runtime-settings/1","revision":7,
            "defaults":crate::MomoRuntimeSettings::default(),"spaces":spaces}),
        )
        .unwrap(),
    )
    .unwrap();
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    assert_eq!(
        runtime.runtime_settings_resource(Some(empty_space)).source,
        "instance"
    );
    runtime
        .patch_runtime_settings(
            Some(uuid::Uuid::now_v7()),
            serde_json::json!({"vision":{"enabled":true}}),
            None,
        )
        .unwrap();
    let no_op = runtime
        .patch_runtime_settings(
            Some(empty_space),
            serde_json::json!({"vision":{"enabled":null}}),
            None,
        )
        .unwrap();
    assert_eq!(no_op.source, "instance");
    assert!(
        runtime
            .patch_runtime_settings(
                Some(uuid::Uuid::now_v7()),
                serde_json::json!({"vision":{"enabled":true}}),
                None
            )
            .is_err()
    );
    assert_eq!(
        runtime.runtime_settings_resource(None).revision,
        no_op.revision
    );
    runtime
        .patch_runtime_settings(
            Some(occupied[0]),
            serde_json::json!({"vision":{"enabled":null}}),
            None,
        )
        .unwrap();
    runtime
        .patch_runtime_settings(
            Some(uuid::Uuid::now_v7()),
            serde_json::json!({"vision":{"enabled":true}}),
            None,
        )
        .unwrap();
    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("runtime-settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["spaces"].as_object().unwrap().len(), 1024);
    assert!(persisted["spaces"].get(empty_space.to_string()).is_none());
    assert!(persisted["spaces"].get(occupied[0].to_string()).is_none());
}

#[tokio::test]
async fn failed_settings_persistence_never_publishes_policy_or_revision() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    std::fs::create_dir(directory.path().join("runtime-settings.json")).unwrap();
    assert!(matches!(
        runtime.patch_runtime_settings(None, serde_json::json!({"vision":{"enabled":true}}), None),
        Err(crate::GovernanceError::Persistence(_))
    ));
    assert!(!runtime.runtime_settings().vision.enabled);
    assert_eq!(runtime.runtime_settings_resource(None).revision, 0);
}

#[tokio::test]
async fn journaled_state_contract_has_atomic_revision_checks_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let space = uuid::Uuid::now_v7();
    let changed;
    {
        let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
        let current = crate::api::runtime_api::mo_state_contract(&runtime, space)
            .await
            .unwrap();
        let yaml = "version: 1\ndimensions: {}\n";
        changed = crate::api::runtime_api::replace_mo_state_contract(
            &runtime,
            space,
            Some(yaml.into()),
            Some(current.revision.clone()),
        )
        .await
        .unwrap();
        assert_eq!(changed.source, "override");
        let error = crate::api::runtime_api::replace_mo_state_contract(
            &runtime,
            space,
            None,
            Some(current.revision),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.kind,
            crate::api::runtime_api::RuntimeApiErrorKind::Conflict
        );
        assert!(
            crate::api::runtime_api::replace_mo_state_contract(
                &runtime,
                space,
                Some("version: 1\ndimensions: []\n".into()),
                None
            )
            .await
            .is_err()
        );
        assert_eq!(
            crate::api::runtime_api::mo_state_contract(&runtime, space)
                .await
                .unwrap()
                .revision,
            changed.revision
        );
    }
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    assert_eq!(
        crate::api::runtime_api::mo_state_contract(&runtime, space)
            .await
            .unwrap()
            .revision,
        changed.revision
    );
    assert_eq!(
        crate::api::runtime_api::replace_mo_state_contract(
            &runtime,
            space,
            None,
            Some(changed.revision)
        )
        .await
        .unwrap()
        .source,
        "builtin"
    );
}

#[tokio::test]
async fn caller_cancellation_does_not_release_an_owned_commit_lock() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = Arc::new(
        MomoRuntime::initialize(directory.path())
            .await
            .expect("runtime"),
    );
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, proceed) = tokio::sync::oneshot::channel();
    let (finished, completed) = tokio::sync::oneshot::channel();
    let task_runtime = runtime.clone();
    let caller = tokio::spawn(async move {
        let guard = task_runtime
            .reserve_space(uuid::Uuid::nil())
            .await
            .expect("Space available");
        task_runtime
            .finish_commit(async move {
                let _guard = guard;
                started.send(()).expect("started");
                proceed.await.expect("release");
                finished.send(()).expect("finished");
            })
            .await
    });
    ready.await.expect("commit entered");
    caller.abort();
    assert!(caller.await.expect_err("aborted caller").is_cancelled());
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            runtime.reserve_space(uuid::Uuid::nil())
        )
        .await
        .is_err()
    );
    release.send(()).expect("continue commit");
    completed.await.expect("commit completed after caller left");
    let _guard = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime.reserve_space(uuid::Uuid::nil()),
    )
    .await
    .expect("lock released");
}

#[tokio::test]
async fn cancellation_registration_is_removed_on_drop_and_preserves_replacement() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = MomoRuntime::initialize(directory.path())
        .await
        .expect("runtime");
    let first = runtime.register_cancellation("request".to_owned());
    let replacement = runtime.register_cancellation("request".to_owned());
    drop(first);
    assert!(runtime.cancel_chat("request"));
    replacement.notified().await;
    drop(replacement);
    assert!(!runtime.cancel_chat("request"));
}

#[tokio::test]
async fn keyed_locks_serialize_only_matching_resources() {
    let locks = tokio::sync::Mutex::new(HashMap::new());
    let first = lock_keyed(&locks, "space-a").await;

    let different = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        lock_keyed(&locks, "space-b"),
    )
    .await
    .expect("a different resource must not share the lock");
    drop(different);

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            lock_keyed(&locks, "space-a"),
        )
        .await
        .is_err(),
        "the same resource must remain serialized",
    );

    drop(first);
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        lock_keyed(&locks, "space-a"),
    )
    .await
    .expect("the resource lock must be released with its guard");
}

#[tokio::test]
async fn embedded_runtime_does_not_load_or_rewrite_host_account_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("model-connections.json");
    std::fs::write(&path, b"invalid host configuration").unwrap();
    let runtime = MomoRuntime::initialize(directory.path()).await.unwrap();
    assert_eq!(runtime.runtime_settings_resource(None).revision, 0);
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid host configuration");
}
