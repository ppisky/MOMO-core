# MOMO Core 1.0.0-rc.5

This candidate completes the current runtime-ownership and recovery work.
Publication requires the Linux, Windows and security CI gates on the exact commit.
Workspace package versions remain `1.0.0`, as for the previous RC builds.

## Changes

- Remove the internal JSON compatibility API instead of maintaining two Rust
  entry points. HTTP serialization, portable format codecs and durable JSON
  records remain at their actual boundaries. Native callers must update to
  the current typed API; no deprecated forwarding facade is supplied.
- Include cancellation-safe ordinary memory
  writes, UUID lock equivalence, and overflow-safe bounded weighted budgets.
- Share typed UUID Space ownership across DMW/NSG operations, retrieval and MOC
  import/export. Admitted file operations keep their locks and complete even
  when the invoking future is cancelled; shutdown drains their owned tasks.
- Persist exact maintenance/review file plans before file mutation. Recover
  prepared operations before subsequent access. A conflicting Space is isolated
  without preventing the instance, unrelated Spaces or diagnostics from starting.
  Recovery acknowledges every prepared batch, including an empty evidence list;
  it cannot admit a Space while silently leaving a second prepared batch behind.
- Add local management `GET /v1/memory/recovery` and
  `POST /v1/memory/recovery/:space_id/retry`. Retry never overwrites conflicting
  operator data; repair the conflicting file before retrying.
- Runtime settings and persistent Prompt Spaces now belong to `MomoRuntime`.
  Creating another `MomoApiService` does not create or reset policy/prompt state.
  The service retains model-connection wiring and response orchestration.
- Canonicalize native response UUID identities before coordination and replay
  fingerprinting; equivalent UUID spellings no longer create distinct operations.
- Split transport routes and SQLite implementation files while retaining
  cross-table SQL transactions.
- Keep unit-test bodies under `tests/unit`, including runtime ownership,
  prepared file commits and response-stream completion tests.

## Upgrade and limits

SQLite migration `0024_prepared_maintenance_commit.sql` adds prepared-operation
state. Back up the data directory before upgrading; an older binary is not a
supported consumer of pending prepared commits. Never discard the journal to
bypass a conflict. Frozen `contracts/1.0` bytes and portable schemas are unchanged.

Hosts now apply instance policy through `PUT /v1/runtime-settings` and prompts
through `PUT /v1/prompt-spaces/{id}`. Core no longer reads `momo.toml` or
`MOMO_CONFIG_PATH`, and runtime settings and prompts are not MOC content.
Native callers use `MomoRuntime`, `MomoApiService` and `api::runtime_api` from
the same workspace revision. See the [migration guide](migration_0_5_0_to_1_0_0.md).

MOC import/export now shares live Space exclusion and cancellation ownership.
This does **not** make an entire multi-module MOC import crash-atomic. Ordinary
administrative file operations retain their existing rollback boundaries; the
durable plan currently covers maintenance and reviewed patches. A power-loss
guarantee, multi-process writing and authenticated multi-tenancy are not claimed.

## Verification

Incoming working-tree baseline: 299 Rust tests passed on this Windows host.
Final local check: `scripts/test-release.ps1` exited successfully on Windows
with Rust 1.96.1 on September 25, 2026:

- Frozen contract checksums, formatting and strict all-target/all-feature Clippy.
- 304 Rust tests, including five additional regressions; 102 MORP Python tests.
- 21/21 offline runtime contracts, with zero model calls and deterministic replay.
- Rustdoc and all-feature workspace release build with the locked dependency set.
- RustSec audit: 476 dependencies scanned, no reported vulnerabilities.
- Removed-identity binary scan and actual loopback `/health` plus empty
  `/v1/memory/recovery` smoke check in an isolated temporary data directory.

Local evidence directories: `target/morp-offline-92986b436d864329bdead4774bdac0d5`
and `target/release-smoke-f8e972b1b50f4dde82f9075615306388`. These are local build
artifacts, not published release assets. The tested `target/release/momo-server.exe`
SHA-256 is `bcf16610734a08a1e71053ba3b48c0aedafb8b796c0e04fbbcae505cb089c255`.

No new provider/model quality result is claimed by this candidate.

Reproduce locally with `./scripts/test-release.ps1` on Windows or
`bash scripts/test-release.sh` on Linux/macOS. An explicitly skipped RustSec
audit is an incomplete release gate, not an audit pass.
