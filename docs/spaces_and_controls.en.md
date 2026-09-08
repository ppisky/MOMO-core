# MOMO Spaces, Conversations, and Controls

[简体中文](spaces_and_controls.zh-CN.md)

This guide explains the four objects that a Core 1.0 host must keep separate.

## Four independent objects

| Object | Purpose | Persistent boundary |
| --- | --- | --- |
| Core instance root | Database, config, and all Spaces in one deployment | Not a user Space |
| Personal Space | One person's DMW, NSG, and MO State source | `personal_space_id` |
| Conversation Space | Ownership of one private or group conversation and its messages | `conversation_space_id` |
| Character | A resource referenced and switched by conversations | Global `character_id`, optional management `owner_space_id` |

There is no character-catalogue UUID. `owner_space_id` says who manages or
exports a card; runtime lookup uses only `character_id`.

## Personal and conversation Spaces

A group host may retrieve personal and shared memory together with independent
weights. Weights allocate retrieval budget and do not grant access. The host
authorizes every readable Space. One maintenance run writes to exactly one
explicit `memory_write_space_id`, even when several Spaces were read.

For a Discord group conversation, mobot normally sends sources equivalent to:

```json
[
  {"space_id":"<person>","label":"personal","weight":70},
  {"space_id":"<channel>","label":"conversation","weight":30}
]
```

The personal Space supports continuity for one person across channels. The
conversation Space prevents group A's messages and shared memory from becoming
group B's history. They are orthogonal ownership boundaries.

## New session, deletion, memory clearing, and character switching

The following operations are deliberately distinct:

- **new session** detaches the host's session mapping without deleting data;
- `delete_conversation` deletes one conversation and its messages;
- `clear_memory` clears DMW, NSG, or both in one target Space without deleting chat history;
- `switch_character` updates the character referenced by an existing conversation.

Controls use `POST /v1/momo/control` and never pass through a model. The
`actor_space_id` is audit context, not an automatic authorization grant; an
exposed HTTP host still owns authentication and its Space access table.

```json
{
  "schema": "momo.control/1.0",
  "request_id": "<unique-request-id>",
  "actor_space_id": "<personal-space>",
  "action": {
    "type": "clear_memory",
    "target_space_id": "<personal-or-group-space>",
    "memory": true,
    "semantic_graph": false
  }
}
```

`delete_conversation` and `switch_character` additionally carry both
`conversation_space_id` and `conversation_id`; Core verifies their ownership.
The same protocol clears either a personal or group Space by changing the
explicit target UUID.

Completed controls are persistently replayable by `actor_space_id` plus
`request_id`. Reusing that identity with different action content returns HTTP
409. Direct CRUD deletion routes belong to the trusted local administration
profile and are not substitutes for this end-user control protocol; see the
[HTTP boundary](http_api_1_0.md).

mobot exposes separate commands rather than overloading one reset operation:

| Intent | CLI / interactive | Discord |
| --- | --- | --- |
| Detach and create a new session | `new` / `/new` | `!momo new` |
| Delete current conversation data | `control delete-conversation` / `/delete` | `!momo delete` |
| Clear personal memory | `control clear-memory --target personal ...` / `/clear-personal` | `!momo clear personal` |
| Clear group memory | `control clear-memory --target conversation ...` / `/clear-conversation` | `!momo clear conversation` |
| Switch character | `control switch-character <UUID>` / `/character <UUID>` | `!momo character <UUID>` |

Memory clearing selects DMW, semantic graph, or both. Natural-language text
cannot acquire deletion authority.

## Weights and the write target

These are host conversation-policy choices. The host resolves them into each
`momo.responses/1.0` request's `memory_sources` and
`memory_write_space_id`. A host may represent its own policy as:

```toml
personal_memory_weight = 70
conversation_memory_weight = 30
conversation_memory_enabled = true
memory_write_target = "personal"
```

The weights are relative shares of a bounded retrieval context, not similarity
thresholds or authorization percentages. Each may be any value from 1 through
100. Disabling conversation memory stops reading that Space but does not delete
it. `memory_write_target` is either `personal` or `conversation`; Core does not
silently fall back to another Space when the selected target is unavailable.

These names are not executable MOMO Core `momo.toml` fields. Portable
round-tripping preserves them if another host owns them, but Core does not use
them to construct a request. See [`runtime_config_0_1.md`](runtime_config_0_1.md)
for the fields executed by this repository.

## MOC import and export

MOC v3 selects owner, conversation, memory, and semantic-graph Spaces
independently. Import preserves source IDs unless an explicit one-to-one
`space_map` converts them.

Different Core deployments therefore do not automatically share memory. The
MOC explicitly declares which Spaces are carried. Preserving an ID continues
the same Space; mapping it is an explicit conversion. A Core instance root is
only a deployment container for many Spaces, never one person's Space or a
character directory.

## Configuration and complete prompts

DMW and NSG maintenance prompts are referenced Markdown files. With the
standard names, `[prompts]` may be omitted and Core reads
`prompts/dmw_distiller.md` and `prompts/nsg_governor.md`. These are complete
normative prompts, not shortened TOML examples. See
[the maintenance prompt guide](maintenance_prompts.en.md) for customization.
