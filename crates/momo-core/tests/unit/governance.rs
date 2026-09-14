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
fn full_portable_document_keeps_unowned_sections_and_uses_momo_vision_prompt() {
    let document = r#"
schema_version = 1

[model_use.chat]
route = "primary"

[vision]
enabled = true
prompt = "Describe the visible scene."

"#;
    let config: MomoConfig = toml::from_str(document).expect("portable config");
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
    assert_eq!(
        effective.visual_description_prompt.as_deref(),
        Some("Describe the visible scene.")
    );
    assert_eq!(effective.audit["visual_description_prompt_source"], "momo");
}

#[test]
fn rejects_host_adapter_wiring_in_portable_document() {
    let error =
        validate_momo_document("schema_version = 1\n[[providers]]\nprovider_id = 'unsafe'\n")
            .expect_err("host field");
    assert!(matches!(error, GovernanceError::HostField(field) if field == "providers"));
}

#[test]
fn maintenance_runtime_is_portable_and_bounded() {
    let config: MomoConfig = toml::from_str(
        r#"
schema_version = 1

[runtime]
memory_distillation_enabled = false
memory_distill_every_turns = 7
semantic_graph_enabled = true
nsg_govern_every_turns = 19
max_concurrent_chats = 4

[mo_state]
profile = "closed_autonomous"
scene_management = true
max_reconcile_steps = 4
max_agent_steps = 8
operation_timeout_ms = 30000
injection_mode = "shadow"

"#,
    )
    .expect("portable runtime");
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
fn portable_config_rejects_prompt_substitution() {
    let document = "schema_version = 1\n[prompts]\nroleplay_director_file = 'other.md'\n";
    assert!(matches!(
        validate_momo_document(document),
        Err(GovernanceError::Invalid(message))
            if message.contains("cannot be configured by momo.toml")
    ));
}

#[test]
fn ddm_rollout_switch_does_not_accept_runtime_profile_paths() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("momo.toml");
    fs::write(
        &path,
        "schema_version = 1\n[mo_state.ddm]\nenabled = true\n",
    )
    .expect("config");

    let config = MomoConfig::load(&path).expect("load config");
    assert!(config.mo_state.ddm.enabled);
    assert!(!MomoConfig::default().mo_state.ddm.enabled);

    fs::write(
        &path,
        "schema_version = 1\n[mo_state.ddm]\nenabled = true\nprofile_files = {}\n",
    )
    .expect("invalid config");
    assert!(MomoConfig::load(&path).is_err());
}
