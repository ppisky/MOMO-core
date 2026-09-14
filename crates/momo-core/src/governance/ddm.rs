//! Runtime rollout switch for character-owned DDM profiles.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DdmRuntimeConfig {
    /// Global rollout gate. Profiles themselves are author-owned character
    /// extensions and are never loaded from runtime filesystem paths.
    #[serde(default)]
    pub enabled: bool,
}
