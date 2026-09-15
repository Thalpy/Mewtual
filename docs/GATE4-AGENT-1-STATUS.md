# Gate 4 Agent 1 status: local Save and automatic handoff runtime

Owner: Agent 1 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-1-local-save-and-automatic-handoff-runtime)).
Proposal: [GATE4-AGENT-1-DESIGN](GATE4-AGENT-1-DESIGN.md), revision 4, **accepted (PASS) at
`25ce89bb705dfc228c7b32874788ebd062e6fcf4`, 2026-09-15, design boundary only**.
Review preamble: 1. Current entries override older ones.

**The design boundary is complete; nothing is implemented.** No production code, test, mutation,
workflow or measurement exists for this scope. The next checkpoint is a bounded implementation on a
separate branch or worktree, using the request in design 18.3.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `a052f78b62a549702686a8741932f1d2f8c98773` | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | design, docs only | **REQUEST CHANGES**: AG1-001 to AG1-005, AG1-TEST-001 |
| 2026-09-15 | Design revision 2 | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | design, docs only | **REQUEST CHANGES**: AG1-004 **closed**; residuals on the other five |
| 2026-09-15 | Design revision 3 | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | `1bcb1bca204d721b848b17c0835faf931ae930e3` | design, docs only | **REQUEST CHANGES**: AG1-001, AG1-002, AG1-003, AG1-005 **closed at the design boundary**; AG1-TEST-001 open, P3 |
| 2026-09-15 | Design revision 4, N31 correction | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | design, docs only | **PASS**: AG1-TEST-001 **closed**; bounded runtime design accepted, no new finding |
| 2026-09-15 | PASS recorded | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | uncommitted working tree | docs only | n/a; records the verdict and the next checkpoint's request |

Working checkout: `M:\Git (local)\CatComs`, branch `Create-suite-2`. **Other agents are working in
this same checkout**: Agent 3's design landed at `7efc9c2` and Agent 2's documents are present
untracked. All four Agent 1 passes touch only these two files, and any commit must be
pathspec-scoped to them. Implementation must move to a separate branch or worktree.

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

## Touched files

All three passes: `docs/GATE4-AGENT-1-DESIGN.md`, `docs/GATE4-AGENT-1-STATUS.md`. No production
code, no test, no shared contract document and no workflow has been changed. Planned files are in
design 5 and 15.

## Executed checks

| Command | Result |
|---|---|
| `git log --oneline`, `git status --short` | Revision 4 starts from `7efc9c2` (Agent 3's design), which contains revision 3 at `1bcb1bc`. Agent 1's two files were unmodified by `7efc9c2`; Agent 2's documents are present untracked. Revision 4 is uncommitted. |
| `git fetch origin Create-suite-2` | Revision 1 and 2 passes both found `origin/Create-suite-2` equal to the local head. |
| `grep` over `docs/GATE4-AGENT-3-DESIGN.md` sections 11 and 13.1 | Agent 3 accepts I-4, names its three affected writers, asks that `save_studio_source_checked`'s `handoff` parameter shape be preserved, and confirms no competing source writer or second pool. |
| `gh pr view 26 --json state,reviews,title,headRefName` | Open, `Create-suite-2`, `reviews: []`. Verdicts on this design were delivered outside GitHub's submitted-review endpoint, so its emptiness is not evidence that no review happened; `e65bfd8` is called unreviewed because HANDOVER and the core note agree, not because of that endpoint. |

**No Cargo command, test, mutation, workflow or benchmark has been executed in any pass, and no new
measurement exists.** Quoted numbers come from the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations. Design 13 lists the eight required
measurements, now including C-3's largest single-record step and escalation rate.

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

1. Commit this PASS record, pathspec-scoped to these two files. No further design review is pending.
2. Move to a separate branch or worktree before writing any code. This checkout is shared with
   Agents 2 and 3.
3. Sequence the implementation so the shared-seam changes land in reviewable order, each with its
   own line in the verdict rather than arriving as one commit:
   1. **I-4** (`inventory_generation`, `EpochMutation` guarded primitives) plus the writer-coverage
      audit and N17 with M20. Nothing else can land soundly first.
   2. **C-3** (owned cursor, time budget, classify-before-invoking parked-body escalation) on top of
      I-4, with N18 and M21. C-3 and I-4 stand or fall together.
   3. **C-1** (structural decode) and **C-4** (transient holds and the I-3 transfer), with N12, N13,
      N23, N24, M8, M13 to M15.
   4. The **runtime** itself: Flows H, S and R, admission, scheduling and the commit, with the
      remaining regressions and mutations.
4. Produce design 13's eight measurements as the implementation lands; the C-3 largest-step figures
   are a prerequisite for calibrating the classify-before-invoking threshold, and until they exist
   the threshold defaults to detaching.
5. Confirm with Agent 2 the prerequisites P1 to P5, the two-hold contract (design 12.1) and the copy
   requirements (design 12.2); give Agent 4 the central edit list in design 15. Agent 3's side of
   the I-4 coordination is already on record at `7efc9c2`, and their "unaffected if I-4 does not
   land" clause is an integration alternative, not an opt-out from a deployed cursor's discipline.
6. Keep native Save unregistered and out of FLIPNOTE-UI-HOOKS until Agent 2's manual lifecycle
   passes its own review and their status note says so.
