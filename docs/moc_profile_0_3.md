# MOC Runtime Profile 0.3

This profile records the Core 1.0 implementation of
[MOMO Container v3](../MOMO_Container_v3.md).

- `format_version` is exactly `3`; v1 and v2 are rejected.
- `config` is instance-level and optional.
- `characters`, `conversations`, `memory`, and `semantic_graph` are selected
  independently for explicit Space UUIDs.
- payloads use `<module>/spaces/<space-id>/...` and require matching
  `[[space_modules]]` declarations.
- character selection may include all resources owned by a Space or an
  explicit list of globally unique character IDs.
- import preserves source Space IDs unless `space_map` explicitly converts
  them.
- a mapping may not collapse two source Spaces into one target.
- applying portable configuration is separately controlled by `apply_config`.
- unknown host modules are verified and reported. Core copies them only to an
  explicit host claim directory and never executes them.
- all known payloads and claim destinations are preflighted before the first
  business-data write; individual writes are atomic, while a whole import is
  not a cross-filesystem crash transaction.
- passphrase protection wraps a complete v3 inner MOC in the authenticated
  encrypted-container envelope.

Example export plan:

```json
{
  "include_config": true,
  "characters": [
    {"space_id": "<owner-space>", "character_ids": ["<character-id>"]}
  ],
  "conversations": ["<conversation-space>"],
  "memory": ["<personal-space>"],
  "semantic_graph": ["<personal-space>"]
}
```

Example import plan:

```json
{
  "apply_config": false,
  "space_map": {"<source-space>": "<target-space>"},
  "conflict_mode": "replace"
}
```
