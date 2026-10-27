# Feature implementation tracker

This tracker is the development gate for the requested operations work and the agreed experimental
ideas. A feature is not “done” until its implementation, focused tests, antagonist review,
documentation, and the repository's mandatory full suites all pass.

Last reconciled against the tree: **2026-09-10**, at `0.3.0-alpha.18`.

Creative Suite current status (2026-10-17): Gate 6 is owner-accepted and merged at `82631a81`;
Gate 7's composed backend/TCP/native acceptance and current UI contract are tracked in
[GATE7-ACCEPTANCE](GATE7-ACCEPTANCE.md) and [GATE7-UI-HANDOFF](GATE7-UI-HANDOFF.md).
The older row below retains its dated checkpoint, rather than asserting pending Gate 6 acceptance
as the current owner status.

Creative Suite row previously reconciled on **2026-10-14** at the Gate 6 candidate
`818e711641bc8193af5cde3be8e56d4817c475f9`; unrelated feature rows retain their earlier audits.

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
| Desktop performance + IPC hardening | 12h | Active; frames gated, bounded chat DOM/render cache/event coalescing, Feedback/QR lazy split, native lock gate, native paged history (`get_message_page`), worker-thread search (`search-worker.ts`) and the remote-media consent gate (`remote-media.ts`) implemented. The IPC ledger now covers **168** registered commands (`apps/desktop/src-tauri/src/lib.rs` `generate_handler!`), not the 100 this row used to claim; `tauri-command-security.test.ts` holds a deliberately low floor of 150 so the extractor cannot silently under-read the list again. **Remaining: view extraction** (`App.svelte` is **~30,000 lines**: 29,915 measured 2026-09-11 at `38a990f`, up from 29,837 at `6576a46`, so re-measure with `wc -l` rather than trusting this figure), the webview timing measurements, and the standing commitment to re-report chunk sizes after every extraction. Production-bundle **sizes** are now recorded (`PERFORMANCE-SECURITY-HARDENING.md:33-42`, 2026-09-11 at `6576a46`): they show the App chunk grew back to 1,146.34 kB, past the 881.49 kB it started at | Stable history anchors, no plaintext cache persistence, stale async responses, locked-webview IPC denial, OS URL/file bounds, remote-media fetch consent, honest R2/R6/R7 claims | [`PERFORMANCE-SECURITY-HARDENING.md`](PERFORMANCE-SECURITY-HARDENING.md), [`design-native-paging.md`](design-native-paging.md); focused frontend/Rust tests; production bundle measurements; full gates before handoff |
| P1: epoch close, owner checkpoints and bounded recovery | 16 | **Active.** The replication core, typed Registry/Studio checkpoint materializers, receipt-bound settlement, recovery-first installation, authenticated paging/discovery, watched rotation, bounded replay and detached Studio/Registry repair are connected. PR #41's Studio consumer uses the lifecycle/refresh evidence for the bounded Gate 4 path. Specialized states outside that path, other managed-file families and wider production orchestration remain later work. | Receipt forgery and replay, owner-tenure equivocation, adoption by a newcomer reading old-owner heads, crash between seal and recovery snapshot, recovery-slot exhaustion, unbounded checkpoint size, page-exchange authentication and capacity abuse, pruning before a durable receipt | [`design-epoch-close.md`](design-epoch-close.md) (revision 5) is authoritative; [`INTERFACES.md`](INTERFACES.md) records the implemented contracts. Focused crash/restart, rotation, paging and replay regressions pass; Gate 4 integration evidence is tracked in [`GATE4-ACCEPTANCE.md`](GATE4-ACCEPTANCE.md). |
| Creative suite / Flipnote Studio | 17 | **Gate 6 implemented, cumulatively reviewed and locally verified; hosted acceptance pending.** Gates 1–4 supply the accepted Index/art/P1 baseline; Gate 5 claims/Ask/Pass is owner-accepted and merged as PR #43 at `7cbe8cbe131e1be8e672d04341c31a1d2b52a98d`. Stacked PR #44/#45 add typed Score/sound persistence, native/UI/export/GIF and the existing deck's archival take consumer. Explicit backend SoundRecord recovery is implemented/bounded-reviewed through `9fed4588`; cumulative review and complete local root/native/frontend/startup/flow checks pass through `818e7116`, while hosted/Linux CI remains pending. Direct Closing Save/native unfinished repair stay unavailable, while Gate 4's bounded lifecycle P5 promotion remains accepted. Gate 7 release/two-real-client acceptance, shared-deck PIXA consensus/global viewer and broader emoji catalog/chat clicks remain later. Games and game-only avatar work stay paused. | Cross-family causal/state/recovery binding, raw conflict/removal retention, exact held-media custody, byte/frame/recipe/work caps, scope/Deafen/download ABA, no remote auto-resume, claims never becoming edit authority, truthful Copy consent and export acknowledgement | [`GATE6-ACCEPTANCE.md`](GATE6-ACCEPTANCE.md) pins the stack/reviews/evidence and explicitly pending checks; [`BACKEND-IMPLEMENTATION.md`](BACKEND-IMPLEMENTATION.md) is the gate board, [`design-creative-suite.md`](design-creative-suite.md) the product contract and [`FLIPNOTE-UI-HOOKS.md`](FLIPNOTE-UI-HOOKS.md) the current native/UI inventory. Earlier successful bounded reviews do not alone close Gate 6 or Gate 7. |

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
