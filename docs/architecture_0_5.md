# MOMO 0.5 architecture and boundary contract

This document is the normative architecture contract for MOMO 0.5. When an
older roadmap, example, or implementation disagrees with this document, this
document wins until the disagreement is removed.

## 1. Core and adapters

MOMO Core owns the native `MomoApi` contract and all domain behaviour:

- characters, conversations, messages, scopes, and local persistence;
- DMW, NSG governance, MO State, retrieval, and context construction;
- response-operation state, idempotency, cancellation, and maintenance;
- typed import and export planning;
- MOC, character compatibility, and LSB carrier codecs;
- request-governance policy and the effective-request audit.

An adapter translates between one external system and `MomoApi`. Adapters do
not own memory, context construction, or model-governance policy. Built-in or
separately distributed adapters may include:

- CLI and Discord host adapters;
- OpenAI Chat Completions, OpenAI Responses, and Anthropic Messages ingress;
- model-provider egress;
- future visual-description adapters.

`momo-server` is the HTTP/SSE transport for `MomoApi`. Business orchestration
must be callable without starting the server. `mobot` is an adapter host and
may expose or consume configured adapters, but it must not duplicate Core's
conversation pipeline.

## 2. Configuration ownership

MOMO uses two TOML documents with different portability and trust boundaries.

### `momo.toml`

Portable product behaviour. It may be included in a MOC snapshot and contains:

- model-use routes expressed as logical adapter names;
- context, memory, NSG, and MO State policy;
- request-override policy;
- visual-description prompts and whether a request may override them;
- character/runtime defaults that are safe to move between hosts.

It must never contain credentials, absolute host paths, listener addresses, or
Discord identifiers.

### `config.toml`

Host-local deployment and adapter wiring. It contains:

- adapter declarations and ordering;
- provider endpoints, protocols, model IDs, and credential environment names;
- listener addresses and authentication policy;
- executable, data, and run directories;
- Discord and other host-specific settings.

It is not included in a portable MOC by default. Secrets remain in environment
variables or an external secret store.

CLI flags are an ephemeral overlay. A flag may override a field only when the
corresponding `momo.toml` governance policy allows that class of override. The
effective value and its source (`default`, `momo`, `config`, or `request`) must
be auditable.

## 3. Request governance

Every model adapter receives an already governed request. Policy is explicit
per field rather than an unrestricted JSON merge.

- Context window is a model capability. A request may ask for a smaller
  working window only when allowed; it may never enlarge the discovered or
  configured capability.
- Output-token, sampling, stop, tool, system-instruction, and visual-prompt
  overrides are independently allow/deny/clamp governed.
- Unknown provider parameters are rejected unless the selected adapter policy
  explicitly allows and names them.
- Core instructions, character instructions, MO State, and user-supplied
  instructions remain separate until the final context assembly audit.
- A model route may ignore a denied request value, but the response metadata
  must report that decision. Silent policy bypass is not allowed.

Visual input is an optional 1.0-facing capability. When enabled, a dedicated
adapter converts images to governed visual-description text. The configured
description prompt may be replaced per request only when policy allows it.

## 4. Import and export algebra

Import and export operations are tagged plans. They are not inferred from file
extensions alone and are not represented by a collection of ambiguous boolean
flags.

An import plan declares exactly one source type and one intended operation.
Examples include native MOC snapshot, external Character Card JSON, external
CHARX, and LSB image carrier.

An export plan declares exactly one artifact type:

- `stored_source`: export the original preserved source bytes without
  conversion;
- `character`: export a generated external compatibility artifact;
- `moc`: export a native MOC snapshot, optionally including preserved external
  compatibility data as an explicit compatibility profile;
- `lsb_image`: embed exactly one typed payload in a lossless image carrier.

Compatibility is an enum (`none`, `preserved_source`, or a named generated
profile), not a boolean. Unsupported combinations fail before any output file
is created.

`lsb_image` has one carrier and one payload. The payload is one of native MOMO
character data, a complete MOC, or a complete CHARX. When the payload is MOC,
no metadata is appended after the image and no second compatibility payload is
embedded.

## 5. Media and container formats

- A MOC file uses the `.moc` extension. Its bytes are a tar archive compressed
  with the Zstandard algorithm. The conventional compound extension for such
  an archive is `.tar.zst`; `.tar.zstd` is not a MOMO format or extension.
- MOMO 0.5 reads and writes MOC format version 2 only. There is intentionally no
  MOC v1 migration path because no supported installed base requires one.
- LSB image carriers support PNG and lossless WebP. They do not support APNG,
  animated WebP, JPEG, or AVIF.
- APNG is rejected rather than selecting an arbitrary frame.
- Character Card compatibility and LSB transport are separate axes. A carrier
  image is not implicitly an external Character Card.

## 6. Module split rule

A unit should become an independent module when it has its own contract,
state/lifecycle, error vocabulary, or test boundary. File size alone is not the
criterion. In particular, HTTP routing, response orchestration, maintenance,
gateway transport, configuration governance, and portable-media operations
must not remain one server module.

## 7. Release rule

0.5.0 is releasable only when every advertised capability is marked across five
independent columns: implemented, exposed, integrated, tested, and documented.
Schema-only or fixture-only work is not `implemented`. A Rust-only codec is not
`exposed` through `MomoApi`. A route that is never used by the production path
is not `integrated`.

After this contract is met, 1.0 may add optional multimodal/vision adapters
without changing the Core ownership boundary.
