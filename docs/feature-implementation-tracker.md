# Feature implementation tracker

This tracker is the development gate for the requested operations work and the agreed experimental
ideas. A feature is not “done” until its implementation, focused tests, antagonist review,
documentation, and the repository's mandatory full suites all pass.

Last reconciled against the tree: **2026-09-10**, at `0.3.0-alpha.18`.

| Feature | Phase | Status | Required security/antagonist focus | Test and documentation gate |
|---|---:|---|---|---|
| Moderation plane | 12a | Implemented + verified 2026-08-20; R7 remains disclosed | Signed immutable fields, role residual honesty, linked-device vote dedup, current-member eligibility, owner-only removal | Owner/admin-only plane, per-user lane graph, member chat vote cards; Rust schema/auth/convergence/alias tests; frontend graph/filter/selection/case tests; full gates passed |
| Durable history UX | 12b | Implemented + verified 2026-08-20 | Vault-sealed drafts/read marks, bounded state, safe legacy migration, lock/load race | Store seal/tamper/repeated-save/size tests; frontend sanitizer/migration tests; user guide; full gates passed |
| Storage health and repair | 12c | Implemented + verified 2026-08-20 | Verify rather than trust file existence; one scan/server/process; label estimates honestly; member-signed CID-bound repair; never unsafe GC | Storage corruption/overwrite tests; app authenticated repair test; bridge dedup/category/pin inventory test; Transfers + sidebar docs; full gates passed |
| Connectivity assistant | 12c | Implemented + verified 2026-08-20 | No false reachability claims; privacy-safe reports; clear joiner/operator split | Existing pure three-state/report tests; zeroconf design and user guide; full gates passed |
| Backup and recovery centre | 12d | Export + vault-secret rotation implemented/verified; restore deferred | Encrypted export, offline-guessing/metadata/history disclosure, symlink refusal, consistent persisted cut, old-backup non-revocation, atomic DEK rewrap | Copy tests; wrong-current/no-replacement and same-DEK rotation tests; full gates passed; post-copy manifest + locked import/rollback remain gated |
| Notification controls | 12e | Parallel work; review later | Permission state, content leakage, per-server precedence | Review the other task's implementation and full suites before merge |
| Voice completion | 12f | Test/review later | Media E2E, signaling auth, TURN privacy, device failure recovery | User test first; focused WebRTC/signaling review afterward |
| Channel governance | 12g | Deferred | Permission semantics, history compatibility, owner/admin enforcement | Design + antagonist review before a wire/schema change |
| Desktop performance + IPC hardening | 12h | Active; frames gated, bounded chat DOM/render cache/event coalescing, Feedback/QR lazy split, native lock gate, native paged history (`get_message_page`), worker-thread search (`search-worker.ts`) and the remote-media consent gate (`remote-media.ts`) implemented. The IPC ledger now covers **168** registered commands (`apps/desktop/src-tauri/src/lib.rs` `generate_handler!`), not the 100 this row used to claim; `tauri-command-security.test.ts` holds a deliberately low floor of 150 so the extractor cannot silently under-read the list again. **Remaining: view extraction** (`App.svelte` is 29,850 lines) and the recorded production-bundle/timing measurements | Stable history anchors, no plaintext cache persistence, stale async responses, locked-webview IPC denial, OS URL/file bounds, remote-media fetch consent, honest R2/R6/R7 claims | [`PERFORMANCE-SECURITY-HARDENING.md`](PERFORMANCE-SECURITY-HARDENING.md), [`design-native-paging.md`](design-native-paging.md); focused frontend/Rust tests; production bundle measurements; full gates before handoff |
| P1: epoch close, owner checkpoints and bounded recovery | 16 | **Active, and one of the two largest current workstreams.** Landed: the replication core (`crates/catcoms-replication/src/registry_epoch/`), typed registry and Studio checkpoint materializers, receipt-bound settlement, durable recovery-first installation (`crates/catcoms-app/src/store/epoch_registry/`), authenticated paged registry catch-up, and authenticated checkpoint discovery (`crates/catcoms-app/src/studio_exchange/discovery.rs`, `crates/catcoms-sync/src/checkpoint_exchange.rs`). **Active slice: owner rotation and running-app recovery.** Not user-visible: `settlement-changed` is emitted natively and no frontend file listens for it, so no feature depends on this yet. Automatic wakeups, global replay scheduling, the specialized settlement states, the Registry/repair producers, other managed-file families and production orchestration are not wired | Receipt forgery and replay, owner-tenure equivocation, adoption by a newcomer reading old-owner heads, crash between seal and recovery snapshot, recovery-slot exhaustion, unbounded checkpoint size, page-exchange authentication and capacity abuse, pruning before a durable receipt | [`design-epoch-close.md`](design-epoch-close.md) (revision 5) is authoritative; [`INTERFACES.md`](INTERFACES.md) records the implemented contracts. Focused crash/restart, rotation, paging and replay regressions pass; full-gate acceptance and final suites remain pending |
| Creative suite / Flipnote Studio | 17 | **Active, the other largest current workstream.** Backend **gates 1-3 implemented** for the Index/art milestone: static `IndexOp`/`FlipnoteOp` codec and causal validation, deterministic projections, durable one-device Save/Reopen with real PIX CIDs and accounted vault ownership, two-member collaboration, same-epoch catch-up and recovery-first Studio adoption, all through real native commands. **Gate 4 (rotation and recovery in the running app) is active**: settlement, watched rotation, Registry pointer/tail maintenance and the recovery List/Read/Export/Ack/Restore surface are connected; running-app succession, signed fault repair and full-gate acceptance remain. Gates 5-7 (collaborative frame claims, sound/export and `.pixa`, backend acceptance + UI handoff) untouched. **The Studio frontend is still an in-memory fixture**: `apps/desktop/src/studio-store.ts` holds the only copy of the projection and nothing under `apps/desktop/src` invokes a `studio_*` command, so none of the backend is reachable by a user. Games and game-only avatar-consent work are paused | Cross-document and malformed domain operations, both concurrent delivery orders, frame/byte/sfx/patch cap boundaries, blob reclamation of referenced pixels, checkpoint preflight size, unopened-key service authority, claim replay/collision and claims never becoming edit locks, export byte-exactness | [`BACKEND-IMPLEMENTATION.md`](BACKEND-IMPLEMENTATION.md) is the gate board and carries per-slice commit evidence; [`design-creative-suite.md`](design-creative-suite.md) is the product contract; [`FLIPNOTE-UI-HOOKS.md`](FLIPNOTE-UI-HOOKS.md) maps every canonical UI dependency to a real command/event or marks it unavailable. Gate 7 requires backend acceptance tests, mandatory suites and resolution of adversarial blocker/high findings |

## Experimental ideas (after phase 12a-12d)

None of these have started. As of 2026-09-10 they also sit behind phases 16 and 17 above, which
are the active work; "after 12a-12d" is the order they were queued in, not a claim that they are
next.

| Idea | Status | First design question | Mandatory antagonist review |
|---|---|---|---|
| Campfire rooms | Queued | What makes a room ephemeral, and who can preserve it? | Expiry races, transcript leakage, late joiners, clock skew |
| Promote conversation to wiki | Queued | Is promotion a snapshot, backlink, or live transclusion? | Authorship attribution, edits after promotion, malicious rich content |
| Memory Keepers | Queued | How are preservation duties elected and revoked? | Availability coercion, storage exhaustion, last-copy lies, member removal |
| Ciphertext mailboxes | Queued | Which offline relay learns what metadata and for how long? | Spam/amplification, replay, enumeration, forward secrecy, deletion claims |
| Guardian recovery | Queued | What threshold recovers access without enabling social takeover? | Colluding guardians, coercion, stale shares, guardian churn, lockout |
| Two-way proximity invites | Queued | What human ceremony binds both nearby devices? | Relay/MITM, ultrasonic/QR replay, location inference, shoulder surfing |
| Community time capsules | Queued | Who commits content/key material and who opens it? | Early disclosure, clock authority, member churn, moderation/legal removal |
| Trust constellations | Queued | Which trust statements are local, shared, or derived? | Sybil graphs, deanonymization, coercive scoring, stale/revoked trust |

## Per-feature development loop

1. Write the compatibility and threat contract.
2. Add tests that demonstrate the unsafe/incorrect state before or alongside the implementation.
3. Implement the smallest end-to-end slice with explicit bounds and deterministic time/randomness.
4. Perform the antagonist checklist and record any residual in `docs/THREAT-MODEL.md`.
5. Update `README.md`, `docs/USER_GUIDE.md`, `docs/INTERFACES.md`, and `docs/HANDOVER.md` as applicable.
6. Run focused checks, then every suite required by `AGENTS.md`.
