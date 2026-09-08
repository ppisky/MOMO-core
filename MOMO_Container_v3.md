# MOMO-STD-0005: MOMO Container Specification v3.0.0

```text
Standard: MOMO-STD-0005                              August 30, 2026
Category: Specification
Status: Implemented MOMO Core 1.0 contract (v3.0.0)
```

## 1. Purpose

MOC v3 is a Zstandard-compressed tar container for explicit MOMO Space data,
portable configuration and host extension modules. A host instance root may
contain many personal and shared Spaces; a MOC includes only the Spaces and
modules selected by an authorized host.

MOC does not treat the instance root, a character collection and a personal
memory Space as the same object. Whole-instance backup is an explicit
multi-Space selection, never a default.

Every container contains `manifest.toml`:

```toml
format = "momo-container"
format_version = 3
created_at = 2026-08-30T00:00:00Z
```

Importers reject every other format version. There is no v2 compatibility or
implicit migration path.

## 2. Module roots

| Module ID | Root | Space-owned | Import order |
| --- | --- | --- | ---: |
| `config` | `config/` | no | 10 |
| `characters` | `characters/` | yes | 20 |
| `conversations` | `conversations/` | yes | 30 |
| `memory` | `memory/` | yes | 40 |
| `semantic_graph` | `semantic_graph/` | yes | 50 |
| `encrypted-container` | `private/` | no | 0 |

Host extension modules use `extensions/<namespace>/...` and are not interpreted
by Core.

Each selected known or host module has one `[[module_definitions]]` entry. Each
payload file has one `[[modules]]` digest entry, as in v2. Space-owned known
modules additionally require `[[space_modules]]` declarations:

```toml
[[space_modules]]
space_id = "01900000-0000-7000-8000-000000000101"
module = "memory"
path = "memory/spaces/01900000-0000-7000-8000-000000000101"
```

`space_id` is an opaque UUID. `module` must be a selected Space-owned known
module. `path` must equal `<module-root>/spaces/<space_id>`. Duplicate pairs,
undeclared Space payloads and Space declarations for config or extension
modules are rejected.

## 3. Stable layout

```text
manifest.toml
config/
  momo.toml
  prompts/*.md
characters/spaces/<owner-space-id>/
  index.json
  <character-id>/character.toml
  <character-id>/character.md
conversations/spaces/<conversation-space-id>/
  index.json
  messages.json
memory/spaces/<memory-space-id>/
  ... DMW files except NSG prefixes ...
semantic_graph/spaces/<memory-space-id>/
  lore/
  rules/
  archive/lore/
  archive/rules/
extensions/<module-id>/
  ... host-owned payload ...
```

Character IDs are globally unique resource IDs. The containing Space records
ownership for management and export; it is not a second character identity.
Conversations store their current optional character ID and may reference a
character carried by another selected Space or omitted from the MOC. Missing
characters become nullable references on import.

DMW and NSG are independently selectable modules in the same memory Space.
Their prefix partition is unchanged from v2.

## 4. Export selection

The Core export plan contains independent selections:

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

An empty list excludes that module. An empty `character_ids` list selects all
characters owned by the selected Space. Duplicate Space/module selections and
characters not owned by their declared Space are rejected. The host authorizes
every selection before calling Core.

## 5. Import mapping

Import preserves source Space IDs by default. A host may explicitly supply a
one-to-one source-to-target map:

```json
{
  "space_map": {
    "<source-space>": "<target-space>"
  }
}
```

Mapping applies independently to every declared Space module. Every source key
must occur in the manifest, target UUIDs must be explicit, and two source
Spaces may not collapse into one target in the same import. This prevents an
import from silently merging two people's memory or a group conversation into
a personal Space.

Config import is selected independently with `apply_config`. Unknown host
modules are validated and reported; they are copied only when the host supplies
an explicit claim directory.

## 6. Controls and conflicts

MOC import conflict mode is applied per resource inside the mapped target
Space. Space mapping never grants authorization and never changes globally
unique character, conversation or message IDs.

Import is validated completely before known business data is committed. A
container with an undeclared Space path, invalid UUID directory, cross-Space
path, duplicate resource, unsafe reference or inconsistent index is rejected.
The preflight also reads every selected DMW/NSG file, validates referenced
prompt assets, and checks host claim destinations. Individual SQLite and file
writes are atomic. Because one import spans SQLite, prompt files, Space files,
and optional host-owned directories, it is not represented as a single
cross-filesystem crash transaction; an operational I/O failure is reported and
the host must retry or restore its deployment snapshot.

## 7. Config and prompts

The optional `config` module contains `config/momo.toml` and all referenced
maintenance-prompt Markdown files. Paths are relative to `momo.toml`, stay
inside `config/`, and are validated as regular non-empty UTF-8 files no larger
than the configured prompt limit. Credentials and host-local adapter wiring are
never included.

Config is a portable behaviour profile selected for the package; it is not
evidence that every Space in the package has one owner or identical access.

## 8. Security and resource limits

- maximum entries: 10,000;
- maximum unpacked size: 2 GiB;
- maximum encrypted inner MOC: 512 MiB;
- SHA-256 required for every payload file;
- no links, devices, absolute paths, parent traversal, duplicates or undeclared
  files;
- no API keys, passwords, access tokens, refresh tokens or host credentials;
- encrypted MOC uses the existing authenticated envelope around a complete v3
  inner container.

## 9. Extension hand-off

Unknown extension modules are verified but never executed by Core. A host may
explicitly export a module directory or claim an imported unknown module into a
host-selected directory. WASM, gRPC or another host runtime owns recognition,
execution and version negotiation.
