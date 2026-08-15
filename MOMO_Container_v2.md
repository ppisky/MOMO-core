# MOMO-STD-0004: MOMO Container Specification v2.0.0

```text
Standard: MOMO-STD-0004                              August 05, 2026
Category: Specification
Status: Implementation Baseline (v2.0.0)
```

## 1. Scope

MOC is a tar archive compressed with Zstandard. It carries explicitly selected
MOMO modules for local backup and manual migration. Compression and hashes
detect damage; they do not provide confidentiality.

Every v2 container MUST contain `manifest.toml` and MUST declare:

```toml
format = "momo-container"
format_version = 2
created_at = 2026-08-05T00:00:00Z
package_type = "snapshot"
```

An importer MUST reject an unknown `format`, version greater than 2, unsafe or
duplicate paths, links, non-file archive entries, undeclared payloads, digest or
size mismatches, and resource-limit violations before committing business data.

## 2. Stable module identifiers and layout

| Module ID | Root | Import order | Ordering dependencies |
|---|---|---:|---|
| `config` | `config/` | 10 | none |
| `characters` | `characters/` | 20 | none |
| `conversations` | `conversations/` | 30 | `characters` |
| `memory` | `memory/` | 40 | none |
| `semantic_graph` | `semantic_graph/` | 50 | none |
| `encrypted-container` | `private/` | 0 | none |

The plural IDs `characters` and `conversations` are canonical. The singular
IDs `character` and `conversation` MUST NOT appear in a v2 manifest.

Dependencies define deterministic import ordering; they do not require the
dependency module to be present. A conversations-only package remains valid,
and missing character references are imported as nullable references.

Each selected module has one `[[module_definitions]]` entry:

```toml
[[module_definitions]]
id = "conversations"
path = "conversations"
dependencies = ["characters"]
import_order = 30
```

Every payload has one `[[modules]]` file-index entry. The historical field name
is retained to keep manifest decoding stable; each entry describes one file,
not a second module declaration:

```toml
[[modules]]
module = "conversations"
path = "conversations/messages.json"
size = 1842
sha256 = "<lowercase SHA-256 hex>"
```

All payload paths MUST be normalized relative paths below their declared module
root. `manifest.toml` is reserved and MUST NOT appear in the payload index.

## 3. DMW and NSG semantic-web partition

`memory` and `semantic_graph` share one runtime workspace but are independent
MOC modules. `semantic_graph` is the stable machine module ID; Chinese product
copy SHOULD refer to NSG as "语义网".

The following workspace prefixes belong to `semantic_graph`:

- `lore/`
- `rules/`
- `archive/lore/`
- `archive/rules/`

Every other workspace file belongs to `memory`. Export and import MUST apply the
same prefix test; a file cannot be present in both modules.

## 4. Character Card v2 payload

Each character directory uses Character Card v2:

```text
characters/<UUID>/
├── character.toml
├── character.md
├── user.md           # optional
└── opening.md        # optional
```

`character.toml` contains `id`, `name`, `version`, `[author] name`, optional
`[author] url`, `character_file`, optional `user_file`, and optional
`opening_file`.
The fields `description`, `language`, `tags`, `[author] uid`, and
`[author] display_name` MUST NOT occur in v2 character metadata.

All referenced assets are distinct, regular UTF-8 Markdown files below the
character directory. Absolute paths, `..`, links, non-Markdown files, and
frontmatter are rejected.

## 5. Package types and deletion records

### 5.1 Snapshot

`package_type = "snapshot"` is a full selected-module snapshot. It has no
sequence bounds and no deletion records. The local portable exporter currently
emits this type.

### 5.2 Incremental

`package_type = "incremental"` carries changed payload files, deletion records,
or both. It MUST set non-negative `base_sequence` and a greater
`through_sequence`. An empty incremental package is invalid.

### 5.3 Deletion

`package_type = "deletion"` contains no payload files and one or more
`[[deletions]]` entries. It uses the same increasing sequence bounds:

```toml
base_sequence = 40
through_sequence = 44

[[deletions]]
module = "conversations"
object_id = "019f0000-0000-7000-8000-000000000001"
revision = 3
change_sequence = 44
deleted_at = 2026-08-05T00:00:00Z
```

The importer applies modules in `import_order`, then deletion records in
`change_sequence` order. Object revisions and local tombstones still govern
conflict handling; a package does not silently override pending local work.

## 6. Extensibility

- Unknown module IDs with safe paths are extracted and reported but are not
  written into known business data.
- Unknown fields in known Character Card metadata are preserved where safe.
- Private or format-round-trip payloads, such as original Tavern JSON,
  unrecognized Tavern `extensions`, extra catalog metadata, and future
  vendor-specific data, SHOULD live in unknown extension modules outside known
  roots. The recommended root shape is `extensions/<namespace>/...`.
- A format version greater than 2 is rejected, not partially imported.
- Non-v2 containers are rejected rather than decoded as v2.
- Singular module IDs are rejected in v2 manifests.

## 7. Resource and credential policy

- Maximum archive entries: 10,000.
- Maximum total unpacked bytes: 2 GiB.
- Maximum private inner container: 512 MiB.
- Per-file SHA-256 is mandatory.
- Symbolic and hard links, devices, directories as archive entries, absolute
  paths, parent traversal, duplicate paths, and unlisted payloads are rejected.
- API keys, login/refresh tokens, passwords, recovery keys, and external-service
  credentials MUST NOT be exported. Encryption is not an exception.

Private containers keep the tar.zstd outer layer. The outer v2 package contains
only its manifest and `private/payload.enc`; the plaintext is a complete normal
MOC. The encryption envelope and its 512 MiB limit are defined by the encryption
profile.

## 8. Import reporting

An import report exposes the source and target format versions plus independent
counts for characters, conversations, messages, DMW files, NSG files, applied
deletions, skipped conflicts, and preserved unknown modules. A skipped or
unimplemented action MUST be reported as such; it must not be presented as a
successful import.
