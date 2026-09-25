//! Prepared after-images for application-owned durable operation journals.
//!
//! SQLite owns journal identity and acknowledgement. Memory owns path safety,
//! conflict detection, and replay of the exact validated file contents.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMemoryCommit {
    version: u32,
    files: Vec<PreparedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedFile {
    path: String,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}

impl PreparedMemoryCommit {
    pub(crate) fn prepare(root: &Path, mutations: &[FileMutation]) -> Result<Self, MemoryError> {
        let mut files = Vec::with_capacity(mutations.len());
        let mut seen = HashSet::new();
        for mutation in mutations {
            let relative = mutation
                .path()
                .strip_prefix(root)
                .map_err(|_| MemoryError::UnsafePath(mutation.path().to_owned()))?;
            validate_relative(relative)?;
            if !seen.insert(relative.to_owned()) {
                return Err(MemoryError::InvalidPatch(
                    "duplicate prepared file target".to_owned(),
                ));
            }
            files.push(PreparedFile {
                path: portable_path(relative),
                before: snapshot_file(mutation.path())?,
                after: match mutation {
                    FileMutation::Write { content, .. } => Some(content.clone()),
                    FileMutation::Delete { .. } => None,
                },
            });
        }
        Ok(Self { version: 1, files })
    }
}

impl MemoryWorkspace {
    /// Validates a patch and freezes its file contents without applying it.
    /// The caller must serialize writers while preparing, journaling and applying.
    pub fn prepare_patch_commit(&self, yaml: &str) -> Result<PreparedMemoryCommit, MemoryError> {
        PreparedMemoryCommit::prepare(&self.root, &self.prepare_patch(yaml)?)
    }

    /// Replays a durable plan. Already written after-images are left untouched.
    /// A file with neither the original nor the intended contents is a conflict;
    /// validate every target before performing the first write.
    pub fn apply_prepared_commit(&self, plan: &PreparedMemoryCommit) -> Result<(), MemoryError> {
        if plan.version != 1 {
            return Err(MemoryError::InvalidPatch(
                "unsupported prepared commit version".to_owned(),
            ));
        }
        let mut seen = HashSet::new();
        let mut mutations = Vec::new();
        for file in &plan.files {
            let relative = Path::new(&file.path);
            validate_relative(relative)?;
            if !seen.insert(relative.to_owned()) {
                return Err(MemoryError::InvalidPatch(
                    "duplicate prepared file target".to_owned(),
                ));
            }
            // resolve checks every component, including symlinks and parent paths.
            let target = self.resolve(relative)?;
            let current = snapshot_file(&target)?;
            if current == file.after {
                continue;
            }
            if current != file.before {
                return Err(MemoryError::InvalidPatch(format!(
                    "prepared commit conflicts with current file: {}",
                    file.path
                )));
            }
            mutations.push(match &file.after {
                Some(content) => FileMutation::Write {
                    path: target,
                    content: content.clone(),
                },
                None => FileMutation::Delete { path: target },
            });
        }
        commit_mutations(&mutations)?;
        *self
            .index_cache
            .write()
            .map_err(|_| MemoryError::InvalidIndex("index lock poisoned".to_owned()))? = None;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/commit.rs"]
mod tests;
