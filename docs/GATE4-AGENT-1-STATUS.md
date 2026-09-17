# Gate 4 Agent 1 status: local Save and automatic handoff runtime

Owner: Agent 1 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-1-local-save-and-automatic-handoff-runtime)).
Proposal: [GATE4-AGENT-1-DESIGN](GATE4-AGENT-1-DESIGN.md), revision 4, **accepted (PASS) at
`25ce89bb705dfc228c7b32874788ebd062e6fcf4`, 2026-09-15, design boundary only**.
Review preamble: 1. Current entries override older ones.

**The design boundary is complete. Implementation has started** on branch
`gate4-agent1-runtime`, from `5a899c2`. Progress and executed evidence are in "Implementation
progress" below. The bounded implementation review uses the request in design 18.3, and must not be
sent until the scope it names has evidence.

### Implementation sequencing, revised from design 15

Design 15 proposed I-4 and C-3 first because they are the largest shared-seam items. That ordering
is changed, deliberately and for stated reasons: C-1 is self-contained, removes the dominant cost
that every other stage depends on, and does not collide with Agents 2 and 3, who are editing the
same store files. I-4 and C-3 bound the commit visit's custody but block no product behaviour, and
they are the highest-conflict changes, so they land last. The per-item verdict request in design
18.3 is unchanged; only the order is.

| Order | Item | State |
|---|---|---|
| 1 | C-1 structural decode, with C-2's digest fences and the R4 replay exclusion | **landed; reviewed PASS; no-replay boundary covered by N24; R4 selection covered by N25/M9** |
| 2 | C-4 transient reference holds, with the I-3 transfer | **landed; seam reviewed PASS; transfer implemented and mutation-proven; dead-code markers removed** |
| 3 | The runtime: Flows S, H and R, admission, scheduling, commit seams | **Flow S and Flow H both land end to end as scheduled jobs. Flow R not started; no native command, gated on Agent 2's P5** |
| 4 | I-4 and C-3 | not started |

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `a052f78b62a549702686a8741932f1d2f8c98773` | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | design, docs only | **REQUEST CHANGES**: AG1-001 to AG1-005, AG1-TEST-001 |
| 2026-09-15 | Design revision 2 | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | design, docs only | **REQUEST CHANGES**: AG1-004 **closed**; residuals on the other five |
| 2026-09-15 | Design revision 3 | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | `1bcb1bca204d721b848b17c0835faf931ae930e3` | design, docs only | **REQUEST CHANGES**: AG1-001, AG1-002, AG1-003, AG1-005 **closed at the design boundary**; AG1-TEST-001 open, P3 |
| 2026-09-15 | Design revision 4, N31 correction | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | design, docs only | **PASS**: AG1-TEST-001 **closed**; bounded runtime design accepted, no new finding |
| 2026-09-15 | PASS recorded | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | `5a899c2` | docs only | n/a; records the verdict and the next checkpoint's request |
| 2026-09-15 | C-1 and C-4 implementation | `5a899c2` | `d67e649`, `7ed6302` | bounded implementation, PR #27 | **PASS by source inspection** for the C-1 decoder, the C-2 digest changes, the R4 filter and the C-4 seam; no production defect found. Two coverage findings: **C1-TEST-002** (P2) and **R4-TEST-001** (P3). C-1 evidence not a completed checkpoint. |
| 2026-09-15 | C1-TEST-002 correction | `7ed6302` | `4d09869` | test only | **PASS**: C1-TEST-002 **closed**; R4-TEST-001 still open |
| 2026-09-15 | R4-TEST-001 correction | `4d09869` | `079e59a` | test plus one cfg(test) helper | N25 and M9; pushed, awaiting the reviewer's source inspection to close |
| 2026-09-15 | I-3 protection transfer | `079e59a` | `65e77d1` | bounded implementation | Mechanism **PASS**; **I3-001** (P2) opened: media admission ran before retry classification |
| 2026-09-16 | I3-001 correction | `65e77d1` | `b7df00b` | bounded implementation | **PASS**: I3-001 **closed** |
| 2026-09-16 | Flow S staged seams and N12(a) | `7b9cf3e` | `1c3a1e0` | bounded implementation | Split and N12(a) **PASS**; **FS-001** (P2) opened: new authoring could durably accept a missing PIX blob |
| 2026-09-16 | FS-001 correction | `1c3a1e0` | `c57faee` | bounded implementation | S1b admission and S3 possession recheck, with N12(d) and M15 |
| 2026-09-16 | Per-actor overlay admission | `c57faee` | `bbfd5e5` | bounded implementation | Seam **PASS**, scoped to its own tests rather than end to end; **FS-002** (P2) opened against Flow S: a stale request still performed media admission before being identified as stale. One P3 API-hardening point on the capture's independent media parameters. |
| 2026-09-16 | FS-002 correction and `AdmittedOverlayMedia` | `bbfd5e5` | `5a024a7` | bounded implementation | Authorization moved ahead of media admission; media minted as one unforgeable value. New stale-basis regression with two hazards and two positive controls, plus M16. Awaiting review. |
| 2026-09-16 | `EpochRecordKind::DraftArchive` seam | `5a024a7` | `705d44b` | integration seam for Agent 2, option (a) | Physical family plumbing only: no payload, writer, release path, reference collector or sub-cap, and no I-4 guard. Two regressions, M17/M18/M19. Reported to Agent 2; handover sent to Agent 3. |
| 2026-09-16 | FS-002 and seam review | `bbfd5e5` | `70eaad4` | bounded implementation, two scopes | **FS-002 closed**, A-1 confirmed preserved, stale-basis fixture and both controls confirmed valid. **A-001** (P3): admitted media not bound to the intent capture receives separately. **B-001** (P2): the shared scope decoder caps every family at Recovery's 501 bytes, refusing a legal 506-byte archive scope. |
| 2026-09-16 | A-001 correction | `70eaad4` | `35929a9` | bounded implementation | **PASS, A-001 closed.** `AdmittedOverlayAuthoring` as one value plus a `MediaOrigin` rechecked at capture and commit; regression and M20. |
| 2026-09-16 | B-001 correction | `35929a9` | `1cac519` | integration seam correction | **PASS, B-001 closed.** Per-family `scope_cap()`; two regressions and M21. |
| 2026-09-16 | Overlay runtime, Flow S detached | `130a64b` | `dda64a9` | bounded implementation | Split, job and result, admission consumed, marker removed. **REQUEST CHANGES**: the `OverlayOwnership` deviation **accepted** (design to change, not the code), the withdrawn control action **PASS**, M23 **PASS**; **RT-001** (P2) and **RT-002** (P2) opened; the cancellation half of N14(a) still open at P3. |
| 2026-09-17 | RT-001 and RT-002 corrections | `dda64a9` | `3893ee2` | bounded implementation | **PASS, both closed.** The `detach`-not-`reserve_overlay` gate placement explicitly signed off. A refused plan releases admission and its pool slot in the worker; `OverlayPlan` selected only when `replay_ready()` holds. M24, M25, design 5.5 corrected. |
| 2026-09-17 | N14(a), the real cancellation race | `3893ee2` | `746c0e2` | test plus one cfg(test) seam | **PASS, N14(a) closed.** Taken before Flow H at the reviewer's direction, because Flow H rewrites the machinery N14(a) guards. A real paused worker, a real `RequestCancellation`, and M26. One non-blocking ergonomics point, since addressed at `3d46016`. |
| 2026-09-17 | Flow H stages H1 and H2 | `3d46016` | `92b55ca` | bounded implementation | The custody boundary settled with the reviewer first. H1 bounded and cheap, H2 detached with captured public context only. One algorithm for both callers. New stamp regression and M27. |
| 2026-09-17 | Flow H stage H3 | `92b55ca` | `c676749` | bounded implementation | The one signing loop, with the two events separable. N31 and M5a/M5b/M28. |
| 2026-09-17 | Flow H stages H4 to H6, scheduled | `c676749` | uncommitted working tree | bounded implementation | H4 detached; the receiver stage machine, probe, slice visit, commit and notify. **Self-reviewed adversarially before commit**: two P1s, six P2s and seven P3s found, ten corrected here. M29, M30, M31. |

Working checkout: `M:\Git (local)\CatComs`. The four design passes were made on `Create-suite-2`;
implementation is on `gate4-agent1-runtime`, which is the current branch.

**Other agents are working in this same checkout, on this same branch.** Every commit must be
pathspec-scoped so it carries no other agent's work. The hazard runs in both directions and both
have now been observed: Agent 3's `63a11e1` and Agent 2's `bd878d7`/`21ca8fa` became ancestors of
this branch, and on 2026-09-16 another session's push published Agent 1's `5a024a7` to
`origin/gate4-agent1-runtime` without Agent 1 pushing it. A local head is therefore not reliably
unpublished, and an Agent 1 commit can reach the remote before its evidence is complete. Resolve
review SHAs against the actual remote rather than assuming.

### Two mis-scoped Agent 1 commits, retained deliberately

`0467e45` and `c3a702a` both carry the Agent 1 subject "A-001: bind admitted media to the operation
that consumes it" but are **not** Agent 1 scope and must not be reviewed as such.

| Commit | What it actually contains |
|---|---|
| `0467e45` | The genuine A-001 change to three `epoch_studio` files, **plus** two of Agent 2's untracked files, `studio/overlay/archive.rs` and `studio/epoch/owner/tests/archive.rs` |
| `c3a702a` | Nothing of Agent 1's: four of Agent 2's modified files, including `docs/GATE4-AGENT-2-STATUS.md` |

Cause: another session staged into the shared index between this session's `git add` and its
`git commit`, so an explicitly pathspec-scoped `add` was not sufficient. The first attempt to
correct it raced the same session and produced the second bad commit; by then `0467e45` had already
been pushed by a third party and merged back at `cee38b3`, so it could not be removed locally.

**These commits are load bearing and must not be rewritten.** Agent 2's two archive files and that
version of their status note exist nowhere else in history. Dropping the commits would delete
Agent 2's work from the branch. The clean Agent 1 versions are `35929a9` (A-001) and `1cac519`
(B-001), and those are the review heads.

The same incident cost Agent 3 uncommitted edits: a `git reset` and a `git stash` from this session
discarded working-tree state they had not yet committed, and they recovered by landing revision 10
in five parts, recorded at `f5ac522`. Nothing was permanently lost, in either direction.

**Rule for this branch from here:** verify the staged set with `git status --short` **after**
`git add` and immediately before `git commit`, in the same invocation, and never run `git reset`,
`git stash` or any other index- or worktree-wide operation while another session may be active.

## Finding ledger

Closure at the design boundary is not implementation acceptance: every mechanism below still needs
code and executed evidence, and the reviewer said so explicitly for each one.

| Finding | Severity | Status | Where |
|---|---|---|---|
| AG1-001 | P2 | **Closed at the design boundary** (revision-3 review). Accepted and completed retries no longer require media admission: S0 is split into common request validation, classification, and new-authoring-only media admission, and S1a is terminal and sync-only. | Design 6.2, 6.3, 14 N30, M16 |
| AG1-002 | P2 | **Closed at the design boundary**, subject to I-4's stated implementation audit. `inventory_generation` replaces the invalid two-token assumption; the reviewer confirmed the underlying facts about the recovery and owner writers and about budget entry. | Design 9.2, R10, 14 N17, M20, M21 |
| AG1-003 | P2 | **Closed at the design boundary.** I-3 establishes ordinary protection before potentially durable I/O and before transient ownership is released. | Design 8.3, R7, 14 N12, M14 |
| AG1-004 | P2 | **Closed** at revision 2. Prepared alone no longer prohibits nondestructive access; Agent 2 still owns the concrete lifecycle. | Design 12.1 |
| AG1-005 | P3 | **Closed at the design boundary.** Admission depends on actual owners, not an actor-side strong reference awaiting cleanup. | Design 5.5, 7.1, 14 N14, M3 |
| FS-001 | P2 | **Closed.** New authoring could durably accept an operation naming pixels the vault does not hold. S1b admission and the S3 possession recheck now exist, with N12(d) and M15. | Status "FS-001" |
| FS-002 | P2 | **Closed** at the `70eaad4` review. Classification is not authorization: a stale request still read, could promote and could hold pixels before the basis comparison refused it. Authorization now precedes media admission, A-1 confirmed preserved, and both positive controls confirmed to discriminate. | Status "FS-002" |
| A-001 | P3 | **Corrected, awaiting review.** `AdmittedOverlayMedia` prevented fabricated facts but was not bound to the `LocalIntent` capture received separately, so media for operation A could be captured against operation B. Now one combined `AdmittedOverlayAuthoring` value with a private `MediaOrigin` rechecked at capture and at commit. | Status "A-001" |
| B-001 | P2 | **Closed** at the `70eaad4` review. `decode_record_scope` prechecked every family at Recovery's 501-byte maximum, so a legal 506-byte maximum-shape `DraftArchive` scope was refused outright. The bound is now per family. | Status "B-001" |
| RT-001 | P2 | **Corrected, awaiting review.** A refused plan parked `OverlayOwnership` against a plan that could never commit, holding this actor's admission and one of four process-wide preparation slots until some later Save collected it, or forever. The worker now releases both the moment planning fails. | Status "RT-001 and RT-002" |
| RT-002 | P2 | **Corrected, awaiting review.** `detach` selected `OverlayPlan` ahead of authoritative catch-up with no `replay_ready()` check, reversing the accepted priority: L7 accepts overlay starving under catch-up, never the reverse. | Status "RT-001 and RT-002" |
| N14(a) cancellation | P3 | **Closed.** The race is now executed rather than modelled: a real paused worker, a real `RequestCancellation`, the ordinary `complete` path, and the recovery when the worker ends by itself. M26 guards it. | Status "N14(a)" |
| AG1-TEST-001 | P3 | **Closed at the design boundary** (revision-4 review). The residual was that N31 required only `remaining() > 0`, which a visit deferring on the priority gate without signing also satisfies, so both the unchanged and the mutated implementation could pass. Revision 4 adds a positive signing precondition (`after < before`, `after > 0`, exact expected count derived from production `remaining()`), a deterministic injected-clock seam, authoritative work staged only after slice selection, and independent per-limit preconditions, with M5 split into M5a and M5b. The reviewer confirmed the production basis: `remaining()` delegates to the pending queue and `sign_next` removes exactly one item only after the signature succeeds, so the delta counts **successfully produced** signatures. | Design 7.3, 14.1 "N31 in full", 14.2 M5a/M5b |

## Audit claims corrected across revisions

| Claim | Correction |
|---|---|
| Ordinary Studio reads always reconstruct a retained branch (rev 1) | Conditional on the source wrapper's required-metadata link byte plus a retained Active or Prepared branch. The unconditional costs are `checked_epoch_replay_state` and the Intents arm of every five-family scan. |
| "About four" reconstructions in the synchronous commit (rev 1) | Withdrawn; path dependent and possibly higher. |
| C-1's moved call sites are "projection-free" (rev 1) | Corrected to branch-replay-free: the reference path retains `base_blob_cids`, which calls `base.graph()` and performs typed seed work. |
| C-1 justified by AEAD sealing (rev 1) | Replaced by invariant I-1, a requirement on the writers. Authentication proves origin and integrity, not typed admission, and not every `write_prepared_intents` call follows a fresh `append`. |
| The digest fence is "strictly stronger" (rev 1) | Withdrawn. The existing decoder already requires canonical re-encoding equality. C-2's justification is cost only. |
| Intents is the one uncached family (rev 2) | Wrong. `inventory_cache` admits only `Registry` and `Studio`, and only when not collecting references, so `Recovery`, `OwnerReceipts` and `Intents` are all uncached. The surviving point is that the Intents arm is the one whose uncached cost scales with a retained branch. |
| `write_prepared_intents` performs an old-record read (rev 2) | Wrong. It consumes a supplied `old: Option<u64>`. C-2 changes its **callers** (`persist_handoff_intents`, `write_studio_overlay_intent`, `save_studio_closing_overlay_with_io`). |
| C-3 is a mechanical refactor (rev 2) | Wrong. It is a semantic consistency change; cross-visit resumption is sound only because of I-4. |
| H1 capture is strictly fresher (rev 2) | Withdrawn. Earlier capture can become stale while H2 runs; what is guaranteed is that a stale authority cannot authorise a signature or a commit. |

## Facts established by the revision-3 audit

Verified in the code at the design base, relied on from revision 3 onward, and confirmed
independently by the revision-3 reviewer:

- The only rotations of an inventory-relevant token are `epoch_intents.rs:479`, `:510`,
  `epoch_intents/retirement.rs:199`, `:233` (`intent_generation`),
  `epoch_recovery/cleanup.rs:94` (`studio_generation`), `:155` (`intent_generation`, coverage
  dependent), and `epoch_studio.rs:168` and `:203` (`studio_generation`, on budget mint and on
  budget **entry**).
- `update_epoch_recovery_accounted_with_writer` and `update_epoch_owner_state_with_writer` reserve,
  write and commit real record replacements and rotate neither token. This is AG1-002's
  counterexample, confirmed.
- `enter_studio_budget_scope` rotates `studio_generation` on every entry, so reusing it as C-3's
  token would also make a parked cursor die on unrelated Studio activity.
- `inventory_cache` is consulted only for `Registry` and `Studio`, and only when not collecting
  references.
- `write_prepared_intents` consumes a supplied `old` and rotates `intent_generation` on both
  branches.
- `StudioOverlay::encode_vault` itself calls `checked_entries`, which is why M8 had to move.
- `Protection::unknown()` drops `pins`; `install()` replaces the known set; `ProtectedBlobs::delete`
  consults only that set. Both halves of AG1-003, before and after the durable write, follow from
  this.

## Dependencies

| Dependency | Current state | Effect if unmet |
|---|---|---|
| Core signing split `e65bfd8` | **Unreviewed.** Both reviewers inspected its interfaces without granting a PASS. | Design 6.1 and 9 are bound to `handoff_authority`, `prepare_handoff_detached`, `sign_next` and `finish`. Contingency in design 16. |
| Agent 2 manual overlay lifecycle | Not started. | Native Save stays unregistered and absent from FLIPNOTE-UI-HOOKS. Prerequisites P1 to P5, design 12.3, now including `ReferenceCapacity` and `InventoryUnstable` holds. |
| Agent 2 live-tenure contract | Not started. | The runtime binds `tenure` as an opaque `u64` and needs "equal value implies the same continuous tenure" (P4). |
| Agent 2 copy contract | Not started. | Design 12.2 lists the requirements the revision-2 reviewer attached to copy-while-Prepared, including that a different channel label for the same Flipnote object is not an independent destination. |
| Agent 3 signed repair | **Design revision 1 landed at `7efc9c2`.** Its section 13.1 accepts I-4, names the owner record write, the recovery stage and the successor write as the three of its writers that must rotate the token, states its design is unaffected if I-4 does not land, and confirms it adds no competing source writer and no second preparation pool. It asks that `save_studio_source_checked`'s `handoff` parameter shape be preserved; this design preserves it. | The coordinated verdict on I-4 now has both sides on record. The exhaustive choke-point audit remains an implementation-review obligation. |
| Agent 4 shared seam, enum, registration and workflow edits | Not started. | Design 15 lists every central edit. I-4 is now the largest and highest-risk item. |

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-1-DESIGN.md) section 5. Summary:

- Core: `StudioOverlay::decode_vault_structural`, `StudioOverlayState::decode_vault_structural`
  (C-1).
- Store: `capture_studio_overlay`, `studio_overlay_is_current`, `studio_overlay_structural`,
  `commit_studio_overlay_state`, `studio_closing_basis`, `VerifiedPersistedSource`,
  `hold_creative_transient` and `CreativeHold` (C-4), `epoch_mutation_guard` and
  `inventory_generation` (I-4), `begin_epoch_storage_scan` / `step_epoch_storage_scan` /
  `finish_epoch_storage_scan` and `EpochStorageCursor` (C-3); `StudioOverlayStamp`,
  `StudioOverlayCapture`, `StudioOverlayWork`, `StudioOverlayPlan`, `StudioOverlayPlanned`,
  `StudioOverlayHold`, `StudioOverlayCapture::plan`.
- App runtime: `OverlayAdmission` (weak-handle bookkeeping), `OverlayRuntime`, `OverlayJob`,
  `OverlayStage`, `OverlayOwnership`, `StudioBackgroundJob::{OverlayPlan, OverlayAssemble}`,
  `StudioBackgroundResult::{OverlayPlanned, OverlayAssembled, OverlayCancelled}`,
  `studio_overlay_live_hold`, `studio_overlay_transfer_hold`.
- Control: `StudioControlAction::{BeginOverlaySave, PrepareOverlaySave, FinishOverlaySave}`,
  `StudioControlResponse::{OverlaySaveBasis, OverlayAcknowledged, OverlaySavePreparation,
  OverlaySaved}`, `StudioOverlayAcknowledgement::{LocalDraft, Handoff}`.
- Settlement: `StudioSettlementState::{LocalDraftRetained, LocalDraftHandedOff}`.
- Native (designed, **not registered**): `studio_overlay_begin`, `studio_overlay_save`, plus
  `transferState: "completed"` on the existing `studio_overlay_read` result.

**None of these exist yet.** A proposed name is not an implementation.

## Invariants this scope must not weaken

1. Prepared, whole Source, Completed, with actual evidence rechecked at every barrier. No durable
   signed prefix, no per-entry retirement, no bypass cache.
2. The complete original pending ledger survives handoff; only the overlay exclusion is removed.
3. HANDOFF-001 complete-target comparison before any completed acknowledgement, source lookup or
   sync reservation.
4. HANDOFF-002's authenticated inventory dependency **and**, separately, `check_handoff_references`
   (R9).
5. The Prepared source-replacement fence and the publication hold, including after restart.
6. First local acceptance derives from the actual Closing source, its matching saved signed close
   and independently observed tenure; ordinary failed Apply stays `NoEvidence`.
7. One shared four-slot `registry_catchup::preparation_pool()`, plus **I-2**: one live per-actor
   overlay admission, proved by a live `Arc` and never by a flag some path must clear.
8. **I-1**: first acceptance fully validates the branch; every later writer preserves the
   already-checked identity and evidence.
9. **I-3**: a job-owned reference hold is released only after ordinary conservative protection
   covers the same references.
10. **I-4**: `inventory_generation` rotates before any five-family durable mutation, including
    temporary siblings and unlinks, and before its first possible I/O.
11. Device and MLS secrets never leave the actor; a detached worker owns authenticated plaintext,
    public context, its permit, its admission token and any transient reference hold only.
12. Completed publication uses ordinary tail and page service; the initial Save window stays at two
    packets.
13. Every refusal, hold, cancellation and capacity failure retains the complete branch.

## Implementation progress

Branch `gate4-agent1-runtime`, based on `5a899c2`. Nothing is merged and no native command is
registered.

### C-1 structural decode, with C-2 and R4

Production changes:

| File | Change |
|---|---|
| `crates/catcoms-replication/src/studio/overlay.rs` | `StudioOverlay::{decode_vault, decode_vault_structural}` over a shared `decode_vault_inner(.., replay)`. The structural path keeps the bounds, version tag, nested seed and metadata bounds, target derivation, receipt and ledger scope equality, `checked_entries` and the canonical `encode_vault == bytes` comparison; it skips only `read`. |
| `crates/catcoms-replication/src/studio/overlay/handoff.rs` | The same pair on `StudioOverlayState`, threading `replay` to the nested branch decoder. |
| `crates/catcoms-app/src/store/epoch_intents.rs` | `EpochIntentState::{decode, decode_structural}`; `read_epoch_intent_record_structural`; `load_epoch_intents_structural`; `read_scoped_intent_plain` made visible in the store. `checked_epoch_replay_state` and `prepare_epoch_intent_with_io` now read structurally; `flush_checked_epoch_intents` takes only the physical size (C-2). |
| `crates/catcoms-app/src/store/epoch_intents/{overlay,retirement}.rs` | Retirement reads structurally; the overlay writer takes only the physical size (C-2). |
| `crates/catcoms-app/src/store/epoch_studio/handoff.rs` | `check_studio_handoff_write`, `check_studio_handoff_publication` and `check_studio_intent_link` read structurally. The commit's unchanged fence compares the complete authenticated plaintext digest and physical size instead of decoding and re-encoding (C-2); `persist_handoff_intents` takes only the size. |
| `crates/catcoms-app/src/store/epoch_recovery/inventory.rs` | The Intents arm decodes structurally. Reference collection still uses `base_blob_cids`, which keeps its typed seed work, and HANDOFF-002's dependency check is untouched. |
| `crates/catcoms-app/src/store/epoch_studio/recovery_disposition.rs`, `studio/control.rs` | Pending-count readers use the structural load. |
| `crates/catcoms-app/src/studio/replay.rs` | **R4**: `studio_replay_evidence` now excludes annotated ids from `own` explicitly, rather than relying on `choose` returning `NoEvidence` for them. Ordinary failed-Save `NoEvidence` is unchanged. |

New regressions, in `crates/catcoms-replication/src/studio/epoch/owner/tests/handoff.rs`:

- `studio_overlay_structural_decode_matches_full_decode_and_keeps_its_entry_checks`: for Index and
  Flipnote at 1 and 4 accepted entries, both decoders produce byte-identical canonical encodings and
  equal target, Prepared, Completed, floor, basis, author and projection; a ledger missing one
  annotated entry is refused by both; trailing and truncated records are refused.
- `studio_overlay_structural_decode_rejects_a_wrong_acceptance_sequence`: patches only the accepted
  entry's big-endian sequence inside the **branch** encoding, asserting first that the field really
  held 1 and that the length did not change.

### Executed evidence for C-1

| Check | Result |
|---|---|
| `cargo check -j 1 -p catcoms-replication --lib`, then `-p catcoms-app --lib` | Both clean. |
| `cargo test -j 1 --config profile.test.package.catcoms-replication.debug=0 -p catcoms-replication --lib studio` | **104 passed, 0 failed**, 63.22 s, before the new tests were added. |
| `... --lib studio_overlay_structural` | **2 passed, 0 failed**, 12.99 s. |
| M8 mutation, `entry.sequence != index as u64 + 1` weakened to `&& false` in the shared `checked_entries` | See below. |
| Restored source, rerun | Diff confirms only the C-1 additions remain; **2 passed, 0 failed**, 12.70 s. |
| `cargo test -j 1 --config profile.test.package.catcoms-app.debug=0 -p catcoms-app --lib studio_overlay -- --test-threads=1` | **37 passed, 0 failed, 2 ignored** (the opt-in profiles), 978.39 s. Log `logs/gate4-a1-c1-overlay.log`. Covers the overlay foundation and the whole handoff transaction, including the crash matrix, the replacement and publication fences, retirement, migration and the reference dependencies. |
| `cargo clippy -j 1 -p catcoms-replication -p catcoms-app --lib --tests -- -D warnings` | Clean, 37.67 s. |
| `cargo fmt --all -- --check` | Clean after reformatting two assertions in the new test. |

**The M8 mutation caught a defect in my own first test.** Against the mutant the sequence test
still passed, because it patched the last 88 bytes of the enclosing version-2 **state** record,
whose tail is the completed-acknowledgement section rather than the branch's entry. The record was
being rejected by an unrelated check, so the fixture proved nothing: exactly the masking class the
reviewer raised three times. The test now patches the **branch** encoding and asserts the field it
targets held 1 beforehand and that the length is unchanged. Against the mutant it then fails at its
intended assertion, "structural decode accepted sequence 2 for the first accepted entry"; after
byte-exact restoration it passes. This is why the guard-breaking step is not optional.

### C-4 transient reference holds

`crates/catcoms-app/src/store/creative_references.rs`: `Protection` gains a `transient` table of
job-owned holds keyed by a `Weak` owner. `unknown` and `install` leave it untouched and a complete
scan may subtract only from `pins`, so a reference no durable record names yet survives the scan
that would otherwise reclaim it. `ProtectedBlobs::delete` reaps dead owners and consults the
transient table before the installed set, under the same guard it already holds through unlink, so
a live hold protects even while durable protection is unknown. `hold_creative_transient` checks the
owner rail and the shared `MAX_CREATIVE_REFERENCES` rail **before** installing anything, and
refuses rather than marking the store unknown. `CreativeHold` releases on drop.

The existing accepted test
`transient_preholds_survive_budget_scans_but_only_complete_reference_scans_can_unpin` already
asserts the hazard this closes: after a complete scan that no durable record informed, the orphan
CID deletes. Two new regressions:

- `job_owned_transient_holds_survive_a_complete_scan_and_release_with_their_owner`: the CID
  survives two complete scans while held, an unrelated orphan still deletes, and the CID becomes
  deletable the moment the owner drops. That last assertion is the point: it is why I-3 must
  install the ordinary conservative holds before the owner can disappear.
- `transient_hold_exhaustion_refuses_admission_without_disturbing_existing_protection`: the owner
  rail refuses one too many, protection stays known, unrelated reclamation still works, and
  releasing one owner readmits exactly one.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib creative_references -- --test-threads=1` | **9 passed, 0 failed**, 3.34 s, including the seven pre-existing tests. |
| M13 mutation, `transient_holds(..) && false` in `ProtectedBlobs::delete` | Fails at "a live job-owned hold did not protect its reference"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

### I-3, the protection transfer

C-4's seam now has its production consumer and the four `#[allow(dead_code)]` markers are gone.

`save_studio_closing_overlay_with_io` takes a job-owned `CreativeHold` over the operation's pixel
references immediately after the operation is decoded and before anything durable happens. The
guard lives to the end of that scope, so it is released only after the write attempt returns,
on success, on error and on unwinding alike. `write_studio_overlay_intent` already installed the
two ordinary conservative holds before its write; that call site is now documented as the second
half of the transfer, because it is what makes dropping the transient owner safe.

The ordering the accepted design requires, as implemented:

```
decode and bound the operation
  -> hold_creative_transient(operation pixels)      // job-owned, covers the in-flight window
  -> basis / exact-retry / eligibility checks
  -> hold_creative(base CIDs) + hold_creative_operation(new operation)   // the transfer
  -> write attempt
  -> drop CreativeHold                              // scope end, after the attempt returns
```

Two regressions in `store/epoch_studio/tests/rotation/overlay.rs`:

- `studio_overlay_acceptance_transfers_pixel_protection_before_its_write`: a complete reference
  scan runs **before** the acceptance and installs a known set excluding the new CID, so the
  assertion cannot pass through fail-closed unknown protection. After the Save succeeds and every
  owner is gone, and before any further scan, the CID is still retained, while an unreferenced
  orphan still reclaims and protection is still known. That control is what distinguishes a real
  transfer from a blanket refusal.
- `studio_overlay_uncertain_acceptance_still_protects_its_pixels`: two injected failures, one where
  the record really lands and only the caller's result is lost, and one that fails leaving a
  temporary sibling. Both leave the pixels protected, because the transfer runs before the write
  attempt and does not depend on its outcome.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib studio_overlay -- --test-threads=1` | **39 passed, 0 failed, 2 ignored**, 1237.73 s. Log `logs/gate4-a1-i3-overlay.log`. The 37 pre-existing overlay and handoff tests plus the two new ones. |
| `cargo test ... --lib creative_references` | **9 passed, 0 failed**, 3.26 s. |
| M14, removing only the two ordinary holds from `write_studio_overlay_intent` | Fails at "an accepted operation's pixels were reclaimable before the next scan". `git diff` confirms the restored file differs from `079e59a` only by the new comment, so the two calls are byte-identical. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean, with no `allow(dead_code)` remaining in `creative_references.rs`. |

### I3-001: media admission moved behind classification

The first placement of I-3's transient hold was wrong: it ran before `completed_retry` and
`exact_retry`, so an already accepted request could be refused by new-authoring reference
admission when the shared rails were full. That is the coupling AG1-001 closed, reintroduced.

The hold now sits after classification and only on the new-authoring path:

```
decode and bound the operation
  -> enter budget, read structural state
  -> completed_retry            -> acknowledge and return
  -> exact_retry                -> flush-only, no media admission
  -> ordinary-intent collision  -> refuse
  -> ONLY IF genuinely new authoring:
       operation_blob_cid, hold_creative_transient
       fresh Closing eligibility and basis
       ordinary I-3 holds, write attempt
  -> drop the transient owner after the attempt returns
```

The ordinary-collision check is hoisted to the same classification stage; the writer keeps its own
copy as defence in depth, which the reviewer explicitly permitted.

`studio_overlay_exact_retry_is_acknowledged_without_media_admission` accepts a pixel-bearing
operation, fills every job-owned hold slot with unrelated live work, proves a further hold is
refused, then retries the exact accepted request. It requires the retry to succeed, to return the
same accepted draft, to leave the durable records unchanged, and to leave the live hold count
unchanged. It calls the store directly rather than through the fixture helper so the failure names
this boundary instead of unwrapping. A `#[cfg(test)] live_transient_holds_for_test` counter makes
"took no hold" an observation rather than an inference from a successful result.

Moving the hold back above classification fails it at "an accepted retry was refused by media
admission: creative reference scan incomplete, unsupported or over bound"; the restored source
passes. The first attempt at this mutation surfaced as a generic `.unwrap()` inside the fixture
helper, which is why the retry now goes through the store API directly.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib studio_overlay -- --test-threads=1` | **40 passed, 0 failed, 2 ignored**, 995.49 s. Log `logs/gate4-a1-i3001-overlay.log`. |
| I3-001 mutation, taking the hold unconditionally | Fails at "an accepted retry was refused by media admission"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

### Flow S staged seams, and N12(a)

`store/epoch_studio/overlay_capture.rs` adds the three stages the design specifies:

- `capture_studio_overlay_save` (custody): authenticates the bounded intent record **without
  decoding it**, captures the stamp (mount, numeric server, complete target, document, actor and
  key, designated owner, MLS epoch, and the record's authenticated plaintext digest with its
  physical size, absence explicit), and takes ownership of the caller's media hold.
- `StudioOverlayCapture::plan` (detached): the expensive stage. Full `EpochIntentState::decode`,
  which reconstructs any retained branch, then `append`, which replays it again with the new
  operation. It owns authenticated plaintext, public context, the basis and the hold; no store,
  Server, device key, MLS secret or writer, and it writes nothing. It repeats the
  ordinary-collision check rather than trusting a decision taken against bytes it cannot see.
- `commit_studio_overlay_save` (custody): re-mints the Closing basis from actual durable state and
  requires the same fingerprint, checks stamp equality, runs the I-3 transfer, then performs one
  accounted intent write. The hold is released only when it returns.

`save_studio_closing_overlay_with_io` now **composes those three inline**, so there is one
algorithm rather than a batch path that can drift from the scheduled one. The classification order
established by I3-001 is preserved exactly: completed retry, exact retry and ordinary collision all
precede media admission, and the exact-retry path returns before any capture.

| Check | Result |
|---|---|
| `cargo test ... --lib studio_overlay -- --test-threads=1`, after the refactor and before the new tests | **40 passed, 0 failed, 2 ignored**, 1123.67 s. Behaviour equivalence for the staged composition. Log `logs/gate4-a1-flows-overlay.log`. |
| `... --lib studio_overlay_detached` | **2 passed, 0 failed**, 22.52 s. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

**N12(a) is now real.** `studio_overlay_detached_acceptance_survives_a_complete_scan_between_capture_and_commit`
installs a known pin set excluding the new CID, captures, runs `plan` with custody genuinely
released, and then completes a full reference scan and attempts protected deletion **while the job
is detached**. The pixels survive; an unreferenced orphan still reclaims, so the scan is complete
and usable rather than fail-closed. After the commit, with every owner gone and before any further
scan, the ordinary holds keep them. Dropping the hold before the capture, so the detached job
carries none, fails the test at "a complete scan reclaimed pixels held by a detached acceptance".
That is a fixture-side check of the assertion's discriminating power, not an isolated production
mutation.

`studio_overlay_detached_plan_is_refused_when_the_record_changed` covers the other half: a plan
derived from superseded bytes is refused at the stamp comparison, records are unchanged and the
accepted branch is undisturbed.

### FS-001: the media-availability contract

Flow S had neither of the two media checks the design specifies.
`catcoms_replication::studio::operation_blob_cid` extracts an address and validates grammar and
scope; it does not establish that the blob exists. The typed admission layer checks operation
semantics and declared byte and count caps, not physical availability. A transient hold is a
liveness claim over an address, and typed replay never fetches pixels. So a valid `InsertFrame`
naming a CID the vault does not hold could be captured, appended, committed and become a durable
local draft referencing bytes the store does not possess.

Both checks now exist, on the new-authoring path only:

- **S1b `admit_studio_frame_pixels`**, before the transient hold and before any detached work:
  the bytes must be present at the declared size, pass `validate_pix`, be 192x144, and are staged
  and promoted into the durable namespace, exactly as the ordinary Apply path does.
- **S3 `check_studio_frame_pixels`**, immediately before the I-3 holds and the intent barrier:
  the bytes must still be physically present at the declared size. External deletion or storage
  damage during the detached stage refuses here rather than producing a durable draft.

The capture carries the request's `(cid, declared bytes)` so the commit knows what to recheck;
possession itself is never carried across the detach. Acknowledgements and exact retries reach
none of this, so AG1-001 and I3-001 are preserved: the exact-retry path still returns before the
capture exists.

`studio_overlay_new_acceptance_requires_pixels_at_admission_and_again_before_the_barrier` covers
both controls. A never-published CID is refused before anything is accepted, with records unchanged
and no local draft. Then a valid capture is followed by physical removal of the bytes underneath
the detached job; every stamp and basis check still passes, so the refusal has to come from the
possession recheck, and the assertion requires that specific error rather than any failure.

| Check | Result |
|---|---|
| `... --lib studio_overlay_ -- --test-threads=1` | **43 passed, 0 failed, 2 ignored**, 1202.15 s. Log `logs/gate4-a1-fs001-fix.log`. |
| **M15**, removing only the S3 possession recheck | Fails at the design's named assertion, "a new acceptance named absent pixels"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

**Fixtures corrected, not weakened.** Enforcing possession broke two pre-existing frame tests,
`studio_overlay_store_reversed_hash_order_reconstructs_full_frame_history` and
`handoff::eligibility::studio_overlay_handoff_replays_dependency_order_and_retains_all_pixel_references`,
because both saved frame operations naming synthetic CIDs the vault never held, with arbitrary
declared sizes. That is precisely the gap FS-001 describes, encoded in fixtures. Both now publish
real 192x144 PIX through `published_pix` and assert pins against the real CIDs, so their subjects,
hash ordering and reference retention, are unchanged and their inputs are now realistic. No
assertion was removed or relaxed.

**What this does and does not cover.** The transfer and its ordering are real and mutation-proven.
Those particular tests prove the transfer on the composed path; N12(a) above now proves the
detached window itself.

### C1-TEST-002: N24 and the unconditional-replay mutation

The revision-4 reviewer found that nothing proved the structural decoder actually skips the replay.
Every branch built through `append` is replayable by construction, so the equivalence and
wrong-sequence tests would both keep passing if `out.read(ledger)?` ran unconditionally: valid
branches replay successfully and malformed ones fail before the replay. The central behavioural
claim of C-1 was unprotected.

`studio_overlay_structural_decode_accepts_a_branch_the_full_decoder_cannot_replay` closes it. The
branch names a sound-effect operation the typed writer does not support. `checked_entries` never
decodes an operation body, so the record is completely consistent structurally: the entry is in the
ledger, authored by the basis author, with the exact envelope hash, sequence 1 and canonical
re-encoding. The fixture obtains a canonical accepted single-entry encoding for a supported
operation, then retargets that entry's id and envelope, which occupy fixed-width fields at offsets
4 and 40 of the 88-byte entry, so the record stays canonical. It asserts the unsupported operation
is genuinely unacceptable through `append`, and that the unmodified canonical record still replays,
so the refusal comes from the retargeted entry and not from the encoding.

Required outcome, proven:

| Decoder | Result |
|---|---|
| `decode_vault_structural` | succeeds |
| `read` and `decode_vault` | refuse |

**The mutation is part of C-1's required evidence, not an ordinary regression.** Replacing
`if replay { out.read(ledger)?; }` with an unconditional `out.read(ledger)?;` makes N24 fail at its
structural-success assertion, while the other two structural tests **still pass** exactly as the
reviewer predicted. Byte-exact restoration confirmed by `git diff` reporting no change against
`d67e649`; all three then pass.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-replication --lib studio` | **107 passed, 0 failed**, 98.36 s. |
| Unconditional-replay mutation | N24 fails at its intended assertion; the other two pass, confirming they cannot cover this boundary. Restored source: 3 passed. |
| `cargo clippy -j 1 -p catcoms-replication --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

### R4-TEST-001: N25 and M9

`studio_replay_evidence_excludes_accepted_overlay_ids_and_keeps_ordinary_own_intents` asserts on
the **actual `ReplayEvidence.own` set** produced by the production path, with the control the
reviewer specified: an accepted overlay id must be absent while a comparable unannotated own
intent, authored by the same device and pending in the same ledger, is present. The test first
establishes that both entries are pending and that only one is annotated, so the assertion is about
selection rather than about the ledger's contents.

The fixture builds the real state rather than a synthetic one: a founded `Server` with its own
store, an installed source filled with real large signed operations to reach the production
rotation threshold, a real seal, a real accepted Closing-overlay entry through
`save_studio_closing_overlay`, the source warmed through the existing detached preparation, the
pristine successor installed, and then an ordinary Apply on that successor for the control.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib studio_replay -- (default threads)` | **10 passed, 0 failed**, 17.88 s. |
| M9 mutation, removing only `&& !intents.is_overlay(id)` | Fails at "an accepted overlay id reached ordinary replay selection"; `git diff` confirms byte-exact restoration; restored suite passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

One production file gained a **`#[cfg(test)]`** helper,
`ServerStore::install_sealed_studio_successor_for_test`. `rotate_studio_owner` refuses an
already-sealed source because it seals inside its own transaction, and the post-seal successor
install that the store's own rotation fixtures perform uses private items that `crate::studio`
cannot reach. The helper runs exactly that existing sequence. It is compiled out of production
builds, adds no callable surface and grants no authority; the alternative was widening real
production visibility for a test.

### Per-actor overlay admission (I-2)

`studio/overlay/admission.rs` implements the admission record and the ownership bundle.
`OverlayAdmission` holds only `Weak` handles and reaps dead owners, so there is no release
transition for any path to forget. `OverlayOwnership` binds the admission token, the shared
preparation permit and any job-owned reference hold, and releases all three together.

Three regressions cover I-2 by ending an owner's life a different way each time: ordinary
completion; a cancelled background waiter whose blocking closure still owns the bundle; a retained
result still holding a clone after the worker has gone; and an abandoned native preparation handle
dropped without any second visit, which also checks the preparation permit came back. A fourth
asserts the bundle's three pieces release together.

**M3**: weakening the reap so a dead owner is treated as live fails all three at their named
assertions, including "an abandoned native handle occupied admission permanently". Restored source
passes.

| Check | Result |
|---|---|
| `... --lib studio::overlay::admission` | **3 passed, 0 failed**. |
| M3 | All three fail at their intended assertions; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

**Marker to remove.** The module carries one `#[allow(dead_code)]` because its consumer is the
receiver's overlay runtime, which lands next and brings the job and result variants with it. The
seam is exercised by its own tests and exposes no callable surface; the annotation and its comment
must be deleted in that commit.

### FS-002: authorization before media admission, and `AdmittedOverlayMedia`

The FS-001 correction put media admission behind classification, which answers "has this request
already been accepted". It does not answer "may this request be authored now". A request whose
basis the document has legitimately moved past is neither an accepted retry nor authorized, but it
still reached S1b: it read a blob, could promote one into the durable namespace, and consulted the
job-owned hold rail, all on its way to being refused for a completely different reason. A stale
editor was told its artwork was missing.

`save_studio_closing_overlay_with_io` now runs the accepted order exactly:

```text
decode/bound request -> enter budget -> structural state read
-> completed retry -> exact retry -> ordinary collision
-> require live tenure -> read current Closing source -> derive fresh basis
-> compare fresh fingerprint with the request basis
-> ONLY NOW: validate and promote PIX, take the CreativeHold, capture, detach, commit
```

**A-1 is preserved.** Agent 2's invariant requires every acknowledgement, exact-retry and
Prepared-resolution branch to stay reachable before any tenure is required. The reorder moves
`tenure.ok_or_else` earlier, but still strictly after `completed_retry`, `exact_retry` and the
ordinary-collision check, all three of which return or refuse before it. Save's tenure remains a
parameter, so ordering is the only guard there; that ordering is now asserted by
`studio_overlay_exact_retry_is_acknowledged_without_media_admission` (retry ahead of media) and by
the new stale-basis regression (authorization ahead of media), and by N-T7b on Agent 2's side.

**P3 hardening: `AdmittedOverlayMedia`.** The staged capture previously took the operation, the
`CreativeHold` and the `(cid, bytes)` frame facts as three independent parameters, so a future
caller could pass a hold for one operation with the frame facts of another, or pass `None` for
either and silently disable the S3 recheck or N12(a)'s protection. Media is now one value with
private fields, mintable only by `ServerStore::admit_studio_overlay_media`, which derives the
frame reference from the operation itself, validates and promotes it, and takes the hold in the
same call. The capture and the plan carry it whole; the commit destructures it and releases the
hold after the write attempt, on success, error and unwind alike.

`studio_overlay_stale_basis_is_refused_before_any_media_admission` advances the Closing source
legitimately, by ingesting a separate sender's valid withheld edit, so the first basis becomes
stale without any fixture editing a gate field, snapshot or stamp. It then submits new authoring
against the stale basis under both hazards the reviewer named, each with a positive control on the
fresh basis proving media admission was genuinely reachable and would have refused:

| Hazard | Stale basis must report | Positive control on the fresh basis |
|---|---|---|
| The named pixels were never published | `Closing overlay basis changed` | `publish the frame PIX before saving its reference` |
| Every job-owned hold slot legitimately occupied | `Closing overlay basis changed` | the reference-rail refusal |

Each stale attempt also asserts the live transient-hold count is unchanged, the group's whole blob
namespace is byte-identical including staging, and the durable intent record is unchanged.

| Check | Result |
|---|---|
| `... --lib studio_overlay_stale_basis` | **1 passed, 0 failed**, 20.51 s. |
| `... --lib store::epoch_studio` | **104 passed, 0 failed, 3 ignored**, 489.95 s. |
| **M16**, moving media admission back above the basis comparison | Fails at the named assertion "a stale request was classified by its media instead of its basis", reporting `publish the frame PIX ...` where `Closing overlay basis changed` is required; restored source passes. |
| `cargo clippy -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |

**Scope limitation carried forward.** The reviewer's I-2 note stands: the admission seam passed on
its own tests, not end to end. Nothing here consumes `OverlayAdmission` yet.

### A-001: binding admitted media to the operation that consumes it

The reviewer accepted the FS-002 ordering and closed FS-002, but held that the claim attached to
`AdmittedOverlayMedia` was not yet true. Correct: private fields prevented a caller **fabricating**
frame facts or separating them from their hold, but `capture_studio_overlay_save` still took
`intent` and `media` as independent arguments with nothing tying them together. A crate-internal
caller could admit media for operation A and capture it against an intent carrying operation B: the
detached plan appends B while S3 rechecks, and the job-owned hold protects, A's pixels. That is a
durable acceptance naming pixels nothing verified, next to unprotected pixels for the ones it does
name. The synchronous adapter never did this; the point of the seam is that the receiver becomes a
second caller.

Taking the reviewer's second option, the combined value:

```rust
pub(crate) struct AdmittedOverlayAuthoring { intent: LocalIntent, media: AdmittedOverlayMedia }
```

`admit_studio_overlay_authoring(target, document, device, operation)` builds the `LocalIntent` and
admits its media in one call and is the only constructor, so the mismatch cannot be **expressed**.
Because a type-level fact executes nothing, the reviewer's third option is layered on top:
`AdmittedOverlayMedia` now carries a private `MediaOrigin { operation, target, document }`, capture
rechecks it against `intent.operation.id(&intent.author)` and its own derived target and document,
and the commit rechecks target and document again, because a plan is a value the receiver holds
across a detach and hands back.

`admitted_media_cannot_be_paired_with_another_operation` forces the mismatch through a
`#[cfg(test)]` `mismatched_for_test` constructor, requires the specific binding error rather than
any failure, asserts no durable record changed, and then captures, plans and commits the correctly
paired request as a positive control, ending with both holds released.

| Check | Result |
|---|---|
| `... --lib admitted_media_cannot_be_paired` | **1 passed, 0 failed**. |
| **M20**, dropping the origin comparison from capture and keeping only the author check | Fails at "media admitted for one operation was captured against another"; restored source passes. |

### B-001: the shared scope decoder's bound was Recovery's, not each family's

A P2 in the seam, and correct. `decode_record_scope` prechecked `scope.len() > 501` before the
family's domain check. That 501 is exactly Recovery's own maximum: its 31-byte domain plus 470
bytes of document framing at `LogicalDocument`'s limits of 256 and 192. The archive's domain is 36
bytes, so its maximum canonical scope is 506. A legal maximum-shape archive would have been
canonically scoped, correctly sealed, correctly named and under its family byte cap, and the
inventory would still have refused it, leaving a record that cannot reconcile into
`EpochIntentBudget` and that blocks any operation needing a complete scan. The claim that the
generic decoder worked "unmodified" was wrong at the boundary.

`EpochRecordKind::scope_cap()` now derives the bound per family from `domain().len()` plus the
shared document framing. It is documented as an **upper bound rather than each family's exact
maximum**, because Registry and Studio additionally pin their logical key to 32 and 16 bytes; being
loose there costs nothing, since `decode_record_scope` re-derives the canonical scope and compares
it byte for byte. Being tight is the failure mode, and that is what is now tested.

| Regression | What it proves |
|---|---|
| `no_family_can_encode_a_scope_its_own_bound_refuses` | For all six families, built at each family's own largest legal document: the real encoding is within its bound and decodes back to the same server and document; the four families with unconstrained keys reach the bound exactly; one byte past the bound is still refused, so the precheck still bounds pre-derivation work. |
| `a_maximum_shape_draft_archive_is_inventoried_rather_than_refused_by_a_scope_bound` | A 256/192 archive scope is 506 bytes, exceeds Recovery's bound, decodes under `DraftArchive`, is rejected under `Intents` and rejects the intent scope in return, and the scanner inventories the file and reconciles its bytes into `EpochIntentBudget`. |

| Check | Result |
|---|---|
| **M21**, restoring the hard-coded `501` | Both fail at named assertions, "DraftArchive refused its own maximal canonical scope" and "a maximum-shape archive scope was refused by a scope bound"; restored source passes. |
| `... --lib store::` | **295 passed, 0 failed, 8 ignored**, 545.64 s. |
| `cargo clippy -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |

### The overlay runtime: Flow S detached, and the one-algorithm split

Item 3 of the sequencing table. The store seams existed but nothing scheduled them, and
`studio/overlay/admission.rs` still carried a dead-code marker.

**One algorithm, enforced structurally.** `save_studio_closing_overlay_with_io` is now a thin
adapter over `start_studio_closing_overlay_with_io`, which performs S0, S1, the terminal S1a
acknowledgement, and for new authoring S1b and the capture, returning `StudioOverlayStart::Settled`
or `::Captured`. The synchronous path composes that with `plan` and the commit inline; the
scheduled path runs the same three with custody released around `plan`. A scheduler cannot drift
from a second copy of the classification, because there is no second copy. The adapter takes its
writer and sync as `FnMut` and lends a reborrow to each stage, so no existing caller changed.

**The job.** `StudioBackgroundJob::OverlayPlan(Box<StudioOverlayCapture>, OverlayOwnership,
OverlayContext)` and `StudioBackgroundResult::OverlayPlanned`. The capture moves in **whole**, which
is the condition the A-001 reviewer attached: nothing decomposes `AdmittedOverlayAuthoring` or
rebuilds its contents. The worker is `spawn_blocking`, matching the existing `Prepare` arm.

**Ownership through cancellation.** `OverlayOwnership` moves into the blocking closure, so the
`tokio::select!` cancellation branch structurally cannot take it back: `CancelledOverlay` carries no
ownership and clears the waiter's bookkeeping only. A cancelled waiter therefore leaves admission
and the shared slot occupied until the worker ends by itself, with no release message from it.
A parked result keeps them too, because its pixels are still protected only by its transient hold.

**Design deviation, stated for review.** Design 5.5 gave `OverlayOwnership` three members; the
third, `Option<CreativeHold>`, is removed. A-001 made `AdmittedOverlayMedia` the single owner of
that hold, minted with the frame facts it protects and carried inside the capture and the plan. A
second `Option<CreativeHold>` here would be a competing owner and a way to hold pixels without the
facts S3 rechecks, which is exactly what A-001 closed. The three still release together in practice,
because a job owns both the bundle and its capture.

**Scheduling.** Overlay has its own per-actor slot rather than sharing `preparing` with source
preparation, so a local Save is not blocked behind catch-up reconstruction for the lifetime of an
actor or the reverse. It is selected ahead of source preparation in `detach`: it holds no network
resource and its permit is already reserved. 7.2's reserve-before-first-read is
`CatchupRuntime::reserve_overlay`, and a refusal releases by dropping, with no bookkeeping to unwind.

| Regression | What it proves |
|---|---|
| `overlay_reservation_shares_the_one_preparation_pool` | N15's central claim, by pointer identity: production draws from the one process-wide `preparation_pool()`, so there is **no overlay-only pool**. |
| `a_full_preparation_pool_refuses_an_overlay_reservation_and_recovers` | A full shared pool is a retryable refusal that consumes no admission, and a freed slot admits the identical retry. |
| `a_cancelled_overlay_waiter_does_not_free_a_still_running_worker_slot` | I-2 and 7.1 through the runtime: one job per actor, and a cancelled waiter frees neither admission nor the slot while the worker still owns the bundle. Both return when the worker ends, with no second visit. |
| `a_queued_or_parked_overlay_keeps_the_actor_busy` | A queued capture and a parked result each keep the actor busy, so a plan awaiting commit cannot be displaced. |

| Check | Result |
|---|---|
| `... --lib` the four above | **4 passed, 0 failed**. |
| **M22**, minting admission directly instead of through `OverlayAdmission::admit` | Fails at "a second overlay job was admitted for the same actor"; restored source passes. |
| **M23**, making `can_admit` clear owners instead of reaping live ones | Fails the runtime test at the same named assertion **and** both seam tests at theirs, proving the runtime property depends on weak-handle reaping rather than on the local waiter flag; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |
| `cargo test -j 1 -p catcoms-app --lib`, no concurrent Cargo work | **637 passed, 0 failed, 11 ignored**, 1144.46 s. Up from 630 at `70eaad4` by the seven tests added since, with no `studio_exchange` failures. |

**What is not done, and why.** N14(b) and N14(c) need a native `PrepareOverlaySave` handle, which
cannot exist while Agent 2's P5 is false. `StudioReceiver::save_overlay` therefore has no production
caller yet and carries one `#[allow(dead_code)]` marker naming its consumer. Its `close` and
`budget` are parameters exactly as on the existing explicit `Server::save_studio_closing_overlay`:
acquiring them means the saved close from the owner journal and a completed five-family inventory,
and deciding when to pay for both is the manual lifecycle Agent 2 owns. A `StudioControlAction`
variant was drafted and **withdrawn** rather than invent that shape unilaterally. No native command
is registered; `studio_overlay_read` remains the only overlay command in `invoke_handler`.

### RT-001 and RT-002: two runtime defects in the Flow S scheduling

Both found by the `dda64a9` review, both real, and neither depends on Agent 2 or P5.

**RT-001 (P2): a refused plan occupied a global slot until some later visit, or forever.** The
result type carried `OverlayOwnership` on the error arm as well as the success arm, and `complete`
parked either. But a refused plan can never be committed, and its media hold has already died with
the capture, so the parked bundle protected nothing while holding this actor's admission and one of
the four process-wide preparation slots. `reserve_overlay` refuses while `overlay_planned.is_some()`,
so it was cleared only if some later Save for the same target happened to collect it. Four such
actors could starve unrelated source and registry preparation indefinitely.

The trigger is not contrived: C-1 deliberately lets a structurally valid branch pass the metadata
read and fail full typed reconstruction later, which is precisely a capture that plans and fails.

`OverlayPlanResult` is now `Result<(Box<StudioOverlayPlan>, OverlayOwnership), AppError>` as the
design always specified, and the worker drops the ownership the moment planning fails. A refusal
parks nothing.

**RT-002 (P2): S2 overtook authoritative catch-up.** `detach` selected `OverlayPlan` first, with no
`replay_ready()` check. Design 7.3 places heavy stages behind that gate, and L7 accepts that overlay
work may starve under sustained catch-up: it never accepted catch-up starving under sustained local
Saves, which is what the selection order actually did. `OverlayPlan` is now chosen only when
`replay_ready()` holds, after source and registry preparation, network passes and discovery.

The gate is in `detach`, not `reserve_overlay`, and that choice is deliberate: 7.2 requires the
reservation before the first bounded read, and gating it would also defer the terminal S1a
acknowledgement, which reads no source, mints no basis, touches no blob and is not a heavy stage.
Only the detached reconstruction yields to catch-up.

| Regression | What it proves |
|---|---|
| `a_refused_plan_releases_admission_and_its_pool_slot_without_a_second_visit` | A **real** planning refusal, through `run()` and the ordinary `complete()` path, with **no** second Save visit: nothing is parked, admission is immediately available, the pool permit has returned, and the actor can start new overlay work at once. The refusal is asserted to be a real `Err` before anything else is checked, so a fixture that silently succeeded could not satisfy it. |
| `a_queued_overlay_waits_behind_authoritative_catch_up` | With a real source preparation parked and a real capture queued, the next detached turn selects `Prepare`, the overlay capture survives that turn unconsumed, and the overlay becomes selectable only once catch-up clears. |

| Check | Result |
|---|---|
| **M24**, leaking the ownership instead of releasing it on a failed plan | Fails at "a refused plan kept this actor's admission"; restored source passes. |
| **M25**, restoring `OverlayPlan` to the front of `detach` | Fails at "an overlay plan overtook parked authoritative source preparation"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |
| `cargo test -j 1 -p catcoms-app --lib`, no concurrent Cargo work | **641 passed, 0 failed, 11 ignored**, 1144.46 s. |

**No fabricated capture, with one precise limit.** Both regressions use
`studio_closing_capture_fixture`, and the **capture itself** is genuinely produced end to end:
source, fill to rotation eligibility, owner decision, seal, basis, then
`start_studio_closing_overlay` returning `Captured`.

The pre-filled ledger is a different matter and should not be overstated. Its 64 operations are
prepared through the production `IntentLedger::prepare` and written through the production
`EpochIntentState::encode`, `seal` and framing at the canonical path, so the record the reader
authenticates is real. But they are **synthesized at the durable-record level**: an ordinary Studio
Apply typed-decodes and validates each operation before persisting its intent, and this fixture does
not go through that. The reviewer made this correction and did not treat it as a finding, because
RT-001 is an ownership invariant for **any** real `plan()` error and the fixture reaches the real
capture and the real `plan()` code rather than injecting an error.

**Design text corrected, as the reviewer directed.** Design 5.5 now carries the revised
`OverlayOwnership` and states the invariant that replaced it: the bundle is admission plus permit,
the capture and plan hold the sole `CreativeHold`, and **the three do not always release together**.
Two code comments that claimed they did are corrected; that claim is what RT-001 was.

### N14(a): the real cancellation race

Closed by executing it rather than modelling it. The reviewer asked for this **before** Flow H, and
the reason is right: Flow H modifies the exact machinery N14(a) protects, so pinning the race first
means a later failure is unambiguous about which change caused it.

`StudioBackgroundJob::pause_overlay_for_test` arms a barrier in the shape
`PreviewJob::pause_for_test` already established: the blocking worker signals once it has entered
and then blocks until released. The barrier is taken out of `OverlayContext` before the closure is
built, so the pause happens **after** `OverlayOwnership` has moved inside the worker, which is the
state the invariant is about.

`a_cancelled_waiter_leaves_a_real_paused_worker_holding_admission_and_its_slot` then runs the real
job: a genuine capture, a real `RequestCancellation` built from a watch channel and fired while the
worker is paused, `run` returning `CancelledOverlay`, and the ordinary `complete` path clearing the
waiter flag. While the worker is still paused it requires that a second overlay job is refused and
that the shared pool is still one permit down; after release it requires both to recover, with no
release message sent from anywhere.

**Why it is not vacuous.** If the barrier were a no-op the worker would finish and `run` would
return `OverlayPlanned`, failing the cancellation assertion. If the worker never entered, the entry
signal would never arrive and the test would hang rather than pass. And the permit assertion is the
anti-vacuity guard for the admission one: a worker that had already finished would have returned its
slot, so `free - 1` would fail.

| Check | Result |
|---|---|
| `... --lib a_cancelled_waiter_leaves_a_real_paused_worker` | **1 passed, 0 failed**, 9.18 s. |
| **M26**, clearing the admission record when a cancelled waiter is completed | Fails at "a cancelled waiter admitted a second overlay job while its worker was still running"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |
| `cargo test -j 1 -p catcoms-app --lib`, no concurrent Cargo work | **644 passed, 0 failed, 11 ignored**, 1027.03 s. |

**A build failure that was not one.** The first attempt to link this test failed with
`LNK1104: cannot open file ...catcoms_app-<hash>.exe`. A process listing showed another agent's
session running that exact test binary out of the shared `target/` directory. The fix was to wait
for their run to end, not to change any code. Worth recognising, because it presents as a compiler
error. Relatedly, `cargo fmt -p catcoms-app` reformats the whole package, including other agents'
uncommitted files: it touched Agent 2's `epoch_draft_archive.rs` here. That reformat is left in the
tree deliberately, because reverting it risks their in-progress work, and it is not committed here.

**Still open.** N14(b) and (c) remain blocked on the native `PrepareOverlaySave` handle and Agent
2's P5, as the reviewer agrees. They are not worth pursuing before that exists.

### Flow H, stages H1 and H2

The custody boundary question was put to the reviewer before building, because getting it wrong
would have put live MLS state in a detached worker. Their resolution, verified against source
before use: `StudioEpoch::restore` reduces immediately to
`restore_scoped(bytes, &group.group_id(), target, actor, owner(group)?)`, and
`prepare_vault_source` has an identical body with a capability-narrowed signature. So the answer is
"the restore belongs in H2" **and** "no `ServerGroup` crosses the boundary" — design 6.1's stage
table needed no correction.

`StudioSourceCapture::rebuild`, the existing detached source preparation, already has exactly this
shape: cheap framing extraction, then `prepare_vault_source` with captured public context. Flow H
follows it rather than inventing a second pattern.

| Stage | Where | What |
|---|---|---|
| H1 `start_studio_handoff_with_io` | custody | classification, interrupted-Prepared resolution, the Index reference check, and the live-authority mint. Bounded authenticated reads only; no reconstruction, no restore. |
| H2 `StudioHandoffCapture::prepare` | detached | full `decode_vault`, private successor reconstruction through `prepare_vault_source`, and `prepare_handoff_detached`. |

**Nothing live crosses the boundary.** The worker receives the group id, the designated-owner
`DeviceId`, the target and the actor. No `ServerGroup`, `MlsDevice`, MLS secret, store handle or
writer. `copy_handoff_source(group)` is deliberately not called from this path; it is the
synchronous compatibility helper, and using it would move live membership state into a worker for
no reason.

**Capturing that context is not continuing authority.** `sign_next` rechecks device, membership,
MLS epoch, observed tenure and the current-owner receipt before every single signature, so a
change during H2 leaves at worst a stale proposal that H3 refuses to sign. Independently,
`prepare_handoff_detached` checks H2's freshly decoded metadata against the authority H1 minted, so
a record that moved under the capture refuses before any signature exists.

**One algorithm.** The synchronous adapter now composes H1, H2 and H3-H5, so there is one
classification and one authorization rather than two that can drift. The durable half is unchanged.

| Check | Result |
|---|---|
| `... --lib handoff` | **36 passed, 0 failed, 2 ignored**, 139.98 s, including the crash-barrier matrix and the fences. |
| `... --lib store::epoch_studio` | **112 passed, 0 failed, 3 ignored**, 443.05 s. Against ~478-490 s for 103-104 tests earlier in this session, so the split is not a regression. |
| `... --lib` (full) | **647 passed, 0 failed, 11 ignored**. Its 20,639 s wall clock is **not** usable evidence: the machine was hibernated mid-run, and the elapsed timer kept counting while it slept. The two subsets above were measured end to end on a waking machine and are the timing evidence. |

**A measurement that looked like a regression and was not.** That 20,639 s is roughly 20x every
previous run of the same suite, which is exactly what a real performance defect would look like.
Re-measuring the two subsets settled it before any conclusion was drawn: `handoff` at 139.98 s
against 145.78 s for the same code, and `store::epoch_studio` at 443.05 s for 112 tests against
478-490 s for 103-104 tests earlier, which is faster with more tests. The cause was hibernation,
not contention as first supposed and not this change. Recorded because a bare 20,639 s in a log is
otherwise a booby trap for whoever reads it next.
| **M27**, deleting the H5 stamp recheck | Fails at "a handoff plan built from superseded records was committed"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |

**A guard that would have shipped unproven.** The H5 stamp recheck had no test: the synchronous
adapter never leaves a gap, so every existing handoff test passes with the check deleted.
`studio_overlay_handoff_plan_is_refused_when_its_records_changed` runs H1, then H2, then closes and
reopens the vault, which is what a crash between H2 and H5 looks like, and requires the commit to
refuse. M27 shows that without the check a plan built against a dead mount commits successfully.

### Flow H, stage H3: the signing slice

`StudioHandoffPlan::sign_slice` is the one signing loop. The synchronous transaction calls it with
no turn cap, no deadline and nothing to yield to, because it holds custody throughout; the
scheduled runtime will call the same function with the injected clock, a slice budget and the
priority answer. There is deliberately no second loop for a scheduler to drift from.

`SigningSlice` records `remaining` at entry and at exit. The core decrements that counter by
exactly one per successful `sign_next`, so the difference is a count of signatures **produced**,
never an inference from "work remains" — which is the AG1-TEST-001 correction, and the reviewer's
hard condition for this stage:

| Outcome | Condition |
|---|---|
| priority yield | `signed == 0`, `remaining` unchanged, and reported as a yield in its own right |
| count or time slice | `0 < signed < before` and `remaining > 0` |
| completion | `remaining == 0` |

The deadline is checked **between** signatures, never inside one, so a slice may overrun by one
whole operation including its authority checks. `MAX_SIGNING_TURNS_PER_VISIT = 32` and
`SIGNING_SLICE_BUDGET_MS = 250` remain an experiment configuration, not a responsiveness
guarantee, until design 13's measurement 3 exists.

`studio_overlay_handoff_signing_slice_reports_yield_bound_and_completion_apart` (N31) walks all
four outcomes on a real 40-operation branch through the real H1 and H2, and asserts that no
signature became durable at any point. Each limiter is exercised with **the other one disabled**,
so neither can stand in for it: the count case passes no deadline at all, and the time case passes
`usize::MAX` turns against a clock that advances 200 ms per read into a 250 ms budget, which
crosses the deadline by construction rather than by hoping operations fall either side of a
wall-clock threshold.

| Check | Result |
|---|---|
| `... --lib handoff` | **37 passed, 0 failed, 2 ignored**, 132.89 s. |
| **M5a**, disabling only the turn cap while time stays below budget | Fails at "the turn cap did not bound the slice", 40 signed where 32 was required. |
| **M5b**, disabling only the deadline while the turn cap stays below its limit | Fails at **a different** assertion, "the slice budget did not bound the slice", 8 signed where 2 was required. |
| **M28**, removing the priority early return | Fails at "a priority yield was not reported as one". |
| `cargo test -j 1 -p catcoms-app --lib`, no concurrent Cargo work | **648 passed, 0 failed, 11 ignored**, 1070.66 s. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |

That 1070.66 s also settles the earlier 20,639 s independently: same machine, same suite, one more
test, back in the normal band. The cause was hibernation, exactly as the operator said.

M5a and M5b failing at different named assertions is the point: it shows each limiter is doing its
own work and that neither one, nor bare "work remains", is standing in for the other.

### Flow H, stage H4: detached assembly

`StudioHandoffPlan::assemble` is the second detached stage. It runs `finish`, which revalidates the
complete signed history and typed projection and builds the manifest; `complete`, which derives the
Completed overlay; `snapshot`, which serializes the candidate source; and the two intent-record
encodings that size the transaction. All of that is expensive and none of it touches the store.

**How H4 can encode without the store.** H2 already decodes the intent record, so the plan carries
that `EpochIntentState` forward. The stamp is what makes this sound rather than a cached guess:
H5 requires the intent record to be byte-identical to the one H2 read, so the carried state is
either still the current state or the plan is refused before anything durable happens. The
reference check still runs against the state **as H2 read it**, before the prepared overlay is
installed, exactly as it did when H4 and H5 were one function.

`assemble` refuses a batch that has not finished signing. `finish` would fail anyway, but somewhere
inside manifest construction; refusing up front names the actual mistake, which is a scheduled
caller assembling a plan it has only partly signed.

| Check | Result |
|---|---|
| `... --lib handoff` | **38 passed, 0 failed, 2 ignored**, 275.02 s. |
| `studio_overlay_handoff_assembly_refuses_a_partly_signed_batch` | Signs a bounded 2-of-4 slice, then requires assembly to refuse with that specific error. |
| `studio_overlay_handoff_plan_is_refused_when_its_records_changed` | Now runs H1 to H4 and asserts none of them wrote anything durable, before the reopened-vault refusal. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |

### Flow H, the scheduled runtime: H1 to H6

`studio/receiver/handoff.rs` carries the stage machine, the H1 eligibility probe, the H3 slice
visit and H5/H6. `StudioBackgroundJob::HandoffPrepare` and `::HandoffAssemble` run H2 and H4 on a
worker with the same ownership discipline as `OverlayPlan`: the bundle moves into the closure, a
cancelled waiter cannot reclaim it, and a refusal releases it there (RT-001's rule).

A handoff shares Flow S's per-actor admission rather than having its own. "One overlay operation is
live per server at a time" is a property of the actor, so a Save and a transfer compete for one
slot instead of doubling per-actor concurrency.

**Reviewed adversarially before commit, and it found more than the tests did.** An independent
review of the whole Flow H diff returned two P1s, six P2s and seven P3s. What follows is what was
wrong and what changed; the review's central charge was that the passing end-to-end test would have
survived deletion of essentially every guard the work claimed to establish, and it was right.

| Finding | What was wrong | Correction |
|---|---|---|
| **P1** | A routine MLS epoch advance during H3 made `sign_next` refuse, that error propagated through `background_step`, and `run` set the receiver's storage pause. Catch-up, replay and receive all stop until the user next opens a Studio document, **and** the job stays parked holding this actor's admission and one of four process-wide permits, with no path that releases either. | Every Flow H stage now absorbs expected refusals: abandon the job, release the bundle, back that target off, return `Ok`. Only genuine storage errors reach the receiver, and no Flow H stage returns `Result` to it any more. |
| **P1** | `Signing` was a terminal trap. The code commented "the next visit with a live tenure resumes it", which is false: the plan's authority pins the MLS epoch, so once it moved the job could never be signed again. Nothing anywhere abandoned a `Signing` job. | `handoff_check_authority` runs as a lifecycle step on every turn, at any stage, and abandons a job whose authority has moved. Placed there rather than in the scheduler so releasing a dead job cannot depend on which branch a turn takes. **This fix was wrong on its first attempt; see the second review below.** |
| **P2** | The tenure check was **lost** in the split. The single visit passed tenure into `prepare_handoff`; `commit_studio_handoff_with_io` took no tenure at all, so a tenure restart for the same owner at the same MLS epoch passed every conjunct. A batch signed under one tenure could become durable under the next. | `tenure` is part of `StudioHandoffStamp` and compared at H5. |
| **P2** | The Index reference check ran only at H1. The stamp covers the Index document's own records, so the referenced Flipnote's source could be evicted, retired or cleaned up during H2 to H4 and H5 would still commit, leaving a durable Index entry pointing at nothing. | `check_index_object_sources` runs at both H1 and H5. |
| **P2** | **RT-002 again.** The handoff detach arm was placed ahead of all catch-up, with a comment arguing that an already-reserved permit earned it. That is the same inversion RT-002 was opened for. H5 and H1 had no `replay_ready()` gate at all. | H2, H4, H5 and H1 are behind `replay_ready()`; the detach arm sits with the Flow S plan, behind catch-up. H3 stays ungated, which is 7.3's documented exception. |
| **P2** | Scalar backoff and no round-robin cursor, so one permanently ineligible document would be selected every turn, fail, drive the shared hold to 300 s and starve every other target for the life of the actor. | Per-target `next_at`/`hold_ms` maps and a selection cursor, as design 5.5 specifies. |
| **P2** | `pending()` ignored handoff entirely, so the driver could stop scheduling turns with a job parked and nothing would ever recover its admission or permit. | `self.handoff.busy()` is a wake condition. |
| **P2** | `handoff_complete`'s catch-all conflated a refusal, a mis-targeted completion and a cancellation, and cleared the **current** job for all three. A completion for a target the actor had moved on from would destroy a live job mid-signing. | Four arms. A mis-targeted completion is ignored outright. |
| **P3** | The probe read the whole rail before reserving, and a failed reservation set no backoff, so it re-read every turn while a Save held admission. A read error was also memoised as "nothing here", suppressing all handoff until an unrelated write rotated the generation. | Reservation failure backs off; only targets actually read and found empty are memoised; a read error backs off instead. |
| **P3** | `sign_slice` was re-entered on a fully signed plan, rechecking live authority for no signature. | Early return at `remaining() == 0`. |

**A bug of my own, found while fixing those.** `abandon` cleared the job *before* `hold` read it,
so no backoff was recorded and the probe restarted the same job on the very next turn: a release
that was actually a hot loop. The new P1 regression caught it.

**The tests the review said proved nothing.** Its sharpest point was that the end-to-end test used
a one-operation branch, so H3 finished in a single slice and could not discriminate the turn cap,
the two-event rule or the priority gate. The fixture now takes an operation count.

| Regression | What it proves |
|---|---|
| `studio_handoff_runs_through_every_scheduled_stage_and_notifies` | The whole flow on a real vault, asserting on **what actually detached** (`handoff-prepare`, `handoff-assemble`) rather than on a transient field, so an inline implementation fails it. |
| `handoff_signing_pages_across_turns_and_a_priority_turn_signs_nothing` | On a 37-operation branch: a priority turn signs **zero** and leaves the count untouched; an ordinary slice stops at exactly the turn cap with work left; a second slice finishes it; nothing is durable throughout. |
| `an_authority_change_during_signing_abandons_the_job_without_pausing_the_receiver` | Both P1s: the receiver is not paused, the job is gone, the shared slot is returned, admission is available, and the next turn still runs. |

| Check | Result |
|---|---|
| `... --lib handoff` | **40 passed, 0 failed, 2 ignored**, 152.18 s. |
| **M29**, making H4's detach condition unreachable | The transfer stalls after `handoff-prepare`; fails at the completion assertion. |
| **M30**, removing `handoff_check_authority` | Fails at "the unsignable job was left parked". |
| **M31**, removing the turn cap from the scheduled slice | Fails at "the slice was not bounded by the turn cap", 37 signed where 32 was required. |
| `cargo clippy -j 1 -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |

**Permit assertions no longer race.** Four tests measured `available_permits()` on the **global**
pool, so they contended with every other test in the process; two of them failed in opposite
directions when run together. All four now inject a private pool through the seam that already
existed for this.

### The second review: two of my own fixes were wrong

A second independent review of `808ef2f` checked whether the ten corrections above were real. Six
were. **Three were partial and one did not cover its own named trigger**, and it found two further
P1s, both of which are the same defects this work was written to close, reintroduced by the fixes.
That is the useful lesson of this pass: a fix written under pressure is a defect site, and the
first review's approval of a *description* is not approval of the code.

**NEW-1, P1: the authority check was blind to the failure it was named after.**
`observed_owner_tenure_start` reports when the current owner's tenure *began*, not the current
epoch. `OwnerTenure::applied` takes its "same owner preserves knowledge" branch on a same-owner
commit, so the value is **unchanged** — and a same-owner MLS commit is the commonest way a job
dies. My status entry above asserted the opposite as fact. The fallback that was supposed to catch
it, `sign_next` refusing, never runs on a priority turn, because `sign_slice` yields before it
signs. So the P1 was reported closed while its main trigger was still open.

Corrected: `HandoffJob` now records the MLS epoch as well as the tenure, and
`handoff_check_authority` compares **both**. `an_mls_commit_during_signing_...` covers the case;
**M32**, reverting to the tenure-only comparison, fails **only** that test while the owner-change
case still passes, which is precisely the shape of the blind spot.

**NEW-2, P1: I found this class of bug, fixed one of four instances, and wrote it up as closed.**
`HandoffRuntime::hold` read `self.job`, and all three of its remaining call sites had already
removed the job, so every H5 refusal recorded **no backoff at all**. H5 has refusals H1 does not —
the three-write preflights and the reference check — so a vault without headroom would replay the
whole pipeline every turn: two inventory drains, a full detached decode, every signature re-signed,
then fail identically, holding admission for most of each cycle. Strictly worse than the original,
because H5 is the expensive end.

Corrected by deleting `hold` entirely. There is now only `hold_target`, which takes the target
explicitly, with a comment at the site saying why the job-reading variant must not come back.

| Also corrected | Was |
|---|---|
| **NEW-3**, P2 | `handoff_commit` took the job *before* building the budget, so a transient inventory or generation failure discarded H1 to H4 entirely: a detached full vault decode plus every signature, thrown away for a retryable error. The budget is now built first. |
| **NEW-4**, P2 | The probe's read-error backoff held `rail[start]`, not the target that failed, so the bad record was re-read every turn while an innocent document was penalised — and because the cursor advances, one bad record walked the whole rail to the 300 s cap. |
| **NEW-5**, P3 | `HandoffCompletion::Cancelled` carried no target, so its arm cleared whatever job was live. All three arms are now gated. |
| **NEW-10**, P3 | Completions were routed by target. Since `handoff_check_authority` abandons at any stage including `Detached`, a worker outlives its job, and after that target's backoff expires a new job for the same target is legitimate — so the dead worker's result landed on it. Every completion now carries a never-reused `HandoffJob::token` and is matched on that. |
| **NEW-8**, P3 | `pending()` used `busy()`, which is true for a job that cannot run — held by backoff or already detached — holding the driver at its active cadence for the job's whole life. Now `runnable(now)`. |
| **NEW-9**, P3 | A `Captured` job survived the storage pause holding a process-wide permit with no path to release it. `release_if_stalled` now abandons any non-detached job when the receiver pauses. |

**Still open, corrected and completed after the second review pointed out the first list was
incomplete.** The earlier list named three items; it should have named eight.

1. **H1 and H5 each drain a full epoch-storage inventory under custody**, which design 6.1 does not
   put in H1 and which C-3's resumable cursor is meant to bound. The scheduled sequence is shipping
   ahead of the mechanism intended to bound its custody.
2. **The new H5 index check adds its own unbounded under-custody read loop**, one `load_studio_epoch`
   per `PutObject`, and belongs with item 1.
3. **`handoff_priority` omits `studio_has_page_request`**, which `run` itself treats as
   authoritative.
4. **A detached Flow H waiter inherits an unrelated request's cancellation**, discarding multi-turn
   signing progress. One type change away from NEW-5's shape.
5. **Design 7.3's `explicit_retry` relief is not wired to the handoff maps**, though the code
   comment cites 7.3's pacing as satisfied.
6. **`next_at`, `hold_ms` and `quiet` are never pruned** against the current watch rail.

The two coverage debts that were items 3 and 4 in the previous revision are now closed, both by
tests that were checked against a deliberately broken build before being trusted:

| Guard | Test | Mutation |
|---|---|---|
| H5 index reference recheck | `studio_overlay_handoff_rechecks_index_object_sources_at_commit_not_only_at_capture` | **M34**: delete the H5 call site. Without it the commit **succeeds**, durably writing an Index entry (`epoch: 1, accepted: 1`) pointing at a source that is no longer there. H1 is asserted to have succeeded, so the test can only be passing because of the H5 call. |
| H5 tenure conjunct | `studio_overlay_handoff_refuses_a_batch_signed_under_a_superseded_tenure` | **M35**: drop `tenure != Some(stamp.tenure)` from `studio_handoff_is_current`. The batch commits under the superseded tenure. |

Both mutated files were confirmed byte-identical to `HEAD` afterwards, and both tests pass on the
restored tree.

### `EpochRecordKind::DraftArchive`, the shared enum seam for Agent 2

Not Agent 1 work. Agent 2's design adds a sixth variant to a closed shared enum, so every match
over it changes in files Agent 1 and Agent 3 are editing concurrently. Their reviewer directed that
it land as an isolated integration commit ahead of Agent 2's implementation, and Agent 2 asked for
it here because this tree is furthest along. Agent 2 then chose **option (a): land the seam without
the mutation guard, and do not pull I-4 forward.**

Built: the variant; `.draft-archive`; the `catcoms/epoch-draft-archive-store/v1` domain;
`epoch_draft_archive::scope_bytes` in the same shape as the intent scope so `decode_record_scope`'s
domain check and canonical re-derivation work unmodified; `MAX_DRAFT_ARCHIVE_SEALED_BYTES` and its
inputs; `epoch_draft_archive_path`; a bounded authenticated reader mirroring
`read_epoch_intent_plain`; `storage_name` gated by `includes_intents()`; final and temporary
recognition through the unchanged `record_name`; a distinct inventory key; a separate
`draft_archive_records` counter; and Intents-class accounting through a new
`EpochRecordKind::intent_class()`, used by both `storage_name`'s gate and
`EpochIntentBudget::from_inventory`.

Not built, by instruction: the payload schema and serializer, the writer, the release path, the
disposal transaction, the reference collector, the 16 MiB archive sub-cap, and anything that
decodes archive contents.

**The scan rails did not move.** The archive's sealed cap is 6,328,360 bytes, about 6 MiB + 35 KiB,
deliberately larger than the intent record's 5 MiB + 1024. Recovery remains the largest family at
`MAX_RECOVERY_SLOTS_BYTES + 1024 + 40` = 18 MiB + 2088, and `MAX_AUTHENTICATED_BYTES` already
derives from recovery's cap, so `ENTRIES_PER_STEP`, the one-body-per-step rail and the byte rail
are untouched.

**The correctness condition Agent 2 asked to have preserved and stated.** Coverage gating by
`includes_intents()` means `RecoveryOnly` and `RecoveryAndOwnerReceipts` do not see archive files.
That is safe only while no coverage narrower than the full five-family scan installs a
deletion-protection set. That rule is unchanged, and it is stronger than "the only caller uses full
coverage": `EpochStorageScan::collect_creative_references` **refuses** to become a reference scan
unless `coverage() == RecoveryOwnerReceiptsIntentsRegistryAndStudio` and no entry has been visited
yet, and `finish_creative_references` is the only path to `Protection::install`. A narrower scan
therefore cannot install protection at all, rather than merely not doing so today.

**One addition beyond the requested list, flagged for Agent 2.** A reference scan that meets an
archive would otherwise collect nothing from it and install a complete, "known" protection set with
the archive's CIDs missing, which is exactly the silent reclamation the coverage condition exists to
prevent. The reference arm therefore fails closed with "draft archive reference collection is not
implemented" until Agent 2's collector replaces it. M19 below shows this is not theoretical: with
the guard removed the scan returns `Ok(CreativeReferences { count: 0 })`.

| Regression | What it proves |
|---|---|
| `draft_archive_is_its_own_physical_family_sharing_the_intent_accounting_class` | A vault with no archive file is unchanged, record for record and byte for byte, with the new counter at zero. Then: the archive is inventoried with its own key and content footprint; the pre-existing records are byte-identical; it charges bytes and a record slot to `EpochIntentBudget` under `MAX_VAULT_INTENT_BYTES`; a coverage excluding intents excludes it; final and temporary names are recognised and noncanonical spellings refused; archive and intent scopes reject each other's domains; the addressed reader reaches the file and an intent scope addresses nothing in it; and a reference scan fails closed, with a positive control proving the same isolated vault completed one before the archive existed. |
| `a_draft_archive_coexists_with_the_same_document_intent_ledger_in_one_accounting_class` | Against a **real** intent ledger written by the production writer: both records exist for one logical document, share a `document` id, hold different record ids, do not displace each other, and sum in one budget. A temporary archive sibling is attributed to the archive family and charged. Cleanup at intent coverage leaves both finals byte-identical. |

| Check | Result |
|---|---|
| `... --lib draft_archive` | **2 passed, 0 failed**. |
| `... --lib store::` | **292 passed, 0 failed, 8 ignored**, 474.61 s. |
| **M17**, reverting the coverage gate to `family == Intents` | Both tests fail at "a coverage that excludes intents inventoried an archive"; restored source passes. |
| **M18**, reverting `from_inventory` to `e.kind == Intents` | Both fail at "an archive's bytes were not charged to the intent accounting class"; restored source passes. |
| **M19**, removing the fail-closed reference arm | Fails at "a reference scan installed a protection set for a vault holding an archive it cannot read", showing the scan returns a complete set of count 0; restored source passes. |
| `cargo clippy -p catcoms-app --all-targets -- -D warnings`, `cargo fmt --all --check` | Clean. |
| `cargo test -j 1 -p catcoms-app -- --test-threads=4`, at `705d44b` with no concurrent Cargo work | **630 passed, 0 failed, 11 ignored** in the lib, 1177.50 s, and 20 passed across the six integration binaries. This covers both `5a024a7` and `705d44b`. |

**A masked assertion I found and corrected, recorded because the reviewer has caught this class
before.** The fail-closed reference claim was first asserted as `creative_pinned_cids().is_err()`
in the main fixture. Adding the positive control showed that vault's reference scan already failed
for an unrelated reason, so the assertion could not discriminate. The claim moved to an isolated
vault holding nothing but the archive, where the control proves the same vault completed a
reference scan before the archive existed and the refusal names the archive specifically.

**A contention result, not a regression.** An earlier full-suite run reported three failures in
`studio_exchange` (`scheduling::studio_actor_owner_return_installs_both_classes_with_cancelled_preview_transport`,
`..._with_three_retained_previews`, `succession::joining::studio_actor_post_succession_joiner_reads_open_history_provisionally`),
all deadline assertions in actor scheduling. Foreground Cargo builds and tests were running
concurrently with it, which this machine's serial-Cargo discipline forbids. All four pass serially,
and the clean run above passes with no failure. Nothing in this scope touches `studio_exchange`.

**One test-only fixture.** `write_draft_archive_for_test` seals and frames an archive at its
canonical path, because there is deliberately no production writer to call. It bypasses nothing the
seam validates: the scope, sealing, framing and path are the production ones, so filename grammar,
filename-to-scope agreement, the bound, authentication, the domain check and canonical scope
re-derivation are all exercised for real; only the opaque body is synthetic, which is precisely what
this family does not interpret. It is `#[cfg(test)]` and `pub(in crate::store)`, and must be deleted
when Agent 2's writer lands.

I-4's participant list in design 9.2 now names `write_studio_draft_archive_with_io` and
`release_studio_draft_archive_with_io`, so the audit obligation attaches to those writers when
Agent 2 builds them rather than to this discriminant.

### Not yet done for C-1

The C-1 call-site table in design 5.1 is implemented but has no test asserting that no moved call
site needs a projection, and the app-side suites beyond `studio_overlay` have not run.

## Touched files

Design passes, on `Create-suite-2`: `docs/GATE4-AGENT-1-DESIGN.md`,
`docs/GATE4-AGENT-1-STATUS.md` only.

Implementation, on `gate4-agent1-runtime` only: the production and test files listed under
"Implementation progress". No shared contract document, no workflow, no native command registration
and no frontend file has been changed, and nothing is merged to `Create-suite-2`. The remaining
planned files are in design 5 and 15.

## Executed checks

| Command | Result |
|---|---|
| `git log --oneline`, `git status --short` | Revision 4 starts from `7efc9c2` (Agent 3's design), which contains revision 3 at `1bcb1bc`. Agent 1's two files were unmodified by `7efc9c2`; Agent 2's documents are present untracked. Revision 4 is uncommitted. |
| `git fetch origin Create-suite-2` | Revision 1 and 2 passes both found `origin/Create-suite-2` equal to the local head. |
| `grep` over `docs/GATE4-AGENT-3-DESIGN.md` sections 11 and 13.1 | Agent 3 accepts I-4, names its three affected writers, asks that `save_studio_source_checked`'s `handoff` parameter shape be preserved, and confirms no competing source writer or second pool. |
| `gh pr view 26 --json state,reviews,title,headRefName` | Open, `Create-suite-2`, `reviews: []`. Verdicts on this design were delivered outside GitHub's submitted-review endpoint, so its emptiness is not evidence that no review happened; `e65bfd8` is called unreviewed because HANDOVER and the core note agree, not because of that endpoint. |

Through the four design passes no Cargo command was executed. Implementation execution is recorded
under "Executed evidence for C-1" above; every run there used `-j 1` with the per-package test
debug override and no concurrent Cargo work, as the shared machine requires.

**No measurement exists yet.** Quoted performance numbers still come from the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations. Design 13's eight measurements are
outstanding, including the C-1 before-and-after comparison that would quantify what the structural
decode actually saves: the code is in, the number is not.

## Proposed UI-hooks update (for Agent 4, not yet applicable)

Do **not** apply until design 12.3's prerequisites pass and the implementation checkpoint passes
review 1. Until then FLIPNOTE-UI-HOOKS must keep saying durable overlay Save is unavailable.

Under "Available now", after the existing `studio_overlay_read` block:

```ts
studio_overlay_begin({ server, channel, object? })
  -> { v: 1; kind: "eligible"; basis: string; accepted: number }
   | { v: 1; kind: "ineligible"; reason: string }

studio_overlay_save({ server, channel, object?, basis, nonce, body })
  -> { v: 1; kind: "local-draft"; channel: string; object: string | null;
       basis: string; accepted: number; alreadySaved: boolean }
   | { v: 1; kind: "acknowledged-handoff"; channel: string; object: string | null;
       basis: string; accepted: number; epoch: string; epochId: string };
```

Accompanying prose:

- `basis` is required and is the original authoring identifier: take it from
  `studio_overlay_begin` for a new branch or from `studio_overlay_read` for an existing one, and
  keep `(basis, nonce, body)` byte-stable across retries. It is an identifier, not authority.
- There is no timestamp field. The actor supplies it, and a retry with a fresh actor timestamp is
  still an exact retry.
- A request that matches no retained entry and whose `basis` is not the currently eligible one
  returns a stale error. It never becomes new work.
- `acknowledged-handoff` records that this exact request was previously transferred into the named
  local destination epoch. It is not delivery, receipt or settlement; after legitimate retirement it
  does not assert that those operations are still in the current signed source or the pending
  ledger, and it does not assert that the referenced frame pixels are still held locally.
- Retrying an already accepted save never requires the referenced PIX bytes to still exist. Saving a
  **new** frame operation does: publish the frame PIX first, and a missing blob refuses the save
  without changing anything.
- Neither result carries `content`. Refresh with `studio_overlay_read`.
- Save is available only for a Closing document whose exact expected checkpoint can be constructed
  from the actual source, its saved signed close and the observed owner tenure. Fault, an
  unavailable seed, unknown tenure and an unconfirmed preview refuse and keep unsaved editor work
  visible; a refusal is never a durable save.
- Automatic transfer happens in the background once a verified eligible successor is installed. It
  sends no packets of its own; peers receive the operations through ordinary paging. A large branch
  completes over several background turns while the app stays responsive.
- `studio_overlay_read` gains `transferState: "completed"`. `kind: "absent"` keeps its meaning.
- Both commands share the latest-view request fence with ordinary reads; run them sequentially for
  the same target. One overlay operation is live per server at a time, and capacity exhaustion,
  storage-reference capacity and an unstable vault inventory all return retryable errors.

## Next actions

1. **Flow H**, the largest remaining piece: H1 capture, H2 detached plan, the H3 signing slice with
   `MAX_SIGNING_TURNS_PER_VISIT`/`SIGNING_SLICE_BUDGET_MS` on the injected clock, H4 detached
   assembly, H5 commit, H6 notify. N31 and M5a/M5b live here, and 7.3's two-distinct-events rule
   (priority yield signs zero; slice bound signs at least one and fewer than all) is the part most
   likely to be got wrong. Then **Flow R**, which needs no media and is independent.
2. Then I-4 with C-3. See the revised sequencing table above.
3. Produce design 13's eight measurements as each item lands; C-1's before-and-after is the first
   and is cheap, since the opt-in profile already exists. **No measurement exists yet.**
4. R4-TEST-001 stays open until the reviewer can inspect `079e59a` on GitHub. C-1's call-site table
   in design 5.1 still has no test asserting that no moved call site needs a projection.
5. Track the Linux ordinary two-process smoke failure observed on `7b9cf3e` in the integration
   ledger; it is unexplained and unrelated to this scope.
6. Confirm with Agent 2 the prerequisites P1 to P5, the two-hold contract (design 12.1) and the copy
   requirements (design 12.2); give Agent 4 the central edit list in design 15. Agent 3's side of
   the I-4 coordination is already on record at `7efc9c2`, and their "unaffected if I-4 does not
   land" clause is an integration alternative, not an opt-out from a deployed cursor's discipline.
7. Keep native Save unregistered and out of FLIPNOTE-UI-HOOKS until Agent 2's manual lifecycle
   passes its own review and their status note says so. Agent 2's P5 is still false.
8. Do not send the design 18.3 implementation review until the scope it names has real evidence. A
   partial branch is not a checkpoint.
9. When Agent 2's archive writer lands, delete `write_draft_archive_for_test`, replace the
   fail-closed reference arm with their collector, and add their two archive writers to the I-4
   audit as design 9.2 now records.
