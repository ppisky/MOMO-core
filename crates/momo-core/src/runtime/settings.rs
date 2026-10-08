//! Persisted runtime policy, serialized updates and sparse Space overrides.
use crate::{GovernanceError, MomoRuntimeSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSettingsResource {
    pub revision: u64,
    pub space_id: Option<Uuid>,
    pub source: String,
    pub settings: MomoRuntimeSettings,
    pub sources: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSettings {
    schema: String,
    revision: u64,
    defaults: MomoRuntimeSettings,
    spaces: BTreeMap<Uuid, Value>,
}

#[derive(Debug)]
pub(crate) struct SettingsStore {
    path: PathBuf,
    stored: StoredSettings,
}

pub(crate) fn merge_patch(target: &mut Value, patch: &Value) {
    if let Value::Object(fields) = patch {
        if !target.is_object() {
            *target = serde_json::json!({});
        }
        let current = target.as_object_mut().expect("object");
        for (key, value) in fields {
            if value.is_null() {
                current.remove(key);
            } else {
                merge_patch(current.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
    } else {
        *target = patch.clone();
    }
}

fn invalid(error: impl ToString) -> GovernanceError {
    GovernanceError::Invalid(error.to_string())
}

// Empty objects contain no override leaves. Arrays (including empty arrays),
// false and zero remain explicit values.
fn prune_empty_objects(value: &mut Value) -> bool {
    if let Value::Object(fields) = value {
        fields.retain(|_, value| !prune_empty_objects(value));
        fields.is_empty()
    } else {
        false
    }
}

fn normalize_space_overrides(stored: &mut StoredSettings) {
    stored.spaces.retain(|_, patch| !prune_empty_objects(patch));
}

fn field_sources(
    value: &Value,
    overrides: Option<&Value>,
    prefix: &str,
    sources: &mut BTreeMap<String, String>,
) {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let name = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            field_sources(value, overrides.and_then(|v| v.get(key)), &name, sources);
        }
    } else {
        sources.insert(
            prefix.into(),
            if overrides.is_some() {
                "space_override"
            } else {
                "instance"
            }
            .into(),
        );
    }
}

/// Machine-readable description of every runtime policy field and its bounds.
pub fn runtime_settings_description() -> Value {
    fn description(path: &str) -> &str {
        match path {
            "schema_version" => "Runtime policy schema version.",
            "history_window.enabled" => {
                "Evict complete old turns from the prompt copy; stored history is retained."
            }
            "history_window.trigger_turns" => "Prompt history turn count that triggers eviction.",
            "history_window.evict_turns" => {
                "Complete oldest turns removed when the history threshold is reached."
            }
            "request_overrides.context_window" => {
                "Allow, ignore or reject request context-window proposals."
            }
            "request_overrides.max_output_tokens" => {
                "Allow, ignore or reject request output-token proposals."
            }
            "request_overrides.sampling" => "Allow, ignore or reject request sampling proposals.",
            "request_overrides.instructions" => "Allow, ignore or reject request instructions.",
            "request_overrides.visual_description_prompt" => {
                "Allow, ignore or reject request image-description instructions."
            }
            "request_overrides.tools" => {
                "Allow, ignore or reject request tool configuration; hosts execute tools."
            }
            "request_overrides.allowed_parameters" => {
                "Provider parameters permitted in requests; protocol-owned names are reserved."
            }
            "runtime.memory_distillation_enabled" => {
                "Schedule and recover automatic DMW distillation; explicit drains remain available."
            }
            "runtime.memory_distill_every_turns" => {
                "Completed response pairs required for automatic DMW maintenance and maximum new-batch size."
            }
            "runtime.semantic_graph_enabled" => {
                "Schedule and recover automatic NSG governance; explicit drains remain available."
            }
            "runtime.nsg_govern_every_turns" => {
                "Completed response pairs required for automatic NSG maintenance and maximum new-batch size."
            }
            "mo_state.profile" => {
                "Select autonomous durable state coordination or the legacy projection profile."
            }
            "mo_state.scene_management" => {
                "Permit automatic maintenance of scene and active threads; explicit manual edits remain available."
            }
            "mo_state.max_reconcile_steps" => {
                "Maximum proposal/validation attempts for each DDM generation or new maintenance patch."
            }
            "mo_state.max_agent_steps" => {
                "Initial response plus committed tool continuations per ordinary user turn; replay consumes no step."
            }
            "mo_state.operation_timeout_ms" => {
                "Deadline for foreground state work and each maintenance batch; admitted commits may finish afterward."
            }
            "mo_state.injection_mode" => {
                "Inject state cues actively or record a shadow projection without injecting cues."
            }
            "mo_state.memory_lifecycle.enabled" => {
                "Create activity-driven lifecycle events from eligible completed responses."
            }
            "mo_state.memory_lifecycle.decay_after_turns" => {
                "Unhit activity turns before memory weight decay."
            }
            "mo_state.memory_lifecycle.forget_after_turns" => {
                "Unhit activity turns before an eligible archived memory can be forgotten."
            }
            "mo_state.memory_lifecycle.decay_factor" => {
                "Weight multiplier applied at each eligible decay event."
            }
            "mo_state.memory_lifecycle.auto_forget" => {
                "Permit physical forgetting after archived-record protection checks."
            }
            "mo_state.ddm.enabled" => {
                "Enable DDM projection; generate and cache a character-based profile when no author override exists."
            }
            "roleplay.enabled" => "Apply the roleplay director prompt to character performance.",
            "vision.enabled" => {
                "Accept governed image input, using direct multimodal support or a description adapter."
            }
            _ => path,
        }
    }
    fn describe(value: &Value, prefix: &str, fields: &mut serde_json::Map<String, Value>) {
        if let Some(object) = value.as_object() {
            for (name, value) in object {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                describe(value, &path, fields);
            }
        } else {
            let kind = if value.is_boolean() {
                "boolean"
            } else if value.is_u64() {
                "integer"
            } else if value.is_number() {
                "number"
            } else if value.is_array() {
                "array"
            } else {
                "string"
            };
            fields.insert(
                prefix.into(),
                serde_json::json!({"type":kind,"default":value,"description":description(prefix)}),
            );
        }
    }
    let mut fields = serde_json::Map::new();
    describe(
        &serde_json::to_value(MomoRuntimeSettings::default()).expect("typed defaults"),
        "",
        &mut fields,
    );
    for (name, minimum, maximum) in [
        ("history_window.trigger_turns", 2, 1000),
        ("history_window.evict_turns", 1, 999),
        ("runtime.memory_distill_every_turns", 1, 200),
        ("runtime.nsg_govern_every_turns", 1, 200),
        ("mo_state.max_reconcile_steps", 1, 16),
        ("mo_state.max_agent_steps", 1, 32),
        ("mo_state.operation_timeout_ms", 1000, 300000),
        ("mo_state.memory_lifecycle.decay_after_turns", 1, 1000000),
        ("mo_state.memory_lifecycle.forget_after_turns", 1, 1000000),
    ] {
        fields[name]["minimum"] = minimum.into();
        fields[name]["maximum"] = maximum.into();
    }
    fields["schema_version"]["const"] = 1.into();
    fields["history_window.evict_turns"]["constraint"] =
        "less than history_window.trigger_turns".into();
    fields["mo_state.memory_lifecycle.decay_factor"]["exclusiveMinimum"] = 0.into();
    fields["mo_state.memory_lifecycle.decay_factor"]["exclusiveMaximum"] = 1.into();
    for (name, descriptor) in &mut fields {
        if name.starts_with("request_overrides.") && name != "request_overrides.allowed_parameters"
        {
            descriptor["enum"] = serde_json::json!(["allow", "ignore", "reject"]);
        }
    }
    fields["mo_state.profile"]["enum"] = serde_json::json!(["closed_autonomous", "v1_projection"]);
    fields["mo_state.injection_mode"]["enum"] = serde_json::json!(["active", "shadow"]);
    fields["request_overrides.allowed_parameters"]["items"] = serde_json::json!({"type":"string"});
    fields["request_overrides.allowed_parameters"]["uniqueItems"] = true.into();
    fields["request_overrides.allowed_parameters"]["reserved"] = serde_json::json!([
        "model",
        "messages",
        "input",
        "stream",
        "context_window",
        "max_tokens",
        "max_output_tokens",
        "temperature"
    ]);
    serde_json::json!({"schema":"momo.runtime-settings-description/1","fields":fields,
        "persistence":"runtime-settings.json","space_overrides":"sparse; PATCH null removes an override",
        "concurrency":"ETag / If-Match, instance-wide monotonic revision"})
}
fn decode(value: Value) -> Result<MomoRuntimeSettings, GovernanceError> {
    let settings: MomoRuntimeSettings = serde_json::from_value(value).map_err(invalid)?;
    settings.validate()?;
    Ok(settings)
}

impl SettingsStore {
    pub(crate) fn load(path: &Path) -> Result<Self, GovernanceError> {
        let mut stored = match std::fs::read(path) {
            Ok(bytes) => {
                if bytes.len() > 4 * 1024 * 1024 {
                    return Err(invalid("runtime settings file exceeds 4 MiB"));
                }
                serde_json::from_slice::<StoredSettings>(&bytes).map_err(invalid)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredSettings {
                schema: "momo.runtime-settings/1".into(),
                revision: 0,
                defaults: MomoRuntimeSettings::default(),
                spaces: BTreeMap::new(),
            },
            Err(error) => return Err(GovernanceError::Persistence(error.to_string())),
        };
        if stored.schema != "momo.runtime-settings/1" {
            return Err(invalid("unsupported or oversized stored runtime settings"));
        }
        Self::validate(&stored)?;
        normalize_space_overrides(&mut stored);
        if stored.spaces.len() > 1024 {
            return Err(invalid("unsupported or oversized stored runtime settings"));
        }
        Ok(Self {
            path: path.to_owned(),
            stored,
        })
    }

    pub(crate) fn space_override(&self, id: Uuid) -> Option<Value> {
        self.stored.spaces.get(&id).cloned()
    }

    pub(crate) fn removal_image(
        mut self,
        from: Uuid,
        to: Option<Uuid>,
    ) -> Result<Option<String>, GovernanceError> {
        let Some(patch) = self.stored.spaces.remove(&from) else {
            return Ok(None);
        };
        if let Some(to) = to {
            if let Some(existing) = self.stored.spaces.get_mut(&to) {
                crate::space_lifecycle::merge_maps(existing, patch, "runtime-settings")
                    .map_err(invalid)?;
            } else {
                self.stored.spaces.insert(to, patch);
            }
        }
        self.stored.revision = self
            .stored
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("settings revision exhausted"))?;
        Self::validate(&self.stored)?;
        let image = serde_json::to_string(&self.stored).map_err(invalid)?;
        if image.len() > 4 * 1024 * 1024 {
            return Err(invalid("runtime settings file exceeds 4 MiB"));
        }
        Ok(Some(image))
    }

    pub(crate) fn apply_removal_image(path: &Path, image: &str) -> Result<(), GovernanceError> {
        let stored: StoredSettings = serde_json::from_str(image).map_err(invalid)?;
        Self::validate(&stored)?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("missing settings parent"))?;
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(invalid)?;
        staged.write_all(image.as_bytes()).map_err(invalid)?;
        staged.as_file().sync_all().map_err(invalid)?;
        staged.persist(path).map_err(invalid)?;
        Ok(())
    }

    fn effective(stored: &StoredSettings, space: Option<Uuid>) -> MomoRuntimeSettings {
        let mut value = serde_json::to_value(&stored.defaults).expect("typed settings");
        if let Some(patch) = space.and_then(|id| stored.spaces.get(&id)) {
            merge_patch(&mut value, patch);
        }
        serde_json::from_value(value).expect("validated settings")
    }

    fn validate(stored: &StoredSettings) -> Result<(), GovernanceError> {
        stored.defaults.validate()?;
        for patch in stored.spaces.values() {
            if !patch.is_object() {
                return Err(invalid("Space overrides must be JSON objects"));
            }
            let mut value = serde_json::to_value(&stored.defaults).map_err(invalid)?;
            merge_patch(&mut value, patch);
            decode(value)?;
        }
        Ok(())
    }

    pub(super) fn resource(&self, space: Option<Uuid>) -> RuntimeSettingsResource {
        let settings = Self::effective(&self.stored, space);
        let value = serde_json::to_value(&settings).expect("typed settings");
        let mut sources = BTreeMap::new();
        field_sources(
            &value,
            space.and_then(|id| self.stored.spaces.get(&id)),
            "",
            &mut sources,
        );
        RuntimeSettingsResource {
            revision: self.stored.revision,
            space_id: space,
            source: if space.is_some_and(|id| self.stored.spaces.contains_key(&id)) {
                "space_override"
            } else {
                "instance"
            }
            .into(),
            settings,
            sources,
        }
    }

    pub(super) fn update(
        &mut self,
        space: Option<Uuid>,
        expected: Option<u64>,
        value: Option<Value>,
        patch: bool,
    ) -> Result<RuntimeSettingsResource, GovernanceError> {
        if expected.is_some_and(|revision| revision != self.stored.revision) {
            return Err(GovernanceError::Conflict {
                expected: expected.expect("checked"),
                actual: self.stored.revision,
            });
        }
        let mut next = self.stored.clone();
        match (space, value) {
            (Some(id), Some(value)) => {
                if !value.is_object() {
                    return Err(invalid("settings must be a JSON object"));
                }
                if patch {
                    merge_patch(
                        next.spaces
                            .entry(id)
                            .or_insert_with(|| serde_json::json!({})),
                        &value,
                    );
                } else {
                    next.spaces
                        .insert(id, serde_json::to_value(decode(value)?).map_err(invalid)?);
                }
            }
            (Some(id), None) => {
                next.spaces.remove(&id);
            }
            (None, Some(value)) => {
                if !value.is_object() {
                    return Err(invalid("settings must be a JSON object"));
                }
                let mut effective = serde_json::to_value(&next.defaults).map_err(invalid)?;
                if patch {
                    merge_patch(&mut effective, &value);
                } else {
                    effective = value;
                }
                next.defaults = decode(effective)?;
            }
            (None, None) => next.defaults = MomoRuntimeSettings::default(),
        }
        Self::validate(&next)?;
        normalize_space_overrides(&mut next);
        if next.spaces.len() > 1024 {
            return Err(invalid("at most 1024 Space overrides are supported"));
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("settings revision exhausted"))?;
        let bytes = serde_json::to_vec_pretty(&next).map_err(invalid)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(invalid("runtime settings file exceeds 4 MiB"));
        }
        let mut file =
            tempfile::NamedTempFile::new_in(self.path.parent().expect("settings directory"))
                .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
        file.write_all(&bytes)
            .and_then(|()| file.as_file().sync_all())
            .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
        file.persist(&self.path)
            .map_err(|e| GovernanceError::Persistence(e.to_string()))?;
        self.stored = next;
        Ok(self.resource(space))
    }
}
