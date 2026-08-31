# Memory Space Runtime 1.0

This document defines the implemented memory boundary for MOMO Core 1.0. The
normative identity model is [space_model_1_0.md](space_model_1_0.md).

## Storage

One Core instance root may contain any number of Spaces. DMW and NSG source
files for one Space live below:

```text
spaces/<space-id>/memory/
```

SQLite and vector-store columns may retain the internal name `scope_id`; that
is a storage implementation detail, not a public wire field. Public APIs use
`space_id` or a role-qualified field such as `personal_space_id`.

## Retrieval

A response request supplies zero or more independent sources:

```json
{
  "memory_sources": [
    {
      "space_id": "<personal-space>",
      "label": "personal",
      "weight": 70,
      "memory": true,
      "semantic_graph": true
    },
    {
      "space_id": "<group-space>",
      "label": "conversation",
      "weight": 30,
      "memory": true,
      "semantic_graph": true
    }
  ]
}
```

Weights are relative token-budget weights from 1 through 100. They do not
grant access. The host authorizes each source before calling Core. Retrieval
results carry the source Space as provenance.

## Writes

Retrieval may read several Spaces, but one maintenance run writes to exactly
one `memory_write_space_id`. The write Space must also occur in
`memory_sources`. Core never copies a fact into every readable Space.

DMW and NSG remain independently selectable. Structured `clear_memory` may
clear either component or both in one explicit target Space. Natural-language
instructions cannot trigger deletion.

## Conversation and character boundaries

A conversation belongs to one `conversation_space_id`; all of its messages
are checked through that Space. Character cards are globally addressed by
`character_id`. A character may have an `owner_space_id` for management and
MOC export, but there is no character-directory UUID and runtime lookup never
requires `character_scope_id`.

## Host policy

Core stores and validates Spaces. It does not decide that a UUID means a
Discord user, channel, guild, tenant, or company. mobot currently derives a
personal Space per platform user and a conversation Space per session. Group
sessions may retrieve both with independent weights.
