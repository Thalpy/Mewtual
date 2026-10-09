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
| 4 | I-4 and C-3 | **I-4 complete**, including requirement 3's type-level enforcement. **C-3's storage half complete and reviewed** (C3-001/002/003 and C3-TEST-001 all closed). What remains is C-3's runtime adoption at six call sites, which is its own checkpoint and wants Agent 3's coordinated verdict first |

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

### The third review: two more capacity-stranding P1s, and a check the design specified

The external reviewer returned **REQUEST CHANGES** on `da7b8fa`. The important prior fixes held up
— NEW-1, NEW-2's original bug, NEW-4, NEW-5, NEW-10 and both new H5 guards were confirmed genuine
— but the round still contained two P1s of the *same class* it was written to close, plus a
load-bearing check the accepted design specifies and this implementation never had.

All four were verified against source before being acted on.

**FLOWH-001, P1: the backoff was neither enforced nor self-waking.** NEW-3 made an H5 budget
refusal record a hold and keep the signed `Ready` job rather than discarding a detached vault
decode and every signature. But `can_commit` looked only at the stage, so on a busy actor H5
retried on every turn, draining a five-family inventory each time — the recorded deadline did
nothing at all. And NEW-8's own fix created the opposite failure: a held job reports not-runnable,
so `pending` is false, so a quiescent actor schedules no further Studio turn while the job holds
admission and one of four process-wide permits. Four such actors take the whole pool for ever.

Corrected in both halves. `can_sign`/`can_commit` take `now` and consult the same `held` that
`runnable` does, so the scheduler and `pending` cannot disagree. `HandoffRuntime::wake_in`
publishes the live job's deadline, `StudioReceiver::wake_in` exposes it, and the actor merges it
into the injected-clock wake it already computes for delivery throttling — no new timer.

**FLOWH-002, P1: a worker finishing after a pause re-stranded its bundle.** `release_if_stalled`
exempts `Detached` because the worker owns the bundle — correct — but a worker that *succeeds*
hands it back, and the job leaves `Detached` for `Signing` or `Ready` holding admission and a
permit. `pending` begins with `!self.paused`, so no further turn is ever scheduled and the release
hook on `run`'s paused path is never reached. `handoff_complete` is now pause-aware, which is the
only point where a job can re-enter a holding stage from `Detached`.

**FLOWH-003, P2: H3 never reauthenticated the wrappers.** Design 6.1's stage table specifies
"per-visit wrapper reauthentication" at H3, and §13 names **M4** as its mutation. I shipped
neither. `sign_next`'s live-authority recheck is not a substitute: it proves who is signing and
under what epoch and tenure, not that the records H2 reconstructed from are still the bytes on
disk. H5 refuses the result later, so nothing durable is wrong — the device's signing authority is
simply spent on a proposal the contract says to reject first. `handoff_sign` now takes the store
and checks the stamp through a new capability-narrowed `studio_handoff_plan_is_current` seam.

**FLOWH-004, P2: the probe read intent bodies before reserving.** §7.2 is *titled* "Reservation
precedes every body read", and the probe reserved after its rail scan — one authenticated intent
record read and structurally decoded per candidate, under custody, holding nothing. The comment at
the reserve site claimed 7.2 while describing "before the first authorization read", which is not
what 7.2 says. The reservation now precedes the scan. A failure to reserve costs nothing and
records no hold, which also closes the previously-disclosed open item that capacity contention
escalated exactly like genuine ineligibility.

**FLOWH-TEST-001 is refuted**, with citation. The reviewer reported that H3's time bound has no
discriminator and that the omission pattern therefore persists. The discriminator exists: case 3
of `studio_handoff_signs_in_bounded_slices` disables the count limiter with `usize::MAX` and
drives a 200 ms-per-read `SteppingClock` against the 250 ms budget, asserting `signed() == 2` and
`remaining() > 0`; case 2 disables the time limiter so only the turn cap can stop it. **M5b** is
recorded at §"Mutations" and fails at a *different* assertion, "the slice budget did not bound the
slice", 8 signed where 2 was required. Both landed in `c676749`, outside the reviewed range, which
is how a diff-scoped read missed them. The reviewer's derived conclusion — that the open-items
list was incomplete again — is nonetheless **correct**, on the strength of FLOWH-001 through -004,
none of which the list named.

| Mutation | Effect |
|---|---|
| M36: `stage_due` ignores the hold | "H5 ran inside its own backoff": the job commits instead of staying `Ready` |
| M37: `wake_in` publishes nothing | "a held job published no deadline, so a quiescent actor would never revisit it" |
| M38: `handoff_complete` not pause-aware | "a completion arriving during a pause parked the bundle in a stage no turn will ever visit" |
| M39 (design **M4**): no per-visit reauthentication | "H3 signed against records that are no longer the ones it authenticated" |
| M40: reserve after the rail scan | the selection cursor advances, proving the scan ran holding no admission and no permit |

> ### Three claims in the scheduling investigation were wrong, and are withdrawn
>
> All three were mine, all three are contradicted by source, and all three had been carried in
> this ledger as established. Verified before withdrawing:
>
> **1. "The install has ~8 s of injected clock."** False. `start` is captured *after* the owner is
> revealed (`scheduling.rs:513-514`), and the loop is `for pass in 0..160` at 250 ms per turn, so
> the bound is **40,000 ms measured from owner return**. I computed `40000 - 32250` as if the
> bound were absolute. There are three separate time concepts here and I conflated two: the outer
> 90 s Tokio deadline is real time; the 40 s install bound is injected time *after owner return*;
> preview and request expiries are independent protocol lifetimes on the injected clock.
>
> **2. "Three ready previews hold three of the four shared preparation permits."** False.
> `PreviewJob::run` puts both permits in `let _permits = (permit, preview_permit);` **inside** the
> `spawn_blocking` closure, so both drop when the blocking work ends; `PreviewCompletion::Prepared`
> carries only the result. A *ready* preview holds **zero** preparation permits. Three
> concurrently *running* parsers can hold three, which is bounded occupancy by design — the
> three-permit preview pool is a **sublimit on** the four-permit shared pool, not seven units of
> capacity. The variant named `three_retained_previews` retains checkpoint-memory slots, a
> different allocator, not parser permits.
>
> **3. "The preview-readiness loop is unbounded."** False. It is `for _ in 0..160` with a
> `"preview {i} never became ready"` assertion. The membership and channel convergence loops
> earlier in the fixture are genuinely unbounded, but they are not the preview loop and should not
> have been described as one.
>
> **What survives:** the preparation semaphores are process-wide while each `Pair` runs its own
> `ManualClock`, so unrelated concurrent tests can consume the same capacity while advancing a
> different simulated clock — which also means "my fixture advanced 30 s, so other retentions
> expired" is invalid reasoning. And `unopened`'s `expect("cold paid preparation")` asserts a job
> must exist immediately without establishing any capacity precondition, which is a source-derivable
> test-isolation weakness independent of whether it caused the observed run.
>
> **What is needed is one failing trace**, not another aggregate pass count: which operation could
> not progress, which resource or deadline prevented it, and who owned that resource. A zero
> reading of `available_permits()` describes legitimate running work; the evidence is the refused
> acquisition at its own site, with the owner identified.
>
> ### Full-suite evidence at the current Flow H correction head: **FAILING / unresolved**
>
> `scheduling::studio_actor_owner_return_installs_both_classes_with_cancelled_preview_transport`
> failed in **2 of 4** full runs after FLOWH-004. A single clean rerun previously attributed this
> to known contention; **that attribution is withdrawn as under-evidenced.** No Flow H correction
> PASS may rely on the full suite until the interaction is isolated or fixed.
>
> **Isolated by measurement: the cause is not Flow H.** The FLOWH-004 hypothesis was that a
> watched target with no transferable overlay now transiently consumes a slot of the four-slot
> process-wide pool while H1 determines there is nothing to do, and that store-wide
> `intent_generation` rotation (`epoch_intents.rs:535`, `:566`) clears the quiet memo on any intent
> write anywhere, making that frequent. Every link in that chain is true. The conclusion was still
> wrong, and instrumenting the fixture said so:
>
> | Measured in the failing fixture | |
> |---|---|
> | probe reservations, whole run | **4** alone, **11** under full-suite load |
> | handoff jobs created | **0** |
> | elapsed, alone | 6498 ms |
> | elapsed, under full-suite load | 6797 ms, against a **90 s** bound |
>
> `jobs = 0` means the machinery under review never executes here: no `can_sign`, `can_commit`,
> `handoff_complete`, `handoff_sign`, no admission or permit ever held by a job. Flow H's entire
> footprint is a handful of `try_acquire` reservations, which are non-blocking and so cannot
> deadlock. And 6.5 s alone versus 6.8 s under load means the failure is **not** gradual slowdown
> with 83 s of headroom to spare — it is a hang, in one of the fixture's unbounded convergence
> loops (`while p.bob.epoch() != p.alice.epoch()`, and the `select!` whose other arm is
> `loop { sync_once() }`).
>
> So this is a pre-existing intermittent hang in a two-actor networked fixture, which the status
> note above had already seen once and misfiled. It needs its own investigation and must not be
> closed by rerunning. What it is **not** is evidence against the Flow H corrections, and that
> distinction is now measured rather than argued.
>
> Two fixes were ruled out in advance and remain ruled out. Giving H1 its own semaphore would
> violate the accepted single four-slot capacity model. Lengthening the test timeout would hide the
> question rather than answer it — and with 83 s of headroom it would not even help.
>
> **Method note, because it cost two wrong answers today.** Both the "documented contention flake"
> disposition and the `intent_generation` mechanism were chains of individually true statements
> that did not support their conclusion. Both were settled in minutes by counting something.
> Neither would have been settled by another full-suite run.

**The paragraph below is superseded by the ledger above and kept only to show what was claimed.**
One full-suite run reported `scheduling::studio_actor_owner_return_installs_both_classes_with_cancelled_preview_transport`
failing: the same test, in the same module, already recorded above as a deadline assertion that
fails under concurrent Cargo load and passes serially. It passes in isolation in 6.6 s, the
checkpoint run at 660 passed with *more* global-permit churn than the final code has, and a clean
re-run with no competing Cargo process passed 660 with zero failures.

That is the hypothesis confirmed rather than assumed, and the reason it was worth confirming is
specific: FLOWH-004 makes the H1 probe hold a **process-wide** permit across its rail scan, and
the `studio_exchange` tests use the real four-slot pool rather than an injected one. A genuine
starvation there would look exactly like a flake. It is not one — but "it was flaky before" was
not sufficient evidence on its own.

### The fourth review: the same class again, at the boundary

FLOWH-002 closed. FLOWH-004's body-read ordering closed. FLOWH-003 now genuinely authenticates
before a non-priority slice. **FLOWH-TEST-001 was withdrawn in full** — the reviewer confirmed the
`c676749` time-bound case and M5b do discriminate, which is the citation standing up.

But FLOWH-001 did **not** close, and the two corrections each introduced a smaller defect of their
own. That is the fourth consecutive round in which that has happened, and it is worth naming the
shape rather than the instances: every one of these has been *two things that should agree about
one fact, computed in two places*.

**FLOWH-001-R1, P1: `pending` and `wake_in` sampled the clock separately.** The gates were made to
agree with each other, and then the two answers derived from them were taken from different reads.
Sample `pending` at `D - 1` — not runnable — let the clock cross `D`, then ask for a deadline:
`wake_in` publishes only future deadlines, so it correctly answers "nothing", no timer is armed,
`studio_pending` stays false, and the `Ready` job holds admission and a process-wide permit until
unrelated work happens by. The original strand, squeezed into the expiry boundary. M37 could not
see it: its mutant published nothing at all, where this needs a *correct* deadline expiring
between two reads.

Corrected structurally rather than by widening a window. `signal_and_wake` takes one sample and
answers both questions from it, so the two are exhaustive by construction: at or past `D` the job
is runnable and `pending` carries it; below `D` the delay is strictly positive. There is no third
case, which is the property the previous shape lacked. `pending` and `wake_in` survive only as
`#[cfg(test)]` conveniences, because a separately-sampled pair is exactly what must not exist in
production.

**FLOWH-003-R1, P2: the reauthentication ran before the priority yield.** §7.3 says a priority
turn gives way immediately; the stamp check reads and hashes the whole authenticated intent record
plus the bounded source record. So a priority turn did precisely the custody body work the
priority gate exists to avoid, and a transient read error on such a turn could abandon a job that
was never allowed to start a signing visit. The check is now behind `!priority`. A yield is not a
visit, so it reauthenticates nothing.

**FLOWH-004-R1, P2: a capacity refusal had no future retry.** Not charging the target was right —
losing a race for capacity says nothing about the document. Recording *nothing* was not: the
reservation is a `try_acquire`, so the actor is not queued behind the permit and nothing tells it
capacity returned. No resource is stranded, but an automatic transfer that resumes only when
unrelated Studio work arrives is not automatic. There is now one flat, non-escalating
`probe_retry_at`, cleared the moment a reservation succeeds, and `wake_in` publishes it — together
with the earliest future `next_at` among targets **still on the watch rail**, which also covers the
abandoned-job case the reviewer raised and bounds what those maps can wake the actor for.

My own test had hidden this by calling `run` again immediately after dropping the permits, which
production does not get for free. That assertion is now on the published deadline instead.

| Mutation | Effect |
|---|---|
| M41: off-by-one in the deadline filter | "at 30999 (deadline 31000) the job was neither runnable nor waited on": the exhaustiveness window opens by exactly one millisecond |
| M42: reauthenticate before the priority gate | "a priority turn must report its yield, not vanish" |
| M43: capacity refusal records no retry | "a capacity refusal left no future retry, so the transfer is dormant" |

M41 is the one worth noting: the race itself cannot be reproduced with a `ManualClock`, because
both reads return the same value. What is tested instead is the invariant that makes a single
sample sufficient — never *neither* runnable nor waited on — asserted at each millisecond across
the boundary. The single sample is structural; the invariant is what a mutation can reach.

### The fifth review: the last two blockers, both the same shape

The reviewer accepted the scheduling-hang isolation and removed it from the Flow H causal argument
entirely, leaving REQUEST CHANGES resting on two deterministic findings. Both were confirmed
against source before being acted on, and both are the same shape as everything before them: a
deadline or a hook that gates one thing while a second thing is needed to make it reachable.

**P1: the pause stranded its own release.** All three sites that set `paused = true` returned
immediately, and `release_if_stalled` lives at the top of `run` — which is exactly the event a
paused receiver prevents, since `pending` begins with `!self.paused` and `wake_in` returns nothing.
A background pass owning a non-detached bundle when an unrelated step errored would hold admission
and a process-wide permit until a user happened to open a Studio document successfully. One
`pause()` helper now sets the flag, the notice and the release together, at the transition.

**P2, FLOWH-004-R1: the capacity retry had neither half.** The probe never read `probe_retry_at`,
so under unrelated Studio traffic it re-attempted the reservation every turn while claiming two
second pacing. And at `now == D` the no-job `wake_in` branch filtered the deadline away as no
longer future, while `runnable` requires a live job — so the timer that fired was the last one the
actor would ever arm. Gate and wake, the same pair as FLOWH-001, missed again in the branch where
`runnable` cannot stand in.

The spin that made the future-only filter look necessary is gone at its source: `quiet_for` now
clears that target's `next_at` and `hold_ms`, because a memoised target's gate is the memo and a
generation rotation is what reopens it. That also stops the pacing maps accumulating an entry per
quiescent document.

| Mutation | Effect |
|---|---|
| M44: pause without releasing | "the pause parked a bundle in a stage no turn will ever visit" |
| M45: probe ignores its own retry deadline | the deadline is rewritten each turn: `left: Some(2000), right: Some(1500)` instead of counting down |
| M46: a due retry is never reported | "at its deadline the retry was neither due nor published" |

**M45 first passed against a build with the gate removed, and that is the finding worth keeping.**
The original assertion was that the selection cursor does not move — but with the pool exhausted
the reservation fails before the cursor advances either way, so it witnessed nothing. Three times
in this work a plausible assertion has proved nothing until a mutation broke it. The green result
is not the evidence; the failing mutant is.

**CI, recorded as separate integration evidence.** Five specialised workflows green at
`da73142`. General CI's Windows job checked out PR merge commit `db2798c`, not the branch head,
and there the two owner-return fixtures failed on an **assertion** — "Studio must install through
the reserved slot" — at 659 passed / 2 failed / 11 ignored. That is a *different failure mode* from
the timeout measured at the branch head, and its text is pool-related. It should be investigated as
a possible second, distinct defect rather than assumed to be the same hang, and it cannot speak to
head-SHA causality because it is a different tree.

### The sixth review: one termination defect, and a claim that overstated itself

The pause-transition P1 closed, and both halves of FLOWH-004-R1 closed. One finding survived.

**FLOWH-004-R2, P2: an expired capacity retry could outlive its own reason.** `probe_retry_at` is
global, not per-target, so unlike `next_at` nothing filters it by the current rail. It is owed only
because an eligible target could not get a permit — and if that target stops being watched before
the deadline, `probe_due` keeps reporting work while `handoff_probe` returns immediately on the
empty rail, holding the receiver at its active Studio cadence for ever with nothing a probe could
do. No permit is stranded, which separates it from the earlier capacity strand, but permanent
useless work is still a defect.

My round-5 request asserted that every due state terminates through "a job, a future deadline, or
the quiet memo". There is a fourth outcome — *the condition that justified the retry ceased to
exist* — and nothing consumed it. Every exit from `handoff_probe` that is not the retry's own gate
now clears it, which makes the real invariant true: **`probe_due` true implies the next probe
either attempts capacity, arms another future gate, or consumes the state.**

| Mutation | Effect |
|---|---|
| M47: empty rail does not consume the retry | "an expired capacity retry survived the disappearance of every eligible target" |
| M48: success does not clear the retry | `left: Some(3000), right: None` |

**M48 only works because the oracle moved.** The round-5 assertion routed this claim through
`wake_in`, which takes its live-job branch once a job exists and never looks at `probe_retry_at` —
so it would have stayed green with the clear deleted. It now asserts on the retry directly. That is
the fourth assertion in this work that proved nothing until a mutant was run against it, and the
reviewer found this one by reading rather than by running.

**Item 6's wording was also wrong and is corrected below.** "Bounded rather than unbounded" was an
overstatement: rail filtering bounds the *scheduling* impact, not the *retained state*.

> ## C-3 is hard-blocked until I-4 enforcement is complete
>
> The reviewer's ruling at `0a9c586`, recorded verbatim in substance because it is the condition
> the rest of this work hangs off:
>
> **I-4 may land incrementally only while `inventory_generation` has no production consumer.
> Partial conversion grants no I-4 acceptance.** C-3, or any other code that releases inventory
> custody and relies on the generation token for consistency, is hard-blocked until:
>
> - all replacement writes are capability-only;
> - **all unchanged-file sync repairs are capability-only**;
> - all unlink, rename and temporary-sibling paths are capability-only;
> - bare five-family mutation primitives are no longer callable from participating production
>   paths;
> - Agent 2's DraftArchive writer and release satisfy the same discipline;
> - the audited **leaf** list is reconciled against source;
> - reads, budget mint and budget entry are proved not to rotate;
> - a final cursor-level invalidation suite passes.
>
> And: **do not propagate `EpochMutation::with()` into further production writers.** Replace it
> with synchronous narrow operations before the remaining conversion.
>
> The reason the gate sits at *activation* rather than at source landing is worth keeping: with no
> production consumer, an under-rotating writer cannot make anything accept stale state, because
> nothing reads the token. The moment a cursor captures it across a custody release, one
> unconverted mutation makes the whole consistency argument false.

> ## I-4: everything except requirement 3 is accepted
>
> At `cad2689` the reviewer closed I4-BUILD-001, I4-HOOK-001 and SCHED-DIAG-001, and confirmed
> requirement 2 stays PASS. Established for the currently implemented mutation paths:
>
> - replacement writes capability-bound
> - retry and sync repairs capability-bound
> - cleanup unlink and directory sync capability-bound
> - project persistence primitives hidden behind the sibling boundary
> - root-sync exception destination-bound
> - production writer callbacks audited synchronous
> - reads and budget-only operations proved not to rotate
>
> **Requirement 3's conversion is the sole remaining source-enforcement step before C-3.** The
> hooks and unified tag are preparation, not enforcement: the transaction signatures still accept
> arbitrary writer and sync closures.
>
> And once it is converted, no further architectural invention is required before C-3 starts. The
> next substantive proof is the cursor-level property in C-3 itself — capture the generation,
> release custody, mutate any inventoried family, reacquire, and the cursor must refuse before
> continuing or issuing an inventory — plus the negative case that reads and budget operations do
> not invalidate it.

> ## Requirement 2: PASS, both halves
>
> Closed at `bc957be` by source review: the generic persistence operations are hidden, and the
> vault-root exception is destination-bound. The reviewer also established something I had stated
> imprecisely — `StagingPath` and `AtomicWritePhase` remain nameable `pub(super)` types, but their
> fields and production constructor are private, so a sibling cannot build one aimed at an
> arbitrary file and trigger its unlinking `Drop`. The accurate claim is "the mutating
> implementation and construction paths are private", not "every associated type name is private".
>
> ### I broke the Unix build and could not have seen it
>
> `a_preplanted_staging_symlink_is_never_followed` still called `staging_candidate` and
> `open_staging_candidate` by name after those became private. It is `#[cfg(unix)]`, so a Windows
> run never compiles it: every local suite passed while CI's Linux job failed at
> `error[E0425]` before running a single test. Attribution is partly older —
> `open_staging_candidate` was already inaccessible at the review base — but this range hid
> `staging_candidate` after replacing its export and did not migrate the consumer.
>
> Fixed by rehoming the test **inside `persistence`**, where it exercises the private primitives
> directly, rather than by making either `pub(super)` again. The planted-symlink open attempt is
> preserved; replacing it with a write to another generated name would test nothing. The module is
> `#[cfg(all(test, unix))]` because its only content is that test.
>
> **Verified rather than assumed.** A temporary probe in the same module referencing
> `staging_candidate`, `open_staging_candidate`, `atomic_write` and `fs` compiles on Windows, so
> every name the Unix test needs resolves there; only `symlink` is Unix-specific and is imported
> in the test body. (`cargo check --target x86_64-linux-android` fails in `ring`'s build script
> before reaching this crate, so it proves nothing.)
>
> ### The after hook could silently accept a no-op injection
>
> `WriteHooks::after` treated `Intercept::Replace` as success and discarded the bytes. A
> fault-injection test moved from a writer callback to an after hook would have run, substituted
> nothing, and reported nothing. That is a masked assertion built into the API, before a single
> test used it. Fixed with a separate `AfterIntercept` carrying only `Continue` and `Fail`, so the
> mistake is unrepresentable rather than rejected at runtime.
>
> ### The pass-count diagnostic claimed something trivially true
>
> I wrote that a failing run reporting 160 passes "is the mechanism, and no semaphore
> instrumentation is needed". False by the loop's own control flow: its only early exit is the
> same conjunction the assertion tests, so **any** failure there entails 160 passes, and "far
> fewer passes" is unreachable. Capacity refusals, expired requests, other work being selected,
> and a mismatched projection all produce the identical line. The count says the bounded loop
> exhausted, not why. Claim withdrawn in the source comment; the combined booleans and the count
> stay as description.
>
> Also narrowed: identical serial and concurrent-3 completion times show no interference **in
> those executions**, not that the variants cannot interfere under another interleaving. 132 of
> 160 passes is a measurement, not a worst-case guarantee.

## C-3: the storage half is complete; the runtime half is the next checkpoint

**Landed** (`5d20cb5`, `5fe2d79`, `6026240`, `5d266bd`, `176e4a1`):

| Piece | Evidence |
| --- | --- |
| Scan is an owned `EpochStorageCursor`, store borrowed per call | The property was previously inexpressible: the old scanner held `&mut ServerStore` for its whole life, so no write could land between its steps |
| Invalidation refused before resuming **and** before issuing | Two mutations, each caught at its own assertion |
| N17 with the real writers | **All six families**, plus the cleanup unlink, which is an operation class rather than a family. Recovery, OwnerReceipts, Intents, Registry and Studio are mine; **DraftArchive's three write shapes were asserted by Agent 2 at `e60d8315`**, closing the last gap. Also the unchanged exact-retry flush and a failed write; Studio carries the negative half (a budget mint must not invalidate). The cursor side needs no per-family test at all - see "DraftArchive N17: CLOSED" below |
| Validation extracted as a pure function | Purity is now a signature, not a claim: if it ever needs the store back, the compiler says so |
| Parked body, detached validation, four rebinding checks | Cursor identity, mount, record id, generation |
| `MAX_INVENTORY_RESTARTS` with `Unstable` | Mutation: removing the bound fails at "restarted more times than its budget allows" |

**Two deviations from 9.2's literal text, both forced:**

1. `budget_ms` is `Option<(&dyn Clock, u64)>`, not a bare `u64`. `check-no-ambient.sh` forbids
   reading the clock ambiently and elapsed time cannot be measured without one. Same shape the
   H3 signing slice already uses.
2. `None` means *no bound and never park*. The 75 single-visit callers hold the store across the
   whole scan; a parked body would strand them rather than shorten any custody hold.

**The classifier detaches everything.** `validation_fits` returns false for every fresh
validation, because 13.7 does not exist and 9.2's rule for that case is to default to detaching.
So a budgeted scan currently does one record per visit. That is the safe direction - detaching a
cheap record costs a visit, inlining an expensive one costs an unbounded custody hold - but it
means **13.7 is now load-bearing for throughput, not just a reporting obligation**.

### The runtime half: designed in `GATE4-AGENT-1-C3-RUNTIME.md`, step 1 in progress

The runtime adoption has its own design document, `GATE4-AGENT-1-C3-RUNTIME.md`, reviewed twice
before any runtime code. Revision 1 (the scratch draft) was **not ready**: two blockers (a queue
head gated on `replay_ready()` could wait on catch-up forever; a second pool permit for overlay
jobs could wedge the four-slot pool) and five highs, among them that the job API silently dropped
the receive limits and that the throughput premise was false (every uncached record parks, one per
visit). Revision 2 answered all 22 findings; its re-review closed both blockers in principle and
found four new highs, all in the later conversion steps. Revision 3 records the decisions for those
steps, and **step 1, the storage prerequisites, was accepted for implementation**.

The list of sites below is the historical one. The current, complete table (about thirty sites,
classified explicit or background, with profile and what each retains when not ready) is in the
design document's section 5.

**Step 1, the storage prerequisites, is implemented** (no runtime owner converted yet):

| piece | what it does | test | mutation, killed |
|---|---|---|---|
| S-1 `EpochInventoryProfile` | coverage plus the four limits as one value; every scan is built through `scan_epoch_files_with(profile)`; `receive()` is exactly the old receive rail; a job keeps its profile on both restart paths | `an_inventory_job_keeps_its_profile_across_a_restart`: 65 records, restart from a step and from finish, both still refuse at the 65th | widening either restart path to `full()` |
| S-2 generation stamp | the issued inventory carries the `inventory_generation` it was finished under; the Studio budget mint refuses a mismatch | `an_inventory_finished_before_a_five_family_write_cannot_mint_after_it` | dropping the new comparison: the stale inventory minted |
| S-3 `drive_epoch_inventory_job` | one visit's drive against an absolute deadline, through a cursor step that takes the deadline as given (`step_until`); never begins a step with no time left; reports an already-parked body as `Parked` | `driving_a_job_respects_the_visit_deadline_and_stops_at_a_park`, `..._treats_its_deadline_as_absolute`, `..._loops_several_one_record_steps_in_one_visit` | dropping the pre-check (8 entries instead of 0); calling the relative step (traversal completed past the deadline); stepping once (stopped after one record); dropping the parked check |
| S-4 held cursor beside non-family files | pins that creating, replacing and removing `.bin`/`.net` siblings, and a real `save_server`, between steps neither faults nor changes records or authenticated bytes | `a_held_cursor_tolerates_non_family_files_changing_beside_it` | (pins platform behaviour; no guard to break) |

**Its review: one high, fixed.** An Opus adversarial review of the first cut found that
`drive_epoch_inventory_job` passed its **absolute** deadline to the step's **relative** form,
which adds it to the current time. A step's own expiry was therefore disabled, one step could run
every non-family name to the next record, and the classifier was handed an enormous remaining
budget, harmless only while `validation_fits` refuses everything. Nothing called it yet; step 2
would have built on it. My test missed it because it used only a deadline already past and
`u64::MAX`, where the two readings coincide. Fixed with a cursor step that takes an absolute
deadline, plus the regression above. Also from that review: a test that `drive` really loops (M1),
an inventory that starts with a token matching nothing so only the finish stamp makes it mintable
(L2), the parked report (L3), a real server write in the S-4 test (L4), a doc warning on the
vault-wide job constructor (L5), and the misplaced test doc comment (L1). The re-review closed
H1 and found nothing new. One residual, recorded rather than changed: with a body taken but not
yet installed, `drive` reports `Stepped` when out of time and an error when time remains. The
planned runtime cannot reach that state, because a job whose body is out for validation is not
stepped (design section 4).

### Step 2 (replay's manual move): landing (2026-10-09)

Built and reviewed as `GATE4-AGENT-1-C3-RUNTIME.md` section 12 records:
- the shared, turn-based `InventoryRuntime`;
- its detached validation through the receiver's background-job machinery;
- N-M1's own-write accounting;
- replay's manual move taking its budget from the runtime, with a synchronous fallback on
  `Unstable` or after 60 s of patience.

Its adversarial review found no blocker and one high, a liveness stall under ordinary writes,
which the fallback answers. Since the classifier landed (`6a79e6f8`), small records of the
uncached families validate inline, so only cold Studio and Registry records, and oversized others,
detach.

**Landed** in the batch with F1 and F4, once Agent 2 freed the shared receiver files. Before
landing, both of their constraints were checked:
- the new `pending_at` term sits inside the pause gate, and `pause()` releases the job;
- the Closing `save_overlay` still takes only its own target's parked plan, so it clears
  `unconfirmed_scheduled` exactly as before.

The `take_any_planned_overlay` mirror I had promised them was not built. A Closing commit needs
that target's own close record, so taking another target's plan could only drop it and make its
caller redo the work. The park deadline (`OVERLAY_PARK_MS`) already bounds a stranded plan, so a
Closing Save on another target answers `Busy` for at most that long.

**Batch review (2026-10-09, Opus, static, at `03e3d672`): no blocker or high.** It covered step 2
on this base, F1 after the rebase and F4. It confirmed step 2's integration with the classifier,
the memo, the parked-plan slot, pause and the lock. It found:

| finding | disposition |
|---|---|
| M-1: step 2 shipped without its HANDOVER and THREAT-MODEL updates, and THREAT-MODEL said no path held a cursor across visits | **fixed**, both documents |
| M-2: F4's paused `Busy` contradicted both `Busy` docs, and costs a scan and media admission per resend | **documented** on both variants and the helper. Integrators must back off while receive is paused. No `Paused` variant: a pause check before the budget would break the exact retry while paused |
| L-1: a validation error about bytes another actor's write replaced paused receive, while one about an own write was dropped | **fixed**: a charged restart (`restart_epoch_inventory_job_if_overtaken`). Pinned by `a_validation_error_about_overtaken_bytes_is_a_charged_restart`, which fails with every error surfaced |
| L-2: stale "until 13.7" comments, and `memoize_overtaken_inventory_result` still dead in production | **fixed**; the function is now `#[cfg(test)]` |
| L-3: the small-vault move test could not tell the shared job from the old scan | **fixed**: it now asserts a budget was minted, and fails with the move forced onto the fallback |
| L-4: the F4 doc overstated that an exact retry never depends on receiver state | **narrowed**: it can still answer `Busy` while the slot holds other work |

**Residual risks, recorded rather than fixed here:**
- a UI lock does not release a queued or parked Save plan (Agent 2's area; the inventory job's
  own lock rule argues for the same);
- a fresh job can detach a second validation while an abandoned one still holds a permit,
  bounded by the pool;
- pause and lock lift the job's backoff;
- replay's job can take the last free permit from catch-up for a visit;
- the I-4 writer audit predates writers added since `2df3564f`, which rely on the type-level
  guard and the raw-fs gate.

### I-4 writer audit at C-3 step 2 (2026-10-06, Opus, static, at `2df3564f`)

Section 8 of the runtime design requires the audit to be re-run before the first production code
that parks a cursor, because only then does an under-rotating writer become unsafe. No blocker or
high. Findings and their dispositions are in the runtime design's section 12 (M-1 raw `std::fs`
now refused by `scripts/check-store-raw-fs.sh` in CI; M-2 a finish-time names-only listing check;
M-3 read-path rotations recorded, the completed-handoff serve memoised and its three sibling
sites a follow-up; L-1 fixed; L-2 and L-3 follow-ups). The
audited writer list, which is what proves coverage beside the type-level guard:

| writer | kinds touched | rotates before first mutating I/O | evidence |
|---|---|---|---|
| Recovery accounted, incl. eviction settle | Recovery + temp | yes | `epoch_recovery.rs:374→376`; `:411` delegates |
| Recovery unaccounted / tooling | Recovery + temp | yes | `epoch_recovery.rs:485→487` |
| Owner journal (all owner writers, fault/repair resave, publication) | OwnerReceipts + temp | yes | `epoch_owner.rs:642→644`, the only write in the family |
| Intents replace (ordinary, overlay accept, handoff stages) | Intents + temp | yes | `epoch_intents.rs:752→754` |
| Intents exact-retry sync | Intents | yes | `epoch_intents.rs:712→714` |
| `flush_checked_epoch_intents` | Intents | yes | `epoch_intents.rs:568` |
| Retirement replace / zero-removal sync | Intents | yes | `retirement.rs:352→354`, `312→314` |
| Handoff completed sync (source precheck) | Intents | yes | `epoch_studio/handoff.rs:702` |
| Handoff publication sync (read-only serve) | Intents | yes; a repeat of a flush this mount already made, with no five-family write since, is now skipped with no I/O (M-3) | `handoff.rs:749` (now `:751`) via `source.rs:158`, through `sync_intent_unless_durable` |
| `sync_intent_unless_durable` (added for M-3) | Intents | yes, through its own guard, when it flushes at all | store `epoch_recovery/inventory.rs`, `RepeatSyncMemo` beside it |
| Draft archive write | DraftArchive + temp | yes | `epoch_draft_archive.rs:255→257` |
| Draft archive exact-retry sync | DraftArchive | yes | `epoch_draft_archive.rs:209→211` |
| Draft archive release (unlink + parent sync) | DraftArchive | yes | `epoch_draft_archive.rs:370→372,379` |
| Registry epoch replace / unchanged sync | Registry + temp | yes | `epoch_registry.rs:492→494`, `457→459` |
| Registry head proof sync / repair barrier | Registry | yes | `epoch_registry/head.rs:343→345`, `383→385` |
| Registry maintenance hint | Registry, Intents | yes | `epoch_registry/page_source.rs:60,71` |
| Registry page receive sync | Registry | yes | `epoch_registry/receive.rs:201→203` |
| Studio source (seal, rotate, adopt, overlay commit, ingest, handoff) replace / unchanged sync | Studio + temp | yes | `epoch_studio.rs:688→690`, `643→645`, the only write in the family |
| Studio discovery proof sync / repair barrier | Studio | yes | `epoch_studio/discovery.rs:400→402`, `439→441` |
| Cleanup unlink batch + directory sync (test-only callers) | all temps | yes, per step before the loop | `epoch_recovery/cleanup.rs:164→197,215` |
| Failed-write staging unlink (`StagingPath::drop`) | any, inside a guarded write | yes (the caller's guard) | `store.rs:567-587,1256-1263` |
| Non-family savers (`.bin`, `.net`, `.cache`, ui-state, pairing, `registry.bin`) | none; same directory, hence M-2 | no, correctly | `store.rs:1203-1228` |
| `remove_server` | none (`servers/` non-family files) | no, correctly | `store.rs:1608-1619` |
| `ServerStore::open` (creates `servers/`, root sync) | directory, before mount | n/a (fresh token) | `store.rs:1383,1390,1401` |
| Vault session lock, passphrase rewrap | vault root only | n/a | `catcoms-storage/src/vault.rs:118-122`; `store.rs:1448` |
| Blob stores | `blobs/` only | n/a | `store.rs:1635-1659` |
| `WriteHooks` seams | decide only, no I/O; `None` is the only production value | n/a | `store.rs:869-894` |
| Test-only raw writers | any | no (`cfg(test)`) | `store.rs:681-684,1162-1179`; `epoch_draft_archive.rs:533,562` |
| catcomsctl, Tauri bridge | nothing in `servers/` outside tests | n/a | `bins/catcomsctl/src/main.rs`; `admission_storage.rs` tests from `:382` |
| Budget mint / scope entry | none | correctly does not rotate | `epoch_studio.rs:187,222` |

Line numbers are as audited at `2df3564f`. The readers' side was confirmed sound: step, install,
finish, mint and reference finish all recheck the token, and `Arc::ptr_eq` against live clones
cannot suffer ABA. Residual risks the audit named, not defects: the token is per `ServerStore`
instance and exclusivity rests on the vault's file lock (weak on Linux NFS); the `servers/` parent
is checked as a non-link directory only at begin; crash-orphaned `.bin` staging files could
exhaust the 1024-entry receive limit, which fails closed.

### What remains, and why it is a separate checkpoint

The runtime still drives scans the old way at six sites: `studio/control.rs` (x2),
`receiver.rs`, `receiver/catchup.rs`, `receiver/handoff.rs`, `receiver/replay.rs`. Each runs a
scan to completion inside one synchronous closure and uses the inventory immediately.

Converting them is not a signature change. Each becomes a multi-visit state machine, because the
budget cannot be built until the scan completes and the work needing it has to wait; the parked
body needs a `StudioBackgroundJob` variant; and the scheduler needs to handle its result. This is
precisely the semantic consistency change section 15 asks for a coordinated verdict on.

**Correction to an earlier note in this ledger:** I recorded `OverlayOwnership` as a gap in the
parked body. It is not. Ownership is created at admission in the runtime and the parked body
travels *alongside* it in the job enum, exactly as `OverlayPlan` and `OverlayAssemble` already
do - no second pool, no capacity released when only the waiter is cancelled. The storage layer
neither creates nor holds it. The requirement attaches to the runtime variant when it is added.

Until that conversion lands, every shipping caller passes no budget and therefore never parks.

The accurate statement of what the budget now does, which is narrower than "the custody bound
exists": **fresh typed validation can be detached, and a supplied deadline stops further units
of work from beginning.** A single filesystem operation is not preemptible through this API, so
this is not a measured latency ceiling, and no production bounded-custody claim follows until
those callers adopt the budgeted path.

The parked plaintext's residency **is** now charged to section 13.4's retained-input sum, at
18 876 416 bytes, which is the largest single accounted term in the permit. Still not established
here: no runtime variant yet demonstrates that a cancelled waiter does not release a still-running
validation's reservation. That remains an activation requirement.

### Review provenance: what is independently closed, and what is only submitted

**A distinction this ledger had been blurring.** "Landed and reviewed PASS" was being used for two
different states, and they need separating:

| State | Meaning |
|---|---|
| **independently reviewed and closed** | a returned verdict says so, and the SHA it was given against is recorded |
| **implemented, tested and submitted** | the work and its mutation evidence exist and are pushed; no returned closure |

- **C3-001, C3-002, C3-003: independently closed.** The returned verdict closing them is on record.
- **C3-TEST-001: implemented, tested and submitted - not independently closed.** The last explicit
  verdict received left it **open at P2**. The corrections that followed, and the heading below
  saying "closed here", are *my* status statements. They are not a closure record, and the
  distinction stands until a verdict is returned or the fixes are re-reviewed.
- **R4-TEST-001: the same.** Its recorded status has been "awaiting the reviewer's inspection of
  `079e59a` on GitHub" for a long time. A push that was once missing should not remain the status
  indefinitely; the outstanding question and its closure evidence need recording properly.

This does not reopen C3-001/002/003 or I-4, and does not suggest the later test work is wrong. It
means the summary list must not present submitted work as reviewed work.

### The C-3 storage review: C3-001, C3-002, C3-003 closed; C3-TEST-001 corrections submitted

The reviewed C-3 code is at `e6111ed`. **Any accepted combined checkpoint must also include
`397f689`**, which restores a check of Agent 2's that I deleted by mistake with a path-scoped
`git add -A` while their tree was mid-mutation. `e6111ed` alone is not a safe base.

C3-TEST-001 asked for three things in the equivalence test, and a fourth was found while
supplying them.

**Compare each footprint field directly, and observe the real counters.** The old `canonical`
helper packed two footprint fields as `protocol + settlement * 1_000_000`, which is lossy: an
offsetting pair of errors cancels. It is now a `CanonicalRecord` tuple with separate fields, all
four `(server, group)` combinations are compared rather than one, and the test retains the
budgeted run's `EpochScanProgress` and compares the scan's own accounting against the unbudgeted
run's via a new `collect_with_progress`. An independent expected byte total is anchored, so the
comparison cannot pass by both sides being zero. Both mutations the review named -
`self.progress.authenticated_bytes = 0` and `+= validated.size` - now fail at *"parking changed
the scan's own accounting counters"*.

**Aggregate-byte refusal and persistent poisoning after a detached round trip.** The rail test
now lifts the limit after the refusal and requires the cursor still to refuse with "restart
required": the refusal is a property of the cursor, not of the current limit. A second case,
`the_aggregate_byte_rail_still_refuses_after_a_record_has_been_parked_and_installed`, drives the
rail through a full park-and-install cycle first.

**The focused reference-scan case is supplied, not deferred.** It compares budgeted against
unbudgeted CID collection.

**The fourth thing, found by mutation: equivalence alone could not see a shared defect.** Both
sides of that comparison run the same `install_body` merge - deliberately, since that sharing is
what makes the comparison meaningful - so dropping CIDs inside the merge empties *both* sets and
the equality still holds. The first mutation I ran did fail, but at the fixture guard, which
reports the fixture as broken rather than the code. The test now also asserts *absolutely* that
the budgeted scan collects the fixture's own known pixel. Re-verified with a mutation confined
to the detached path: it fails at "a budgeted reference scan lost the fixture's own pixel".

Five corrections in the same pass. The first two were found by re-deriving the claims rather
than re-reading them; the next two were found by tooling; the last is to wording I wrote while
making the first four.

- The deadline test's comment had the arithmetic wrong. `SteppingClock` post-increments, so at
  200 ms/read against a 250 ms budget the entry sample reads 200 and fixes the deadline at 450;
  the next sample reads 400, which is *not* past it. **Two** entries are processed, not one. The
  assertion was a loose `< 49`; since the arithmetic is fully deterministic it is now `== 2`.
- `a_cursor_that_spans_budget_bookkeeping_still_mints_a_budget_from_its_inventory` claimed mint
  *and* entry activity. Its surviving-cursor leg exercises mints only. Budget entry is not
  independently reachable - `enter_studio_epoch_budget` is internal and always runs inside
  `edit_studio_epoch`, which writes a record and so legitimately invalidates the cursor - so
  entry appears only in the control leg, where the refusal is the point. The doc comment now
  says exactly that, and an inline comment that contradicted itself two lines later is fixed.

**A third correction, to my own fix.** Splitting the old `|`-separated format string into tuple
fields silently dropped `entry.document.doc_type`. Nine fields went in and nine came out - the
count still looked right - so the loss was invisible on inspection and no test noticed, because
every record in that fixture happens to share a `doc_type`. It is restored as field three. The
general lesson, which is why it is recorded rather than quietly fixed: a formatted string with
*n* separators and a tuple with *n* elements are not evidence of the same *n*.

**A fourth, found by clippy rather than by a test.** The equivalence test tracked the budgeted
scan's progress in a variable reassigned at three points in the loop, including once after each
detached install. Clippy reported the post-install assignment as never read - correctly: the
next iteration's assignment always overwrote it first. The comparison *was* reading the right
value, because the final step's progress reflects every install, but by accident of control flow
rather than by construction, and the post-install capture I thought I was making did not exist.
It is now a single read of the cursor's own counters taken immediately before `finish` consumes
it, which cannot be wrong in that way. Recorded because a passing test said nothing about this;
the lint did.

The same pass gated `collect_cursor_creative_references` and `finish_cursor_creative_references`
behind `#[cfg(test)]`. No production path drives an owned cursor yet - `creative_pinned_cids`
reaches the same two cursor methods through `EpochStorageScan`, which holds the borrow - so
ungated they were dead code in a shipping build. The test loses nothing: it still exercises the
same cursor methods production uses. They ungate with the runtime adoption.

**A fifth, to a claim in this same pass.** I wrote that the expected byte total was "anchored
independently of either scan". It is not - it is derived from the budgeted inventory's own
records. What it actually provides is a *different code path*: per-record footprints produced by
validation, versus counters accumulated by the scan. That still defeats a mutation skewing both
scans identically, which is the property that was wanted, but it is not external anchoring and
the comment no longer says it is.

The deadline's public contract is now stated on `step_epoch_storage_scan` rather than implied:
the deadline is checked between entry-processing units, with a one-entry minimum for a nonzero
step allowance, and individual entry work is not preempted. A visit can therefore overrun its
budget by the cost of whichever entry was in flight; callers needing a hard ceiling must bound
`steps` as well.

### The full parallel library suite is not a clean signal on this machine

Two consecutive full `--lib` runs at the same tree, default thread count, `-j 2`:

| Run | Result | Failures |
|---|---|---|
| 1 | 710 passed, 1 failed, 11 ignored (587 s) | `scheduling::studio_actor_owner_return_..._cancelled_preview_transport` |
| 2 | 707 passed, 4 failed, 11 ignored (613 s) | the three `studio_actor_owner_return_...` cases and `succession::joining::studio_actor_post_succession_joiner_reads_open_history_provisionally` |

**A different subset each run, and all four pass serially in 35 s at that same tree.** The
failures report `passes=160/160, injected=40000/40000 ms`, against a budget whose own comment
records a healthy run as 91 to 132 passes: the loop was exhausted.

**That is an observation, not a diagnosis, and an earlier version of this section wrongly stated
it as one** - "the scheduled actor starving for wall clock" - which is exactly the inference
already withdrawn under SCHED-DIAG-001. Exhausting the pass loop is consistent with several
causes. A varying failure set under parallelism together with a serial pass establishes
**sensitivity to execution conditions**; it does not identify which resource or which transition
was responsible. It is consistent with the already-tracked `studio_actor_owner_return` Linux CI
failure and the known `studio_exchange::tests::unopened` behaviour, and the owner-return
investigation remains open and is not settled by anything here.

This work cannot be the cause, and that is checkable rather than asserted: the uncommitted
production delta is one doc comment plus `collect_cursor_creative_references` and
`finish_cursor_creative_references`, which no production path calls - only the new test does.
Everything else in the diff is inside `mod tests`.

**Consequence for evidence in this ledger:** a green full-suite number from a default-threaded
run is not reproducible here, so it should not be quoted as one. The ledger's existing rule -
`-j 1`, no concurrent Cargo work - exists for this reason, and I did not follow it for these two
runs. The C-3 acceptance evidence is the serial run: `store::epoch_recovery::inventory` plus
`store::epoch_studio::tests` at `--test-threads=1`, **167 passed, 0 failed, 3 ignored**.

## Design 13.1, 13.3 and 13.8: already measured, never mapped to the obligation

Before building anything further I checked what the existing profiles already report. Two of them
- `OVERLAY_PROFILE` and `HANDOFF_SIGNING_PROFILE`, both written during Flow H and predating
design 13's list - turn out to cover substantial parts of three obligations. Nothing new was
needed; what was missing was the mapping.

Release, isolated of other test work:

| shape | ops | intent bytes | decode | draft | candidate | **C-3 inventory** | durable handoff |
|---|---|---|---|---|---|---|---|
| Flipnote | 1 | 1 524 | 1 ms | 1 ms | 2 ms | **0 ms** | 37 ms |
| Flipnote | 32 | 9 048 | 22 ms | 20 ms | 37 ms | **1 ms** | 113 ms |
| Flipnote | 256 | 63 636 | 578 ms | 590 ms | 925 ms | **2 ms** | 2 083 ms |
| Index | 1 | 1 433 | 1 ms | 1 ms | 2 ms | **1 ms** | 40 ms |
| Index | 32 | 9 794 | 16 ms | 17 ms | 23 ms | **0 ms** | 85 ms |
| Index | 256 | 70 430 | 523 ms | 459 ms | 495 ms | **2 ms** | 1 113 ms |

**The mapping of these to 13.1's stages is WITHDRAWN.** An earlier version of this section called
13.1 "measured" on the strength of the table above. It is not, because the intervals these
profiles time are not the intervals 13.1 names:

| printed as | what is actually timed |
|---|---|
| `decode_ms` | the whole of `load_epoch_intents` |
| `draft_ms` | an on-demand `local_draft` reconstruction |
| `candidate_ms` | the **synchronous compatibility** `prepare_handoff` |
| `inventory_ms` | the external `budget(...)` test helper |
| `handoff_ms` | the whole synchronous handoff adapter |
| `max_turn_ms` | one direct core `sign_next` call |

The synchronous adapter performs H1, detached-stage work **inline**, unrestricted signing,
assembly and H5 under a single call, so its elapsed time is not "the H5 portion". One core
signature is not a scheduled H3 visit, which may cover several signatures plus application-level
checks. And `inventory_ms` times a test helper's call, not the inventory on the path a handoff
takes.

So these are **useful component measurements under their own function boundaries**, and that is
how they are recorded. 13.1 remains **incomplete**: it needs the actual H1, scheduled-H3-slice and
H5 intervals at 1/32/256 with the inventory separated on the corresponding path.

**Labelled as what it is:** separate inventory/budget-**helper** time compared with the complete
**synchronous** handoff call - 2 ms against 2 083 ms at 256 operations. That is **not** a
measurement of the scheduled H5 share, and not "the inventory a handoff actually pays"; an earlier
version of this paragraph said both.

Two further caveats, because it would otherwise read as retiring C-3's premise. **This fixture's
vault is small** - `source_bytes` 1 607 describes one *source record*, not the whole vault, which
also holds the intent record and other retained metadata. And 13.7 measured a 4 MB Studio record's
validation alone at 24 ms and a 128-frame record at 197 ms. So the inventory helper is cheap
*here*, and 13.7 says what one record can cost. The two are about different quantities: this
measures a helper call on a small fixture; 13.7 measures a single record's validation.

**13.3 - worst single-signature time, since the deadline is checked between signatures:
`max_turn_ms = 1` at 256 operations**, for both Index and Flipnote. The signing loop already
asserts that no turn exceeds one operation, so this figure comes from a test that runs in the
ordinary suite rather than an opt-in profile. **Partial**: these are roughly 100-byte title
operations and a small roster, not 13.3's "largest admitted individual operation and roster
shape".

**13.8 - synchronous component evidence only; "mostly measured" is withdrawn.** `prepare_ms` 529,
`finish_ms` 244 and `max_turn_ms` 1 for Flipnote are real figures from the direct signing loop, and
they are not the scheduled handoff's wall clock. `256 x max_turn_ms` is the maximum times the
count, **not** the sum of the 256 signature durations; and the direct loop excludes actor receive
cadence, queued visits and waits. So the missing piece is not only the visit count 13.8 names -
the scheduled elapsed time itself is absent.

## Design 13.4: retained input and output, with the stage and lifetime table

13.4 asks for the sum of the accounted bounds on retained input and output within one permit, and
says explicitly that this is **not a measured heap ceiling**. So it is arithmetic over the caps,
not a profile, and it can be completed by reading them.

This is the **fifth** attempt. The previous four were each refuted by review, and the trail below
records what each got wrong, because a section this often mistaken should be audited against its
own history rather than read as if it had always said this. What changed this time is that the
rows come from the functions that build the objects and from the stage that writes them, not from
the struct definitions - the two places the earlier attempts read instead.

**Scope, declared.** One permit - one handoff job spanning H1 capture, H2 prepare, the H3 signing
turns, H4 assemble, H5 commit, plus a C-3 parked body during adoption. The figure below is a
**sum of accounted bounds**, exactly as 13.4 words it: rows that never coexist are still both
counted, so it is an accounting sum, **not** the peak and **not** a heap ceiling.

It is also **not an upper bound on peak residency**, which an earlier draft of this paragraph
claimed. The sum covers 13.4's named terms only; the stage tally below shows transients that are
outside the list entirely - four snapshot-sized write buffers in H5, two full document clones in
H2 - and nothing stops those from exceeding the difference. A sum over a chosen list bounds
nothing it does not enumerate.

### Two of the four "missing representations" I reported were misread

The previous round listed four retained representations as omitted. Two of those were wrong, and
the error was reading struct definitions instead of the functions that build them.

`prepare_handoff_detached(self, mut source: StudioEpoch, ledger: IntentLedger, ..)` takes all three
**by value** and moves them into `StudioHandoffSigning { changes, metadata: self, ledger, .. }`
(`overlay/handoff/preparation.rs:95-131`). `PreparedOverlayChanges::prepare(mut source, ..)`
likewise moves the source into `Self { source, .. }`. So:

- "`StudioHandoffSigning` **clones** metadata and ledger" - wrong at that site; both are moves.
- "`PreparedOverlayChanges` holds a **second** `StudioEpoch`" - wrong; it is the same one, moved.

The substance survives relocated, which is why the rows stay. The clones are real but happen one
level up, at the **call site** in H2: `state.handoff_metadata()..clone()` (`handoff_capture.rs:282`)
and `state.ledger.clone()` (`:302`). `StudioHandoffPlan` then retains `state` *and* `signing`
together, so the permit really does hold the ledger twice and the overlay metadata twice - just not
for the reason first given. The other two rows were verified correct as written.

### Stage and lifetime, with move relationships preserved

`->` means moved, not copied. Only `clone` rows are extra live copies.

| stage | created | fate | retained after the stage |
|---|---|---|---|
| H1 capture | `intent_bytes`, `source_bytes` (`Zeroizing`) | **borrowed, never moved out**: `prepare(self)` decodes from `&self.intent_bytes` and `&self.source_bytes` and only `stamp`, `basis` and `authority` leave `self`, so both buffers stay live through **all** of H2 and drop at its end | neither |
| H2 decode | `state: EpochIntentState` | -> the plan | state |
| H2 clone | `metadata` **clone** of the state's overlay (`:282`), `ledger` **clone** (`:302`) | -> signing | both, beside the originals in `state`; jointly bounded by one record |
| H2 successor | `source: StudioEpoch` from the snapshot | -> `changes.source` | the successor |
| H2 probe | `self.clone().set_prepared(..)` framing probe (`preparation.rs:115`); `source_hash`'s transient full snapshot (`:112`) | discarded in-statement | none |
| H2 change set | `graph` (doc clone), per-op `staged` (second doc clone), `probe` (a full decoded second `EpochGate`), projection clones, `operations` map | "die here, before signing" (`epoch/handoff/preparation.rs:98`) | none |
| H2 batch | `pending: VecDeque<UnsignedChange>` - **raw deltas plus decoded `DomainOp`s**; the encoded form built for the admission probe is dropped (`:80-91`) | -> signing | the deltas |
| H3 turns | one `delta.clone()` per turn (`:119`) | pending entry popped as `signed` grows | pending + signed, together ~N ops |
| H4 assemble | `prepared_state`/`completed_state` **clones** (`:244`,`:246`), two record encodings taken for `len()`, a source-record encoder holding a **second copy** of the snapshot | clones and buffers dropped; only `len()` kept | `candidate`, `snapshot`, `prepared`, `state`, three `u64` |
| H5 commit | **the busiest stage, not an empty one.** A second full `checked_studio_source` restore then dropped (`handoff.rs:320-325`); a full source plaintext read for the before-hash (`:326-330`); two more intent plaintext reads (`:339`, `:394`); Prepared **encoded** for its hash (`:400`) and **again** to write, where `plain`, `sealed` and `framed` are simultaneously live (`epoch_intents.rs:714`); `save_studio_source_checked` taking a **fresh** `unit.snapshot()` and copying it into `plain`, `sealed` and `framed` while the commit's own `snapshot` is still alive for the capability hash (`:413`); then `resolve_studio_handoff_with_io` doing another intent decode, another full source restore, another snapshot re-encode, a `complete()` that is `self.clone()`, and the Completed encode/seal/frame triple; then one further `checked_epoch_replay_state` | writes | - |
| C-3 adoption | one parked body | held across the turn | the parked body |

The H1 and H5 rows are both corrections. H1's buffers were described as released when `prepare`
consumed `self`, which is true only at the *end* of H2 - they are in fact resident throughout the
change-set loop, which is what makes H2 large. H5 was recorded as creating nothing, which was the
worst error in the table: it is the stage that encodes both records and writes the source.

### The sum 13.4 asks for

Over exactly the eight items 13.4 names:

| # | item | bound | basis |
|---|---|---|---|
| 1 | captured intent plaintext | 5 243 904 | intent `MAX_RECORD_BYTES` |
| 2 | captured source plaintext | 8 388 568 | `source::MAX_RETAINED_BYTES` 8 MiB is the bound H1's read **actually applies** (`handoff_capture.rs:392`), and it bounds the file, so the plaintext is that less the 40-byte seal |
| 3 | decoded state | 5 243 904 | *proxy*; the record cap covers ledger + overlay jointly |
| 4 | restored private successor | 9 527 424 | *proxy*; `MAX_STUDIO_EPOCH_SNAPSHOT_BYTES` |
| 5 | signed candidate | 9 527 424 | *proxy*; same |
| 6 | encoded Prepared record | 5 243 904 | intent `MAX_RECORD_BYTES`; **built twice in H5** |
| 7 | encoded Completed record | 5 243 904 | same, via the same writer |
| 8 | C-3's one parked body | 18 876 416 | exact; `MAX_RECOVERY_SLOTS_BYTES + 1024` |
| | **sum of the accounted bounds** | **67 295 448** | **64.18 MiB** |

**Rows 6 and 7 were 8 bytes and that was wrong** - the fourth error in this section, and an
instructive one. `StudioHandoffCommit` really does keep only `prepared_bytes` and `completed_bytes`
as `u64`, so "the implementation does not retain the encoded records" was true of *that struct*.
But the declared scope is the **permit**, not a struct, and within the permit both encodings are
built: H4 builds each to take its `len()` (`handoff_capture.rs:248-249`), and H5 builds Prepared
again for its hash (`handoff.rs:400`) and a third time to write, where `plain`, `sealed` and
`framed` are all live at `mutation.write` (`epoch_intents.rs:714`); Completed goes through the same
writer. Charging 8 bytes also applied the *opposite* rule to the one this section declares: rows 1
and 2 are charged at full cap precisely because "rows that never coexist are still both counted".
Two rules in one table is not an accounting.

**The correction trail, kept so the rows can be audited against what they replaced.** Four
attempts preceded this one, and every one of them was refuted by review rather than by me:

| attempt | figure | what was wrong |
|---|---|---|
| 1 | 63.5 MiB | charged `MAX_EXTENSION` to whole Prepared and Completed **records**; charged the *Studio snapshot* cap to a "decoded state" that is an `EpochIntentState`; charged two encoded records as not retained |
| 2 | 55.4 MiB | rows 3, 4, 6 and 7 "corrected", but the printed total did not match its own rows, and retained representations were omitted |
| 3 | no total | declined to total at all, and listed four omitted representations of which **two were misread** as clones |
| 4 | 55.26 MiB | rows 6 and 7 at 8 bytes under a rule the rest of the table did not use; row 2 on a cap the read does not apply; H5 recorded as creating nothing; the peak mislocated; one object charged twice |

Attempt 1's criticism "charged two encoded records the implementation does not retain at all" was
itself the error: attempt 1 had those rows closer to right than attempts 2 through 4 did.

**Retained beyond 13.4's list.** Three objects, not the five attempt 4 listed:

| item | bound | where |
|---|---|---|
| the ledger and overlay-metadata clones, **jointly** | 5 243 904 | both are clones of parts of one decoded record whose encoding is bounded together by intent `MAX_RECORD_BYTES` (`handoff_capture.rs:282`, `:302`). Charging `MAX_INTENT_LEDGER_BYTES + MAX_EXTENSION` = 7 405 568 exceeded their joint bound |
| pending + signed | 4 194 304 | *proxy*; `MAX_EPOCH_BYTES` bounds the **encoded** ops the gate admitted, while what is retained is raw deltas plus decoded `DomainOp`s |
| the commit's encoded snapshot | 9 527 424 | `handoff_capture.rs:251`, retained at `:264` |
| | **18 965 632** | **18.09 MiB** |

"The commit's prepared overlay state" is gone from that list because it is **not a fifth object**.
`prepared_manifest(mut self, ..)` returns `StudioHandoffCandidate { source: candidate, metadata:
self }` (`overlay/handoff.rs:559-587`), and `into_parts` hands that same value to `assemble` as
`prepared` (`:131-133`, `handoff_capture.rs:239`). So the signing's metadata clone and the commit's
`prepared` are one object at two stages - exactly the moved-versus-cloned mistake this section was
rewritten to fix, made again one table lower down.

**Combined accounted retention: 86 261 080 bytes, 82.26 MiB.**

### Where the largest simultaneous accounted set is, by stage

Attempt 4 said "the peak is H4 `assemble`" and justified declining to number it on the grounds
that "the three largest rows are automerge documents". **Both halves were wrong.** Of the eight
rows only 4 and 5 are automerge-backed (`StudioEpoch` -> `EncryptedDoc.doc: AutoCommit`); row 8 is
a plain `Zeroizing<Vec<u8>>` the same section calls exact, row 2 is a plain buffer, and row 3
holds no document at all - `IntentLedger` is a `BTreeMap<Hash32, LocalIntent>` and
`StudioOverlayState` is a seed `Vec<u8>` plus a `Vec<Entry>`. Having priced H4's three states at
15 731 712 in the same paragraph, the refusal to number was selective rather than principled.

So here is the tally, under one declared document proxy: `MAX_CHECKPOINT_BYTES + MAX_EPOCH_BYTES` =
**6 291 456** for one automerge document's content.

| stage | simultaneous accounted items | total |
|---|---|---|
| **H5** source write | candidate 9 527 424, the commit's retained snapshot 9 527 424 (still live for the capability hash at `:413`), a **fresh** `unit.snapshot()`, and its copies in `plain`, `sealed` and `framed`, plus the state with Prepared installed 5 243 904 | **62 408 532** (59.52 MiB) |
| **H2** change-set loop | `intent_bytes` 5 243 904 and `source_bytes` 8 388 568 both still resident, `state` 5 243 904, the clone pair 5 243 904, the successor 9 527 424, `pending` 4 194 304, `graph` and `staged` 6 291 456 each, the probe gate 3 145 728 | **53 570 648** (51.09 MiB) |
| **H4** assemble | three `EpochIntentState` 15 731 712, `prepared` 2 162 688, candidate 9 527 424, retained snapshot 9 527 424, the encoder's second snapshot copy 9 527 424, one record encoding 5 243 904 | **51 720 576** (49.32 MiB) |

**H4 is the smallest of the three, not the peak.** H5 is the largest, which follows from the H5 row
of the stage table: it is the only stage that writes, and a write costs a fresh snapshot plus a
plain, a sealed and a framed copy of it while the commit's own snapshot is still held.

These totals are **sums of proxies, not heap measurements**, and the proxy is an encoded size while
the objects are decoded ones - an automerge document's heap footprint can exceed its encoded size.
They also exclude the transients H5 releases between steps. The ordering is what they support; the
absolute figures are indicative.

**The parked body is 28.1% of 13.4's list** and 21.9% of the combined figure.

**Withdrawn permanently:** "the implementation is better than the design's list by ~10 MiB". The
gap it rested on was the two length-only rows, and those rows were the error. The implementation
retains **more** than 13.4's list accounts for, not less.

**The currency, where a proxy is used.** Rows charging an encoded cap for a decoded object are a
declared proxy, not a measurement. 13.4's "not a measured heap ceiling" licenses encoded-byte
accounting - it does not make an encoded cap an upper bound on heap, nor excuse omitting a
representation. Which rows are proxies is now stated per row rather than asserted in bulk, because
the bulk claim was false.

`MAX_STUDIO_EPOCH_SNAPSHOT_BYTES` is `MAX_CHECKPOINT_BYTES` 2 MiB + `MAX_EPOCH_BYTES` 4 MiB +
`MAX_EPOCH_GATE_BYTES` 3 MiB + `MAX_RECEIPT_BOOK_BYTES` 8 KiB + `MAX_RECEIPT_BYTES` 1 KiB +
`4 * MAX_EPOCH_OPERATIONS` 80 KB + 1 KiB = 9 527 424; the intent record cap is
`MAX_INTENT_LEDGER_BYTES + 1024` = 5 243 904.

### The parked body is large in absolute terms, which is the part that survives

**18 876 416 bytes - about 18 MiB - for one parked record**, because the bound must be the
*largest* family's record cap and Recovery's is three retained snapshots at 6 MiB each. Every
other family is smaller: Registry and Studio 9.1 MiB, DraftArchive ~6.0 MiB, Intents 5.0 MiB,
OwnerReceipts 8.25 **KiB**. Recovery is the largest of the six, which is the only property the
bound needs - and because `validation_fits` is a stub returning `false`, a budgeted scan parks
every uncached record of every family, so the park point is not confined to one family.

It is the one term whose bound is **exact rather than proxied** - a real `Zeroizing<Vec<u8>>` of
authenticated plaintext - which is why it is the figure worth carrying into the runtime adoption
even while the proxied rows remain proxies. It is **28.1%** of 13.4's eight-item sum and 21.9% of
the combined accounted retention, which makes C-3 the largest single accounted term in the permit.

The operational point stands on the absolute figure alone: a runtime holding a parked body across
a scheduler turn holds up to ~18 MiB of authenticated plaintext, and `Zeroizing` governs its
disposal rather than its residency.

This is the concrete cost of the activation requirement this ledger has been carrying as "the
parked plaintext's residency is not charged to section 13.4's retained-input sum". Charging it
costs 18 MiB and makes C-3 the dominant retained-input term in the design. That is worth knowing
before the runtime adoption, not after: a runtime that holds a parked body across a scheduler turn
is holding up to 18 MiB of authenticated plaintext, and `Zeroizing` governs its disposal but not
its residency.

### Three caveats, because a sum of bounds is not a measurement

1. **Rows 3 to 5 use encoded bounds as proxies for in-memory state, and only 4 and 5 are
   automerge-backed.** A restored automerge document's heap footprint is not its encoded size and
   can exceed it; row 3's `EpochIntentState` holds no document, so its proxy is much tighter, and
   rows 1, 2, 6, 7 and 8 are plain buffers whose encoded bound is near-exact. 13.4 says this is
   not a heap ceiling, so encoded bounds are the right currency - but which rows are proxied has
   to be stated per row, because stating it in bulk produced a false claim.
2. **The sum is not a snapshot of concurrent residency, and not an upper bound on one either.**
   Whether all eight terms are ever simultaneously resident is a separate question this sum does
   not answer and 13.4 does not ask; the stage tally above answers it per stage instead. An
   earlier caveat here said "Prepared and Completed are not both live at once", which contradicts
   H4 holding `prepared_state` and `completed_state` together at `handoff_capture.rs:244-249`.
3. **It is a bound on the accounted terms only.** Anything not on 13.4's list - the sealing
   scratch, the write triples, the directory iterator - is outside it by construction, which is
   also why it cannot bound peak residency.

### What that leaves genuinely unmeasured

**One of the eight is complete: 13.4**, and it is the only one that can be closed by reading the
source, because it is the only item that asks for arithmetic over declared caps rather than a run.
The other seven need measurements. 13.5 and 13.7 now have source-correct fixtures and executed
runs against part of what they ask; 13.2 has nothing at all, and it is also the section the
uncovered clauses of 13.5 and 13.7 both defer to, which makes it the load-bearing gap rather than
merely the emptiest cell. An earlier version of this table put 13.1, 13.4, 13.6 and
13.7 in a "done" column; a review rejected all four, and on inspection it was right about each -
13.4 needed three further corrections after that verdict before it stood. The states below are the
four this ledger should have been distinguishing all along:

- **source-correct fixture** - the fixture builds what its label says, verified;
- **executed measurement** - a run produced figures from that fixture;
- **supported conclusion** - the figures bear the weight the prose puts on them;
- **independently closed** - a returned verdict says the obligation is met.

| item | state | what is absent |
|---|---|---|
| 13.1 | **component measurements only; stage mapping rejected** | actual H1, scheduled-H3-slice and H5 intervals at 1/32/256, inventory separated on the handoff's own path. What exists times `load_epoch_intents`, an on-demand draft, the synchronous compatibility adapter and a test helper's `budget(...)` |
| 13.2 | **scoped, nothing measured** | maximal accepted shapes. The axes are enumerated with their caps and all but one shown reachable; "256 maximal-body operations" is **not satisfiable as worded** and needs the 16 384-byte reading, since `MAX_DOMAIN_OP_BYTES` at 256 exceeds every relevant cap; "a large roster" has no figure in the design. No fixture exists |
| 13.3 | **small-shape observation** | the largest admitted individual operation and roster, with its authority checks. 1 ms is a sampled maximum over ~100-byte title operations |
| 13.4 | **complete as the arithmetic 13.4 asks for; not a measurement** | scope declared (one permit, sum of accounted bounds), stage/lifetime table with moves distinguished from clones, **64.18 MiB** over 13.4's eight items plus 18.09 MiB retained beyond its list. The largest simultaneous accounted set is H5's source write at 59.52 MiB under a declared document proxy; H4 is the smallest of the three priced stages. Five earlier attempts at this section were each refuted by review |
| 13.5 | **source-correct fixture on the operation-count axis; one clause uncovered** | the three real production seams are timed at every depth 1 to 255, with custody separated from the detached plan, depth read back through `local_draft()` and the 255/256 premise asserted. "Against a maximal Closing source and seed" is **not** covered - the source is at rotation eligibility, not the byte ceiling - and S3's I-3 hold is bracketed rather than measured |
| 13.6 | **executed measurement; obligation partly unaddressed** | `checked_epoch_replay_state` and the five-family inventory over several large retained branches, both named by 13.6. What exists measures the decoder pair and the two load entry points |
| 13.7 | **source-correct fixtures, executed measurements, narrower conclusions** | accepted-ceiling runs for each family; DraftArchive entirely; a largest-single-step figure at a ceiling rather than at fixture sizes; restart behaviour under a real workload rather than a deterministic guard rotation. **2026-10-08:** Intents now at its ceilings in both modes and DraftArchive at its ceiling in accounting mode; still missing are OwnerReceipts at its cap, DraftArchive in reference mode, and Recovery above 4 MiB |
| 13.8 | **synchronous component evidence** | scheduled end-to-end wall clock and the visit count. `256 x max_turn_ms` is not the sum of the signature durations, and the direct loop excludes receive cadence and queued visits |

**13.2 and 13.5 have nothing at all**, and are the same kind of work: custody per stage at shapes
this suite does not build. Neither is blocked on another agent.

### The restart test is a liveness result, not a restart rate

Recorded correctly here after a review pointed out the mismatch.
`c3_restart_budget_bounds_retries_but_does_not_survive_sustained_writes` rotates a bare guard at
fixed step intervals, drives the job with **`None`** as its budget so nothing ever parks, and
performs no actual write. It establishes two state-machine outcomes - quiescent completes,
overtaken-before-every-step exhausts the budget and reports `Unstable` - and that is a
**deterministic liveness and termination test**.

It is not 13.7's restart *rate*: no workload, no write arrivals independent of scan progress, no
detached-worker timing, and the `Parked` arm is never exercised because the calls are unbudgeted.

## Design 13.2: scoped, not measured, and one clause needs a reading

**Nothing is measured.** What follows is the reachability analysis, recorded because 13.2 is the
section both 13.5's and 13.7's uncovered clauses defer to, which makes it load-bearing rather than
merely empty, and because one of its axes is not satisfiable as literally worded.

13.2 asks for requirement 1's per-stage Flow H custody - H1, one H3 slice, H5, with C-3's
inventory separated - at maximal accepted shapes: "the 5 MiB plus 1024-byte intent record filled
by 256 maximal-body operations, a 2 MiB seed, the 64 KiB combined metadata ceiling, the maximal
accepted projection widths used by the inspection tests, and a large roster."

### The one clause that does not work as written

"**256 maximal-body operations**" cannot mean bodies at `MAX_DOMAIN_OP_BYTES`. That cap is 64 KiB,
and 256 of them is 16 777 216 bytes - over three times the intent record cap the same sentence
names, and four times `MAX_INTENT_BYTES_PER_DOCUMENT`, which is the aggregate canonical
domain-operation bound at 4 MiB. The binding constraint is the aggregate, not the per-operation
cap:

| bound | value | per operation at 256 operations |
|---|---|---|
| `MAX_DOMAIN_OP_BYTES` | 65 536 | not binding |
| `MAX_INTENT_BYTES_PER_DOCUMENT` | 4 194 304 | **16 384** |
| intent record cap | 5 243 904 | 20 484, but this includes framing and author ids |

So "maximal-body" has to mean **16 384 bytes per operation** - maximal subject to filling the
ledger with 256 of them - and the record reaches its 5 243 904 cap only with per-entry framing and
author ids making up the remaining 1 049 600. A fixture built to the 20 484 figure would refuse at
`IntentLedger::prepare`; one built to 65 536 would refuse on the fourth operation. This is the
reading the fixture should adopt, and it is recorded here rather than chosen silently, because
either of the other two readings produces a fixture that cannot exist and would look like an
implementation defect.

### The axes, with their caps and reachability

| axis | cap | reachable | note |
|---|---|---|---|
| intent record filled by 256 operations | 5 243 904 record, 16 384 per op | yes, on the reading above | |
| seed | `MAX_CHECKPOINT_BYTES` 2 097 152 | yes | |
| combined metadata ceiling | `MAX_METADATA` 64 KiB | yes | private to `studio/overlay.rs`; the operative bound on `encode_vault` |
| maximal projection width, Flipnote | `FLIPNOTE_MAX_FRAMES` 999 | yes | |
| maximal projection width, Index | `MAX_INDEX_OBJECTS` 64 | yes | also `MAX_INDEX_PRIMITIVES` on the change path |
| large roster | no constant named | **undefined** | 13.2 says "a large roster" without a figure, so the fixture has to pick one and say so |

### What this is not

It is not a fixture, not a run, and not an estimate of either. `fill_studio_epoch_fixture` - the
source every existing Flow H and Flow S measurement is built on - fills to
`close_candidate_ready()`, which is rotation eligibility at roughly half of `MAX_EPOCH_BYTES`, on
a single-member group with a small projection. None of the five axes above is exercised by it. Any
figure in this ledger that came from that fixture describes a small-shape document, and the
sections that depend on a maximal one say so in their own text rather than relying on this one.

## Design 13.5: Flow S custody per stage, on the operation-count axis only

**Source-correct fixture and an executed measurement of one of 13.5's two clauses.** The harness
is `profile_flow_s_stages` in `epoch_studio/tests/performance.rs`, with
`flow_s_stage_profile_smoke` running the same code paths cheaply in the default suite.

**What the obligation asks for.** "Flow S custody per stage at 1, 32 and 255 accepted operations,
including S1's structural classification, the S1b and S3 basis derivations against a maximal
Closing source and seed, and S3's I-3 holds."

**Covered: the operation-count axis.** Every stage boundary is a real production seam, not a
harness subdivision. The scheduled runtime splits Flow S into a custody visit
(`start_studio_closing_overlay`, which is S0 + S1 + S1b + capture), a detached plan
(`StudioOverlayCapture::plan`, S2) and a second custody visit (`commit_studio_overlay`, S3); the
profile calls exactly those three, so custody is reported separately from detached work as L1
requires and no second algorithm is being timed. The S1b/S3 basis derivation is also available in
isolation as `prepare_studio_closing_overlay`, which is what both stages re-run.

**Not covered: "against a maximal Closing source and seed".** The source is
`fill_studio_epoch_fixture`'s - ten operations carrying 220 000-byte commit messages, filled to
`close_candidate_ready()`, which is *rotation eligibility and not the byte ceiling*. Its physical
size is printed with every run so the gap is visible rather than implied. That clause is 13.2's
subject and is measured there or not at all; this section does not claim it.

**Not measured: S3's I-3 hold as a duration.** The hold is taken inside
`admit_studio_overlay_authoring`, which is called from within the start stage, and released when
the commit returns. Nothing in the production API exposes the moment it is taken, so its residency
can only be bracketed: it is at most `start + plan + commit` and at least `plan + commit`. The
structural fact that it spans the detached stage is asserted by the runtime tests already; this
profile adds the bracket, not a measurement.

### Why this is a curve rather than three points

`catcoms_rt::Clock` is millisecond-only - the ambient gate permits no finer source - so one accept
at one depth is a single tick-resolution reading and carries almost no information. The profile
therefore times **every** accept from depth 1 to 255 and reports the three depths 13.5 names as
points on that curve, each accompanied by a `Spread` over the five adjacent depths.

That neighbourhood spread is labelled `neighbourhood(+-2 depths)` in the output and is **not** a
spread at a fixed depth: it mixes five different depths, so it describes the local variability of
a rising curve, not repeat variance at one point. Distinguishing those two is the whole reason it
carries a label instead of being printed as an ordinary spread.

Two stages *can* be repeated at a fixed depth, and those get real spreads over 16 samples: the
S1b/S3 basis derivation, which writes nothing, and S0 + S1 classification of an accepted retry,
which writes but does not append. The retry path is the one that isolates 13.5's "S1's structural
classification", because `exact_retry` returns before any source read - though it still includes
the one accounted intent write the retry performs, and the figure is labelled accordingly rather
than presented as pure classification.

### What the fixture pins rather than assumes

- **255 is not arbitrary.** `MAX_STUDIO_OVERLAY_OPS` is 256, so 255 is the largest accepted count
  from which a further append is still legal. `assert_overlay_headroom` asserts that the deepest
  measured depth plus one equals the cap, so if the cap moves the test fails rather than quietly
  measuring a different thing.
- **The depth is read back, not counted.** After the curve, `local_draft().unwrap().accepted()` -
  the production reader - must equal the number of accepts performed. A writer's own tally would
  have accepted a fixture that never reached the depth it was timed at.
- **The basis must not move.** Local acceptance writes no source, so the fingerprint derived
  before the first accept is re-derived after the last and required to be equal. If acceptance
  ever began touching the source, every authorization in the run would have been against a stale
  basis and the whole curve would be meaningless.
- **Classification must actually classify.** The start stage asserts `Captured` for a fresh
  operation and the retry asserts `Local`, so neither figure can come from the other path.

### Figures

Release run, 255 timed accepts, `source_bytes = 2 207 858` - which is **23.2%** of studio
`MAX_RECORD_BYTES`, the arithmetic form of "this source is not maximal".

| depth | start (S0+S1+S1b) | plan (S2, detached) | commit (S3) | **custody** = start+commit |
|---|---|---|---|---|
| 1 | 67 | 1 | 73 | **140** |
| 32 | 39 | 51 | 67 | **106** |
| 255 | 47 | **1 689** | 125 | **172** |

All in milliseconds, one sample per depth; the neighbourhood spreads in the raw output agree
(depth 255's plan band is 1 608/1 658/1 689).

| repeatable stage, 16 samples | depth 1 | depth 32 | depth 255 |
|---|---|---|---|
| S1b/S3 basis derivation (raw upper median) | 62 | 62 | **32** |
| S0+S1 on an accepted retry, **including its write** | 22 | 56 | **426** |

**The detached stage is the only one that scales, and it scales badly.** From depth 32 to 255 - an
8x increase - the plan grew **33x**, which is about `depth^1.7`. Both ends of that comparison are
well resolved (50 ms and 1 658 ms raw medians), which is why the ratio is quoted from 32 rather
than from depth 1: depth 1's plan reads 1-3 ms, a raw median of 2 ms, right at the clock's floor,
so a "1 689x" figure taken from there would be mostly resolution artefact.

**Custody is nearly flat: `depth^0.23`**, 106 ms to 172 ms across the same 8x. Start is
`depth^0.11` and commit `depth^0.13`. This is the result the design's structure predicts and the
first measured support for it: L1 accepts unbounded total latency provided custody per visit stays
bounded, and the expensive reconstruction is exactly what S2 detaches. Cumulatively over all 255
accepts, **82.3% of the work is in the detached stage** - 121 052 ms against 25 954 ms of custody.

**Custody is not monotonic, and that is not noise to be smoothed away.** Depth 1 costs *more*
custody (140 ms) than depth 32 (106 ms), and the neighbourhood bands separate cleanly (start
53-67 ms at depth 1 against 35-45 ms at depth 32). The first accept pays cold-cache costs that
later ones do not. Reporting the curve rather than three isolated points is what made this visible.

**The basis derivation is depth-independent, as it should be.** 62, 62, then 32 ms - flat, then
*lower* at the deepest point. It reads the source, and local acceptance never writes the source, so
overlay depth cannot affect it; the drop at 255 is warm cache after 255 preceding reads. This is
a confirmation rather than an anomaly, and it is also the measured form of the invariant the
harness asserts separately by re-deriving the fingerprint and requiring equality.

### One finding against the design's own text

**Design 6.2 says "so S1 is cheap". At depth 255 the accepted-retry path costs 426 ms under
custody**, growing as `depth^0.98` - linear in accepted operations. The design's reasoning is that
classification needs only entry ids, envelopes, `basis()`, author, target and the ledger, all of
which the structural decode yields; that is true of the *comparisons*, but the path still loads the
whole intent record and, on recognising the retry, writes it back.

**What this measurement cannot do is attribute the 426 ms.** The exact-retry path runs
classification and `write_studio_overlay_intent` with no seam between them, so the figure bounds
classification from above and does not isolate it. Since the record being written also grows with
depth, the split is unresolved and is not guessed at here. The defensible statement is narrow:
**the accepted-retry path as a whole is linear in depth and is not cheap at the cap**, whatever the
internal split, and it holds custody throughout.

That matters for the acknowledgement flow specifically, because 6.3 makes acknowledgement-after-
rollover a retry-classification outcome, and L1's acceptance criterion is actor responsiveness.
A 426 ms custody-held operation is not a correctness problem and no claim here says it is; it is a
responsiveness figure that the scheduled runtime's own budget has to be calibrated against, in the
same way 13.3's largest-single-signature figure calibrates `SIGNING_SLICE_BUDGET_MS`.

## Design 13.6: what C-1's structural decode saves

**Executed measurement of the decoder pair; the obligation is only partly addressed.** 13.6 names
`checked_epoch_replay_state` and a five-family inventory over several large retained branches, and
neither is measured - what follows measures `EpochIntentState::{decode, decode_structural}` and
the two load entry points. That narrowing was deliberate and is recorded below, but it means 13.6
is **not** complete, and an earlier version of this ledger's summary said it was.

Isolated workspace, release, 8 trials, pure figures batched 64x and end-to-end figures 16x,
all reported as
`min/upper_median/max` microseconds with the raw millisecond median alongside.

| shape | ops | plaintext bytes | full decode | structural decode | ratio |
|---|---|---|---|---|---|
| Flipnote | 1 | 1 484 | 593 us | 15 us *(raw 1 ms, 4 of 8 samples zero)* | ~40x |
| Flipnote | 32 | 9 008 | 18 484 us | 93 us | ~199x |
| Flipnote | 256 | 63 596 | **447 296 us** | **812 us** | **~551x** |
| Index | 256 | 70 390 | 310 031 us | 796 us | ~389x |

**A full decode of a 256-operation intent record takes about 447 milliseconds. The structural
decode of the same bytes takes about 0.8.** That is the single largest effect measured anywhere in
design 13 so far, and it is what C-1 removes from every metadata reader that does not need a
projection - including the inventory's Intents arm, which is why L5 named that term as the one
scaling with a retained branch.

**The end-to-end pair agrees, which answers the concern that I/O would conceal the difference.**
`load_epoch_intents` against `load_epoch_intents_structural` at 256 operations: 447 875 us against
1 000 us. The read is the same on both sides and is small next to a 447 ms replay, so at this
shape the end-to-end figure is dominated by the decode rather than by the file. At one operation
the I/O term is at the clock's resolution floor and the comparison there says little.

**Scaling.** The full decode grows faster than linearly in operation count - 1 to 32 operations
costs 31x, and 32 to 256 a further 24x for an 8x increase, so roughly `n^1.5` over the upper
range. The structural decode grows more slowly. So the gap *widens* with branch length, which is
the direction that matters: the records where a full decode hurts most are exactly the ones where
structural decode saves most.

**Index is cheaper than Flipnote at the same operation count** - 310 ms against 447 ms - despite a
slightly larger record, which is another instance of the pattern 13.7 found: encoded size does not
predict typed-reconstruction cost.

### What this measurement establishes, and what it does not

**Measured:** the two production pairs, `EpochIntentState::{decode, decode_structural}` on
identical bytes, and `ServerStore::{load_epoch_intents, load_epoch_intents_structural}` through
the real read path. Both pairs differ only by a `replay` flag, so this is a genuine before-and-
after rather than a reconstruction.

**Not measured, and 13.6 names both:** `checked_epoch_replay_state`, which adds
`budget.verify_record` and an intents preflight on top of the decode, and the five-family
inventory arm. The decoder pair is the mechanism by which C-1 affects those, and the saving above
is per intent record read, but neither end figure has been taken. **This is a deliberate narrowing
and is recorded as one**, not an omission discovered later.

**One operation is at the resolution floor:** `pure_structural_us` at ops=1 has four of eight raw
samples reading zero. The ~40x ratio there is the least trustworthy figure in the table.

**Correctness first, in the ordinary suite.** Two tests guard the measurement, and neither asserts
any duration:

- `c1_structural_and_full_decode_agree_on_everything_structural_computes` re-encodes both decoded
  states and compares the **bytes**, which subsumes the basis fingerprint, author, entry ids,
  envelopes, sequences and timestamps, the Prepared and Completed contents, the legacy flag and
  the ledger. An earlier version compared five accessors and left all of that unchecked.
- `c1_structural_and_full_decode_both_refuse_a_tampered_sequence` corrupts the last entry's
  sequence and requires **both** decoders to refuse. Comparing two decoders' output on one valid
  record cannot catch a structural path that stopped checking refusals - such a path agrees on
  every valid input, runs faster, and would be *rewarded* by the timing comparison. This is the
  store-level case; the replication crate carries the rest.

### A P1 in the fixture, found by adversarial review before the numbers were quoted

The first version of this measurement was of the **wrong record format**.
`StudioOverlay::encode_vault` emits version 1, and `StudioOverlayState::decode_vault` takes a
compatibility branch for v1 that hard-codes `prepared: None`, `completed: None`,
`minimum_new_basis_closed_epoch: 0` and `legacy: true`. The shared fixture assembled its record
purely by splicing encoded entries, so it was a legacy record. Three consequences, all bad:

1. the oracle's Prepared, Completed and minimum-basis comparisons were comparing **constants**;
2. the v2 header parse and `validate()` never ran, so a whole class of structural work was absent;
3. v2 structural decode performs `checked_entries` four times against v1's two, so the reported
   structural cost was roughly **half** a real record's and the saving was overstated.

Fixed by splicing `count - 1` entries and adding the last through the real
`StudioOverlayState::append`, which sets `legacy = false`. The total is unchanged, so every
existing consumer of that fixture keeps its assertions, and the record is now the format
production writes. The agreement test asserts the version byte directly, so a silent revert fails
loudly instead of producing plausible numbers.

### A defect this found in 13.7's reporting

`Spread::resolved_for_ratio` tested the **per-unit** figure against one clock tick. For a batch of
64 that demands a 64 ms batch, so a well-resolved 12 ms batch - twelve ticks, among the best
figures in a run - divided down to 187 us and was reported **"unresolved"**. Resolution is a
property of the clock, so the predicate now tests the raw sample, and `Spread` carries the raw
upper median and prints it.

**Consequence: some `deferrable_fraction_of_measured=unresolved` entries in the 13.7 tables above
were suppressed when they were sound.** Those tables are from runs made under the old predicate
and are marked accordingly; 13.7 is re-run in the next checkpoint rather than being patched by
arithmetic here. `a_well_resolved_batch_is_not_reported_as_unresolved` pins all three rules this
predicate has had wrong in sequence: the raw-sum rule, the zero-median rule and the per-unit rule.

## CI: the handoff mutation harness was red from `17dd54fc` to `fee9b993`, and that was mine

**What failed.** The "Studio Closing overlay handoff" workflow failed on all three pushes since
9.1, at the `index-commit` entry, which 9.1 added. Its mutant replaced the call to
`check_index_objects_at_commit` at H5's call site. That left the helper dead, and the workflow
builds with `RUSTFLAGS='-D warnings'`, so the build failed before the test ran.

**Why I didn't see it.** Locally the harness ran without that flag, so the mutant compiled and
read as DETECTED. Worse, the harness stops at its first failure, so in CI none of the entries
after it ever ran: `verified-evidence`, `proof-generation`, and this checkpoint's `proof-digest`,
`plan-intent-digest`, `plan-source-digest` and `successor-probe`.

**The fix.** The mutant now empties the helper's per-object loop from inside the helper
(`.into_iter().filter(|_| false)`), so every item stays used.

**Evidence.** Run locally with `RUSTFLAGS='-D warnings'`, `index-commit` and every entry after it
were DETECTED at their named assertions and passed once restored (2026-10-09).

**A separate, pre-existing CI timeout, not addressed here.** The "Studio Closing overlay
foundation" workflow's `lifecycle-mutations` job has been cancelled at its 60-minute limit on
every push back to at least `83328240`. That harness is shared with Agent 2, so splitting it
across jobs is a decision for both of us; it is listed in next actions.

## Design 18.3 bounded implementation review (2026-10-09, Opus, static): PASS WITH FINDINGS

**No blocker, no high.** Three mediums and five lows. The review covered Agent 1's runtime
boundary at `524315a7` against base `5a899c22`, and is checkpoint 1 of design 18.3 only. It is
**not Gate 4 acceptance**, which stays with Agent 4, and Gate 5 stays closed. F1 to F3 had to be
fixed or explicitly accepted before native Save registers.

Per-item verdicts:
- **PASS:** I-4 and its writer audit; C-3's storage half with the classifier and refused-result
  memo; C-1 (closure preserved); C-4 (closure preserved).
- **PASS WITH FINDINGS:** the runtime (Flows S and H as scheduled jobs, admission, scheduling,
  commit seams, 9.1's no-restore commit).

What the reviewer executed:
- `check-store-raw-fs.sh` and `check-no-ambient.sh`: both passed.
- Four baseline tests: passed.
- Three hand mutations:
  - M5a (turn cap disabled): detected;
  - M5b (deadline disabled): detected;
  - the priority predicate forced to `false`: **survived** (F2).

Everything else, including the classifier's 14 mutations, was checked by inspection.

| finding | what | disposition |
|---|---|---|
| F1 MEDIUM | S1a's exact-retry acknowledgement replayed the whole branch under custody (`overlay.read`) to return a projection no caller used | **fixed, landing with C-3 step 2** once Agent 2 freed their files (2026-10-09). They reviewed the diff against their files statically and found no change to their Save's outcome contract |
| F2 MEDIUM | N31 is not the accepted actor-level test, and `handoff_priority` is unpinned | **open, its own push next.** Staging real priority work mid-signing needs a second member delivering inbound or a page request, a fixture of its own. Agent 2 has freed `catchup/tests.rs` for it |
| F3 MEDIUM | H5's custody terms are understated, and bounded custody is not established | **measured, deviation recorded**; see "H5's repeated terms, priced" below |
| F4 LOW | a Save captured while the receiver is paused strands admission, a pool slot and a media hold | **fixed, `2b9ba0ae`**, in Agent 2's chosen shape, at both entry points (below) |
| F5 LOW | the refused-result memo tests never checked what the memo returned | **fixed, `e01113a5`** |
| F6 LOW | the raw-fs gate's allowlist counted lines per file | **fixed, `c61a8560`** |
| F7 LOW | "each check redundant by construction" was untrue at the link byte | **fixed, `7258b525`**, with a correction of the finding itself (below) |
| F8 LOW | no executed evidence for design M1, M2 and M6, and no fixture for them | **fixed, `e5bb38ef`**, and M6's guard turned out to be unbuilt (below) |

### F1: S1a acknowledges without rebuilding (local, held)

`StudioOverlaySave` gains `Acknowledged { basis, accepted }`. Both fields are structural facts of
the stored branch, and S1a returns them with no replay. Agent 2's Unconfirmed receiver maps the
new variant to the `Saved { basis, accepted }` it already reported, so their outcome is unchanged.

**Who could notice (checked 2026-10-09, at Agent 2's request).** No production code receives a
retry's outcome expecting a draft:
- the Tauri workspace and `bins` never name `StudioOverlaySave`;
- `Server::save_studio_closing_overlay` has no caller outside tests;
- the Closing receiver's `save_overlay` has none at all.

When native Save registers, its Closing result must map `Acknowledged`, as Agent 2's Unconfirmed
result already does. That is with Agent 4.

**Contract change, recorded:** an exact retry no longer returns `StudioOverlaySave::Local`. Seven
tests asserted the retry's projection. They now take the acknowledgement and compare the stored
draft, read back through `local_draft`, which checks the same property (the retry left the
authored draft as it was) against what is on disk.

The regression is `studio_overlay_store_exact_retry_rebuilds_no_draft`:
- **it counts every reconstruction.** The counter sits inside `StudioOverlay::read`, behind the
  replication crate's test-only `test-counters` feature, which only `catcoms-app`'s
  dev-dependency enables. A first version counted only `local_draft` calls, which missed a direct
  `overlay.read` and the full decode's replay (review of these fixes, M-2);
- its control reads the draft and must move the counter;
- it failed before the fix;
- CI's overlay harness gains `retry-rebuild` (through `local_draft`) and `retry-read` (a direct
  `overlay.read`, the defect's own form). Each puts the rebuild back beside the
  acknowledgement, so only the counter can catch it. Both DETECTED locally.

Held because it touches Agent 2's `receiver/unconfirmed.rs` (one match arm), two of their
Unconfirmed Save test files and `studio/copy/tests.rs`. It is ordered last in the local line, so
everything else ships without it.

**13.5's 426 ms retry figure at depth 255 is attributed to this replay by reading the code, not
by re-measurement.** No measurement has been taken since the fix. The retry still does two
structural decodes of the intent record and one more authenticated read for the flush.

### F4: a capture made while paused is dropped, and a pause releases a queued one (`2b9ba0ae`)

The shape is Agent 2's choice. The refusal sits at the capture, not at the Save's entry, so an exact
retry of accepted work, answered at S1 before any capture, stays `Saved` while paused.

**What changed:**
- `queue_capture_unless_paused` serves both entry points. While paused it drops a fresh capture,
  answers `Busy` and records no scheduling request, releasing admission, the pool permit and the
  media hold with the capture.
- `pause()` releases a capture still queued, and forgets its request. A detached one completes
  and parks, and the park deadline drops it.

**Tests, each broken on purpose:**
- **The capture check removed:** fails
  `studio_actor_unconfirmed_save_captured_while_paused_answers_busy_and_holds_nothing`.
- **A check at the entry instead:** fails
  `studio_actor_unconfirmed_save_exact_retry_while_paused_is_still_saved`.
- **The pause release removed:** fails both
  `studio_actor_unconfirmed_save_a_capture_queued_then_paused_is_released` and the catch-up level
  `a_pause_releases_a_queued_save_capture_with_its_admission_and_permit`.

**What is not covered:**
- The Closing `save_overlay` has no caller yet, so its use of the shared helper is covered through
  the helper, not end to end.
- The media-hold release is not observed: the catch-up fixture captures a title operation, which
  holds no media.

### F3: H5's repeated terms, priced

The release stage profile now times one of each repeated H5 term on the commit before H5
consumes it, and asserts that H4's carried bytes equal the candidate's encoding. Upper medians of
5 trials, release, **on a host shared with two other agents' builds**:

| shape | H5 commit | one snapshot encode | one `blob_cids` | one seed graph |
|---|---|---|---|---|
| Index, 1 op | 27 ms | < 1 ms | < 1 ms | 1 ms |
| Index, 256 title ops | 83 ms | < 1 ms | 2 ms | < 1 ms |
| Flipnote, 256 title ops | 82 ms | < 1 ms | 1 ms | < 1 ms |
| Flipnote, 32 frames | 35 ms | < 1 ms | 1 ms | 1 ms |
| Flipnote, 128 frames | 64 ms | < 1 ms | 4 ms | < 1 ms |
| **Flipnote, 256 frames** | **113 ms** (max 248) | < 1 ms | 7 ms | < 1 ms |
| Index, 16 PutObjects | 34 ms | < 1 ms | < 1 ms | < 1 ms |
| Index, 63 PutObjects (the cap less the base's one) | 45 ms | < 1 ms | 1 ms | < 1 ms |

"< 1 ms" means every one of the five samples read zero at the clock's millisecond resolution.

What it settles:
- **Reusing H4's snapshot bytes is not worth its boundary change.** Each encode measured under a
  millisecond, at the clock's resolution, at every shape, so three are bounded by about 3 ms.
  Reuse would have the writer trust caller-supplied snapshot bytes. **Deviation recorded:** H5
  keeps re-encoding. (These sources are small: see the source-axis gap in C-3 runtime 15.9.)
- **The projections are the larger repeated term, and at a full frame branch they matter:** three
  `blob_cids` cost about 21 ms at 256 frames. Computing it once would recover about 14 ms. This
  is now a prerequisite of C-3 step 3, not an option (C-3 runtime 15.8).
- **Most of H5's growth with branch length is neither.** 27 to 83 ms from 1 to 256 title
  operations, and 35 to 113 ms from 32 to 256 frames, is not encodes, projections or seed loads.
  What remains is unattributed: barrier 2's evidence comparison, the Prepared and Completed
  intent-record encodes, and larger durable writes. The 1-op floor of about 25 ms is presumably
  mostly the three durable writes with their flushes. That was not measured separately.

So the scan's share in C-3 runtime design 15.1, 125 ms less the commit, depends on the branch:

| branch | commit | share left |
|---|---|---|
| short (1 op) | 25 ms | about 100 ms |
| full title branch | 83 ms | about 40 ms |
| Index at its object cap | 45 ms | about 80 ms |
| full frame branch | 113 ms | **about 12 ms** |

That is less than the 16 ms traversal of an 8 MiB vault.

Two other figures from the same run:
- **H1 costs 29 ms with 63 PutObjects.** It still restores once per referenced object, the "H1
  still restores per PutObject" item under 9.1.
- **The full-profile inventory for that 64-record vault costs 11 ms.**

The full run took 52 minutes, most of it building the 256-frame fixtures, whose Saves are
quadratic in branch length.

### F7: the finding was right about the proof, not about the record

Within the post-write proof, only the size-and-digest comparison sees the trailing link byte:
- `check_studio_intent_link` accepts an unlinked record;
- dropping the link leaves the snapshot hash unchanged.

The new test lands the candidate's own plaintext with its link dropped, and H5 refuses at the
proof. But removing the comparison does not let that record through. The encoding is canonical
(a link is a trailing `1`), so a dropped link always shrinks the record, and resolve's flush-only
fence checks the file length and refuses it later, as "retry file changed". So the CI entry
`proof-digest` pins the refusal **to the proof**, the designed point that spends the budget before
resolve reads anything. It does not pin the refusal itself.

The earlier claim below, "the proof's digest and its field checks are each redundant with the
others by construction", is corrected to: the digest overlaps the field checks except at the link
byte.

### F8: M1, M2 executed; M6's guard built, then executed

- **M1 and M2** (the plan's currency check keeps only the size of the intent, then the source,
  wrapper): `studio_overlay_handoff_plan_is_stale_after_a_same_size_wrapper_replacement` reseals
  each record with one plaintext byte changed at the same size, between H2 and a signing turn. The
  plan must stop being current. CI entries `plan-intent-digest` and `plan-source-digest`, both
  DETECTED at "a stale plan reached a signing turn".
  - **M1 was not in fact redundant** until the review of these fixes (M-1, below). H5's step 6
    compared two of its own reads.
- **M6's guard did not exist.** H1 captured whatever successor was installed, so a missing,
  Faulted, replaced or already-edited successor was refused only by H2's
  `check_overlay_successor`, detached, after the reconstruction the probe exists to spare, every
  probe period. H1 now runs the header classification the eligibility view already uses
  (`overlay_successor_hold_in_vault`: a bounded authenticated read, no restore) and refuses
  before capture. H2's check stays authoritative.
  - **Ordering, as finally placed:** after the live authority mint and before the Index object
    check.
    - A first placement before the authority mint broke
      `..._old_owner_receipt_refuses_even_when_original_author_is_current`, because the probe
      pre-empted the `verify_current_owner` refusal that test isolates.
    - A second, after the Index check, made a non-pristine Index successor pay that check's
      restores every probe period (review LOW-1, below).
    - The move also put the Index object check after the authority mint. So an Index branch
      with both a missing object and a stale receipt now refuses at the mint instead of with
      "unavailable Flipnote", which the eligibility row reports. That affects diagnostics only:
      no test depends on it, and the receiver discards H1's error. It also means a device with
      stale authority no longer pays for the restores.
  - **Contract change, recorded:** a Prepared branch over a faulted source is now refused at H1
    as "successor is not transferable: Fault", the reason the lifecycle row already gave, instead
    of at H2 as "epoch does not accept operations".
  - Pinned by `studio_overlay_handoff_h1_refuses_a_non_pristine_successor_before_capture`; CI
    entry `successor-probe`, DETECTED at "H2 started for a non-pristine successor".

### Review of the 18.3 fixes (2026-10-09, Opus, static): no blocker or high

Reviewed at `0808f5a2..fcda06cd`, static only, because the release profile was running. It found
two mediums and five lows.

**What it checked and found sound:**
- S1a's acknowledgement equals what a reconstruction reports;
- the probe never refuses what H2 would accept, and does not pre-empt the Prepared resolution;
- every new harness anchor is unique and every mutant compiles;
- the raw-fs gate's awk is mawk-safe;
- 15.8's census of 24 guard sites.

| finding | disposition |
|---|---|
| M-1 MEDIUM: M1 was the only guard, since H5's step 6 compared its own two reads | **fixed, `505b3a24`** (below) |
| M-2 MEDIUM: F1's rebuild counter misses a direct `StudioOverlay::read` and the full intent decode's replay | **fixed, with F1 (held)**: the counter moved into `StudioOverlay::read`, plus a second harness entry |
| LOW-1: the probe ran after the Index check's restores | **fixed, `505b3a24`**: the probe now precedes it, and a new test counts zero restores |
| LOW-2: `INTERFACES.md` and `HANDOVER.md` still describe a retry returning the draft | **fixed, with F1 (held)**: in F1's follow-up commit, so they ship with the contract |
| LOW-3: STATUS and doc-comment truthfulness | **fixed here**, except the stale test name in F1's own doc comment, which goes with F1 |
| LOW-4: test level differs from the design (M6 and M1/M2 are store-level, not actor-level) | **recorded** below; the M6 test now requires `SuccessorNotPristine` |
| LOW-5: the raw-fs gate keyed only the matched line of an open chain | **fixed, `505b3a24`**: the open flags that change a file are matched too |

**M-1 in detail.**
- **The gap:** design 9.3 step 6 compares the intent record with the captured values, and the
  design's M1 row calls the digest check redundant on that basis. H5 instead compared its pre-write
  re-read with its own first read, under one exclusive borrow, which cannot fail.
- **The fix:** H5's first read must now be the stamp's intent
  (`StudioHandoffStamp::captured_intent`).
- **The regression:** `studio_overlay_handoff_plan_is_stale_after_a_same_size_wrapper_replacement`
  now also drives the stale plan through signing, assembly and H5, and requires the changed record
  to survive with no Prepared.
- **Executed both ways, at first only by hand.** The two runs made at `505b3a24` both had the
  test's gate assertion temporarily disabled:
  - with the M1 mutant and the fix, the test passed, because step 6 refused alone;
  - with the M1 mutant and the old step 6, it failed at "H5 committed a plan whose intent record
    changed at the same size".

  As committed there, the gate assertion came first, so neither CI nor any test could notice step
  6 being removed. The re-review of these fixes found that (MEDIUM-1). The committed test now
  asserts the intent's gate **last**, after H5, so the CI entry `plan-intent-digest` itself pins
  both guards:
  - DETECTED at the gate's assertion with step 6 present;
  - "did not fail at its intended assertion" with step 6 also removed, since the test then fails
    at "H5 committed a plan".

  Both were executed through the harness on 2026-10-09.
- **What this means for M1:** it is now redundant as the design says. Its entry observes the early
  refusal, and through the reordered test it also guards step 6.

**LOW-4, recorded rather than closed.** Three of these tests sit below the level the design names:
- M6's evidence is store-level: a `Captured` start is what the receiver schedules H2 from.
- M1 and M2 observe the H3 gate's return value, not a counted signing turn.
- Both are the same class as F2: actor-level N31 coverage needs the receiver.

**Re-review of those fixes (2026-10-09, Opus, static plus `cargo tree`): no blocker or high.**
- **Confirmed sound:**
  - step 6's placement: nothing can bypass it, and it changes no legitimate outcome;
  - the probe move;
  - the `test-counters` feature's hygiene: it is absent from every non-test build, catcomsctl
    and the src-tauri workspace included, and `Cargo.lock` and `cargo deny` are unaffected;
  - every harness anchor and mutant.
- **MEDIUM-1:** step 6 had no automatic regression, and these notes overstated the evidence.
  **Fixed,** as described under M-1 above.
- **LOW-1:** the Index check's move was unrecorded. **Recorded** under F8's ordering.
- **LOW-2:** documentation nits. Fixed here; the `INTERFACES.md` placement goes with F1.
- **Residual risks, as it lists them:**
  - a reconstruction moved onto another thread would escape the thread-local counter;
  - seed-only graph loads are not counted;
  - the raw-fs gate cannot see open flags passed as variables.

## Design 9.1, no graph restore on the commit path, built (2026-10-08)

This builds 9.1 as its implementation plan, 9.1.1 revision 2, specifies. That plan's design review
found no blocker; its one high was the repair-prefix defect, fixed first in `904e447f`. It is the
first prerequisite C-3 runtime design 15.7 names for C-3 step 3.

### What changed in H5 (`commit_studio_handoff_with_io`)

- **Before the write: no restore.** H2 now carries `HandoffFacts`: the blake3 of the source
  plaintext it decoded, the stamped physical size, and the restored source's protocol bytes. The
  stamp check H5 already ran proves the source on disk is those bytes. `stamped_studio_source`
  requires the facts to match the stamp, builds the accounting record and runs the fresh budget's
  `verify_record`. The second full read only to hash the source is gone too. The writer always
  replaces: the before-snapshot fact was dropped, because at H5 the candidate always differs.
- **After the write: a proof, not a restore.** `VerifiedPersistedSource` (new
  `epoch_studio/source/persisted.rs`) has private fields and one constructor. That constructor
  re-reads what landed, bounded by the family's sealed cap rather than the 8 MiB retained bound,
  and requires:
  - the writer's version: mount, server, target, physical size and plaintext digest;
  - the landed channel, and a snapshot hash equal to the candidate's;
  - a valid intent link.

  Any failure, including a re-read that does not authenticate, refuses the commit with Prepared
  retained and the budget spent.
- **Resolve takes `Option<VerifiedPersistedSource>`.** `Some` replaces resolve's one restore, and
  `into_checked` rechecks mount, server, target, group and the inventory generation. A verified
  source with anything but Complete evidence refuses without writing. Every other caller passes
  `None` and is unchanged: H1, adoption, rotation, and repair's two calls (Agent 3's file, a
  mechanical edit).
- **The Index object check at H5 is header-only** (`check_index_objects_at_commit`, amendment A1).
  Per distinct referenced Flipnote it checks that the object holds work, plus its intent link.
  H1 keeps the full load.
- **The probe resolves a Prepared branch without a tenure** (step 4b). H1 already resolved before
  it asked for one. So a Prepared record left by a refused H5 no longer holds the target's service
  until a fence runs, when tenure is Unknown or Imported.

### Tests, each broken on purpose

Ten new tests in `tests/rotation/overlay/handoff/persisted.rs`:

- zero restores during H5, for a Flipnote and for an Index with two references;
- a facts oracle against a real restore, extended past the write: the persisted bytes restore to
  the candidate's blob CIDs and snapshot;
- M17's post-write substitution, which flips a receipt-book byte. That substitute clears barrier
  2's fence (asserted as a precondition), so without the proof Completed would be written;
- a written source that does not authenticate is a proof failure, and the budget is spent;
- a stamp refusal with zero restores;
- an Index reference edited between H1 and H5 still committing;
- an Index reference made pristine, relabelled under another channel, or linked to intent
  metadata that does not exist, between H1 and H5, refused at H5 (added after the re-review,
  which found the link check had no test);
- the restart path still restoring exactly once;
- the proof's bindings: target, candidate hash, and generation;
- the verified arm accepting only Complete evidence.

Plus `the_probe_enters_h1_without_a_tenure_only_to_resolve_a_prepared_branch`, a decision table
for step 4b in `studio/receiver/handoff.rs`. All 126 existing overlay tests pass unchanged.

Eight mutations were run by hand, each killed. The files were confirmed byte-exact after the
first six, and restored by hand after the last two:

- the verified arm forced to restore;
- the pre-write restore put back;
- the H5 Index check dropped (killed by the existing
  `studio_overlay_handoff_rechecks_index_object_sources_at_commit_not_only_at_capture`);
- the whole re-read removed;
- the Complete-only rule dropped;
- the generation binding dropped;
- after the review, the H5 Index rule forced to accept, killed by the pristine-and-relabelled
  test;
- after the re-review, the H5 intent-link check deleted, killed by that test's unlinked case.

Four of them are now CI entries in `check-studio-handoff-mutations.py`, each DETECTED locally.

**Two things no mutation can show, and why:**

- **The proof's digest overlaps its field checks, except at the link byte** (corrected after the
  design 18.3 review, F7; this said "each redundant with the others by construction"). The
  plaintext is scope, channel, snapshot and link. The field checks see the first three, but the
  link check accepts an unlinked record, so a dropped link is seen only by the size-and-digest
  comparison, and later by resolve's flush-only length fence. `proof-digest` now pins it. M17
  remains the whole re-read removed.
- **The facts-to-stamp equality cannot be reached from the tests.** `HandoffFacts`' fields are
  private to the capture module, and in normal flows they always agree.

### Implementation review (2026-10-08, Opus, static): no blocker or high

The review confirmed each of these:

- `stamped_studio_source` produces exactly `checked_studio_source`'s `observed`;
- the empty `before` skips no check;
- the writer's returned unit is the one whose snapshot landed;
- the landed snapshot makes resolve's flush-only save equivalent, and an independent fence;
- invalidating the budget is sufficient;
- the probe cannot loop or spin.

Its findings and what happened to them:

- **M1, the post-write test was killed only by its message.** Its link-byte substitute failed an
  earlier decode, so the test did not show the proof was load-bearing. It now flips a
  receipt-book byte, which the fence skips, and asserts that precondition. The docstring and the
  14.2 M17 row were corrected.
- **M2, the "built as specified" claim overstated.** The cheap missing regressions were added:
  - a write that does not authenticate, with the budget spent;
  - pristine and relabelled references at H5;
  - the facts oracle past the write.

  The rest are listed below as not done, and the design header now says so.
- **LOW-1:** the proof's doc claimed it "cannot be paired with another unit". It now says how the
  unit is tied to the bytes, and by what.
- **LOW-2:** the refusal message said "is not the written candidate" even for an I/O failure. It
  now says "could not be proved to be".
- **LOW-3:** H5 read each referenced object twice. It now reads it once.
- **LOW-4:** step 4b's steady-state cost and pacing are recorded in design 9.1.1.
- **LOW-5:** the facts-to-stamp check is half tautological. Recorded, not changed.

**Re-review of those fixes:** no blocker or high. It confirmed that the inline Index rule matches
`studio_object_holds_work` in every case: missing, wrong channel, unreadable and pristine.

- **Its medium:** the H5 intent-link check still had no test. Now covered by the unlinked case,
  and breaking the check fails it.
- **Its lows, all fixed:**
  - the test comment now says what the setup models;
  - the proof's doc names its third builder and says "at construction";
  - `eligibility.rs` says the rule is inlined and must be kept in step;
  - the duplicate-PutObject claim cites the observed refusal.

### What is not done, and what it leaves expensive

- **Regressions the plan listed and this does not have:**
  - **a successor between 8 MiB and `MAX_SEALED_BYTES`**, which needs an 8 MiB Studio fixture;
  - **a repaired destination or repaired reference through H5**, which needs Agent 3's repair
    fixtures;
  - **the probe resolving after a refusal at receiver level, with tenure Unknown**: no receiver
    fixture produces an actor without an observed tenure, and only the decision table covers it;
  - **duplicate PutObjects:** the Save path refuses a second PutObject for an object already in
    the branch as a malformed op. This was observed while writing these tests: a fixture that
    saved two PutObjects for one object failed in the Save helper with "epoch studio: malformed
    op". So the deduplication is defensive.
- **Still expensive:** H5 still does two seed `graph()` loads and several candidate `blob_cids`
  projections, and for an Index one authenticated read per referenced object. H1 still restores
  per PutObject and for an interrupted Prepared record.
- **Not measured:** the commit phase on its own (C-3 15.7, step 2).

### The commit phase, a first look (debug build, 2026-10-08)

`profile_studio_overlay_handoff_stages` (`tests/rotation/overlay/handoff/performance.rs`, opt-in)
times each stage separately, on a fresh vault per trial: H1, H2, H3, H4, the H5 budget's
inventory, and H5 itself. Its smoke run was a **debug** build on a machine that was not quiet. So
these numbers show shape only; the gate needs a release run in a quiet window. Upper medians of 5
trials, in milliseconds:

| target | branch ops | H1 | H3 signing | H5 inventory | H5 commit | H2, H4 (detached) |
|---|---|---|---|---|---|---|
| Index | 1 | 7 | 11 | 1 | 53 | 44, 20 |
| Index | 32 | 12 | 184 | 3 | 100 | 390, 280 |
| Index | 256 | 47 | 1 455 | 15 | 888 | 9 188, 4 410 |
| Flipnote | 1 | 7 | 11 | 1 | 54 | 52, 22 |
| Flipnote | 32 | 11 | 183 | 3 | 97 | 534, 317 |
| Flipnote | 256 | 41 | 1 437 | 15 | 851 | 12 004, 5 639 |

What it already says:

- **H5's commit still scales with branch length** after 9.1: about 0.9 s for 256 operations, in
  debug. Its remaining costs are listed above: the seed graph loads, the candidate's `blob_cids`,
  and three durable writes with flushes.
- **H3's signing is paged across visits by design**, so its total is not one custody hold.
- **These sources are small.** At most 0.2 MB of title-only history, and no frames.

So the release run must add a frame-heavy source, and an Index with many PutObjects, before C-3
15.7 can price what remains of the 125 ms share. **Done 2026-10-09, in release:** see "H5's
repeated terms, priced" under the design 18.3 review, and the full table below.

## Fix: the Studio header readers refused repaired records (2026-10-08)

Found by the design review of 9.1.1 (H-1), in code that predates it. A Flipnote that has been
through a bound repair keeps the repair-bound snapshot prefix (form 3) on every successor, and
`StudioEpoch::restore` reads it through `RepairBinding::decode_prefix`. Two header-only readers in
`crates/catcoms-replication/src/studio/epoch/handoff.rs` accepted only forms 1 and 2:
`VaultShape::read` and `preserves_vault_source`. The effects, all while a full restore of the
same bytes succeeded:

- **A handoff into a repaired destination wedged.** Barrier 2 (`preserves_vault_source`) refused
  as Malformed after Prepared was written. The Prepared resolution's flush, and every ordinary
  write, cross that fence too. So nothing could write that document while Prepared stood, and
  every retry redid all the signing.
- P2's structural checks (`vault_holds_work`, `overlay_successor_hold_in_vault`, and
  `evidence_in_vault`'s read) and copy's probe reported a repaired object as missing or
  unreadable.

**The fix.** Both readers now decode the prefix with `RepairBinding::decode_prefix` and discard
the binding, which neither compares; `preserves_vault_source` already ignored the adopting flag.
The review confirmed the fence need not compare the binding:
- on the handoff path the bytes are already pinned exactly by the write capability;
- on every other path no legitimate write changes the binding while Prepared stands;
- comparing the binding alone would be half a check, since the fence also ignores the receipt
  book and the phase.

`reframe_vault_for_test`, a test-only helper that copies a one-byte prefix, now asserts it is not
given a form-3 source.

**Test.** `studio::epoch::owner::tests::studio_vault_header_readers_accept_a_repair_bound_source`
fails before the fix, and fails with either half reverted. Its review (Opus, static) found no
blocker or high.

**Checks.**
- fmt, clippy, the two gate scripts and `cargo deny` passed.
- The root suite with `--no-fail-fast` passed: 43 binaries, 2 180 tests.
- The tauri `cargo check` passed.
- The frontend suite passed: 1 282 tests.
- The tauri suite passed every binary except six-client at `six_client_recovery.rs:416`. That test
  failed again run alone, twice. A control at `06526bd9`, which does not have this fix, failed
  twice too. The assertion is a 90 s real-clock wait for chat history to converge after a
  partition, which touches no Studio code. So it is the host's known flake, failing more often
  tonight, not this change.

**Open, from the review:**
- **Follow-up tests:** form-3 cases in `owner/tests/eligibility.rs::classify`, holding the
  structural hold, the full hold and `check_overlay_successor` together on a repaired source;
  `evidence_in_vault` and `unconfirmed_base_state_in_vault` on form 3; and an app-level handoff
  into a repaired Flipnote, asserting Completed and P2 eligibility.
- **A design question for Agents 1 and 3:** `check_overlay_successor` and both holds never ask
  whether the overlay's receipt is a repaired loser (`ReceiptBook::is_repaired_loser`).
  Same-tenure repairs are covered, but a cross-tenure Transitioned repair whose source opening is
  the losing receipt appears restorable, and would then accept a handoff of work based on the
  loser. Not confirmed against the design.

## C-3 classifier and refused-result memo, built (2026-10-08)

This builds parts A and B of `GATE4-AGENT-1-C3-RUNTIME.md` section 14. Revision 2 of that section
passed a design review and a re-review with no blocker or high. Store code only; no runtime site
changes. **Step 3 stays gated** (14.5).

### What is built

- **`validation_fits` is calibrated** (`store/epoch_recovery/inventory.rs`).
  - The rule: `(100 us + rate x KiB) x 4` must fit in `min(remaining - 1 ms, 25 ms)`.
  - Families admitted, in accounting mode only: Recovery to 64 KiB at 3 us per KiB; Intents at
    16 us per KiB; OwnerReceipts to 747 bytes; DraftArchive to its sealed cap at no
    size-dependent rate.
  - Never inline: Registry, Studio, and every family in reference mode.
  - The constants and the reasoning behind each envelope are in `inline_calibration`.
- **The validation memo** (`inventory/cache.rs`) gains two operations:
  - `evict_mismatch`, run on every Registry or Studio read: an entry the bytes just read
    contradict is removed;
  - `put_if_vacant`.

  Everything that enters the memo now goes through one helper, `memoize`. Its comment says why
  extending the memo to Intents must carry the intent facts.
- **A detached Registry or Studio result refused as `Invalidated` is memoized.** This happens in
  `install_validated` before the job restarts, and never over an entry already present.
  A runtime that discards a result itself memoizes it through
  `restart_epoch_inventory_job_uncharged(job, pending)`, which step 2's runtime uses.
  `ServerStore::memoize_overtaken_inventory_result` is now test-only and keeps the memo's rules
  testable in isolation. Each memoized result is checked against:
  - the job's own cursor (scan identity, mount, awaited record);
  - the store's current mount.
- **A test-only switch, `detach_every_validation_for_test`, is on `ServerStore`**, so a job's
  restarted cursors keep it.

### Tests whose contract changed, recorded

These plant small records that the calibrated classifier now inlines. They are switched to detach
every validation, because their subject is the detached stage. They now assert "a record the
classifier detaches parks"; what it detaches is the classifier's own tests' job. Each already
asserted that something parked, which is what stops one from passing after it stops exercising
the path.

- `a_budgeted_cursor_parks_each_record_and_completes_through_the_detached_stage`
- `a_budgeted_scan_produces_the_same_inventory_as_an_unbudgeted_one`
- `a_rail_violation_still_refuses_after_a_record_has_been_parked_and_installed`
- `the_aggregate_byte_rail_still_refuses_after_a_record_has_been_parked_and_installed`
- `a_detached_validation_is_refused_by_the_wrong_scan_record_or_generation`
- `driving_a_job_respects_the_visit_deadline_and_stops_at_a_park`
- the 13.7 harness, through `case()`, which covers `c3_visit_profile_smoke` and
  `c3_multi_family_scan_parks_records_from_several_families`

Running the inventory tests with the classifier on and no switch failed exactly these eight, which
matches the review's enumeration (seven tests plus the harness).

### New tests, each broken on purpose

Thirteen mutations were applied, and each was killed. The first twelve were killed by the new
tests; the last by the switched tests and `the_detach_switch_survives_a_job_restart`:

- `INLINE_SAFETY` set to 1;
- the 25 ms cap removed;
- the 1 ms floor removed;
- the envelope check removed;
- Studio admitted;
- `references` ignored;
- eviction on mismatch removed;
- the vacancy check removed;
- the store-mount check removed;
- no memo on the `Invalidated` exits;
- an unchecked memo on a fault exit;
- the remaining time not sampled per record (run again after a formatting restructure of that
  block);
- the test switch ignored, which fails `the_detach_switch_survives_a_job_restart` and the switched
  tests.

After the implementation review, a fourteenth mutation removed `memoize`'s family gate. It was
killed by `a_budgeted_scan_memoizes_only_registry_and_studio_records`. Both files were confirmed
byte-identical to their pre-mutation hashes after each round.

### Checks run

The full run was on the reviewed tree:

- fmt, clippy `-D warnings`, `check-no-ambient`, `check-store-raw-fs` and `cargo deny` passed;
- the root suite with `--no-fail-fast` ran all 43 binaries: 2 178 passed, 0 failed;
- the tauri workspace's `cargo check` passed;
- the tauri suite with `--no-fail-fast` passed every binary except the known six-client flake at
  `six_client_recovery.rs:416`. It failed at the same line in all three full runs today, including
  the run on the test-only harness commit. On this branch no production inventory scan passes a
  deadline, so part A has no production caller yet; part B's production effect is limited to the
  eviction;
- the frontend suite passed: 1 282 tests.

The review's fixes are test code, comments and docs, all in catcoms-app or the docs. For those,
fmt, clippy, the whole catcoms-app lib suite and the six-client test on its own were run again.

| test | what it pins |
|---|---|
| `the_inline_classifier_admits_only_measured_families_inside_their_envelopes` | the rule as a table: each family and mode, envelope edges, the floor, the exact threshold, the cap, and `u64::MAX` |
| `a_small_recovery_record_validates_inline_while_a_cold_studio_record_parks` | inlining through a real cursor; the budgeted inventory equals the unbudgeted one |
| `the_slice_running_down_turns_an_inline_validation_into_a_park` | remaining time sampled per record against a visit's absolute deadline, with exact clock arithmetic |
| `the_detach_switch_survives_a_job_restart` | the switch's placement (re-review MEDIUM-1) |
| `a_refused_studio_result_warms_the_cache_for_the_restarted_job` | part B: the restarted job parks no Studio record and reuses one |
| `a_stale_entry_is_evicted_so_a_refused_result_still_warms` | piece 1 (review HIGH-2a) |
| `a_refused_result_never_displaces_an_entry_put_since_the_read` | piece 3 |
| `an_overtaken_result_from_another_scan_mount_store_or_record_warms_nothing` | piece 2's bindings, fault exits, and vacancy on a second call |
| `a_record_rewritten_after_its_validation_misses_the_warmed_entry` | a rewrite evicts the warmed entry and misses |
| `a_budgeted_scan_memoizes_only_registry_and_studio_records` | `memoize`'s family gate: inlined Recovery and DraftArchive records leave no entry |
| `a_vacant_only_put_never_displaces_an_existing_version`, `eviction_removes_only_a_contradicted_version_of_the_same_record` | the two cache operations directly |

One branch is defensive and unreached by any test: the second `Invalidated` exit, where the
cursor's generation differs from the result's. A result with a matching scan identity cannot reach
it without forging. It memoizes the same way as the first exit.

### What is not done

- **Part B's runtime half: landed with step 2 (2026-10-09).** Its implementation review (M-1) set
  two conditions on step 2's merge, and both hold:
  - the store's uncharged restart takes the pending result and memoizes it before replacing the
    cursor (`restart_epoch_inventory_job_uncharged(job, pending)`);
  - `an_own_write_refresh_memoizes_the_overtaken_result` pins that the refresh memoizes.

  `memoize_overtaken_inventory_result` therefore never gained a production caller. It is now
  `#[cfg(test)]`, kept to test the memo's rules in isolation.
- **Part C and step 3 are not built.** The follow-up measurements are in design 14.7: structured
  Recovery shapes, and version-2 owner journals at their cap.

### Implementation review (2026-10-08, Opus, static): no blocker or high

The review confirmed the code matches design 14.2 and 14.3. It found:

- **M-1, step 2's integration:** recorded above as a gate for step 2.
- **L-1:** the docstring of `a_record_rewritten_after_its_validation_misses_the_warmed_entry` now
  says it pins "a rewrite evicts and misses", not the digest check alone.
- **L-2:** the slice test's docstring now says what it cannot catch: a sample taken at step entry
  rather than at the record.
- **L-3, doc wording:** fixed in the threat model, design 9.2, C-3 section 7 and this entry.
- **L-4:** the cache comments now say what actually keeps a hit correct.
- **L-5:** `memoize` takes the record rather than the whole body, as the design specified. Its
  family gate is now pinned by `a_budgeted_scan_memoizes_only_registry_and_studio_records`. The
  deviation from 14.3 is recorded there.

Its residual gap was that `canonical()` ignored the Intents facts, so a budgeted-versus-unbudgeted
comparison could not see lost Unconfirmed facts. It now compares them too.

### Documentation updated

- Design 9.2, consequence 2: the memo relaxation.
- L6: what "sustained" means, priced.
- The threat model: a new bullet after the I-4 one.
- C-3 runtime: sections 1 and 14.

## Design 13.7, partially delivered: the first measurement in this design

Until now every measurement obligation in design 13 was outstanding and the ledger said so. This
is the first one with numbers behind it. It is **partial**, and the boundaries are stated below
rather than left to be discovered.

### The uncached families at their ceilings (2026-10-08)

This closes the gap section 13 of `GATE4-AGENT-1-C3-RUNTIME.md` names: OwnerReceipts and Intents
had been measured only at trivial sizes, and DraftArchive not at all. With Recovery, these are the
four families `validation_fits` could ever admit inline. Studio and Registry are driven by
structure, not bytes, so no byte threshold is safe for them (see "Registry and Studio measured"
below).

**Conditions.** The user cleared the machine for this window: no other agent was building.
Isolated worktree at `a62a7f80` plus the harness added for this (`profile_c3_uncached_families` in
`inventory/tests/performance.rs`, committed with this entry). Release build,
`RUST_MIN_STACK=33554432`, `SystemClock`, 8 interleaved trials, 64 validation repetitions per
record per trial, validation cache cleared before every trial, page cache warm. Runs 2 and 3 are
the same case set (138.98 s and 141.05 s). Run 4 adds the retained-branch rows in reference mode
(247.03 s), with the rest of the set interleaved alongside them as before.
Run 1 is not a measurement: it panicked in reference mode on a fixture defect. The generic
`document` helper's keys are not a Studio document's, and a reference scan decodes each intent as
a Studio operation. `flipnote_document` is the fix.

**Units.** Validation is the upper median of the 8 per-trial means. Each mean is over a 64-repetition
batch timed on a 1 ms clock, so the resolution is about 15.6 us and "0" means a batch shorter than
one tick. Read-and-park is one sample per trial, so it resolves only to 1 ms; the column shows run 3,
or run 4 for the rows only run 4 has.
"Per KiB" is the run 3 validation figure over physical bytes.

| family, shape | physical bytes | mode | read-and-park | validation, runs 2 / 3 / 4 | per KiB |
|---|---|---|---|---|---|
| Recovery | 1 195 | accounting | 0 us | 0 / 0 / 0 us | - |
| Recovery | 262 315 | accounting | 1 000 us | 171 / 171 / 171 us | 0.7 us |
| Recovery | 1 048 747 | accounting | 2 000 us | 2 359 / 2 406 / 2 468 us | 2.3 us |
| Recovery | 4 194 475 | accounting | 8 000 us | 9 031 / 9 171 / 9 156 us | 2.2 us |
| Intents, 100 intents | 13 556 | accounting | 0 us | 93 / 93 / 93 us | 7.0 us |
| Intents, 1 000 intents | 134 156 | accounting | 0 us | 1 015 / 1 015 / 1 000 us | 7.7 us |
| Intents, 10 000 intents (count ceiling) | 1 340 156 | accounting | 3 000 us | 11 390 / 11 406 / 11 296 us | 8.7 us |
| Intents, 100 intents | 13 556 | references | 0 us | 218 / 218 / 218 us | 16.5 us |
| Intents, 1 000 intents | 134 156 | references | 1 000 us | 2 250 / 2 250 / 2 234 us | 17.2 us |
| Intents, 10 000 intents (count ceiling) | 1 340 156 | references | 3 000 us | 23 781 / 23 937 / 24 093 us | 18.3 us |
| Intents, 64 KiB bodies | 1 049 372 | accounting | 2 000 us | 484 / 500 / 500 us | 0.5 us |
| Intents, 64 KiB bodies (byte ceiling) | 4 197 020 | accounting | 8 000 us | 2 796 / 3 062 / 2 906 us | 0.7 us |
| Intents, Closing branch of 64 ops | 16 330 | accounting | 0 us | 187 / 187 / 187 us | 11.7 us |
| Intents, Closing branch of 256 ops (op ceiling) | 62 218 | accounting | 0 us | 765 / 781 / 781 us | 12.9 us |
| Intents, Closing branch of 1 op | 1 273 | references | 0 us | - / - / 218 us | **175 us** |
| Intents, Closing branch of 64 ops | 16 330 | references | 0 us | - / - / 500 us | 31 us |
| Intents, Closing branch of 256 ops (op ceiling) | 62 218 | references | 0 us | - / - / 1 375 us | 23 us |
| OwnerReceipts, one receipt | 513 | accounting | 0 us | 0 / 0 / 0 us | - |
| OwnerReceipts, receipt and decision close | 747 | accounting | 0 us | 0 / 0 / 0 us | - |
| DraftArchive | 1 048 696 | accounting | 2 000 us | 0 / 0 / 0 us | 0 |
| DraftArchive (payload ceiling, less 1 KiB) | 6 334 584 | accounting | 10 000 us | 0 / 0 / 0 us | 0 |
| Studio, title history, in the branch vaults | 1 359 | accounting | 0 us | 500 / 500 / 468 us | not a byte rate |
| Studio, title history, in the branch vaults | 1 359 | references | 0 us | - / - / 500 us | not a byte rate |

The branch rows in reference mode use run 4's figure for "per KiB". Every reference-mode branch case
was checked against its oracle on every trial: the fixture's operations are title edits, so the
collected set must be empty, and it was.

**What this establishes.**

1. **Intents cost tracks entries, not bytes.** At the byte ceiling, the 64 KiB opaque bodies cost
   0.7 us per KiB. At the count ceiling, minimal title intents cost 8.7 us. A retained Closing
   branch costs 12.9 us at the operation ceiling, which makes it the densest shape per byte. So a
   byte threshold for Intents in accounting mode is safe only at the branch's rate. The
   accounting decode copies a branch's seed as opaque bytes and reads its entries as fixed-size
   fields (`decode_vault_structural`), so a larger seed adds bytes at the opaque rate, not
   structure.
2. **In reference mode, Intents is driven by structure, like Studio.** A one-operation branch
   costs 218 us with references against under 15 us without, which is 175 us per KiB. The cause is
   `base_blob_cids`, which rebuilds the seed's graph (`self.base.graph()`) to enumerate its CIDs.
   This fixture's seed is a title-only history, so its graph is trivial. A dense flipnote seed would
   cost what the same graph costs as a Studio record, and that has been measured at about 240 ms for
   128 frames. So no byte threshold is safe for Intents in reference mode. Without a branch,
   reference collection roughly doubles the count shape's rate (8.7 to 18.3 us per KiB), because it
   decodes every pending intent as a Studio operation.
3. **The most expensive Intents record measured costs 11.4 ms to validate for accounting and
   23.9 ms with references.** That is 10 000 intents with no branch. It is the most expensive
   record of any uncached family measured so far, and it is still an order of magnitude below a
   dense Studio record. Point 2 says why a branch with a dense seed could cost more in reference
   mode.
4. **Recovery repeats the earlier profile.** 9.0 to 9.2 ms at 4 MiB, against 11.0 ms in the first
   profile on a contended machine. Recovery's own ceiling is 18 MiB plus 2 088 bytes, and **it is
   not measured above 4 MiB**. **These Recovery records are opaque projections** (`stage_sized`:
   filler projection; empty tombstones, elements, conflicts and applied operations). The
   accounting decode does work per item, so a structured record can cost more per byte, and that
   is unmeasured. So 2.3 us per KiB is the cost of Recovery's bytes, not a bound on its structure
   (design review of C-3 section 14, HIGH-3).
5. **DraftArchive accounting does no work that grows with size.** The accounting arm calls
   `storage_record` and never decodes the payload (`validate_record_body`). It stays below
   resolution all the way to the payload ceiling. Reading and authenticating the record, which
   takes 10 ms at the ceiling, is the record's whole cost, and no classifier moves it.
   **DraftArchive reference mode is not measured.** The test writer seals opaque bodies, and
   reference mode needs a canonical archive whose payload decodes.
6. **OwnerReceipts is measured only on small journals.** The two journals measured (513 and 747
   bytes) are below resolution. The family's sealed cap is about 27 KiB (journal, close, nine
   receipts, attestations). No fixture builds a journal at that cap, so the cap itself is
   unmeasured. Only accounting mode was run. The decode does not branch on `references`
   (`validate_record_body`), so reference mode does the same work; that is read from the code,
   not measured.
7. **Read-and-park is about 2 us per KiB for every family.** That is 8 ms at 4 MiB and 10 ms at
   6.3 MB. It is paid inline whatever the classifier decides.
8. **The three runs agree within 10% on every resolved validation row.** The largest gap is the
   Intents byte ceiling, 2.80 against 3.06 ms. Contended runs earlier in this ledger moved by up to
   86% on identical fixtures. So treat this as the quiet-machine figure, from one host, in a
   release build only.

These are the **worst rates over the shapes measured**, not a proven worst case for each family.
The classifier proposal in section 14 of `GATE4-AGENT-1-C3-RUNTIME.md` uses them on those terms.

### The result established so far, stated at its actual width

**On Registry and Studio - the families C-3 was designed for - validation is 2.4x to 4x the
read-and-park phase, a 60 to 80% share, and it scales with operation count. On Recovery the two
are merely comparable. Changing `validation_fits` affects only the validation term, in every
case.**

Two earlier versions of this statement were wrong in opposite directions. The first said
detachment "bounds about half" of custody - it does not, and the harness was not measuring total
custody at all (see "Three measurement boundaries, corrected"). The second, after that
correction, said the two phases are "of comparable magnitude", which was true of the only family
then measured and **not** true of the two that motivated the design. Recovery was the
unrepresentative case.

Release build, `SystemClock`, 8 trials, 64 validation repetitions per record per trial, warm
page cache, zero validation-cache hits. Means per record per trial.

**Accounting only** (`RecoveryOnly` coverage, opaque projections - the control):

| authenticated bytes | read-and-park | validation | install | fraction of the two |
|---|---|---|---|---|
| 1 195 | 125 us | 3 us | below resolution | 2% |
| 16 555 | 500 us | 17 us | below resolution | 3% |
| 262 315 | 750 us | 355 us | below resolution | 32% |
| 1 048 747 | 2 375 us | 2 830 us | below resolution | 54% |
| 4 194 475 | 9 000 us | 11 009 us | below resolution | 55% |

**Reference collecting** (five-family coverage, canonical projections, distinct CIDs):

| frames = CIDs | authenticated bytes | read-and-park | validation | install | fraction |
|---|---|---|---|---|---|
| 1 | 574 | 875 us | 7 us | below resolution | 0% |
| 16 | 4 834 | 750 us | 50 us | below resolution | 6% |
| 128 | 36 642 | 875 us | 480 us | below resolution | 35% |
| 512 | 145 698 | 1 375 us | 2 228 us | below resolution | 61% |

**The two read-and-park columns are not comparable.** The accounting scan is `RecoveryOnly`; the
reference scan is necessarily five-family, so its step traverses more of the directory. That is
a coverage difference, not a size effect, and it is why the reference table's read-and-park
barely moves with record size.

**Three things this says that the first profile could not.**

1. **Reference collection is the expensive validator, by roughly an order of magnitude per
   byte.** Accounting runs about 1.4 to 2.6 us per KiB; reference collection about 13 to 15.
   The design's expensive case is the one that was previously unmeasured.
2. **Reference-collection cost tracks the reference count rather than bytes.** A structural
   axis, not a byte axis. Three measurements of the identical fixtures at 16, 128 and 512
   references: 3.1 / 3.8 / 4.4 us each block-ordered, then 4.8 / 6.5 / 8.1 block-ordered again,
   then **2.9 / 3.2 / 3.7 interleaved**. The direction is consistent across all three; the rate
   is not, and the interleaved set is both the lowest and by far the flattest - which is what
   the discipline was expected to do.
3. **The installation term is small at these reference counts.** This was the open question
   about the missing phase, and the answer is concrete: `install_validated_record` stayed below
   millisecond resolution even summed over eight trials, including the 512-CID merge. It is
   *not* established for larger reference sets, and the phase is now timed so that will show.

The fraction column converges to 55 to 61% at the large end of both modes. It is still a
fraction of two measured components, not a share of total custody and not a speedup.

**What follows for the threshold, corrected.** The classifier alone cannot meet a custody target
below the current read/authentication cost. The earlier claim that "bounding `steps` and the
per-record byte ceiling are the levers that can" was wrong: entry limits and byte limits bound
**how much work is admitted**, not elapsed time, and this implementation explicitly describes
individual entry work as non-preemptible with a one-entry minimum. Entry limits, byte limits and
these measurements have to be considered together, and a strict latency guarantee would need a
stronger execution model than any of them. **No accepted record-size limit should be changed on
the strength of this first profile.**

### Three measurement boundaries, corrected

**1. The harness was not timing the whole custody path.** It timed one unbatched
`step_epoch_storage_scan` sample and a batched `revalidate`. It did **not** time
`install_validated_record`, `finish_epoch_storage_scan` or `begin`. The displayed percentage was
therefore `V / (R + V)` over two measured components - not a measured share of total scan
custody and not a before/after speedup. The omission matters most for **reference** scans, where
installation merges CID sets and dependency metadata rather than inserting an accounting record,
which is precisely the case the extension below adds. The phases are now named and reported
separately: `read_and_park`, `validation_batch` / `validation_mean`, `install`, `finish`, `begin`.

The module comment also said term 1 was "the visit's measured custody minus" validation. Nothing
was ever subtracted - the step is timed directly. Corrected.

The harness holds `&mut ServerStore` for the whole profiling run. These are timings of
prospective stage bodies, **not** observed actor custody releases.

**2. "Neither family nor size is known before authentication" was wrong about this code.** The
scanner takes a candidate family from the **filename**, an untrusted size from
`symlink_metadata`, and already applies that family's `sealed_cap`, the aggregate byte precheck
and `check_cold_bytes` - all before any body is read or authenticated. Using an untrusted size to
*limit* work is not the same as using it to *accept* a record as authentic, but it is a
scheduling decision that does precede authentication.

So the correct statement is: `validation_fits` is *currently called* after read/authentication,
and moving its threshold cannot move that preceding work. The term is **retained by the current
read/authentication boundary**, not "unavoidable" - the earlier wording turned an implementation
boundary into a claimed impossibility. Nothing here proposes moving authentication; that would
be a separate design question about key custody, input binding and lifecycle fences.

**3. The sampling was asymmetric and the conditions were unstated.** Validation was repeated 64x
on one **resident** plaintext; read-and-park was sampled **once** per record; the files were
written immediately before the scan, so the page cache was warm. This is a component-cost
experiment, not a cold-storage or worst-case benchmark. "Cold" in `uncached_bytes` and
`check_cold_bytes` means the **validation cache**, which is a different thing from a cold
filesystem cache and must not be reported as one. And `revalidate` times `run(&self)` plus the
drop of its temporary result; it does **not** time the consuming `validate(self)` lifecycle
including release of the parked plaintext. Sharing `run` prevents validator drift - it does not
make the two lifecycles identical.

The harness now repeats `TRIALS = 8` complete scans on fresh cursors, sums the single-sample
phases, and prints the build profile, repetition count, trial count, cache-hit count and page
cache condition on every result line.

### The canonical reference fixture, and one contract it made explicit

The with-references half needs a Recovery record whose projection the inspector will actually
decode: `StudioRecovery::decode_snapshot` parses the projection as a versioned sequence of
`DomainOp`s and validates each one, so the opaque filler the accounting control stages is
refused outright. The fixture therefore builds a real `StudioEpoch`, inserts `frames` frames
each naming a real blob CID, and takes its canonical projection through
`StudioRecovery::snapshot` - the same constructor production uses.

It returns the CIDs it planted, and the measurement **asserts the collected set equals them**
before reporting any timing. A reference scan that returned an empty or short set quickly would
otherwise look like a cheap one, which is the failure mode that makes a reference benchmark
worthless.

**The accounting-only fixture is kept, clearly labelled, not replaced.** Its numbers stay
comparable with the first profile.

**A defect in that fixture, found by its own output.** The first canonical run printed
`frames=512 cids_collected=251`. The blob content was `[(n % 251) as u8; 10]`, so past 251
frames the CIDs repeated: the fixture planted 512 frames but only 251 distinct references. The
set comparison still passed, because both sides are sets - so the *correctness* assertion was
blind to it, and only the printed count gave it away. A reference-count axis built on that would
have been fiction above 251. Content is now `(n as u64).to_be_bytes()`, and both the fixture and
the measurement assert the distinct-CID count equals the frame count, so it cannot recur
silently.

Building it surfaced a contract worth recording: **reference collection is full-coverage only.**
`collect_creative_references` refuses any coverage narrower than the five-family one, because a
partial inventory must not be allowed to replace a transient pre-publication hold. The first
version of this harness asked for a reference scan at `RecoveryOnly` and was correctly refused
with "reference scan requires fresh full inventory". That also means nothing is cacheable during
a reference scan - `cacheable` requires `references.is_none()` - so in that mode every record
parks on every trial, which the fixture now asserts rather than assumes.

### DraftArchive N17: CLOSED by Agent 2 at `e60d8315`

The handover was sent and answered. Agent 2 added
`every_archive_write_shape_rotates_the_inventory_generation`, covering **all three** mutation
shapes - fresh replacement, the exact-retry flush that changes no bytes but must still rotate,
and the attempt that fails before placing any bytes. They confirmed the third is the only one
testing *ordering* rather than presence, and mutation-verified it by moving the guard after the
refusal: that test fails and only that test, because a writer rotating solely on success
satisfies the other two while still leaving a scan captured before a failed write believing it
is current.

**So every one of the six inventoried families now has its N17 writer obligation asserted**, and
the C-3 table row above is updated accordingly. My part was the family-agnostic cursor test; the
per-writer assertions were theirs, which is the split the mischaracterisation below had wrong.

They also confirmed, which I could not see from my side: **no production caller of either
mutating archive entry point exists yet** - both are `expect(dead_code)` outside `cfg(test)`, and
the disposal transaction will be the writer's first. So the requirement-3 contract confirmation
had nothing outstanding to chase.

Two things they handed back, both worth keeping:

- **No CI status exists on any of their reviewed commits, and every run of theirs used
  `RUST_MIN_STACK=33554432`.** That is a workaround rather than a clean result, and it is a
  shared-environment fact: see "the aborts" below, where it turns out to bear directly on my own
  unexplained profile failures.
- **A clippy warning in *my* file**, `performance.rs:1985`, `push` immediately after
  `Vec::new()`. Mine, introduced by the grouped-profile restructure - and I had reported "clippy
  clean" from a run made *before* that restructure and never re-ran it. Fixed. The lesson is
  narrow and repeatable: **a gate result is only evidence for the tree it ran against**, which is
  the same mistake as the stale build fingerprint in a different coat.

### A test that passes for a reason other than the one it names

Agent 2 asked for this to be in a document rather than a message, and they are right that it is
worth more than a footnote. Twice in this work a test passed while checking something other than
its stated claim:

- the bare-guard cursor test asserted the refusal message "restart required". The cursor refused
  correctly, but that is the **poisoned-rail** message; invalidation reports "invalidated by a
  concurrent record mutation". The test would have passed with a rail bug masquerading as an
  invalidation.
- the same test's finish assertion was `is_err()`, which holds for an incomplete scan whether or
  not it was invalidated - so it passed with the invalidation check deleted.

Both were caught by reading rather than by failing. The general form: **when a refusal has more
than one cause, asserting that it refused is not asserting why.** Distinguish the reason, or the
test covers the union of causes and pins none of them. Agent 2 reports the same failure mode cost
them two rounds on the intent rails.

### The DraftArchive N17 "gap" was mischaracterised - retained for the record

This ledger has repeatedly said "DraftArchive is the one inventoried family N17 does not cover",
and offered it to Agent 2 as something owed. On inspection that is the wrong description of the
gap.

**The cursor side needs no per-family test at all.** `check_not_invalidated` compares exactly one
thing - `Arc::ptr_eq` between the cursor's captured generation and the store's current one - and
that comparison carries **no family information**. A cursor cannot refuse for Recovery and fail to
refuse for DraftArchive; either the writer rotated the token or it did not. The five existing
parked-cursor tests each drive a real writer and so prove two things at once, which is what made
the matrix look as though it needed a sixth row.

`a_parked_cursor_refuses_after_a_bare_guard_rotation_with_no_family_writer` now separates them:
it parks a cursor, takes the mutation guard with **nothing written**, and requires refusal at both
the next step and at issue, with a positive control that the same sequence completes without a
rotation. That makes N17's matrix a matrix of *writer* obligations, and means a new family needs a
rotation assertion rather than a cursor fixture.

**What DraftArchive actually still needs is one line on its writer.** Its *release* path already
has a rotation assertion, in `release_rotates_the_inventory_generation_so_a_scan_cannot_overtake_it`
- Agent 2's test, deliberately written at the generation rather than with a cursor because C-3's
surface was moving at the time. That reasoning was sound and no longer applies, but the test is
right as it stands. Its *writer*, `write_studio_draft_archive_with_io`, has **no** rotation
assertion; `preserve()` calls it without checking the token.

**The outstanding item is three write shapes, not one line.** An earlier version of this section
said "one `before`/`after` pair", which under-specifies it against the standard this ledger
already applies to the other families: the Registry entry in the C-3 table counts the unchanged
exact-retry flush and a *failed* write as part of N17, and the Registry test drives all three.
DraftArchive has three mutation paths - an exact-retry sync, a fresh replacement, and release -
of which only release is asserted. The failed-attempt shape is the one that actually tests I-4's
ordering requirement, that rotation precedes the *first possible* I/O rather than following a
successful one, so it is the shape least safe to skip.

It is Agent 2's writer on Agent 2's fixture and sits in their handover rather than being done
here; what is recorded here is the correct size of it.

**A wrong guess caught by running the test.** The refusal assertion first looked for "restart
required". The cursor refused correctly but with a different message - "invalidated by a
concurrent record mutation". Those are two distinct refusals: a cursor that hit an accounting rail
is *poisoned* and says the former; one overtaken by a mutation says the latter. Asserting the
wrong string would have let a rail bug masquerade as an invalidation with the test still passing,
which is exactly the class of imprecision this ledger has had to correct before.

### Registry and Studio measured, and they invert the Recovery conclusion

These are the families whose expensive typed reconstruction motivated C-3, so they are the ones
whose numbers matter. Release, 8 trials, fresh cache, **all 23 cases interleaved**, reported as
`min/median/max` microseconds per record per trial.

Latest run, with fixture shapes verified and the resolution guard corrected. `(zN)` is how many
of the eight samples read zero milliseconds; `unresolved` means a phase did not resolve beyond
one clock tick, so no ratio is reported for it.

| family | ops (requested = actual) | authenticated bytes | read-and-park | validation batch mean | fraction |
|---|---|---|---|---|---|
| Registry | 2 | 321 650 | 0/1 000/1 000 (z1) | 1 343/**1 859**/2 734 | unresolved |
| Registry | 8 | 1 285 460 | 2 000/**3 000**/4 000 | 4 828/**5 531**/6 406 | 64% |
| Registry | 24 | 3 855 634 | 7 000/**8 000**/9 000 | 14 578/**19 140**/20 375 | 70% |
| Studio (titles) | 3 | 322 505 | 0/1 000/1 000 (z1) | 2 000/**2 312**/2 703 | unresolved |
| Studio (titles) | 12 | 1 768 854 | 3 000/**4 000**/5 000 | 10 046/**12 343**/13 796 | 75% |
| Studio (titles) | 24 | 3 697 338 | 6 000/**8 000**/9 000 | 21 187/**23 890**/29 140 | 74% |

Studio is 24 rather than 32 because the builder truncates above about 25 at this payload; the
shape check now enforces that requested equals actual, and this run satisfied it. The two
smallest rows report `unresolved` rather than a confident fraction, which is the corrected guard
working - their read-and-park is one tick or less.

**Changing the profiling protocol changed the Registry and Studio figures by about 2.5x, and the
block-ordered numbers previously recorded here are withdrawn as portable measurements.** Registry
at 24 ops read 39 056 us under the old protocol and 15 343 us under the new; Studio, 59 099
against 24 609. Recovery moved far less and stayed inside its own spread.

**An earlier version of this section attributed that to "ordering alone" and to measuring "from
the same steady state". Both overclaim, and are withdrawn.** The patch changed several things at
once, so ordering is not isolated:

- execution order became round-robin instead of blocked;
- all stores are now constructed and held for the whole run, rather than built one at a time;
- the three cache modes moved from successive profiles of **one** store to separate stores;
- the reported central statistic changed from an arithmetic mean to an order statistic;
- the delay between building a fixture and measuring it changed, as did the set of live stores.

The supported conclusion is: **changing the profiling protocol materially changed the reported
Registry and Studio timings, so those values are not portable across protocols.** Which of the
changed variables is responsible is not established.

The old runs remain valid observations of what those protocols produced. What is withdrawn is
their general interpretation, not the fact that the runs produced those numbers.

**Interleaving is also not yet counterbalanced.** Every round runs the cases in the same order,
so a given case always follows the same predecessor - one may consistently follow a long
validation batch while another consistently follows a cache hit. Round-robin spreads drift over
the run; it does not equalise those local conditions. There is no warm-up exclusion and no
stabilisation check either. Repeating an identical fixed-order run would measure repeatability
*under that order*, not remove its confounding.

**What a controlled comparison would need**, and is the next measurement rather than a claim
here: one fixed fixture corpus with verified shapes, used for both protocols; identical
cache-reset rules and identical summary statistics on both sides; counterbalanced protocol order
so neither always runs first; a recorded seeded permutation or balanced ordering within the
interleaved runs; and the raw per-trial samples retained.

**A specific claim of mine that this corrects.** I reported Studio's validation as "five times
Recovery's for the same bytes". Within one interleaved run it is **2.5x**: Studio at 32 ops is
24 609 us against Recovery's 9 953 us at 4.19 MB. The direction was right and the multiple was
inflated by comparing two differently-ordered measurements.

### Two claims withdrawn outright: the Studio operation axis was not what it was labelled

**1. The Studio fixture silently truncates, so the per-operation denominator was wrong.**
`build` in the Studio profiling module stops early once the epoch is nearly full -
`if bytes >= MAX_EPOCH_BYTES - 64 * 1024 { break }` - and returns however many operations it
managed, with **no requirement that the count match the request**. At 160 KiB per message the
4 MiB epoch fits about 25. The run that reported "Studio 32 ops" therefore cannot have built 32,
and its physical record size of 4 179 459 bytes sits right at the cap, consistent with exactly
that truncation.

So `24 609 us / 32` divided by a number the fixture never reached. **The claim that Studio's
per-operation cost is flat is withdrawn**, along with the per-operation constants for that
family. Registry is unaffected: `Source::fill` ingests `count + 1` and asserts that any
acceptance has `n < count`, so it panics unless exactly `count` were accepted and the last
refused - the count is guaranteed by that builder rather than assumed.

Corrected: every case now carries a `FixtureShape` with `requested_ops` and an **observed**
`actual_ops`, read back both from the builder's returned operation vector and from the persisted
source's own `op_count()`, which must agree. `check_case_structure` fails any case where
requested and actual differ, rather than letting a wrong denominator through. Studio's requested
counts are now 3, 12 and 24, chosen to fit.

**2. The Studio reference-mode cases had no CIDs to collect.** `title_op` emits
`FlipnoteOp::SetHeader(Title(..))`, which names no pixel. Those sources' CID sets are **empty**,
so the reference-mode timing was the timing of reference collection over a history with nothing
to collect.

**"Reference collection is free for Studio" is therefore withdrawn.** The defensible statement is
narrower: *reference-mode and fresh-accounting validation were similar for these title-only
Studio histories.* Nothing was established about a source that actually holds frames.

Corrected: `FixtureShape::cids` is recorded for every reference-mode case - `Some(0)` is a
meaningful value and now visible rather than implicit - and `check_case_structure` refuses a
reference-mode case that did not record a CID count. A new `studio_frame_cases` builds Studio
sources through real `InsertFrame` operations naming distinct stored blobs, so there is a Studio
reference case with a non-empty set. The title-only cases are kept, relabelled
`studio_titles_*`, because the comparison is still worth having once it is labelled honestly.

**Registry's similarity between modes has a source explanation and survives.** The Registry arm
of the validator collects no CIDs in either mode, so equal fresh-validation cost there is
expected from the code rather than inferred from the timing; what reference mode changes for
Registry is the surrounding cache behaviour.

### The restart budget bounds retries; it does not make a moving vault scannable

**This is a deterministic liveness and termination test, not 13.7's restart-rate measurement** -
see "The restart test is a liveness result, not a restart rate" for why, and treat that as the
current statement. An earlier version of this section opened by calling it "the restart rate under
concurrent writes", which is the equivalence that description withdraws: the test rotates a bare
guard at fixed step intervals, drives the job **unbudgeted** so nothing ever parks, and performs
no write at all.

What it does establish is worth having. A write either lands between two steps or it does not, and
a restart discards the cursor's progress whatever the machine's speed - so the outcomes below are
properties of the state machine rather than timings.

`c3_restart_budget_bounds_retries_but_does_not_survive_sustained_writes` drives a real
`EpochInventoryJob` with a `epoch_mutation_guard()` landing every *n* steps:

| write rate | restarts | outcome |
|---|---|---|
| none | 0 | inventory issued |
| one per step | 3 (the whole budget) | **`Unstable`** |

**A vault written to on every step never completes, whatever the budget is**, because a restart
discards all progress - so the scan can never get further than one step before being overtaken
again. Enlarging `MAX_INVENTORY_RESTARTS` would not help; it would only delay the refusal.

That makes L6's "under sustained writes a commit is held and retried" a statement about
**liveness, not latency**. The failure mode is not a slow scan, it is `Unstable` and a caller that
must back off - which is the behaviour the runtime adoption has to handle, and a reason the
adoption is its own checkpoint rather than a signature change.

Mutation-verified: removing the budget check so restarts are unbounded fails the test at its
budget assertion, then restores byte-exact.

**Every item design 13.7 names now has *something* against it - which is not the same as being
measured, and an earlier version of this sentence claimed the latter.** Withdrawn. Item by item:

| 13.7 item | what exists |
|---|---|
| maximum continuous custody per scan slice | measured, at fixture sizes |
| largest single-record step per family, with and without reference collection | Registry and Studio **at fixture sizes, not at accepted ceilings**. Recovery to 4 MiB, against an 18 MiB ceiling. **Updated 2026-10-08** (see "The uncached families at their ceilings"): Intents at its count, byte and branch-operation ceilings, in both modes; DraftArchive at its payload ceiling, accounting mode only; OwnerReceipts on small journals only, not at its 27 KiB cap |
| how often detached validation is needed | trivially always, since `validation_fits` returns false. Not a measurement of anything |
| visits per full scan | measured, and on one multi-family vault rather than a realistic one |
| restart rate under concurrent writes | a **deterministic liveness test**, not a rate - see below |

So 13.7 is source-correct fixtures plus executed measurements with narrower conclusions than the
obligation asks for. What is missing is coverage as well as confidence: accepted-ceiling runs,
DraftArchive, a real workload for the restart behaviour, and single runs throughout with frames
still confounded with bytes.

### The factorial answers the confound: it is the operation axis, not the reference axis

The `(frames, distinct_cids)` factorial holds frame count *and* encoded size fixed while varying
how many of the references are distinct - the record carries the same signed operations either
way, each naming one 32-byte CID. Isolated workspace, release, 8 interleaved trials, accounting
mode, `validation_batch_mean` upper medians:

| frames | distinct CIDs | bytes | validation | change |
|---|---|---|---|---|
| 16 | 1 | 16 626 | 5 750 us | - |
| 16 | 16 | 16 626 | 5 640 us | **-2%** |
| 128 | 1 | 130 487 | 196 718 us | - |
| 128 | 128 | 130 487 | 200 687 us | **+2%** |

**Distinct reference count has no measurable effect on validation cost.** Two percent either way,
at a 16x change in reference count, inside the spreads. And the direction is not even consistent -
it falls at 16 frames and rises at 128.

That was **predicted before the run and recorded in the code**: `blob_cids()` parses every signed
operation regardless of how many distinct CIDs result, so only the resulting set's size differs.
The prediction also said any reference cost should surface in `install` or `finish` instead.
`install_us` was `0/0/0(z8)` for every case including the 128-distinct-CID one - all eight samples
below the clock's resolution. **That is "not resolved at this resolution", not "free"**; an
earlier version of this sentence said the merge is free, which a row of zero-millisecond samples
cannot establish.

**And with references held fixed, the frame axis grows superlinearly over the measured range** -
not "confirmed" as the causal driver, since frames and signed-history bytes still move together
there. 16 to 128 frames at a constant single reference: 5 750 to 196 718 us, **34.2x for 8x the
frames**, an endpoint slope of
`n^1.70`. That matches the earlier confounded measurement almost exactly, which now means
something it did not before: the superlinear growth belongs to the **operation** axis, not to the
reference axis it used to be entangled with.

**What is still confounded:** frames and bytes. Eight times the operations is eight times the
signed history, so the `(16,1)` to `(128,1)` comparison moves both. Separating those needs a
payload axis at fixed frame count - `build`'s `message` padding could supply it - and is not done.
So the honest statement is that cost tracks operation count *or* the bytes that come with it, and
**not** distinct reference count, which is now excluded.

### The protocol comparison: scheduling does not move these numbers

Four arms, ABBA, one fixed corpus shape, identical cache rules and statistic, varying only the
schedule. `step_total` upper medians per trial:

| case | blocked (arm 1) | interleaved (arm 2) | interleaved (arm 3) | blocked (arm 4) |
|---|---|---|---|---|
| recovery_accounting | 9 000 us | 9 000 us | 8 000 us | 9 000 us |
| registry fresh, 8 ops | 3 000 us | 3 000 us | 3 000 us | 2 000 us |
| registry warm, 8 ops | 3 000 us | 3 000 us | 3 000 us | 3 000 us |
| registry references | 2 000 us | 3 000 us | 2 000 us | 3 000 us |

**No detectable difference between the protocols.** Every figure sits within one clock tick of
every other, blocked and interleaved alike, in both positions.

**The causal exclusion drawn from this is WITHDRAWN.** An earlier version said "this says the
scheduling was not the cause" of the 2.5x shift. It does not, for a reason that makes the
experiment inapplicable rather than merely weak:

**The table compares `step_total`, and the 2.5x shift was in validation time.** `step_total`
accumulates immediately after `step_epoch_storage_scan` and **before the detached validation
batch runs**, so it excludes the very phase whose movement the comparison was invoked to explain.
A schedule that changed validation cost 2.5x while leaving read-and-authenticate unchanged would
produce exactly the table above. That is a direct counterexample, not a caveat.

The corpus is also wrong for the question: Recovery and Registry at eight operations, not Studio,
and not the Registry-24 case the earlier headline came from.

So what this supports is only: **no large difference was observed in step totals for these
small-corpus arms.** The earlier shift's cause remains unidentified, and investigating it needs
the same *validation* metric on the corresponding shapes, with replicated counterbalanced arms
and recorded corpus identities and cache conditions.

**What this comparison cannot say.** One observation per (protocol, slot) cell, so there is no
arm-to-arm noise estimate; the corpus is small and deliberately so; the arms differ in corpus
*instance* as well as schedule, since cases cannot be reused without inheriting warm state; and
ABBA gives Blocked the cold first slot while Interleaved never occupies it.

**And it cannot rule out an effect of the size previously claimed either** - an earlier version of
this sentence said it could, which is the same causal exclusion withdrawn above. `step_total`
excludes the validation phase the 2.5x shift was in, so there is no demonstrated detection bound
for the omitted phase or the omitted shapes. What the comparison supports is only that no large
difference was observed in step totals for these arms.

### A P1 in the interleaving itself, found by adversarial review

The interleaved schedule was documented as rotating the case order each round so no case kept a
fixed predecessor. **It did not do that.** `(round + offset) % n` emits
`c[r], c[r+1], … c[r+n-1]`, which preserves the cyclic order: every case except the round's first
follows exactly the predecessor it always did. Measured at the profile's real shape, **35 of 35
cases had at most one predecessor across all eight rounds.** The property the interleaved arm was
described as having was entirely absent, and it would have gone into this ledger as established.

Replaced with a seeded Fisher-Yates permutation per round, with the seed printed on every result
row so an order can be reproduced. `c3_interleaved_rounds_vary_each_case_predecessor` pins it: put
the rotation back and it fails with "35 of 35 cases have at most one predecessor across 8 rounds".

Three further corrections from the same review:

- **The `black_box` was a no-op.** `revalidate` returns `Result<(), AppError>`, so black-boxing its
  result pinned a unit value; and the justifying comment claimed the call "crosses a crate
  boundary", which is false - `revalidate`, `run` and `validate_record_body` are all in
  `catcoms-app`. Now pinned on the input side, which is what prevents hoisting.
- **`interleaved=true` was printed on the blocked arm's rows.** The schedule is now threaded into
  the report as `order=`, with an arm index, a slot label and a block header, and the stale
  `units=` string was corrected after `Spread`'s display gained the raw median.
- **`protocol_comparison`'s own doc overclaimed three ways** - "one fixed corpus" (it is one fixed
  corpus *shape*; four instances), "varies only the schedule" (fresh directories, fresh device
  keys, differing global position, and by construction a differing fixture-to-measurement delay),
  and "measured once in each position" (Blocked holds global slots 1 and 4; Interleaved never runs
  cold-first). All three are now stated as they are.

### The earlier frame comparison, superseded by the factorial above

Building the Studio frame fixture to fix the empty-CID problem produced the independent axis the
ops-versus-bytes confound needed, by accident: frame-bearing records are **small**, so cost and
bytes move in opposite directions.

| Studio case | authenticated bytes | validation (min/upper-median/max us) |
|---|---|---|
| titles, 24 ops | 3 697 338 | 21 187 / **23 890** / 29 140 |
| frames, 16 | 16 626 | 5 875 / **7 296** / 7 484 |
| frames, 128 | 130 487 | 211 765 / **252 703** / 260 921 |

**A 130 KB frame-bearing record's validation was about ten times a 3.7 MB title-only record's.**
Bytes fell by a factor of 28 while measured cost rose by a factor of 10.

The defensible conclusion, stated at that width: **encoded size alone does not explain validation
cost across the measured Studio shapes, so structural shape has to be represented in the
measurements or conservatively bounded.** That is enough to reject "small record, therefore cheap
validation" for this corpus. It is **not** a finding that bytes are irrelevant - the two shapes
differ in operation type, operation count, materialised document structure and reference content
all at once, so no single factor is isolated. An earlier version of this section said "bytes are
decisively not the driver", which claims more than the comparison supports and is withdrawn.

**Growth between 16 and 128 frames was more than linear:** 8x the frames cost about 34.6x, which
is an **endpoint slope** of roughly `n^1.7`. That is not an established complexity law and must
not be used to extrapolate to the frame ceiling - doing so needs intermediate points and repeated
runs, neither of which exists. What it does support is that **no per-operation constant describes
this shape**, so a threshold built from one would underestimate where it matters most.

**The headline number, precisely labelled:** roughly **253 ms was the upper median of
batch-average validation time** for the 128-frame, 130 KB fixture. It is not the worst individual
validation latency, not an end-to-end read latency, and not measured actor custody - the harness
times repeated `revalidate()` calls separately from the consuming validation and from
installation. A conservative classifier would eventually want the worst individual figure, which
this batching cannot produce.

*(The figures in this subsection are from the contended run. They are replicated within 11% by the
isolated run recorded under "The isolated release profile" below, which is the one to quote.)*

**Reference mode versus accounting, on a fixture with real CIDs.** Frames, 128: 252 703 us
accounting against 248 328 us reference-collecting, inside their own spreads, with 128 distinct
CIDs collected and the collection now *verified* rather than assumed (see the oracle below).
Stated as: **for the verified 128-frame fixture, reference-mode validation was similar to fresh
accounting validation in this run; incremental cost was not resolved. Installation stayed below
the harness's resolution.** "Free" and "confirmed equivalent" are both withdrawn - the paths are
not the same work, since the reference path additionally returns CIDs and required-metadata for
later merging, and "no difference resolved at this resolution" is not "no difference".

**Caveats carried forward:** the frame axis varies frame count, CID count and bytes together, so
it separates Studio's cost from bytes only because the title axis moves bytes the other way.
Within the frame series, frame count and CID count remain confounded with each other; a factorial
case holding one fixed is still not done.

### The reference-result oracle, which was missing

The profile timed reference scans and **threw their results away**. `run_trial` called
`finish_cursor_creative_references` and discarded the returned set, and the structural check
required only that the fixture had *recorded* a CID count. So these were different claims, and
only the first was checked:

- the fixture contains 128 distinct referenced CIDs - established;
- the profiled scan returned those 128 CIDs - **not asserted anywhere**.

A Studio collector regression that completed successfully with an empty set would have satisfied
every structural check and produced a fast, meaningless number - and the reference-mode timings
above would have been the timings of collecting nothing, which is precisely the mistake this
ledger had already recorded once.

Corrected: each case carries an `ExpectedRefs` - a group and its exact CID set, or "no group holds
any", which is the right expectation for Registry and for the title-only Studio sources. Every
reference-mode trial now compares the returned set against it **after the timer stops**, so
checking is not charged to the phase being checked, and also requires the total reference count to
match so a collector attributing references to the wrong document cannot pass by coincidence.
`check_case_structure` refuses any reference-mode case that carries no expectation.

**Verified by mutation, and the mutation exposed a second gap.** Replacing `cids.extend(...)` with
a discard in the Studio arm of `validate_record_body` left **all four existing tests passing** -
because the only fixture reaching that arm with references on lived in the `#[ignore]`d profile,
while the canonical reference smoke test stages a *Recovery* record and exercises a different arm.
So `c3_studio_frame_reference_scan_returns_its_planted_cids` was added to the ordinary suite. With
the mutation applied it fails at *"the profiled Studio reference result differs from the fixture's
expected set: collected 0 of 4 expected CIDs for this group"*; restored byte-exact, it passes.

### The axes for the large-record fixtures remain confounded

Both fixtures vary operation count at a **fixed** 160 KiB message payload, so encoded bytes rise
roughly in proportion. This experiment therefore cannot distinguish

- cost driven by operation count, from
- cost driven by bytes, with roughly constant bytes per operation.

A validator whose cost depended only on bytes would look approximately linear per operation
here. **So the conclusion that a threshold "should be written against operation count" is
premature and is withdrawn.** Both observed values are retained; neither is named as the causal
driver.

Separating them needs two independent axes: hold operation count fixed while varying message
size, then vary operation count at approximately fixed total encoded size.

The Recovery reference experiment has the same problem in a different shape: raising the frame
count raises operation count, distinct-CID count and projection bytes together. It establishes a
curve for those fixture shapes, not that reference count independently explains it.

### What survives, stated at the width the evidence supports

**Survives:** for the Registry and Studio cases measured, validation takes substantially longer
than the measured read-and-park phase, and `validation_fits` can move only the former. Registry's
similarity between accounting and reference modes has a direct source explanation, and Studio's
is now measured on a fixture with 128 real CIDs rather than an empty set. The installation phase
stayed below resolution everywhere it was measured, including that 128-CID merge. **Bytes are not
Studio's cost driver** - a 130 KB frame record costs ten times a 3.7 MB title record - and
**frame-bearing validation is superlinear in frame count**, so no per-operation constant
describes it.

**Withdrawn or not established:** every absolute figure; the isolated effect of ordering; the
flat Studio per-operation rate, whose denominator was wrong; operation count as *the* causal axis
for the large-record fixtures, where it still covaries with bytes; and, within the frame series,
frame count as distinct from CID count.

**"Studio's validator is the most expensive per byte" needs its fixture qualifier.** The observed
Studio history was more expensive per byte than the observed *opaque* Recovery accounting record,
but those bytes represent different work, and three families remain unmeasured. It is a statement
about two fixtures, not a family ranking.

**A flatter per-reference curve is not evidence that the new protocol is more accurate.** It
matched what was expected, which is not a control. Accuracy needs independent verification, and
resemblance to the anticipated model is not that.

### Read-and-park sits at the clock's resolution below about a megabyte

`read_and_park` for the 322 KB Registry and Studio records reads `0/1 000/1 000`. The
block-ordered harness reported 2 125 us for the same phase.

**An earlier version of this section explained that as "an artifact of averaging zeros, ones and
twos". That explanation is arithmetically impossible and is withdrawn.** A mean of samples drawn
only from {0, 1 000} us cannot exceed 1 000. The block-ordered figure came from a summed
`read_and_park_ms` of 17 ms over 8 trials, so its samples included 2 ms and 3 ms readings - the
phase genuinely took longer under that protocol. The difference between 2 125 and ~1 000 is a
**protocol difference, not an averaging artifact**, and averaging is not the defect being
described. The defect was reporting one summary figure without its distribution, its resolution
or its conditions.

What is true: the fraction column is **not trustworthy for any row whose read-and-park upper
median is at or below one millisecond**, because a one-tick measurement is known only to within
100% of itself.

Two related corrections to the reporting, both from the reviewer:

- **The middle figure is the upper median**, `sorted[len / 2]` - the fifth of eight, not an
  interpolated midpoint. For `0,0,0,0,1000,1000,1000,1000` it reports 1 000 where an arithmetic
  median would give 500. The convention is kept, because it never invents a value the clock did
  not produce, but it is now named `upper_median` rather than `median`. And a printed
  `0/1 000/1 000` does **not** establish that half the samples were zero - it is compatible with
  several zero counts - so every spread now carries an explicit `zero_samples` count. The claim
  that half were zero is withdrawn as unsupported by what was printed.
- **The resolution guard was too weak twice.** First it checked the raw *sum*, so seven zeros
  plus one 1 ms sample passed and printed **100% deferrable**. Then it rejected only a zero
  median, which let a one-tick median through - contradicting this ledger's own stated policy.
  It now requires both upper medians to exceed one tick, and
  `c3_a_phase_resolved_to_one_tick_or_less_reports_no_fraction` pins all three cases: the
  seven-zeros case, the one-tick case, and a positive control above one tick.
- **The validation spread is a spread of batch means**, not of individual validations: each
  sample is 64 validations divided by 64, so eight averages. It cannot reveal a single slow
  validation inside an ordinary batch, which is what a conservative worst-case threshold would
  need. Renamed `validation_batch_mean_us`, and the claim that every reported timing is "never a
  mean" is withdrawn - the batched phase is a mean by construction.

**Reference collection is free for Registry and Studio, and expensive for Recovery.** That looked
contradictory until the reason was checked, and the reason is structural:

| family | accounting validator | what reference collection adds |
|---|---|---|
| Recovery | treats the projection as **opaque bytes** - decode plus footprint | a full canonical decode and per-operation validation the accounting path never does |
| Studio | already a full typed reconstruction | `blob_cids()` on the unit it has already restored |
| Registry | no CID collection at all for `DocRegistry` | nothing |

Measured under interleaving, medians: Studio at 32 ops was 24 609 us accounting against 23 375 us
reference-collecting, and Registry at 24 ops 15 343 against 15 375 - differences inside their
own min/max spreads. Recovery's reference collection, by contrast, runs many times its
accounting validator per byte. **So "does this scan collect references" is a first-order input
to the classifier for Recovery and very nearly irrelevant for Registry and Studio.** It is
already a `validation_fits` parameter; this says what it is worth per family.

This is one of the findings that **held across both orderings**, which is why it is stated
plainly while the absolutes are not.

**Installation stayed below resolution in every one of these cases**, including the 3.8 MB
Registry and 4.2 MB Studio records.

### The warm-cache comparison, and why it must not be read as "the cache is slow"

Per-trial step totals, fresh against warm:

| case | fresh | warm |
|---|---|---|
| Registry 2 ops | 2 125 us | 1 500 us |
| Registry 8 ops | 5 375 us | 6 375 us |
| Registry 24 ops | 13 750 us | 17 000 us |
| Studio 3 ops | 3 500 us | 1 625 us |
| Studio 12 ops | 6 375 us | 7 250 us |
| Studio 32 ops | 14 875 us | 19 500 us |

At the larger sizes the warm scan's steps were **not** faster, and at 24 and 32 ops they were
somewhat slower. That is not evidence that the cache costs anything, and it should not be
reported as such. The comparison is confounded by construction: in the fresh runs the record
**parks**, so the validation is outside the step entirely; in the warm runs there is a cache
hit, so the validation does not happen at all. **Both step figures therefore exclude validation,
and what they actually measure in both cases is read, authenticate and digest** - the term
neither the cache nor the classifier removes. The residual spread is inside the run-to-run
variance already recorded and is not interpreted here.

The useful distinction the two modes do establish is not about speed:

- a **cache hit** means the validation never happens;
- **parking** means it happens later.

Both leave read-and-park in the visit. That is the same boundary this whole measurement keeps
arriving at, from a third direction.

### Registry and Studio: three modes, not one, because they are the cacheable families

Registry and Studio are the families whose expensive typed reconstruction motivated C-3, and
they are also the only two the validation cache covers: `cacheable` is
`matches!(family, Registry | Studio) && references.is_none()`. That makes them structurally
different from Recovery to measure, in a way that would have corrupted the means silently.

**A cache hit is never parked** - by design, because the expensive thing is exactly what the
cache avoided. So a repeated-trial profile of a cacheable family performs **one** fresh
validation and then seven cache hits, and those seven trials contribute no validation sample at
all while the mean divides by whatever count happened to accumulate. Measuring these families
needs the modes separated:

| mode | cache | what it measures |
|---|---|---|
| accounting, fresh | cleared between trials | a fresh typed reconstruction, repeatably |
| accounting, warm | carried across trials | the cache-hit path |
| reference collecting | bypassed entirely | the expensive validator, never cached |

`RecordCache::clear_for_test` exists for the first of those and nothing else.

This is **pinned by a test on a frozen clock**, not discovered in a timing run:
`c3_cacheable_family_parks_when_fresh_and_hits_cache_when_warm` requires that a cleared cache
parks the Registry record in every trial and reports zero hits, that a warm cache produces hits
and stops parking in every trial, and that a reference scan reports zero hits and parks in every
trial regardless of cache state. If any of those three stops holding, the profile's arithmetic
is wrong and a test says so rather than a number quietly shifting.

### The profile now runs in groups, because all cases alive at once exhausted the machine

Two distinct resource failures, a day apart, both worth recording because both were first
mistaken for something else.

**The release lib test aborted with exit `0xffffffff` and no panic** once the profile reached 37
cases. Every case holds an open `ServerStore` and a temporary directory, and the multi-family
cases hold five families each. The profile now builds and runs **groups**, each interleaved
internally and dropped before the next is built.

**The cost of grouping, stated rather than glossed:** cases are interleaved *within* a group, so
a difference between two cases in the same group is comparable and a difference **across** groups
is not. Groups are therefore drawn so each measurement's actual comparisons fall inside one - the
whole Studio factorial in one group, the Registry operation sweep in one, the Recovery size sweep
in one. Cross-group figures in the tables above should be read as separate experiments.

**Then the compiler itself ran out of memory** - `rustc-LLVM ERROR: out of memory`, exit
`STATUS_STACK_BUFFER_OVERRUN`, during the *release build* rather than during any test, while
other sessions were building concurrently. With them finished, 12.6 GB of 63.8 GB was free.

**And the third failure was self-inflicted, which retroactively undermines the first's
diagnosis.** A third run reached five of seven groups and then died with the same
`0xffffffff` - because **I killed it**. Every one of my test invocations begins by stopping
lingering `catcoms*` processes, a habit adopted to clear a Windows linker lock; that kill is
indiscriminate and it terminates my own background profiles. Starting the next run is what ended
the previous one.

So of three aborts: one was a compiler OOM under external contention, one was me, and the first -
the 37-case abort that motivated grouping - has **no established cause**. It may have been
resource exhaustion, and it may have been the same contention that produced the OOM minutes
later. The grouping is kept on its own merits, because 37 simultaneously open stores and
temporary directories is a bad idea regardless, but **the claim that all cases alive at once
exhausted the machine is withdrawn as unproven.**

Two practice changes follow, both mine to keep:

- **Do not kill `catcoms*` processes while a background run of this session's own is in flight.**
  The linker-lock workaround and a running profile are incompatible.
- Treat a bare `0xffffffff` with no panic as *unexplained* rather than as evidence of resource
  exhaustion. It is what a force-kill looks like too.

This is now the fifth resource- or environment-related failure on this checkout, after the stale
fingerprint, the stale dependency rlibs and the parallel-suite starvation. The standing rule they
add up to: **`-j 1`, one run at a time, and treat any build or timing failure here as
environmental - or self-inflicted - until shown otherwise.**

### Build contention on this machine, and what the evidence for this checkpoint actually is

`git worktree list` shows many concurrent agent worktrees on this checkout, **two of them
directly inside `target/`**, with three `cargo`/`rustc` processes running from them during this
work. Three separate times the tree failed to compile with `catcoms_mls::GroupMode` and
`catcoms_discovery::reconnect` unresolved, while both are exported from unmodified sources -
once `clippy` in the *same* invocation compiled the lib test successfully while `cargo test`
reported those errors. Sibling worktrees building the same crate names into shared artifacts is
the consistent explanation.

**The evidence for this checkpoint is therefore the run that completed before the contention set
in, and that is stated rather than implied.** On this exact source: the debug lib test built
clean, all four structural tests passed, and the 29-case interleaved release profile completed
and satisfied every structural assertion - including the new requested-versus-actual shape check,
which is what proves the Studio operation counts in the tables above were really built.
Re-confirmation afterwards was not obtainable; retrying only added to the contention. The
`check-no-ambient.sh` and formatting gates were run after, and pass.

### A stale build fingerprint silently invalidated a build during this work

Worth recording, because it is the kind of thing that makes every other number on this page
suspect if it is not caught.

After rewriting the profiling module, `cargo test --lib --no-run` reported `Finished` in 0.6 s
and named an executable **thirteen minutes older than the source file**. Nothing had been
recompiled. The test list had 700 entries where the tree should have produced 724, and the new
module was absent - so the run that appeared to pass was the *previous* binary. Touching the
source, touching the parent module and touching `lib.rs` all failed to trigger a rebuild.

`cargo clean -p catcoms-app` then surfaced **19 compile errors that had been invisible**: one of
mine, and eighteen `no method named archive_id` in Agent 2's draft-archive code. Those eighteen
were not a defect in their work - `archive_id` exists in
`catcoms-replication/src/studio/overlay/archive.rs` - but `catcoms-app` was linking a **stale
`catcoms-replication` rlib** predating it. `cargo clean -p catcoms-replication` was needed as
well.

This machine has another agent's git worktree under `.claude/worktrees/`, which is the known
shared-target-directory hazard.

**Consequence for this ledger:** "it compiled" and "the tests passed" are only evidence here if
something actually rebuilt. A `Finished` line with no compilation and an executable older than
the sources is not a pass. Where a result matters, check that the build did work, or clean the
package first.

### What is measured, and what is extrapolation

Measured: Recovery, accounting-only, 1 KiB to 4 MiB; and Recovery with reference collection over
a canonical fixture (below).

**Extrapolated, and labelled as such:** `MAX_RECOVERY_SLOTS_BYTES` is `3 * 6 MiB + 1024`, about
18 MiB, which is 4.5x beyond the largest measured point. Carrying the observed rates out gives
order-of-tens-of-milliseconds for both phases. That is an extrapolation from a linear fit over a
range that does not include the ceiling, not a measurement of the ceiling. It is not refined
further here; measuring valid near-ceiling records is the way to replace it, not arithmetic.

### Run-to-run variance is large enough to bound what any of this can be used for

The Registry/Studio run re-measured the identical Recovery fixtures. Same tree, same build, same
machine, nothing changed but what else ran in the process beforehand:

| Recovery case | run A | run B | spread |
|---|---|---|---|
| 4 MiB accounting, read-and-park | 9 000 us | 12 250 us | +36% |
| 4 MiB accounting, validation | 11 009 us | 15 294 us | +39% |
| 512-reference, validation | 2 228 us | 4 144 us | **+86%** |

**This is bigger than the 20% recorded after the first profile, and that figure is withdrawn.**

> **SUPERSEDED.** The three bullets that stood here are no longer current guidance. They are
> replaced by "Two claims withdrawn outright" and "What survives, stated at the width the
> evidence supports" above, and the reasons are recorded there. Kept only so the sequence of
> corrections stays readable.
>
> - The claim that "within-run comparisons survive" and that the Studio-versus-Recovery gap "at
>   equal bytes is a 5x gap" is **withdrawn twice over**: the interleaved run put the same
>   comparison at about 2.5x, and the Studio denominator it rested on was wrong anyway, because
>   the fixture had truncated below its requested operation count.
> - The per-operation flatness claim is **withdrawn** for Studio for the same reason, and for
>   both families it is confounded with bytes, which rise in proportion.
> - Only "absolute constants do not survive" was right, and it turned out to understate the
>   problem: the absolutes moved 2.5x under a protocol change, not merely 40%.
>
> The variance table above stands - those runs did produce those numbers.

### The repetition discipline, now in place

All three parts of what the variance above demanded are implemented, and they change what the
profile is allowed to claim rather than making any single number better.

**Cases are interleaved, not run in blocks.** Every fixture and mode is built up front as a
`Case` owning its own store, and `run_interleaved` runs one trial of *every* case before the
second trial of any of them. Previously each case's eight trials ran consecutively, so comparing
two cases compared two different moments in a long run - and with drift of that size on this
machine, that comparison was not safe to make. Round-robin spreads the drift across every case.
It does not make a figure more accurate; it makes differences **within one profile** mean
something. Differences between separate runs still do not.

Because a `Case` owns its store, each of the three modes gets its own build of the same fixture
shape. That is the cost of interleaving them with each other, and it is worth paying: the
fresh/warm/reference comparison was previously three consecutive profiles of one store.

**Every timing is reported as `min/median/max` microseconds, never a mean**, with the raw
millisecond total alongside so an all-zero spread reads as "below the clock's resolution" rather
than "free". The deferrable fraction is taken from medians and suppressed unless both components
resolved.

**The structural preconditions are asserted per case, after the run**, by
`check_case_structure`, and that checker has since been hardened on the reviewer's points -
previously several of these were only checked on one smoke fixture and extrapolated to the rest:

- **phase-vector alignment for every case, not one.** `trials()` reads only the read-and-park
  vector's length, so a validation or install vector of a different length would have gone
  unnoticed and its spread would have covered a different trial set;
- **per-trial cache hits and parked counts**, replacing a single scalar that held only the *last*
  trial's hit count. That scalar could not distinguish "a hit on every trial" from "a hit on all
  but the first", which are different experiments - and warm mode is the second, because the
  first trial is what populates the cache. The checker now requires exactly that shape: trial 0
  misses and parks, every later trial hits and parks nothing;
- **observed fixture shape.** `requested_ops` versus an `actual_ops` read back from the fixture,
  with the case failing when they differ; `physical_bytes`; and a `cids` count that a
  reference-mode case must record.

Every result line now prints the fixture shape alongside the timings, so a reader can see what
was actually built rather than what was asked for.

None of this repeats whole *runs*, which is the one remaining part: cross-run variance is still
unmeasured except by the accident recorded above, and remains the reason no absolute constant
here should be treated as calibration.

### Not covered

- **Repeating whole runs.** Interleaving, distributions and per-case structural checks are now
  in place (see "The repetition discipline"), but nothing repeats an entire profile and compares
  run against run. Cross-run variance is still known only from the accident recorded above.
- **Near-ceiling shapes for Registry and Studio.** Measured to 24 and 32 operations; neither is
  at its largest accepted shape, and the per-operation constant is what would be extrapolated.
- **OwnerReceipts, Intents and DraftArchive**, which are unmeasured.
- **Structure as well as encoded size.** A byte-linear curve for an opaque Recovery projection
  establishes nothing about validators that walk operation counts, retained history, reference
  counts, metadata, or conflict and tombstone shape. Each family needs its own structural axis,
  not just a size axis.
- **Restart rate under concurrent writes**, and visits per full scan on a realistic multi-family
  vault. The visits figure here is an artifact of parking every record in a tiny fixture, not a
  scan-shape measurement. This comes **after** the single-record numbers are interpretable,
  otherwise a slow full scan cannot be attributed between validation, reference merging,
  rescans and scheduling.
- **Near-ceiling records.** Everything above the largest measured point is extrapolation.

Retained-input accounting and cancelled-worker ownership are **not** in this scope. They belong
to the runtime adoption checkpoint, and storage-only timing does not evidence them.

`validation_fits` is therefore **unchanged** and still returns false for everything. One family's
curve without its reference-collection half is not enough to set a threshold, and 9.2's rule for
the absence of the figures is to default to detaching. Changing it on this evidence would be
exactly the overreach the rule exists to prevent.

### How it is built

`ParkedEpochRecord::validate` now delegates to a private `run(&self)`, and a `#[cfg(test)]`
`revalidate` calls the same `run`. That is deliberate: a measurement with its own copy of the
validation would stop measuring the production path the first time either changed, and nothing
would say so. The batching exists because `scripts/check-no-ambient.sh` forbids `Instant::now`
everywhere under `crates/`, test code included, so the finest available clock is
`catcoms_rt::Clock` at milliseconds and one record's validation can round to zero against it.

Five tests now, following the existing `studio_source_profile_smoke` /
`profile_studio_source_operations` pattern: four structural ones in the ordinary suite
(`c3_visit_profile_smoke`, `c3_canonical_reference_fixture_collects_its_planted_cids`,
`c3_cacheable_family_parks_when_fresh_and_hits_cache_when_warm`,
`c3_a_phase_resolved_to_one_tick_or_less_reports_no_fraction`) and the `#[ignore]`d
`profile_c3_visit_cost`. The first two run on a
`ManualClock` and assert only structure, never duration - a timing assertion in CI is a
machine-speed assertion in disguise. `profile_c3_visit_cost` is `#[ignore]`d and prints.

**The smoke test's assertions are scoped to exactly what they prove.** Making `validation_fits`
return true fails it at "a record did not park, so the classifier is no longer detaching
everything and this measurement no longer isolates the validation phase". That proves the
profile's structural precondition and nothing else: it does **not** show that the timers cover
the intended phases, that installation is cheap, or that the fraction estimates a speedup. The
phase-coverage claims need their own deterministic observations, so the smoke test now also
requires every trial to complete, every record to contribute a sample in **every** trial (or the
per-phase means divide by the wrong count), zero validation-cache hits in an accounting Recovery
scan (a cache hit is never parked, so a record would silently stop contributing a validation
sample), and no fraction at all from a frozen clock. The canonical fixture separately asserts
the collected CID set equals the planted one.

## Requirement 3: COMPLETE

### Exact-head execution evidence

Run on the branch head itself, not a PR merge checkout.

| Target | Result |
| --- | --- |
| `catcoms-app --lib` (whole suite, not the `store::` filter) | **679 passed, 0 failed, 11 ignored** |
| `--test product_e2e` | 13 passed |
| `--test tcp_product_e2e` | 1 passed |
| `--test process_recovery_e2e` | 2 passed, 1 ignored |
| `--test studio_inspection_fixture` | 2 passed |
| `--test studio_preview_fixture` | 1 passed |
| `clippy --all-targets` | clean |

Each integration binary was run separately, so a library failure could not prevent the
recovery-sweep check in `product_e2e` from executing. The `studio_actor_owner_return` case that
failed in CI's Linux job passed here; it remains separately tracked and is not attributed to this
work either way.

`scripts/check-no-ambient.sh` **now passes** (2026-09-28), which supersedes this ledger's
standing claim that it exits 1 and has been red since 2026-09-13. That claim is stale, not wrong
at the time.

**This work briefly broke that gate and someone else fixed it.** The 13.7 profiling module's doc
comment contained the literal string `` `Instant::now` `` while explaining that the gate forbids
it. The script greps `Instant::now` across every `crates/**/*.rs` with only `catcoms-rt`
exempted, and makes no exception for comments - so describing the prohibition violated it. The
fix, made on `Chat-method-redesign` and merged here by PR #29, rewords it to "direct OS clock
reads". That wording is kept.

Worth recording as a class of mistake: a gate implemented as a text search over sources is
tripped by *documentation about the gate*. Prose describing a forbidden construct has to avoid
naming it.

### Two review findings, both closed

**The after decision now carries an operation identity.** Splitting `after` into
`after_write`/`after_sync`/`after_unlink` fixed the fixed `Fail` variant but left the custom
`Hooked.after` callback routing through one shared path with only a tag and a path to go on -
which cannot distinguish a flush from a later replacement of the same record. Two rotation crash
matrices were testing the earlier flush while claiming the replacement, and passing, because an
error plus a hit flag plus a successful exact restart are consistent with either. The frozen
fixture masked it further: its predecessor is already Closing, so the phase and projection checks
hold for both failures. `CompletedOperation::{Write, Sync, Unlink}` is now passed to the hook,
and both matrices assert the firing event is the last one observed and check the record's own
bytes - the only evidence that separates the two failures in the frozen case.

The first version of this claimed both matrices also asserted the *preceding* Source sync. Only
the ordinary one did; the review caught the overstatement. The frozen matrix now carries the
same assertion, which matters for a different mutation than the one that motivated the finding:
asserting only the final event rejects an injection that fires on the preceding flush, but not a
change that deletes that flush altogether.

**The flush-only refusal now has consumer-level tests.** One per enforcing leaf (Intents,
Registry, Studio), each submitting an otherwise-valid replacement under a flush-only step and
requiring refusal, an unchanged record, and that the replacement hook was never consulted. Both
controls are present: the same replacement succeeds under `WriteStep::new`, and an unchanged
record under the flush-only step is *observed* reaching its sync. Deleting each leaf's
`permit_replacement()` fails its test.

**Zero `writer`/`sync`/`unlink` seam parameters remain anywhere in the crate**, down from 86
across 24 files. Every transaction performs its own physical operations; callers decide around
them and cannot substitute one.

**In a non-test build `WriteHooks` has exactly one inhabitant, `None`.** `Fail`, `MustNotWrite`
and `Hooked` are all `#[cfg(test)]`, and `Never` is uninhabited. So "production supplies no write
implementation" is a property of the type, not an audit of the current call sites. That is what
requirement 3 asked for.

### What made it converge this time

The decisive measurement came before any edit: **only 14 sites in the whole crate actually
invoke a seam parameter.** Everything else forwards. So the semantic work - write versus sync,
which primitive, tag routing, before/after ordering - was confined to 14 places, and the
remaining ~250 sites were parameter substitution the compiler could enumerate.

The sweeps failed because they tried to rewrite bodies by pattern. Changing signatures by hand
and letting the compiler list the call sites gave a strictly decreasing error count:
**102 → 47 → 26 → 16 → 2 → 0** in the library, then **121 → 90 → 76 → 61 → 45 → 26 → 16 → 2 → 0**
across the tests. Compare 170 → 181 → 207 for the second sweep.

### Two things the conversion had to invent

**`WriteStep`**, carrying a step's tag *and* whether it may replace at all. Five production sites
passed a writer that always returned an error - "replay assessment must not rewrite the epoch",
"handoff resolution requires unchanged source". Those are **assertions, not implementations**,
and `WriteHooks::None` would have silently permitted every one of them. A permissive default
there would have deleted five production guards while looking like a mechanical conversion.

The tag had to be a passed value rather than a constant because the same callee is a different
step in different transactions: `save_studio_source` is tagged `Source` at rotation.rs:228 and
`Successor` at rotation.rs:405.

**`WriteHooks::Fail` and `MustNotWrite`**, closure-free injections. `WriteHooks` borrows its
decisions, so a helper cannot build one and return it with a closure inside; without these, every
one of ~60 injected-failure tests would have needed a `let`-bound closure. `Fail` spells its error
out as a `FailError` rather than holding an `AppError`, because `AppError` is not `Clone` and a
one-shot injection that silently stopped firing would be a different test from the one its author
wrote.

### What the tests gained

Several were **strengthened by the conversion**, because the closures they used to pass were
doing the write themselves and so bypassing the capability:

- cleanup's interrupted-unlink test called `fs::remove_file` directly; the real `remove_io`, with
  its symlink and regular-file checks, now runs and the test only decides whether it should.
- every `after`-style injection used to perform its own `write_for_test` and then return an
  error. The transaction now performs that write, so "fails after the bytes are in place" is
  tested against the real replacement rather than a second path to disk.
- `epoch_studio/rotation.rs` held two closures that were **identity maps between per-transaction
  tag enums**, complete with an `unreachable!` arm for tags the other enum lacked. One store-wide
  tag deleted the translation and the unreachable arm with it.
- `epoch_registry/replay.rs` needed a `RefCell` because two adapter closures each wanted
  `&mut sync` and only one could hold it. One set of decisions removed the aliasing rather than
  working around it.

### The conversion hazard, found the hard way

A closure seam often carries assertions invisible at the call site. Cleanup's sync closure was
`panic!("failed traversal must not report a synced batch")` - an assertion that no sync happens,
not an injection. A helper without a sync slot silently dropped it, and that assertion turned out
to be the *only* thing in the module that catches deleting `before_unlink` from the production
loop: the other panicking unlink hooks sit on paths where no unlink was reachable anyway.
**Enumerate what each closure asserts before replacing it, not just what it does.**

**One conversion hazard, found the hard way.** A closure seam often carries assertions that are
invisible at the call site — cleanup's sync closure was `panic!("failed traversal must not report
a synced batch")`, an assertion that no sync happens, not an injection. A helper without a sync
slot silently dropped it, and that assertion is the *only* thing in the module that catches
deleting `before_unlink` from the production loop. Enumerate what each closure asserts before
replacing it, not just what it does.

**Ordering, fixed across all conversions** (the reviewed shape):

```text
guard acquired and budgets invalidated
  → before decision        (may refuse, may replace bytes; runs after the I-4 rotation)
  → fixed capability operation  (the store's own; a caller cannot substitute one)
  → after decision         (may refuse; the bytes are already in place)
  → success/accounting publication
```

**What is landed besides the two paths:** the `WriteHooks` interceptor type, and one store-wide
`WriteTag` replacing the eight per-transaction enums. 315 store tests pass on that.

**Why the tag unification matters more than it looks.** The first conversion attempt made the
hooks generic over each transaction's own tag, so every forwarding site had to reconcile two tag
types — which is why its errors cascaded rather than converged, and why it was reverted. With one
store-wide tag, forwarding is a value rather than a type to reconcile. The error count on the tag
change itself went 87 → 13 → 7 → 1 → 0, which is the convergence the generic version never showed.

**The second attempt also failed, and the measurement is the reason.** Converting the leaf
bodies and sweeping the callers moved the error count **170 → 181 → 207**. Rising under each
sweep is divergence: the seam shapes vary more than a regex sweep can assume, so each pass was
creating more mismatches than it fixed. I reverted rather than push on, because a half-converted
seam layer has neither property and looks like it has one — the same conclusion as the first
attempt, reached by a different route.

**What the conversion actually requires**, now that two mechanical attempts have failed: the
remaining ~40 transaction functions need converting **by hand, one transaction at a time**, each
with its own callers and injected-failure tests, rather than by pattern. The shapes differ enough
— some functions take a writer and a sync, some only one, some forward into others, some dispatch
on a tag — that sweeping them together is what diverges.

That is real work rather than a rename, and it should be done deliberately rather than at the end
of a long session. The type and the tag are in place so it is no longer blocked on a design
question; it is blocked only on the effort.

## I-4, slice 7: the root-sync exception was path-generic

The reviewer raised this as a conditional warning without having seen the source. It was a real
bypass, and the comment above it made it worse by asserting the property the signature lacked:

```rust
/// Named rather than path-generic, like the savers below.
pub(super) fn sync_vault_root(dir: &Path) -> std::io::Result<()> { sync_directory(dir) }
```

`sync_vault_root(&store.dir.join("servers"))` would have flushed the **inventoried** directory
with no capability. A path-generic directory sync wearing a specific name is not a restriction.

Fixed by the same rule the six savers already follow — **the implementation chooses the
destination**:

```rust
impl ServerStore {
    pub(super) fn sync_own_vault_root(&self) -> std::io::Result<()> { sync_directory(&self.dir) }
}
```

`ServerStore::open` constructs the store and flushes through it. A fifth probe confirms the free
function is gone: `E0425: cannot find function 'sync_vault_root' in module 'super::persistence'`.

The method stays reachable from siblings, deliberately: `open` lives in the parent and a parent
cannot see a child's private items. But it derives `self.dir`, so it can only ever flush the vault
root — the inventoried records live one level down in `servers`, which it never touches. The
exception cannot be redirected, which is the property that was asked for.

**A process note.** The reviewer observed that `gate4-agent1-runtime` still resolved to `9beaa9b`,
so the privacy fixes and this one were reported rather than inspected. That is the right thing to
insist on: two of my claims about this boundary have now failed under their reading, so my
description of it should not be load-bearing. Pushed before the next review.

## I-4, slice 6: the relocation's own test visibility had reopened it

The sibling module was the right architecture and the enforcement claim was still false, for a
reason worth recording because it is the same shape as everything else in this work.

**`pub(super)` from inside `persistence` means visible in `store`, and therefore visible to every
sibling epoch module.** Those markers existed so *tests* could reach the primitives. So the
relocation closed the door and its own test visibility propped it open:
`create_staging_file`, `atomic_write_with_hook` and `atomic_write_with_hook_and_sync` were all
reachable from an epoch module without a capability — and `create_staging_file` creates a
temporary sibling from an arbitrary path, which is explicitly one of the mutations I-4 must
invalidate. `sync_directory` and `StagingPath`, whose `Drop` unlinks, had not moved at all.

**My probe passed only because it named the one function that happened to be private.** It proved
"an epoch module cannot call `atomic_write`" and I reported it as "no path-generic physical
persistence API is reachable". The reviewer's instruction — probe the boundary, not a function —
is what found it. That is the ninth assertion in this work that proved less than it claimed, and
the third the reviewer caught by reading.

Now inside and private: the staging counter and constants, `StagingPath` and its `Drop`,
`AtomicWritePhase`, `sync_directory`, `create_staging_file`, both atomic-hook variants. The three
leaf flushes route their parent sync through `m.sync_parent_io(..)`, so even the directory flush
goes through the capability. The one legitimate non-inventoried directory flush became a named
`persistence::sync_vault_root(dir)`.

Test access is genuine `#[cfg(test)] pub(super)` **wrappers** around private functions, not
production-visible functions re-exported under `cfg(test)`.

| Probe from an epoch module | Result |
|---|---|
| `persistence::atomic_write_with_hook` | `E0603: private function` |
| `persistence::create_staging_file` | `E0603: private function` |
| `persistence::sync_directory` | `E0603: private function` |
| `persistence::write_for_test` | `E0425: not found` — absent in a non-test build |

**The contract this actually establishes**, in the reviewer's terms: no project persistence
primitive usable for five-family mutation exists without `EpochMutation`, backed by the audited
writer list. Not the stronger "a write cannot be expressed" contract — an epoch module holding a
`PathBuf` can still call `std::fs::write`, and §9.2's choke-point-plus-audit pairing reads as
accepting that.

## I-4, slice 5: the relocation, and what is provably enforced now

**The path-class gate condition is closed.** All physical persistence writing moved into
`store::persistence`, a **sibling** of the epoch modules rather than an ancestor, so items private
to it are genuinely beyond their reach — which is the only mechanism Rust offers, since a
descendant always sees an ancestor's private items and can construct an ancestor's private marker
types.

What escapes the module is deliberate and small: the `EpochMutation` capability, and six
**path-specific** savers for the non-inventoried records (`write_ui_state_record`,
`write_server_net_record`, `write_address_cache_record`, `write_pairing_ledger_record`,
`write_server_record`, `write_registry_record`). Each writes only its own record, so none can be
aimed at an inventoried family, and **no path-generic writer exists outside the module at all**.
Test-only re-exports carry `#[cfg(test)]`, so they cannot weaken the production property.

**Proved by trying it, not by reading.** A probe planted in `epoch_intents.rs` —

```rust
fn i4_enforcement_probe(store: &ServerStore, scope: &[u8], bytes: &[u8]) -> Result<(), AppError> {
    atomic_write(&store.epoch_intent_path(scope), bytes)
}
```

— fails with `error[E0425]: cannot find function 'atomic_write' in this scope`. Adding a bare
inventoried write is now impossible rather than forbidden by audit. The probe was removed after.

**The relocation also found the live bypass the reviewer predicted.** `epoch_studio/rotation.rs`
had a closure that received the capability and then called the bare primitive anyway. It had
already obtained a guard so it was not a correctness bug, but it compiled, which was the whole
point. It now calls `m.write(p, b)` and could not do otherwise.

### Requirement 3 is audited, not yet type-enforced

All 27 production writer closures are literally `|m, p, b| m.write(p, b)` or forwarders into one,
and the three multi-line ones in `rotation.rs` call `m.write` / `sync(m, ..)` synchronously. **No
production path defers I/O past the guard's lifetime.** That is an audit result.

The **type** still permits it, because the seam is an arbitrary closure receiving the capability.
Closing that needs the injection strategy inverted, which is a real change rather than a rename:

```rust
enum WriteIntercept<'h> {
    None,
    #[cfg(test)] Before(&'h mut dyn FnMut(&Path, &[u8]) -> Result<(), AppError>),
}
// transaction: intercept.check(path, bytes)?; mutation.write(path, bytes)?;
```

Production passes `None` and never supplies a writer at all, so deferral is unrepresentable rather
than merely absent. The cost is every injected-failure test that currently fails *by writing*, plus
the abort-phase tests that use `atomic_write_with_hook_and_sync` directly and would need capability
methods. I have not done it: it is the third large refactor in this area and the design above
should be ruled on before forty test sites are rewritten around it.

## I-4, slice 4: cleanup, and the one gate item that needs a decision

**I4-003 closed.** `EpochStorageCleanup` unlinked temporary siblings and synced the inventoried
directory without rotating anything — the exact "unlink or leave a temporary sibling" category
§9.2 names, in production, not test code. One capability now spans each destructive batch, taken
before the first possible unlink and held across the removals and the parent sync. That is the
"one rotation covers N mutations" property: the guard holds the store exclusively, so no cursor
can be captured between the first removal and the flush.

The physical syscalls moved onto the capability as `remove_io` and `sync_parent_io`, so the seam
decides *whether* to fail rather than owning the removal. `remove` is gone; `remove_io` is the
single unlink operation.

**M58**, swapping only that batch's guard for an unrotating stand-in, fails at "cleanup unlinked
an inventoried temporary sibling without rotating".

### Two claims I made that were wrong, corrected

**"Deferred I/O is unrepresentable" was an overclaim.** Removing `with()` was necessary but not
sufficient. The tagged writer seams still hand a callback the capability and let the callback
perform the write, and nothing stops a callback cloning the path and bytes, spawning, and
returning `Ok`. The stale-cursor race is reachable through that. It is closed for cleanup, where
the syscalls now live on the capability, and open for the seven writer seams.

**The path-class gap needs a relocation, not a newtype.** Worth recording why, because the
mechanism is not obvious:

- Rust module privacy cannot express "visible to `store` but not to `store::epoch_*`" — a
  descendant always sees an ancestor's private items. So any `atomic_write` reachable from
  `store.rs` is reachable from every epoch module.
- A private marker type in `store.rs` fails identically: descendants can construct it.
- The only mechanism that works is a module the epoch modules are **not** descendants of. But
  then `store.rs`'s own six non-inventoried writers cannot reach the primitive either, so **they
  must move into that module too**, which then exposes the capability plus *path-specific*
  non-inventoried savers and no path-generic writer at all.

That is a real relocation of where persistence lives. The reviewer declined to prescribe the
module rearrangement and I am not improvising it; the gate stays closed on this one item with a
concrete plan rather than a guess.

### Gate status

| C-3 gate condition | |
|---|---|
| replacement writes capability-only | **done** |
| unchanged-file sync repairs capability-only | **done**, eight branches |
| unlink / rename / temp-sibling capability-only | **done** |
| bare primitives unreachable from participating paths | **done** — `store::persistence`, with the primitives private rather than `pub(super)`; four probes fail to compile |
| no escaping writer callbacks in production | **audited clean, not type-enforced** — no production path defers; the seam type still permits it |
| Agent 2's archive writer and release | write and retry done; `release_studio_draft_archive_with_io` does not exist yet |
| audited leaf list reconciled | done for existing paths |
| reads, budget mint and entry proved not to rotate | done |
| cursor-level invalidation suite | belongs to C-3 |

674 passed, 0 failed, 11 ignored; clippy `--all-targets -D warnings` clean.

## I-4, slice 3: the capability, and the eight retry branches

**I4-001 closed, further than asked.** There is now **no `with()` at all**. Once every seam was
typed to take `&EpochMutation<'_>`, the escape hatch had no callers: injected-failure closures
receive the capability and do their own I/O. So the deferred-I/O hole — rotate, spawn, drop the
guard, let a cursor capture the new token, then mutate under it — is not merely unreachable, it is
inexpressible. The three leaf sync primitives take the capability themselves, so `sync_intent`,
`sync_registry` and `sync_studio` cannot be called without one.

**I4-002 was eight branches, not four.** Beyond the reviewer's intents, intent retirement,
registry and Studio source: `epoch_registry/head.rs`, `epoch_registry/receive.rs`,
`epoch_registry/page_source.rs`, `epoch_studio/discovery.rs`, and two in `epoch_studio/handoff.rs`
(the completed-handoff retry and the publish check). Agent 2's archive retry too, which the typed
seam forced me to touch; their file carries a comment saying so and naming what remains theirs.

### The gap that needs a ruling rather than a guess

§9.2 says the bare helpers "stop being reachable **for five-family paths**". That is not
expressible with a capability parameter on `atomic_write`, because the same primitive writes
`ui_state`, `server_net`, `address_cache`, the pairing ledger and the server record. I tried it
universally and saw the consequence immediately: saving a UI preference would rotate the token and
invalidate a captured inventory. So `atomic_write` stays bare and is reached for inventoried
records only through `EpochMutation::write`. **Closing this properly needs the path class in the
type** — separate primitives, or a newtype for an inventoried path. It is documented at the
primitive and is the one outstanding item on the C-3 gate list that I will not pick unilaterally.

### The eighth masked assertion, caught before it shipped

My first I4-002 test asserted "an exact retry flushed the record without rotating" — and passed
for the wrong reason. `epoch_recovery.rs` has no `reserve_sync` **at all**, so that writer has no
exact-retry branch; repeating the transition simply performed a second *replacement*, which
rotates anyway. The reviewer had already said as much about the recovery leaves, and I wrote the
test regardless.

The assertion now lives on a call that is *guaranteed* to take the sync branch, because its writer
panics if anything rewrites. **M57**, swapping only that branch's guard for an unrotating stand-in,
fails it at the named assertion.

That is eight assertions in this work that proved nothing until a mutation ran against them. Three
were caught by the reviewer reading rather than running.

### Evidence

673 passed, 1 failed, 11 ignored; clippy `--all-targets -D warnings` clean at zero.

The failure is the known scheduling install assertion. **A correction to my own earlier note:** I
described the clean 674/0 run after the registry-wake fix as what the investigation predicted. That
was over-read. The wake fix now has two samples — one clean, one failing with the same "Registry
must install through the reserved slot" — which is the unchanged ~20% rate. **The wake fix has not
demonstrably fixed the scheduling failure.** It remains a real production bug worth having fixed on
its own merits, and it is not the cure. The mode-B margin is also tighter than stated: owner returns
at 32250 against a 40 s bound, so the install had **7.75 s**.

## I-4, slice 1: the guard and its invariant

`inventory_generation` and `EpochMutation` exist. The guard is obtainable only from
`epoch_mutation_guard()`, which rotates **before** handing one out, so rotation precedes the
caller's first possible I/O rather than following a successful write. It is not undone on drop and
is not conditional on success.

Landed deliberately **before** the audited writer conversion. The list is 66 `atomic_write` sites
and 34 unlinks, and converting them on top of an unproven guard is the wrong order; the
unconverted primitives carry explicit markers naming what is pending, rather than being quietly
half-done.

Two things the first conversion surfaced:

- **The guard borrows the store, so paths must be resolved before rotation.** Gather, rotate, then
  touch disk. That makes "rotate before first I/O" a borrow-checker property rather than a
  remembered one.
- **`flush_checked_epoch_intents` held only `&self` while mutating disk.** The store did not
  require exclusive access to write. I-4 makes that a type error. The cascade was two levels deep.

**M52**, handing out the guard without rotating, fails at "a captured inventory would survive a
write it never saw". The negative half is asserted too: reads must not rotate, or a cross-visit
cursor dies on ordinary activity — which is why `studio_generation` could not be reused.

**Agent 2's answer closes the intent-write concern I raised.** Each appended operation is a full
record rewrite plus a store-wide `intent_generation` rotation, and a bulk copy is N of each — but
every item is a distinct user action and no path rotates in an automated loop. So the quiet memo's
invalidation rate is bounded by human interaction, not by a loop. They checked source rather than
answering from memory, and separately verified `intent_class` for DraftArchive against the
preservation guarantee, which was the one decision I had made on their behalf.

### I-4, slice 2: six writers converted

Recovery, accounted recovery, intents, intent retirement, owner state, registry and Studio source
now rotate through the guard. Three more `&self`-mutating functions surfaced in `epoch_studio.rs`
and were widened; the cascade settled in three iterations.

**A masked assertion I would have shipped, and the reason the design pairs a choke point with an
audited list.** The first per-family test asserted "an accounted recovery write did not rotate" —
and **M53 passed against a build with the accounted writer's guard removed.** It drives
`update_epoch_recovery`, a *different* write path, and never touched the accounted writer at all.
The message named a writer the test did not exercise, which is worse than no coverage because it
reads as coverage. Two writers reach the same family and only one was tested: covering each
**writer** is the point, not each family. The accounted path now has its own test beside its own
helper, and **M54** fails it at the named assertion.

`epoch_draft_archive.rs`'s writer is Agent 2's and is deliberately not converted here. The design
attaches that audit obligation to them when they build the writers; they are actively editing that
file and converting it underneath them would be worse than handing it over.

Still open for I-4: the cleanup unlinks, which need `EpochStorageCleanup` restructured because it
holds the store mutably across its whole pass; the injected-failure writer seams; and per-family
rotation evidence for the four families that still lack it. `EpochMutation::write` and `remove`
keep their markers until then.

### The two scheduling tests have **two** failure modes, not one

This corrects what was handed to the reviewer as two separate defects.

| Mode | Shape | Where seen |
|---|---|---|
| 90 s timeout | never completes; 6.8 s normally, 83 s headroom | branch head, twice |
| install assertion | progresses, then its own loop exhausts | CI at `db2798c`; twice locally |

**All three variants fail, and both halves of the assertion fire.** The `Ready` pressure variant
(`..._with_three_retained_previews`) has now failed too, at the Registry assertion, so it is not
confined to the two cancelled-preview cases. Across thirteen full runs since the registry-wake fix
the rate is roughly a quarter, with both modes and both halves appearing.

**Both halves of the assertion fire, in different runs.** CI saw "Studio must install", one local
run saw "Registry must install", the next saw "Studio" again — with the registry writer converted
in between. So it is not class-specific, which also disposes of a correlation worth naming: the
first local sighting coincided with my widening a signature in `epoch_registry/page_source.rs`, and
the next run failed on the *Studio* half instead.

~~**The margin is ~8 s, not 40 s.**~~ **Withdrawn — see the correction at the top of this
document.** The 40 s bound is measured from owner return, not absolutely, so the install has the
full forty. The progress markers are identical across sightings (`preview 0 ready at 9250`,
`preview 1 ready at 17500`, `owner returning at ~32000`), which remains a real observation; the
margin arithmetic built on them was not.

The assertion mode was believed confined to the merge checkout. It is not: it reproduced on this
branch. Both are plausibly one root cause — registry install failing to complete in time —
presenting differently depending on where it stalls, and chasing them as two bugs would waste the
effort.

**It is not the I-4 slice.** The mechanism rules it out: `inventory_generation` is written and
never read in production, the converted site performs the identical flush on the identical path in
the identical order, and the two widened signatures have no runtime effect. A stashed-baseline run
was clean (670 passed) — but that is weak evidence on its own at the ~20% failure rate these runs
show, and is recorded as corroboration rather than proof. The decisive part is that the same
assertion was observed at `db2798c`, a tree that cannot contain work written after it.

## Flow H checkpoint: accepted at `636cc58`

> **The source-level PASS is bound to `636cc58dfcaca5d1e10d7b3400b9e8cafe237efe` and to nothing
> later.** `ed4ba50` restores two lost open items and touches no Flow H code, but it was not
> inspected by the reviewer and the PASS is not extended to it by assertion. Any later commit on
> this branch — mine or Agent 2's — is outside it.

| | |
|---|---|
| **Flow H H1–H6** | implemented; correction chain through FLOWH-004-R3 **closed** |
| **Known Flow H residuals** | ten, all disclosed below, nonblocking for this checkpoint |
| **Separate defects** | the owner-return branch-head hang, and the `db2798c` merge-checkout reserved-slot assertion — two independent investigations, not one |
| **Not accepted or implemented** | Flow R, I-4, C-3, all eight §13 measurements, native exposure and Agent 2's P5 |

### What seven rounds of review actually found

Almost none of it was algorithmic. Every serious failure was **two representations of the same
scheduling fact drifting apart**:

| | |
|---|---|
| gate vs wake | a hold the commit arm never read; a deadline the driver never woke for |
| owner vs waiter | a worker outliving the job that spawned it; a pause whose release needed the turn the pause prevented |
| job vs completion | a completion routed by target rather than by the job that asked for it |
| deadline class vs deadline class | a per-target hold and a global capacity gate each claiming to be authoritative |
| one fact, two reads | `pending` and `wake_in` sampling the clock separately |

For the remaining work the instruction to myself is to look for that shape *first*, before looking
for conventional bugs. It is also why `pending` and `wake_in` no longer exist as a separately
sampled pair in production, and why `hold` was deleted rather than fixed: the cheapest way to stop
two representations drifting is to stop there being two.

### Sequencing: I-4 and C-3 are one boundary, and they come before Flow R

C-3's cross-visit inventory cursor depends on I-4 making its invalidation token trustworthy, so
they are a coupled unit rather than two items. Flow R must not be built on the current unbounded
inventory path and then split again afterwards — open items 1, 2 and 8 are all that same custody
problem, and R1 would add a fourth instance of it.

### FLOWH-004-R3: PASS

The seventh review returned **PASS / CLOSED** on `636cc58`, having derived the termination
invariant from source rather than accepting it: with no live job the scheduler has two mutually
exclusive modes, the capacity gate's and the rail's, and `handoff_probe` exhausts every reachable
exit. It also confirmed the precedence cannot starve a target — a target hidden behind the gate
could not have proceeded during those failed capacity attempts anyway — and that the residual
`try_acquire` fairness question belongs to the existing shared-pool model rather than to this
change.

Two omissions from the open list were found and are restored as items 9 and 10 below. Neither is
a new defect; both are things that had been disclosed and then lost.

### The seventh review: two deadline systems disagreeing

FLOWH-004-R2 closed on both counts. One finding survived, and it is the same shape at one more
remove: not two readings of one deadline, but two independent deadline classes.

**FLOWH-004-R3, P2: a due target hold could bypass a future capacity gate.** `handoff_probe`'s
capacity gate sits *after* its eligibility test, so a target whose own hold had expired passed the
eligibility test and then returned at the gate — while `probe_due` reported it. The two-second
pacing throttled the `try_acquire` correctly and paced the receiver not at all: for the whole
capacity wait the actor stayed on its active Studio cadence taking turns that could only return.

The invariant I offered to close R2 was therefore still false. The fix is the reviewer's
precedence rule: while `probe_retry_at` exists it dominates every per-target deadline, in
`probe_due` *and* in `wake_in`'s no-job branch, because that is exactly what the gate does.

| Mutation | Effect |
|---|---|
| M49: `probe_due` loses the precedence | "an expired target hold reported due underneath a future capacity gate" |
| M50: `wake_in` loses the precedence | `left: Some(1000), right: Some(2000)` — the unnecessary intermediate wake |
| M51: the no-eligible-target exit does not clear | `left: Some(3000), right: None` |

**M50 passed at first, and the reason is worth keeping.** One target cannot exercise it: the probe
only arms a gate when something is eligible, and an eligible target's own deadline has by then
expired, which `wake_in` filters out either way. It takes a *second* watched target held 30 s out
with the gate armed at 29 s, so the target deadline falls inside the gate. The reviewer predicted
this shape before the test existed.

**M51 was the reviewer's, entirely.** `handoff_probe` has two pre-capacity exits and only the
empty-rail one was guarded; a mutant deleting the other's clear passed both existing tests. The
new test took two attempts: driving it through `run` never reached the probe, because
`background_step` gates it behind `replay_ready` and the two-target fixture does not satisfy that.
It now calls `handoff_probe` directly, since the claim is about what that function does at that
exit rather than which scheduler turn reaches it.

That is five assertions in this work that proved nothing until a mutant ran against them, two of
them found by reading rather than running. The mutation step is not optional.

**Evidence caveat.** Agent 2 now commits to this branch (`0287910`), so full-suite counts include
their work. 668 passed / 0 failed is honest for my tests and is no longer a clean isolation of my
changes.

**Still open.** The list has been incomplete in every round: three when it should have been eight,
eight when it should have been twelve, twelve again missing three findings, then missing two, then
missing one plus an overstated bound, and then missing the precedence defect and the unguarded
clear site. What follows is what is known to be open, and is again not offered as a guarantee of
completeness.

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
6. **Stale pacing state no longer wakes for an unwatched `next_at`, but `quiet`, `next_at` and
   `hold_ms` still require pruning for bounded retained state.** The previous wording — "bounded
   rather than unbounded" — overstated it, and the reviewer was right to reject it. A target that
   is backed off and then becomes unwatched *before* it is ever successfully re-read and memoised
   quiet keeps its `next_at` and `hold_ms` entries for ever; rail filtering stops that entry waking
   the actor, but it does not remove it. Scheduling impact is bounded to the current rail;
   retained memory across unlimited target churn is not.
7. ~~An abandoned target is not re-probed on a timer.~~ **Closed by FLOWH-004-R1.** `wake_in` now
   publishes the earliest future `next_at` among rail targets when there is no job, so an
   abandoned target's backoff expiry wakes the actor. Restricting it to the rail is also what
   makes item 6 tolerable rather than a prerequisite: a deadline for an unwatched target can no
   longer wake anything.
8. **H1's "cheap by construction" claim does not hold for the interrupted-`Prepared` branch**,
   which still enters the synchronous `resolve_studio_handoff_with_io` backstop under custody.
   Flow R is what removes that, and Flow R is not started. Noted by the third review; it is
   excluded from this round's claim but must not disappear from the later custody review.
9. **`next_token` uses `saturating_add(1)`**, so at `u64::MAX` every subsequent job takes the same
   token and the routing added for NEW-10 stops distinguishing jobs. Unreachable at any real
   scale. **This item was disclosed in an earlier round and then silently vanished when the list
   was renumbered** — not fixed, not withdrawn, just lost, and the reviewer had to notice its
   absence. It is restored here for that reason as much as for the defect.
10. **`handoff_complete`'s mis-targeted `Prepared` and `Assembled` arms have no direct coverage.**
    The superseded-completion regression exercises `Cancelled` only. Test debt rather than a
    defect, previously disclosed and also absent from the renumbered list.

**On the renumbering.** Items 9 and 10 were both lost the same way: the list was rewritten as
prose each round rather than maintained, so an entry that was not part of that round's narrative
simply did not get carried. That is a worse failure than an incomplete list, because it looks like
progress. Entries are now only removed with an explicit "closed by" line, as items 3, 4 and 7 have.

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
"Implementation progress". One shared contract document has been changed: `docs/INTERFACES.md`'s
"Closing overlay foundation" paragraph, which describes this scope's own adapters and had gone stale
- it listed two Save outcomes, a tenure accessor Agent 2 has since removed, and a request without a
branch. No workflow, no native command registration and no frontend file has been changed, and
nothing is merged to `Create-suite-2`. The remaining planned files are in design 5 and 15.

## Flow S carries branch identity: the generation namespace now runs on the real Save path

Agent 2 found that `classify_request`, `admit_new_branch` and `new_admitted` had **no production
callers** - Flow S carried `basis` only, so the branch-generation namespace they built never ran on
any real Save, and a delayed request for a disposed branch could be accepted into a new one. A
P1/P5 blocker, and mine. Verified in source, then fixed once Agent 2 exposed the derivation
(`e5205318`: `request_branch_id`, and `admit_first_branch` for a document with no record).

### What the Save path does now

| stage | before | now |
|---|---|---|
| preparation | basis only | a `StudioOverlaySaveTicket` - the basis **and** the branch the Save must name, both from one fresh basis in one custody visit, the branch from `request_branch_id` and never from the caller |
| S1 | bare `completed_retry`, keyed by basis | `classify_request` against the branch the request names: `Transferred` -> `HandedOff`; `Disposed` -> the new terminal `StudioOverlaySave::Disposed` (design N17); `Active` -> exact-retry recognition, else on to S1b; `Unmatched` -> on to S1b |
| S1b | basis match, then media | basis match, then **admission**: `admit_new_branch`, or `admit_first_branch` when there is no record. `Stale` is refused here, before any media work |
| S2 (plan) | `unwrap_or_else(StudioOverlayState::new)` + an `append` that mints the next generation whenever no branch is live | the branch S1b decided: the live one, or one built by `new_admitted` / `new` - each **rechecked** against the record the worker decodes, and refused on disagreement |

**Neither production Save site can open a branch on its own any more.** One was
`write_studio_overlay_intent`'s new-authoring tail, unreachable since new authoring moved to the
staged path (its only caller passed `basis: None`) but still a second place a branch could be
opened; it is removed and the function is now the exact-retry flush barrier it had become. The
other was the plan's `unwrap_or_else(new)`, replaced as above. `append`'s own minting stays for
now - it is Agent 2's, and refusing there must land after this or the current Save breaks in
between. That change is theirs, on top of this commit.

**Terminal acknowledgements still come first.** Transferred, disposed and exact-retry outcomes are
all decided before S1b requires tenure, mints a basis, reads a source or touches media. As first
committed (`baf8a9cf`) that sentence was **true of the order and false of the coverage**: a
transferred branch stopped being acknowledged once a newer branch was admitted. See the review
record below; fixed in the following commit.

**The rollover floor still guards new branches.** Checked rather than assumed: `append` calls
`check_basis_floor` unconditionally, and `new_admitted` carries `minimum_new_basis_closed_epoch`
across, so a branch opened through admission is floor-checked in the plan exactly as before.

### One correction to my own plan, from Agent 2

I had said a request whose branch is admitted by someone else between preparation and S1b comes
back `Stale`. **Half right.** The identity is a pure function of basis and generation, so if the
winner was admitted on the *same* basis it carries exactly the id the late request holds, and that
request is `Active` and appends - the outcome preparing after the admission would have produced.
Only a winner on another basis, or a branch disposed or transferred in between, makes it `Stale`.
No test here expects `Stale` for a same-basis race.

### Tests, each broken on purpose

| test | what it proves | mutations, all killed |
|---|---|---|
| `a_delayed_request_for_a_branch_whose_manifest_was_replaced_is_stale_on_the_save_path` | design N17b through the real Save entry and the production disposal transaction, both targets: G1 accepted and discarded, a disjoint G2 on the same basis accepted - **then the delayed G1 request is acknowledged as `Disposed`** (the control: G1's manifest is still retained) - G2 discarded, restart, G2's delayed request acknowledged, **and the identical G1 request now `Stale`**, opening no branch, no ledger entry, no generation step | namespace bypassed in S1b and the plan's old minting restored: the G1 request comes back `Ok(Local(.. accepted: 1 ..))` - accepted into a new branch, which is the hazard itself. `Disposed` classification suppressed in S1: the control fails |
| `a_first_save_naming_an_identity_no_admission_would_mint_is_stale` | the no-record case `admit_first_branch` exists for; the same operation with the ticket's branch is accepted as the positive control | the same namespace bypass |
| `a_durable_prepared_handoff_is_resolved_with_no_observed_tenure` | V8's third case, in both outcomes resolution can reach. **`Complete`** (crash after the source landed): settles with no tenure to the same outcome and byte-identical source as a known-tenure control on a copied vault. **`Absent`** (crash before it): resolution returns the branch to Active with no tenure; only the *new* handoff that follows is refused, and should be | a tenure requirement hoisted above resolution in H1: "resolution refused with tenure None". Killed by the `Complete` half; the `Absent` half's distinguishing assertion - Prepared resolved despite the refusal - would also fail under it by inspection, but was not run in isolation |

**The first version of that last test was wrong, and the failure was the test's, not V8's.** It
crashed before the source landed and expected a settled outcome under no tenure. Resolution of
that case returns the branch to Active - tenure-free and durable - after which H1 starts a new
handoff, which is new authoring and correctly needs tenure. I had conflated resolution with what
follows it. Reading `resolve_studio_handoff_with_io` showed its two outcomes, `Complete` and
`Absent`, and the test now claims exactly what each one does.

Running the control and the hazard **in one test, on the same request** is the point of the first
row: Agent 2 noted the shorter dispose-G1 / admit-G2 / retry-G1 sequence does not reproduce the
hazard because the retained manifest catches it. Showing the acknowledgement turn into `Stale` at
exactly the moment G2's disposal replaces G1's manifest is what demonstrates that the long sequence
is doing the work.

### Existing tests whose meaning changed, rather than just their arguments

- **The rollover-floor test now proves two defences separately.** The forgotten retry resends the
  branch its own ticket named, and is now refused by the **namespace** as `Stale`, before the plan
  where the floor lives. A request prepared *after* the rewind is handed the current next branch,
  so the namespace admits it and only the **floor** stops it, with `EpochScope`. Before this, the
  floor was reachable only because nothing refused earlier; this is the first version of the test
  that shows it holding on its own.
- **Every retry after a transfer now resends its original branch.** After a transfer the generation
  does not move, so a fresh derivation names the *next* branch; a test that re-derived would be
  testing a request no client sends. Those tests capture the branch while it is live, as its client
  would hold it.
- **Tests that assemble a capture by hand now pass a branch and a decision**, taken from the same
  core functions S1b calls - and the plan's recheck means a wrong pair would be refused, not
  trusted.
- **One assertion was tightened because it misreported my own fixture mistake.**
  `studio_overlay_uncertain_acceptance_still_protects_its_pixels` asserted only `failed.is_err()`.
  I first placed the branch derivation between the test's budget and its Save; deriving creates
  fresh budgets, which made the outstanding one stale, so the Save was refused with "Studio budget
  is stale" before ever reaching the protection transfer the test exists for. `is_err()` accepted
  that, and the failure surfaced only at the later pixel check as "left its pixels reclaimable" -
  which reads as an I-3 regression and was not one. Confirmed by re-running the old placement under
  the tightened assertion, which names the real error. The assertion now requires the injected
  failure, and the derivation runs first, so the fixture's own order is unchanged.
- **Retries after a rotation read the branch from the record, not from a fresh derivation.**
  `prepare` in the handoff tests rotates to a successor, after which no Closing basis can be minted
  at all; `live_branch` reads the live branch's id, which is what the Save's client holds.

### Live tenure at my V1 sites

**Preparation now requires tenure through the typed seam.** `Server::prepare_studio_closing_overlay`
calls `require_observed_owner_tenure()` first, and that function's `expect(dead_code)` is gone, as
Agent 2 asked for its first caller. Preparation is the one V1 site where requiring first is
correct: it mints a fresh basis and has no terminal path, so nothing V8 protects can be stranded by
refusing before it.

Acceptance is unchanged - both the old accessor and the seam admit `Known` alone. What changes is
that `Imported` and `Unknown` are refused **apart**, because they describe different things the
device holds - nothing, or an unverifiable imported value. **Corrected:** this paragraph first said
"one is fixed by observing the owner take office, the other is not fixed by waiting". That was
false, and I put the same claim in a code comment. A review of Agent 2's traced
`OwnerTenure::applied`: both end at the same event, the next contiguous step that derives a fresh
tenure (an owner change, or the committer's membership restarting on a new leaf), and neither is
cleared by elapsed time or an ordinary commit. Now pinned by
`owner_tenure_imported_and_unknown_both_end_at_the_next_observed_owner_change`. The split is still
right - P2 now carries `TenureImported` beside `TenureUnknown` - but it is about what the device
holds, not about how the state ends.

**Tested on a real joiner, with the founder as control** -
`preparation_refuses_a_member_with_no_observed_tenure_and_says_which_case`. A plain joiner's
`Unknown` is asserted as a precondition rather than assumed, so if joiners ever start observing
tenure the test fails on that instead of passing vacuously. Both servers get identical arguments
that would refuse either one; the founder must get past the requirement and refuse for a
non-tenure reason, the joiner must refuse at it. **Two mutations, both killed:** reverting to
`authoring_owner_tenure_start()` yields the store's generic "needs observed owner tenure"; removing
the requirement lets the joiner through to "source missing; fetch before sealing".

**V8 holds on both flows, and every terminal case is now anchored under absent tenure.** An exact
Save retry - `studio_overlay_store_uncertain_writes_and_changed_source_retry_at_physical_cap`,
which I wrongly reported as unanchored: its two `None`-tenure calls are exact retries. A completed
handoff acknowledgement - three tests, now passing `StudioOwnerTenure::Unknown`, plus
`a_transferred_branch_is_still_acknowledged_after_a_newer_branch_is_admitted` for the case the
review found. A disposed-branch acknowledgement - the N17b test's two `Disposed` answers, moved to
`Unknown` after the review pointed out they ran under a known tenure. A durable-Prepared resolution -
`a_durable_prepared_handoff_is_resolved_with_no_observed_tenure`, the one that genuinely was
missing: every earlier resolution passed `Some(0)`, so a requirement hoisted above it would have
passed every test. ("All three" in `baf8a9cf` predated the fourth case that commit itself added.)

**S1b and S3 now require tenure at their own points, through the typed value.** The Server *reads*
`observed_owner_tenure()` and passes the `Copy` value down; S1b and S3 each call
`require_owner_tenure` after classification, so `Imported` and `Unknown` are refused there with
their own messages. My first plan - pass a `Result<u64, AppError>` and apply `?` at each stage -
does not work, as Agent 2 pointed out: `AppError` is not `Clone` and both stages need the value.
The typed value keeps everything that plan wanted: refusal at the stage (A-1), V8's order, and no
laundering, since `convert` is the only producer and `require` is exhaustive. `require` is
`pub(crate)` for this, re-exported as `crate::studio::require_owner_tenure`.

**Not changed: H1.** The handoff's store signatures are not touched by the branch change, so the
reason for bundling - not churning the same call sites twice - does not apply to them, and the
scheduled runtime already holds a target with no tenure rather than surfacing a message. It still
refuses correctly with the generic text. Recorded as a separate, small follow-up rather than
silently dropped from the plan Agent 2 agreed to.

### The six-client native restart failure: flaky, and it entered with PR #29, not this branch

Reported to me as "the six-client restart failure, inherited from their base". The native test is
`six_client_recovery::six_client_native_restart_and_partition_recovery` (`apps/desktop/src-tauri`).
It entered this branch through the merge `a12209bc` of PR #29 (`Chat-method-redesign`), whose last
commit is "Record six-client recovery acceptance".

**It is flaky on this host, not failing.** Five runs at each of three points:

| commit | what it is | passed |
|---|---|---|
| `2e87cf12` | PR #29's own head - no gate4 code at all | **2 / 5** |
| `a12209bc` | the merge into this branch | **3 / 5** |
| `74a5e577` | this branch's head | **2 / 5** |

The same rate at the PR's own head is what rules out a gate4 regression: the code this branch adds
is not present there. The earlier "fails reproducibly" attribution came from single runs.

When it fails, it fails the same way every time: after all six rejoin, each partition has the common
chat and the post-restart conversation but **not the other partition's isolated-period messages**.
Live gossip after the heal works; the backfill does not land inside the test's 90 s wall-clock
convergence bound. `MESSAGE-FLOW.md` 4.4 gives the reconcile sweep 30 s and 31 s minimum retry
delays, and passing runs sometimes take up to ~160 s in total, so the bound and the retry cadence
are close enough that host speed decides the result.

Not fixed here and not mine to fix: whether healed partitions should converge faster or the test's
bound should change is a chat-sync decision for PR #29's owner. Two notes for whoever takes it: CI
has never run this test on gate4 branches, because the Linux job's `npm test` fails first on three
native-command registration checks; and one failure is not evidence of anything - run it several
times.

### CI: the handoff mutation harness went dark at `baf8a9cf`, and that was mine

Reported to me as "the studio-handoff completed-target failure, inherited from the base". **It was
not inherited.** The "Studio Closing overlay handoff" workflow passed at `e5205318` and has failed on
every commit from `baf8a9cf` on, with
`mutation did not fail at its intended assertion: completed-target`.

The cause is the S1 ordering this work introduced. The `completed-target` mutation removes
`check_target` from `completed_retry` and expects a retry for another channel to be wrongly
acknowledged. Since `baf8a9cf`, S1 classifies through `classify_request` first, which runs its own
`check_target` and refuses that retry before `completed_retry` is ever reached - the fallback runs
only after classification has passed on the same target. So the old anchor became a guard no Save
can observe removing: an equivalent mutant. The entry now mutates `classify_request`'s check, the
one that actually stands in the way; it is detected at the intended assertion
("completed retry acknowledged a different channel") and the restored regression passes.

**The worse half: every other entry has been unchecked since.** The harness raises on the first
undetected mutation, and `completed-target` is first in the list, so from `baf8a9cf` to now none of
the other handoff mutations has run in CI. Re-running the full harness after the fix found exactly
that: **a second entry, `retry-floor`, was also failing, invisibly.** It disables the rollover floor
and expected the old assertion text of the rollover test I split. The mutation *is* detected - the
split test fails at its floor half, and with the request **accepted**, `Ok(Local(..))`, which is the
evidence that the floor is now independently load-bearing - but the harness was matching a message
the test no longer prints. Entry updated to the new assertion.

**Full harness after both fixes, at `b7b7a1d0` (which includes Agent 2's `append` refusal): all 10
mutations detected at their intended assertions, all 10 restored regressions pass.** The first
complete pass since `baf8a9cf`.

**Why my gate missed it.** The owned-surface run I adopted after the earlier scope failure runs the
test trees; it does not run this workflow's mutation harness, which changes source and expects
specific tests to fail. A change to *which check guards a path* passes every test and breaks the
harness, and only the harness can see it. Mutation harnesses for code I touch are now part of the
gate, not something CI tells me about afterwards.

### Review of `baf8a9cf`: PASS WITH FINDINGS, one of them a regression of mine

A fable review, read-only against the committed blobs. It confirmed the hazard closed: it traced
N17b through the production Save and found S1b refusing the delayed G1 request before media, and
found **no production path that can open a branch except through the plan's recheck**. Five
findings, all verified in source before acting:

| # | finding | verdict | disposition |
|---|---|---|---|
| 1 | a transferred branch's acknowledgement stops being owed once a newer branch is admitted: `classify_request` derives the transferred identity at the *current* generation, while the `completed_retry` I replaced keyed it on basis and operation | **regression, mine** | fixed: `Unmatched` falls back to `completed_retry` before the pending check, where the old code had it. Acknowledgement only, never acceptance. New test written **first** and run against the unfixed code: it failed, refused not for tenure as the review predicted but earlier, as "ordinary intent cannot become an accepted overlay" - a transferred operation stays pending until a rotation retires it |
| 2 | a ticket naming a live branch on a superseded basis, or a Prepared one, cannot succeed, but was refused in the plan **after** media promotion | pre-existing shape, now an explicit product of preparation | fixed: S1b refuses both, with the same `EpochScope` / `EpochClosed` the plan's `append` gave, ahead of media. Anchored in `source_version`'s accepted case with a frame naming unpublished pixels; removing the check makes it answer "publish the frame PIX", so the order is observable |
| 3 | the new `Disposed` acknowledgement was only ever tested under a known tenure | untested | fixed: both `Disposed` answers in the N17b test now run under `Unknown` |
| 4 | `INTERFACES.md` still listed two Save outcomes, a removed tenure accessor and a basis-only request; the commit message said "a hoisted requirement kills it" without the ledger's qualification that the `Absent` half was not run in isolation | overclaimed / stale | `INTERFACES.md`'s Closing-overlay paragraph rewritten to the current contract. The commit message is pushed and stays as it is; this row is the correction |
| 5 | the acknowledgement flush decoded the whole record with the replaying reader to learn its physical size | inherited inefficiency | fixed: `read_scoped_intent_plain` for the size, as the other flush barriers already do |

Finding 1 is the one that matters, and it is worth being exact about its cause. `classify_request`'s
degradation after a newer admission is documented in Agent 2's own comment as acceptable - "the
same degradation design 6.6 accepts for forgotten disposals ... refusal, never acceptance" - and as
a statement about *safety* it is right. What it is not is V8-neutral, and I took the classifier's
safety argument as covering availability without checking. The old basis-keyed check already gave
the acknowledgement, so restoring it costs nothing in the namespace and needs no layout change.

## G4-A1-S: Flow S over either basis (store seam)

Flow S is now one algorithm for both provenances, as design 8.7 of Agent 2's design asks ("Agent 1's
Flow S unchanged", with the mint substituted). Only the mint differs, and it is consumed at exactly
two points, S1b and S3, both reached only by new authoring.

| piece | where | what it does |
|---|---|---|
| `StudioOverlayMint` | `store/epoch_studio/overlay_capture.rs` | `Closing { close, tenure }`, minted by the store from the installed source exactly as before; or `Unconfirmed(Result<StudioUnconfirmedOverlayBasis, AppError>)`, the caller's live-preview mint **attempt**, failed or not |
| `mint_studio_overlay_basis` | same | the one place S1b and S3 obtain the fresh basis, so the two cannot drift |
| `OwnedOverlayBasis` | same | what the capture and plan carry: an owned basis, never the preview's seed handle |
| `start_studio_overlay`, `commit_studio_overlay_with`, `save_studio_overlay` | `store/epoch_studio/overlay.rs` | the general entry points; every `*_closing_*` entry point is now a thin `Closing` wrapper with its exact old signature |
| `Server::mint_unconfirmed_overlay_basis` | `studio_exchange/provisional/seed.rs` (Agent 2's file) | seed-scope recheck, then sync's sanctioned mint; the app's only route to an Unconfirmed basis |
| `StudioOverlayBasis::target()` | `catcoms-replication/src/studio/overlay.rs` (Agent 2's file) | widened to `pub` for the target check below |

**What the store enforces for Unconfirmed, at S1b and again at S3:**

1. **No stored source** for the document, probed under custody. Sync's mint cannot see the store.
   It is a metadata probe, not a source read: any entry at the record's path refuses, including a
   corrupt or non-regular one, and absence must also agree with the budget, so a record unlinked
   while still accounted refuses. This comes before the mint result is opened, so "installed
   source" is the answer even when the preview has also expired. (The first cut used
   `checked_studio_source`, which restored the whole source on the actor just to refuse; review L4.)
2. **The mint attempt**, surfaced verbatim.
3. **The basis is for exactly this target**, else `EpochScope`. Not cosmetic, and found by
   mutation: a Flipnote's logical key omits its channel, and with the check removed a basis
   minted for one channel opened a branch for a request naming another.
4. **The mint is of this MLS epoch**: it recorded the current MLS epoch, and its provider is still
   a member. The sanctioned mint cannot fail this in the visit it was made. It refuses a basis
   kept across an MLS-epoch change (review L2), and nothing more: a basis kept **within** one MLS
   epoch, past its hint's expiry or its preview's eviction, still passes. Minting in the same
   custody visit as the stage that consumes it is therefore a caller obligation, and
   G4-A2-PREVIEW must enforce it. The membership half cannot fire alone, since any membership
   change advances the epoch; it is defence in depth.
5. Fingerprint equal to the request's basis, then branch admission, exactly as for Closing.

A commit refuses a mint of the other kind than its plan, by name, before anything reads the store
(review L3).

No tenure is consulted anywhere on the Unconfirmed path (Agent 2 design 8.5).

**The ordering property, both kinds.** Classification, `Transferred` and `Disposed`
acknowledgements, `completed_retry`, exact retries and the ordinary-pending refusal all run before
the mint is looked at. For Closing that is V8 unchanged. For Unconfirmed it is the analogue: a
preview that expired, was evicted or vanished on restart blocks new authoring and nothing else.

**S3 re-enters the live check.** The commit takes a mint attempt made in the commit visit, never
one parked with the plan. The fingerprint is stable across a preview refresh (provider, MLS epoch
and time are admission facts, not fingerprinted), so a refreshed mint still commits. The stamp
covers only the Intents record, so **a confirmed source received over the network while the plan
is detached is caught only by S3's installed-source check**, not by the stamp. That is the case the
re-run exists for, and it is tested directly.

**Closing is unchanged.** Error text is byte-identical. The provenance a new branch records now
comes from `basis.provenance()` instead of a hard-coded `Closing`, which agrees by construction.

**One check I added and then removed.** A "live draft of the other kind" refusal in S1b changed
only the error text. Fingerprint domains differ per provenance, so a cross-kind request can never
equal the live basis: it is refused as `EpochScope` when it names the live branch, and `Stale`
otherwise, because `admit_new_branch` never admits beside a live branch. An untestable guard that
only rewords an existing refusal belongs in G4-A1-MAP, not here.

**Not wired in production.** The general entry points, the `Unconfirmed` variant and the Server
mint carry `#[cfg_attr(not(test), allow(dead_code))]`, each commented as waiting for G4-A2-PREVIEW
(Agent 2's preview Save: Server/actor Save, rails, reconciliation, native results). That work, and
G4-A1-MAP's structured reasons, are not part of this change.

### Tests, on a real fetched preview, each guard broken on purpose

`studio_exchange/tests/provisional/seed/tail/unconfirmed_save.rs`. Every basis comes from the
production mint over a preview fetched, parsed and tail-completed over the wire. The saving member
is a plain joiner, asserted `Unknown` as a precondition.

| test | proves |
|---|---|
| `..._first_append_joins_survives_refresh_and_retries_without_a_preview` | Index and Flipnote first append; the recorded provenance is the mint's own provider, MLS epoch and time; no source written; a mint one second later names the same basis and joins the live branch; after a restart the mint fails, yet the exact retry is answered from the reconstructed branch; new authoring surfaces the mint's own error and changes nothing |
| `..._refuses_beside_an_installed_source` | refused at S1b with the preview still live; the ordinary Apply's own Intents row is left byte-identical |
| `..._commit_reruns_the_live_check_after_the_detached_plan` | staged form: a source **received** during the detach (stamp unmoved, precondition asserted), a **local** Apply (the stamp answers first) and **expiry** all refuse at S3 and write nothing |
| `..._refuses_stale_requests_and_a_basis_for_another_channel` | Unconfirmed wording for a changed basis and a stale branch; a basis for another channel of the same Flipnote object refuses `EpochScope` |
| `..._exact_retry_is_answered_beside_a_newly_received_source` (review M1) | accept, receive the confirmed epoch over the network, retry exactly: acknowledged, nothing opened; new work beside the source refuses |
| `..._refuses_a_corrupt_or_unlinked_source_rather_than_reading_it_as_absent` (M2) | a corrupt record refuses as an installed source; one unlinked while the budget accounts it refuses with `BudgetError::Inventory`; a sentinel mint failure is never what comes back |
| `..._commits_from_a_replacement_preview_and_after_restart` (M3) | staged success: S1b from preview A, commit from a separately fetched preview B; the branch keeps A's admission facts; after a restart a third preview appends |
| `..._refuses_a_basis_minted_under_an_earlier_mls_epoch` (L2) | a basis kept across a real membership commit refuses at S1b |
| `unconfirmed_plan_refuses_a_closing_commit_mint_by_name` (L3) | an Unconfirmed plan given a Closing mint refuses with that reason, not an unrelated one |

| mutation (applied in the isolated worktree, restored after) | killed by |
|---|---|
| drop the installed-source refusal | `refuses_beside_an_installed_source` and the S3 `received` case: both Saves succeeded |
| drop the target check | the cross-channel Save succeeded |
| refuse a failed mint before classification | the exact retry after restart |
| skip the Unconfirmed re-mint at S3 | the S3 `received` and `expiry` cases |
| ignore the budget's absence check (`verify_record(None)`) | the unlinked case heard the sentinel |
| treat a present but unreadable record as absent | the corrupt case got `Inventory` instead of "installed source" (the budget still refused: two layers) |
| drop the MLS-epoch/member guard | the kept basis was accepted |
| drop the plan/mint kind check | the commit failed with an unrelated tenure refusal instead |

### Review of the first cut: no blocker or high; three mediums and six lows, dispositioned

An Opus adversarial review (Fable was rate-limited), read-only against the isolated worktree's diff
at `e5a52386`, with the focused tests executed. It verified: the Closing wrappers are exact
equivalents; the mint is consumed only after classification, exact retry and the pending check;
cross-kind requests are refused without the removed check; provenance always comes from the basis;
the cross-channel test pins the target check and nothing else; no new construction path; and every
anchor in the six mutation harnesses still matches exactly once.

| # | finding | disposition |
|---|---|---|
| M1 | no test of an exact retry after a confirmed source arrives; moving the presence check ahead of classification would pass every test | **fixed**: `unconfirmed_save_exact_retry_is_answered_beside_a_newly_received_source` |
| M2 | "a corrupt or unreadable source refuses" was claimed, not tested | **fixed, and the check itself changed** (see L4): `..._refuses_a_corrupt_or_unlinked_source_rather_than_reading_it_as_absent`, with a sentinel mint failure that must not be what comes back |
| M3 | replacement and restart-append untested; the commit doc overclaimed ("survives the ready entry being replaced"); the acceptance row and Agent 2's 8.1 "not yet built" note are stale | **fixed**: `..._commits_from_a_replacement_preview_and_after_restart`; the doc now says exactly what holds (same seed and receipt match; another candidate does not); acceptance row corrected; Agent 2 asked to update their own 8.1 note |
| L1 | S1b binds the target only; a basis with another author or group passes S1b and is refused at S2 after media work | **fixed** once Agent 2 added `author()` and `document()` (`f3ce1758`): the Unconfirmed mint now refuses a basis naming another group's document, then one minted for another device, before media work. `unconfirmed_save_refuses_a_basis_for_another_device_or_group_at_s1b`: the provider offers the store the saver's basis, and a second independent group's basis is offered to the first. Dropping the document check lets the author check answer; dropping the author check moves the refusal to S2 as `EpochAuthority` |
| L2 | the store has no evidence the mint attempt is fresh; a kept basis would skip every live recheck | **fixed**: the basis must record the current MLS epoch and a provider still in the group; `..._refuses_a_basis_minted_under_an_earlier_mls_epoch` hands the store a kept basis after a real membership commit |
| L3 | the plan does not record its kind; a wrong-kind commit mint fails closed with misleading text | **fixed**: refused first, by name; `unconfirmed_plan_refuses_a_closing_commit_mint_by_name` |
| L4 | the presence check ran a full `restore_unit` on the actor just to refuse | **fixed**: a metadata probe. Any entry at the record path refuses; absence must also agree with the budget, so a record unlinked while accounted refuses |
| L4b | eager minting copies up to 2 MiB of seed bytes per attempt, even for retries that never use it | **follow-up for G4-A2-PREVIEW**: the mint needs `&sync` while the store runs inside `with_registry_context(&mut)`, so laziness needs the caller's design |
| L5 | provenance-stays, a generation-2 Unconfirmed branch over an existing record, and the staged success path were untested | provenance-stays and staged success **fixed** (first and replacement tests). Generation 2 after a disposal is a **follow-up**: it needs the disposal flow, and a regression to hard-coded `Closing` there fails safe at `new_admitted`'s own agreement check |
| L6 | `seed.rs` said the store adds "the one check"; `EpochScope` now covers three cases G4-A1-MAP must tell apart | doc **fixed**; the mapping note is carried into G4-A1-MAP |

### Re-review of the fixes: M1-M3 closed; no blocker, high or medium; eight lows

A second Opus pass, read-only and static, against the same worktree. It confirmed each new test
reaches the guard it names, that the metadata probe cannot miss a real record (symlinks, case,
temporaries and orphans all considered), that `verify_record(None)` cannot refuse a legitimate
first append, that the MLS guard compares the fresh mint with the current group rather than with
the branch's recorded epoch, so a live branch is not stranded by a membership change, and that no
mutation-harness anchor moved.

| # | finding | disposition |
|---|---|---|
| 1 | the freshness wording overclaimed: the guard catches a basis kept across an MLS-epoch change, not one kept within an epoch past expiry | wording **fixed** here; the same-visit mint is recorded as a G4-A2-PREVIEW obligation |
| 2 | the membership half of the guard cannot fire alone | labelled defence in depth |
| 3 | no test that a live Unconfirmed branch accepts a fresh mint's append after an MLS-epoch change; tightening the guard to the branch's recorded epoch would strand every branch unnoticed | **fixed** in the follow-up commit: `unconfirmed_branch_accepts_a_fresh_mint_after_an_mls_epoch_change` (third member joins, preview refetched, append accepted, provenance unchanged) |
| 4 | a failed parent-directory probe does not invalidate the budget, though the comment implies parity with `checked_studio_source` | **fixed**: every probe failure invalidates the budget; a present record is not a failure. No test: it needs an IO fault on the vault directory |
| 5 | the kind check's position before the store reads is not pinned, and the reverse pairing is untested | **fixed**: the Unconfirmed-plan test now hands the commit a stale budget, and `closing_plan_refuses_an_unconfirmed_commit_mint_by_name` is the reverse pairing on a real Closing capture. Moving the check after budget entry fails both with "Studio budget is stale" |
| 6 | this section's tables and executed-checks line were stale | **fixed** here |
| 7 | `catcoms-sync/.../provisional/seed.rs:229-230` (Agent 2's) says the app checks before minting; it checks after the attempt, before opening it | passed to Agent 2 |
| 8 | no S3 test with a real preview of a different candidate | follow-up: the fixture's candidates are deterministic, so a different candidate needs a second fixture |

**Executed** at `e5a52386` plus this change, in `M:/catcoms-a1-verify`: see the commit message for
the final run; the first cut's run at `c6f7fea0` passed 838 app and 344 replication library tests
with every integration binary green, and frontend `npm test` passed 1282 of 1282.

## Agent 2's asks after Flow S (2026-10-07)

From Agent 2's reply to the Flow S note. Each row says where it stands.

| ask | state |
|---|---|
| M1: the H1 handoff probe must skip non-Closing branches | **done**: the probe selects only a live branch this device authored whose `live_overlay_provenance()` is `Closing`; an Unconfirmed one is memoised quiet (design 7.2), so it takes no reservation, no backoff, no capture. Regression `the_handoff_probe_leaves_an_unconfirmed_branch_alone`, before and after a confirmed source arrives, with the mutation (any provenance) executed and failing. Reviewed (Opus, static): no blocker, high or medium; the review's LOWs (comment precision, the installed-source variant, `live_overlay_provenance`, 7.2) are taken. Still untested: an Unconfirmed target ahead of an own Closing one on the same rail, which needs a member with tenure on that fixture |
| `new_admitted`'s redundant `provenance` parameter | **done by Agent 2** in `30194a40`, by agreement: the provenance is the basis variant's own, and their test now pins that each basis yields its own |
| where the 8.3 per-server and vault-wide rails are called | **done by Agent 2** in `30194a40`, at the points named: S1b (after the branch half, before media admission) and S3 (after the fresh mint's fingerprint check, before pixels, holds and the write), from the budget entered in that call |
| `save_overlay` reports another request's plan as `Saved`, and only takes its own target's plan | **agreed, waiting**: mirrors Agent 2's Unconfirmed fix (`Busy`, never `Saved`; take any parked plan); in `studio/receiver.rs`, so it lands with C-3 step 2 |
| the `cfg_attr(not(test), allow(dead_code))` markers | **removed** where the preview Save is now the production caller, with their "until G4-A2-PREVIEW" comments |
| a refused S2 plan is dropped, so a deterministic refusal leaves the Save answering `Scheduled` indefinitely (found by the G4-A1-MAP inventory) | **sent to Agent 2** as a likely HIGH in their live Unconfirmed path, with a proposed fix and regression; mirrored in the Closing `save_overlay` with its other two fixes |
| L4b lazy minting | deferred; the API is mine |

## G4-A1-CORE: the core signing review, and its two test findings

The SHA-pinned review of `8190dc4..e65bfd8` returned a **bounded PASS for the production code**
with two medium test-coverage findings; the verdict, findings and what was executed are recorded in
`GATE4-HANDOFF-SIGNING-REVIEW.md`. A finding re-review then closed SIGN-TEST-002 and narrowed
SIGN-TEST-001.

- **SIGN-TEST-002, closed**: the authority/receipt binding, pinned through a real second close
  cycle, with the `receipt` mutant.
- **SIGN-TEST-001, editor-cap and aggregate halves closed**: `local_policy` through a structurally
  decoded over-cap branch (`local-policy` mutant), and the probe gate through an **honest**
  branch (`probe-gate` mutant).
- **SIGN-TEST-001b, submitted**: a positive handoff by a non-owner member, and the per-device cap
  pinned in both directions (owner exempt, member charged), with three mutants
  (`author-is-owner`, `owner-charged`, `device-exempt`); short re-review outstanding. Residual and
  not part of the correction: no isolated mutant for the per-operation preflight or the framing
  probe, neither of which is the first refusal for any cheap input.
- **A product gap, for Agent 2**: the honest over-gate branch is valid local work that can never be
  handed off automatically. For a non-owner author the binding limit is the 1 MiB per-device cap,
  a quarter of the epoch budget; P2's classifier reports such a branch as Transferable; and no 8.3
  rail bounds signed size. The handoff refuses it safely before signing.

## A verification-scope failure of mine, recorded because the fix alone would hide it

**`origin/gate4-agent1-runtime` was red for six of my commits and I did not notice.**

Agent 2's slice 5 (`8ceb23bd`) made a Studio overlay record v3 rather than v2, because minting a
branch where `active` is `None` is now a generation event. That legitimately invalidated an
assertion in **my** test,
`studio_overlay_handoff_rollover_floor_rejects_forgotten_retry_after_rewind`, which hand-decodes
the extension to find the completed-block offset:

```
metadata.rs:84  assertion `left == right` failed:  left: 3,  right: 2
```

`8ceb23bd` sits below `216a03c9`, so from that point on the pushed branch failed that test. I then
pushed `216a03c9`, `d794349d`, `8287e9cf`, `e60c60b5`, `b5c0f4f4` and `e4d21148` on top of it,
each time reporting the gates as passing.

**Why the reports were not false but were worthless.** Each run really did pass: I ran a filter of
the tests I had just touched - `performance::c3`, `performance::c1_`, `store::measure::tests`, the
bare-guard cursor test, later `flow_s`. Every one of those passed every time. The filter simply
never contained the failing test, because I had not edited that file in weeks. **A verification
scope drawn around the diff cannot see a regression another agent's commit causes in a file I own
but did not touch** - and on a shared branch that is the most likely kind of breakage, not the
least.

Agent 2 found it and fixed it in `66df230c`, in my file, and flagged it in their commit message
rather than editing quietly. Their diagnosis and their fix are both right - I read the diff and
confirmed the failure mode matches it exactly.

**The correction is to the scope, not to the test.** Verification now runs the whole owned surface
- the `epoch_studio` and `epoch_recovery` test trees - not a filter over the diff. The filter
stays useful for fast iteration inside a change; it is not evidence for a push.

Two things this does not excuse. The branch's redness was discoverable at any point by running
more than I ran. And "gates pass" in six commit messages was a claim about a filter while reading
as a claim about the branch; where those messages said clippy and tests passed, they meant the
named subset, and that qualification belonged in them.

## Executed checks

| Command | Result |
|---|---|
| `cargo test -p catcoms-app --lib -- performance::c3 performance::c1_ store::measure::tests a_parked_cursor_refuses_after_a_bare flow_s_stage_profile_smoke rollover` at `e4d21148` | **15 passed, 1 failed.** The failure is the rollover-floor test above, caused by Agent 2's published slice 5 and fixed by their unpushed `66df230c`. This is the run that should have been happening all along. |
| `git log --oneline`, `git status --short` | Revision 4 starts from `7efc9c2` (Agent 3's design), which contains revision 3 at `1bcb1bc`. Agent 1's two files were unmodified by `7efc9c2`; Agent 2's documents are present untracked. Revision 4 is uncommitted. |
| `git fetch origin Create-suite-2` | Revision 1 and 2 passes both found `origin/Create-suite-2` equal to the local head. |
| `grep` over `docs/GATE4-AGENT-3-DESIGN.md` sections 11 and 13.1 | Agent 3 accepts I-4, names its three affected writers, asks that `save_studio_source_checked`'s `handoff` parameter shape be preserved, and confirms no competing source writer or second pool. |
| `gh pr view 26 --json state,reviews,title,headRefName` | Open, `Create-suite-2`, `reviews: []`. Verdicts on this design were delivered outside GitHub's submitted-review endpoint, so its emptiness is not evidence that no review happened; `e65bfd8` is called unreviewed because HANDOVER and the core note agree, not because of that endpoint. |

Through the four design passes no Cargo command was executed. Implementation execution is recorded
under "Executed evidence for C-1" above; every run there used `-j 1` with the per-package test
debug override and no concurrent Cargo work, as the shared machine requires.

**This paragraph is superseded.** It was written when 13.7 was the only measurement with any
numbers. Six of the eight now have something against them and **none is complete** - see "What
that leaves genuinely unmeasured" above for the per-item state in the four categories this ledger
now distinguishes. Performance numbers quoted elsewhere in this ledger still come from the
existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations.

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

## Accepted sequencing for the remaining work

Agreed with the reviewer, whose two adjustments to my proposed order are adopted: build isolation
and the reference oracle come **before** the next measurement, not after it.

| Priority | Action | Done when |
|---|---|---|
| **First** | Send the SHA-pinned Agent 2 interface **confirmation** (not a stale checklist), the Agent 3 coordination request, and Agent 4's design 15 edit list | each recipient has the real dependency SHAs, the interface contract, and the specific decision being asked of them |
| **Alongside** | Isolated verification workspace, and the reference-result oracle | oracle **done** (see above, mutation-verified); isolation still outstanding |
| **Then** | C-1's bounded before/after measurement, with the shared pure decoder timed separately from end-to-end inventory work so setup and I/O cannot conceal the difference | same valid encoded fixtures, structural and full paths separated, shared metadata outputs verified, raw repeated-run data kept |
| **Then** | Counterbalanced fixed-corpus 13.7 runs, and the frame-versus-CID factorial cases | actual shapes and outputs verified; protocol, order and run identity retained |
| **After coordination and storage acceptance** | C-3 runtime adoption, then Flow R | runtime ownership, cancellation, retained-input accounting and bounded progress **demonstrated**, not inferred from storage tests |

### Provenance of the review-fix verification, by blob identity

A review noted that "isolated worktree at `e60d8315` plus copied changes" is not the same source
identity as the integrated head, and asked for the tested files to be compared against their
committed blobs. Done, and they match exactly:

| file | tested blob | committed at `2b862b8a` |
|---|---|---|
| `inventory/tests/performance.rs` | `fa7704df6ce54775cf959c89c664220108a5ec87` | **same** |
| `.../overlay/handoff/performance.rs` | `f00480a3cb7db614dfcc7aad652f8253be46801c` | **same** |

So the 14 tests and clippy run did execute the exact code now under review, for those two files.
What that still does **not** cover is integration with the intervening Agent-2 commits: the
worktree's base was `e60d8315`, five commits behind, and the main tree could not be used because
Agent 2's uncommitted `disposal.rs` did not compile. An exact-head run remains outstanding, and
the claim here is precisely "these blobs passed", not "the head passed".

### The isolated verification workspace, established

| Property | Value |
|---|---|
| Worktree | `M:/catcoms-verify-137`, detached, **outside the repo and therefore outside its `target/`** |
| Pinned commit | `d30d1b5e31d3ce6f2c59111c160d01ab2e13b85b` |
| Worktree cleanliness | `git status --porcelain` empty at creation |
| Build output | `CARGO_TARGET_DIR=M:/catcoms-verify-137-target`, separate from the shared `target/` |
| Redirected intermediates | none - no repo-level `.cargo/config.toml`, and `CARGO_TARGET_DIR` was previously unset |
| Toolchain | pinned by `rust-toolchain.toml` to 1.89.0; `rustc 1.89.0 (29483883e 2025-08-04)`, `cargo 1.89.0 (c24e10642 2025-06-23)` |
| Disk | M: had 402 GB free, so the isolated output does not compete for space |

The worktree is deliberately **not** under `.claude/worktrees/` either, since that path sits inside
the repo. Executable path and hash are recorded with each run, so a result can be traced to the
artifact that produced it without requiring recompilation as ritual.

**First run from the isolated workspace.** Debug lib test, built in 4m08s with no contention:

| Property | Value |
|---|---|
| Executable | `M:/catcoms-verify-137-target/debug/deps/catcoms_app-92a5297ae14f0f5d.exe` |
| SHA-256 | `E752AE02915C27722965AC9B1BD0264B165B6E89DDB1B14DA611B93119BC9304` |
| Size | 62 979 072 bytes |
| Worktree at run time | clean |
| Result | the five `performance::c3` structural tests **5 passed, 0 failed** in 85.94 s |

That is the first result on this ledger whose artifact can be traced to a pinned source SHA. It
establishes that the structural checks, including the new reference oracle, pass from a known build
of a known commit rather than from whatever the shared target directory happened to hold.

### The isolated release profile: the first replication on this ledger

| Property | Value |
|---|---|
| Executable | `M:/catcoms-verify-137-target/release/deps/catcoms_app-5a0650d663014b39.exe` |
| SHA-256 | `B7B9A2131756C8577A30633582F9D04DE7A23F67B568C436491C9D9188E539E3` |
| Run | 29 cases, 8 trials, interleaved, 380.20 s, no competing build |
| Reference results | **verified on every reference-mode trial** by the new oracle |

Against the contended run of the same source, the same cases:

| case | contended | isolated | difference |
|---|---|---|---|
| Studio titles, 24 ops (3.70 MB) | 23 890 us | **23 843 us** | 0.2% |
| Studio frames, 128 (130 KB), accounting | 252 703 us | **238 734 us** | 5.5% |
| Studio frames, 128, reference-collecting | 248 328 us | **239 968 us** | 3.4% |
| Studio frames, 16 (16.6 KB) | 7 296 us | **6 750 us** | 7.5% |
| Registry, 24 ops (3.86 MB) | 19 140 us | **16 968 us** | 11% |
| Recovery, 4.19 MB | 11 031 us | **10 421 us** | 5.5% |

**Two things this settles and one it does not.**

It **replicates the frame finding**: a 130 KB frame-bearing record's validation was 10.0x a
3.70 MB title-only record's, at a 28th of the bytes - 10.0x isolated against 10.6x contended. And
the endpoint slope between 16 and 128 frames comes out at `n^1.71` isolated against `n^1.70`
contended. Those are the first figures here to survive an environment change, which is a
materially better footing than one run.

It **also replicates reference mode's similarity to accounting for Studio**, now with the CID sets
checked on every trial rather than discarded: 239 968 against 238 734 us at 128 verified CIDs, a
0.5% difference inside their own spreads. Installation of that verified 128-CID merge was
`0/0/0(z8)` - every one of the eight samples below the clock's resolution.

What it does **not** settle: the contended and isolated figures agree to within 11%, which means
**contention was not distorting these particular numbers materially**. So the earlier 2.5x
block-versus-interleaved shift was a *protocol* effect, not a contention effect - the two
explanations were being carried together and only one of them is supported here. Contention broke
the *build*, repeatedly and unmistakably; it did not visibly move these timings. That distinction
is now on record rather than conflated.

Still absent, and still the reason no constant here is calibration: a counterbalanced
blocked-versus-interleaved comparison on this fixed corpus, and a factorial case separating frame
count from CID count.

**Build isolation is acceptance work, not housekeeping.** What this ledger records about the
contention is symptoms plus a proposed explanation - unresolved exports alongside concurrent
builds do **not** by themselves prove shared artifacts caused every failure. They justify
isolating before drawing further conclusions. The next evidence run needs: a pinned worktree
**outside every build-output directory**; a dedicated build-output location for it (checking for
any separately configured intermediate directory too, since isolating final executables alone is
insufficient when intermediates are redirected); and no competing benchmark or compilation load,
because unique output directories remove artifact interference but not CPU, memory or I/O
contention during timing. Record the source SHA, worktree cleanliness, toolchain, profile, relevant
configuration, executable path and executable hash. A legitimate cache hit is not invalid evidence
- the requirement is reliable source-to-artifact provenance, not recompilation as ritual.

**Do not run broad cleanup against `target/` while it contains agent worktrees.** Two live ones
are inside it. They must be relocated deliberately before that directory is treated as disposable.

The ambient-gate and formatting passes recorded at this checkpoint are **reported passing at this
checkpoint**; they do not convert the unreproduced build attempts into a clean exact-head
acceptance run.

**Standing summary, corrected.** Not "storage and correctness work is essentially done", which
understates what remains:

> The core storage mechanisms are largely implemented. Remaining acceptance includes storage-test
> closure provenance, cross-agent integration, runtime custody and cancellation, and measured
> behaviour.

Those runtime properties are correctness work, not wiring - and the quarter-second validation case
makes that more important, not less.

## Next actions

1. **C-3**, now the largest remaining piece and unblocked: `EpochStorageCursor` as an owned type
   replacing the exclusive-borrow scanner, invalidation checked both before resuming expensive
   work and before issuing the inventory, the time budget with classify-before-invoking
   detachment, the single parked body carrying its job's original `OverlayOwnership` and
   rechecked against cursor identity, mount, record id and `inventory_generation`, and
   `MAX_INVENTORY_RESTARTS` with backoff rather than a fallback to an unbounded scan. N17 and
   N18 are its evidence.

   **Not blocked on measurement 13.7.** The largest-step figures calibrate the classifier, and
   9.2 already states the rule for their absence: default to detaching. So C-3 starts in its
   conservative mode and 13.7 tunes it later.

   Section 15 asks for a **coordinated verdict** on the borrow-to-cursor change, because it is a
   semantic consistency change shared with Agent 3 and every Studio write path, not a mechanical
   signature change. Get that before converting call sites, not after.
   **Status: the storage half is done.** See the C-3 section above. What remains is the runtime
   adoption of the cursor at six call sites, which is its own checkpoint.

   **Status, 2026-10-09:**
   - **Step 2** (replay's manual move) **lands** in the batch with F1 and F4, now that Agent 2
     has freed the shared receiver files.
   - **The classifier and refused-result memo** (C-3 runtime 14, parts A and B) are built at
     `6a79e6f8`.
   - **Step 3, and Flow R after it, need more than the classifier.** Design 9.1 is built
     (`17dd54fc`), and the commit phase is measured on its own (2026-10-09, release, shared
     host). The route's revision 2 is C-3 runtime 15.8, and its design review is 15.9: no
     blocker, one high. **The order of work is 15.9's:**
     1. H5's source-growing terms computed once. **The source axis is measured** (C-3 runtime
        15.10, 2026-10-09). The commit grows about 0.06 ms per base frame: 86 ms at 998 frames
        and one operation, and 147 ms with a full branch, past the visit. The repeated
        projections are about half of the former. Next:
        - item 0's first half (compute each once in H5);
        - re-measure;
        - its second half (carry H2's and H4's projections, a design change needing review);
        - attribute the branch-length growth;
     … and the remaining items as listed;
     2. the indexed, pruned memo (M2);
     3. the all-family memo with warms at all eight writers, behind a forced-warm token;
     4. the restart progress rule, with per-key credit and a ceiling;
     5. the rule that divides a visit between scan and commit;
     6. the traversal measurements;
     7. the H5 write-every-turn test;
     8. the touched-path cursor decision, which moves into step 3 if item 1 leaves no margined
        share.
   - **Steps 4 and 5** still need section 7's measurements.
2. Then **Flow R**, which needs no media and is independent. It was deliberately sequenced after
   this boundary so it is not built on the unbounded inventory path and then split again.
3. Produce design 13's eight measurements as each item lands; C-1's before-and-after is cheap,
   since the opt-in profile already exists. **13.7 (updated 2026-10-08):** Recovery, Registry
   and Studio were measured earlier. Intents, OwnerReceipts and DraftArchive were measured on
   2026-10-08 (see "The uncached families at their ceilings"), and the classifier proposal that
   follows from them is section 14 of `GATE4-AGENT-1-C3-RUNTIME.md`, awaiting design review.
   Still outstanding:
   - OwnerReceipts at its 27 KiB cap;
   - DraftArchive in reference mode;
   - Recovery above 4 MiB;
   - realistic full scans;
   - the restart rate under concurrent writes.

   The remaining seven measurements have no numbers.
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
8. **Design 18.3's implementation review is back** (2026-10-09): a bounded PASS WITH FINDINGS, no
   blocker or high. See its section above. F1 and F4 land in the batch with C-3 step 2.
   **Still open: F2,** the actor-level N31 plus a mutation entry for `handoff_priority`, as its
   own push. Native Save must not register until F2 is closed, besides Agent 2's P5.

   **Also with Agent 2:** the shared overlay-lifecycle harness exceeds its 60-minute CI job on
   every push. Agent 2 will shard it by index (`--shard K/N`) after this batch lands.
9. **Reconcile the landed archive code with its review status.** This item was stale and is
   rewritten. At this head `epoch_draft_archive.rs` already contains
   `write_studio_draft_archive_with_io`, `release_studio_draft_archive_with_io` and
   `inventory_references`; the writer and release take `WriteHooks`, the mutation paths acquire
   `EpochMutation`, release checks the expected archive identity before unlinking, and the
   collector decodes the archive and obtains its blob references. So "when their archive writer
   lands, replace the fail-closed reference arm with their collector" no longer describes
   reality. **Presence is not a review PASS** - what is needed is the actual verdict status for
   that code, not a checklist written before it existed.

   **And `write_draft_archive_for_test` must not simply be deleted.** Its own comment says every
   *valid* archive in the tests goes through the production writer and that this helper exists
   only where the payload is deliberately malformed or misplaced. Removing it would remove the
   fault injection that proves the reader rejects those states. Valid-archive setup should use
   the production writer; malformed-record injection keeps this helper.

   Remaining archive work, accurately: **reconcile the landed writer/release/collector with their
   review status, retain malformed-record injection, and assert `inventory_generation` rotation
   on the two draft-archive *write* shapes** - the exact-retry sync and the fresh replacement.
   Release is already asserted. **No DraftArchive cursor test is needed**; see "The DraftArchive
   N17 gap was mischaracterised" for why the cursor refusal is family-agnostic.

   There is also a stale comment in that file, at `epoch_draft_archive.rs:255`, saying
   `release_studio_draft_archive_with_io` "does not exist yet" - 68 lines above its
   implementation. It is Agent 2's file, so it goes in the handover rather than being edited here.
10. **Send Agent 2 a requirement-3 contract confirmation**, SHA-pinned. Still not sent, but the
    framing in the earlier version of this item was wrong: it read as "you are still writing
    against the old API", and the landed archive code already consumes the new `WriteHooks` form.
    Telling them otherwise would be inaccurate and unhelpful.

    What to send instead is the contract to preserve plus a request to confirm the remaining
    callers: the eight per-transaction tag enums are gone, replaced by one store-wide `WriteTag`,
    and `WriteTag::Archive` is what the draft-archive record carries. Transactions accept no
    `writer`/`sync`/`unlink` closures at all: a writer takes `WriteStep` and
    `&mut WriteHooks<'_>` and performs its own operations through `EpochMutation`. "Flush only,
    never replace" is `WriteStep::flush_only`, and the leaf must call `permit_replacement()`
    before its replacement branch for that to mean anything. Include the stale
    `epoch_draft_archive.rs:255` comment.

    **Their status document also needs reconciling**, and this is theirs to do: its top-level
    table still says no production code or tests have been written, which the archive code
    contradicts. Correcting stale implementation rows is **not** permission to promote P5 - that
    stays false until the required implementation reviews are complete.
