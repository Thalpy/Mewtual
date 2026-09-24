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
| N17 with the real writers | **Five of the six families** - Recovery, OwnerReceipts, Intents, Registry, Studio - plus the cleanup unlink, which is an operation class and not the sixth family. **DraftArchive is not covered.** Also the unchanged exact-retry flush and a failed write; Studio carries the negative half (a budget mint must not invalidate) |
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

Also not established here: the parked plaintext's residency is not charged to section 13.4's
retained-input sum, and no runtime variant yet demonstrates that a cancelled waiter does not
release a still-running validation's reservation. Both are activation requirements.

### The C-3 storage review: C3-001, C3-002, C3-003 closed; C3-TEST-001 closed here

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

## Design 13.7, partially delivered: the first measurement in this design

Until now every measurement obligation in design 13 was outstanding and the ledger said so. This
is the first one with numbers behind it. It is **partial**, and the boundaries are stated below
rather than left to be discovered.

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

### Registry and Studio measured, and they invert the Recovery conclusion

These are the families whose expensive typed reconstruction motivated C-3, so they are the ones
whose numbers matter. Release, 8 trials, fresh cache, **all 23 cases interleaved**, reported as
`min/median/max` microseconds per record per trial.

| family | ops | authenticated bytes | read-and-park | validation | fraction |
|---|---|---|---|---|---|
| Registry | 2 | 321 650 | 0/1 000/1 000 | 1 218/**1 546**/1 703 | 60% |
| Registry | 8 | 1 285 460 | 2 000/**3 000**/5 000 | 4 703/**5 171**/5 734 | 63% |
| Registry | 24 | 3 855 634 | 6 000/**7 000**/7 000 | 14 125/**15 343**/16 343 | 68% |
| Studio | 3 | 322 505 | 0/1 000/2 000 | 1 671/**1 890**/2 515 | 65% |
| Studio | 12 | 1 768 854 | 2 000/**4 000**/5 000 | 8 296/**9 328**/9 875 | 69% |
| Studio | 32 | 4 179 459 | 6 000/**8 000**/10 000 | 21 750/**24 609**/26 640 | 75% |

**Interleaving changed the Registry and Studio absolutes by about 2.5x, and the block-ordered
figures previously recorded here are withdrawn.** Registry at 24 ops read 39 056 us block-ordered
and 15 343 us interleaved; Studio at 32 ops, 59 099 against 24 609. Recovery moved far less and
stayed inside its own spread. Nothing about the code changed between them - only the order cases
were built and run in. The earlier numbers measured each fixture immediately after building it;
these measure every case from the same steady state.

That is a 2.4x methodological effect on the design's headline family, from ordering alone. It is
the clearest possible argument for the discipline, and it means **no absolute figure produced by
the block-ordered harness should be quoted.**

**A specific claim of mine that this corrects.** I reported Studio's validation as "five times
Recovery's for the same bytes". Within one interleaved run it is **2.5x**: Studio at 32 ops is
24 609 us against Recovery's 9 953 us at 4.19 MB. The direction was right and the multiple was
inflated by comparing two differently-ordered measurements.

**Validation is linear in operation count**, which is the structural axis and not a byte axis:

- Registry: 773, 646, 639 us per operation at 2, 8 and 24.
- Studio: 630, 777, 769 us per operation at 3, 12 and 32.

Flat within about 5% above the smallest point in each family, across a 12x and 11x operation
range. A threshold for these families should be written against operation count; authenticated
size is a proxy only while the per-operation payload stays constant. The constants themselves
are **shape, not calibration** - they moved 2.5x under reordering.

**What survived the reordering, and is therefore worth believing:** validation exceeds
read-and-park on both families and the gap widens with operation count; per-operation cost is
flat; reference collection costs the same as accounting for both; Studio's validator is the
most expensive per byte. **What did not survive:** every absolute, and the size of the
Studio-versus-Recovery multiple.

### Read-and-park sits at the clock's resolution below about a megabyte

The spread makes visible what a mean concealed. `read_and_park` for the 322 KB Registry and
Studio records reads `0/1 000/1 000` - half the samples are zero. The block-ordered harness
reported a mean of 2 125 us for the same phase, which looked like a measurement and was an
artifact of averaging zeros, ones and twos.

So the fraction column is **not trustworthy for any row whose read-and-park median is at or
below one millisecond**, which is every sub-megabyte row in these tables. It is retained for the
larger rows, where both phases resolve.

This also produced a real defect, caught by the output: a phase with seven zero samples and one
1 ms sample passes a "did this resolve" check on its *sum* while its median is still zero, and
the fraction then printed **100% deferrable** - the same trap as the earlier `visit_ms == 0`
case, one level down. Small reference-scan rows hit it. The fraction is now suppressed unless
both medians are nonzero, and `c3_a_phase_median_of_zero_reports_no_fraction` pins it with a
positive control.

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

What survives it and what does not:

- **Within-run comparisons survive.** The Registry and Studio per-operation constants are flat
  across a 12x and 11x range *inside one run*, and the Studio-versus-Recovery ratio at equal
  bytes is a 5x gap measured in the same process. A 40% drift does not explain a 5x gap.
- **Absolute constants do not survive.** "1.65 ms per Registry operation" is a shape, not a
  calibration constant, and must not be used as one.
- **The Recovery reference-count curve is weaker than first stated.** Per-reference cost was
  7.0 / 3.1 / 3.8 / 4.4 us in run A and 11.0 / 4.8 / 6.5 / 8.1 us in run B at 1 / 16 / 128 /
  512. Both rise with count; neither is flat, and they disagree by up to 1.9x. The claim that
  cost tracks reference count "near-linearly" is retained only as a direction, not a rate.
- The Registry and Studio tables above are from **one run each**. They have not been repeated.

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
`check_case_structure`: every trial completed; a cleared cache parked the record in every trial
and reported no hits; a warm accounting scan of a cacheable family did report hits; a reference
scan reported none. The smoke tests additionally require that all three phase sample vectors
have the same length as the trial count, since a spread over ragged samples would silently
divide by the wrong number.

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

Three tests, following the existing `studio_source_profile_smoke` /
`profile_studio_source_operations` pattern. `c3_visit_profile_smoke` and
`c3_canonical_reference_fixture_collects_its_planted_cids` run in the ordinary suite on a
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

`scripts/check-no-ambient.sh` still exits 1, unchanged by this work and red since 2026-09-13.

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

**One of design 13's eight measurements now has numbers: 13.7, partially** - Recovery only,
accounting-only from 1 KiB to 4 MiB plus a canonical reference-collecting case. See "Design
13.7, partially delivered" above for what it found, what is extrapolation, what the three
corrected measurement boundaries were, and what is still uncovered. The other seven are
outstanding, including the C-1 before-and-after comparison that would quantify what the
structural decode actually saves: the code is in, the number is not. Performance numbers quoted
elsewhere in this ledger still come from the existing
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
2. Then **Flow R**, which needs no media and is independent. It was deliberately sequenced after
   this boundary so it is not built on the unbounded inventory path and then split again.
3. Produce design 13's eight measurements as each item lands; C-1's before-and-after is cheap,
   since the opt-in profile already exists. **13.7 is partially done** - Recovery, accounting
   and reference-collecting - and found that for these fixtures the read-and-park and validation
   phases are of comparable magnitude at megabyte scale, with `validation_fits` able to move
   only the latter. **Next, in this order: Registry and Studio** (the families whose expensive
   typed reconstruction motivated the design, both scan modes, real histories at their largest
   accepted shapes), then OwnerReceipts, Intents and DraftArchive, varying structure and not
   only encoded size; then realistic full scans and the restart rate under concurrent writes.
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
8. Do not send the design 18.3 implementation review until the scope it names has real evidence. A
   partial branch is not a checkpoint.
9. When Agent 2's archive writer lands, delete `write_draft_archive_for_test`, replace the
   fail-closed reference arm with their collector, and add their two archive writers to the I-4
   audit as design 9.2 now records.
10. **Tell Agent 2 what requirement 3 changed under them**, which has not been sent. The eight
    per-transaction tag enums are gone, replaced by one store-wide `WriteTag`; `WriteTag::Archive`
    covers the draft archive record and is what their `release_studio_draft_archive_with_io`
    should carry. Transactions no longer accept `writer`/`sync`/`unlink` closures at all: a new
    writer takes `WriteStep` and `&mut WriteHooks<'_>` and performs its own operations through
    `EpochMutation`. A caller that needs "flush only, never replace" says so with
    `WriteStep::flush_only`, and the leaf must call `permit_replacement()` before its
    replacement branch for that to mean anything.
