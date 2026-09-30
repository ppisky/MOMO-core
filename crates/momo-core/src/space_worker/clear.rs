//! The clear intent and its file after-image share one journal transaction.
//! SQL cleanup and vector removal can then be retried before admitting commands.
use super::*;

impl SpaceHandle {
    pub(crate) fn clear_memory(
        &self,
        memory: bool,
        semantic_graph: bool,
    ) -> Result<usize, MemoryError> {
        self.submit(move |worker| {
            worker.execute_with_clear(
                move |workspace| {
                    let mut snapshot = workspace.export_snapshot()?;
                    let before = snapshot.files.len();
                    snapshot.files.retain(|path, _| {
                        let nsg = matches!(
                            path.split('/').collect::<Vec<_>>().as_slice(),
                            ["lore", ..]
                                | ["rules", ..]
                                | ["archive", "lore", ..]
                                | ["archive", "rules", ..]
                        );
                        !((nsg && semantic_graph) || (!nsg && memory))
                    });
                    let removed = before - snapshot.files.len();
                    workspace.import_snapshot(&snapshot)?;
                    MemoryWorkspace::initialize_layout(workspace.root())?;
                    Ok(removed)
                },
                Some((memory, semantic_graph)),
            )
        })
    }

    pub(crate) fn pending_clear(&self) -> Result<Option<(bool, bool)>, MemoryError> {
        self.submit(SpaceWorker::pending_clear)
    }

    pub(crate) fn acknowledge_clear(&self) -> Result<(), MemoryError> {
        self.submit(|worker| {
            worker
                .runtime
                .block_on(
                    sqlx::query("DELETE FROM pending_clear WHERE id = 1")
                        .execute(&mut worker.journal),
                )
                .map_err(unavailable)?;
            Ok(())
        })
    }
}

impl SpaceWorker {
    pub(super) fn pending_clear(&mut self) -> Result<Option<(bool, bool)>, MemoryError> {
        self.runtime
            .block_on(
                sqlx::query_as("SELECT memory, semantic_graph FROM pending_clear WHERE id = 1")
                    .fetch_optional(&mut self.journal),
            )
            .map_err(unavailable)
    }
}
