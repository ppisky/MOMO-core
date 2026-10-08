//! Runtime rollout switch for MO State DDM generation and projection.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DdmRuntimeConfig {
    /// Effective instance/Space gate. MO State generates a validated cache when
    /// no optional author-owned character override exists.
    #[serde(default)]
    pub enabled: bool,
}
