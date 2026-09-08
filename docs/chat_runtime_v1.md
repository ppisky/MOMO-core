# MOMO native response runtime

**Status:** 1.0 local release candidate; not published
**Updated:** 2026-09-05

The production conversation entry point is the native `MomoApi` operation,
exposed by `momo-server` as `POST /v1/momo/responses`. The older low-level
`chat_complete_json` event protocol is an internal outbound adapter boundary,
not the client-facing runtime contract.

Every request must explicitly include `momo.schema = "momo.responses/1.0"`.
Missing, unknown, and future schema generations are rejected. See the
[HTTP boundary](http_api_1_0.md) for the distinction between the stable product
wire and local administration routes.

## Operation sequence

```text
validate request and request ID
  -> validate the personal Space, conversation Space, and global character ID
  -> replay a completed operation or lock a new attempt within the personal Space
  -> govern request overrides
  -> pass original images to a multimodal conversation route, or optionally
     describe them through logical route `vision` for a text-only model
  -> atomically persist the resolved user input
  -> in closed-autonomous mode, recover due per-Space maintenance
  -> lock the managed Space, retrieve DMW/NSG, observe source revisions,
     and publish or replay a durable MO State scene/state snapshot
  -> assemble a bounded context
  -> call the logical conversation route
  -> atomically persist the assistant result, maintenance turn, and completed
     response replay record
```

The v2 MO State manager is request-woken rather than a permanently running
thread. Its per-Space lease serializes responses with direct DMW/NSG mutation,
while separate Spaces remain concurrent. A successful response completion and
the corresponding `projected -> completed` state operation transition commit
in one SQLite transaction. The next event recovers pending maintenance before
observing source fingerprints, so a closed chat host does not need to call DMW,
NSG, or scene endpoints itself.

## Roleplay context contract

Core owns stored conversation history. A structured request may submit new
`user` message/input blocks or a paired tool continuation; it may not submit
`system` or `assistant` message items. Runtime system instructions use the
top-level `instructions` field and pass the configured allow/ignore/reject
governance policy. Once allowed, they are placed in system context rather than
in trimmable conversation history.

The system context is assembled in this logical order: governed runtime
instructions, current character Markdown, character-relative user Markdown,
retrieved DMW, MO State, then active NSG lore. Multi-Space DMW and NSG entries
retain their source label and Space ID in the rendered context so personal and
shared facts are not silently presented as one owner. The complete system
message and newest conversation messages share one hard input budget;
`truncated_messages` reports when either a system message or retained history
message had to be shortened.

An existing conversation resolves the current stored content of its bound
`character_id` on every response. Updating that card therefore affects future
turns in existing conversations; Core 1.0 does not pin a character revision per
conversation. `opening_markdown` is a portable session-opening asset, but the
native response operation does not insert or render it automatically. A host
that wants an opening message must expand any template variables and present
it itself; merely creating a conversation does not add the asset to history,
DMW, or MO State.

When a maintenance threshold is reached, Core retrieves relevant existing DMW
and NSG material from the explicit memory Spaces and sends that read-only context,
the pending turns, and the current timestamp to the selected maintenance route.
Context retrieval must succeed before a model may propose a patch; Core never
falls back to transcript-only blind writes.
The generated maintenance patch is durably staged before file mutation. A
crashed batch reuses that exact patch, whose DMW and NSG operations are
idempotently replayable, and only then atomically acknowledges its source turns.

Image references are never silently dropped. When image input is present,
`vision.enabled` must be true. If the conversation route advertises `image`,
Core sends the original image blocks directly and does not use the fallback
prompt. Otherwise, the gateway's optional `vision` route must advertise the
`image` modality and Core persists bounded visual descriptions rather than raw
image bytes. A pending operation stores its resolved input and vision usage, so
a crash/retry does not repeat visual inference.

For a direct multimodal turn, persisted text history contains the user's text
and an image-count marker, not image bytes or a generated description. The
original image is therefore available to the conversation model only on that
turn. The text-only fallback path persists its bounded visual description, so
that description remains available later. Hosts that need durable visual recall
with a multimodal main model must explicitly supply safe text or store an
authorized memory; Core does not retain remote image URLs as chat history.

A tool continuation must identify an existing conversation and include exactly
one preceding `function_call` for each `function_call_output`, paired by
`call_id`. It is a new idempotent response operation in the same conversation,
not a resume of the original response request ID. Tool-only JSON participates
in the current model call and is classified as a `tool_result`, but it is not
persisted as a user-authored chat message. The completed textual assistant
answer is persisted normally. Durable paused-run state and Core-enforced
multi-step tool-loop limits are outside `momo.responses/1.0`.

## Context and budget

Core keeps character instructions, user context, DMW memory, MO State, NSG,
and conversation messages as separate sections until final assembly. The
effective context window and output reserve come from capability discovery and
portable request governance. If optional embedding, retrieval, or MO State
work fails, the response continues with a warning; failure of the selected
conversation or vision model route fails the operation.

When the system content exceeds its available budget, Core allocates capped
weighted shares to runtime instructions / character / user / memory / state /
semantic graph in the ratio 4:4:2:3:3:2. Empty sections do not participate;
sections that fit return their unused share to the others. These are context
allocation weights, independent of cross-Space retrieval weights. Rendering
order stays unchanged. The newest turn retains its existing reservation, and
Core checks the combined text against the selected token counter after joining
sections. Extremely small budgets can still omit sections.

`PreparedContext.section_audit` identifies each nonempty source section and
reports `original_tokens`, `injected_tokens`, `truncated`, and `omitted`.
Counts include section headings and any injected truncation marker but exclude
shared message overhead; their sum need not equal the tokenized combined text.
Native responses copy these diagnostics into `momo.request_audit.text_context`
and warn when a system section is shortened. Governance approval does not mean
every approved instruction survived an impossibly small context budget.
This diagnostic explicitly flags when subsequently appended structured input
is excluded from its text-only count; it is not a claim to count image tokens.

## Streaming

Foreground model generation and background maintenance have independent
per-Space slots. One foreground call and one maintenance call may run together;
DMW and NSG maintenance share the background slot. This bounds local generation
concurrency without making a foreground reply wait for an entire maintenance
model call. Provider-wide admission control still belongs to the gateway, so a
reserved Core slot does not guarantee upstream capacity or a latency SLA.
Memory writes retain their existing locks and revision checks. Explicit
maintenance drains still wait for completion and are not ordinary reply latency.

The HTTP transport uses bounded SSE with `response.*` lifecycle events,
including `response.created`, output-item/content deltas, terminal
`response.completed`, and `response.failed`. The server-side channel is bounded
to 64 events, individual events and the total stream have explicit byte limits,
and UTF-8/SSE decoding does not assume network chunk boundaries.

Cancellation is addressed by request ID. Completed responses replay from local
storage; both operations are namespaced by the request's `personal_space_id`.
Reusing a request ID with a different normalized request inside that scope
returns a conflict. A conversation ID is accepted only when it belongs to the
explicit `conversation_space_id`, and its messages are read through the same
scoped lookup. Partial assistant text is not committed as a completed assistant
message. See [`space_model_1_0.md`](space_model_1_0.md) for the complete
identity boundary.

Only the frozen 1.0 fixtures, including multimodal input, live under
`contracts/1.0`; publishing still waits for the credentialed provider smoke test. See
[`roadmap_1_0_0.md`](roadmap_1_0_0.md).
