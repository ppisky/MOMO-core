# Historical MOMO Container v2 draft

> **Superseded before publication:** v2 applied one `scope_id` to unrelated
> character, conversation, memory and semantic-graph modules. The Space-aware
> format is defined by [`MOMO_Container_v3.md`](MOMO_Container_v3.md). Core 1.0
> does not read or emit v2.

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

The `config` module contains `config/momo.toml` and every maintenance-prompt
Markdown file referenced by that document. Prompt references remain relative
to `momo.toml`, MUST resolve within `config/`, and MUST be present in the file
index. Importers MUST reject a config module whose prompt reference is missing,
unsafe, escaping, empty, non-UTF-8, or oversized; they MUST NOT substitute an
inline fallback prompt.

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

## 4. MOMO Character Card v2 payload

Each character directory uses the independent MOMO Character Card v2 format.
It is not the external `chara_card_v2` JSON/PNG format:

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

External CCv1/v2/v3 source fields are not part of the MOMO character payload.
When explicitly requested, compatibility data uses the separate
`tavern_compat` module. `preserved_source` stores `metadata.json` and the exact
original `source.json`, `source.png`, or `source.charx` bytes. A generated
profile instead stores one named generated artifact and is never imported as
source provenance. Compatibility remains external data, not a MOMO Character
Card v2 payload.

## 5. Snapshot-only package model

MOC v2 contains one complete snapshot of the explicitly selected modules.
There are no incremental, deletion-only, sequence-range, or migration package
variants in the 0.5 contract. Unsupported manifest fields are rejected rather
than accepted as an unimplemented promise.

## 6. Extensibility

- Unknown module IDs with safe paths are validated and reported but are not
  written into known business data or executed by Core.
- A host MAY explicitly claim validated unknown modules into a host-selected
  directory after extraction and decryption succeeds. The claim report MUST
  include the module ID, declared root, dependencies, import order, and claimed
  path. Without an explicit claim directory, temporary payloads are discarded.
- A host MAY explicitly provide an extension-module directory during export.
  The container builder validates the module ID, dependencies, declared order,
  regular-file boundary, path safety, size, and digest, but does not interpret
  the payload.
- Claim and export are host hand-off mechanisms, not a plugin execution model.
  WASM, gRPC, or another host runtime remains responsible for recognizing and
  executing the module contract.
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

Private containers keep the same `.moc` outer container. The outer v2 package contains
only its manifest and `private/payload.enc`; the plaintext is a complete normal
MOC. The encryption envelope and its 512 MiB limit are defined by the encryption
profile.

## 8. Import reporting

An import report exposes the source format version plus independent counts for
characters, conversations, messages, DMW files, NSG files, skipped conflicts,
and unknown modules, including their claim status and claimed paths. A skipped or
unimplemented action MUST be reported as such; it must not be presented as a
successful import.
