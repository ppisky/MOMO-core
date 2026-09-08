# MOMO 0.5.0 Code Review and Completion Record

[简体中文](review_0_5_0.zh-CN.md)

**Status:** historical pre-release record; superseded by the Core 1.0 Space model

**Updated:** 2026-08-27

This document records the implementation that existed at the time. Plans,
placeholder interfaces, and future capabilities are not counted as complete.
[`architecture_0_5.md`](architecture_0_5.md) defines the historical boundary.

> This is no longer the current interface contract. Core 1.0 does not read the
> 0.5 wire or MOC formats and does not provide a compatibility layer.

## Four-layer boundary

| Layer | Owns | Explicitly does not own | Result |
| --- | --- | --- | --- |
| MOMO Core | Domain state, MomoApi, context, DMW/NSG/MO State, request governance, import/export, MOC, LSB | Discord/CLI protocols, provider listeners, HTTP process governance | Aligned |
| momo-server | Loopback HTTP/SSE for MomoApi, body limits, concurrency, timeouts, metrics, error mapping | Conversation orchestration, idempotent state, maintenance prompts, compatibility inference | Aligned |
| mobot adapter host | CLI/Discord mapping, session mapping, module assembly, Core process connection | Memory retrieval, context assembly, maintenance buffering, a second conversation pipeline | Aligned |
| model adapters | OpenAI Chat/Responses, Anthropic Messages, Embeddings protocol mapping and provider reliability | MOMO character, memory, state, and persistence semantics | Aligned |

An adapter may be thin and mostly forward HTTP. Its independent value is
external protocol, identity, lifecycle, and field mapping—not duplicated Core
functionality.

## Completion matrix

| Capability | Implemented | External API | Production path | Tests | Docs |
| --- | :---: | :---: | :---: | :---: | :---: |
| Native `POST /v1/momo/responses` | ✓ | ✓ | ✓ | ✓ | ✓ |
| SSE lifecycle, tool calls, cancellation | ✓ | ✓ | ✓ | ✓ | ✓ |
| Request-ID idempotency, persistent replay, conflict detection | ✓ | ✓ | ✓ | ✓ | ✓ |
| DMW/NSG/MO State and background maintenance | ✓ | ✓ | ✓ | ✓ | ✓ |
| `config.toml` / `momo.toml` ownership boundary | ✓ | ✓ | ✓ | ✓ | ✓ |
| CLI/request allow-ignore-reject governance and audit | ✓ | ✓ | ✓ | ✓ | ✓ |
| Strictly typed MOC import/export plans | ✓ | ✓ | ✓ | ✓ | ✓ |
| Original external-source byte preservation and export | ✓ | ✓ | ✓ | ✓ | ✓ |
| Explicit generated CCv2/CCv3/CHARX compatibility exports | ✓ | ✓ | ✓ | ✓ | ✓ |
| MOC v2 snapshot and compatibility profile | ✓ | ✓ | ✓ | ✓ | ✓ |
| PNG/lossless-WebP LSB with one typed payload | ✓ | ✓ | ✓ | ✓ | ✓ |
| APNG/animated-WebP/JPEG/AVIF rejection | ✓ | ✓ | ✓ | ✓ | ✓ |
| OpenAI/Anthropic/Embedding model adapters | ✓ | ✓ | ✓ | ✓ | ✓ |
| High-level Discord/CLI MomoApi adapter | ✓ | ✓ | ✓ | ✓ | ✓ |

## Boundary problems found and corrected

- Server once owned response locks, in-memory replay caches, and cancellation
  flags, effectively forming a second response state machine. These moved into
  `MomoApiService`.
- Locks for completed requests once survived indefinitely, so cancelling an ID
  could contaminate a later idempotent retry. Cancellation now applies only to
  active operations, with RAII cleanup on success, error, timeout, or dropped
  futures.
- Maintenance prompts, Patch application, and turn acknowledgement once lived
  in Server. The boundary became: Server owns no maintenance semantics; Core
  registers and runs maintenance from portable runtime policy.
- mobot's unused low-level `/v1/chat/*` client and the corresponding Core route
  were removed.
- Four mobot fields without production consumers were removed:
  `retrieval_max_tokens`, `context_history_turns`,
  `memory_distill_recent_turns`, and `nsg_govern_recent_turns`.
- Raw model completion/stream and cancellation functions stopped being public
  product APIs. Callers use `MomoApiService::execute` and `cancel`.
- Server response transport, SSE encoding, and HTTP errors moved to separate
  modules; Core orchestration remains callable without starting HTTP.

## Explicitly outside 0.5.0

- No MOC v1 migrator, incremental MOC, or deletion manifest.
- No APNG, animated WebP, JPEG, or AVIF LSB carriers.
- No second information block appended to a carrier image and no simultaneous
  second compatibility payload.
- Executable vision input was deferred to 1.0; 0.5 only froze the visual
  description prompt and override-governance boundary.
- No promise of lossless conversion for audio, video, Realtime, or arbitrary
  third-party extensions.

## Verification evidence and remaining risk

- MOMO Core workspace: 160 tests passed; strict Clippy `-D warnings` passed.
- mobot: 53 tests passed; strict Clippy `-D warnings` passed.
- Five `contracts/0.5` fixtures matched byte-for-byte SHA-256 across both repos.
- Neither repo had a Python runtime or Python test dependency at the time.

The remaining release work was not a code-boundary gap: run one local
Core-to-gateway streaming smoke test with real provider credentials, then test
permissions, reconnects, and message chunking with a real Discord application.
Ignored local configuration could still use the old single-file layout; the
operator needed to back it up and split it into two TOML files rather than let
release code silently migrate or overwrite it.
