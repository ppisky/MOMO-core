//! Instance-wide request exclusion and task lifecycle; no domain policy lives here.

use super::MomoApiError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as SyncMutex, Weak},
};
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Default)]
pub(crate) struct ResponseCoordination {
    pub(super) response_attempts: Arc<Mutex<HashMap<String, String>>>,
    pub(super) operation_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    pub(super) conversation_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    operation_states: Arc<SyncMutex<HashMap<String, OperationState>>>,
    pub(super) maintenance_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    pub(super) generation_gates: Arc<Mutex<HashMap<String, Weak<Semaphore>>>>,
    pub(super) maintenance_tasks: Arc<SyncMutex<Vec<tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug, Default)]
struct OperationState {
    active: usize,
    cancelled: bool,
    scope_id: String,
}

pub(super) struct OperationGuard {
    request_id: String,
    states: Arc<SyncMutex<HashMap<String, OperationState>>>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = states.get_mut(&self.request_id) else {
            return;
        };
        state.active -= 1;
        if state.active == 0 {
            states.remove(&self.request_id);
        }
    }
}

impl ResponseCoordination {
    pub(super) fn cancel_operation(&self, request_id: &str) -> bool {
        self.operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(request_id)
            .is_some_and(|state| {
                state.cancelled = true;
                state.active > 0
            })
    }
    pub(super) fn enter_operation(&self, request_id: &str, scope_id: &str) -> OperationGuard {
        let mut states = self
            .operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = states.entry(request_id.to_owned()).or_default();
        state.active += 1;
        state.scope_id = scope_id.to_owned();
        OperationGuard {
            request_id: request_id.to_owned(),
            states: Arc::clone(&self.operation_states),
        }
    }

    pub(crate) fn ensure_active(&self, request_id: &str) -> Result<(), MomoApiError> {
        if self
            .operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(request_id)
            .is_some_and(|state| state.cancelled)
        {
            Err(MomoApiError::cancelled())
        } else {
            Ok(())
        }
    }

    pub(super) fn has_active_responses(&self, scope_id: &str) -> bool {
        self.operation_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|state| state.active > 0 && state.scope_id == scope_id)
    }

    pub(super) async fn generation_gate(&self, scope_id: &str, lane: &str) -> Arc<Semaphore> {
        let mut gates = self.generation_gates.lock().await;
        let gate_key = format!("{lane}:{scope_id}");
        if gates.len() >= 1_024 {
            gates.retain(|_, gate| gate.strong_count() > 0);
        }
        if let Some(gate) = gates.get(&gate_key).and_then(Weak::upgrade) {
            gate
        } else {
            let gate = Arc::new(Semaphore::new(1));
            gates.insert(gate_key, Arc::downgrade(&gate));
            gate
        }
    }

    pub(super) async fn conversation_lock(
        &self,
        scope_id: &str,
        conversation_id: &str,
    ) -> Arc<Mutex<()>> {
        let mut locks = self.conversation_locks.lock().await;
        if locks.len() >= 1_024 {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        let key = format!("{scope_id}:{conversation_id}");
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        }
    }
}
