//! Named runtime prompt resources with compiled defaults and HTTP-friendly replacement semantics.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::product_prompts;

pub const PROMPT_SPACES_SCHEMA: &str = "momo.prompt-spaces/1.0";
pub const MAX_PROMPT_SPACE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PromptSpaceId {
    Assistant,
    VisionFallback,
    RoleplayDirector,
    MemoryDistillation,
    SemanticGraphGovernance,
}

impl PromptSpaceId {
    pub const ALL: [Self; 5] = [
        Self::Assistant,
        Self::VisionFallback,
        Self::RoleplayDirector,
        Self::MemoryDistillation,
        Self::SemanticGraphGovernance,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::VisionFallback => "vision_fallback",
            Self::RoleplayDirector => "roleplay_director",
            Self::MemoryDistillation => "memory_distillation",
            Self::SemanticGraphGovernance => "semantic_graph_governance",
        }
    }

    #[must_use]
    pub const fn default_content(self) -> &'static str {
        match self {
            Self::Assistant => product_prompts::ASSISTANT,
            Self::VisionFallback => product_prompts::VISION_FALLBACK,
            Self::RoleplayDirector => product_prompts::ROLEPLAY_DIRECTOR,
            Self::MemoryDistillation => product_prompts::MEMORY_DISTILLATION,
            Self::SemanticGraphGovernance => product_prompts::SEMANTIC_GRAPH_GOVERNANCE,
        }
    }
}

impl std::str::FromStr for PromptSpaceId {
    type Err = PromptSpacesError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|id| id.as_str() == value)
            .ok_or_else(|| PromptSpacesError::Unknown(value.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptSpaceSource {
    Builtin,
    Override,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PromptSpace {
    pub id: PromptSpaceId,
    pub content: String,
    pub source: PromptSpaceSource,
    pub revision: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPromptSpaces {
    schema: String,
    #[serde(default)]
    overrides: BTreeMap<PromptSpaceId, String>,
}

#[derive(Debug, Serialize)]
struct StoredPromptSpacesRef<'a> {
    schema: &'static str,
    overrides: &'a BTreeMap<PromptSpaceId, String>,
}

#[derive(Debug)]
struct PromptSpacesInner {
    path: Option<PathBuf>,
    overrides: Mutex<BTreeMap<PromptSpaceId, String>>,
}

/// Process-wide prompt registry. Clones share one atomically updated set of overrides.
#[derive(Debug, Clone)]
pub struct PromptSpaces {
    inner: Arc<PromptSpacesInner>,
}

impl Default for PromptSpaces {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl PromptSpaces {
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(PromptSpacesInner {
                path: None,
                overrides: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self, PromptSpacesError> {
        let path = path.as_ref().to_path_buf();
        let overrides = if path.exists() {
            let stored: StoredPromptSpaces = serde_json::from_slice(&fs::read(&path)?)?;
            if stored.schema != PROMPT_SPACES_SCHEMA {
                return Err(PromptSpacesError::UnsupportedSchema(stored.schema));
            }
            for (id, content) in &stored.overrides {
                validate_content(*id, content)?;
            }
            stored.overrides
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            inner: Arc::new(PromptSpacesInner {
                path: Some(path),
                overrides: Mutex::new(overrides),
            }),
        })
    }

    #[must_use]
    pub fn get(&self, id: PromptSpaceId) -> PromptSpace {
        let overrides = self
            .inner
            .overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        prompt_space(id, overrides.get(&id))
    }

    #[must_use]
    pub fn list(&self) -> Vec<PromptSpace> {
        let overrides = self
            .inner
            .overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        PromptSpaceId::ALL
            .into_iter()
            .map(|id| prompt_space(id, overrides.get(&id)))
            .collect()
    }

    pub fn replace(
        &self,
        id: PromptSpaceId,
        content: String,
    ) -> Result<PromptSpace, PromptSpacesError> {
        validate_content(id, &content)?;
        let mut overrides = self
            .inner
            .overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = overrides.clone();
        if content == id.default_content() {
            next.remove(&id);
        } else {
            next.insert(id, content);
        }
        self.persist(&next)?;
        *overrides = next;
        Ok(prompt_space(id, overrides.get(&id)))
    }

    pub fn reset(&self, id: PromptSpaceId) -> Result<PromptSpace, PromptSpacesError> {
        let mut overrides = self
            .inner
            .overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = overrides.clone();
        next.remove(&id);
        self.persist(&next)?;
        *overrides = next;
        Ok(prompt_space(id, None))
    }

    fn persist(
        &self,
        overrides: &BTreeMap<PromptSpaceId, String>,
    ) -> Result<(), PromptSpacesError> {
        let Some(path) = &self.inner.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or_else(|| {
            PromptSpacesError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Prompt Spaces path has no parent directory",
            ))
        })?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(
            &mut temporary,
            &StoredPromptSpacesRef {
                schema: PROMPT_SPACES_SCHEMA,
                overrides,
            },
        )?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn prompt_space(id: PromptSpaceId, content: Option<&String>) -> PromptSpace {
    let (content, source) = content.map_or_else(
        || (id.default_content().to_owned(), PromptSpaceSource::Builtin),
        |content| (content.clone(), PromptSpaceSource::Override),
    );
    let revision = hex::encode(Sha256::digest(content.as_bytes()));
    PromptSpace {
        id,
        content,
        source,
        revision,
    }
}

fn validate_content(id: PromptSpaceId, content: &str) -> Result<(), PromptSpacesError> {
    if content.trim().is_empty() || content.len() > MAX_PROMPT_SPACE_BYTES {
        return Err(PromptSpacesError::InvalidContent {
            id,
            max_bytes: MAX_PROMPT_SPACE_BYTES,
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum PromptSpacesError {
    #[error("unknown Prompt Space {0:?}")]
    Unknown(String),
    #[error("Prompt Space {id:?} must contain 1 to {max_bytes} bytes")]
    InvalidContent { id: PromptSpaceId, max_bytes: usize },
    #[error("unsupported Prompt Spaces schema {0:?}")]
    UnsupportedSchema(String),
    #[error("Prompt Spaces I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Prompt Spaces JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
#[path = "../tests/unit/prompt_spaces.rs"]
mod tests;
