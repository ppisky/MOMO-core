use super::*;

#[test]
fn denied_ignored_and_allowed_overrides_are_distinct() {
    let policy = RequestOverridePolicy {
        context_window: OverrideMode::Ignore,
        max_output_tokens: OverrideMode::Allow,
        sampling: OverrideMode::Reject,
        instructions: OverrideMode::Allow,
        visual_description_prompt: OverrideMode::Ignore,
        tools: OverrideMode::Allow,
        allowed_parameters: BTreeSet::from(["seed".to_owned()]),
    };
    let parameters = Map::from_iter([("seed".to_owned(), json!(42))]);
    let effective = policy
        .apply(
            8_192,
            1_024,
            RequestedOverrides {
                context_window: Some(4_096),
                max_output_tokens: Some(512),
                temperature: None,
                instructions: Some("request instructions"),
                visual_description_prompt: Some("request vision prompt"),
                parameters: &parameters,
                tool_configuration_requested: false,
            },
        )
        .expect("governed request");
    assert_eq!(effective.context_window, 8_192);
    assert_eq!(effective.max_output_tokens, 512);
    assert_eq!(
        effective.instructions.as_deref(),
        Some("request instructions")
    );
    assert!(effective.visual_description_prompt.is_none());
    assert_eq!(effective.parameters["seed"], 42);
}

#[test]
fn runtime_settings_select_the_vision_prompt_space() {
    let config: MomoRuntimeSettings = serde_json::from_value(json!({
        "schema_version": 1,
        "vision": {"enabled": true}
    }))
    .expect("runtime settings");
    config.validate().expect("valid config");
    let parameters = Map::new();
    let effective = config
        .govern(
            8_192,
            1_024,
            RequestedOverrides {
                context_window: None,
                max_output_tokens: None,
                temperature: None,
                instructions: None,
                visual_description_prompt: None,
                parameters: &parameters,
                tool_configuration_requested: false,
            },
        )
        .expect("governed request");
    assert!(effective.visual_description_prompt.is_none());
    assert_eq!(
        effective.audit["visual_description_prompt_source"],
        "prompt_space"
    );
}

#[test]
fn runtime_settings_reject_legacy_vision_prompt_content() {
    assert!(
        serde_json::from_value::<MomoRuntimeSettings>(json!({
            "schema_version": 1,
            "vision": {"enabled": true, "prompt": "legacy"}
        }))
        .is_err()
    );
}

#[test]
fn runtime_settings_reject_host_adapter_wiring() {
    assert!(
        serde_json::from_value::<MomoRuntimeSettings>(json!({
            "schema_version": 1,
            "providers": [{"provider_id": "unsafe"}]
        }))
        .is_err()
    );
}

#[test]
fn maintenance_runtime_settings_are_bounded() {
    let config: MomoRuntimeSettings = serde_json::from_value(json!({
        "schema_version": 1,
        "runtime": {
            "memory_distillation_enabled": false,
            "memory_distill_every_turns": 7,
            "semantic_graph_enabled": true,
            "nsg_govern_every_turns": 19
        },
        "mo_state": {
            "profile": "closed_autonomous",
            "scene_management": true,
            "max_reconcile_steps": 4,
            "max_agent_steps": 8,
            "operation_timeout_ms": 30000,
            "injection_mode": "shadow"
        }
    }))
    .expect("runtime settings");
    config.validate().expect("valid runtime");
    assert!(!config.runtime.memory_distillation_enabled);
    assert_eq!(config.runtime.memory_distill_every_turns, 7);
    assert!(config.runtime.semantic_graph_enabled);
    assert_eq!(config.runtime.nsg_govern_every_turns, 19);
    assert_eq!(config.mo_state.profile, MoStateProfile::ClosedAutonomous);
    assert!(config.mo_state.scene_management);
    assert_eq!(config.mo_state.injection_mode, MoStateInjectionMode::Shadow);

    let mut invalid = config;
    invalid.runtime.nsg_govern_every_turns = 0;
    assert!(matches!(
        invalid.validate(),
        Err(GovernanceError::Invalid(_))
    ));
}

#[test]
fn runtime_settings_reject_prompt_space_content() {
    assert!(
        serde_json::from_value::<MomoRuntimeSettings>(json!({
            "schema_version": 1,
            "prompt_spaces": {"roleplay_director": "other"}
        }))
        .is_err()
    );
}

#[test]
fn ddm_rollout_switch_does_not_accept_runtime_profile_paths() {
    let config: MomoRuntimeSettings = serde_json::from_value(json!({
        "schema_version": 1,
        "mo_state": {"ddm": {"enabled": true}}
    }))
    .expect("runtime settings");
    assert!(config.mo_state.ddm.enabled);
    assert!(!MomoRuntimeSettings::default().mo_state.ddm.enabled);

    assert!(
        serde_json::from_value::<MomoRuntimeSettings>(json!({
            "schema_version": 1,
            "mo_state": {"ddm": {"enabled": true, "profile_files": {}}}
        }))
        .is_err()
    );
}
