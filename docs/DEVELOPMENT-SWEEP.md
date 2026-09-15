# Development sweep

This is the working record for the development sweep started on 2026-08-26. It reconciles the
roadmap documents with the code currently in the tree. Update it when an item is inspected,
implemented, verified, deferred, or found to conflict with another direction.

Last reconciled against the tree: **2026-09-10**, at `0.3.0-alpha.18`.

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
| 1 | P1: epoch close, owner checkpoints and bounded recovery | In progress | The largest active workstream (`eea698d`..`022bbc9`, 2026-09-07 to 2026-09-09). Landed: the replication core (`crates/catcoms-replication/src/registry_epoch/`), the typed registry and Studio checkpoint materializers, receipt-bound settlement (`.../registry_epoch/settlement.rs`), durable recovery-first installation (`crates/catcoms-app/src/store/epoch_registry/{recovery,installation}.rs`), authenticated paged registry catch-up and authenticated checkpoint discovery (`crates/catcoms-app/src/studio_exchange/discovery.rs`, `crates/catcoms-sync/src/checkpoint_exchange.rs`). Active slice: owner rotation and running-app recovery. **No frontend consumes any of it**: `settlement-changed` is emitted at `apps/desktop/src-tauri/src/lib.rs:2098` and no file under `apps/desktop/src` listens for it. Design and per-section status in `design-epoch-close.md`. |
| 2 | Creative suite / Flipnote Studio backend | In progress | The other large active workstream (`f77b8f9`..`ccddd23`). `BACKEND-IMPLEMENTATION.md` is the authoritative gate board: gates 1-3 are implemented for the Index/art milestone (typed `IndexOp`/`FlipnoteOp` codec and causal validation, deterministic projections, durable one-device Save/Reopen with real PIX CIDs, two-member collaboration, same-epoch catch-up and recovery-first Studio adoption). **Gate 4 (rotation and recovery in the running app) is active**; gates 5-7 (frame claims, sound/export, acceptance) are untouched. The Studio **frontend is still an in-memory fixture**: `apps/desktop/src/studio-store.ts` holds the only copy of the projection and nothing under `apps/desktop/src` invokes any `studio_*` command. Frontend integration map: `FLIPNOTE-UI-HOOKS.md`. |
| 3 | Two-process CLI smoke test on Linux and Windows | Implemented, verification pending | `scripts/two-client-smoke.sh`, `scripts/two-client-smoke.ps1`, and `.github/workflows/two-client.yml` were committed on 2026-08-27 in `7362cda`. The Windows path still needs a recorded local run. |
| 4 | Isolated Linux NAT scenarios | Implemented, verification pending | `scripts/two-client-netns.sh` covers mapped-direct and relay-only paths. Static review locally; execution requires Linux root/network namespaces. |
| 5 | Fast product-level two-client acceptance scenario | In progress | `crates/catcoms-app/tests/tcp_product_e2e.rs` (538 lines) covers steps 1-4 of the `todo.md` scenario over real TCP: found/invite/join, bidirectional chat, deterministic file transfer with CID comparison, presence and disconnect. Steps 5 (restart from the same sealed store, then rediscovery/catch-up without a new invite) and 6 (failure-only labelled diagnostic export) remain open; sealed-store restart stays covered separately in `product_e2e` because loopback records are intentionally not published as rediscovery routes. |
| 6 | Packaged two-desktop-process harness | Open | Add a test-only semantic control surface and separate app-data roots. Initial scenario: found, invite, join, bidirectional text, clean shutdown. |
| 7 | Backup verification/import/rollback | Open | Design and implement locked staging, post-copy verification/manifest, atomic swap, and rollback. Vault-secret rotation is already implemented. |
| 8 | Voice completion | Open | SFrame/Encoded Transform use of the MLS media key, epoch rotation, VAD/DTX, NAT transport work, user test, and adversarial review. |
| 9 | Desktop performance and IPC hardening | In progress | Bounded DOM/cache/event and lock-gate work is landed, and three of the four remaining items have since shipped: native paging (`get_message_page`, `apps/desktop/src-tauri/src/lib.rs:11429`, called from `App.svelte`), worker search (`apps/desktop/src/search-worker.ts`) and remote-media consent (`apps/desktop/src/remote-media.ts`). **Only view extraction remains**: `App.svelte` is still **~30,000 lines** (29,915 measured 2026-09-11 at `38a990f`, and still growing: 29,837 at `6576a46`). Re-measure with `wc -l apps/desktop/src/App.svelte` rather than trusting the figure here. Production-bundle sizes are now recorded (`PERFORMANCE-SECURITY-HARDENING.md:33-42`, measured 2026-09-11 at `6576a46`), and they show the App chunk has grown back past its pre-split size. Still open: the webview **timing** half (the 7 ms / 75 ms pair is a one-off manual observation at 1004 KiB and is not settled) and the standing commitment to re-report chunk sizes after every extraction. |
| 10 | Endpoint scheduler permits and socket-start accounting | In progress | Added non-cloneable exact-endpoint permits plus transport-actor `Submitted`/`Suppressed` acknowledgement. Post-join rendezvous/cache dials commit only when the actor accepts the exact endpoint; duplicates and command failures refund on drop. `DiscoveryPolicy::refund_endpoint_budget` (`crates/catcoms-discovery/src/lib.rs:880`) is still a scalar count, not an opaque reservation receipt. Pre-join migration, relay outer-socket leases, and process-wide in-flight limits remain. |
| 11 | Post-join discovery resilience | Complete and verified | Exact-descriptor asynchronous reciprocal/helper control, manual bounded redial, TTL-aware renewal, two-helper indirect probes, HyParView-like logical views, CYCLON-local sampling, Connectivity evidence/actions, and old-route withdrawal regressions are implemented. The final adversarial re-review found no remaining finding, and the mandatory root, Tauri, and frontend suites plus formatting, lint, deterministic-runtime, static, and production-build checks passed. |
| 12 | Last-copy-safe storage retention | In progress | Integration contract now specifies a vault-sealed local index, signed nonce-bound per-CID holder probes, post-await rechecks, dedup/pin rules, and crash reconciliation. Still unwired: `RetentionIndex` has no consumer outside `catcoms-storage`; the only reference elsewhere is a doc comment in `crates/catcoms-app/src/lib.rs`. Never substitute connectivity for possession. |
| 13 | Diagnostics M4-M7 remainder | In progress | `design-diagnostics-suite.md` is the plan of record: M0-M3 done, M4 has reading and capture control but no findings/checks panels or virtualised list, M6 mostly done (native disclosure findings on Copy/Save, canonical allowlist plus validation for the public issue path), **M5 and M7 not started**. Source-typed migration, budgets and CI gates remain. |
| 14 | Sync/replication frontier and catch-up hardening | In progress | Landed since 2026-09-09: position-paged catch-up so a wide frontier cannot starve a peer (`78e5135`), a 512-head frontier cap (`e0fda1e`), naming the membership chain a member cannot complete (`ed7f7d6`), and micelle-healing regressions across interrupted reconciliation (`c8f2719`, `f61e5dc`). `docs/MESSAGE-FLOW.md` now traces send/gossip/catch-up. Remaining work list is at the end of that document. |
| 15 | Notification controls review | Deferred | Parallel implementation is referenced but not identified as safe to merge; review after the first sweep. |
| 16 | Channel governance | Deferred | Requires a compatibility/threat design and antagonist review before wire/schema changes. |
| 17 | Android and broad phase-14 hardening | Deferred | Large standalone phases; begin after the bounded desktop/recovery work above. |
| 18 | Experimental feature queue | Deferred | Eight ideas remain design-gated in `feature-implementation-tracker.md`. |

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
   regression tests. Updated their stale rows; the early-`main.ts`
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
  Local reports with paths, URLs, credential-shaped values, unsafe raw addresses, opaque blobs, or
  legacy prose are disclosed for review without pretending they are publication-safe. The public
  issue path now uses a separate native canonical allowlist that excludes prose, names, addresses,
  runtime field names and bridge events, then validates the result as defence in depth.
- Added `validate_diagnostics_report` and made the report-copy action pass it before touching the
  clipboard, closing the unchecked Copy-vs-checked-Save split. Added the command to the IPC security
  ledger; compile and command-ledger verification remain queued.
- Added focused native regression tests for local-path findings without echoing the offending bytes,
  Safe-vs-Enhanced raw-address handling, publication refusal for bridged prose, and a public-renderer
  canary covering prose, names, addresses, runtime field names and legacy events.
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

### 2026-09-10

Reconciliation pass. The board had been stale since 2026-08-29 and described none of the two
largest workstreams of the intervening three weeks. What changed, and why:

- Added **P1/epoch close** (row 1) and the **creative suite / Flipnote Studio backend** (row 2) as
  first-priority items. Between them they account for most of `v0.3.0-alpha.15..HEAD`. Both rows
  name what has landed and what has not; in particular neither is user-visible yet, because no
  frontend consumes settlement events and the Studio surface is still an in-memory fixture.
- Added **sync/replication frontier hardening** (row 14) for the catch-up paging, frontier cap and
  micelle-healing work of 2026-09-09/10, with `MESSAGE-FLOW.md` as its trace.
- Row 3 (two-process CLI smoke test): the scripts are no longer uncommitted; they landed in
  `7362cda` on 2026-08-27. The remaining gap is a recorded Windows run, not the code.
- Row 5 (fast two-client acceptance): removed the toolchain blocker and replaced the vague status
  with which numbered steps of the scenario are actually covered. Steps 1-4 are; 5 and 6 are not.
- Row 9 (desktop performance): three of the four named remainders shipped. Corrected the row to say
  that **only view extraction** is left, and recorded the measured `App.svelte` size rather than an
  adjective.
- Rows 10, 12 and 13 gained the exact symbol/file evidence for what is still missing, so a later
  reader does not have to re-derive it.
- Retired the "pending a Rust toolchain" / "cargo is not installed" blocker throughout. `cargo`
  1.89.0 is installed locally and CI runs the full matrix.
- Deliberately **not** changed: backup import/rollback (row 7) and last-copy-safe retention
  (row 12) are still genuinely absent, and post-join discovery (row 11) is still complete. Those
  rows were spot-checked and left alone.

## Verification log

Record exact commands and outcomes here. A CI configuration being present is not proof that it
passes.

| Date | Command / check | Outcome |
|---|---|---|
| 2026-08-26 | Documentation and worktree audit | Completed; conflicts recorded above. |
| 2026-08-26 | `scripts/two-client-smoke.ps1` | Blocked before build: `cargo` was not on `PATH` in that shell, and no prebuilt `target/debug/catcomsctl.exe` was available. **Superseded 2026-09-10:** see below. |
| 2026-08-26 | PowerShell parser over `scripts/two-client-smoke.ps1` | Passed with no syntax errors. |
| 2026-08-26 | Bash parsing for the Linux harnesses | Not run: the Windows App Execution Alias for `bash.exe` is inaccessible and no usable WSL shell is available. |
| 2026-08-26 | `git diff --check` | Passed. |
| 2026-08-26 | `tcp_product_e2e` extension | Source updated; compile/test pending because `cargo` was unavailable in that shell. **Superseded 2026-09-10:** see below. |
| 2026-08-26 | PowerShell parser after failure-artifact hardening | Passed with no syntax errors. |
| 2026-09-10 | `cargo --version` | `cargo 1.89.0 (c24e10642 2025-06-23)`. The "no Rust toolchain" blocker recorded on 2026-08-26 is retired. |
| 2026-09-10 | `.github/workflows/ci.yml` inspection | CI pins `dtolnay/rust-toolchain@1.89.0` and runs `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all --all-features`, `scripts/check-no-ambient.sh` and `cargo-deny` across the matrix. A toolchain is available to CI and locally; only the *desktop* Cargo workspace is still outside the root matrix. |
| 2026-09-10 | Board reconciliation against the tree | Rows rewritten from evidence: native paging, worker search and remote-media consent confirmed present; `App.svelte` recorded at 29,850 lines (that figure matched no revision and was corrected on 2026-09-11 to ~30,000: 29,915 at `38a990f`); `tcp_product_e2e.rs` measured at 538 lines; `RetentionIndex` confirmed to have no consumer outside `catcoms-storage`; `refund_endpoint_budget` confirmed still scalar. Suites were **not** re-run as part of this reconciliation. |
