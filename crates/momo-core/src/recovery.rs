//! Recovery of prepared maintenance operations across SQLite and memory files.

use crate::{CoreError, MomoCore};

#[cfg(test)]
#[path = "../tests/unit/recovery.rs"]
mod tests;

impl MomoCore {
    /// Runs under the data-directory instance lock before the Core is published.
    pub(crate) async fn recover_maintenance_commits(&self) -> Result<(), CoreError> {
        let mut spaces = std::collections::BTreeSet::new();
        for pending in self.store().pending_maintenance_batches().await? {
            if pending.prepared_commit_json.is_some() {
                spaces.insert(
                    uuid::Uuid::parse_str(&pending.batch.scope_id).map_err(|error| {
                        momo_memory::MemoryError::InvalidPatch(error.to_string())
                    })?,
                );
            }
        }
        for pending in self.store().pending_memory_patch_commits().await? {
            spaces.insert(pending.review.scope_id);
        }
        for space in spaces {
            // A damaged Space must not prevent unrelated Spaces or diagnostics starting.
            if let Err(error) = self.recover_space_commits(space).await {
                tracing::warn!(%space, %error, "Space isolated pending memory recovery");
            }
        }
        Ok(())
    }

    pub fn memory_recovery_status(&self) -> std::collections::BTreeMap<uuid::Uuid, String> {
        self.recovery_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn mark_memory_commit_pending(&self, space: uuid::Uuid) {
        self.recovery_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                space,
                "prepared memory commit awaiting acknowledgement".to_owned(),
            );
    }

    fn clear_memory_commit_pending(&self, space: uuid::Uuid) {
        self.recovery_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&space);
    }

    /// Typed, ordered ownership shared by imports, retrieval and all memory writers.
    /// Pending commits are completed before admitting any subsequent operation.
    pub(crate) async fn lock_spaces(
        &self,
        spaces: impl IntoIterator<Item = uuid::Uuid>,
    ) -> Result<Vec<tokio::sync::OwnedMutexGuard<()>>, CoreError> {
        let spaces: std::collections::BTreeSet<_> = spaces.into_iter().collect();
        let mut guards = Vec::with_capacity(spaces.len());
        for space in &spaces {
            let lock = {
                let mut locks = self.space_locks.lock().await;
                if locks.len() >= 1024 {
                    locks.retain(|_, lock| lock.strong_count() > 0);
                }
                if let Some(lock) = locks.get(space).and_then(std::sync::Weak::upgrade) {
                    lock
                } else {
                    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(*space, std::sync::Arc::downgrade(&lock));
                    lock
                }
            };
            guards.push(lock.lock_owned().await);
        }
        let core = self.clone();
        self.finish_commit(async move {
            for space in spaces {
                if core.memory_recovery_status().contains_key(&space) {
                    core.recover_space_commits(space).await?;
                }
            }
            Ok(guards)
        })
        .await
        .map_err(|error| momo_memory::MemoryError::InvalidPatch(error.to_string()))?
    }

    pub(crate) async fn finish_commit<T: Send + 'static>(
        &self,
        commit: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<T, tokio::sync::oneshot::error::RecvError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = sender.send(commit.await);
        });
        {
            let mut tasks = self
                .commit_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks.retain(|task| !task.is_finished());
            tasks.push(task);
        }
        receiver.await
    }

    pub(crate) async fn wait_for_commits(&self) {
        loop {
            let tasks = std::mem::take(
                &mut *self
                    .commit_tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            if tasks.is_empty() {
                return;
            }
            for task in tasks {
                if let Err(error) = task.await {
                    tracing::warn!(%error, "owned commit failed");
                }
            }
        }
    }

    async fn recover_space_commits(&self, space: uuid::Uuid) -> Result<(), CoreError> {
        let result = async {
            for pending in self
                .store()
                .pending_space_maintenance_commits(space)
                .await?
            {
                self.commit_prepared_maintenance(&pending).await?;
            }
            for pending in self
                .store()
                .pending_space_memory_patch_commits(space)
                .await?
            {
                self.commit_prepared_review(&pending).await?;
            }
            Ok::<_, CoreError>(())
        }
        .await;
        let mut failures = self
            .recovery_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(()) => {
                failures.remove(&space);
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                failures.insert(space, message.clone());
                Err(CoreError::Recovery {
                    space_id: space,
                    message,
                })
            }
        }
    }

    pub(crate) async fn commit_prepared_review(
        &self,
        pending: &momo_storage::PendingMemoryPatchReview,
    ) -> Result<momo_storage::MemoryPatchReview, CoreError> {
        self.apply_journaled_memory_commit(pending.review.scope_id, &pending.prepared_commit_json)
            .await?;
        let review = self
            .store()
            .resolve_memory_patch_review(
                pending.review.scope_id,
                pending.review.id,
                momo_storage::MemoryPatchReviewStatus::Approved,
                Some("ok"),
                None,
            )
            .await?
            .ok_or_else(|| {
                momo_memory::MemoryError::InvalidPatch(
                    "prepared review was already resolved".to_owned(),
                )
            })?;
        self.clear_memory_commit_pending(pending.review.scope_id);
        Ok(review)
    }

    async fn apply_journaled_memory_commit(
        &self,
        space_id: uuid::Uuid,
        encoded: &str,
    ) -> Result<(), CoreError> {
        let invalid = |message: String| momo_memory::MemoryError::InvalidPatch(message);
        let plan: momo_memory::PreparedMemoryCommit =
            serde_json::from_str(encoded).map_err(|error| invalid(error.to_string()))?;
        let core = self.clone();
        tokio::task::spawn_blocking(move || {
            core.memory_for_space_unchecked(space_id)?
                .apply_prepared_commit(&plan)
        })
        .await
        .map_err(|error| invalid(format!("memory commit task failed: {error}")))??;
        Ok(())
    }

    /// Requires the caller to hold the Space write lock, or exclusive startup ownership.
    pub(crate) async fn commit_prepared_maintenance(
        &self,
        pending: &momo_storage::PendingMaintenanceBatch,
    ) -> Result<(), CoreError> {
        let invalid = |message: String| momo_memory::MemoryError::InvalidPatch(message);
        let kind = match pending.batch.kind.as_str() {
            "memory" => momo_storage::MaintenanceKind::Memory,
            "semantic_graph" => momo_storage::MaintenanceKind::SemanticGraph,
            _ => return Err(invalid("unknown journal maintenance kind".to_owned()).into()),
        };
        let space_id = uuid::Uuid::parse_str(&pending.batch.scope_id)
            .map_err(|error| invalid(error.to_string()))?;
        self.apply_journaled_memory_commit(
            space_id,
            pending
                .prepared_commit_json
                .as_deref()
                .ok_or_else(|| invalid("maintenance file plan is missing".to_owned()))?,
        )
        .await?;
        self.store()
            .complete_maintenance_batch(&pending.batch.batch_key, &pending.batch.request_ids, kind)
            .await?;
        self.clear_memory_commit_pending(space_id);
        Ok(())
    }
}
