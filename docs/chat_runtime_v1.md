# MOMO native response runtime

**Status:** 1.0 local release candidate; not published
**Updated:** 2026-08-29

The production conversation entry point is the native `MomoApi` operation,
exposed by `momo-server` as `POST /v1/momo/responses`. The older low-level
`chat_complete_json` event protocol is an internal outbound adapter boundary,
not the client-facing runtime contract.

## Operation sequence

```text
validate request and request ID
  -> validate personal, conversation, and character-catalogue scopes
  -> replay a completed operation or lock a new attempt within the personal scope
  -> govern request overrides
  -> pass original images to a multimodal conversation route, or optionally
     describe them through logical route `vision` for a text-only model
  -> atomically persist the resolved user input
  -> retrieve DMW/NSG and compile MO State
  -> assemble a bounded context
  -> call the logical conversation route
  -> persist the assistant result and maintenance turn
  -> store the completed response for idempotent replay
```

Image references are never silently dropped. When image input is present,
`vision.enabled` must be true. If the conversation route advertises `image`,
Core sends the original image blocks directly and does not use the fallback
prompt. Otherwise, the gateway's optional `vision` route must advertise the
`image` modality and Core persists bounded visual descriptions rather than raw
image bytes. A pending operation stores its resolved input and vision usage, so
a crash/retry does not repeat visual inference.

## Context and budget

Core keeps character instructions, user context, DMW memory, MO State, NSG,
and conversation messages as separate sections until final assembly. The
effective context window and output reserve come from capability discovery and
portable request governance. If optional embedding, retrieval, or MO State
work fails, the response continues with a warning; failure of the selected
conversation or vision model route fails the operation.

## Streaming

The HTTP transport uses bounded SSE with `response.*` lifecycle events,
including `response.created`, output-item/content deltas, terminal
`response.completed`, and `response.failed`. The server-side channel is bounded
to 64 events, individual events and the total stream have explicit byte limits,
and UTF-8/SSE decoding does not assume network chunk boundaries.

Cancellation is addressed by request ID. Completed responses replay from local
storage; both operations are namespaced by the request's personal `scope_id`.
Reusing a request ID with a different normalized request inside that scope
returns a conflict. A conversation ID is accepted only when it belongs to the
explicit `conversation_scope_id`, and its messages are read through the same
scoped lookup. Partial assistant text is not committed as a completed assistant
message. See [`identity_scope_1_0.md`](identity_scope_1_0.md) for the complete
identity boundary.

The exact historical fixtures remain under `contracts/0.5`. The frozen 1.0
fixtures, including multimodal input, live under `contracts/1.0`; publishing
still waits for the credentialed provider smoke test. See
[`roadmap_1_0_0.md`](roadmap_1_0_0.md).
