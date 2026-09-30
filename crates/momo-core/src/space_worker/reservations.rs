//! Mailbox admission for operations spanning several Space commands/stores.
//! A reservation carries no workspace or mutex; dropping it sends a release.
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, mpsc};
use tokio::sync::oneshot;
use uuid::Uuid;

enum Request {
    Acquire(BTreeSet<Uuid>, oneshot::Sender<SpaceReservation>),
    Release(BTreeSet<Uuid>),
}

#[derive(Debug, Clone)]
pub(crate) struct SpaceReservations(Arc<mpsc::Sender<Request>>);

pub(crate) struct SpaceReservation {
    spaces: BTreeSet<Uuid>,
    sender: Arc<mpsc::Sender<Request>>,
}

impl Drop for SpaceReservation {
    fn drop(&mut self) {
        let _ = self
            .sender
            .send(Request::Release(std::mem::take(&mut self.spaces)));
    }
}

impl SpaceReservations {
    pub(crate) fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let sender = Arc::new(sender);
        let weak = Arc::downgrade(&sender);
        std::thread::Builder::new()
            .name("space-admission".into())
            .spawn(move || {
                let mut active = BTreeSet::new();
                let mut pending = VecDeque::new();
                while let Ok(request) = receiver.recv() {
                    match request {
                        Request::Acquire(spaces, reply) => pending.push_back((spaces, reply)),
                        Request::Release(spaces) => active.retain(|id| !spaces.contains(id)),
                    }
                    let mut blocked = BTreeSet::new();
                    let mut remaining = VecDeque::new();
                    while let Some((spaces, reply)) = pending.pop_front() {
                        if reply.is_closed() {
                            continue;
                        }
                        if !spaces.is_disjoint(&active) || !spaces.is_disjoint(&blocked) {
                            blocked.extend(&spaces);
                            remaining.push_back((spaces, reply));
                        } else if let Some(sender) = weak.upgrade() {
                            active.extend(&spaces);
                            let _ = reply.send(SpaceReservation { spaces, sender });
                        }
                    }
                    pending = remaining;
                }
            })
            .expect("start Space admission owner");
        Self(sender)
    }

    pub(crate) async fn acquire(
        &self,
        spaces: BTreeSet<Uuid>,
    ) -> Result<SpaceReservation, crate::CoreError> {
        let (reply, result) = oneshot::channel();
        let error =
            || momo_memory::MemoryError::InvalidPatch("Space admission supervisor stopped".into());
        self.0
            .send(Request::Acquire(spaces, reply))
            .map_err(|_| error())?;
        Ok(result.await.map_err(|_| error())?)
    }
}
