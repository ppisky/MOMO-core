# MOMO Space model and control contract 1.0

**Status:** normative pre-release contract  
**Compatibility:** replaces the unpublished Scope-shaped 1.0 request contract  
**Updated:** 2026-08-30

## 1. Instance root is not a Space

The directory passed to `MomoCore::initialize` is a host instance root. It may
contain data belonging to many people and shared conversations. It is not one
person's MOMO Space and MUST NOT be exported as though every item below it had
one owner.

A MOMO Space is a logical, persistent ownership boundary identified by an
opaque UUID. The host creates or deterministically derives Space IDs after it
authenticates a platform principal or shared context. Core enforces the Space
IDs supplied by that trusted host; Core does not infer Discord users, groups,
channels, projects, or permissions from raw platform identifiers.

Typical host mappings are:

- one personal Space per person;
- one shared Space per group, channel, project, or deliberately shared session;
- additional Spaces only when a product has a real ownership boundary for
  them.

There is no character-catalogue Space in the response contract. A character is
a resource, not an access namespace.

## 2. Space and access scope are different concepts

`space_id` answers where persisted data belongs. An access scope answers which
Spaces one operation may read or write. Access scope is request-time authority,
not another persisted UUID and not a directory name.

Core APIs use explicit role-qualified Space fields rather than a generic
`scope_id`:

| Field | Meaning |
| --- | --- |
| `personal_space_id` | Private Space used for the speaker's DMW, NSG, MO State, maintenance, replay and cancellation |
| `conversation_space_id` | Space that owns the selected conversation and its messages; personal for a private session or shared for a group session |
| `memory_sources[].space_id` | Host-authorized Spaces that may contribute read-only DMW/NSG context |
| `memory_write_space_id` | The one explicit Space eligible for background DMW/NSG maintenance writes |

In a private single-person session, personal and conversation Space IDs may be
equal. In a group conversation they normally differ: participants share the
conversation Space while every speaker continues to use a different personal
Space.

## 3. Character ownership and selection

Every character card has a globally unique `character_id`. Character
management metadata MAY record an `owner_space_id` and host-defined sharing
policy, but response generation selects a card by `character_id` only. The
unpublished `character_scope_id` request field is removed.

Core's local transport trusts the host to authorize character selection. A
remote deployment still requires an authenticated host or proxy. Character
ownership metadata is used for management, export and policy; it is not a
second identity for one character.

A conversation stores its current optional `character_id`. A host changes it
through an explicit control operation. Supplying a different card accidentally
on an ordinary response MUST NOT silently mutate the conversation.

## 4. Weighted memory reads and one write target

The response access scope contains zero or more memory sources:

```json
{
  "memory_sources": [
    {
      "space_id": "<personal-space>",
      "label": "personal",
      "weight": 100,
      "memory": true,
      "semantic_graph": true
    },
    {
      "space_id": "<group-space>",
      "label": "group",
      "weight": 40,
      "memory": true,
      "semantic_graph": true
    }
  ],
  "memory_write_space_id": "<personal-space>"
}
```

Weights are integers from 1 through 100. They affect deterministic ranking and
budget allocation; they do not grant access. Duplicate Space IDs, unknown
fields, zero weights, and weights above 100 are rejected. The host MUST
authorize every listed Space before calling Core.

Retrieval may read several Spaces, but maintenance writes to at most one
explicit `memory_write_space_id`. The write target MUST occur in
`memory_sources`. Omitting it disables automatic memory and semantic-graph
writes for that response. A group Space is never written merely because it was
read.

## 5. Conversation and memory controls

Control operations use a separate structured `momo.control/1.0` contract. They
are not natural-language model instructions and are never executed from model
output.

The initial control actions are:

- `detach_session`: host-only operation that removes a session-to-conversation
  mapping. It starts a new conversation next time but preserves the old
  conversation in Core.
- `delete_conversation`: permanently targets one `conversation_id` inside one
  `conversation_space_id`. It never clears memory.
- `clear_memory`: clears explicitly selected `memory` and/or
  `semantic_graph` modules in one target Space. It also clears derived vector
  state and pending maintenance for those modules. It never deletes a
  conversation.
- `switch_character`: changes the stored character of one conversation after
  validating both resource IDs. It preserves history unless the host separately
  detaches or deletes the conversation.

Every destructive Core control carries a unique request ID, an actor Space, an
explicit target Space, and the exact target resource or modules. Core validates
UUID syntax, ownership, action bounds and idempotent replay. It fails closed on
an ownership mismatch. End-user authorization remains a host responsibility.

## 6. Group behaviour

For one message in a group chat:

```text
speaker's personal Space
  -> private DMW/NSG/MO State read
  -> optional private maintenance write

group conversation Space
  -> shared conversation and message history
  -> optional group DMW/NSG read with an explicit weight
  -> no group-memory write unless explicitly selected and authorized

character_id
  -> current character resource
```

This model allows shared short-term context without merging participants'
private long-term memory. A future host module may add explicit group-memory
write policy without changing Core's personal-memory boundary.

## 7. Storage vocabulary

New public models, JSON fields, logs and documentation use `space_id` with a
role-qualified prefix. Internal query code may use the word *scope* only for a
temporary access set, never as the persisted owner field.

The host instance root SHOULD organize file-backed state below
`spaces/<space_id>/`. SQLite rows use `space_id` or `owner_space_id` according
to whether the row is itself Space-owned or is a globally addressed resource
with an owner. Historical `memory/scopes/<uuid>` data is converted to the new
Space layout once; it is not retained as a compatibility spelling.

## 8. MOC boundary

MOC exports explicit Space-owned modules from the host instance root. It MUST
NOT apply one UUID simultaneously to characters, conversations, memory and NSG.
Each selected module records its source Space independently, and import either
preserves that Space ID or applies an explicit source-to-target Space map.

Configuration and host extension modules are selected independently from Space
data. A whole-instance backup is an explicit multi-Space selection; it is never
the default for a personal export.
