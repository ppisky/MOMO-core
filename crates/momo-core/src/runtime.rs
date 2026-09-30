//! Instance-owned runtime resources shared by application services.

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::{Notify, OwnedMutexGuard};

use crate::{CapabilityRegistry, CoreError, MomoCore, OwnedState};

#[derive(Debug)]
pub(crate) struct CancellationSignal {
    cancelled: AtomicBool,
    notify: Notify,
}

/// Dropping an aborted model future must release its runtime registration too.
pub(crate) struct CancellationRegistration<'a> {
    runtime: &'a MomoRuntime,
    request_id: String,
    signal: Arc<CancellationSignal>,
}

impl CancellationRegistration<'_> {
    pub(crate) async fn notified(&self) {
        self.signal.notified().await;
    }
}

impl Drop for CancellationRegistration<'_> {
    fn drop(&mut self) {
        let request_id = self.request_id.clone();
        let signal = Arc::clone(&self.signal);
        let _ = self.runtime.cancellations.try_call(move |registrations| {
            if registrations
                .get(&request_id)
                .is_some_and(|registered| Arc::ptr_eq(registered, &signal))
            {
                registrations.remove(&request_id);
            }
        });
    }
}

impl CancellationSignal {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) async fn notified(&self) {
        if !self.is_cancelled() {
            self.notify.notified().await;
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_one();
    }
}

/// Instance-owned composition root for Core state and concurrency controls.
///
/// Transport adapters receive a reference to this runtime; they do not own its
/// lifecycle and no runtime resource is stored in process-global state.
#[derive(Debug)]
pub struct MomoRuntime {
    core: Arc<MomoCore>,
    settings: OwnedState<crate::MomoRuntimeSettings>,
    prompt_spaces: crate::PromptSpaces,
    response_coordination: Arc<crate::orchestration::coordination::ResponseCoordination>,
    cancellations: OwnedState<HashMap<String, Arc<CancellationSignal>>>,
    capabilities: OwnedState<CapabilityRegistry>,
    memory_patch_review_locks: tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    control_locks: tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
}

impl MomoRuntime {
    /// Own a commit through caller cancellation and graceful shutdown.
    pub(crate) async fn finish_commit<T: Send + 'static>(
        &self,
        commit: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<T, tokio::sync::oneshot::error::RecvError> {
        self.core.finish_commit(commit).await
    }

    pub(crate) fn response_coordination(
        &self,
    ) -> Arc<crate::orchestration::coordination::ResponseCoordination> {
        Arc::clone(&self.response_coordination)
    }

    pub async fn initialize(data_dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let core = Arc::new(MomoCore::initialize(data_dir).await?);
        let prompt_spaces =
            crate::PromptSpaces::load_or_default(core.data_dir().join("prompt-spaces.json"))?;
        Ok(Self {
            core,
            settings: OwnedState::new(crate::MomoRuntimeSettings::default()),
            prompt_spaces,
            cancellations: OwnedState::new(HashMap::new()),
            response_coordination: Arc::default(),
            capabilities: OwnedState::new(CapabilityRegistry::default()),
            memory_patch_review_locks: tokio::sync::Mutex::new(HashMap::new()),
            control_locks: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    #[must_use]
    pub fn core(&self) -> &MomoCore {
        self.core.as_ref()
    }

    pub fn update_runtime_settings(
        &self,
        settings: crate::MomoRuntimeSettings,
    ) -> Result<(), crate::GovernanceError> {
        settings.validate()?;
        self.settings.call(move |current| *current = settings);
        Ok(())
    }

    pub fn runtime_settings(&self) -> crate::MomoRuntimeSettings {
        self.settings.call(|settings| settings.clone())
    }

    pub const fn prompt_spaces(&self) -> &crate::PromptSpaces {
        &self.prompt_spaces
    }

    pub(crate) fn core_handle(&self) -> Arc<MomoCore> {
        Arc::clone(&self.core)
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        self.core.data_dir()
    }

    pub(crate) fn register_cancellation(&self, request_id: String) -> CancellationRegistration<'_> {
        let signal = Arc::new(CancellationSignal::new());
        let id = request_id.clone();
        let registration = Arc::clone(&signal);
        self.cancellations.call(move |cancellations| {
            cancellations.insert(id, registration);
        });
        CancellationRegistration {
            runtime: self,
            request_id,
            signal,
        }
    }

    pub(crate) fn cancel_chat(&self, request_id: &str) -> bool {
        let request_id = request_id.to_owned();
        self.cancellations.call(move |cancellations| {
            let Some(signal) = cancellations.get(&request_id) else {
                return false;
            };
            signal.cancel();
            true
        })
    }

    pub(crate) const fn capabilities(&self) -> &OwnedState<CapabilityRegistry> {
        &self.capabilities
    }

    pub(crate) async fn lock_memory_patch_reviews(&self, space_id: &str) -> OwnedMutexGuard<()> {
        lock_keyed(&self.memory_patch_review_locks, space_id).await
    }

    pub(crate) async fn lock_control_resource(&self, resource_key: &str) -> OwnedMutexGuard<()> {
        lock_keyed(&self.control_locks, resource_key).await
    }

    pub(crate) async fn reserve_space(
        &self,
        space_id: uuid::Uuid,
    ) -> Result<crate::space_worker::reservations::SpaceReservation, CoreError> {
        Ok(self
            .core
            .reserve_spaces([space_id])
            .await?
            .pop()
            .expect("one Space guard"))
    }
}

async fn lock_keyed(
    keyed_locks: &tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    key: &str,
) -> OwnedMutexGuard<()> {
    let lock = {
        let mut locks = keyed_locks.lock().await;
        if locks.len() >= 1_024 {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key.to_owned(), Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}

#[cfg(test)]
#[path = "../tests/unit/runtime.rs"]
mod tests;
