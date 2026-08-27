use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use axum::response::sse::Event;
use momo_core::{MAX_RESPONSE_SSE_EVENT_BYTES, MAX_RESPONSE_STREAM_BYTES};
use serde_json::{Value, json};
use tokio::sync::mpsc;

#[derive(Clone)]
pub(super) struct ResponseStream {
    pub(super) tx: mpsc::Sender<Result<Event, Infallible>>,
    pub(super) sequence: Arc<AtomicU64>,
    pub(super) transmitted_bytes: Arc<AtomicUsize>,
}

impl ResponseStream {
    pub(super) fn send(&self, mut event: Value) -> Result<(), String> {
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel);
        let object = event
            .as_object_mut()
            .ok_or_else(|| "response stream event must be an object".to_owned())?;
        object.insert("sequence".to_owned(), json!(sequence));
        let encoded = serde_json::to_string(&event).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_RESPONSE_SSE_EVENT_BYTES {
            return Err("response SSE event exceeded its byte limit".to_owned());
        }
        let previous = self
            .transmitted_bytes
            .fetch_add(encoded.len(), Ordering::AcqRel);
        if previous.saturating_add(encoded.len()) > MAX_RESPONSE_STREAM_BYTES {
            self.transmitted_bytes
                .fetch_sub(encoded.len(), Ordering::AcqRel);
            return Err("response stream exceeded its cumulative byte limit".to_owned());
        }
        self.tx
            .try_send(Ok(Event::default().data(encoded)))
            .map_err(|error| error.to_string())
    }
}

impl momo_core::MomoResponseEventSink for ResponseStream {
    fn send(&self, event: Value) -> Result<(), String> {
        Self::send(self, event)
    }
}
