//! Deterministic MO State compiler.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::PathBuf,
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use super::{MemoryDocument, MemoryError, MemoryWorkspace, RetrievedMemory, TokenCounter};
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
        match self.load_user_state_contract() {
            Ok(Some(user)) => {
                merge_contract(&mut contract, user);
                audit.contract_source = "builtin_default+user_override".to_owned();
            }
            Ok(None) => {}
            Err(error) => {
                audit.degraded = true;
                audit.warnings.push(error.to_string());
            }
        }
        audit.contract_version = contract.version;

        let signals = self.loaded_dmw_signals(retrieved_memory, &mut audit)?;
        let mut directives = BTreeMap::<String, Vec<(String, String)>>::new();
        add_scene_constraints(&signals, retrieved_nsg, &mut directives);
        evaluate_rules(
            &contract,
            &signals,
            retrieved_nsg,
            &mut directives,
            &mut audit,
        );
        let mut ordered = order_directives(&contract, directives);
        let budget = max_context_tokens.saturating_mul(STATE_CONTEXT_TOKEN_RATIO) / 100;
        trim_to_budget(&mut ordered, budget, counter);
        audit.dimensions_active = ordered.len();
        audit.directives_emitted = ordered.iter().map(|(_, values)| values.len()).sum();
        let context = format_state_context(&ordered);
        audit.token_count = counter.count(&context);
        Ok(MoStateContext { context, audit })
    }

    fn load_user_state_contract(&self) -> Result<Option<StateContract>, MemoryError> {
        let path = self.root().join("config/state_contract.yaml");
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > CONTRACT_MAX_SIZE => Err(MemoryError::InvalidAccess(
                "state_contract.yaml exceeds 64 KiB".to_owned(),
            )),
            Ok(_) => {
                let text = fs::read_to_string(path)?;
                yaml_serde::from_str(&text)
                    .map(Some)
                    .map_err(|error| MemoryError::InvalidAccess(error.to_string()))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn loaded_dmw_signals(
        &self,
        retrieved: &[RetrievedMemory],
        audit: &mut MoStateAudit,
    ) -> Result<Vec<DmwSignal>, MemoryError> {
        let mut signals = Vec::new();
        for memory in retrieved {
            let document = match self.read_unchecked_for_state(&memory.path) {
                Ok(document) => document,
                Err(error) => {
                    audit.degraded = true;
                    audit.warnings.push(error.to_string());
                    continue;
                }
            };
            signals.push(DmwSignal {
                id: document.metadata.id,
                kind: document.metadata.kind,
                weight: document.metadata.weight.unwrap_or_default(),
                tags: document
                    .metadata
                    .tags
                    .into_iter()
                    .map(|value| normalize(&value))
                    .collect(),
                relations: document.metadata.relations,
                body: document.body,
            });
        }
        Ok(signals)
    }

    fn read_unchecked_for_state(&self, relative: &PathBuf) -> Result<MemoryDocument, MemoryError> {
        self.read(relative)
    }
}

fn merge_contract(base: &mut StateContract, user: StateContract) {
    base.version = user.version;
    for (dimension, contract) in user.dimensions {
        base.dimensions.insert(dimension, contract);
    }
    if valid_priority(&user.conflict_priority) {
        base.conflict_priority = user.conflict_priority;
    }
}

fn valid_priority(priority: &[String]) -> bool {
    let expected = default_priority().into_iter().collect::<BTreeSet<_>>();
    priority.iter().cloned().collect::<BTreeSet<_>>() == expected
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
        .all(|value| tags.contains(&value));
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
                signal.kind == "event"
                    && signal.tags.contains(&event_tag)
                    && !signal.tags.contains(&witness)
                    && !normalize(&signal.body).contains(&witness)
            })
        }
        "misconception" => signals.iter().any(|signal| {
            signal.kind == "event"
                && signal.tags.contains("misconception")
                && condition.character_ref.as_ref().is_none_or(|character| {
                    signal
                        .relations
                        .values()
                        .flatten()
                        .any(|value| value == character)
                })
        }),
        _ => false,
    }
}

fn order_directives(
    contract: &StateContract,
    directives: BTreeMap<String, Vec<(String, String)>>,
) -> Vec<(String, Vec<String>)> {
    contract
        .conflict_priority
        .iter()
        .filter_map(|dimension| {
            directives.get(dimension).map(|values| {
                (
                    dimension.clone(),
                    values
                        .iter()
                        .map(|(_, directive)| directive.clone())
                        .collect::<Vec<_>>(),
                )
            })
        })
        .collect()
}

fn trim_to_budget(
    ordered: &mut Vec<(String, Vec<String>)>,
    budget: usize,
    counter: &impl TokenCounter,
) {
    for removable in [
        "emotional_tone",
        "relational_stance",
        "epistemic_state",
        "physiological_state",
    ] {
        if counter.count(&format_state_context(ordered)) <= budget {
            return;
        }
        ordered.retain(|(dimension, _)| dimension != removable);
    }
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
    use crate::ConservativeTokenCounter;

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
}
