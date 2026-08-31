//! Structured control-plane operations that never pass through a model.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MOMO_CONTROL_SCHEMA: &str = "momo.control/1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MomoControlRequest {
    pub schema: String,
    pub request_id: String,
    pub actor_space_id: Uuid,
    pub action: MomoControlAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MomoControlAction {
    DeleteConversation {
        conversation_space_id: Uuid,
        conversation_id: Uuid,
    },
    ClearMemory {
        target_space_id: Uuid,
        memory: bool,
        semantic_graph: bool,
    },
    SwitchCharacter {
        conversation_space_id: Uuid,
        conversation_id: Uuid,
        character_id: Uuid,
    },
}

impl MomoControlRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != MOMO_CONTROL_SCHEMA {
            return Err(format!("unsupported MOMO control schema {:?}", self.schema));
        }
        if self.request_id.trim().is_empty() || self.request_id.len() > 256 {
            return Err("control request_id must contain 1 to 256 bytes".to_owned());
        }
        if let MomoControlAction::ClearMemory {
            memory,
            semantic_graph,
            ..
        } = self.action
            && !memory
            && !semantic_graph
        {
            return Err("clear_memory must select memory and/or semantic_graph".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_memory_requires_an_explicit_module() {
        let request = MomoControlRequest {
            schema: MOMO_CONTROL_SCHEMA.to_owned(),
            request_id: "control-1".to_owned(),
            actor_space_id: Uuid::now_v7(),
            action: MomoControlAction::ClearMemory {
                target_space_id: Uuid::now_v7(),
                memory: false,
                semantic_graph: false,
            },
        };
        assert!(request.validate().is_err());
    }
}
