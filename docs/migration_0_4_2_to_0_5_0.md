# MOMO 0.5 development transition notes

MOMO 0.5 is a pre-1.0 contract reset, not a compatibility release. There is no
supported installed base that justifies retaining ambiguous 0.4 APIs or adding
a MOC v1 migrator. Rebuild the application and create new 0.5 development data
when an old development snapshot conflicts with the frozen contract.

The native high-level entry point is `POST /v1/momo/responses`; it is part of
`MomoApi`, not an OpenAI-compatible Responses endpoint. OpenAI Responses,
Chat Completions, Anthropic Messages, Discord, CLI, model providers, and future
vision components are adapters around the native contract.

MOMO 0.5 reads and writes MOC format version 2 snapshots only. It rejects MOC
v1, incremental/deletion package fields, APNG, animated WebP, AVIF, and
undeclared import/export combinations. A `.moc` file is a tar archive compressed
with Zstandard; its extension is `.moc`.

Deployment uses two files:

- `config.toml` contains host-local adapter wiring, endpoints, listener policy,
  executable/data paths, Discord settings, and credential environment names.
- `momo.toml` contains portable behaviour, logical model use, memory/state
  policy, request-override governance, and the future visual-description prompt.

CLI/request values are not an unrestricted merge. `momo.toml` declares whether
context window, output tokens, sampling, instructions, tools, visual prompt,
and named provider parameters are allowed, ignored, or rejected. Native
responses expose the effective-request audit.

Streaming callers must dispatch by event `type` and preserve monotonic
`sequence`. Explicit cancellation uses
`POST /v1/momo/responses/{request_id}/cancel`.
