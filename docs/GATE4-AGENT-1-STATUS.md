# Gate 4 Agent 1 status: local Save and automatic handoff runtime

Owner: Agent 1 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-1-local-save-and-automatic-handoff-runtime)).
Proposal: [GATE4-AGENT-1-DESIGN](GATE4-AGENT-1-DESIGN.md), currently revision 2.
Review preamble: 1. Current entries override older ones.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Runtime design proposal, revision 1 | `a052f78b62a549702686a8741932f1d2f8c98773` | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | design, docs only | **REQUEST CHANGES**: AG1-001 to AG1-005 and AG1-TEST-001 |
| 2026-09-15 | Design revision 2, all findings answered | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | uncommitted working tree | design, docs only | re-review not yet requested |

Working checkout: `M:\Git (local)\CatComs`, branch `Create-suite-2`. Implementation must move to a
separate branch or worktree before any code change; both documentation passes deliberately share
the checkout and touch only these two files.

## Revision-1 review outcome

Reviewer verdict: REQUEST CHANGES, source and design inspection only, no Cargo, tests, mutations
or measurements executed. The reviewer confirmed the overall direction (detached reconstruction,
an explicit live-authority step, finite signing slices and unchanged durability semantics) and
located the problems in the surrounding protocol and ownership boundaries. No prior closure was
reopened, and `e65bfd8` was inspected as an interface, not granted a PASS.

| Finding | Severity | Subject | Status in revision 2 |
|---|---|---|---|
| AG1-001 | P2 | Flow S gated a saved retry behind fresh Closing eligibility, and the native request dropped the original basis, so a delayed retry after acknowledgement rollover could be accepted as new work | **Answered.** Acknowledgement is classified under custody on the structural decode before any basis minting; the native request carries its original basis; an unmatched request returns stale; the retry floor remains an independent second fence; the handed-off wording no longer promises current source or ledger presence. Design 6.2, 6.3, 5.6, 5.7. |
| AG1-002 | P2 | H7 retained a five-family scan, a `checked_studio_source` restore and a second restore inside the resolver, all under one uninterrupted lease | **Answered.** No graph restore remains on the runtime commit path: installed-source facts are reused behind exact wrapper stamps, and the persisted source is proved by digest equality against the retained candidate, with `resolve_studio_handoff_with_io` taking that verified input so there is one algorithm. Inventory gets C-3, a resumable generation-guarded cursor with a bounded restart count and a hold rather than an unbounded fallback. Design 9, L6. |
| AG1-003 | P2 | The S0 pixel pre-hold does not survive the detached interval: `Protection::unknown` drops it and a completed scan reinstalls from durable state, after which the new CID can be deleted while every S3 stamp check still passes | **Answered.** C-4 adds bounded job-owned transient holds that `unknown()` and `install()` do not touch and a complete scan may not subtract, owned through every detached stage, plus an S3 possession revalidation that refuses before the intent barrier. Design 8, R7. |
| AG1-004 | P2 | A durably unresolvable Prepared record became a permanent export and copy prohibition through the interface Agent 2 is required to use | **Answered.** The hold is split into a transient live-operation hold and a durable transfer hold; read-only export and inspection of a typed-readable Prepared branch are permitted without resolving it; destructive disposition still refuses. Design 12.1. |
| AG1-005 | P3 | "One tracked job" does not imply one live reservation, because cancellation clears the tracked job while the worker still owns its permit; native Save had no relationship to the actor's admission record; H0 read and decrypted a record with no reservation | **Answered.** An explicit `OverlayAdmission` token held by the job, every detached worker including a cancelled one, and every retained result, with drained weak handles blocking admission; native Save participates through `StudioReceiver::control`; the permit and token are reserved before the first bounded read and H0 is folded into H1. Design 7, invariant I-2. |
| AG1-TEST-001 | P3 | M1, M6, M12 and M9 were masked by other required checks | **Answered.** Every mutation observes at the boundary its guard protects; intentionally redundant guards are labelled and assert early refusal and resource consumption; M12's fixture now removes the matching signed close from the owner journal so wrapper stamps cannot mask it; M9 asserts absence from actual replay evidence. Design 14.2. |

### Audit claims corrected in revision 2

| Claim | Correction |
|---|---|
| R1: ordinary Studio reads always reconstruct a retained branch | Conditional. The intent decoder is reached from source loading and warm authentication only when the source wrapper carries the required-metadata link byte; a branch accepted without a subsequent source write does not make ordinary reads decode it, and a Completed-only record has no branch to replay. The unconditional cost is `checked_epoch_replay_state` and the uncached Intents arm of the five-family scan. |
| R1: "about four" reconstructions in the synchronous commit | Withdrawn. The count is path dependent and can be higher; no timing claim follows from it. |
| C-1 moved call sites are "projection-free" | Corrected to branch-replay-free. The reference path deliberately retains `base_blob_cids`, which calls `base.graph()` and performs typed seed work; the source-write and completion paths still need their separate signed-source evidence. |
| C-1 trust argument from AEAD sealing | Replaced by invariant I-1, a requirement on the writers. AEAD authentication proves origin and integrity, not typed admission, and not every `write_prepared_intents` call follows a fresh `append`. |
| R5: the digest comparison is "strictly stronger" | Withdrawn. The existing decoder already requires canonical re-encoding equality, so the old comparison already covered the complete encoded contents. The justification is cost, not strength; no normalization counterexample is claimed. |
| R3 | Adds that `close_for` is historical evidence, not fresh owner authority, and that the observed-tenure verification inside `prepare_settlement` remains necessary. |
| R4 | Labelled selection hardening; it is not evidence that the current ordinary Apply guard permits an overlay bypass. `NoEvidence` is preserved. |
| Reference protection described as one mechanism | R9 separates `check_handoff_references` (candidate plus pending coverage, in memory, at the commit) from HANDOFF-002's authenticated inventory dependency. Both survive and are tested separately. |

### Reviewer answers adopted as binding

Per-visit wrapper reauthentication, conditional on slice exclusivity (tested by N9); the
structural and full decode split with full validation as the default; the existing narrow
authority interface with no new constructor, subject to one flagged deviation in design 6.1; both
a count limit and a time budget calibrated against admitted byte and work bounds, with the
one-operation overrun acknowledged and the receive cadence reported separately from maximum
continuous custody; restart on MLS change under an explicit eventual-stability condition with a
required mid-signing test and a manual escape.

## Dependencies

| Dependency | Current state | Effect if unmet |
|---|---|---|
| Core signing split `e65bfd8` | **Unreviewed.** The revision-1 reviewer inspected its interfaces without granting it a PASS; PR #26 has no submitted review and HANDOVER records no returned CI run for that checkpoint. | Design 6.1 and 9 are bound to `handoff_authority`, `prepare_handoff_detached`, `sign_next` and `finish`. Contingency in design 16. |
| Agent 2 manual overlay lifecycle | Not started. | Native Save stays unregistered and absent from FLIPNOTE-UI-HOOKS. Prerequisites P1 to P5, design 12.3. |
| Agent 2 live-tenure contract | Not started. | The runtime binds `tenure` as an opaque `u64` and needs "equal value implies the same continuous tenure" (P4). |
| Agent 3 signed repair | Not started. | Must respect `studio_overlay_live_hold`, the Prepared transfer hold and the existing fences rather than adding a competing writer or pool; C-3 changes a scan seam it also uses. |
| Agent 4 shared enum, seam, registration and workflow edits | Not started. | Design 15 lists every central edit, now including C-3 and C-4, which touch shared files adjacent to HANDOFF-002's reviewed scan. |

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-1-DESIGN.md) section 5. Summary:

- Core: `StudioOverlay::decode_vault_structural`, `StudioOverlayState::decode_vault_structural`
  (C-1). No other core change; the signing split is consumed as implemented.
- Store: `capture_studio_overlay`, `studio_overlay_is_current`, `studio_overlay_structural`,
  `commit_studio_overlay_state`, `studio_closing_basis`, `hold_creative_transient` (C-4),
  `begin_epoch_storage_scan` / `step_epoch_storage_scan` / `finish_epoch_storage_scan` (C-3);
  `StudioOverlayStamp`, `StudioOverlayCapture`, `StudioOverlayWork`, `StudioOverlayPlan`,
  `StudioOverlayPlanned`, `StudioOverlayHold`, `CreativeHold`, `StudioOverlayCapture::plan`.
- App runtime: `OverlayAdmission`, `OverlayRuntime`, `OverlayJob`, `OverlayStage`,
  `OverlayOwnership`, `StudioBackgroundJob::{OverlayPlan, OverlayAssemble}`,
  `StudioBackgroundResult::{OverlayPlanned, OverlayAssembled, OverlayCancelled}`,
  `studio_overlay_live_hold`, `studio_overlay_transfer_hold`.
- Control: `StudioControlAction::{BeginOverlaySave, PrepareOverlaySave, FinishOverlaySave}`,
  `StudioControlResponse::{OverlaySaveBasis, OverlayAcknowledged, OverlaySavePreparation,
  OverlaySaved}`, `StudioOverlayAcknowledgement::{LocalDraft, Handoff}`.
- Settlement: `StudioSettlementState::{LocalDraftRetained, LocalDraftHandedOff}`.
- Native (designed, **not registered**): `studio_overlay_begin`, `studio_overlay_save`, plus one
  new `transferState: "completed"` value on the existing `studio_overlay_read` result.

**None of these exist yet.** A proposed name is not an implementation; no other agent may consume
one before it is implemented and this table says so.

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
7. One shared four-slot `registry_catchup::preparation_pool()`, plus invariant I-2: one live
   per-actor overlay admission, owned through cancelled workers and retained results.
8. Invariant I-1: first acceptance fully validates the branch, and every later writer preserves the
   already-checked identity and evidence rather than re-deriving it.
9. Device and MLS secrets never leave the actor; a detached worker owns authenticated plaintext,
   public context, its permit, its admission token and any transient reference hold only.
10. Completed publication uses ordinary tail and page service; the initial Save window stays at two
    packets.
11. Every refusal, hold, cancellation and capacity failure retains the complete branch, and a
    transient reference hold is released only by its actual owner.

## Touched files

Both passes: `docs/GATE4-AGENT-1-DESIGN.md`, `docs/GATE4-AGENT-1-STATUS.md`. No production code,
no test, no shared contract document and no workflow was changed. Planned files are in design 5
and 15.

## Executed checks

| Command | Result |
|---|---|
| `git fetch origin Create-suite-2`, `git rev-parse HEAD origin/Create-suite-2` | Revision 1: both `a052f78b62a549702686a8741932f1d2f8c98773`. Revision 2: both `ac12822f04337b3e388618f81ce4a4b29d1e9b87`, the reviewed head; revision 2 is uncommitted in the working tree. |
| `gh pr view 26 --json state,reviews,title,headRefName` | Open, `Create-suite-2`, `reviews: []`. The revision-1 verdict was delivered outside GitHub's submitted-review endpoint; the endpoint's emptiness is not evidence that no review happened, and it remains the basis for calling `e65bfd8` unreviewed only because HANDOVER and the core note agree. |

**No Cargo command, test, mutation, workflow or benchmark has been executed in either pass, and no
new measurement exists.** Quoted numbers come from the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations. Design 13 lists the eight
measurements the implementation must produce, now including the largest admitted individual
operation and roster shape, C-3's per-slice custody and restart rate, and the receive cadence
reported separately from maximum continuous custody.

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
  keep `(basis, nonce, body)` byte-stable across retries. It is an identifier, not authority; a new
  acceptance still derives and verifies eligibility from durable state.
- There is no timestamp field. The actor supplies it, and a retry with a fresh actor timestamp is
  still an exact retry.
- A request that matches no retained entry and whose `basis` is not the currently eligible one
  returns a stale error. It never becomes new work.
- `acknowledged-handoff` records that this exact request was previously transferred into the named
  local destination epoch. It is not delivery, receipt or settlement, and after legitimate
  retirement it does not assert that those operations are still in the current signed source or
  the pending ledger.
- Neither result carries `content`. Refresh with `studio_overlay_read`, which owns the detached
  reconstruction and the 32 MiB conversion bound.
- Save is available only for a Closing document whose exact expected checkpoint can be constructed
  from the actual source, its saved signed close and the observed owner tenure. Fault, an
  unavailable seed, unknown tenure and an unconfirmed preview refuse and keep unsaved editor work
  visible; a refusal is never a durable save.
- Automatic transfer happens in the background once a verified eligible successor is installed. It
  sends no packets of its own; peers receive the operations through ordinary paging. A large
  branch completes over several background turns while the app stays responsive.
- `studio_overlay_read` gains `transferState: "completed"`. `kind: "absent"` keeps its meaning.
- Both commands share the latest-view request fence with ordinary reads; run them sequentially for
  the same target. Capacity exhaustion returns a retryable error; one overlay operation is live per
  server at a time, and the shared four preparation slots stay owned until actual worker or result
  destruction, including after cancellation.

## Next actions

1. Commit revision 2 and send the re-review request in design 18 with the head SHA filled in.
2. Confirm with Agent 2 the prerequisites P1 to P5 and the two-hold contract in design 12.1;
   confirm with Agent 3 that repair uses the existing fences and that C-3's cursor refactor is
   acceptable to their scan call sites; give Agent 4 the central edit list in design 15, which now
   includes C-3 and C-4.
3. Do not start implementation on this checkout. Hold C-1, C-3 and C-4 until the design verdict
   returns: all three touch shared files adjacent to HANDOFF-002's reviewed scan.
