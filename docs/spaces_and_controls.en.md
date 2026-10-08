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

A person normally keeps a stable personal Space. Hosts may also configure
stable function Spaces; selecting an assistant, editing its prompt, or starting
a conversation need not change Spaces. Function Spaces describe management and
access boundaries, not the originating assistant, participants, or applicability
of every record. Core does not prescribe the host's feature layout.

The [memory identity and provenance extension](memory_identity_and_provenance.md)
uses default isolation and explicit sharing with stable assistant identity and
mutable prompts. User preferences may be shared; A's experiences and promises
do not transfer to B. Shared reads of an authoritative record are preferred to
automatic per-function copies. The working tree implements the record checks
in [Profile 1](memory_provenance_runtime.md); hosts still authorize Space access.

A group host may retrieve personal and shared memory together with independent
weights. Weights allocate retrieval budget and do not grant access. The host
authorizes every readable Space. One maintenance run writes to exactly one
explicit `memory_write_space_id`, even when several Spaces were read.

For a group conversation, a host may select personal and shared sources:

```json
[
  {"space_id":"<person>","label":"personal","weight":70,"memory":true,"semantic_graph":true},
  {"space_id":"<conversation-space>","label":"conversation","weight":30,"memory":true,"semantic_graph":true}
]
```

The personal Space supports continuity for one person across conversations. The
conversation Space prevents group A's messages and shared memory from becoming
group B's history. They are orthogonal ownership boundaries.

## New session, deletion, memory clearing, and character switching

The following operations are deliberately distinct:

- **new session** detaches the host's session mapping without deleting data;
- `delete_conversation` deletes one conversation and its messages;
- `clear_memory` clears DMW, NSG, or both in one target Space without deleting chat history;
- `switch_character` updates the character referenced by an existing conversation.

Deleting a conversation preserves extracted DMW/NSG. Current deletion keeps
local tombstones/deleted snapshots and does not remove that conversation's
text from the independent pending maintenance queue; queued turns may still
produce memories later. Deleting chat, stopping further extraction, and
revoking derived memories are different operations, not one current guarantee.
See the [maintenance behavior guide](memory_maintenance_current_behavior.zh-CN.md).

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

Hosts map user actions to the following responsibilities. Core does not define
client commands or interaction labels.

| Intent | MOMO integration |
| --- | --- |
| Start a new session | Host selects/creates a conversation and replaces its session mapping |
| Delete conversation data | `delete_conversation` with explicit conversation Space and conversation IDs |
| Clear personal or shared memory | `clear_memory` with an explicit target Space UUID and selected modules |
| Switch character | `switch_character` with explicit conversation and character IDs |

Memory clearing selects DMW, semantic graph, or both. Natural-language text
cannot acquire deletion authority.

## Weights and the write target

The host chooses each `momo.responses/1.0` request's `memory_sources` and
`memory_write_space_id`. Each source carries an explicit Space UUID, weight
and module flags. Weights from 1 through 100 are relative shares of a bounded
retrieval context, not similarity thresholds or authorization percentages.
Omitting a conversation source stops reading it for that request without
deleting its data. The write target is an explicit Space UUID, not a client
configuration enum; Core validates it against the requested sources.

Core does not prescribe how a host stores or presents these choices. Runtime
maintenance policy uses the separate typed settings resource, described in
[`runtime_config_0_1.md`](runtime_config_0_1.md).

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

The assistant, vision fallback, DMW, NSG, and foreground role-play prompts have
reviewed Markdown defaults under `crates/momo-core/src/product_prompts/`,
compiled into `momo_core` with `include_str!`. MOMO does not read `momo.toml`;
the prompts are not part of MOC or any Space. Runtime-owned replacements use the typed `PromptSpaces` Rust API or the
Prompt Spaces HTTP API; normal runtime overrides persist in its data directory. See [the Prompt Spaces guide](maintenance_prompts.en.md).

## 2026-09-30 provenance profile update

The working tree implements [memory provenance Profile 1](memory_provenance_runtime.md). This profile supersedes earlier planned-extension notes for identity, record eligibility, scoped scenes, source controls and the 12/2 context window. MOC preserves origin evidence while dropping imported local grants; source Space IDs are not rewritten by `space_map`.
