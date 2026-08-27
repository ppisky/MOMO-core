//! HTTP/SSE transport policy for the native MomoApi response operation.

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize},
    },
};

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use momo_core::{
    MomoApiError, MomoApiErrorKind, MomoResponse, MomoResponseRequest, ResponseOutputItem,
    api::simple,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{
    AppState, http_error::ApiError, response_stream::ResponseStream, update_route_metrics,
};

const RESPONSE_STREAM_BUFFER: usize = 64;

pub(super) async fn create_response(
    State(state): State<AppState>,
    payload: Result<Json<MomoResponseRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(|rejection| {
        let status = rejection.status();
        let code = if status == StatusCode::PAYLOAD_TOO_LARGE {
            "request_too_large"
        } else {
            "invalid_json"
        };
        ApiError::new(status, code, rejection.body_text(), false)
    })?;
    let input = request
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let request_id = request
        .momo
        .request_id
        .clone()
        .unwrap_or_else(simple::new_request_id);
    if request.stream || request.momo.stream {
        let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(RESPONSE_STREAM_BUFFER);
        let stream = ResponseStream {
            tx,
            sequence: Arc::new(AtomicU64::new(0)),
            transmitted_bytes: Arc::new(AtomicUsize::new(0)),
        };
        stream
            .send(json!({
                "type": "response.created",
                "request_id": request_id,
                "response": {"id": format!("resp_{request_id}"), "status": "in_progress"},
            }))
            .map_err(ApiError::internal)?;
        tokio::spawn({
            let stream = stream.clone();
            let request_id = request_id.clone();
            async move {
                match execute_response_bounded(
                    state,
                    request,
                    request_id.clone(),
                    input,
                    Some(stream.clone()),
                )
                .await
                {
                    Ok(response) => send_completed_events(&stream, &request_id, response),
                    Err(error) => {
                        let _ = stream.send(json!({
                            "type": "response.failed",
                            "request_id": request_id,
                            "error": error.error,
                        }));
                    }
                }
            }
        });
        return Ok(Sse::new(ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response());
    }
    let response = execute_response_bounded(state, request, request_id, input, None).await?;
    Ok(Json(response).into_response())
}

fn send_completed_events(stream: &ResponseStream, request_id: &str, response: MomoResponse) {
    let message_id = format!("msg_{request_id}");
    let _ = stream.send(json!({
        "type": "response.output_text.done",
        "request_id": request_id,
        "item_id": message_id.as_str(),
        "output_index": 0,
        "content_index": 0,
        "text": response.output_text.as_str(),
    }));
    let _ = stream.send(json!({
        "type": "response.content_part.done",
        "request_id": request_id,
        "item_id": message_id.as_str(),
        "output_index": 0,
        "content_index": 0,
        "part": {
            "type": "output_text",
            "text": response.output_text.as_str(),
            "annotations": [],
        },
    }));
    if let Some(item) = response.output.first() {
        let _ = stream.send(json!({
            "type": "response.output_item.done",
            "request_id": request_id,
            "output_index": 0,
            "item": item,
        }));
    }
    for (output_index, item) in response.output.iter().enumerate().skip(1) {
        if let ResponseOutputItem::FunctionCall { id, arguments, .. } = item {
            let _ = stream.send(json!({
                "type": "response.function_call_arguments.done",
                "request_id": request_id,
                "item_id": id,
                "output_index": output_index,
                "arguments": arguments,
            }));
        }
        let _ = stream.send(json!({
            "type": "response.output_item.done",
            "request_id": request_id,
            "output_index": output_index,
            "item": item,
        }));
    }
    let _ = stream.send(json!({
        "type": "response.completed",
        "request_id": request_id,
        "response": response,
    }));
}

async fn execute_response_bounded(
    state: AppState,
    request: MomoResponseRequest,
    request_id: String,
    input: String,
    stream: Option<ResponseStream>,
) -> Result<MomoResponse, ApiError> {
    let started = std::time::Instant::now();
    let route = request.model.clone();
    let permit = Arc::clone(&state.response_concurrency)
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::rate_limited("response concurrency limit reached")
                .with_request_id(&request_id)
        })?;
    update_route_metrics(&state, &route, |metrics| {
        metrics.requests = metrics.requests.saturating_add(1);
        metrics.inflight = metrics.inflight.saturating_add(1);
    })
    .await;
    let result = match tokio::time::timeout(
        state.response_timeout,
        execute_response(state.clone(), request, request_id.clone(), input, stream),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            let _ = state.momo_api.cancel(&request_id);
            Err(ApiError::gateway_timeout(
                "response orchestration timed out",
            ))
        }
    };
    drop(permit);
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    update_route_metrics(&state, &route, |metrics| {
        metrics.inflight = metrics.inflight.saturating_sub(1);
        metrics.latency_ms = metrics.latency_ms.saturating_add(elapsed);
        match &result {
            Ok(response) => {
                metrics.succeeded = metrics.succeeded.saturating_add(1);
                metrics.input_tokens = metrics
                    .input_tokens
                    .saturating_add(response.usage.input_tokens);
                metrics.output_tokens = metrics
                    .output_tokens
                    .saturating_add(response.usage.output_tokens);
            }
            Err(error) if error.error.code == "cancelled" => {
                metrics.cancelled = metrics.cancelled.saturating_add(1);
            }
            Err(error) if error.error.code == "timeout" => {
                metrics.timed_out = metrics.timed_out.saturating_add(1);
            }
            Err(_) => metrics.failed = metrics.failed.saturating_add(1),
        }
    })
    .await;
    result.map_err(|error| error.with_request_id(&request_id))
}

async fn execute_response(
    state: AppState,
    request: MomoResponseRequest,
    request_id: String,
    input: String,
    stream: Option<ResponseStream>,
) -> Result<MomoResponse, ApiError> {
    state
        .momo_api
        .execute(
            &request,
            &request_id,
            &input,
            stream
                .as_ref()
                .map(|value| value as &dyn momo_core::MomoResponseEventSink),
        )
        .await
        .map_err(momo_api_error)
}

pub(super) fn model_api_error(error: String) -> ApiError {
    let lowercase = error.to_ascii_lowercase();
    if lowercase.contains("cancelled") {
        return ApiError::cancelled();
    }
    if lowercase.contains("timed out") || lowercase.contains("timeout") {
        return ApiError::gateway_timeout("model gateway timed out");
    }
    if let Some(status) = parse_upstream_status(&error) {
        if status == 429 {
            let mut api_error = ApiError::rate_limited("model gateway rate limit exceeded");
            api_error.error.upstream_status = Some(status);
            return api_error;
        }
        let mut api_error = ApiError::bad_gateway(format!("model gateway returned HTTP {status}"));
        api_error.error.upstream_status = Some(status);
        api_error.error.retryable = status >= 500;
        return api_error;
    }
    ApiError::bad_gateway(error)
}

fn momo_api_error(error: MomoApiError) -> ApiError {
    match error.kind {
        MomoApiErrorKind::BadRequest => ApiError::bad_request(error.message),
        MomoApiErrorKind::Conflict => ApiError::conflict(error.message),
        MomoApiErrorKind::Cancelled => ApiError::cancelled(),
        MomoApiErrorKind::Model => model_api_error(error.message),
        MomoApiErrorKind::Internal => ApiError::internal(error.message),
    }
}

fn parse_upstream_status(message: &str) -> Option<u16> {
    let marker = "HTTP ";
    let start = message.find(marker)? + marker.len();
    message[start..]
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

pub(super) async fn cancel_response(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Json<Value> {
    let cancelled = state.momo_api.cancel(&request_id);
    Json(json!({
        "request_id": request_id,
        "cancelled": cancelled,
    }))
}
