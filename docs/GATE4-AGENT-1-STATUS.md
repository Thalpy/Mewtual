# Gate 4 Agent 1 status: local Save and automatic handoff runtime

Owner: Agent 1 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-1-local-save-and-automatic-handoff-runtime)).
Proposal: [GATE4-AGENT-1-DESIGN](GATE4-AGENT-1-DESIGN.md). Review preamble: 1.
This file is the running record of seams, invariants, touched files, executed checks, review
findings and the proposed UI-hooks update. Current status entries override older statements.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Runtime design proposal | `a052f78b62a549702686a8741932f1d2f8c98773` | uncommitted working tree | design, docs only | not yet requested |

Working checkout: `M:\Git (local)\CatComs`, branch `Create-suite-2`, HEAD
`a052f78b62a549702686a8741932f1d2f8c98773`, verified equal to `origin/Create-suite-2` by
`git fetch origin Create-suite-2` at the start of this pass; the tree was clean.
`gh pr view 26` reports PR #26 open on `Thalpy/Mewtual` / `Create-suite-2` with an **empty
review list**. Implementation must move to a separate branch or worktree before any code change;
this documentation pass deliberately shares the checkout and touches only the two new files.

## Dependencies

| Dependency | Current state | Effect if unmet |
|---|---|---|
| Core signing split `e65bfd8` ([note](GATE4-HANDOFF-SIGNING-REVIEW.md)) | **Awaiting the user's adversarial review.** No review on PR #26; HANDOVER records that no CI run for `e65bfd8` was returned. | Design sections 5.2 and 6 are bound to `handoff_authority`, `prepare_handoff_detached`, `sign_next` and `finish`. Contingency in design section 14. |
| Agent 2 manual overlay lifecycle | Not started. | Native `studio_overlay_save` stays unregistered and absent from FLIPNOTE-UI-HOOKS. Prerequisites P1 to P5 in design section 12. |
| Agent 2 live-tenure contract for A -> B -> A | Not started. | The runtime binds `tenure` as an opaque `u64` and needs "equal value implies the same continuous tenure" (P4). |
| Agent 4 shared enum, registration and workflow edits | Not started. | Design section 16 lists every central edit the runtime needs. |
| Agent 3 signed repair | Not started. | Must respect `studio_overlay_runtime_hold` and the Prepared source-replacement fence rather than adding a competing writer or pool. |

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-1-DESIGN.md) section 5. Summary:

- Core (`catcoms-replication`): `StudioOverlay::decode_vault_structural`,
  `StudioOverlayState::decode_vault_structural`. No other core change; the signing split is
  consumed as implemented.
- Store (`catcoms-app`): `ServerStore::{capture_studio_overlay, studio_overlay_is_current,
  studio_closing_basis, commit_studio_overlay_state}`; `StudioOverlayStamp`,
  `StudioOverlayCapture`, `StudioOverlayRequest`, `StudioOverlayPlan`, `StudioOverlayPlanned`,
  `StudioOverlayHold`, `StudioOverlayCapture::plan`.
- App runtime: `OverlayRuntime`, `OverlayJob`, `OverlayStage`,
  `StudioBackgroundJob::{OverlayPlan, OverlayPrepare, OverlayAssemble}`,
  `StudioBackgroundResult::{OverlayPlanned, OverlayPrepared, OverlayAssembled, OverlayCancelled}`,
  `studio_overlay_runtime_hold`.
- Control: `StudioControlAction::{PrepareOverlaySave, FinishOverlaySave}`,
  `StudioControlResponse::{OverlaySavePreparation, OverlaySaved}`,
  `StudioOverlaySaveResult::{LocalDraft, HandedOff}`.
- Settlement: `StudioSettlementState::{LocalDraftRetained, LocalDraftHandedOff}`.
- Native (designed, **not registered**): `studio_overlay_save`, plus one new
  `transferState: "completed"` value on the existing `studio_overlay_read` result.

**None of these exist yet.** A proposed name is not an implementation; no other agent may consume
one before it is implemented and this table says so.

## Invariants this scope must not weaken

1. Prepared -> whole Source -> Completed, with actual evidence rechecked at every barrier.
   No durable signed prefix, no per-entry retirement, no bypass cache.
2. The complete original pending ledger survives handoff; only the overlay exclusion is removed.
3. HANDOFF-001 complete-target comparison before any completed acknowledgement, source lookup or
   sync reservation.
4. HANDOFF-002 complete reference traversal and the source-to-intent link dependency.
5. The Prepared source-replacement fence and the publication hold, including after restart.
6. First local acceptance derives from the actual Closing source, its matching saved signed close
   and independently observed tenure; ordinary failed Apply stays `NoEvidence`.
7. One shared four-slot `registry_catchup::preparation_pool()`; the original permit is owned from
   capture to final release; cancellation never refunds a live owner's slot.
8. Device and MLS secrets never leave the actor; a detached worker owns authenticated plaintext,
   public context and its permit only.
9. Completed publication uses ordinary tail and page service; the initial Save window stays at
   two packets.
10. Every refusal, hold, cancellation and capacity failure retains the complete branch.

## Audit observations recorded against the design base

These are properties of the accepted code, reported as required rather than as reopened
closures. Detail and exact call paths are in design section 3.

| Id | Observation |
|---|---|
| R1 | `StudioOverlay::decode_vault` ends with `out.read(ledger)`, so decoding a retained branch performs a full typed reconstruction. It runs on ordinary Studio reads of that document, on `check_studio_intent_link` (reached from `load_studio_epoch`, `checked_studio_source`, `capture_studio_source` and `studio_source_bytes_match`), on every `read_epoch_intent_record`, and on the five-family inventory scan, where the Intents family is not covered by `inventory_cache`. |
| R2 | `StudioOverlay::append` validates through `staged.read(ledger)`, so saving the Nth operation replays N operations under actor and vault custody. |
| R3 | Every non-retry local Save re-derives the Closing basis through `prepare_settlement` and `checkpoint_for_close`; required by the accepted recheck rule, bounded by the Closing source, currently measured only on a small fixture. |
| R4 | `studio_replay_evidence` filters `own` by author only; Active and Prepared overlay ids are not excluded explicitly. Nothing is replayed by accident at the design base because `choose` returns `NoEvidence`, but the explicit exclusion the handoff design requires is missing. |
| R5 | The commit proves "intents unchanged" by decoding and re-encoding rather than comparing the authenticated plaintext digest. |
| R6 | The Prepared fence on the rotation, adoption, shared-write and publication paths resolves synchronously under custody. |

## Touched files

This pass: `docs/GATE4-AGENT-1-DESIGN.md` (new), `docs/GATE4-AGENT-1-STATUS.md` (new).
No production code, no test, no shared contract document and no workflow was changed.
Planned files for the implementation checkpoint are listed in design sections 5 and 16.

## Executed checks

| Command | Result |
|---|---|
| `git fetch origin Create-suite-2`, `git rev-parse HEAD origin/Create-suite-2` | Both `a052f78b62a549702686a8741932f1d2f8c98773`; no later remote change. |
| `git status --short` | Clean at the start of the pass. |
| `gh pr view 26 --json state,reviews,title,headRefName` | Open, `Create-suite-2`, `reviews: []`. |

**No Cargo command, test, mutation, workflow or benchmark was executed in this pass, and no new
measurement exists.** Every number quoted in the design comes from the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations. Design section 13 lists the six
measurements the implementation must produce, including maximal accepted seed, metadata, body,
projection and roster shapes, and the retained input/output accounting that is explicitly not a
heap ceiling.

## Review findings

None yet. The design review has not been requested; the copyable request is in design section 18
and needs the head SHA of the commit that adds these documents.

## Proposed UI-hooks update (for Agent 4, not yet applicable)

Do **not** apply this until design section 12's prerequisites P1 to P5 pass and the
implementation checkpoint passes review 1. Until then FLIPNOTE-UI-HOOKS must keep saying that
durable overlay Save is unavailable.

Under "Available now", after the existing `studio_overlay_read` block, add:

```ts
studio_overlay_save({ server, channel, object?, nonce, body })
type OverlaySave =
  | { v: 1; kind: "local-draft"; channel: string; object: string | null;
      basis: string; accepted: number; alreadySaved: boolean;
      transferState: "active"; shared: false; receipted: false; content: StudioContent }
  | { v: 1; kind: "handed-off"; channel: string; object: string | null;
      basis: string; accepted: number; epoch: string; epochId: string;
      shared: true; receipted: false };
```

Accompanying prose to add at the same time:

- Save is available only for a Closing document whose exact expected checkpoint can be
  constructed from the actual source, its saved signed close and the observed owner tenure.
  Fault, an unavailable seed, unknown tenure and an unconfirmed preview refuse and keep the
  editor's unsaved work visible; a refusal is never a durable save.
- `kind: "local-draft"` is vault-local only. `shared: true` means the operations are in the local
  signed source and remain pending in the ordinary ledger; it is never delivery, receipt or
  settlement. `receipted` is always `false` on this command; receipt state comes from settlement
  events only.
- `alreadySaved: true` is an exact retry of the same complete envelope; it creates no second
  operation.
- Automatic transfer happens in the background once a verified eligible successor is installed.
  It sends no packets of its own; peers receive the operations through ordinary paging. A large
  branch completes over several background turns while the app stays responsive.
- `studio_overlay_read` gains `transferState: "completed"` for a record holding only a completed
  acknowledgement. `kind: "absent"` keeps its existing meaning.
- Both commands share the latest-view request fence with ordinary reads; run them sequentially
  for the same target. The original native request and session span both custody visits.
  Capacity exhaustion returns a retryable error; the shared four preparation slots stay owned
  until actual worker or result destruction, including after cancellation.

## Next actions

1. Send the design review request in design section 18 after committing these two documents and
   filling the head SHA.
2. Confirm with Agent 2 the prerequisite list P1 to P5 and the `studio_overlay_runtime_hold`
   contract; confirm with Agent 3 that repair uses the existing fence and no second writer or
   pool; give Agent 4 the central edit list in design section 16.
3. Do not start implementation on this checkout. Create the implementation branch or worktree,
   and hold the C-1 decoder split and the `inventory.rs` change until the design verdict returns,
   since both touch accepted HANDOFF-002 territory.
