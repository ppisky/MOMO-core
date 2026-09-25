use super::*;
use std::sync::atomic::{AtomicU64, AtomicUsize};

#[tokio::test]
async fn oversized_completion_fails_before_emitting_a_partial_terminal_sequence() {
    let (tx, mut rx) = mpsc::channel(8);
    let stream = ResponseStream {
        tx,
        sequence: Arc::new(AtomicU64::new(0)),
        transmitted_bytes: Arc::new(AtomicUsize::new(0)),
    };
    let output_text = "x".repeat(momo_core::MAX_RESPONSE_SSE_EVENT_BYTES + 1);
    let response = MomoResponse {
        id: "resp_large".to_owned(),
        object: "response".to_owned(),
        status: "completed".to_owned(),
        model: "conversation".to_owned(),
        output: vec![ResponseOutputItem::Message {
            id: "msg_large".to_owned(),
            role: "assistant".to_owned(),
            status: "completed".to_owned(),
            content: vec![momo_core::ResponseOutputContent::OutputText {
                text: output_text.clone(),
                annotations: Vec::new(),
            }],
        }],
        output_text,
        usage: momo_core::ResponseUsage::default(),
        finish_reason: Some("stop".to_owned()),
        momo: momo_core::MomoResponseMetadata {
            schema: momo_core::MOMO_RESPONSE_SCHEMA.to_owned(),
            request_id: "large".to_owned(),
            conversation_id: "conversation".to_owned(),
            route: "conversation".to_owned(),
            upstream_request_id: None,
            warnings: Vec::new(),
            state_audit: Value::Null,
            request_audit: Value::Null,
        },
    };

    let error = send_completed_events(&stream, "large", response)
        .await
        .expect_err("oversized terminal event");
    assert!(rx.try_recv().is_err(), "terminal batch must be atomic");
    send_stream_limit_failure(&stream, "large", error)
        .await
        .expect("failure event");
    assert!(
        rx.try_recv().is_ok(),
        "failure event must remain deliverable"
    );
}
