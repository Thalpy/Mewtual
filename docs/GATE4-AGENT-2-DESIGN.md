# Gate 4 Agent 2: overlay lifecycle, provisional local work and repeated tenure

Status: **revision 1, design proposal, awaiting adversarial review. No production code is written,
no test has been added and no Cargo command was executed.**

Design base: `1bcb1bca204d721b848b17c0835faf931ae930e3`. Scope is
[Agent 2 of the four handoffs](GATE4-AGENT-HANDOFFS.md); progress is in
[GATE4-AGENT-2-STATUS](GATE4-AGENT-2-STATUS.md). The applicable review scope is
[review preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

This document carries **two review boundaries the assignment requires to be separable**:

1. the manual lifecycle (inspect, export, copy, disposition), stale bases and repeated tenure;
2. the **separately reviewed** extension for durable local work based only on an
   `AwaitingTenureReceipt` preview (section 8), which the accepted Closing-overlay foundation
   deliberately excludes.

Section 9 additionally proposes a **correction to locally observed owner tenure**. It is a change
to an authority-bearing observation and is called out for its own verdict line.

Accepted work this design must not weaken: the Closing-overlay foundation (`b1b0ec9`,
OVERLAY-TEST-001 closed), the handoff design (HANDOFF-001) and its bounded core/store
implementation (`62f06d4`, HANDOFF-002 closed), the detached-inspection proposal (`0b28f06`) and
its read-only implementation (`c47ae0b`, INSPECTION-TEST-001 closed), the combined scheduling
block (`6b71d96`), and the accepted provisional read-only preview contract
(`a89bde6`/`134394e`, NATIVE-TEST-001 and TAIL-TEST-001 closed). No closure is reopened.

**Unmet dependencies.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at `e65bfd8` is
unreviewed. Agent 1's runtime design is at revision 2+ and unaccepted; this design consumes its
seams by name only and section 14 records what breaks if they change.

## 1. Outcome and boundary

Every retained local branch has a bounded, authorized way to be inspected, exported, copied into
current authorized work, or explicitly disposed of, and none of those is available by accident.
Repeated owner changes and newcomers preserve the difference between unconfirmed history, local
work and verified authority, and a fail-closed Unknown tenure has at least one implemented,
legitimate way to become Known.

In scope: the four manual operations and their durable records; stale, rewound and nonpristine
bases; preview-based local work; A -> B -> A succession and rejoining; the live-tenure contract
consumed by Agents 1 and 3; truthful native results, events and UI-hooks rows.

Out of scope: actor-scheduled Save and automatic handoff runtime, preparation permits, the
inventory cursor and transient reference holds (Agent 1); signed fault repair (Agent 3);
integration, shared-document edits and full-gate acceptance (Agent 4). **Nothing here registers
`studio_overlay_save`**; section 13 states exactly which of Agent 1's registration prerequisites
this design satisfies.

There is **no import command**. Export is a one-way local backup. Accepting an exported archive
back into a vault would be a new authority boundary with its own provenance problem and is not
proposed for Gate 4.

## 2. What was audited

Read in full at the design base, with the facts each established:

| Source | Fact established and used below |
|---|---|
| `crates/catcoms-replication/src/studio/overlay.rs` | `BasisData` binds target, author, `source_id`, `source_version`, the full `Receipt` and the seed bytes; `fingerprint()` derives from that exact encoding. `StudioOverlay::read` reconstructs from `base.graph()` (the seed alone), applying each accepted operation under `local_policy`, `prepare_local_write`, `validate` and `recovery::preflight`. `decode_vault` ends in `read(ledger)` and a canonical re-encode equality check. `MAX_STUDIO_OVERLAY_OPS = 256`, `MAX_METADATA = 64 KiB`, `MAX_EXTENSION = MAX_CHECKPOINT_BYTES + MAX_METADATA`. |
| `.../studio/overlay/handoff.rs` | `StudioOverlayState` is v1-or-v2: v1 is the bare `StudioOverlay`, v2 adds `target`, `minimum_new_basis_closed_epoch`, an optional `Prepared` and an optional `Completed` manifest. `encode_vault` emits **v1 byte-for-byte** when `legacy && !prepared && !completed && floor == 0`. `Completed` retains `entries` (id, envelope, sequence, ts) after `active` is dropped, and `validate` tolerates those entries being absent from the ledger while requiring any present entry to match author and envelope. `metadata` is charged as `bytes.len() - seed_bytes`, so a record with no `active` must fit 64 KiB entirely. |
| `crates/catcoms-app/src/store/epoch_intents.rs` | `MAX_RECORD_BYTES = MAX_INTENT_LEDGER_BYTES + 1024 = 5 MiB + 1024`; `MAX_VAULT_INTENT_BYTES = 64 MiB`. `EpochIntentState::{encode,decode}` wrap `IntentLedger` plus the optional extension under tag `2`; an unknown extension tag fails closed. `is_overlay`, `handoff_metadata`, `handoff_prepared`, `local_draft` already exist. `read_scoped_intent_plain` returns authenticated plaintext and physical size without decoding. |
| `.../store/epoch_intents/retirement.rs` | One private `retire_included_with_io` serves receipt retirement and manual-recovery disposition. It refuses while `handoff_prepared()`, refuses on any full-envelope conflict, and **filters every `state.is_overlay(id)` out of the removal set**, which is the foundation hold this design must replace with an explicit transition. On `removed == 0` it performs a sync-only exact-retry flush that reseeds nothing. |
| `.../store/epoch_intents/overlay.rs` | `write_studio_overlay_intent` performs `exact_retry` -> ordinary-collision refusal -> basis fingerprint equality -> `append` -> `hold_creative(base_blob_cids)` + `hold_creative_operation` -> one accounted write. |
| `.../store/epoch_intents/inspection.rs` | `capture_studio_inspection` reads one bounded record, stamps `(mount, server, document, target, author, blake3(plain), physical_bytes)`, and `rebuild` decodes and calls `local_draft()`. `studio_inspection_is_current` re-reads and compares that digest and size. **This is the entire capture/delivery machinery export and copy must reuse.** |
| `crates/catcoms-app/src/studio/inspection.rs` | Two-visit job: `begin_studio_inspection` takes one permit from the shared `registry_catchup::preparation_pool()`, `rebuild` runs on `spawn_blocking` (the worker owns the permit through cancellation), `finish_studio_inspection` rechecks channel, current membership, `(group, device, owner, mls)` and the store stamp. `StudioOverlayInspection::inspect` is fenced by a 5-second delivery guard. |
| `.../studio/restore.rs` | `plan(current, historical, history: &[StudioRecovery], item, mode, restorer)` is a **pure planner over two projections**. It refuses unless `current.document() == historical.document()` and channels match, screens deletions across the current projection and every history version, enforces `FLIPNOTE_MAX_FRAMES`, `FLIPNOTE_FRAME_BYTES`, `MAX_INDEX_OBJECTS` and over-cap, and returns one canonical `DomainOp` body authored by the restorer. `Object` creation sets `created_by: restorer`. |
| `.../studio/control.rs` | `Preview {snapshot,item,mode}` -> `Apply(StudioRecoveryApply)` is the accepted two-phase shape: plan, then apply with `epoch_id` + `expected_projection` staleness fences and a `contains_exact_operation` exact-retry shortcut, routed through the **ordinary** `StudioRequest::Apply` publication path. `InspectOverlay`/`FinishOverlayInspection` are routed before recovery decoding. `Export {snapshot}` returns bounded canonical bytes with no durable effect. |
| `.../store/epoch_studio/recovery_disposition.rs` | `move_studio_intents_to_recovery_with_io` is the accepted recovery-before-removal template: match complete own envelopes against actual retained/staged recovery, refuse ids present in the current signed log, re-stage the exact snapshot through the accounted recovery writer, **then** remove from the ledger; an empty exact retry performs a sync-only flush. |
| `.../studio/replay.rs` | `studio_replay_evidence` filters `own` by author only; `choose` returns `NoEvidence` when no recovery version carries the exact envelope. Recovery versions are the only replay evidence. |
| `.../studio/receiver/catchup/preview.rs`, `.../studio/preview.rs`, `studio_exchange/provisional/seed.rs`, `catcoms-sync/src/registry_seed/provisional/seed.rs` | A ready preview is `PreparedProvisionalStudioSeed { seed: UnconfirmedStudioSeed, tail: TailProgress, hint }`. `unconfirmed_is_unexpired(now)` is `now < hint.expires`, a fixed 60 s from discovery. The projection is `seed.projection()`, parsed from `(target, hint.head.receipt(), raw)` and then advanced by the signed tail. At most three ready previews, evicted by capacity, expiry, unwatch, mount/server replacement and membership change. `ProvisionalStudioHintUse` exposes `target`, `peer`, `provider`, `receipt`. |
| `crates/catcoms-sync/src/owner_tenure.rs` | `OwnerTenure::new` sets `Some(0)` only for a locally founded epoch-zero group. `unknown` sets `None`. `applied(before, group)` returns `Some(after.epoch)` only when the owner actually changed across a **contiguous** epoch step, preserves knowledge across same-owner commits, and yields `None` on any gap or absent owner. `start()` and `encode()` refuse unless the saved `Position` still equals the live group's. |
| `crates/catcoms-sync/src/lib.rs` | Three construction sites: `ChannelSync::new` -> `OwnerTenure::new`; `restore` -> `decode` or, for a legacy snapshot with no tenure bytes, `unknown`; **`new_joined` -> `unknown` unconditionally**, and `new_joined` is the sole Welcome/join constructor. |
| `crates/catcoms-sync/src/receipt_head.rs`, `receipt_head/detached.rs` | `prepare_receipt_head_snapshot` and `with_durable_owner_snapshot` require `observed_owner_tenure_start()` to be `Some`. `complete_checkpoint_head_scoped` accepts an owner proof when the provider is the live designated committer and local observation is either absent or equal, minting a `HeadSelection { tenure: proof.tenure_start_group_epoch, verified }`. `registry_seed.rs` carries that `tenure` through `RegistrySeedFetch` into `RegistrySeedUse`/`CheckpointSeedSelectionUse`, so the **authoritative install path already supplies a tenure to a node whose local observation is Unknown**. |
| `crates/catcoms-replication/src/epoch.rs` | `Receipt::verify_current_owner` requires live committer identity, roster key equality, `tenure_start == expected` and `tenure_start <= group.epoch()`. `ReceiptHeadProof::verify` additionally binds document, receipt hash, requester and nonce. `ReceiptBook`/`OwnerReceiptJournal` are already tenure-keyed: `TenureSelection`, first-of-tenure inheritance, `changes_tenure` requiring a strictly newer `tenure_start`, and equivocation only within one `tenure_id`. `ReceiptRepair::verify_current_owner` refuses v1 and any earlier tenure of the same key. |
| `crates/catcoms-app/src/studio_exchange/tests/succession/joining.rs` | The reproduced boundary: a newcomer joining through a recycled low leaf **becomes owner**, `newcomer.sync.observed_owner_tenure_start() == None`, while the displaced `provider` observed the transition and reports a strictly higher value. |
| `apps/desktop/src-tauri/src/studio/{inspection,recovery}.rs`, `studio.rs` | `studio_overlay_read` is the registered two-visit read. `studio_recovery_export` is the accepted export precedent: `{format, bytes, bytesB64}` under `bounded_view`'s 32 MiB ceiling. `invoke_control`/`invoke_custody` supply the single session/request/instance fence. |

## 3. Audit observations

### O1: the foundation's hold is a removal filter, not a lifecycle

`retire_included_with_io` drops every annotated id from the removal set. The consequence is not
merely "disposition is missing": annotated entries can **never** be removed, because they will
never be covered by a receipt (they were never in a signed source) and the manual-recovery path
requires them to occur in an actual retained recovery snapshot, which they never do. A retained
branch is therefore permanent storage until an explicit overlay-aware transition exists. This is
the concrete defect the assignment calls "not acceptable as the final user-facing lifecycle".

### O2: a disposed branch's bodies cannot be archived inside the existing record

The extension's metadata ceiling is 64 KiB excluding seed bytes, and 64 KiB **including** them once
`active` is dropped. A single `DomainOp` is bounded by `MAX_DOMAIN_OP_BYTES = 64 KiB` and a branch
may hold 256 of them, so the complete envelopes of a disposed branch cannot be retained in the
extension. Retaining them elsewhere would require a sixth inventoried record family, which collides
head-on with Agent 1's I-4 mutation-generation work and Agent 3's writers. Section 6.4 therefore
makes preservation **explicit and verified** (a durable copy) or **explicitly waived** (a confirmed
discard), and retains a bounded manifest either way. It does not invent a silent archive.

### O3: `StudioRecovery` is the wrong container for an unsent branch

`StudioRecovery::snapshot` requires a `selecting_receipt` and a `base_close` tied to an actual
source opening, drives `studio/replay.rs`'s `choose`, and occupies the two-retained/one-staged
eviction rail with its seven-day deadline. Writing an unsent draft there would fabricate source
provenance, make never-shared work eligible for automatic replay, and evict genuine history. The
foundation's rule ("an unsent local branch cannot be passed off as `StudioRecovery` signed
historical evidence") is therefore a hard constraint, not a preference, and section 6.3 reuses
recovery's **planner** while deliberately not reusing its **container**.

### O4: `restore::plan` is exactly the copy engine the assignment asks for

"Copy uses authorized current typed edits with explicit user intent" describes `restore::plan`
precisely: per item, explicit mode, conflict/deleted/full dispositions, one canonical body, new
authorship by the copier, routed through ordinary Save. The only structural obstacle is its
`current.document() == historical.document()` precondition, which is correct for the primary
same-document case and must be widened, under narrow rules, for the cross-document case Agent 1's
12.2 needs while a branch is Prepared.

### O5: a preview's durable content is already content-addressed

A ready preview's base is fully identified by the candidate `Receipt` bytes and the seed bytes that
hash to its `seed_change_hash`, at `doc_id = epoch_id(type, key, closed_epoch + 1,
close_record_hash)`. None of that requires the live preview, the provider connection, the hint
lifetime or the ready-cache slot. A durable preview-based branch can therefore be reconstructed
after expiry from its own record, with no promotion of the preview to trusted history, **provided
the persisted base is the seed checkpoint and not the volatile signed tail** (section 8.2).

### O6: the newcomer's Unknown tenure has two different shapes, and only one is unsolved

- As a **reader/verifier**, a newcomer with `None` already has a legitimate path: a fresh
  nonce-bound owner proof yields `HeadSelection { tenure }`, and `registry_seed.rs` already carries
  that value into the authoritative install path. `complete_checkpoint_head_scoped` accepts the
  proof's value only when local observation is absent, and refuses when it disagrees.
- As an **owner**, a newcomer with `None` cannot call `prepare_receipt_head_snapshot`, so it can
  issue no receipt and rotate nothing. The reproduced fixture is exactly this case.

### O7: the owner case is decidable from local membership history, not from a peer's statement

`new_joined` is the sole Welcome constructor and calls `unknown(group)` unconditionally. But if the
local device **is** the designated committer in the group it has just joined, then its current
tenure necessarily started at that join epoch, because it was not a member of the group at any
earlier epoch and therefore cannot have been that group's committer at any earlier epoch. This is
an inference from the device's own membership history, not from Welcome's claim about somebody
else's tenure, not from a hint, a receipt's claimed tenure or one peer's statement. Section 9.3
makes that correction, states its one residual precisely, and does **not** extend it to legacy
restores, where the device may have been committer for an unknown number of prior epochs.

### O8: a lifecycle classification does not need reconstruction

Everything the manual lifecycle must decide before doing work — is there a branch, is it Active or
Prepared, is its provenance Closing or Unconfirmed, is its basis still derivable, has it been
disposed — is available from the structural fields of the extension plus the source record's
metadata. Full reconstruction is needed only for display, export and copy planning, all of which
already run detached under the inspection job. This is what makes the lifecycle cheap enough to
drive native results and events without a second heavy path.

## 4. Design principles

1. **Reuse the accepted read machinery literally.** Export and copy planning are new *rebuild
   functions* for the existing inspection capture, permit, stamp and delivery fence. No second
   capture path, no second pool, no second staleness contract.
2. **Read-only operations change no durable byte.** Inspect and export never clear Prepared, never
   retire, never mark anything settled and never authorize a later removal.
3. **Removal is a transition with durable evidence, not the absence of a hold.** Work leaves the
   ledger only through one explicit, authorized, exactly retryable transaction that either proves
   the work survives elsewhere or records that the user destroyed it.
4. **Copy authors new work.** A copied operation is a fresh, ordinarily authorized, ordinarily
   admitted, ordinarily accounted edit by the copier. It is never a promotion of the branch.
5. **Provenance is carried, never inferred.** A branch built on an unconfirmed preview is marked as
   such in its own durable record, and every authority-bearing consumer refuses it by that mark
   rather than by hoping the preview has expired.
6. **Expiry governs the preview, not the work.** Nothing that removes a preview may remove a
   durably accepted draft, and nothing that retains a draft may revive a preview.
7. **Tenure is observed or Unknown.** A claimed tenure may authorize verifying somebody else's
   receipt under a fresh proof; it may never authorize this device's own authoring, signing, repair
   or rotation.
8. **Refusal retains work**, and every refusal names an actionable state.

## 5. Concrete APIs

New leaf modules owned by Agent 2:

```
crates/catcoms-replication/src/studio/overlay/disposal.rs      (v3 arm, manifests, provenance)
crates/catcoms-app/src/store/epoch_intents/disposal.rs         (the disposal transaction)
crates/catcoms-app/src/store/epoch_intents/archive.rs          (bounded export serialization)
crates/catcoms-app/src/studio/lifecycle.rs                     (structural classification)
crates/catcoms-app/src/studio/overlay/copy.rs                  (copy planning and apply)
apps/desktop/src-tauri/src/studio/overlay.rs                   (native export/copy/dispose)
```

Shared files edited: `studio/overlay.rs` and `overlay/handoff.rs` (core, v3 encoding and the
provenance guard), `store/epoch_intents.rs` and `.../retirement.rs`, `store/epoch_intents/
inspection.rs`, `studio/{restore,control,dispatch,settlement,inspection}.rs`. Section 14 lists them
for Agent 4.

### 5.1 Core: provenance, and the v3 record

```rust
/// Where a branch's base came from. Carried in the record; never inferred at use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StudioOverlayProvenance {
    /// An installed Closing source, its matching saved signed close and observed tenure.
    Closing,
    /// An authenticated member's unconfirmed preview checkpoint. No installed source and no
    /// verified owner authority. See section 8.
    Unconfirmed,
}

impl StudioOverlay  { pub fn provenance(&self) -> StudioOverlayProvenance; }
impl StudioOverlayState {
    pub fn provenance(&self) -> Option<StudioOverlayProvenance>;
    pub fn disposed(&self) -> Option<&StudioOverlayDisposal>;
    /// Terminal acknowledgement for an exactly matching request whose branch was disposed.
    /// Checked beside `completed_retry`, before any basis minting or source lookup.
    pub fn disposed_retry(&self, target: StudioTarget, basis: [u8; 32], intent: &LocalIntent)
        -> Result<Option<StudioOverlayDisposal>, ReplError>;
    /// The one transition that may drop an active branch without transferring it. The caller
    /// has already proved authorization, destination durability (for `Copied`) and explicit
    /// user confirmation (for `Discarded`); this method only rebuilds and validates state.
    pub fn dispose(&self, ledger: &IntentLedger, decision: StudioDisposalDecision,
                   sequence: u64, at: u64) -> Result<(Self, BTreeSet<[u8; 32]>), ReplError>;
    /// Copy bookkeeping for one destination, written after the destination Save is durable.
    pub fn record_copy(&self, ledger: &IntentLedger, progress: StudioCopyProgress)
        -> Result<Self, ReplError>;
    pub fn copy_progress(&self) -> Option<&StudioCopyProgress>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioDisposalMode {
    /// Every accepted entry has a durable destination operation recorded in `copy_progress`.
    Copied { destination: [u8; 32], destination_epoch: u128 },
    /// The user explicitly destroyed the work after observing it.
    Discarded,
}

/// Bounded manifest retained after the branch is gone. Full bodies are NOT retained (O2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioOverlayDisposal {
    pub target: StudioTarget, pub author: DeviceId, pub provenance: StudioOverlayProvenance,
    pub basis: [u8; 32], pub branch: [u8; 32], pub mode: StudioDisposalMode,
    pub accepted: usize, pub sequence: u64, pub at: u64,
    /* private: Vec<Entry> (id, envelope, sequence, ts), <= MAX_STUDIO_OVERLAY_OPS */
}

/// One destination, appended entry by entry as each destination Save becomes durable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioCopyProgress {
    pub destination: [u8; 32],      // blake3 of the destination LogicalDocument's canonical bytes
    pub destination_epoch: u128,
    pub basis: [u8; 32],
    /* private: Vec<([u8; 32] source_entry_id, [u8; 32] destination_op_id)>, <= 256 */
}
```

**Encoding rules.** A third version tag `3` follows the accepted v1/v2 discipline exactly:

1. A state expressible as v1 still encodes as v1 byte-for-byte (the existing `legacy` rule).
2. A state expressible as v2 — `provenance == Closing`, no `disposed`, no `copy` — still encodes
   as **v2 byte-for-byte**. Existing records are not rewritten, so Agent 1's C-2 digest fence and
   the accepted `encode_vault(ledger)? == bytes` equality are unaffected for every current vault.
3. Tag `3` is emitted only when `provenance == Unconfirmed`, `disposed.is_some()` or
   `copy.is_some()`. Its layout is the v2 layout followed by: a provenance byte (0 `Closing`,
   1 `Unconfirmed` with its `provider`, `observed_mls_epoch` and `observed_at_ms`), an optional
   `copy` block, and an optional `disposed` block.
4. A reader accepts exactly one complete v1, v2 or v3 form. Unknown version bytes, trailing bytes,
   duplicate ids, noncanonical order and a wrong `sequence` reject, and the existing canonical
   re-encode equality check runs unchanged. Old readers already fail closed on tag 3
   (`byte(&mut d)? != 2` -> `Malformed`).
5. For `Unconfirmed`, the **nested v1 basis blob is untouched**: its `source_id` and
   `source_version` fields must be canonically zero, and the provenance fields live in the v3
   outer record. The nested encoder and its bounds are not modified.

**Validation additions to `StudioOverlayState::validate`**, all inside the existing method so the
decoder and `encode_vault` share one predicate:

- no operation id occurs in more than one of `active`, `completed.entries`, `disposed.entries`;
- `disposed.accepted == disposed.entries.len()`, `validate_manifest(disposed.entries)` passes, and
  a ledger entry that is still present for a disposed id must match its author and envelope
  (the same tolerance `completed` already has);
- `copy.entries` are a subset of `active.entries` by id, with no duplicate source or destination
  id, and `copy` is absent whenever `active` is `None`;
- `disposed.mode == Copied { destination, .. }` requires a `copy` whose `destination` and `basis`
  match and whose source-entry set equals `disposed.entries` by id;
- `provenance == Unconfirmed` forbids `prepared` (section 8.5), and forbids a nonzero nested
  `source_id`/`source_version`;
- the combined metadata ceiling is charged as today. Two full 256-entry manifests plus target,
  basis and mode fields occupy approximately 44 KiB of the 64 KiB budget; the encoder refuses over
  it and the branch stays retained. Section 12 records this as a real limit L3.

**The provenance guard.** `StudioOverlayState::prepare_handoff`, `prepare_handoff_detached` and
`prepared_manifest` refuse unless `provenance == Closing`, before any authority work. This is the
single core-side fence that makes an unconfirmed branch structurally incapable of becoming signed
history, independent of any app-level check.

### 5.2 Store: the classification, the archive and the disposal transaction

```rust
/// Structural only: one bounded authenticated read plus the source record's metadata.
/// No reconstruction, no seed parse, no typed replay, no graph restore (O8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioOverlayLifecycle {
    Absent,
    Draft {
        provenance: StudioOverlayProvenance,
        transfer: StudioOverlayTransfer,          // Active | Prepared
        eligibility: StudioOverlayEligibility,
        basis: [u8; 32], branch: [u8; 32], accepted: usize,
        copied: usize,
    },
    Transferred(StudioHandoffOutcome),
    Disposed(StudioOverlayDisposal),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOverlayEligibility {
    /// The branch's own basis is still derivable and automatic transfer may be attempted.
    Transferable,
    /// Manual action only. Each reason is separately displayable and separately tested.
    Manual(StudioOverlayManualReason),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOverlayManualReason {
    SourceMissing, SourceNotClosing, SourceReplaced, SourceRewound,
    SuccessorNotPristine, SuccessorMissing, ReceiptChanged, CloseMissing,
    TenureUnknown, Fault, NotCurrentAuthor,
    /// Unconfirmed provenance: never transferable. Section 8.6 refines this.
    Unconfirmed(StudioUnconfirmedState),
}

impl ServerStore {
    /// Caller holds live membership custody. Bounded reads only.
    pub(crate) fn studio_overlay_lifecycle(
        &self, server: u64, group: &[u8], target: StudioTarget, author: DeviceId,
    ) -> Result<StudioOverlayLifecycle, AppError>;

    /// One accounted, atomic replacement of the intent record: the disposal manifest is written
    /// in the SAME sealed plaintext that removes the named annotated entries. There is no window
    /// in which the entries are gone without their evidence, and no second record is touched.
    /// Retires no ordinary intent, prunes no source, deletes no blob, writes no recovery record.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn dispose_studio_overlay_with_io(
        &mut self, server: u64, document: &LogicalDocument, target: StudioTarget,
        group: &ServerGroup, device: &MlsDevice, request: &StudioOverlayDisposalRequest,
        ts: u64, rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget, intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioOverlayDisposal, AppError>;

    /// Same shape, for the copy bookkeeping write that follows a durable destination Save.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn record_studio_overlay_copy_with_io(/* .. */)
        -> Result<StudioCopyProgress, AppError>;
}

/// Caller-supplied, fully explicit. Every field is compared against durable state.
pub struct StudioOverlayDisposalRequest {
    pub basis: [u8; 32],
    /// `branch_hash(active, ledger)` as returned by the inspection the user actually saw.
    pub branch: [u8; 32],
    pub accepted: usize,
    pub mode: StudioDisposalMode,
}
```

Export reuses the inspection capture with a second rebuild function:

```rust
pub(crate) enum StudioInspectionPurpose { Draft, Archive, CopyPlan(StudioOverlayCopyChoice) }
impl StudioInspectionCapture {
    /// Existing `rebuild` is `rebuild_for(Draft)`. Every purpose runs the SAME
    /// `EpochIntentState::decode` + `handoff_metadata` target/author checks, and returns the
    /// SAME `StudioInspectionStamp` for the existing `studio_inspection_is_current` recheck.
    pub(crate) fn rebuild_for(self, purpose: StudioInspectionPurpose)
        -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError>;
}
#[derive(Debug)]
pub(crate) struct StudioInspectedDraft {
    pub(crate) target: StudioTarget,
    pub(crate) prepared: bool,
    pub(crate) draft: Option<StudioLocalDraft>,
    /* new, all None unless the matching purpose was requested */
    pub(crate) archive: Option<Zeroizing<Vec<u8>>>,
    pub(crate) plan: Option<StudioOverlayCopyPlan>,
    pub(crate) branch: Option<[u8; 32]>,
    pub(crate) provenance: Option<StudioOverlayProvenance>,
}
```

### 5.3 The export archive format

`catcoms-studio-draft-v1`, produced only from an already reconstructed branch:

```
u8   version = 1
u8   provenance (0 Closing, 1 Unconfirmed)
bytes document.server_id, u16 doc_type tag, bytes document.logical_key
u8   target kind, bytes channel, [bytes object]
bytes author(32), bytes basis(32), bytes branch(32)
bytes receipt (canonical Receipt::encode)
bytes seed
[provenance == 1: bytes provider(32), u64 observed_mls_epoch, u64 observed_at_ms]
u32  count
     per entry, in saved sequence: bytes id(32), u64 sequence, u64 ts,
                bytes author(32), bytes operation (DomainOp::encode)
```

Bounded by construction: the source of every field is the already-bounded record, so the archive
is at most `MAX_RECORD_BYTES` plus framing (5 MiB + ~9 KiB), well inside `bounded_view`'s 32 MiB
after base64. It carries **no signature and no vault key material**, and is explicitly not
evidence: it cannot be decoded back into a `StudioClosingOverlayBasis`, a `StudioOverlay`, a
`VerifiedReceipt` or any store record, because no decoder for it exists in the vault direction.

### 5.4 App: control actions and responses

```rust
pub enum StudioControlAction {
    // ... existing ...
    /// Cheap structural state for the lifecycle row. No permit, no reconstruction.
    OverlayLifecycle,
    /// Export: first visit reuses `begin_studio_inspection` unchanged.
    ExportOverlay,
    FinishOverlayExport(Box<StudioPreparedInspection>),
    /// Copy: first visit reuses `begin_studio_inspection`; the choice travels with the finish.
    PrepareOverlayCopy(Box<StudioOverlayCopyChoice>),
    FinishOverlayCopyPreview(Box<StudioPreparedInspection>),
    /// Applies exactly the previewed body through the ORDINARY Save path, then records progress.
    ApplyOverlayCopy(Box<StudioOverlayCopyApply>),
    /// Explicit, fully bound, exactly retryable.
    DisposeOverlay(Box<StudioOverlayDisposalRequest>),
}
pub enum StudioControlResponse {
    // ... existing ...
    OverlayLifecycle(StudioOverlayLifecycleView),
    OverlayExport(StudioOverlayArchive),
    OverlayCopyPreview(StudioOverlayCopyPreview),
    OverlayCopied { target: StudioTarget, destination: StudioTarget,
                    already_saved: bool, copied: usize, remaining: usize },
    OverlayDisposed(StudioOverlayDisposal),
}

pub struct StudioOverlayCopyChoice {
    pub destination: StudioTarget,
    pub item: StudioRecoveryItem,
    pub mode: StudioRecoveryMode,
}
pub struct StudioOverlayCopyPreview {
    pub target: StudioTarget, pub destination: StudioTarget,
    pub basis: [u8; 32], pub branch: [u8; 32],
    pub epoch_id: u128, pub fingerprint: [u8; 32],
    pub plan: StudioRecoveryPlan,
}
pub struct StudioOverlayCopyApply {
    pub destination: StudioTarget, pub item: StudioRecoveryItem, pub mode: StudioRecoveryMode,
    pub basis: [u8; 32], pub branch: [u8; 32], pub source_entry: [u8; 32],
    pub epoch_id: u128, pub expected_projection: [u8; 32],
    pub nonce: [u8; 16], pub body: Vec<u8>,
}
```

`StudioOverlayArchive` holds `Zeroizing<Vec<u8>>` plus `basis`, `branch`, `accepted` and
`provenance`, and has a content-free `Debug`, matching the existing rule that export bytes are
private vault content.

### 5.5 Native surface

Registered by this design (all read-only or explicitly user-initiated, none of which enables Save):

```ts
studio_overlay_read({ server, channel, object? })            // extended, section 11
studio_overlay_lifecycle({ server, channel, object? })
studio_overlay_export({ server, channel, object? })
studio_overlay_copy_preview({ server, channel, object?, destination, choice, mode })
studio_overlay_copy_apply({ server, channel, object?, destination, edit })
studio_overlay_dispose({ server, channel, object?, basis, branch, accepted, mode })
```

All six use the existing `InvokeContext`: one UI session generation, one actor instance, one native
operation slot, one `ViewRequest` per `(state, server, target)` and one `RequestCancellation`
spanning both custody visits, exactly as `studio_overlay_read` does today. Export and copy preview
additionally keep the existing 5-second `StudioInspectionDelivery` fence, so a late conversion
cannot return a value after the actor has invalidated it.

## 6. The four manual operations

### 6.1 Inspect: unchanged

`studio_overlay_read` keeps its accepted contract. Section 11 adds fields that describe lifecycle
state; the existing `absent` and `local-draft` kinds, the `readOnly: true` marker, the `basis`
semantics and the absence of `epochId`/`epoch`/`phase`/publication/receipt flags are preserved.

### 6.2 Export

| # | Stage | Custody | Work |
|---|---|---|---|
| X1 | Begin | yes | `begin_studio_inspection`: channel known, current membership, `(group, device, owner, mls)` context, one shared preparation permit, `capture_studio_inspection` (one bounded authenticated read, stamp). **Identical to the accepted read.** |
| X2 | Rebuild | detached | `rebuild_for(Archive)`: full `decode_vault`, target and author checks, complete reconstruction (so a non-replayable branch is refused here and surfaced as a hold, per Agent 1's I-1), then serialize section 5.3 from the reconstructed entries and ledger envelopes. |
| X3 | Finish | yes | `finish_studio_inspection`'s existing rechecks: registry instance, channel, current membership, `(group, device, owner, mls)` equality and the store stamp digest+size. |
| X4 | Deliver | native | Existing delivery guard, `bounded_view`, base64. |

Rules:

- **E1.** Export writes no durable byte. It does not clear `Prepared`, cancel a live job, retire an
  entry, advance a floor, mark anything settled, release a reference hold or record that an export
  occurred. Under Agent 1's 12.1 table it is permitted while a **transfer hold** exists and refused,
  retryably, while a **live hold** exists.
- **E2.** Export requires current membership, through the unchanged `inspection_context`. A removed
  member acquires no offline export right: the same check that gates the accepted read gates this.
  No new code path reads the intent record without it.
- **E3.** Export is not `.pixa`, not settlement, and not permission to delete the original. Having
  exported grants nothing: disposal has its own explicit request, its own matching fences and its
  own truthful mode (6.4). The native result says so in `readOnly: true` and in the hooks row.
- **E4.** The archive names the referenced PIX CIDs inside the operation bodies; it does not contain
  pixel bytes and does not promote, hold or fetch them. A user who wants the pixels uses the
  existing `request_blob_bounded` while the references are still protected.

### 6.3 Copy into current

Two-phase, mirroring the accepted recovery `Preview` -> `Apply` shape exactly (O4).

**Destination scope.**

- **Same-document copy** (primary): the destination is the branch's own logical document, which
  must now be **Open**. This is the ordinary stale-base outcome: the document rotated past the
  branch's Closing basis and is open again. `restore::plan`'s document equality holds unchanged.
- **Cross-document copy** (the case Agent 1's 12.2 needs while a branch is Prepared): the
  destination is a **different** logical document of the **same `doc_type` in the same channel** —
  in practice Flipnote -> Flipnote. Index is same-document only, because the Index's logical key is
  the channel.
- **C-0, answering Agent 1's 12.2 directly.** The destination's identity is its complete
  `LogicalDocument` (server id, doc type, logical key), not its channel label. A different channel
  label naming the same Flipnote object is **the same destination** and is refused as
  "destination is the branch's own document" while a transfer hold exists. The destination is
  bound by `blake3` of its canonical `LogicalDocument` bytes in `StudioCopyProgress`, rechecked at
  the write.

**Planner change.** `restore::plan` currently takes `history: &[StudioRecovery]` only to screen
deletions. It becomes:

```rust
pub(crate) fn plan(
    current: &StudioProjection, historical: &StudioProjection,
    history: &[&StudioProjection], item: StudioRecoveryItem,
    mode: StudioRecoveryMode, restorer: DeviceId, scope: PlanScope,
) -> Result<StudioRecoveryPlan, AppError>;
pub(crate) enum PlanScope {
    /// Existing behaviour: identical document and channel. The one existing caller passes this.
    SameDocument,
    /// Same doc_type, same channel, different logical key. Deletion screening uses the
    /// destination's current projection and the destination's recovery history only.
    CrossDocument,
}
```

`control::preview` passes `SameDocument` and maps its `&[StudioRecovery]` to projections; no
behavioural change for recovery. `CrossDocument` keeps every capacity, conflict, tombstone and
over-cap check and drops only the logical-key equality.

| # | Stage | Custody | Work |
|---|---|---|---|
| C1 | Begin | yes | `begin_studio_inspection` (unchanged), carrying the `StudioOverlayCopyChoice`. |
| C2 | Plan | detached | `rebuild_for(CopyPlan(choice))`: full `decode_vault`, reconstruct the draft projection, then `restore::plan` against the destination projection captured in C1. |
| C3 | Preview | yes | `finish_studio_inspection`'s rechecks, plus: destination channel known, destination source still **Open** with the same `doc_id`, `expected_projection = projection.recovery_fingerprint()`. Returns one bounded proposed body or an explicit hold. Saves nothing. |
| C4 | Apply | yes | `prepare_studio_overlay_copy`: exact-retry shortcut first (`contains_exact_operation` on the destination), then re-plan from durable state and require `epoch_id`, `expected_projection`, `disposition == Ready` and byte-identical `body`. Then the **ordinary** `StudioRequest::Apply` publication path. |
| C5 | Record | yes | After the destination Save is durable, `record_studio_overlay_copy_with_io` appends `(source_entry, destination_op_id)` to `copy_progress` on the **branch's own** record, under the ordinary accounted writer. |

Rules:

- **C1'.** Copy is refused, retryably, while a live hold exists on either document, and refused
  while a transfer hold exists on the **destination**. It is permitted while a transfer hold exists
  on the source, and does not clear `Prepared`, retire the original envelopes, or count as evidence
  that the original handoff completed (Agent 1's 12.2).
- **C2'.** A bulk copy is the user issuing C3/C4 per item. There is no batch command. Each item
  consumes ordinary admission, typed policy, capacity preflight, reference protection and content
  budget, so a full destination refuses per item with `Full` rather than half-applying a batch.
  The runtime custody rule agreed with Agent 1 is respected because C2 is the only detached stage
  and it owns the same single shared preparation permit the accepted inspection owns.
- **C3'.** Authorship: the copied operation is authored by the copier with a fresh nonce.
  `StudioRecoveryPlan::original_author` carries the branch author for display, exactly as recovery
  does; it is never written into the destination operation except through
  `IndexOp::PutObject { created_by: restorer }`, which is already the restorer.
- **C4'.** C4 and C5 are two records and therefore not atomic. A crash between them leaves the
  copy durable and unrecorded; the exact retry finds `contains_exact_operation` true, returns
  `already_saved`, and re-writes C5 idempotently. This is the accepted recovery-apply retry
  contract, unchanged.
- **C5'.** `copy_progress` is bookkeeping, not a claim of delivery, inclusion, settlement or
  equivalence. Its only privileged use is as the precondition for `Dispose { Copied }`.

### 6.4 Disposition

One transaction, two truthful modes, and a mandatory durable manifest (O2).

**Preconditions, all checked under one exclusive store visit before any write:**

| # | Check |
|---|---|
| D1 | Current membership and the complete target/channel scope; the requester is the branch's own author. A member may not dispose of another device's branch. |
| D2 | No live hold (Agent 1's `studio_overlay_live_hold`) and **no transfer hold**: a Prepared branch refuses disposal outright and must be resolved or held first. |
| D3 | `request.basis`, `request.branch` and `request.accepted` equal the durable branch's `basis()`, `branch_hash(active, ledger)` and entry count. A stale request from a UI that has not re-inspected since the branch changed refuses with `BranchChanged`. This is the "explicit user intent" fence: the caller must name the exact thing it saw. |
| D4 | For `Copied { destination, destination_epoch }`: `copy_progress` exists, its `destination`, `destination_epoch` and `basis` match, and its source-entry set equals the branch's entry set. Then, **re-verified now against durable state**, every recorded destination operation id is present in the destination's current signed source or its pending ledger. A destination that has since lost an operation refuses with `CopyIncomplete` and the branch is retained. |
| D5 | For `Discarded`: no copy is required; the request must carry `confirm_discard: true` and D3's exact branch identity. Nothing else substitutes for it. |
| D6 | Intent and storage preflight for the complete replacement peak, as for every other intent write, including at the vault cap. |

**The write.** `EpochIntentState` is rebuilt as: the ledger with exactly the branch's annotated ids
removed, and the extension with `active` and `copy` cleared and `disposed` set. The result is
encoded, preflighted, sealed and atomically persisted through the existing writer and sync barrier
in **one** replacement. Only durable completion returns a disposal acknowledgement.

**Ordering and what is not touched.**

- No ordinary intent is removed. The disposal removal set is exactly `disposed.entries`, all of
  which are annotated and all of which belong to the named branch. A mixed ordinary/annotated
  ledger keeps every ordinary entry, and the receipt-retirement path keeps its existing overlay
  filter unchanged — it is still incapable of removing an annotated id. This is the only path that
  can, and it can remove nothing else.
- No source, recovery, owner-journal, Registry or blob write accompanies the transaction. In
  particular **disposal deletes no pixels**. It stops the disposed branch from contributing to the
  conservative reference set; whether those CIDs become reclaimable depends on every other holder,
  and reclamation happens later through the ordinary cleanup path. For `Copied`, the destination's
  own operations keep the copied pixels referenced. For `Discarded`, unreferenced pixels do become
  reclaimable, which is precisely what the user confirmed.
- `minimum_new_basis_closed_epoch` is **not** advanced. A fresh Save on a still-eligible basis after
  a disposal is a new, legitimate decision. A delayed retry of a *disposed* request is caught by
  `disposed_retry` and returns the terminal disposal acknowledgement, never a new branch.

**Exact retry.** `disposed_retry(target, basis, intent)` runs beside `completed_retry` in the Save
classification (Agent 1's S1), before basis minting, tenure, source lookup or media admission. A
retried disposal request whose branch is already disposed with the same `basis`/`branch` returns the
saved `StudioOverlayDisposal` after a **sync-only** accounted flush, exactly as
`retire_included_with_io` does for `removed == 0`. A different request still fails.

**What disposal is not.** An equal projection, a seed marker, a source installation, a successful
export, an eviction acknowledgement, a membership change, a preview expiry and a failed copy are
each, individually and together, not disposal. Disposal happens only through this transaction.

**Honest limitation, for the reviewer.** `Discarded` does not preserve the operation bodies
anywhere. P1's recovery-before-removal is satisfied structurally for `Copied` and is **explicitly
waived by the user** for `Discarded`, with durable evidence of the waiver either way. The rejected
alternative — a sixth inventoried record family holding a full draft archive — is rejected for O2's
reasons and because it would collide with Agent 1's I-4 and Agent 3's writers. Section 16 asks the
reviewer to rule on this trade.

## 7. Stale, rewound and nonpristine bases

`studio_overlay_lifecycle` classifies from durable state alone (O8):

| Observation | Reason | Automatic transfer | Manual path |
|---|---|---|---|
| No installed source for the document | `SourceMissing` | refused | inspect, export, dispose |
| Source is Open, Settled or Fault | `SourceNotClosing` / `Fault` | refused | inspect, export, **copy** (Open), dispose |
| Source is Closing but `source_id`/`source_version` differ from the branch's basis | `SourceReplaced` | refused | inspect, export, dispose |
| `receipt.closed_epoch < minimum_new_basis_closed_epoch` | `SourceRewound` | refused | inspect, export, dispose |
| Saved signed close for the branch's receipt is absent from the owner journal | `CloseMissing` | refused | inspect, export, dispose |
| Receipt head no longer names the branch's receipt | `ReceiptChanged` | refused | inspect, export, dispose |
| Successor destination exists and is not pristine | `SuccessorNotPristine` | refused | inspect, export, dispose |
| `observed_owner_tenure_start()` is `None` | `TenureUnknown` | refused | inspect, export, dispose |
| Branch author is not the current local device | `NotCurrentAuthor` | refused | inspect only |
| `provenance == Unconfirmed` | `Unconfirmed(..)` | **never** | section 8.6 |
| Everything matches | — | eligible | all |

Rules:

- **S1.** A refusal to transfer never removes, rebases, renumbers or rewrites the branch. The
  `Hold` outcomes Agent 1's runtime produces map onto these reasons; this table is the consumer
  contract for them and satisfies Agent 1's registration prerequisite P2.
- **S2.** Reference protection is unchanged by staleness. The inventory's Intents arm already
  enumerates the branch's `base_blob_cids()` (via the reference path Agent 1's C-1 deliberately
  retains) and every pending operation's CIDs. Base-only, superseded and removed-frame pixels stay
  protected for as long as the branch is retained, including across restart, and are released only
  by the 6.4 transition.
- **S3.** A branch whose record is authenticated, canonical and structurally consistent but which
  fails typed reconstruction (Agent 1's I-1 boundary) classifies as
  `Manual(SourceReplaced)` for display purposes and supports **inspect (as an error), and
  disposal**; export and copy refuse at X2/C2 because they need the reconstruction. The user is not
  stranded: `Discarded` remains available and names the reason truthfully.
- **S4.** `check_basis_floor` remains the independent second fence after a rewind, on both the
  append path and the disposal path's basis comparison.

## 8. Durable local work on an awaiting-tenure preview — separate design review

This section is the extension the foundation deliberately excluded and the assignment requires to
be separately reviewed. **It is requested for review, not presented as accepted**, and section 17's
request asks for a distinct verdict line. A permanently disabled placeholder would not satisfy the
requirement, so a complete contract is specified.

### 8.1 Provenance

```rust
/// Minted ONLY inside a live `with_provisional_studio_seed` callback, so every current-scope
/// check has already passed: mount, numeric server, channel, copied Studio watch, attempt
/// generation, current membership, proven provider identity and an unexpired hint. There is no
/// public constructor and no path from a caller-supplied receipt, epoch id or projection.
pub struct StudioUnconfirmedOverlayBasis(/* private */);
impl StudioUnconfirmedOverlayBasis {
    pub fn fingerprint(&self) -> [u8; 32];
}
```

It binds exactly: `target`, the local device as `author`, the hint's candidate `Receipt` bytes, the
**seed bytes actually parsed into the preview's base**, the provider `DeviceId`, the current MLS
epoch and the receiver-clock observation time. `source_id` and `source_version` are canonically
zero. Additional admission conditions at mint time:

- `tail_complete()` must be true, so the preview rests on a finite authenticated prefix rather than
  a partially fetched one;
- the target's logical document must have **no installed source**. If an installed source exists it
  takes precedence and the ordinary paths apply; a preview may never shadow installed history;
- the requester must be a current member and the provider a current member and the proven endpoint
  identity of the peer the hint came from.

`fingerprint()` covers the provenance discriminant, so an `Unconfirmed` basis and a `Closing` basis
over the same receipt and seed have different fingerprints and cannot be interchanged in any
request.

### 8.2 What is persisted, and why the tail is not

The persisted base is **the seed checkpoint only**: the candidate receipt plus the exact seed bytes
whose hash equals `receipt.seed_change_hash`, at
`doc_id = epoch_id(doc_type, logical_key, closed_epoch + 1, close_record_hash)`.

The volatile signed tail is **not persisted**. Reasons: the tail is bounded at 20,000 operations /
4 MiB, which cannot coexist with a 2 MiB seed inside the 5 MiB record; and persisting other
members' signed operations would require re-verifying foreign signatures out of the vault on every
restart, which is a new authority surface for no user benefit — those operations arrive properly
through the ordinary installed source later.

The consequence is stated plainly and must be tested: **typed admission for each accepted operation
runs against the seed-only base**, so an operation that is only valid against the tail (for example
replacing a frame that exists only in the tail) is refused at acceptance with an explicit reason,
even though the live preview displays that frame. The live preview's merged display and the
persisted draft base are different things, and the native result labels them differently
(section 11).

The rejected alternative — persisting a bounded tail prefix, for example 64 operations / 256 KiB —
is recorded in section 16 for the reviewer.

### 8.3 Local storage and quotas

Simultaneous, not additive, and all checked before acknowledgement:

| Rail | Value | Rationale |
|---|---|---|
| Unconfirmed branches per logical document | 1 | Same as Closing. |
| Unconfirmed branches per server | 3 | Mirrors the accepted three preview-eligible slots, so unconfirmed durable work cannot outgrow the mechanism that produced it. |
| Accepted operations per unconfirmed branch | **64** | A quarter of the Closing limit; unconfirmed work is speculative and must not consume a Closing-sized budget. |
| Seed bytes | 2 MiB | Existing checkpoint limit, unchanged. |
| Extension metadata | 64 KiB | Unchanged. |
| Record total | `MAX_RECORD_BYTES` | Unchanged. |
| Vault-wide unconfirmed bytes | **8 MiB** | A ceiling *inside* the existing 64 MiB `MAX_VAULT_INTENT_BYTES`, not an additional allowance. |

The per-server count and the vault-wide byte total are produced by the inventory's Intents arm,
which already reads every intent record. This requires Agent 1's structural decode to expose the
provenance discriminant and the branch's charged bytes — a concrete dependency handed over in
section 14. A refusal is `StorageRefused { reason }` and retains all existing work.

### 8.4 Expiry versus retained work

- **E-a.** Acceptance captures the basis while the preview is current. After the durable write the
  branch is independent of the preview object entirely: reconstruction reads the persisted seed
  bytes from the record.
- **E-b.** Preview expiry, capacity eviction, replacement by a newer preview, unwatch and rewatch,
  lock, mount or numeric-server replacement, membership change and restart **must not** remove,
  invalidate, downgrade or silently rebase a durably accepted branch. The only thing they remove is
  the live preview.
- **E-c.** Conversely a retained branch never revives a preview, never extends a hint's lifetime,
  never re-enters the ready cache and never produces a `StudioRead::AwaitingTenureReceipt` result.
  The existing preview eviction/lifetime contract is unchanged in both directions.
- **E-d.** A refused or failed acceptance leaves the editor's work unsaved and visible and reports
  no durable success, exactly as the Closing path does.

### 8.5 What an unconfirmed branch can never do

Enforced at the core boundary (5.1's provenance guard) and again at every app consumer:

installed source; epoch gate; `VerifiedCheckpoint`; owner tenure; receipt issuance, verification or
publication; signing; Registry pointer publication; settlement; receipt-covered retirement;
`StudioRecovery` evidence; `studio_replay_evidence`; ordinary Apply; automatic handoff or any
`Prepared` state. `prepared` is forbidden by `validate`, so the record itself cannot express a
transferable unconfirmed branch.

What it **can** do: be inspected, exported, copied into an authorized current destination, and
disposed of — the same four manual operations, with the same fences, plus the reconciliation
classification below.

### 8.6 Reconciliation, derived rather than persisted

When an authoritative installed source finally exists for the document, the branch's state is
**computed on read** from durable state. Nothing is written, so there is no reconciliation crash
window and no new durable field:

| Condition | `StudioUnconfirmedState` | Effect |
|---|---|---|
| No installed source yet | `AwaitingSource` | inspect, export, dispose |
| Installed source's `doc_id` equals the branch's base `doc_id` **and** its opening checkpoint's seed change hash equals the branch's `seed_change_hash` | `BaseConfirmed` | the above, plus **copy into that source when it is Open** |
| Anything else | `BaseSuperseded` | inspect, export, dispose; copy into the current Open source is still offered, planned against the actual current projection, with the honest label that the base differed |

Reconciliation never applies an operation, never promotes the branch to `Closing` provenance, never
mints a basis, never writes a source, receipt, pointer, recovery record or owner-journal entry, and
never converts the candidate receipt into a `VerifiedReceipt`. `BaseConfirmed` is a statement about
two hashes agreeing, not a statement that the preview's provider was ever the owner.

### 8.7 Acceptance path

A new control action distinct from the Closing path, so neither can be reached with the other's
request:

```rust
/// Only reachable while a ready preview for this exact target is current. Returns the
/// unconfirmed basis fingerprint; it is an identifier, not authority.
BeginUnconfirmedOverlaySave,
PrepareUnconfirmedOverlaySave { basis: [u8; 32], nonce: [u8; 16], body: Vec<u8> },
FinishUnconfirmedOverlaySave(Box<StudioPreparedOverlaySave>),
```

The staging, stamps, admission token, permit ownership, PIX admission placement, retry
classification and commit ordering are **Agent 1's Flow S**, unchanged, with three substitutions:
`studio_closing_basis` becomes the 8.1 mint; the S3 re-mint re-enters
`with_provisional_studio_seed` and requires the same fingerprint; and the 8.3 rails are charged
alongside the ordinary ones. If Agent 1's Flow S is not implemented, this path is not implemented
either; it is not a second writer.

## 9. Repeated-owner tenure

### 9.1 What already works, and what the integration must prove

`ReceiptBook` and `OwnerReceiptJournal` are already tenure-keyed, and a continuously present member
already observes A -> B -> A correctly: `applied` returns `Some(after.epoch)` on each contiguous
owner change, so A's second tenure has a strictly greater start than its first, its `tenure_id`
differs, and an old receipt of A's first tenure fails `verify_current_owner` against the observer's
current expectation. The integration work is to prove this end to end through **real actors, real
membership changes and real restart**, not through unit fixtures, and to prove the refusals.

### 9.2 The five required cases

| # | Case | Required outcome |
|---|---|---|
| T1 | A -> B -> A, observer continuously present | Three distinct observed starts; A's second tenure strictly greater; A's first-tenure receipts refused as current; A's first-of-second-tenure receipt inherits the highest checkpoint it holds a verified receipt for and every later receipt of that tenure repeats the same inherited fields. |
| T2 | Restart between each transition | The saved value survives `encode`/`decode`, and `Position` inequality after an omitted integration hook fails closed rather than persisting fresh MLS with stale authority. |
| T3 | Member joining **between** owner changes | Unknown at join; it gains verified reading through a fresh owner proof and the authoritative install path; it never derives a tenure from Welcome's epoch, a hint, a receipt's claim or one peer's statement. |
| T4 | Hidden higher old-tenure history | History above the new tenure's inherited checkpoint is treated as a rewind into recovery, never silently adopted; the losing history is preserved. |
| T5 | A rejoining owner that becomes owner **by its own join** | Section 9.3. |

### 9.3 The correction: a joining device that is itself the committer

**Problem.** `new_joined` calls `OwnerTenure::unknown` unconditionally (O6, O7). A device that joins
into a recycled low leaf becomes the designated committer with `None`, so it can never call
`prepare_receipt_head_snapshot`, never issue a receipt and never rotate. Every peer that witnessed
the transition knows the answer; the new owner does not. A fail-closed Unknown with no legitimate
progress path is exactly what review preamble 2 calls incomplete integration.

**Correction.** In `ChannelSync::new_joined` only, and only when the local device is the group's
designated committer at the join epoch, the observed tenure start is that join epoch:

```rust
// crates/catcoms-sync/src/owner_tenure.rs
impl OwnerTenure {
    /// A device that has just entered a group through Welcome was not a member at any earlier
    /// epoch, and therefore cannot have been that group's designated committer at any earlier
    /// epoch. If it IS the committer now, its current tenure necessarily started here. This is
    /// an inference from this device's own membership history. It is deliberately NOT available
    /// to `unknown`, which also serves legacy snapshots where the device may have been committer
    /// for an unknown number of prior epochs.
    pub(super) fn joined(group: &ServerGroup, device: &MlsDevice) -> Self {
        let mut state = Self::unknown(group);
        if group.designated_committer() == Some(device.device_id()) {
            state.start = Some(state.position.epoch);
        }
        state
    }
}
```

`new_joined` calls `joined(&this.group, &this.device)` in place of `unknown(&this.group)`. Nothing
else changes: `applied`, `start`, `encode` and `decode` are untouched, and `decode`'s existing
`start > epoch` rejection still holds because `start == epoch` here.

**Why this is not "deriving tenure from Welcome's current epoch".** The prohibited inference is
about *somebody else's* tenure: a joiner cannot know when the current owner took office, because the
owner may have held the low leaf for many epochs before the join. The inference here is about *this
device's own* tenure, and it rests on a fact the device knows without trusting anyone's statement —
it was not in this group before this epoch. The Welcome's epoch number is trusted exactly as much as
the group itself already is: a fabricated group is a fabricated group regardless of this rule.

**Safety argument.** Every peer verifies a receipt's `tenure_start_group_epoch` against its **own**
observation, so a wrong value produces receipts nobody accepts. A witnessing peer computes
`after.epoch` for the add commit, which is the same value. The failure mode is therefore liveness,
not a forged authority, and it is detectable: the owner's receipts are refused.

**The one residual, stated precisely.** If a device is removed and re-added in the **same** commit
while remaining the committer, witnesses see `before.owner == after.owner` and preserve the old
start, while the rejoining device computes the new epoch. The two disagree and the rejoining owner's
receipts are refused until a later witnessed transition. This is bounded to a liveness failure. It
cannot occur when a rejoining device presents a distinct membership identity, which is the current
join behaviour. Mitigation and test: N-T6 asserts that a real remove-then-rejoin in **separate**
commits produces agreement between the rejoining owner and every witness, and the residual is
recorded as limit L5 with the recommendation that the membership policy not remove and re-add the
designated committer in one commit.

**What this does not fix.** A **legacy snapshot** whose owner has no saved tenure bytes stays
Unknown, correctly: the device may have been committer for an unknown number of prior epochs. A
device that becomes committer across an epoch gap it did not observe also stays Unknown. Section
9.5 sketches what those would need and does not implement it.

### 9.4 The live-tenure seam for Agents 1 and 3

```rust
/// Independently observed start of the CURRENT owner's tenure. `Unknown` is fail-closed for
/// every authoring, signing, repair-issuance, rotation and publication decision. A fresh owner
/// proof, a candidate receipt's claim, a hint, a reused key, a Welcome and the current group
/// epoch are each, and together, insufficient to make this `Known`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOwnerTenure { Known(u64), Unknown }

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub fn observed_owner_tenure(&self) -> StudioOwnerTenure;
    /// Fail-closed accessor for authoring stages. Never substitutes a claimed value.
    pub(crate) fn require_observed_owner_tenure(&self) -> Result<u64, AppError>;
}
```

Contract, satisfying Agent 1's registration prerequisite P4 and Agent 3's dependency:

- **V1.** `Unknown` is fail-closed for: minting a Closing overlay basis, first local acceptance,
  handoff preparation, every signing turn, the commit, receipt issuance, rotation, Registry pointer
  publication, and Agent 3's live v2 repair issuance and application.
- **V2.** A returning owner in a new tenure observes a **strictly different** value from its earlier
  tenure. `OwnerReceiptJournal`'s `changes_tenure` already requires strictly newer; V2 is the
  observation-side counterpart and is asserted directly in N-T1.
- **V3.** The value is read at every custody visit that needs it and is never cached across one.
  Agent 1's stamp compares tenure equality for its authoring stages and deliberately does not for
  acknowledgement; that split is preserved.
- **V4.** A verified-reading tenure obtained from a fresh owner proof travels only inside the
  existing private `HeadSelection`/`RegistrySeedFetch`/`CheckpointSeedSelectionUse` types. It is
  never written into `owner_tenure`, never returned by `observed_owner_tenure`, and never usable as
  this device's own authoring tenure.
- **V5.** Agent 3 must take `require_observed_owner_tenure()` for issuance and must hold, not
  substitute, on `Unknown`. `ReceiptRepair::verify_current_owner` already refuses v1 and an earlier
  tenure of the same key; V5 is the app-side obligation not to feed it a claimed value.

### 9.5 Not proposed for implementation

For a legacy-snapshot owner and an unobserved-gap owner, the only remaining evidence is what other
members witnessed. A bounded **witnessed-transition attestation** — k distinct current members each
signing `(group id, group epoch, owner device id, owner leaf key, tenure start, requester, fresh
nonce)`, accepted only on unanimous agreement — would close it. It is **not proposed for Gate 4**:
it is a new authority protocol, its guarantee is a quorum-of-witnesses property rather than a
cryptographic proof, and colluding attesters could misstate a start epoch. It is recorded here so
the reviewer can see the alternative that was considered and rejected in favour of 9.3's
local-evidence correction, which needs no new protocol and no new trust.

## 10. References, admission and budgets

- **R1.** Retained branches keep their conservative protection unchanged: the inventory's reference
  path enumerates `base_blob_cids()` plus every pending operation's CIDs, so base-only, superseded
  and removed-frame pixels survive cleanup and reopen for as long as the branch is retained.
- **R2.** Copy adds ordinary references through the destination's own operations, under
  `hold_creative_operation` and the ordinary admission and possession checks. A copy whose pixels
  are no longer possessed refuses before the destination Save, with the branch intact.
- **R3.** Disposal releases only the disposed branch's contribution, and only after the manifest is
  durable. It performs no unlink. Section 6.4 states the reclamation consequence per mode.
- **R4.** The disposal transaction obeys the same budget discipline as every other intent write:
  full replacement peak preflight, both generations invalidated on failed I/O, refusal at the vault
  cap without spending deletion credit, and an exact sync-only retry that requires no replacement
  headroom.
- **R5.** Unconfirmed branches charge the ordinary per-server content budget in addition to 8.3's
  rails. They consume no settlement or protocol reserve and create no unscanned cache.

## 11. Native results, events and the proposed UI-hooks update

Extended read (additive; every existing field and its meaning is preserved):

```ts
type OverlayInspection =
  | { v: 1; kind: "absent"; channel: Decimal; object: Hex32 | null }
  | { v: 1; kind: "local-draft"; channel: Decimal; object: Hex32 | null;
      basis: Hex64; branch: Hex64; accepted: number;
      transferState: "active" | "prepared";
      provenance: "closing" | "unconfirmed";
      eligibility: "transferable" | "manual";
      manualReason: OverlayManualReason | null;
      unconfirmedState: "awaitingSource" | "baseConfirmed" | "baseSuperseded" | null;
      copiedEntries: number;
      readOnly: true; content: StudioContent }
  | { v: 1; kind: "disposed"; channel: Decimal; object: Hex32 | null;
      basis: Hex64; branch: Hex64; accepted: number;
      mode: "copied" | "discarded"; destinationEpochId: Hex32 | null };

type OverlayManualReason =
  | "sourceMissing" | "sourceNotClosing" | "sourceReplaced" | "sourceRewound"
  | "successorNotPristine" | "successorMissing" | "receiptChanged" | "closeMissing"
  | "tenureUnknown" | "fault" | "notCurrentAuthor" | "unconfirmed" | "notReplayable";
```

Proposed rows for `FLIPNOTE-UI-HOOKS.md` (Agent 4 applies them; they are not applied here):

| UI action | Native command and invoke arguments | Result |
|---|---|---|
| Read a retained local draft | `studio_overlay_read({server, channel, object?})` | Extended `OverlayInspection` above; still read-only, still never an ordinary view |
| Show the draft's lifecycle row | `studio_overlay_lifecycle({server, channel, object?})` | Cheap structural state and reason; no content, no reconstruction |
| Back up a draft | `studio_overlay_export({server, channel, object?})` | Bounded `{format:"catcoms-studio-draft-v1", basis, branch, accepted, provenance, bytes, bytesB64}`; changes nothing and authorizes no deletion |
| Preview copying one item into current work | `studio_overlay_copy_preview({server, channel, object?, destination, choice, mode})` | One bounded proposed domain edit or an explicit hold; saves nothing |
| Apply that exact copy | `studio_overlay_copy_apply({server, channel, object?, destination, edit})` | Ordinary provisional content Save into the destination, then durable copy bookkeeping |
| Dispose of a draft | `studio_overlay_dispose({server, channel, object?, basis, branch, accepted, mode})` | Terminal disposal record; `mode:"copied"` requires a verified complete copy, `mode:"discarded"` destroys the bodies |

Truthfulness rules the rows must carry, and which the tests assert:

- `local-draft` is **local only**. It is never delivery, inclusion, settlement, receipt or another
  member's view, with or without `copiedEntries > 0`.
- `provenance:"unconfirmed"` must be displayed as unconfirmed history; it is not an installed
  source, an owner confirmation or a receipt, and `unconfirmedState:"baseConfirmed"` means two
  hashes agree, not that the preview's provider was ever owner.
- `manualReason:"tenureUnknown"` means this device cannot presently prove the current owner's
  tenure. It is not a claim that anything is wrong with the work.
- `kind:"disposed"` with `mode:"discarded"` states plainly that the bodies are gone.
- A storage refusal keeps the work unsaved and visible and never reports durable success.

Events, both reusing the existing bounded `SettlementNotices` rail and the existing
`settlement-changed` channel rather than a new one:

```rust
pub enum StudioSettlementState {
    // ... existing ...
    /// A retained local branch needs a manual decision. Read the lifecycle before labelling.
    LocalDraftManual,
    /// A retained local branch was explicitly disposed of.
    LocalDraftDisposed,
}
```

Neither is a delivery, settlement or finality claim, and both require the consumer to re-read
present state, exactly as the existing variants do. Agent 1's `LocalDraftRetained` and
`LocalDraftHandedOff` are separate and unaffected.

## 12. Crash, interruption and recovery ordering

| Interruption | Result |
|---|---|
| Any detached rebuild (export, copy plan), including cancellation | No durable byte changed; the branch, ledger, protection and permit are intact until the actual worker drops them; the stamp recheck refuses a stale result. |
| Between C4's destination Save and C5's bookkeeping | The copy is durable and unrecorded. The exact retry finds it via `contains_exact_operation`, returns `already_saved`, and re-writes C5. `Dispose { Copied }` refuses until C5 is recorded. |
| During C5's write or flush | Exact retry re-writes idempotently; a sync-only retry at capacity is possible. |
| During the disposal write, before rename | Nothing removed, nothing recorded. The branch is intact and the request is retryable verbatim. |
| After the disposal rename, before the directory flush | The exact retry reloads the authenticated record, sees `disposed`, and performs the sync-only flush that `retire_included_with_io` already implements for `removed == 0`. No second manifest, no second sequence. |
| Disposal requested while a `Prepared` branch exists | Refused by D2; Agent 1's synchronous fence or Flow R resolves the transfer first. |
| Restart with a retained unconfirmed branch and no preview | The branch reconstructs from its own seed bytes; `AwaitingSource` until an installed source exists. |
| Restart mid-copy with the destination rotated | The preview fingerprint and `epoch_id` refuse; re-preview against the new Open epoch. |

## 13. Prerequisites this design supplies to Agent 1

Against Agent 1's section 12.3:

| Agent 1 requirement | Supplied by | State |
|---|---|---|
| P1: reviewed manual lifecycle (inspect, export, copy-into-current, explicit disposition), lossless across restart and refusal | Sections 6.1-6.4, 12 | **Designed; not implemented and not reviewed.** |
| P2: every `StudioOverlayHold` variant mapped to a user-visible actionable state, including `StaleRequest`, `PixelMissing`, `ReferenceCapacity`, `InventoryUnstable` and a durably unresolvable Prepared branch | Section 7's table plus section 11's `OverlayManualReason`. `PixelMissing`, `ReferenceCapacity` and `InventoryUnstable` are transient runtime refusals rather than lifecycle states and map to the retryable native error path with their own messages; an unresolvable Prepared branch maps to `transferState:"prepared"` with `eligibility:"manual"`. | **Designed.** |
| P3: truthful native results, events and UI-hooks rows | Section 11 | **Designed.** |
| P4: a live-tenure contract for `observed_owner_tenure_start()` | Section 9.4 V1-V5, with 9.3's progress path | **Designed.** |
| P5: an explicit statement in the status note that P1-P4 are implemented and reviewed | [GATE4-AGENT-2-STATUS](GATE4-AGENT-2-STATUS.md) | **Not yet true. Native Save must stay unregistered.** |

Agent 1 must **not** register `studio_overlay_save` on the strength of this document. The status
note carries the single authoritative statement, and it currently says no.

## 14. Dependencies and integration changes for Agent 4

| File | Change | Note |
|---|---|---|
| `crates/catcoms-replication/src/studio/overlay.rs` | `StudioOverlayProvenance` on `BasisData` and its fingerprint; `provenance()` accessors | Core; own verdict line |
| `.../studio/overlay/handoff.rs`, new `overlay/disposal.rs` | v3 encoding, `disposed`/`copy` arms, extended `validate`, `dispose`, `record_copy`, `disposed_retry`, and the **provenance guard on `prepare_handoff*`** | Core; shared with Agent 1's C-1 structural split |
| `crates/catcoms-app/src/store/epoch_intents.rs` | `StudioOverlayLifecycle` and `studio_overlay_lifecycle`; the Intents-arm provenance and unconfirmed byte counters | Shared with Agent 1 (C-1, C-3) and Agent 3 |
| `.../store/epoch_intents/retirement.rs` | Unchanged behaviour; the overlay filter stays. Only the new disposal writer may remove an annotated id | Shared with Agent 3 |
| new `.../store/epoch_intents/disposal.rs`, `archive.rs` | The disposal transaction and the archive serializer | Agent 2 leaves |
| `.../store/epoch_intents/inspection.rs` | `StudioInspectionPurpose`, `rebuild_for`, extra `StudioInspectedDraft` fields; capture, stamp and `studio_inspection_is_current` unchanged | Shared with Agent 1 |
| `crates/catcoms-app/src/studio/restore.rs` | `history: &[&StudioProjection]` and `PlanScope` | Agent 2 |
| `crates/catcoms-app/src/studio/control.rs`, `dispatch.rs` | New actions and responses (5.4) | **Central enum edit; coordinate with Agent 1 and Agent 3** |
| `crates/catcoms-app/src/studio/inspection.rs` | `rebuild_for` plumbing on the existing two-visit job | Shared with Agent 1 |
| `crates/catcoms-app/src/studio/settlement.rs` | Two new `StudioSettlementState` variants | **Shared with Agent 1's two variants** |
| new `crates/catcoms-app/src/studio/{lifecycle.rs, overlay/copy.rs}` | Classification and copy driver | Agent 2 leaves |
| `crates/catcoms-sync/src/owner_tenure.rs`, `lib.rs` | `OwnerTenure::joined` and the `new_joined` call site | **Authority-bearing; own verdict line** |
| `crates/catcoms-app/src/lib.rs`, `studio.rs` | `StudioOwnerTenure` accessors (9.4) | Shared with Agents 1 and 3 |
| `apps/desktop/src-tauri/src/lib.rs`, `studio.rs`, new `studio/overlay.rs` | Five new commands, registration, security and capability rows | Agent 4 registers |
| `docs/FLIPNOTE-UI-HOOKS.md`, `INTERFACES.md`, `BACKEND-IMPLEMENTATION.md`, `GATE4-ACCEPTANCE.md` | Section 11's rows and the lifecycle contract | **Agent 4 owns; not edited here** |
| `.github/workflows/studio-overlay.yml`, `.github/scripts/` | A `lifecycle` job and a new mutation script | Agent 4 owns |

**Handed to Agent 1:** the structural decode must expose the provenance discriminant and the
branch's charged bytes (8.3); the acknowledgement classification must consult `disposed_retry`
beside `completed_retry` (6.4); Flow S's basis mint is parameterized by provenance (8.7); the two
holds of Agent 1's 12.1 are consumed exactly as specified, and 12.2's copy requirements are answered
in 6.3 C-0 and C1'.

**Handed to Agent 3:** section 9.4's `StudioOwnerTenure` seam, V1 and V5; the rule that repair must
resolve an interrupted Prepared overlay through the existing fence and must never remove an
annotated id outside the 6.4 transaction; and the fact that an `Unconfirmed` branch is not
repairable history.

## 15. Limits, costs and measurements

Nothing here is measured. Required measurements, for Index and Flipnote:

1. Export at 1, 64 and 256 accepted operations, and at the maximal record shape (5 MiB + 1024 with a
   2 MiB seed): detached rebuild time, serialized size, base64 size, and the two custody visits
   separately from the detached stage.
2. Copy preview at the same shapes, separating reconstruction from `restore::plan`.
3. The disposal transaction at 256 entries with both manifests present: encode, seal and replacement
   peak, and the sync-only exact retry at the vault cap.
4. `studio_overlay_lifecycle` on a vault with several large retained branches, to confirm O8's claim
   that classification needs no reconstruction.
5. An unconfirmed branch at its 64-operation rail with a 2 MiB seed, including the per-server and
   vault-wide counters produced by the Intents arm.

Limits:

- **L1.** Export and copy planning require full typed reconstruction, so they are unavailable for a
  structurally valid but non-replayable branch (S3). Disposal remains available.
- **L2.** Copy is per item by design. A 256-operation branch is 256 preview/apply pairs. There is no
  batch command and no batch atomicity.
- **L3.** Two full 256-entry manifests plus their headers occupy roughly 44 KiB of the 64 KiB
  metadata ceiling. A branch that has already completed a 256-entry transfer and then accumulates a
  second 256-entry branch can refuse disposal with `MetadataFull`; the branch stays retained. The
  exact figure must be measured, not assumed.
- **L4.** `Discarded` destroys the operation bodies. Only the bounded manifest survives.
- **L5.** Section 9.3's residual: a same-commit remove-and-re-add of the designated committer makes
  the rejoining owner and its witnesses disagree, refusing that owner's receipts until a later
  witnessed transition. Liveness only.
- **L6.** A legacy-snapshot owner and an unobserved-gap owner remain Unknown and cannot rotate.
  Section 9.5 is not implemented.
- **L7.** The persisted unconfirmed base is the seed checkpoint, not the previewed tail (8.2), so a
  draft operation valid only against the tail is refused at acceptance.
- **L8.** There is no import path for an exported archive.

## 16. Open questions for the reviewer

1. **Disposal and P1's recovery-before-removal (6.4).** Is `Copied` with a verified durable
   destination, plus `Discarded` with an explicit confirmation and a durable manifest, an acceptable
   reading of recovery-before-removal? Or must a full-body archive exist, accepting a sixth
   inventoried record family and its collision with Agent 1's I-4 and Agent 3's writers?
2. **The v3 record versus a separate record.** Is extending the intent extension to v3 — keeping
   v1/v2 byte-identical for existing vaults — preferable to a separate disposal record, given that a
   separate record would break the single-atomic-replacement property of 6.4?
3. **Section 9.3's inference.** Is "a device that has just joined cannot have been this group's
   committer earlier, so if it is the committer now, its tenure started now" sound as local
   evidence, and is restricting it to `new_joined` (never `unknown`, never `restore`) the right
   boundary? Is L5's residual acceptable, or must the membership policy forbid a same-commit
   remove-and-re-add of the committer?
4. **Seed-only unconfirmed base (8.2).** Is refusing operations that are valid only against the
   previewed tail acceptable, or should a bounded tail prefix (for example 64 operations / 256 KiB)
   be persisted, with the foreign-signature re-verification that implies?
5. **Unconfirmed rails (8.3).** Are 3 branches per server, 64 operations per branch and an 8 MiB
   vault-wide ceiling the right shape, or should unconfirmed work be bounded per *channel* instead?
6. **Cross-document copy (6.3).** Is Flipnote -> Flipnote within one channel the right bound for the
   copy-while-Prepared case, or should copy be restricted to the same logical document only, leaving
   Agent 1's 12.2 case unserved?

## 17. Test and mutation plan

### 17.1 Normal regressions

| # | Level | Case | Independent observation |
|---|---|---|---|
| N1 | store | Index and Flipnote export at 1, 64 and 256 operations | Archive decodes to the exact ordered entries with original authors, nonces, bodies, sequences and timestamps; the projection rebuilt from the archive equals the pre-export `StudioLocalDraft` projection including frame positions, conflicts, tombstones, attribution claims and pixel CIDs; **every durable byte of the intent, source, gate, recovery, owner and Registry records is unchanged**. |
| N2 | actor | Export across restart | The archive from a reopened vault is byte-identical to the pre-restart archive. |
| N3 | store | Export of a `Prepared` branch with no live worker | Succeeds; `prepared` is still set, the publication hold is intact, no entry is retired, no floor advanced. |
| N4 | actor | Export by a non-member and by a removed member | Refused at the existing membership check before any record read; no archive is produced. |
| N5 | store | Mixed ledger: ordinary intents plus a 3-entry branch, then export | The archive contains exactly the three annotated entries; the ordinary intents are untouched and still `NoEvidence` under `choose`. |
| N6 | actor | Same-document copy after rotation: Closing branch, document reopens Open, copy each item | Each item routes through the ordinary Save path, is authored by the copier with a fresh nonce, and appears in the destination's projection; `copy_progress` grows by one durable entry per item; the branch is unchanged throughout. |
| N7 | actor | Copy exact retry after a lost response | `already_saved` true, no second destination operation, no second `copy_progress` entry, byte-preserving sync-only persistence. |
| N8 | actor | Copy with a stale `expected_projection` or `epoch_id` | Refused; nothing saved; re-preview succeeds. |
| N9 | store | Copy admission failures: destination at `FLIPNOTE_MAX_FRAMES`, at `FLIPNOTE_FRAME_BYTES`, at `MAX_INDEX_OBJECTS`, over-cap, tombstoned target, and a missing PIX | Each yields its specific disposition or refusal; no partial destination write; the branch and its references are intact. |
| N10 | store | Cross-document copy while `Prepared` | Permitted into a genuinely distinct Flipnote in the same channel; refused when the destination is the branch's own logical document reached through a different channel label; `prepared` never cleared and nothing retired. |
| N11 | store | Disposal `Copied`, full branch | The manifest is durable, exactly the annotated ids are removed, every ordinary intent remains, and the destination operations are unchanged. |
| N12 | store | Disposal `Copied` after the destination legitimately loses an operation | `CopyIncomplete`; the branch is fully retained with every envelope and timestamp. |
| N13 | store | Disposal `Discarded` with the exact branch identity, then reopen | Manifest present with `mode:"discarded"`, entries gone, ordinary entries intact, projection of the ordinary ledger unchanged. |
| N14 | store | Disposal with a wrong `basis`, wrong `branch`, wrong `accepted`, wrong author, wrong channel, or missing `confirm_discard` | Each refuses at its own check with the branch intact; each fixture passes every earlier check first. |
| N15 | store | Disposal while a transfer hold exists, and while a live hold exists | Refused (D2); read-only export still succeeds in the transfer-hold case. |
| N16 | store | Interrupt the disposal write before rename, after rename before flush, and during flush | Reproduces section 12's table; the post-rename exact retry performs a sync-only flush with no second manifest and no second sequence. |
| N17 | store | A delayed Save retry for a disposed entry | `disposed_retry` returns the terminal disposal acknowledgement before any basis mint, tenure read, source lookup or media work; no new branch; no second envelope. |
| N18 | store | Retirement of a receipted closure that names both ordinary and disposed ids | Ordinary ids retire; disposed ids are already absent; no annotated id of a **live** branch is ever removed by that path. |
| N19 | store | Actual blob cleanup across the lifecycle | Base-only, superseded, removed-frame and pending CIDs survive cleanup and reopen while the branch is retained; after `Copied` disposal the destination keeps them referenced; after `Discarded` disposal a genuinely unreferenced CID becomes reclaimable and a still-referenced one does not. |
| N20 | store | Each `StudioOverlayManualReason` from real durable state | Source missing, Open, Settled, Fault, replaced source version, rewound below the floor, absent saved close, changed receipt head, nonpristine successor, Unknown tenure and a foreign author each produce their own reason with automatic transfer refused and the branch retained. |
| N21 | store | `studio_overlay_lifecycle` cost | No `decode_vault` reconstruction, no seed parse and no graph restore occur on the classification path (structural assertion plus a counter). |
| N22 | store | Non-replayable branch (S3) | Classification and disposal succeed; export and copy refuse at the detached rebuild; metadata readers do not fail. |
| N23 | store | v3 encoding discipline | A `Closing` branch with no disposal and no copy re-encodes **byte-identically to v2**; a v1 record still re-encodes as v1; a v3 record round-trips; unknown version, trailing bytes, a duplicate id across `active`/`completed`/`disposed`, a wrong sequence, a wrong author, a changed envelope, a `Copied` mode without matching `copy_progress`, and an `Unconfirmed` state with `prepared` set each reject. |
| N24 | store | Metadata ceiling | A branch that fits accepts; one byte over refuses with `MetadataFull` and no partial output, retaining the branch. |
| N25 | actor | Unconfirmed acceptance (8.7) for Index and Flipnote | Durable, reopens, projection and envelope order preserved; no source, gate, recovery, owner or Registry byte written; `provenance:"unconfirmed"`. |
| N26 | actor | Unconfirmed draft survives preview loss | Expire the hint, evict by capacity, replace with a newer preview, unwatch and rewatch, lock, remount, change membership, and restart: the draft, its basis, its entries and its projection are unchanged after each, and `studio_read` reports no preview. |
| N27 | actor | The preview never revives | The retained branch produces no `AwaitingTenureReceipt` result, occupies no preview slot, extends no hint lifetime and enters no ready cache. |
| N28 | store | Unconfirmed authority refusals | `prepare_handoff`, `prepare_handoff_detached`, settlement, receipt-covered retirement, replay evidence, ordinary Apply and publication each refuse an unconfirmed branch, each at its intended check, each fixture passing earlier validation. The Closing basis path still refuses an unconfirmed preview. |
| N29 | store | Tail-only operation (8.2) | An operation valid only against the previewed tail is refused at acceptance against the seed-only base, with an explicit reason and no durable change. |
| N30 | store | Unconfirmed rails | The fourth branch on a server, the 65th operation, and the vault-wide 8 MiB ceiling each refuse before acknowledgement with all existing work retained; the counters survive restart. |
| N31 | actor | Reconciliation (8.6) | With no installed source, `AwaitingSource`; after installing a source whose `doc_id` and seed change hash match, `BaseConfirmed` and copy becomes available; after installing a different source, `BaseSuperseded`. **No durable byte changes at any reclassification.** |
| N-T1 | actor | Real A -> B -> A with actual membership changes and restart between each transition | Three distinct observed starts, strictly increasing; A's second tenure differs from its first; A's first-tenure receipt is refused as current by every witness; A's first-of-second-tenure receipt inherits correctly and every later receipt repeats the inherited fields. |
| N-T2 | actor | A newcomer joining between owner changes | `Unknown`; it gains verified reading only through a fresh nonce-bound owner proof; a replayed earlier-tenure receipt with the same key is refused; it never derives a tenure from Welcome's epoch, a hint or a receipt's claim. |
| N-T3 | actor | The reproduced fixture, with 9.3 | The newcomer that becomes owner by its own join observes `Some(join epoch)`, equal to the value its witnesses observe; it can then prepare a durable owner snapshot and issue its first receipt; peers verify it. |
| N-T4 | actor | Legacy snapshot restore and an unobserved gap | Both remain `Unknown` and refuse rotation; 9.3 does not leak into `unknown` or `restore`. |
| N-T5 | actor | Hidden higher old-tenure history | History above the new tenure's inherited checkpoint enters recovery as a rewind, is preserved, and is never silently adopted. |
| N-T6 | actor | Remove then rejoin in **separate** commits, the rejoining device becoming owner | The rejoining owner and every witness observe the same start; its receipts verify. |
| N-T7 | actor | `Unknown` is fail-closed everywhere (V1) | Basis minting, first acceptance, handoff preparation, signing, commit, receipt issuance, rotation and repair issuance each refuse under `Unknown`, each at its own check. |
| N-T8 | actor | Native session, view request and instance changed between visits and after conversion, for export, copy preview and disposal | Rejected; no partial value; anything already durable stays durable. |

### 17.2 Isolated mutations

Each requires a unique anchor, one executed failing test, the intended assertion, byte-exact source
restoration and a passing restored regression.

| # | Guard removed | Test | Assertion, at the protected boundary |
|---|---|---|---|
| M1 | The `StudioOverlayState::validate` rule that no id appears in more than one of `active`, `completed`, `disposed` | N23 | "an id was both live and disposed": the branch decoded and re-encoded successfully. |
| M2 | The provenance guard in `prepare_handoff_detached` | N28 | "an unconfirmed branch produced a handoff candidate": a signing state exists. |
| M3 | D3's `branch` equality in the disposal request | N14 | "a stale request disposed a branch it had not seen": entries removed although the branch had changed since inspection. |
| M4 | D4's re-verification that each recorded destination operation is still present | N12 | "a branch was disposed while its copy was incomplete": entries removed with the destination operation absent. |
| M5 | D5's `confirm_discard` requirement | N14 | "a discard proceeded without explicit confirmation". |
| M6 | D2's transfer-hold refusal | N15 | "a Prepared branch was disposed": `prepared` cleared or entries removed under a live transfer hold. |
| M7 | The overlay filter in `retire_included_with_io` | N18 | "receipt retirement removed a live annotated id". |
| M8 | The disposal writer's restriction of the removal set to the named branch's entries | N11 | "disposal removed an ordinary intent". |
| M9 | The single-replacement property: write the manifest and the ledger removal as two writes | N16 | "entries were removed with no durable manifest" observed at the interruption between them. |
| M10 | `disposed_retry` running before basis minting in the Save classification | N17 | "a disposed retry demanded a fresh Closing basis". |
| M11 | The archive's ordered-entry serialization (emit hash order instead of saved sequence) | N1 | "the exported order differed from the authored order", with a fixture whose nonce hashes deliberately sort opposite the authored order. |
| M12 | The export path's `studio_inspection_is_current` stamp recheck | N-T8 | "an archive was delivered from a record that had changed between visits". |
| M13 | The export path's membership check | N4 | "a removed member exported a draft". |
| M14 | C4's `expected_projection`/`epoch_id` staleness fence | N8 | "a stale copy body was saved". |
| M15 | C4's `contains_exact_operation` retry shortcut | N7 | "a copy retry created a second destination operation". |
| M16 | `PlanScope::CrossDocument`'s same-channel and same-doc-type restriction | N10 | "a copy crossed into an unrelated document". |
| M17 | 8.1's requirement that no installed source exists when minting an unconfirmed basis | N28 | "a preview shadowed installed history". |
| M18 | 8.1's `tail_complete()` requirement | N25 variant | "a partially fetched preview became a durable base". |
| M19 | 8.2's seed-only admission base (validate against the merged preview projection instead) | N29 | "a tail-only operation was accepted". |
| M20 | 8.3's per-server unconfirmed branch rail, then separately the per-branch operation rail and the vault-wide byte ceiling | N30 | "an unconfirmed branch exceeded its rail": three mutations, one per bound. |
| M21 | 8.6's `doc_id` **and** seed-change-hash pair in the reconciliation predicate (keep only one) | N31 | "a superseded base reported `baseConfirmed`". |
| M22 | 9.3's `designated_committer == device` condition in `OwnerTenure::joined` (make it unconditional) | N-T2 | "a joiner that is not the committer invented the current owner's tenure": the newcomer reports `Some` where every witness reports a different value. |
| M23 | The restriction of 9.3 to `new_joined` (apply `joined` in `restore`'s legacy branch too) | N-T4 | "a legacy snapshot manufactured a tenure start". |
| M24 | V1's fail-closed `Unknown` in `require_observed_owner_tenure` (substitute the current group epoch) | N-T7 | "an Unknown-tenure device authored, signed or issued". |
| M25 | `complete_checkpoint_head_scoped`'s disagreement check between a proof's claimed tenure and local observation | N-T2 | "a claimed tenure overrode a locally observed one". |
| M26 | The `Unconfirmed`-forbids-`prepared` rule in `validate` | N23 | "an unconfirmed branch encoded a transferable state". |

Existing `studio-overlay`, `studio-handoff`, `studio-inspection` and `studio-native` mutations are
retained unchanged.

### 17.3 Harness and workflow

A new `.github/scripts/check-studio-overlay-lifecycle-mutations.py`, following
`check-studio-overlay-mutations.py`, with logs under `logs/gate4-overlay-lifecycle-*.log`. Requested
workflow patch for Agent 4: a `lifecycle` job in `.github/workflows/studio-overlay.yml` running the
focused core, store and actor suites and the mutation script, publishing its logs, added to a
required workflow. Local execution stays serial: `-j 1`, the existing per-package test debug
override, no concurrent Cargo work, no blanket cleanup.

## 18. Review request

Fill `[FULL_HEAD_SHA]` with the commit that adds this document before sending. Do not send a
placeholder.

```text
Review type: design.
Base: 1bcb1bca204d721b848b17c0835faf931ae930e3. Head: [FULL_HEAD_SHA].
Compare: https://github.com/Thalpy/Mewtual/compare/1bcb1bca204d721b848b17c0835faf931ae930e3...[FULL_HEAD_SHA]
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 1 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies: e65bfd8 is still unreviewed; Agent 1's runtime design is unaccepted and is
consumed by name only; native Save stays unregistered and out of FLIPNOTE-UI-HOOKS, and
GATE4-AGENT-2-STATUS explicitly states that Agent 1's P1-P5 are NOT yet satisfied.

Please return THREE separable verdicts, because the assignment requires them separable:

(a) The manual lifecycle, stale bases and repeated-tenure integration (sections 5-7, 9.1-9.2,
    9.4, 10-15, 17).
(b) The separately reviewed preview-local-work extension (section 8). A permanently disabled
    placeholder would not satisfy the requirement; judge the actual contract.
(c) The locally observed tenure correction in section 9.3, which changes an authority-bearing
    observation in catcoms-sync.

Challenge lossless inspection, export, copy and disposition for branches that cannot auto-hand-off.
Section 6.1 claims export and copy reuse the accepted read-only inspection capture, permit, stamp
and delivery fence literally, adding only new rebuild functions; verify that from
store/epoch_intents/inspection.rs and studio/inspection.rs rather than from the prose. Attack
mixed ordinary/annotated ledgers, full-envelope matching, explicit user action, copy admission
failures, exact retries and restart at each transition. Confirm that export, an equal projection,
a marker, an eviction acknowledgement, a membership change and a failed copy cannot discard
accepted work or label it settled, and that a removed member acquires no offline export right.

Section 6.4 is the central design decision. It makes disposal one atomic intent-record replacement
that writes a bounded manifest and removes exactly the named annotated ids, with two modes:
Copied, which re-verifies at the barrier that every recorded destination operation is still
durable, and Discarded, which destroys the bodies under an explicit confirmation. Observation O2
argues the full envelopes cannot be archived in the extension (64 KiB metadata versus 256 bodies
of up to MAX_DOMAIN_OP_BYTES) and that a sixth inventoried record family would collide with
Agent 1's I-4 and Agent 3's writers. Judge whether that satisfies P1's recovery-before-removal or
whether a full-body archive is required; question 16.1 states the alternative.

Verify the record discipline: a Closing branch with no disposal and no copy must re-encode
byte-identically to v2, v1 must still re-encode as v1, and the canonical re-encode equality and
bounds must be preserved. Attack N23's rejection list and M1, M9 and M26.

Verify actual PIX cleanup protects base-only, superseded, removed and pending references while a
branch is retained, that disposal performs no unlink, and that section 6.4's per-mode reclamation
statement is accurate against creative_references.rs and the cleanup path.

For section 8, inspect the evidence format, lifetime, quotas, complete scope binding and
reconciliation. The claims to attack: the persisted base is content-addressed (receipt bytes plus
seed bytes hashing to seed_change_hash) so the draft survives preview expiry, eviction,
replacement, unwatch, lock, remount, membership change and restart without promoting the preview
to trusted history; the volatile tail is deliberately not persisted, so a tail-only operation is
refused at acceptance; and the core provenance guard on prepare_handoff* makes an unconfirmed
branch structurally incapable of becoming signed history. Confirm a preview is never an installed
source, tenure, receipt or signing capability, that the old Closing-overlay basis still rejects an
unconfirmed preview, and that reconciliation writes nothing. Attack M17-M21 and M26.

For section 9, exercise actual A -> B -> A succession and rejoining and a newcomer with Unknown
tenure. Verify independent authority acquisition, first-receipt inheritance, exact retry with a
returning owner, stale same-key signature refusal, hidden higher old-tenure history and durable
recovery. Then judge 9.3 specifically. Its claim is that a device that has just entered a group
through Welcome was not a member at any earlier epoch and therefore cannot have been that group's
designated committer at any earlier epoch, so if it IS the committer now, its own tenure started
at the join epoch. Check that this is distinct from deriving tenure from Welcome's current epoch
for somebody else, that it is confined to ChannelSync::new_joined and never reaches
OwnerTenure::unknown or the restore path, and that decode's start > epoch rejection still holds.
Attack limit L5: a same-commit remove-and-re-add of the designated committer makes the rejoining
owner and its witnesses disagree. Say whether L5 is acceptable as a liveness residual or must be
closed. If you reject 9.3, say explicitly what the implemented legitimate progress path for a
newly joined lowest-leaf owner should be, because fail-closed Unknown with no such path is
incomplete integration and section 9.5's witnessed-attestation protocol is deliberately not
proposed for Gate 4.

Check section 11's native results and events for truthfulness: local-only versus awaiting receipt
versus stale/manual versus recovery versus storage refusal versus repeated-tenure transitions, and
that no row implies delivery, inclusion, settlement or another member's view. Check section 13's
claim about Agent 1's P1-P5 and confirm that nothing here authorizes registering studio_overlay_save.

Answer the six questions in section 16. Return PASS for each of (a), (b) and (c) separately, or
numbered findings with severity, file/line, trigger, impact, evidence and required correction,
stating which of the three boundaries each finding belongs to. A PASS accepts design only: no
implementation, no measurement and no native Save exposure is claimed, signed repair and combined
runtime integration are separate, and full Gate 4 acceptance remains with Agent 4.
```
