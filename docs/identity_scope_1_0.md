# Identity and scope contract for 1.0

**Status:** normative 1.0 contract; implementation verified
**Updated:** 2026-08-30

## 1. What a scope is

A `scope_id` is an opaque UUID naming one isolated Core workspace. It is not a
hard-coded product tenant, a server-instance setting, or the platform's raw user
ID. The host maps an authenticated platform principal or shared context to a
stable UUID before calling Core. Random UUID generation and deterministic UUID
derivation are both implementation mechanisms; neither makes one UUID suitable
for every user.

Core does not authenticate Discord users or know that a source ID represents a
user, guild, channel, project, or account. Authentication, authorization, and
the mapping from those source identities to UUIDs remain host responsibilities.
Core is still responsible for enforcing the UUID boundaries supplied by that
trusted host.

There is no process-wide or compiled default scope in 1.0. In particular,
`MOMO_SCOPE_ID` and a `DEFAULT_SCOPE_ID` constant are not part of the 1.0 server
contract. Starting a server must not select the workspace used by later
requests.

## 2. Three independent namespaces in a response operation

A native response operation can touch three kinds of state. They must not be
silently treated as the same namespace:

| Field | Owns | Typical mobot mapping |
| --- | --- | --- |
| `momo.scope_id` | Personal DMW, NSG, MO State and background maintenance | Stable UUID derived from the platform user identity |
| `momo.conversation_scope_id` | Conversation and all messages in it | Stable UUID derived from a Discord channel session or a per-user session |
| `momo.character_scope_id` | Character-card catalogue used to resolve `character_id` | Explicit host-managed catalogue UUID |

The three UUIDs may be equal for a single-user installation, but equality is a
host decision, not a Core fallback. All three fields are explicit in a 1.0
response request. A `character_id` must resolve inside `character_scope_id`; a
`conversation_id` must resolve inside `conversation_scope_id`.

### Character catalogue versus character identity

`character_scope_id` is not a second identity for one character. It names the
catalogue and ownership boundary that may contain multiple character cards.
`character_id` names one concrete card inside that catalogue. Characters in the
same catalogue share one `character_scope_id` but have different
`character_id` values.

The current SQLite schema gives every character a globally unique primary key,
so `character_id` is sufficient to locate a row mechanically. It is not
sufficient to prove that the host intended to authorize that row. Core queries
the pair `(character_scope_id, character_id)` and fails when the character does
not belong to the supplied catalogue. This is the same distinction as a tenant
or repository ID plus a resource ID: the first establishes the allowed
namespace, and the second selects the resource within it.

A host with one fixed catalogue should persist its catalogue UUID once rather
than generate a new one per character or request. mobot 0.1 exposes both values
in deployment configuration because it selects one fixed character; this is a
host configuration detail, not a requirement for an end user to type two UUIDs
for every message.

This separation is required for mobot's documented Discord behaviour. A channel
may intentionally share short-term conversation history while each speaker
continues to retrieve and update only their personal long-term memory. Using
the personal UUID as the conversation owner, or accepting a conversation ID
without checking its owner, breaks that boundary.

## 3. Resource invariants

The following rules apply to every native server route and Rust orchestration
entry point:

1. Every stateful request carries the relevant UUID explicitly. Read, update,
   archive, restore, delete, export, review, maintenance and vector-status routes
   are not exceptions.
2. A conversation belongs to exactly one `conversation_scope_id` for its entire
   lifetime. Its messages inherit that ownership and cannot be read or mutated
   through an unscoped conversation lookup.
3. A character belongs to exactly one `character_scope_id`. Referencing a
   character from a response or a new conversation requires a lookup in that
   catalogue scope.
4. Request replay, in-flight locking and cancellation use the pair
   `(scope_id, request_id)`. A caller in one personal scope cannot collide with,
   replay, or cancel an operation in another scope merely by choosing the same
   request ID.
5. UUID syntax validation is necessary but not sufficient. Core must also
   verify that each referenced resource belongs to the UUID supplied for that
   resource class before returning data or writing state.
6. Multi-scope retrieval never acts as authorization. The host must authorize
   every requested scope first; Core then keeps retrieval and provenance
   isolated per supplied UUID.

Operations that cannot prove these invariants fail closed. They do not retry
against another scope, fall back to the first character, or use server startup
configuration to make the request succeed.

## 4. UUID derivation

Hosts may persist generated UUIDs in an identity table or derive stable UUIDs
with UUID v5. Deterministic derivation must use an application-controlled
namespace and a type-qualified source key, for example
`discord-user:<user-id>` or `discord-channel:<guild-id>:<channel-id>`. Different
resource classes should use different qualified keys so that a user UUID and a
channel UUID cannot alias accidentally.

Raw platform IDs and derivation keys are host data. They are not accepted as a
compatibility spelling of `scope_id` and are not persisted by Core merely to
reconstruct a UUID later.

## 5. Local-server trust boundary

`momo-server` is loopback-only by default and has no end-user authorization
layer. Supplying a syntactically valid UUID therefore does not grant access by
itself: a remotely reachable deployment must be placed behind a trusted proxy
that authenticates the caller and restricts which scope fields it may submit.
`MOMO_SERVER_ALLOW_REMOTE=1` only changes the bind safety check; it does not add
authentication or tenant authorization.

## 6. Upgrade rule

An installation that used the historical fixed UUID may keep that UUID only as
the explicit owner of the existing data it actually contains, such as a
character catalogue selected by the host. It must not continue to use that UUID
as the personal scope of every user. Existing data must be inspected and then
migrated or assigned to an explicit namespace according to its real ownership.

The local `v1.0.0` candidate tag records this contract after cross-scope
negative tests proved that conversations, messages, operations, cancellation,
characters, memory, NSG and portable operations fail closed. The tag has not
been pushed and no GitHub release has been published; external publication
remains subject to the release roadmap, including credentialed provider smoke
tests.

A host-side session cache is not proof of ownership unless it stores the
conversation scope alongside the conversation ID. The 1.0 host does not decode
the former ID-only session schema. With no supported installed base, operators
discard that file rather than guessing ownership or maintaining a compatibility
state machine.
