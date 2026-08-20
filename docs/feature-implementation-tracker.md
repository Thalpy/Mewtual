# Feature implementation tracker

This tracker is the development gate for the requested operations work and the agreed experimental
ideas. A feature is not “done” until its implementation, focused tests, antagonist review,
documentation, and the repository's mandatory full suites all pass.

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

## Experimental ideas (after phase 12a-12d)

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
