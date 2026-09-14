//! Deterministic Dynamic Disposition Model projection.
//!
//! DDM profiles are authored data. This module only evaluates explicit,
//! governed signals; it never infers dispositions from free-form prose.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::MemoryError;

const DEFAULT_TOP_K: usize = 3;
const DEFAULT_SALIENT_THRESHOLD: f64 = 0.65;
const DEFAULT_DOMINANT_THRESHOLD: f64 = 0.85;
const DEFAULT_HYSTERESIS_MARGIN: f64 = 0.05;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DdmCalculationProfile {
    LogitAdditive,
    Multiplicative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DdmProfile {
    pub schema: String,
    pub character_id: String,
    pub revision: u64,
    pub profile: DdmCalculationProfile,
    #[serde(default)]
    pub selection: DdmSelection,
    pub dispositions: Vec<DdmDisposition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DdmSelection {
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_salient_threshold")]
    pub salient_threshold: f64,
    #[serde(default = "default_dominant_threshold")]
    pub dominant_threshold: f64,
    #[serde(default = "default_hysteresis_margin")]
    pub hysteresis_margin: f64,
}

impl Default for DdmSelection {
    fn default() -> Self {
        Self {
            top_k: default_top_k(),
            salient_threshold: default_salient_threshold(),
            dominant_threshold: default_dominant_threshold(),
            hysteresis_margin: default_hysteresis_margin(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DdmDisposition {
    pub id: String,
    pub base_activation: f64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub exclusive_group: Option<String>,
    #[serde(default)]
    pub modulation: DdmModulation,
    pub expression: DdmExpression,
    #[serde(default)]
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DdmModulation {
    #[serde(default)]
    pub context: Vec<DdmRule>,
    #[serde(default)]
    pub state: Vec<DdmRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DdmRule {
    pub id: String,
    pub signal: String,
    pub when: DdmSignalValue,
    #[serde(default)]
    pub missing: DdmMissingPolicy,
    #[serde(default)]
    pub delta: Option<f64>,
    #[serde(default)]
    pub multiplier: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DdmMissingPolicy {
    #[default]
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DdmSignalValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DdmExpression {
    pub latent: String,
    pub salient: String,
    pub dominant: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DdmBand {
    Latent,
    Salient,
    Dominant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectiveDisposition {
    pub id: String,
    pub base_activation: f64,
    pub context_effect: f64,
    pub state_effect: f64,
    pub effective_activation: f64,
    pub band: DdmBand,
    pub cue: String,
    pub constraints: Vec<String>,
    pub matched_rule_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DdmAudit {
    pub character_id: String,
    pub profile_revision: u64,
    pub constraints: Vec<String>,
    pub effective_dispositions: Vec<EffectiveDisposition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressed_disposition_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub previous_bands: BTreeMap<String, DdmBand>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub next_bands: BTreeMap<String, DdmBand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hysteresis_applied: Vec<String>,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct DdmSignalSnapshot {
    values: BTreeMap<String, DdmSignalValue>,
    evidence: BTreeMap<String, Vec<String>>,
}

impl DdmSignalSnapshot {
    pub fn insert(
        &mut self,
        id: impl Into<String>,
        value: DdmSignalValue,
        evidence_id: impl Into<String>,
    ) {
        let id = id.into();
        self.values.insert(id.clone(), value);
        let evidence_id = evidence_id.into();
        let evidence = self.evidence.entry(id).or_default();
        if !evidence.contains(&evidence_id) {
            evidence.push(evidence_id);
        }
    }
}

impl DdmProfile {
    pub fn parse_yaml(text: &str) -> Result<Self, MemoryError> {
        let profile: Self = yaml_serde::from_str(text)
            .map_err(|error| MemoryError::InvalidAccess(format!("invalid DDM profile: {error}")))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != "momo.ddm/1" {
            return Err(invalid("DDM schema must be momo.ddm/1"));
        }
        if self.character_id.trim().is_empty() || self.revision == 0 {
            return Err(invalid(
                "DDM character_id must be non-empty and revision must be positive",
            ));
        }
        if self.dispositions.is_empty() || self.dispositions.len() > 64 {
            return Err(invalid("DDM must define 1 to 64 dispositions"));
        }
        if !(1..=16).contains(&self.selection.top_k)
            || !valid_activation(self.selection.salient_threshold)
            || !valid_activation(self.selection.dominant_threshold)
            || self.selection.salient_threshold >= self.selection.dominant_threshold
            || !self.selection.hysteresis_margin.is_finite()
            || !(0.0..=0.25).contains(&self.selection.hysteresis_margin)
            || self.selection.hysteresis_margin >= self.selection.salient_threshold
        {
            return Err(invalid(
                "DDM selection must use top_k 1..=16, ordered thresholds within [0, 1], and hysteresis_margin within [0, 0.25] below the salient threshold",
            ));
        }
        let mut disposition_ids = std::collections::BTreeSet::new();
        let mut rule_ids = std::collections::BTreeSet::new();
        for disposition in &self.dispositions {
            if disposition.id.trim().is_empty()
                || !disposition_ids.insert(disposition.id.as_str())
                || !valid_activation(disposition.base_activation)
                || disposition
                    .exclusive_group
                    .as_deref()
                    .is_some_and(|value| !valid_identifier(value))
                || disposition.expression.latent.trim().is_empty()
                || disposition.expression.salient.trim().is_empty()
                || disposition.expression.dominant.trim().is_empty()
                || disposition
                    .constraints
                    .iter()
                    .any(|value| value.trim().is_empty())
            {
                return Err(invalid(format!(
                    "invalid DDM disposition {}",
                    disposition.id
                )));
            }
            for rule in disposition
                .modulation
                .context
                .iter()
                .chain(&disposition.modulation.state)
            {
                let effect_valid = match self.profile {
                    DdmCalculationProfile::LogitAdditive => {
                        rule.delta
                            .is_some_and(|value| value.is_finite() && (-8.0..=8.0).contains(&value))
                            && rule.multiplier.is_none()
                    }
                    DdmCalculationProfile::Multiplicative => {
                        rule.multiplier
                            .is_some_and(|value| value.is_finite() && (0.0..=4.0).contains(&value))
                            && rule.delta.is_none()
                    }
                };
                if rule.id.trim().is_empty()
                    || rule.signal.trim().is_empty()
                    || !rule_ids.insert(rule.id.as_str())
                    || !valid_signal(&rule.signal, &rule.when)
                    || !signal_value_valid(&rule.when)
                    || !effect_valid
                {
                    return Err(invalid(format!("invalid DDM rule {}", rule.id)));
                }
            }
        }
        Ok(())
    }

    pub fn evaluate(&self, signals: &DdmSignalSnapshot) -> DdmAudit {
        self.evaluate_with_previous(signals, &BTreeMap::new())
    }

    #[must_use]
    pub fn evaluate_with_previous(
        &self,
        signals: &DdmSignalSnapshot,
        previous_bands: &BTreeMap<String, DdmBand>,
    ) -> DdmAudit {
        let mut constraints = self
            .dispositions
            .iter()
            .flat_map(|disposition| disposition.constraints.iter().cloned())
            .collect::<Vec<_>>();
        constraints.sort();
        constraints.dedup();
        let mut effective = self
            .dispositions
            .iter()
            .map(|disposition| {
                evaluate_disposition(
                    self,
                    disposition,
                    signals,
                    previous_bands.get(&disposition.id).copied(),
                )
            })
            .collect::<Vec<_>>();
        effective.sort_by(|left, right| {
            right
                .effective_activation
                .total_cmp(&left.effective_activation)
                .then_with(|| left.id.cmp(&right.id))
        });
        let next_bands = effective
            .iter()
            .map(|disposition| (disposition.id.clone(), disposition.band))
            .collect::<BTreeMap<_, _>>();
        let hysteresis_applied = effective
            .iter()
            .filter_map(|disposition| {
                let previous = previous_bands.get(&disposition.id)?;
                let raw = raw_band(self, disposition.effective_activation);
                (*previous != raw && disposition.band == *previous).then(|| disposition.id.clone())
            })
            .collect::<Vec<_>>();
        let groups = self
            .dispositions
            .iter()
            .map(|disposition| {
                (
                    disposition.id.as_str(),
                    disposition.exclusive_group.as_deref(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut selected_groups = BTreeSet::new();
        let mut suppressed_disposition_ids = Vec::new();
        effective.retain(|disposition| {
            let Some(group) = groups.get(disposition.id.as_str()).copied().flatten() else {
                return true;
            };
            if selected_groups.insert(group) {
                true
            } else {
                suppressed_disposition_ids.push(disposition.id.clone());
                false
            }
        });
        if effective.len() > self.selection.top_k {
            suppressed_disposition_ids.extend(
                effective[self.selection.top_k..]
                    .iter()
                    .map(|disposition| disposition.id.clone()),
            );
            effective.truncate(self.selection.top_k);
        }
        suppressed_disposition_ids.sort();
        let source_fingerprint = evaluation_fingerprint(self, signals, previous_bands);
        DdmAudit {
            character_id: self.character_id.clone(),
            profile_revision: self.revision,
            constraints,
            effective_dispositions: effective,
            suppressed_disposition_ids,
            previous_bands: previous_bands.clone(),
            next_bands,
            hysteresis_applied,
            source_fingerprint,
        }
    }
}

fn evaluate_disposition(
    profile: &DdmProfile,
    disposition: &DdmDisposition,
    signals: &DdmSignalSnapshot,
    previous_band: Option<DdmBand>,
) -> EffectiveDisposition {
    let (context_effect, mut matched_rule_ids, mut evidence_ids) =
        matched_effect(&disposition.modulation.context, signals, profile.profile);
    let (state_effect, state_rules, state_evidence) =
        matched_effect(&disposition.modulation.state, signals, profile.profile);
    matched_rule_ids.extend(state_rules);
    evidence_ids.extend(state_evidence);
    evidence_ids.sort();
    evidence_ids.dedup();
    let effective_activation = match profile.profile {
        DdmCalculationProfile::LogitAdditive => {
            let base = disposition.base_activation.clamp(1e-6, 1.0 - 1e-6);
            let logit = (base / (1.0 - base)).ln();
            1.0 / (1.0 + (-(logit + context_effect + state_effect)).exp())
        }
        DdmCalculationProfile::Multiplicative => {
            (disposition.base_activation * context_effect * state_effect).clamp(0.0, 1.0)
        }
    };
    let band = hysteretic_band(profile, effective_activation, previous_band);
    let cue = match band {
        DdmBand::Latent => &disposition.expression.latent,
        DdmBand::Salient => &disposition.expression.salient,
        DdmBand::Dominant => &disposition.expression.dominant,
    };
    EffectiveDisposition {
        id: disposition.id.clone(),
        base_activation: disposition.base_activation,
        context_effect,
        state_effect,
        effective_activation,
        band,
        cue: cue.trim().to_owned(),
        constraints: disposition.constraints.clone(),
        matched_rule_ids,
        evidence_ids,
    }
}

fn raw_band(profile: &DdmProfile, effective_activation: f64) -> DdmBand {
    if effective_activation >= profile.selection.dominant_threshold {
        DdmBand::Dominant
    } else if effective_activation >= profile.selection.salient_threshold {
        DdmBand::Salient
    } else {
        DdmBand::Latent
    }
}

fn hysteretic_band(
    profile: &DdmProfile,
    effective_activation: f64,
    previous_band: Option<DdmBand>,
) -> DdmBand {
    let margin = profile.selection.hysteresis_margin;
    match previous_band {
        None | Some(DdmBand::Latent) => raw_band(profile, effective_activation),
        Some(DdmBand::Salient) => {
            if effective_activation >= profile.selection.dominant_threshold {
                DdmBand::Dominant
            } else if effective_activation < (profile.selection.salient_threshold - margin).max(0.0)
            {
                DdmBand::Latent
            } else {
                DdmBand::Salient
            }
        }
        Some(DdmBand::Dominant) => {
            if effective_activation >= (profile.selection.dominant_threshold - margin).max(0.0) {
                DdmBand::Dominant
            } else if effective_activation
                >= (profile.selection.salient_threshold - margin).max(0.0)
            {
                DdmBand::Salient
            } else {
                DdmBand::Latent
            }
        }
    }
}

fn evaluation_fingerprint(
    profile: &DdmProfile,
    signals: &DdmSignalSnapshot,
    previous_bands: &BTreeMap<String, DdmBand>,
) -> String {
    let bytes = serde_json::to_vec(&(profile, signals, previous_bands))
        .expect("serializing validated DDM inputs cannot fail");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn matched_effect(
    rules: &[DdmRule],
    signals: &DdmSignalSnapshot,
    profile: DdmCalculationProfile,
) -> (f64, Vec<String>, Vec<String>) {
    let mut effect = match profile {
        DdmCalculationProfile::LogitAdditive => 0.0,
        DdmCalculationProfile::Multiplicative => 1.0,
    };
    let mut matched = Vec::new();
    let mut evidence = Vec::new();
    for rule in rules {
        let Some(actual) = signals.values.get(&rule.signal) else {
            continue;
        };
        if !signal_matches(actual, &rule.when) {
            continue;
        }
        match profile {
            DdmCalculationProfile::LogitAdditive => effect += rule.delta.unwrap_or_default(),
            DdmCalculationProfile::Multiplicative => effect *= rule.multiplier.unwrap_or(1.0),
        }
        matched.push(rule.id.clone());
        if let Some(ids) = signals.evidence.get(&rule.signal) {
            evidence.extend(ids.iter().cloned());
        }
    }
    (effect, matched, evidence)
}

fn signal_matches(actual: &DdmSignalValue, expected: &DdmSignalValue) -> bool {
    match (actual, expected) {
        (DdmSignalValue::Bool(left), DdmSignalValue::Bool(right)) => left == right,
        (DdmSignalValue::Number(left), DdmSignalValue::Number(right)) => {
            (*left - *right).abs() <= f64::EPSILON
        }
        (DdmSignalValue::Text(left), DdmSignalValue::Text(right)) => {
            normalize(left) == normalize(right)
        }
        _ => false,
    }
}

fn signal_value_valid(value: &DdmSignalValue) -> bool {
    match value {
        DdmSignalValue::Bool(_) => true,
        DdmSignalValue::Number(value) => value.is_finite(),
        DdmSignalValue::Text(value) => !value.trim().is_empty(),
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b':')
        })
}

fn valid_signal(signal: &str, value: &DdmSignalValue) -> bool {
    let boolean_family = [
        "dmw.tag.",
        "dmw.kind.",
        "nsg.node.",
        "state.dimension.",
        "scene.participant.",
        "scene.source_ref.",
    ]
    .iter()
    .any(|prefix| signal.strip_prefix(prefix).is_some_and(valid_identifier));
    if boolean_family {
        return matches!(value, DdmSignalValue::Bool(_));
    }
    match (signal, value) {
        ("scene.status", DdmSignalValue::Text(value)) => matches!(
            normalize(value).as_str(),
            "inactive" | "active" | "transitioning" | "closed"
        ),
        ("request.event_type", DdmSignalValue::Text(value)) => {
            matches!(normalize(value).as_str(), "user_message" | "tool_result")
        }
        ("request.has_image", DdmSignalValue::Bool(_)) => true,
        _ => false,
    }
}

fn valid_activation(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn normalize(value: &str) -> String {
    value.nfkc().collect::<String>().trim().to_lowercase()
}

fn invalid(message: impl Into<String>) -> MemoryError {
    MemoryError::InvalidAccess(message.into())
}

const fn default_top_k() -> usize {
    DEFAULT_TOP_K
}

const fn default_salient_threshold() -> f64 {
    DEFAULT_SALIENT_THRESHOLD
}

const fn default_dominant_threshold() -> f64 {
    DEFAULT_DOMINANT_THRESHOLD
}

const fn default_hysteresis_margin() -> f64 {
    DEFAULT_HYSTERESIS_MARGIN
}

#[cfg(test)]
mod tests {
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
        assert!(
            DdmProfile::parse_yaml(&yaml.replace("request.has_image", "request.intent")).is_err()
        );
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
}
