# MOMO Prompt Spaces

[简体中文](maintenance_prompts.zh-CN.md)

Prompt Spaces are process-wide, named prompt resources. MOMO Core ships reviewed
Markdown defaults, but the active value is no longer limited to the value
compiled into the binary. A host replaces an active value through the native
HTTP API. Core persists overrides as internal JSON state under its data root;
it does not read prompt content from `momo.toml`.

## Fixed spaces

| id | default role |
| --- | --- |
| `assistant` | General system instruction; defaults to `You are a helpful assistant.` |
| `vision_fallback` | Instruction for the text description produced when the conversation route cannot accept images |
| `roleplay_director` | Foreground character-performance policy |
| `memory_distillation` | DMW maintenance policy |
| `semantic_graph_governance` | NSG governance policy |

These identifiers are a closed contract. Arbitrary prompt names are rejected,
so a typo cannot create unused state. Prompt Spaces are not Spaces, have no
per-user ownership, and are not included in MOC import or export.

## HTTP API

```http
GET /v1/prompt-spaces
GET /v1/prompt-spaces/assistant
PUT /v1/prompt-spaces/assistant
Content-Type: application/json

{"content":"You answer accurately and concisely."}
```

`PUT` replaces the whole value and returns the active resource. Content must be
non-blank and no larger than 262,144 bytes. The response contains `source`
(`builtin` or `override`) and a SHA-256 `revision` of the active content.

```http
DELETE /v1/prompt-spaces/assistant
```

`DELETE` removes the override and restores the compiled default; it does not
delete the named resource. Replacements are visible to future prompt lookups
without restarting Core and survive restart. Persistence happens before the
in-memory value is committed, so a failed write does not report or expose a
replacement that was not stored.

The default `assistant` value is used only when a response has neither governed
request `instructions` nor a non-empty character card. Loading a character
suppresses the generic assistant default. Explicit request instructions remain
when the override policy allows them.

`roleplay_director` is the static execution policy placed in the final System
context's `# Roleplay Direction` section. It is not MO State: MO State supplies
current scene, epistemic, and state evidence, while the director tells the
model how to perform the character from the card and that evidence.
`roleplay.enabled` and `vision.enabled` belong to `/v1/runtime-settings`; their
prompt bodies are Prompt Spaces.

## Host configuration boundary

A host may keep user-facing configuration in any format. For example, mobot
may read its own TOML prompt settings and issue one `PUT` request per configured
Prompt Space during reconciliation. That translation belongs to mobot:

```text
user configuration -> host validation -> HTTP PUT /v1/prompt-spaces/:id -> MOMO
```

MOMO does not parse mobot configuration, accept prompt file paths from it, or
restore the former TOML prompt fields. This keeps the API usable by every host
without making mobot a protocol dependency.

## Source and validation

Defaults remain reviewable under `crates/momo-core/src/product_prompts/` and are
embedded with `include_str!`. Compilation therefore still fails when a default
source is absent, while operators are free to replace active values through the
same HTTP contract in every build.

From the repository root:

```bash
cargo test -p momo_core prompt_spaces
cargo test -p momo-server prompt_spaces
```
