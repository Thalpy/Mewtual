# Development sweep

This is the working record for the development sweep started on 2026-08-26. It reconciles the
roadmap documents with the code currently in the tree. Update it when an item is inspected,
implemented, verified, deferred, or found to conflict with another direction.

Status meanings:

- **Open**: no implementation has been identified yet.
- **In progress**: currently being changed or verified.
- **Implemented, verification pending**: code exists, but this sweep has not established that its
  required checks pass.
- **Done**: implementation and proportionate verification are complete.
- **Deferred**: intentionally left for a later phase, with the reason recorded.
- **Conflict**: documents prescribe incompatible states or directions. Do not choose a direction
  during the first sweep; continue with another item and raise the conflict afterward.

## First-sweep board

| Priority | Work item | Status | Evidence / next action |
|---:|---|---|---|
| 1 | Two-process CLI smoke test on Linux and Windows | Implemented, verification pending | `scripts/two-client-smoke.sh`, `scripts/two-client-smoke.ps1`, and `.github/workflows/two-client.yml` exist as uncommitted work. Review and run the local Windows path first. |
| 2 | Isolated Linux NAT scenarios | Implemented, verification pending | `scripts/two-client-netns.sh` covers mapped-direct and relay-only paths. Static review locally; execution requires Linux root/network namespaces. |
| 3 | Fast product-level two-client acceptance scenario | In progress | Real-TCP coverage now includes bidirectional chat, deterministic file transfer, presence, and disconnect. Sealed-store restart/catch-up remains covered separately because loopback records are intentionally not published as rediscovery routes. Compile/test verification is pending a Rust toolchain. |
| 4 | Packaged two-desktop-process harness | Open | Add a test-only semantic control surface and separate app-data roots. Initial scenario: found, invite, join, bidirectional text, clean shutdown. |
| 5 | Backup verification/import/rollback | Open | Design and implement locked staging, post-copy verification/manifest, atomic swap, and rollback. Vault-secret rotation is already implemented. |
| 6 | Voice completion | Open | SFrame/Encoded Transform use of the MLS media key, epoch rotation, VAD/DTX, NAT transport work, user test, and adversarial review. |
| 7 | Desktop performance and IPC hardening | In progress | Existing bounded DOM/cache/event and lock-gate work is landed. Remaining sequence is view extraction, native paging, worker search, remote-media consent, and measurements. |
| 8 | Endpoint scheduler permits and socket-start accounting | In progress | Added non-cloneable exact-endpoint permits plus transport-actor `Submitted`/`Suppressed` acknowledgement. Post-join rendezvous/cache dials commit only when the actor accepts the exact endpoint; duplicates and command failures refund on drop. Pre-join migration, relay outer-socket leases, and process-wide in-flight limits remain. |
| 9 | Post-join discovery resilience | Complete and verified | Exact-descriptor asynchronous reciprocal/helper control, manual bounded redial, TTL-aware renewal, two-helper indirect probes, HyParView-like logical views, CYCLON-local sampling, Connectivity evidence/actions, and old-route withdrawal regressions are implemented. The final adversarial re-review found no remaining finding, and the mandatory root, Tauri, and frontend suites plus formatting, lint, deterministic-runtime, static, and production-build checks passed. |
| 10 | Last-copy-safe storage retention | In progress | Integration contract now specifies a vault-sealed local index, signed nonce-bound per-CID holder probes, post-await rechecks, dedup/pin rules, and crash reconciliation. Wire/persistence implementation remains; never substitute connectivity for possession. |
| 11 | Diagnostics M4-M7 remainder | In progress | Native validation now gates Save, Copy, and bounded GitHub issue preparation; the reviewed feedback URL builder/native destination allowlist are reused. Source-typed migration, findings/checks, virtualisation, budgets, and CI gates remain. |
| 12 | Notification controls review | Deferred | Parallel implementation is referenced but not identified as safe to merge; review after the first sweep. |
| 13 | Channel governance | Deferred | Requires a compatibility/threat design and antagonist review before wire/schema changes. |
| 14 | Android and broad phase-14 hardening | Deferred | Large standalone phases; begin after the bounded desktop/recovery work above. |
| 15 | Experimental feature queue | Deferred | Eight ideas remain design-gated in `feature-implementation-tracker.md`. |

## Resolved documentation conflicts

These were checked against implementation and tests after the first sweep:

1. **Vault re-keying:** implemented by `catcoms-storage::change_vault_passphrase`, called through
   `ServerStore`, and covered for successful rotation, wrong-current-secret, and same-secret
   refusal. Corrected the stale handover limitation; secret *recovery/reset* remains absent.
2. **Hardening phase number:** the current roadmap table is authoritative: Android is phase 13 and
   hardening/security review is phase 14. Corrected the stale handover reference.
3. **Voice phase 2:** Encoded-Transform/SFrame media encryption is not implemented. The bridge
   exposes `call_media_key`, but the frontend has no call site or encoded-frame transform. Added an
   explicit status correction to the design.
4. **Diagnostics scoreboard:** P3-018, P3-019 and P3-020 were fixed by later work and have focused
   regression tests. Updated their stale rows; the named M6 export-validator and early-`main.ts`
   capture remainders stay open.

## Activity log

### 2026-08-26

- Audited the Markdown roadmap and identified the primary open workstreams.
- Inspected the dirty worktree and preserved the existing two-client documentation, scripts, and
  workflow as user-owned work.
- Created this tracker and separated CLI-process automation from the still-open packaged-desktop
  harness.
- Began review and local verification of the Windows two-process CLI smoke test.
- Extended `tcp_product_e2e.rs` with Bob-to-Alice messaging and a deterministic file listing,
  authenticated chunk fetch, byte comparison, and CID verification over the real TCP connection.
- Kept restart/rediscovery out of the loopback TCP test: production intentionally strips loopback
  addresses from published records, so claiming public-route rediscovery there would test a false
  configuration. The sealed-store restart and catch-up behavior remains covered in `product_e2e`.
- Reconciled the four initially recorded documentation conflicts against code and regression tests;
  corrected `HANDOVER.md`, `design-voice.md`, and the diagnostics scoreboard.
- Audited storage retention wiring. The safety engine exists and is tested in isolation; live
  integration needs durable blob metadata plus an authenticated, fresh per-CID holder probe. Deferred
  eviction integration rather than weaken the engine's last-copy invariant.
- Hardened the Windows two-process smoke harness so failures record a privacy-safe outcome and last
  phase, and both child process handles are stopped/disposed on every exit path.
- Confirmed the early `main.ts` load-failure bootstrap already exists (`public/boot-failure.js`,
  parser-first script, timed no-script panel, CSP-compatible) and corrected the tracker rather than
  duplicating it.
- Wired the existing independent diagnostics export validator into `save_diagnostics_report`.
  Reports with paths, URLs, credential-shaped values, unsafe raw addresses, or opaque blobs are now
  refused before disk write; non-blocking legacy-prose categories are returned and shown by the UI.
  Rust/desktop verification is intentionally queued for the later verification pass.
- Added `validate_diagnostics_report` and made the report-copy action pass it before touching the
  clipboard, closing the unchecked Copy-vs-checked-Save split. Added the command to the IPC security
  ledger; compile and command-ledger verification remain queued.
- Added focused native regression tests for local-path refusal without echoing the offending bytes,
  Safe-vs-Enhanced raw-address handling, and non-blocking bridged-prose review disclosure.
- Added debug-console “Prepare issue” through the existing reviewed feedback composer and native
  exact-destination allowlist. It validates first, opens only a prefilled browser page, copies the
  full report when URL bounds require truncation, and leaves submission to the user.
- Began endpoint scheduler hardening with exact, single-use `EndpointDialPermit`s. Unused permits
  automatically refund process/server/peer/socket/prefix counters, cannot be cloned or replayed,
  and are consumed by post-join sync immediately before transport submission. Added a queued test
  proving all scopes refund once and a committed permit remains charged.
- Extended `MeshTransport` with actor dial outcomes. The libp2p actor now reports whether an exact
  endpoint entered its pending dial path or was suppressed as duplicate/already connected; sync
  commits a permit only for `Submitted`, so suppressed and failed command deliveries refund without
  a count-based guess. Existing non-actor transports keep a compatible submitted-by-default seam.
- Added `design-retention-integration.md`, defining the missing durable index and authenticated
  holder-probe protocol, stale-decision rechecks, dedup/pin transaction, recovery behavior, bounds,
  and required antagonist tests before automatic GC can be wired safely.

## Verification log

Record exact commands and outcomes here. A CI configuration being present is not proof that it
passes.

| Date | Command / check | Outcome |
|---|---|---|
| 2026-08-26 | Documentation and worktree audit | Completed; conflicts recorded above. |
| 2026-08-26 | `scripts/two-client-smoke.ps1` | Blocked before build: `cargo` is not installed/on `PATH` in this shell, and no prebuilt `target/debug/catcomsctl.exe` is available. |
| 2026-08-26 | PowerShell parser over `scripts/two-client-smoke.ps1` | Passed with no syntax errors. |
| 2026-08-26 | Bash parsing for the Linux harnesses | Not run: the Windows App Execution Alias for `bash.exe` is inaccessible and no usable WSL shell is available. |
| 2026-08-26 | `git diff --check` | Passed. |
| 2026-08-26 | `tcp_product_e2e` extension | Source updated; compile/test pending because `cargo` is unavailable. |
| 2026-08-26 | PowerShell parser after failure-artifact hardening | Passed with no syntax errors. |
