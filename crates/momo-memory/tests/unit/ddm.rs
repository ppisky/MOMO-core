use super::*;

fn profile() -> DdmProfile {
    DdmProfile::parse_yaml(
        r#"
schema: momo.ddm/1
character_id: character-1
revision: 1
profile: logit_additive
selection:
  top_k: 2
  salient_threshold: 0.65
  dominant_threshold: 0.85
dispositions:
  - id: protect_companion
    base_activation: 0.72
    modulation:
      context:
        - id: danger
          signal: dmw.tag.danger
          when: true
          delta: 0.9
      state:
        - id: exhausted
          signal: state.dimension.physiological_state
          when: true
          delta: -0.35
    expression:
      latent: Keep concern implicit.
      salient: Offer concrete help.
      dominant: Prioritize immediate safety while preserving agency.
    constraints:
      - Never decide the companion's voluntary actions.
"#,
    )
    .expect("profile")
}

#[test]
fn neutral_and_missing_signals_preserve_base_activation() {
    let output = profile().evaluate(&DdmSignalSnapshot::default());
    let activation = output.effective_dispositions[0].effective_activation;
    assert!((activation - 0.72).abs() < 1e-9);
}

#[test]
fn explicit_positive_and_negative_signals_are_monotonic_and_audited_once() {
    let profile = profile();
    let mut danger = DdmSignalSnapshot::default();
    danger.insert("dmw.tag.danger", DdmSignalValue::Bool(true), "event:1");
    danger.insert("dmw.tag.danger", DdmSignalValue::Bool(true), "event:1");
    let raised = profile.evaluate(&danger).effective_dispositions.remove(0);
    let mut both = danger;
    both.insert(
        "state.dimension.physiological_state",
        DdmSignalValue::Bool(true),
        "state:physiological_state",
    );
    let moderated = profile.evaluate(&both).effective_dispositions.remove(0);
    assert!(raised.effective_activation > 0.72);
    assert!(moderated.effective_activation < raised.effective_activation);
    assert_eq!(raised.evidence_ids, ["event:1"]);
    assert_eq!(raised.matched_rule_ids, ["danger"]);
}

#[test]
fn profile_is_not_mutated_by_projection() {
    let profile = profile();
    let original = profile.clone();
    let _ = profile.evaluate(&DdmSignalSnapshot::default());
    assert_eq!(profile, original);
}

#[test]
fn persisted_previous_band_prevents_threshold_oscillation() {
    let mut profile = profile();
    profile.dispositions[0].base_activation = 0.64;
    let previous = BTreeMap::from([("protect_companion".to_owned(), DdmBand::Salient)]);
    let retained = profile.evaluate_with_previous(&DdmSignalSnapshot::default(), &previous);
    assert_eq!(retained.effective_dispositions[0].band, DdmBand::Salient);
    assert_eq!(retained.hysteresis_applied, ["protect_companion"]);

    profile.dispositions[0].base_activation = 0.59;
    let exited = profile.evaluate_with_previous(&DdmSignalSnapshot::default(), &previous);
    assert_eq!(exited.effective_dispositions[0].band, DdmBand::Latent);
    assert!(exited.hysteresis_applied.is_empty());
}

#[test]
fn scene_and_request_signals_are_closed_and_typed() {
    let yaml = r#"
schema: momo.ddm/1
character_id: character-1
revision: 1
profile: logit_additive
dispositions:
  - id: visual_alertness
    base_activation: 0.5
    modulation:
      context:
        - id: active_scene
          signal: scene.status
          when: active
          missing: neutral
          delta: 0.2
        - id: image_input
          signal: request.has_image
          when: true
          delta: 0.2
    expression:
      latent: Observe.
      salient: Check visible details.
      dominant: Act on immediate visible risk.
"#;
    assert!(DdmProfile::parse_yaml(yaml).is_ok());
    assert!(DdmProfile::parse_yaml(&yaml.replace("request.has_image", "request.intent")).is_err());
    assert!(DdmProfile::parse_yaml(&yaml.replace("when: true", "when: image")).is_err());
}

#[test]
fn mutually_exclusive_groups_select_one_deterministically() {
    let mut profile = profile();
    profile.selection.top_k = 4;
    profile.dispositions[0].exclusive_group = Some("response_mode".to_owned());
    let mut second = profile.dispositions[0].clone();
    second.id = "withdraw".to_owned();
    second.base_activation = 0.3;
    second.modulation = DdmModulation::default();
    second.exclusive_group = Some("response_mode".to_owned());
    profile.dispositions.push(second);
    profile.validate().expect("exclusive profile");
    let audit = profile.evaluate(&DdmSignalSnapshot::default());
    assert_eq!(audit.effective_dispositions.len(), 1);
    assert_eq!(audit.effective_dispositions[0].id, "protect_companion");
    assert_eq!(audit.suppressed_disposition_ids, ["withdraw"]);
    assert_eq!(audit.next_bands.len(), 2);
}

#[test]
fn source_fingerprint_binds_signals_and_previous_bands() {
    let profile = profile();
    let empty = profile.evaluate(&DdmSignalSnapshot::default());
    let mut signals = DdmSignalSnapshot::default();
    signals.insert("dmw.tag.danger", DdmSignalValue::Bool(true), "event:1");
    let signaled = profile.evaluate(&signals);
    assert_ne!(empty.source_fingerprint, signaled.source_fingerprint);
    assert_eq!(
        signaled.source_fingerprint,
        profile.evaluate(&signals).source_fingerprint
    );
    let previous = BTreeMap::from([("protect_companion".to_owned(), DdmBand::Salient)]);
    assert_ne!(
        signaled.source_fingerprint,
        profile
            .evaluate_with_previous(&signals, &previous)
            .source_fingerprint
    );
}
