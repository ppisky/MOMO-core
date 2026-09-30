//! Prepared after-images for application-owned durable operation journals.
//!
//! SQLite owns journal identity and acknowledgement. Memory owns path safety,
//! conflict detection, and replay of the exact validated file contents.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMemoryCommit {
    version: u32,
    pub(crate) files: Vec<PreparedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedFile {
    pub(crate) path: String,
    pub(crate) before: Option<Vec<u8>>,
    pub(crate) after: Option<Vec<u8>>,
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
    /// Open a materialization without bootstrapping or modifying file contents.
    /// The owner must replay its durable journal before serving commands.
    pub fn open_existing(root: impl AsRef<Path>) -> Result<Self, MemoryError> {
        std::fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: std::fs::canonicalize(root.as_ref())?,
            index_cache: std::cell::RefCell::new(None),
        })
    }

    /// Prepare a complete snapshot replacement, creating safe parent directories
    /// but leaving file contents unchanged. Owners journal the snapshot first.
    pub fn prepare_snapshot_commit(
        &self,
        snapshot: &MemorySnapshot,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        if snapshot.version != 1 {
            return Err(MemoryError::InvalidPatch(
                "unsupported snapshot version".into(),
            ));
        }
        provenance::validate_snapshot_provenance(snapshot)?;
        let current = self.export_snapshot()?;
        let mut mutations = Vec::new();
        for (path, content) in &snapshot.files {
            let relative = Path::new(path);
            validate_relative(relative)?;
            if let Some(parent) = relative
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                create_managed_directory(&self.root, parent)?;
            }
            let target = self.resolve(relative)?;
            if current.files.get(path) != Some(content) {
                mutations.push(FileMutation::Write {
                    path: target,
                    content: content.as_bytes().to_vec(),
                });
            }
        }
        for path in current
            .files
            .keys()
            .filter(|path| !snapshot.files.contains_key(*path))
        {
            mutations.push(FileMutation::Delete {
                path: self.resolve(Path::new(path))?,
            });
        }
        PreparedMemoryCommit::prepare(&self.root, &mutations)
    }

    pub fn prepare_identity_patch_commit(
        &self,
        yaml: &str,
        identity: Option<&momo_domain::provenance::MemoryIdentity>,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        PreparedMemoryCommit::prepare(
            &self.root,
            &self.prepare_patch_for_identity(yaml, identity)?,
        )
    }
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
        *self.index_cache.borrow_mut() = None;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/commit.rs"]
mod tests;
