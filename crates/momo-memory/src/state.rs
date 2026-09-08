//! Deterministic MO State compiler.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use super::{MemoryError, MemoryWorkspace, RetrievedMemory, TokenCounter};
use crate::nsg::RetrievedNsg;

const CONTRACT_MAX_SIZE: u64 = 65_536;
const STATE_CONTEXT_TOKEN_RATIO: usize = 10;
const DEFAULT_CONTRACT: &str = r#"
version: 1
dimensions:
  relational_stance:
    signal_source: "dmw"
    match_mode: "first"
    rules:
      - id: "stance_conflict"
        condition:
          weight_min: 0.3
          tags_any: ["conflict", "tension"]
        directives:
          - "保持关系张力，不要过快和解。"
      - id: "stance_attachment"
        condition:
          weight_min: 0.6
          tags_any: ["attachment", "trust", "dependency"]
        directives:
          - "表达上保留亲近感与连续的关系记忆。"
  emotional_tone:
    signal_source: "dmw"
    match_mode: "first"
    rules:
      - id: "tone_loss"
        condition:
          signal_tags_any: ["loss", "farewell", "regret"]
        directives:
          - "语气放轻，避免轻佻或突兀转移话题。"
      - id: "tone_danger"
        condition:
          signal_tags_any: ["danger", "fear", "threat"]
        directives:
          - "语气保持警觉，优先回应眼前风险。"
  scene_constraint:
    signal_source: "dmw+nsg"
    match_mode: "accumulate"
    rules: []
  physiological_state:
    signal_source: "dmw+nsg"
    match_mode: "first"
    rules:
      - id: "phys_wounded"
        condition:
          dmw_event_tag: "wounded"
          nsg_constraint_match: "injury"
        directives:
          - "行动描写必须体现受伤后的迟滞与体力限制。"
  epistemic_state:
    signal_source: "dmw"
    match_mode: "first"
    rules:
      - id: "epistemic_secret_absence"
        condition:
          mode: "absence"
          event_tag: "secret"
          required_witness_tag: "witness"
        directives:
          - "不要让角色知道其未见证的秘密事件。"
conflict_priority:
  - "scene_constraint"
  - "physiological_state"
  - "epistemic_state"
  - "relational_stance"
  - "emotional_tone"
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MoStateAudit {
    pub timestamp: i64,
    pub contract_version: u32,
    pub contract_source: String,
    pub dimensions_evaluated: usize,
    pub dimensions_active: usize,
    pub matched_rules: Vec<String>,
    pub conflicts_resolved: usize,
    pub directives_emitted: usize,
    pub token_count: usize,
    pub degraded: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MoStateContext {
    pub context: String,
    pub audit: MoStateAudit,
}

#[derive(Debug, Clone)]
struct DmwSignal {
    id: String,
    kind: String,
    weight: f64,
    touch_at: i64,
    tags: BTreeSet<String>,
    relations: BTreeMap<String, Vec<String>>,
    body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateContract {
    #[serde(default = "default_version")]
    version: u32,
    dimensions: BTreeMap<String, DimensionContract>,
    #[serde(default = "default_priority")]
    conflict_priority: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DimensionContract {
    #[serde(default)]
    signal_source: Option<String>,
    #[serde(default = "default_match_mode")]
    match_mode: String,
    #[serde(default)]
    rules: Vec<StateRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateRule {
    id: String,
    #[serde(default)]
    condition: RuleCondition,
    directives: Vec<String>,
    #[serde(default)]
    conflicts_with: Vec<String>,
    #[serde(default)]
    conflict_resolution: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RuleCondition {
    #[serde(default)]
    weight_min: Option<f64>,
    #[serde(default)]
    weight_max: Option<f64>,
    #[serde(default)]
    tags_any: Vec<String>,
    #[serde(default)]
    tags_all: Vec<String>,
    #[serde(default)]
    signal_tags_any: Vec<String>,
    #[serde(default)]
    signal_tags_all: Vec<String>,
    #[serde(default)]
    dmw_event_tag: Option<String>,
    #[serde(default)]
    nsg_constraint_match: Option<String>,
    #[serde(default)]
    nsg_consequence_ref: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    event_tag: Option<String>,
    #[serde(default)]
    required_witness_tag: Option<String>,
    #[serde(default)]
    character_ref: Option<String>,
}

fn default_version() -> u32 {
    1
}

fn default_match_mode() -> String {
    "first".to_owned()
}

fn default_priority() -> Vec<String> {
    [
        "scene_constraint",
        "physiological_state",
        "epistemic_state",
        "relational_stance",
        "emotional_tone",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

impl MemoryWorkspace {
    pub fn compile_mo_state(
        &self,
        retrieved_memory: &[RetrievedMemory],
        retrieved_nsg: &[RetrievedNsg],
        max_context_tokens: usize,
        counter: &impl TokenCounter,
    ) -> Result<MoStateContext, MemoryError> {
        let mut audit = MoStateAudit {
            timestamp: Utc::now().timestamp(),
            contract_version: 1,
            contract_source: "builtin_default".to_owned(),
            dimensions_evaluated: 5,
            ..MoStateAudit::default()
        };
        let mut contract: StateContract = yaml_serde::from_str(DEFAULT_CONTRACT)
            .map_err(|error| MemoryError::InvalidPatch(error.to_string()))?;
        match self.load_user_state_contract(&mut audit) {
            Ok(Some(user)) => {
                merge_contract(&mut contract, user, &mut audit);
                audit.contract_source = "builtin_default+user_override".to_owned();
            }
            Ok(None) => {}
            Err(error) => {
                audit.degraded = true;
                audit.warnings.push(error.to_string());
            }
        }
        audit.contract_version = contract.version;

        let signals = loaded_dmw_signals(retrieved_memory, &mut audit);
        let mut directives = BTreeMap::<String, Vec<(String, String)>>::new();
        add_scene_constraints(&signals, retrieved_nsg, &mut directives);
        evaluate_rules(
            &contract,
            &signals,
            retrieved_nsg,
            &mut directives,
            &mut audit,
        );
        let mut ordered = order_directives(&contract, directives, &mut audit);
        let budget = max_context_tokens.saturating_mul(STATE_CONTEXT_TOKEN_RATIO) / 100;
        if trim_to_budget(&mut ordered, budget, counter) {
            audit.degraded = true;
            audit.warnings.push(
                "scene constraints alone exceed the MO State token budget; state context omitted"
                    .to_owned(),
            );
        }
        audit.dimensions_active = ordered.len();
        audit.directives_emitted = ordered.iter().map(|(_, values)| values.len()).sum();
        let context = format_state_context(&ordered);
        audit.token_count = counter.count(&context);
        Ok(MoStateContext { context, audit })
    }

    fn load_user_state_contract(
        &self,
        audit: &mut MoStateAudit,
    ) -> Result<Option<StateContract>, MemoryError> {
        let path = self.root().join("config/state_contract.yaml");
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > CONTRACT_MAX_SIZE => Err(MemoryError::InvalidAccess(
                "state_contract.yaml exceeds 64 KiB".to_owned(),
            )),
            Ok(_) => {
                let text = fs::read_to_string(path)?;
                let value: yaml_serde::Value = yaml_serde::from_str(&text)
                    .map_err(|error| MemoryError::InvalidAccess(error.to_string()))?;
                parse_user_contract(value, audit).map(Some)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

fn parse_user_contract(
    value: yaml_serde::Value,
    audit: &mut MoStateAudit,
) -> Result<StateContract, MemoryError> {
    let mapping = value.as_mapping().ok_or_else(|| {
        MemoryError::InvalidAccess("state contract root must be a mapping".to_owned())
    })?;
    let version = match mapping.get("version").and_then(yaml_serde::Value::as_u64) {
        Some(version) => u32::try_from(version).unwrap_or(u32::MAX),
        None => {
            warn_contract(
                audit,
                "state contract version is missing or invalid; using version 1".to_owned(),
            );
            1
        }
    };
    let dimensions = mapping
        .get("dimensions")
        .and_then(yaml_serde::Value::as_mapping)
        .ok_or_else(|| {
            MemoryError::InvalidAccess("state contract dimensions must be a mapping".to_owned())
        })?;
    let mut parsed_dimensions = BTreeMap::new();
    for (name, raw_dimension) in dimensions {
        let Some(name) = name.as_str() else {
            warn_contract(
                audit,
                "state contract contains a non-string dimension key; dimension ignored".to_owned(),
            );
            continue;
        };
        if let Some(dimension) = parse_user_dimension(name, raw_dimension, audit) {
            parsed_dimensions.insert(name.to_owned(), dimension);
        }
    }
    let conflict_priority = match mapping.get("conflict_priority") {
        Some(value) => {
            yaml_serde::from_value::<Vec<String>>(value.clone()).unwrap_or_else(|error| {
                warn_contract(
                    audit,
                    format!("invalid conflict_priority: {error}; using default"),
                );
                default_priority()
            })
        }
        None => default_priority(),
    };
    Ok(StateContract {
        version,
        dimensions: parsed_dimensions,
        conflict_priority,
    })
}

fn parse_user_dimension(
    name: &str,
    value: &yaml_serde::Value,
    audit: &mut MoStateAudit,
) -> Option<DimensionContract> {
    let Some(mapping) = value.as_mapping() else {
        warn_contract(
            audit,
            format!("state dimension {name} is not a mapping; dimension ignored"),
        );
        return None;
    };
    let signal_source = mapping
        .get("signal_source")
        .and_then(yaml_serde::Value::as_str)
        .map(str::to_owned);
    let match_mode = mapping
        .get("match_mode")
        .and_then(yaml_serde::Value::as_str)
        .unwrap_or("first")
        .to_owned();
    let mut rules = Vec::new();
    if let Some(raw_rules) = mapping.get("rules") {
        let Some(sequence) = raw_rules.as_sequence() else {
            warn_contract(
                audit,
                format!("state dimension {name} rules is not an array; rules ignored"),
            );
            return Some(DimensionContract {
                signal_source,
                match_mode,
                rules,
            });
        };
        for (index, raw_rule) in sequence.iter().enumerate() {
            match yaml_serde::from_value::<StateRule>(raw_rule.clone()) {
                Ok(rule) => rules.push(rule),
                Err(error) => warn_contract(
                    audit,
                    format!(
                        "invalid rule at index {index} in dimension {name}: {error}; rule ignored"
                    ),
                ),
            }
        }
    }
    Some(DimensionContract {
        signal_source,
        match_mode,
        rules,
    })
}

fn loaded_dmw_signals(retrieved: &[RetrievedMemory], audit: &mut MoStateAudit) -> Vec<DmwSignal> {
    let mut signals = Vec::new();
    for memory in retrieved {
        let Some(state) = &memory.state_signal else {
            audit.degraded = true;
            audit.warnings.push(format!(
                "retrieved memory {} has no embedded MO State signal metadata",
                memory.id
            ));
            continue;
        };
        signals.push(DmwSignal {
            id: memory.id.clone(),
            kind: state.kind.clone(),
            weight: state.weight,
            touch_at: state.touch_at,
            tags: state.tags.iter().map(|value| normalize(value)).collect(),
            relations: state.relations.clone(),
            body: memory.body.clone(),
        });
    }
    signals
}

fn merge_contract(base: &mut StateContract, user: StateContract, audit: &mut MoStateAudit) {
    if user.version == 1 {
        base.version = user.version;
    } else {
        warn_contract(
            audit,
            format!(
                "unsupported state contract version {}; using version 1",
                user.version
            ),
        );
    }
    for (dimension, mut contract) in user.dimensions {
        if !default_priority().contains(&dimension) {
            warn_contract(
                audit,
                format!("unknown state dimension {dimension}; override ignored"),
            );
            continue;
        }
        if sanitize_dimension(&dimension, &mut contract, audit) {
            base.dimensions.insert(dimension, contract);
        }
    }
    if valid_priority(&user.conflict_priority) {
        base.conflict_priority = user.conflict_priority;
    } else {
        warn_contract(
            audit,
            "conflict_priority is not an exact five-dimension permutation; using default"
                .to_owned(),
        );
    }
    remove_duplicate_rule_ids(base, audit);
}

fn valid_priority(priority: &[String]) -> bool {
    let expected = default_priority().into_iter().collect::<BTreeSet<_>>();
    priority.len() == expected.len()
        && priority.iter().cloned().collect::<BTreeSet<_>>() == expected
}

fn warn_contract(audit: &mut MoStateAudit, warning: String) {
    audit.degraded = true;
    audit.warnings.push(warning);
}

fn sanitize_dimension(
    dimension: &str,
    contract: &mut DimensionContract,
    audit: &mut MoStateAudit,
) -> bool {
    let expected_source = match dimension {
        "relational_stance" | "emotional_tone" | "epistemic_state" => "dmw",
        "scene_constraint" | "physiological_state" => "dmw+nsg",
        _ => return false,
    };
    if contract.signal_source.as_deref() != Some(expected_source) {
        warn_contract(
            audit,
            format!("dimension {dimension} has invalid signal_source; built-in dimension retained"),
        );
        return false;
    }
    if !matches!(contract.match_mode.as_str(), "first" | "accumulate") {
        warn_contract(
            audit,
            format!("dimension {dimension} has invalid match_mode; built-in dimension retained"),
        );
        return false;
    }
    if contract.rules.len() > 20 {
        warn_contract(
            audit,
            format!("dimension {dimension} exceeds 20 rules; extra rules ignored"),
        );
        contract.rules.truncate(20);
    }
    contract.rules.retain(|rule| {
        let valid = !rule.id.trim().is_empty()
            && !rule.directives.is_empty()
            && rule
                .directives
                .iter()
                .all(|directive| !directive.trim().is_empty())
            && rule
                .conflicts_with
                .iter()
                .all(|rule_id| !rule_id.trim().is_empty())
            && rule
                .conflict_resolution
                .iter()
                .all(|directive| !directive.trim().is_empty())
            && condition_valid_for_dimension(dimension, &rule.condition);
        if !valid {
            warn_contract(
                audit,
                format!(
                    "invalid rule {} in dimension {dimension}; rule ignored",
                    if rule.id.trim().is_empty() {
                        "<empty>"
                    } else {
                        rule.id.as_str()
                    }
                ),
            );
        }
        valid
    });
    true
}

fn condition_valid_for_dimension(dimension: &str, condition: &RuleCondition) -> bool {
    let weights_valid = condition.weight_min.is_none_or(f64::is_finite)
        && condition.weight_max.is_none_or(f64::is_finite)
        && match (condition.weight_min, condition.weight_max) {
            (Some(minimum), Some(maximum)) => minimum <= maximum,
            _ => true,
        };
    let no_generic_only_any = |values: &[String]| {
        values.is_empty()
            || values
                .iter()
                .map(|value| normalize(value))
                .any(|value| !is_generic_tag(&value))
    };
    match dimension {
        "relational_stance" => {
            weights_valid
                && no_generic_only_any(&condition.tags_any)
                && condition.signal_tags_any.is_empty()
                && condition.signal_tags_all.is_empty()
                && condition.dmw_event_tag.is_none()
                && condition.nsg_constraint_match.is_none()
                && condition.nsg_consequence_ref.is_none()
                && condition.mode.is_none()
                && condition.event_tag.is_none()
                && condition.required_witness_tag.is_none()
                && condition.character_ref.is_none()
        }
        "emotional_tone" => {
            condition.weight_min.is_none()
                && condition.weight_max.is_none()
                && condition.tags_any.is_empty()
                && condition.tags_all.is_empty()
                && no_generic_only_any(&condition.signal_tags_any)
                && condition.dmw_event_tag.is_none()
                && condition.nsg_constraint_match.is_none()
                && condition.nsg_consequence_ref.is_none()
                && condition.mode.is_none()
                && condition.event_tag.is_none()
                && condition.required_witness_tag.is_none()
                && condition.character_ref.is_none()
        }
        "physiological_state" => {
            condition
                .dmw_event_tag
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && condition
                    .nsg_constraint_match
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                && condition.weight_min.is_none()
                && condition.weight_max.is_none()
                && condition.tags_any.is_empty()
                && condition.tags_all.is_empty()
                && condition.signal_tags_any.is_empty()
                && condition.signal_tags_all.is_empty()
                && condition.mode.is_none()
                && condition.event_tag.is_none()
                && condition.required_witness_tag.is_none()
                && condition.character_ref.is_none()
        }
        "epistemic_state" => {
            let mode = condition.mode.as_deref().unwrap_or("absence");
            let mode_valid = match mode {
                "absence" => condition
                    .event_tag
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
                "misconception" => condition
                    .character_ref
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
                _ => false,
            };
            mode_valid
                && condition.weight_min.is_none()
                && condition.weight_max.is_none()
                && condition.tags_any.is_empty()
                && condition.tags_all.is_empty()
                && condition.signal_tags_any.is_empty()
                && condition.signal_tags_all.is_empty()
                && condition.dmw_event_tag.is_none()
                && condition.nsg_constraint_match.is_none()
                && condition.nsg_consequence_ref.is_none()
        }
        "scene_constraint" => contract_scene_condition_is_empty(condition),
        _ => false,
    }
}

fn contract_scene_condition_is_empty(condition: &RuleCondition) -> bool {
    condition.weight_min.is_none()
        && condition.weight_max.is_none()
        && condition.tags_any.is_empty()
        && condition.tags_all.is_empty()
        && condition.signal_tags_any.is_empty()
        && condition.signal_tags_all.is_empty()
        && condition.dmw_event_tag.is_none()
        && condition.nsg_constraint_match.is_none()
        && condition.nsg_consequence_ref.is_none()
        && condition.mode.is_none()
        && condition.event_tag.is_none()
        && condition.required_witness_tag.is_none()
        && condition.character_ref.is_none()
}

fn remove_duplicate_rule_ids(contract: &mut StateContract, audit: &mut MoStateAudit) {
    let mut seen = BTreeSet::new();
    for dimension in &contract.conflict_priority {
        if let Some(dimension_contract) = contract.dimensions.get_mut(dimension) {
            dimension_contract.rules.retain(|rule| {
                if seen.insert(rule.id.clone()) {
                    true
                } else {
                    warn_contract(
                        audit,
                        format!("duplicate state rule id {}; later rule ignored", rule.id),
                    );
                    false
                }
            });
        }
    }
    let known = contract
        .dimensions
        .values()
        .flat_map(|dimension| dimension.rules.iter().map(|rule| rule.id.clone()))
        .collect::<BTreeSet<_>>();
    for dimension in contract.dimensions.values_mut() {
        for rule in &mut dimension.rules {
            rule.conflicts_with.retain(|target| {
                let valid = known.contains(target) || target.starts_with("scene_");
                if !valid {
                    warn_contract(
                        audit,
                        format!(
                            "rule {} references unknown conflict target {target}; reference ignored",
                            rule.id
                        ),
                    );
                }
                valid
            });
            if rule.conflicts_with.is_empty() && !rule.conflict_resolution.is_empty() {
                warn_contract(
                    audit,
                    format!(
                        "rule {} has conflict_resolution without conflicts_with; resolution ignored",
                        rule.id
                    ),
                );
                rule.conflict_resolution.clear();
            }
        }
    }
}

fn add_scene_constraints(
    signals: &[DmwSignal],
    nsg: &[RetrievedNsg],
    directives: &mut BTreeMap<String, Vec<(String, String)>>,
) {
    let mut output = Vec::new();
    for signal in signals.iter().filter(|signal| signal.id == "current_scene") {
        if let Some(environment) = markdown_section(&signal.body, &["Environment", "环境"]) {
            output.push((
                "scene_environment".to_owned(),
                format!("当前环境约束：{}。", collapse_whitespace(&environment)),
            ));
        }
    }
    for node in nsg {
        for line in node.body.lines() {
            if let Some(value) = line.strip_prefix("CONSTRAINT: ")
                && !value.trim().is_empty()
            {
                output.push(("nsg_constraint".to_owned(), value.trim().to_owned()));
            }
            if let Some(value) = line.strip_prefix("CONSEQUENCE: ")
                && !value.trim().is_empty()
            {
                output.push(("nsg_consequence".to_owned(), value.trim().to_owned()));
            }
        }
    }
    if !output.is_empty() {
        directives.insert("scene_constraint".to_owned(), output);
    }
}

fn evaluate_rules(
    contract: &StateContract,
    signals: &[DmwSignal],
    nsg: &[RetrievedNsg],
    directives: &mut BTreeMap<String, Vec<(String, String)>>,
    audit: &mut MoStateAudit,
) {
    for (dimension, dimension_contract) in &contract.dimensions {
        if dimension == "scene_constraint" {
            continue;
        }
        let _source = dimension_contract
            .signal_source
            .as_deref()
            .unwrap_or_default();
        let mut matched = Vec::new();
        for rule in &dimension_contract.rules {
            if rule.directives.is_empty() || !rule_matches(dimension, rule, signals, nsg) {
                continue;
            }
            audit.matched_rules.push(rule.id.clone());
            matched.extend(
                rule.directives
                    .iter()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| (rule.id.clone(), value.trim().to_owned())),
            );
            if dimension_contract.match_mode != "accumulate" {
                break;
            }
        }
        if !matched.is_empty() {
            directives.insert(dimension.clone(), matched);
        }
    }
}

fn rule_matches(
    dimension: &str,
    rule: &StateRule,
    signals: &[DmwSignal],
    nsg: &[RetrievedNsg],
) -> bool {
    match dimension {
        "relational_stance" => signals
            .iter()
            .filter(|signal| signal.kind == "relationship")
            .any(|signal| weight_and_tags_match(signal, &rule.condition)),
        "emotional_tone" => {
            let mut events = signals
                .iter()
                .filter(|signal| signal.kind == "event")
                .collect::<Vec<_>>();
            events.sort_by(|left, right| right.weight.total_cmp(&left.weight));
            let tags = events
                .into_iter()
                .take(3)
                .flat_map(|signal| signal.tags.iter().cloned())
                .collect::<BTreeSet<_>>();
            tags_match(
                &tags,
                &rule.condition.signal_tags_any,
                &rule.condition.signal_tags_all,
            )
        }
        "physiological_state" => physiological_match(&rule.condition, signals, nsg),
        "epistemic_state" => epistemic_match(&rule.condition, signals),
        _ => false,
    }
}

fn weight_and_tags_match(signal: &DmwSignal, condition: &RuleCondition) -> bool {
    if condition
        .weight_min
        .is_some_and(|minimum| signal.weight < minimum)
        || condition
            .weight_max
            .is_some_and(|maximum| signal.weight > maximum)
    {
        return false;
    }
    tags_match(&signal.tags, &condition.tags_any, &condition.tags_all)
}

fn tags_match(tags: &BTreeSet<String>, any: &[String], all: &[String]) -> bool {
    let any_matches = any.is_empty()
        || any
            .iter()
            .map(|value| normalize(value))
            .any(|value| !is_generic_tag(&value) && tags.contains(&value));
    let all_matches = all
        .iter()
        .map(|value| normalize(value))
        .all(|value| !is_generic_tag(&value) && tags.contains(&value));
    any_matches && all_matches
}

fn physiological_match(
    condition: &RuleCondition,
    signals: &[DmwSignal],
    nsg: &[RetrievedNsg],
) -> bool {
    let Some(event_tag) = condition.dmw_event_tag.as_deref().map(normalize) else {
        return false;
    };
    if !signals
        .iter()
        .any(|signal| signal.kind == "event" && signal.tags.contains(&event_tag))
    {
        return false;
    }
    if let Some(needle) = condition.nsg_constraint_match.as_deref().map(normalize) {
        let nsg_match = nsg
            .iter()
            .any(|node| normalize(&node.body).contains(&needle));
        if !nsg_match {
            return false;
        }
    }
    condition
        .nsg_consequence_ref
        .as_ref()
        .is_none_or(|id| nsg.iter().any(|node| node.id == *id))
}

fn epistemic_match(condition: &RuleCondition, signals: &[DmwSignal]) -> bool {
    match condition.mode.as_deref().unwrap_or("absence") {
        "absence" => {
            let Some(event_tag) = condition.event_tag.as_deref().map(normalize) else {
                return false;
            };
            let witness = condition
                .required_witness_tag
                .as_deref()
                .map(normalize)
                .unwrap_or_else(|| "witness".to_owned());
            signals.iter().any(|signal| {
                if signal.kind != "event" || !signal.tags.contains(&event_tag) {
                    return false;
                }
                let has_witness_marker =
                    signal.tags.contains(&witness) || normalize(&signal.body).contains(&witness);
                let witness_is_target = condition
                    .character_ref
                    .as_deref()
                    .is_none_or(|character| has_relation(signal, character));
                !(has_witness_marker && witness_is_target)
            })
        }
        "misconception" => signals.iter().any(|signal| {
            let Some(character) = condition.character_ref.as_deref() else {
                return false;
            };
            signal.kind == "event"
                && signal.tags.contains("misconception")
                && has_relation(signal, character)
                && !signals.iter().any(|candidate| {
                    candidate.kind == "event"
                        && candidate.tags.contains("corrected")
                        && candidate.touch_at > signal.touch_at
                        && (has_relation(candidate, &signal.id)
                            || has_relation(candidate, character))
                })
        }),
        _ => false,
    }
}

fn has_relation(signal: &DmwSignal, target: &str) -> bool {
    signal
        .relations
        .values()
        .flatten()
        .any(|value| value == target)
}

fn order_directives(
    contract: &StateContract,
    directives: BTreeMap<String, Vec<(String, String)>>,
    audit: &mut MoStateAudit,
) -> Vec<(String, Vec<String>)> {
    let mut accepted_rules = BTreeSet::<String>::new();
    let mut resolved_rules = BTreeSet::<String>::new();
    let mut ordered = Vec::new();
    for dimension in &contract.conflict_priority {
        let Some(values) = directives.get(dimension) else {
            continue;
        };
        let mut surviving = Vec::new();
        for (rule_id, directive) in values {
            let conflicts = accepted_rules
                .iter()
                .any(|accepted| rules_conflict(contract, rule_id, accepted));
            if conflicts {
                if resolved_rules.insert(rule_id.clone()) {
                    audit.conflicts_resolved += 1;
                    if let Some(rule) = find_rule(contract, rule_id) {
                        surviving.extend(rule.conflict_resolution.iter().cloned());
                    }
                }
                continue;
            }
            accepted_rules.insert(rule_id.clone());
            surviving.push(directive.clone());
        }
        if !surviving.is_empty() {
            ordered.push((dimension.clone(), surviving));
        }
    }
    ordered
}

fn find_rule<'a>(contract: &'a StateContract, rule_id: &str) -> Option<&'a StateRule> {
    contract
        .dimensions
        .values()
        .flat_map(|dimension| &dimension.rules)
        .find(|rule| rule.id == rule_id)
}

fn rules_conflict(contract: &StateContract, left: &str, right: &str) -> bool {
    find_rule(contract, left)
        .is_some_and(|rule| rule.conflicts_with.iter().any(|target| target == right))
        || find_rule(contract, right)
            .is_some_and(|rule| rule.conflicts_with.iter().any(|target| target == left))
}

fn trim_to_budget(
    ordered: &mut Vec<(String, Vec<String>)>,
    budget: usize,
    counter: &impl TokenCounter,
) -> bool {
    for removable in [
        "emotional_tone",
        "relational_stance",
        "epistemic_state",
        "physiological_state",
    ] {
        if counter.count(&format_state_context(ordered)) <= budget {
            return false;
        }
        ordered.retain(|(dimension, _)| dimension != removable);
    }
    if counter.count(&format_state_context(ordered)) > budget {
        ordered.clear();
        return true;
    }
    false
}

fn format_state_context(ordered: &[(String, Vec<String>)]) -> String {
    if ordered.is_empty() {
        return String::new();
    }
    let mut output =
        "[STATE_CONTEXT: MO State v1.0]\n\n以下行为约束由状态编译器生成，你必须严格遵守。任何违背均视为生成失败。\n"
            .to_owned();
    for (dimension, values) in ordered {
        output.push_str("\n## ");
        output.push_str(match dimension.as_str() {
            "scene_constraint" => "场景约束",
            "physiological_state" => "生理状态",
            "epistemic_state" => "认知掩码",
            "relational_stance" => "关系姿态",
            "emotional_tone" => "情绪基调",
            _ => dimension,
        });
        output.push('\n');
        for value in values {
            output.push_str("- ");
            output.push_str(value);
            output.push('\n');
        }
    }
    output.push_str("\n[/STATE_CONTEXT]");
    output
}

fn markdown_section(text: &str, headings: &[&str]) -> Option<String> {
    let mut capture = false;
    let mut output = Vec::new();
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ").map(str::trim) {
            if capture {
                break;
            }
            capture = headings
                .iter()
                .any(|heading| normalize(heading) == normalize(title));
            continue;
        }
        if capture {
            output.push(line);
        }
    }
    let section = output.join("\n").trim().to_owned();
    (!section.is_empty()).then_some(section)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_generic_tag(value: &str) -> bool {
    value.chars().count() <= 1 || matches!(value, "角色" | "主角" | "城市" | "魔法")
}

fn normalize(value: &str) -> String {
    value.nfkc().collect::<String>().trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConservativeTokenCounter, RetrievedStateSignal};
    use std::path::PathBuf;

    fn retrieved_signal(
        id: &str,
        kind: &str,
        weight: f64,
        touch_at: i64,
        tags: &[&str],
        relations: BTreeMap<String, Vec<String>>,
        body: &str,
    ) -> RetrievedMemory {
        RetrievedMemory {
            id: id.to_owned(),
            path: PathBuf::from(format!("missing/{id}.md")),
            body: body.to_owned(),
            estimated_tokens: body.chars().count(),
            source_character_ids: Vec::new(),
            injection_scope: None,
            injection_conversation_id: None,
            injection_character_id: None,
            state_signal: Some(RetrievedStateSignal {
                kind: kind.to_owned(),
                weight,
                touch_at,
                tags: tags.iter().map(|value| (*value).to_owned()).collect(),
                relations,
            }),
        }
    }

    #[test]
    fn formats_empty_context_when_no_signals_match() {
        let root = tempfile::tempdir().expect("root");
        let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
        let context = workspace
            .compile_mo_state(&[], &[], 1024, &ConservativeTokenCounter)
            .expect("compile");
        assert!(context.context.is_empty());
        assert!(!context.audit.degraded);
    }

    #[test]
    fn compiles_only_from_the_retrieval_snapshot() {
        let root = tempfile::tempdir().expect("root");
        let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
        let memory = retrieved_signal(
            "relationship_test",
            "relationship",
            0.8,
            1,
            &["conflict"],
            BTreeMap::new(),
            "not present on disk",
        );
        let context = workspace
            .compile_mo_state(&[memory], &[], 100_000, &ConservativeTokenCounter)
            .expect("compile without re-reading missing path");
        assert!(context.context.contains("保持关系张力"));
        assert!(!context.audit.degraded);
    }

    #[test]
    fn invalid_override_rules_degrade_locally_and_priority_must_be_exact() {
        let root = tempfile::tempdir().expect("root");
        let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
        fs::write(
            root.path().join("config/state_contract.yaml"),
            r#"
version: 1
dimensions:
  relational_stance:
    signal_source: dmw
    match_mode: first
    rules:
      - id: invalid_condition
        condition:
          unknown_filter: [conflict]
        directives: ["must not appear"]
      - id: valid_condition
        condition:
          tags_any: [conflict]
        directives: ["valid override"]
conflict_priority:
  - scene_constraint
  - scene_constraint
  - physiological_state
  - epistemic_state
  - relational_stance
  - emotional_tone
"#,
        )
        .expect("override");
        let memory = retrieved_signal(
            "relationship_test",
            "relationship",
            0.8,
            1,
            &["conflict"],
            BTreeMap::new(),
            "body",
        );
        let context = workspace
            .compile_mo_state(&[memory], &[], 100_000, &ConservativeTokenCounter)
            .expect("compile");
        assert!(context.context.contains("valid override"));
        assert!(!context.context.contains("must not appear"));
        assert!(context.audit.degraded);
        assert!(
            context
                .audit
                .warnings
                .iter()
                .any(|warning| warning.contains("exact five-dimension permutation"))
        );
    }

    #[test]
    fn state_context_never_exceeds_its_hard_budget() {
        let root = tempfile::tempdir().expect("root");
        let workspace = MemoryWorkspace::initialize(root.path()).expect("workspace");
        let scene = retrieved_signal(
            "current_scene",
            "current",
            0.0,
            1,
            &[],
            BTreeMap::new(),
            "## Environment\nThis scene is much larger than the available budget.",
        );
        let context = workspace
            .compile_mo_state(&[scene], &[], 10, &ConservativeTokenCounter)
            .expect("compile");
        assert!(context.context.is_empty());
        assert_eq!(context.audit.token_count, 0);
        assert!(context.audit.degraded);
    }

    #[test]
    fn epistemic_absence_is_scoped_to_the_target_character() {
        let mut relations = BTreeMap::new();
        relations.insert("characters".to_owned(), vec!["char_other".to_owned()]);
        let event = DmwSignal {
            id: "secret".to_owned(),
            kind: "event".to_owned(),
            weight: 1.0,
            touch_at: 1,
            tags: ["secret", "witness"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            relations,
            body: String::new(),
        };
        let condition = RuleCondition {
            mode: Some("absence".to_owned()),
            event_tag: Some("secret".to_owned()),
            required_witness_tag: Some("witness".to_owned()),
            character_ref: Some("char_target".to_owned()),
            ..RuleCondition::default()
        };
        assert!(epistemic_match(&condition, &[event]));
    }

    #[test]
    fn a_later_related_correction_clears_a_misconception() {
        let character = "char_target".to_owned();
        let misconception = DmwSignal {
            id: "misconception_1".to_owned(),
            kind: "event".to_owned(),
            weight: 1.0,
            touch_at: 10,
            tags: ["misconception"].into_iter().map(str::to_owned).collect(),
            relations: BTreeMap::from([("characters".to_owned(), vec![character.clone()])]),
            body: String::new(),
        };
        let correction = DmwSignal {
            id: "correction_1".to_owned(),
            kind: "event".to_owned(),
            weight: 1.0,
            touch_at: 11,
            tags: ["corrected"].into_iter().map(str::to_owned).collect(),
            relations: BTreeMap::from([("events".to_owned(), vec![misconception.id.clone()])]),
            body: String::new(),
        };
        let condition = RuleCondition {
            mode: Some("misconception".to_owned()),
            character_ref: Some(character),
            ..RuleCondition::default()
        };
        assert!(!epistemic_match(&condition, &[misconception, correction]));
    }

    #[test]
    fn duplicate_priority_entries_are_rejected() {
        let mut priority = default_priority();
        priority.push("scene_constraint".to_owned());
        assert!(!valid_priority(&priority));
    }

    #[test]
    fn explicit_rule_conflicts_are_resolved_by_dimension_priority() {
        let mut contract: StateContract =
            yaml_serde::from_str(DEFAULT_CONTRACT).expect("default contract");
        let emotional = contract
            .dimensions
            .get_mut("emotional_tone")
            .expect("emotional dimension");
        let lower_rule = emotional
            .rules
            .iter_mut()
            .find(|rule| rule.id == "tone_loss")
            .expect("loss rule");
        lower_rule.conflicts_with = vec!["stance_conflict".to_owned()];
        lower_rule.conflict_resolution = vec!["以克制语气保留关系张力。".to_owned()];
        let directives = BTreeMap::from([
            (
                "relational_stance".to_owned(),
                vec![("stance_conflict".to_owned(), "保持关系张力。".to_owned())],
            ),
            (
                "emotional_tone".to_owned(),
                vec![("tone_loss".to_owned(), "原始低优先级指令。".to_owned())],
            ),
        ]);
        let mut audit = MoStateAudit::default();
        let ordered = order_directives(&contract, directives, &mut audit);
        let context = format_state_context(&ordered);
        assert!(context.contains("保持关系张力"));
        assert!(context.contains("以克制语气保留关系张力"));
        assert!(!context.contains("原始低优先级指令"));
        assert_eq!(audit.conflicts_resolved, 1);
    }
}
