//! Single-owner, volatile coordination state. A failed owner closes its mailbox;
//! callers never recover a partially mutated value or a poisoned lock.
use std::{fmt, sync::mpsc};

type Command<T> = Box<dyn FnOnce(&mut T) + Send>;
pub(crate) struct OwnedState<T> {
    sender: mpsc::SyncSender<Command<T>>,
}

impl<T> fmt::Debug for OwnedState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedState").finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Default for OwnedState<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Send + 'static> OwnedState<T> {
    pub(crate) fn new(mut state: T) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Command<T>>(64);
        std::thread::Builder::new()
            .name("core-state-owner".into())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    command(&mut state);
                }
            })
            .expect("start state owner");
        Self { sender }
    }

    pub(crate) fn try_call<R: Send + 'static>(
        &self,
        command: impl FnOnce(&mut T) -> R + Send + 'static,
    ) -> Result<R, &'static str> {
        let (reply, result) = mpsc::sync_channel(1);
        self.sender
            .send(Box::new(move |state| {
                let _ = reply.send(command(state));
            }))
            .map_err(|_| "state owner stopped")?;
        result
            .recv()
            .map_err(|_| "state owner failed; state discarded")
    }

    pub(crate) fn call<R: Send + 'static>(
        &self,
        command: impl FnOnce(&mut T) -> R + Send + 'static,
    ) -> R {
        self.try_call(command)
            .expect("coordination owner unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panicked_owner_never_exposes_partially_mutated_state() {
        let state = OwnedState::new(vec![1]);
        assert!(
            state
                .try_call::<()>(|state| {
                    state.push(2);
                    panic!("injected owner panic");
                })
                .is_err()
        );
        assert!(state.try_call(|state| state.clone()).is_err());
    }
}
