//! A Space address never exposes its workspace. Only its worker owns memory.
//!
//! Commands run on a disposable working copy. A FULL-synchronous SQLite
//! after-image journal commits before the materialized files change. A panic
//! discards the entire incarnation; the supervisor reopens durable state.

use momo_memory::{MemoryError, MemorySnapshot, MemoryWorkspace};
use sqlx::{
    Connection, Row,
    sqlite::{SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqliteSynchronous},
};
use std::{
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};
use uuid::Uuid;

const MAX_WORKERS: usize = 128;
type Job = Box<dyn FnOnce(&mut SpaceWorker) + Send>;
type Lookup = (
    Uuid,
    mpsc::SyncSender<Result<Arc<SpaceHandle>, MemoryError>>,
);

fn unavailable(message: impl std::fmt::Display) -> MemoryError {
    MemoryError::InvalidPatch(format!("Space worker unavailable: {message}"))
}

#[derive(Debug, Clone)]
pub(crate) struct SpaceSupervisor {
    owner: Arc<SupervisorOwner>,
    failures: Arc<crate::OwnedState<std::collections::BTreeMap<Uuid, String>>>,
}

#[derive(Debug)]
struct SupervisorOwner {
    sender: Option<mpsc::SyncSender<Lookup>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for SupervisorOwner {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl SpaceSupervisor {
    pub(crate) fn start(
        root: PathBuf,
        instance_lock: Arc<std::fs::File>,
    ) -> Result<Self, MemoryError> {
        Self::start_with_limit(root, instance_lock, MAX_WORKERS)
    }

    fn start_with_limit(
        root: PathBuf,
        instance_lock: Arc<std::fs::File>,
        limit: usize,
    ) -> Result<Self, MemoryError> {
        let failures: Arc<crate::OwnedState<std::collections::BTreeMap<Uuid, String>>> =
            Arc::default();
        let worker_failures = Arc::clone(&failures);
        let (sender, receiver) = mpsc::sync_channel::<Lookup>(64);
        let thread = std::thread::Builder::new()
            .name("space-supervisor".into())
            .spawn(move || {
                let mut workers: HashMap<Uuid, Arc<SpaceHandle>> = HashMap::new();
                while let Ok((id, reply)) = receiver.recv() {
                    if !workers.contains_key(&id) && workers.len() >= limit {
                        // Only evict addresses that no caller or admitted command owns.
                        if let Some(idle) = workers
                            .iter()
                            .find(|(_, h)| Arc::strong_count(h) == 1)
                            .map(|(id, _)| *id)
                        {
                            workers.remove(&idle);
                        }
                    }
                    let result = if let Some(handle) = workers.get(&id) {
                        Ok(Arc::clone(handle))
                    } else if workers.len() >= limit {
                        Err(MemoryError::WorkspaceCapacity { limit })
                    } else {
                        SpaceHandle::start(
                            root.join(id.to_string()).join("memory"),
                            Arc::clone(&instance_lock),
                            id,
                            Arc::clone(&worker_failures),
                        )
                        .map(|handle| {
                            let handle = Arc::new(handle);
                            workers.insert(id, Arc::clone(&handle));
                            handle
                        })
                    };
                    let _ = reply.send(result);
                }
            })?;
        Ok(Self {
            owner: Arc::new(SupervisorOwner {
                sender: Some(sender),
                thread: Some(thread),
            }),
            failures,
        })
    }

    pub(crate) fn status(&self) -> std::collections::BTreeMap<Uuid, String> {
        self.failures.call(|failures| failures.clone())
    }

    pub(crate) fn space(&self, id: Uuid) -> Result<Arc<SpaceHandle>, MemoryError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.owner
            .sender
            .as_ref()
            .expect("live supervisor")
            .send((id, reply))
            .map_err(unavailable)?;
        result.recv().map_err(unavailable)?
    }
}

#[derive(Debug)]
pub(crate) struct SpaceHandle {
    sender: Option<mpsc::SyncSender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(test)]
    root: PathBuf,
}

impl SpaceHandle {
    fn start(
        root: PathBuf,
        instance_lock: Arc<std::fs::File>,
        id: Uuid,
        failures: Arc<crate::OwnedState<std::collections::BTreeMap<Uuid, String>>>,
    ) -> Result<Self, MemoryError> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(64);
        let (ready, result) = mpsc::sync_channel(1);
        let worker_root = root.clone();
        let thread = std::thread::Builder::new().name("space-worker".into()).spawn(move || {
            let _instance_lock = instance_lock;
            let mut worker = match SpaceWorker::open(&worker_root) {
                Ok(worker) => {
                    failures.call(move |failures| { failures.remove(&id); });
                    let _ = ready.send(Ok(())); Some(worker)
                }
                Err(error) => {
                    let message = error.to_string();
                    failures.call(move |failures| { failures.insert(id, message); });
                    let _ = ready.send(Err(error)); return;
                }
            };
            while let Ok(job) = receiver.recv() {
                if worker.is_none() {
                    // Failed recovery is kept isolated. A later request may retry
                    // after the durable conflict has been repaired.
                    match SpaceWorker::open(&worker_root) {
                        Ok(rebuilt) => {
                            worker = Some(rebuilt);
                            failures.call(move |failures| { failures.remove(&id); });
                        }
                        Err(error) => {
                            let message = error.to_string();
                            failures.call(move |failures| { failures.insert(id, message); });
                        }
                    }
                }
                let Some(state) = worker.as_mut() else { drop(job); continue; };
                if catch_unwind(AssertUnwindSafe(|| job(state))).is_err() || state.failed {
                    tracing::error!(path = %worker_root.display(), "discarding Space worker incarnation");
                    worker = None;
                    failures.call(move |failures| { failures.insert(id, "worker incarnation discarded; journal recovery required".into()); });
                }
            }
        })?;
        result.recv().map_err(unavailable)??;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
            #[cfg(test)]
            root,
        })
    }

    /// Every closure runs inside the worker. Borrowed workspace references
    /// cannot escape through the Send + 'static result boundary.
    pub(crate) fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&MemoryWorkspace) -> Result<T, MemoryError> + Send + 'static,
    ) -> Result<T, MemoryError> {
        self.submit(move |worker| worker.execute(operation))
    }

    fn submit<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut SpaceWorker) -> Result<T, MemoryError> + Send + 'static,
    ) -> Result<T, MemoryError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .expect("live address")
            .send(Box::new(move |worker| {
                let outcome = operation(worker);
                let _ = reply.send(outcome);
            }))
            .map_err(unavailable)?;
        result
            .recv()
            .map_err(|_| unavailable("command interrupted; durable outcome may require recovery"))?
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for SpaceHandle {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct SpaceWorker {
    workspace: MemoryWorkspace,
    cache: MemoryWorkspace,
    cached_snapshot: MemorySnapshot,
    journal: SqliteConnection,
    runtime: tokio::runtime::Runtime,
    failed: bool,
    checkpointed: bool,
    #[cfg(test)]
    fault: Option<u8>,
    #[cfg(test)]
    terminate_on_fault: bool,
}

impl SpaceWorker {
    fn open(root: &Path) -> Result<Self, MemoryError> {
        catch_unwind(AssertUnwindSafe(|| Self::restore(root)))
            .map_err(|_| unavailable("panic while rebuilding from journal"))?
    }

    fn restore(root: &Path) -> Result<Self, MemoryError> {
        // Never follow a Space, materialization or journal symlink. The UUID
        // path is chosen by the supervisor, not supplied by a command.
        let space = root.parent().expect("Space directory");
        for path in [
            space.to_path_buf(),
            root.to_path_buf(),
            space.join("memory-journal.sqlite3"),
        ] {
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(MemoryError::UnsafePath(path));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        std::fs::create_dir_all(root)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let options = SqliteConnectOptions::new()
            .filename(
                root.parent()
                    .expect("Space directory")
                    .join("memory-journal.sqlite3"),
            )
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full);
        let mut journal = runtime
            .block_on(SqliteConnection::connect_with(&options))
            .map_err(unavailable)?;
        runtime.block_on(sqlx::query("CREATE TABLE IF NOT EXISTS memory_journal (revision INTEGER PRIMARY KEY AUTOINCREMENT, snapshot TEXT NOT NULL, applied INTEGER NOT NULL DEFAULT 0)").execute(&mut journal)).map_err(unavailable)?;
        runtime.block_on(sqlx::query("CREATE TABLE IF NOT EXISTS pending_clear (id INTEGER PRIMARY KEY CHECK(id = 1), memory INTEGER NOT NULL, semantic_graph INTEGER NOT NULL)").execute(&mut journal)).map_err(unavailable)?;
        let checkpointed: bool = runtime
            .block_on(
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memory_journal)")
                    .fetch_one(&mut journal),
            )
            .map_err(unavailable)?;
        // Open without creating defaults: initialization must never overwrite a
        // partially applied after-image before recovery gets to inspect it.
        let mut workspace = MemoryWorkspace::open_existing(root)?;
        // The journal is authoritative on every start, including eviction and
        // clean process restarts. `applied` is only a materialization marker.
        let query = "SELECT revision, snapshot FROM memory_journal ORDER BY revision DESC LIMIT 1";
        let pending = runtime
            .block_on(sqlx::query(query).fetch_all(&mut journal))
            .map_err(unavailable)?;
        for row in pending {
            let snapshot: MemorySnapshot =
                serde_json::from_str(row.get("snapshot")).map_err(unavailable)?;
            workspace.import_snapshot(&snapshot)?;
            runtime
                .block_on(
                    sqlx::query("UPDATE memory_journal SET applied = 1 WHERE revision = ?")
                        .bind(row.get::<i64, _>("revision"))
                        .execute(&mut journal),
                )
                .map_err(unavailable)?;
        }
        // Bootstrap in private storage too, then journal the initialized image.
        let before = workspace.export_snapshot()?;
        // This reserved directory is disposable and stays inside the instance
        // data directory. Never leave private memory in system temporary files.
        let cache_path = space.join(".memory-worker-cache");
        match std::fs::symlink_metadata(&cache_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(MemoryError::UnsafePath(cache_path));
            }
            Ok(_) => std::fs::remove_dir_all(&cache_path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let staged = MemoryWorkspace::open_existing(&cache_path)?;
        staged.import_snapshot(&before)?;
        let staged = MemoryWorkspace::initialize_layout(&cache_path)?;
        let after = staged.export_snapshot()?;
        let mut worker = Self {
            workspace,
            cache: staged,
            cached_snapshot: after.clone(),
            journal,
            runtime,
            failed: false,
            checkpointed,
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            terminate_on_fault: false,
        };
        worker.publish(&before, &after, None)?;
        workspace = MemoryWorkspace::initialize_layout(root)?;
        worker.workspace = workspace;
        Ok(worker)
    }

    fn execute<T>(
        &mut self,
        operation: impl FnOnce(&MemoryWorkspace) -> Result<T, MemoryError>,
    ) -> Result<T, MemoryError> {
        self.execute_with_clear(operation, None)
    }

    fn execute_with_clear<T>(
        &mut self,
        operation: impl FnOnce(&MemoryWorkspace) -> Result<T, MemoryError>,
        clear: Option<(bool, bool)>,
    ) -> Result<T, MemoryError> {
        if self.pending_clear()?.is_some() {
            return Err(unavailable("memory clear awaiting database cleanup"));
        }
        let before = self.cached_snapshot.clone();
        let result = match operation(&self.cache) {
            Ok(result) => result,
            Err(error) => {
                // A rejected command may have mutated its private cache. Discard
                // that incarnation as well; no failed state is reused.
                if self.cache.import_snapshot(&before).is_err() {
                    self.failed = true;
                } else {
                    self.cache = MemoryWorkspace::open_existing(self.cache.root())?;
                    self.cached_snapshot = before;
                }
                return Err(error);
            }
        };
        let after = match self.cache.export_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        if let Err(error) = self.publish(&before, &after, clear) {
            self.failed = true;
            return Err(error);
        }
        self.cached_snapshot = after;
        Ok(result)
    }

    #[cfg(test)]
    fn crash_at(&mut self, point: u8) {
        if self.fault == Some(point) {
            if self.terminate_on_fault {
                std::process::exit(91);
            }
            panic!("injected Space worker fault {point}");
        }
    }

    fn publish(
        &mut self,
        before: &MemorySnapshot,
        after: &MemorySnapshot,
        clear: Option<(bool, bool)>,
    ) -> Result<(), MemoryError> {
        if before.files == after.files && self.checkpointed && clear.is_none() {
            return Ok(());
        }
        #[cfg(test)]
        self.crash_at(0);
        let encoded = serde_json::to_string(after).map_err(unavailable)?;
        // Keep the latest two complete after-images as checkpoints. Deletion
        // and admission are in one SQLite transaction, so no recovery gap exists.
        let revision = self
            .runtime
            .block_on(async {
                let mut tx = self.journal.begin().await?;
                let result = sqlx::query("INSERT INTO memory_journal(snapshot) VALUES (?)")
                    .bind(encoded)
                    .execute(&mut *tx)
                    .await?;
                let revision = result.last_insert_rowid();
                if let Some((memory, semantic_graph)) = clear {
                    sqlx::query(
                        "INSERT INTO pending_clear(id, memory, semantic_graph) VALUES (1, ?, ?)",
                    )
                    .bind(memory)
                    .bind(semantic_graph)
                    .execute(&mut *tx)
                    .await?;
                }
                sqlx::query("DELETE FROM memory_journal WHERE revision < ? AND applied = 1")
                    .bind(revision - 1)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok::<_, sqlx::Error>(revision)
            })
            .map_err(unavailable)?;
        self.checkpointed = true;
        #[cfg(test)]
        self.crash_at(1);
        // Commit only changed files; the complete journal image can rebuild all
        // files even if the process dies during this materialization.
        let plan = self.workspace.prepare_snapshot_commit(after)?;
        #[cfg(test)]
        if self.fault == Some(3) {
            let mut partial = serde_json::to_value(&plan).map_err(unavailable)?;
            partial["files"]
                .as_array_mut()
                .expect("plan files")
                .truncate(1);
            self.workspace
                .apply_prepared_commit(&serde_json::from_value(partial).map_err(unavailable)?)?;
            self.crash_at(3);
        }
        self.workspace.apply_prepared_commit(&plan)?;
        #[cfg(test)]
        self.crash_at(2);
        self.runtime
            .block_on(
                sqlx::query("UPDATE memory_journal SET applied = 1 WHERE revision = ?")
                    .bind(revision)
                    .execute(&mut self.journal),
            )
            .map_err(unavailable)?;
        Ok(())
    }
}

pub(crate) mod address;

pub(crate) mod reservations;

mod clear;

#[cfg(test)]
#[path = "../tests/unit/space_worker.rs"]
mod tests;
