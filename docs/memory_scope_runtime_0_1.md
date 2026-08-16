# Memory Scope Runtime 0.1

## Boundary

MOMO Core treats each memory scope as an opaque UUID-backed workspace. It does
not know whether a scope represents a Discord user, channel, project, or other
host concept. Host applications are responsible for identity mapping and access
control before calling Core.

The public memory contract uses only `scope_id`. Memory retrieval, MO State
compilation, and DMW/NSG patch requests require it; there is no implicit default
scope and no `owner_id` compatibility field.

The same identifier is persisted as `scope_id` in SQLite, vector records, and
patch reviews. Filesystem workspaces live at `memory/scopes/<scope_id>`. The
0.3.1 migration converts the earlier database columns and directory name; it
does not keep a second public spelling.

## Multi-scope retrieval

`POST /v1/memory/retrieve-scoped` accepts one to eight sources:

```json
{
  "scopes": [
    {"scope_id": "<uuid>", "label": "personal", "weight": 3},
    {"scope_id": "<uuid>", "label": "channel", "weight": 2}
  ],
  "query": "current input",
  "max_tokens": 1024,
  "include_memory": true,
  "include_semantic_graph": true
}
```

Core divides the total budget by weight, retrieves each workspace in isolation,
and adds this provenance to every returned DMW or NSG item:

```json
{"memory_scope": {"id": "<uuid>", "label": "personal"}}
```

DMW and NSG retrieval are independently selectable. Vector arguments keep the
all-or-none rule and are valid only when semantic-graph retrieval is enabled.
The same query vector is used against each scope's isolated vector cache.
The JSON facade consumes this request as one document rather than seven
positional arguments.

## Writes

Memory and NSG patch endpoints require `scope_id`. Scoped NSG pending-list,
approve, and reject requests require the same field. Unknown compatibility
fields are rejected. Core validates and applies patches inside only that scope.

The pending control plane is not a runtime gate. Automatic Draft governance can
continue without user interaction; pending candidates preserve author authority
only when an automatic change crosses the Canon boundary.

Scope classification is intentionally not a Core responsibility. A host may use
platform metadata, user policy, or a model router to decide which scopes receive
a candidate, but it must never send an unauthorized scope to Core and expect
retrieval ranking to act as access control.
