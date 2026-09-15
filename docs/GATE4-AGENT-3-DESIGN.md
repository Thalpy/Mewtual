# Gate 4 Agent 3: runtime signed fault repair

Status: **revision 1, design proposal, awaiting adversarial review. No production code is written.**

Design base: `1bcb1bca204d721b848b17c0835faf931ae930e3` on branch `Create-suite-2`, which is
Agent 1's design revision 3 commit. Scope is
[Agent 3 of the four handoffs](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair);
progress is in [GATE4-AGENT-3-STATUS](GATE4-AGENT-3-STATUS.md). Review preamble 3.

**Unmet dependencies.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at `e65bfd8` is
still unreviewed; Agent 1's runtime seams and Agent 2's live-tenure contract are design proposals,
not implementations. Sections 13.1 and 13.2 state exactly what this scope needs from each and what
it does if they change. Nothing here reopens the SUC, TAIL, NATIVE, OVERLAY, HANDOFF or INSPECTION
closures.

No Cargo command was executed for this pass. Every quoted number is either a constant read from
source at the design base or an explicitly labelled estimate.

## 0. What exists, and what is missing

The assignment warns that "the codec/book already exists; it is not a runtime repair transaction".
That is exactly right, and the split is sharper than the phrase suggests.

| Capability | State at the design base | Evidence |
|---|---|---|
| `ReceiptRepair` v1/v2 record, canonical encode/decode, byte-exact re-encode check | **Exists** | `epoch.rs:1230-1445` |
| v2 tenure-separated signing (`sign_in_tenure`) and `verify_current_owner` | **Exists** | `epoch.rs:1267-1360` |
| `ReceiptBook::apply_repair` returning the losing receipt | **Exists** | `epoch.rs:1729-1776` |
| `ResolvedRepair` retention of both full receipts across checkpoint/restart | **Exists** | `epoch/repair_state.rs:8-59` |
| Repaired-loser and losing-baseline screening on every book admission path | **Exists** | `epoch/repair_state.rs:74-84`, `epoch.rs:1620`, `:1690`, `epoch/adoption.rs:30` |
| Book versions 4/5 under the unchanged 8 KiB cap, with restart validation | **Exists** | `epoch.rs:1799-1811`, `:1870-1874`, `:1920-1922` |
| Golden vector, maximal-shape and v1-cannot-authorize tests | **Exists** | `epoch/repair_state/tests.rs`, `tests/epoch_close.rs:274` |
| A gate transition **out of** `EpochPhase::Fault` | **Missing** | `epoch.rs:2522-2563` has no Fault exit; `epoch.rs:1727` says so in prose |
| A restart-valid persisted source whose fault has been repaired | **Missing** | `epoch.rs:2352` requires `book.fault` whenever the gate is `Fault` |
| Durable owner issuance state for a repair (sequence, pending, applied) | **Missing** | `store/epoch_owner.rs:29-34` holds a receipt journal and one close only |
| Any construction of `RecoveryReason::Repair` | **Missing** | zero matches for `RecoveryReason::Repair` outside the enum |
| Repair on the wire, in either direction | **Missing** | `receipt_head.rs:541` always sends `repair: None`; `receipt_head/detached.rs:223` discards any answer carrying one |
| Catch-up service of repairs by record hash | **Missing** | no `ReceiptRepair` symbol anywhere in `catcoms-sync` outside `receipt_head/wire.rs` |
| Actor scheduling, control request, native command or `repairing` event | **Missing** | `StudioSettlementState` (`studio/settlement.rs:11-21`) has no `Repairing`; no control action |
| A production path that ingests a *second* receipt into a Studio source at all | **Missing** | `seal_studio_epoch` has no non-test caller; see R6 |

So: the record format and the one-document bookkeeping are done and tested. Every part that makes a
running app enter, survive, exit and distribute a fault is absent, and two of the absences (R2, R6)
mean the existing pieces cannot even be composed without core changes.

## 1. Outcome and boundary

Conflicting owner receipts produce a visible, persistent, read-only Fault. Only the actual current
owner, with an independently observed issuer tenure, can select one of the exact conflicting
receipts and sign a `ReceiptRepair` v2. That decision is durable before it is published. Applying
it preserves the losing work before any replacement, exits Fault visibly, and lets the running app
resume. Peers and newcomers obtain the same bounded evidence and converge.

In scope: repair issuance, durable application, distribution, the visible exit, and the Registry
bucket repairs that Index and Flipnote discovery depend on.

Out of scope: local Save, automatic handoff and the preparation/commit seams (Agent 1); manual and
provisional overlay lifecycle, preview-local work and the tenure authority protocol itself
(Agent 2); shared-document edits, native registration and full-gate acceptance (Agent 4). Games,
announcements and other managed families stay unwired.

Restore and Copy are not repair. A projection that happens to match, an absent error, a receipt
hint and an acknowledged eviction are not repair. This document treats each of those as a thing the
runtime must actively refuse to confuse with repair, and section 15 tests each refusal.

## 2. What was audited

Read in full at the design base:

- `crates/catcoms-replication/src/epoch.rs` (`ReceiptRepair`, `ReceiptBook`, `EpochGate`,
  `OwnerReceiptJournal`, `RecoverySlots`), `epoch/repair_state.rs` and its tests,
  `epoch/adoption.rs` (`ingest_adoption`, `verify_adoption_state`).
- `crates/catcoms-replication/src/studio/epoch.rs`, `studio/epoch/{settlement,adoption}.rs`,
  `studio/recovery.rs`, `registry_epoch.rs` public surface.
- `crates/catcoms-app/src/store/epoch_owner.rs`, `store/epoch_recovery.rs` (action enum and the
  accounted writer), `store/epoch_studio.rs` (`seal_studio_epoch`, `checked_studio_source`,
  `save_studio_source_checked`), `store/epoch_studio/{rotation,adoption,handoff,discovery}.rs`,
  `store/epoch_intents/retirement.rs`, `store/epoch_registry/{owner,head,recovery}.rs`.
- `crates/catcoms-app/src/studio/{settlement,control,dispatch}.rs`,
  `studio/receiver/catchup/{rotation,discovery,registry_runtime}.rs`,
  `studio_exchange/{discovery,rotation}.rs`, `registry_head.rs`.
- `crates/catcoms-sync/src/receipt_head.rs`, `receipt_head/{wire,detached,service}.rs`,
  `registry_seed/detached.rs`, `owner_tenure.rs`, `epoch_service.rs`.
- `apps/desktop/src-tauri/src/studio/{settlement,recovery}.rs`.
- `docs/design-epoch-close.md` sections 7, 8, 10, 11, 12, 13; `docs/GATE4-AGENT-1-DESIGN.md`
  sections 5, 9, 12, 15; `docs/FLIPNOTE-UI-HOOKS.md` row "History fault / repair progress".

## 3. Audit observations

### R1: `apply_repair` is a struct mutation with no gate, no disk and no wire

`ReceiptBook::apply_repair` verifies authority, matches the named pair, swaps
`tenure`/`latest`/`previous_until_installed`/`fault`/`repair_sequence`/`resolved_repair`, and
returns the loser (`epoch.rs:1729-1776`). Its own doc comment at `epoch.rs:1725-1728` states that
settlement orchestration "must also move the associated gate out of `Fault`; that
recovery-dependent transition is intentionally not part of this core primitive yet."

### R2: a repaired book with a Fault gate is not a restorable source

`EpochGate::verify_restart_mode`'s Fault arm (`epoch.rs:2352-2357`) is
`book.fault.as_ref().is_some_and(...)`, and `verify_adoption_state`'s Fault arm
(`epoch/adoption.rs:115-123`) is the same shape. `StudioEpoch::restore_scoped`
(`studio/epoch.rs:589-593`) calls one of them on every restore. `apply_repair` sets
`self.fault = None`. Therefore, if a repair were applied to the book and the unit persisted while
the gate is still `Fault`, the next restore fails with `Malformed` and the whole source is
unreadable. **Nothing in the existing code can apply a repair to a persisted source without a new
gate transition.** This is the single strongest piece of evidence that no runtime exists, and it is
the reason section 5.1 starts with a gate API rather than a store API.

### R3: `apply_repair` can lower a retained head

`check_opening_receipt` (`epoch.rs:1669-1708`) records a fault between the retained `opening`
receipt and a late conflicting receipt for the same closed epoch, while explicitly keeping `latest`
intact because "latest may be the successor's sealing receipt". `apply_repair` then sets
`latest = selected` unconditionally. If the selected receipt is the opening one, a valid newer head
at `gate.epoch` is silently replaced by an older receipt and `previous_until_installed` is dropped.

The existing test `repair_exact_retry_preserves_newer_head_and_named_pair_mismatch_holds`
(`epoch/repair_state/tests.rs:189`) covers only the **Duplicate** arm: a newer receipt ingested
*after* the repair was applied. The first-application-over-a-retained-higher-head case is untested
and, on inspection, wrong. Section 5.1 corrects it in the Studio/Registry wrapper rather than by
changing accepted core semantics; section 16 U-2 asks the reviewer whether it should move into core.

### R4: a repaired loser is still treated as a high-water anchor by adoption

`ReceiptBook::ingest_adoption` (`epoch/adoption.rs:37-46`) faults whenever the incoming receipt
conflicts with `opening` or `previous_until_installed`. `verify_adoption_state`'s Closing arm
(`epoch/adoption.rs:109-113`) additionally requires `prior.closed_epoch < latest.closed_epoch` for
any anchor of the same tenure. After a repair whose loser *is* the source's opening receipt, the
selected receipt closes the **same** epoch as that anchor. Both predicates therefore reject the
only state in which a repaired rewind can legally sit. Section 5.1 C-3 fixes this with one
principle applied twice: a repaired loser stops being an anchor.

### R5: the wire field exists and is inert in both directions

`ReceiptHeadAnswer` carries `repair: Option<ReceiptRepair>` and `encode_answer`/`decode_answer`
bound and scope it (`receipt_head/wire.rs:13-17`, `:143-206`). But `serve_receipt_head_with_handoff`
hardcodes `repair: None` (`receipt_head.rs:541`), and `complete_checkpoint_hint` returns `Ok(None)`
whenever an answer carries a repair (`receipt_head/detached.rs:223-225`). `wire.rs:11` says so:
"This release sends no repair because its durable signed repair adapter is not yet implemented."
`complete_checkpoint_head_scoped` passes the whole answer up, so the receiving half needs no new
wire kind; only the serving half and the hint classifier change.

### R6: today a Studio source can only ingest receipts the current owner freshly proved

`ServerStore::seal_studio_epoch` has **no non-test caller**. The only production path that puts a
receipt into a Studio source is `adopt_studio_checkpoint`, reached from `install_studio_seed_step`
(`studio_exchange/discovery.rs:268`), which consumes a `ServerCheckpointFetch` minted only from a
verified `ReceiptHeadProof` (`receipt_head/detached.rs:172-207`). Two consequences:

1. A peer enters Fault only when the current owner proves two different receipts for one epoch to
   it (a real restore/rollback accident), or when a newly proved receipt conflicts with the
   retained `opening`. Both are reachable and both are the acceptance cases.
2. **The owner's own source never ingests a peer's receipt.** An owner whose journal was rolled
   back holds one decision; the other exists only at peers. `ReceiptBook::apply_repair` requires
   `self.fault` to be `Some`, so such an owner cannot issue a repair at all. Section 16 U-1 is the
   load-bearing open decision that follows.

### R7: Fault is already durable, visible and refusing, with no exit

`studio_owner_rotation_needed` returns false on Fault (`store/epoch_studio/rotation.rs:51`);
`maintain_registry_owner` returns `None` (`store/epoch_registry/owner.rs:49`);
`registry_runtime.rs:154-161` sets the bounded diagnostic "Registry bucket needs owner repair" and
moves to the next bucket; `StudioEpoch::receipt_head` errors in Fault (`studio/epoch.rs:199-204`),
which is what stops head service; `page_source.rs:235` refuses checkpoint service. The state is
correct and scoped. It simply never ends.

### R8: `RecoveryReason::Repair` has no producer

Zero matches for `RecoveryReason::Repair` outside its declaration. `StudioRecovery::snapshot`
(`studio/recovery.rs:127`) accepts it: only the `Rewound` case has a special epoch-zero rule.
`RecoverySnapshot::id()` covers the reason, so a Repair snapshot is a distinct, idempotent record.

### R9: the adoption transaction is the right shape for a repair rewind

`finish_studio_checkpoint_adoption_with_io` (`store/epoch_studio/adoption.rs:133-230`) already
does: validate every retained recovery slot as typed, verify its inventory record, promote a due
eviction and hold on a pending warning, stage the plan's snapshot, hold again if that warning is
now pending, then build a separate successor and write it atomically over the source. A repair
rewind differs only in the recovery reason, in how the fault is cleared first, and in the checked
token of section 9. Reusing this half is the smallest reviewable change.

### R10: the common source fences that a repair writer must honour

`save_studio_source_checked` (`store/epoch_studio.rs:518-596`) is the single Studio source writer;
it calls `check_studio_handoff_write` and installs conservative blob holds before either barrier.
`retire_included_with_io` refuses outright while `state.handoff_prepared()`
(`store/epoch_intents/retirement.rs:159-161`). Both `rotate_studio_owner` and
`adopt_studio_checkpoint` call `resolve_studio_handoff` before any journal, recovery or source side
effect (`rotation.rs:121`, `adoption.rs:80`). A repair transaction is a common source writer and
must do all three.

### R11: the app enum has no `Repairing`, the canonical design does

`design-epoch-close.md` section 13 lists `Repairing`, `HeldForStorage`, `StorageRefused` and
`AwaitingTenureReceipt` as specified but unconnected. `StudioSettlementState`
(`studio/settlement.rs:11-21`) implements seven: Open, Closing, Settled, Fault, RecoveryAvailable,
RecoveryEvictionPending, RefreshRequired. The native mapping
(`apps/desktop/src-tauri/src/studio/settlement.rs:14-17`) covers exactly those seven and its test
asserts the full set. So `Repairing` and `StorageRefused` are additions to an already-tested
contract, not new invented states.

### R12: `restore_verified_from_vault` and `receipts_conflict` are not reachable from the store

`Receipt::restore_verified_from_vault` is `pub(crate)` (`epoch.rs:1125`) and `receipts_conflict` is
a private free function (`epoch.rs:1534`). The app store therefore cannot validate a
conflicting-receipt pair at all today. Section 5.1 C-4 adds one shared public checker rather than
letting the store reimplement the predicate, so there is exactly one place to mutate in testing.

## 4. Design principles and invariants

**P1. Repair is a decision, not an inference.** No code path selects a winner. The runtime detects,
persists and displays the conflict, and refuses until an explicit current-owner action names both
receipts and the chosen one.

**P2. Nothing new is invented where an accepted transaction already has the shape.** The rewind
half reuses the accepted adoption transaction; the owner decision reuses the accepted
persist-before-publish journal record; distribution reuses the accepted receipt-head route and its
rails; recovery reuses the accepted staged-slot transaction and its eviction warning.

**P3. The persisted intermediate states are states the code already validates.** The only new
durable shapes are (a) an owner record extension and (b) a receipt book that carries a resolved
repair, which decode already supports at versions 4 and 5.

Invariants, each with a mutation in section 15.2:

- **I-1** A repair never changes a gate without the receipt book changing in the same lock, and
  never changes the book without the gate, in either direction. (Mutation M1.)
- **I-2** The losing branch is durably readable as recovery before any byte of the replacing source
  is written. Enforced by a token minted only by a returned durable recovery save. (Mutation M2.)
- **I-3** Only the actual current designated committer, whose signature key matches the record and
  whose issuer tenure start was independently observed at this exact custody visit, can authorize a
  live repair. Historical v1 evidence and an earlier tenure of the same key fail closed.
  (Mutations M3, M4.)
- **I-4** Both full conflicting receipts, the selected hash and the sequence must match the
  locally held fault exactly. A different named pair holds; it never clears the local fault and
  never substitutes a related receipt. (Mutation M5.)
- **I-5** A repair retires no intent, discards no overlay branch, and reduces no pending ledger.
  (Mutation M6.)
- **I-6** A repair is served only after the record that proves it is durably flushed, and serving
  never marks anything published or delivered. (Mutation M7.)
- **I-7** An exact repair retry is idempotent: it preserves newer progress, does not restage
  recovery under a new id, and does not clear an unrelated active fault. (Mutation M8.)
- **I-8** Every hold (unknown tenure, pair mismatch, eviction pending, insufficient space,
  uncertain IO, interrupted Prepared overlay) retains the complete branch and is visible.
  (Mutation M9.)
- **I-9** A repaired loser is not a high-water anchor, but a *third* differing receipt is still new
  equivocation and faults. (Mutation M10.)

## 5. Concrete APIs

None of these exist. A proposed name is not an implementation.

### 5.1 Core additions, `catcoms-replication`

**C-1. Gate exit.** In `epoch.rs`, on `EpochGate`:

```rust
/// Leave the read-only Fault phase under the same lock that changes the receipt book.
/// `head` is the receipt the book retains after the repair. The caller has ALREADY verified
/// current-owner authority and the exact named pair; this only re-establishes a coherent
/// phase, and refuses any shape a restart would not validate.
pub(crate) fn exit_fault_for_repair(
    &self,
    head: &Receipt,
    opening: Option<&Receipt>,
    adopting: bool,
) -> Result<EpochPhase, ReplError>;
```

Rules, chosen so that every result satisfies `verify_restart_mode` or `verify_adoption_state`
without changing either:

| Condition | New phase | `receipt_hash` |
|---|---|---|
| `adopting` is requested (rewind) | `Closing` | `Some(head.hash())` |
| `head.closed_epoch == self.epoch` | `Closing` | `Some(head.hash())` |
| `Some(head) == opening` | `Open` | `None` |
| otherwise | error `ReceiptConflict`, phase unchanged | unchanged |

`Settled` and any non-`Fault` phase error without mutating. Accepted operations, per-device
accounting and the bounded quarantine are untouched: a repair changes admission authority, not the
admitted set.

**C-2. Source-level application.** In `studio/epoch.rs` on `StudioEpoch`, and symmetrically on
`RegistryEpoch`:

```rust
pub enum StudioRepairExit {
    /// The retained source already descends from the selected receipt (or the selected receipt
    /// closes this very epoch). Fault is cleared and the source resumes in `phase`.
    Resumed { phase: EpochPhase, losing: Receipt },
    /// The source's opening receipt is the loser. The whole current version must be preserved
    /// as `RecoveryReason::Repair` and the selected checkpoint installed. `adopting` is set.
    Rewound { selected: Receipt, losing: Receipt },
    /// Exact repair already resolved and no active fault. Nothing changed.
    Duplicate { losing: Receipt },
    /// The named pair is not the pair this source holds, or the issuer tenure is unknown here.
    /// The fault is retained unchanged.
    Held(StudioRepairHold),
}

pub fn apply_receipt_repair(
    &mut self,
    repair: &ReceiptRepair,
    group: &ServerGroup,
    issuer_tenure_start: u64,
) -> Result<StudioRepairExit, ReplError>;
```

Algorithm:

1. Capture `retained_head = self.receipts.latest().cloned()` before anything mutates. (R3.)
2. `self.receipts.apply_repair(repair, group, issuer_tenure_start)`. Authority precedes the retry
   shortcut inside that call, which is already correct.
3. On `Duplicate`, return `Duplicate` without touching the gate.
4. On `Applied`, decide the head:
   - if `retained_head` is `Some(h)`, `h` is neither member of the repair's named pair, and
     `!receipts_conflict(&h, &selected)`, restore `latest = h` and the retained
     `previous_until_installed`. This is the R3 correction; the restored receipts are the exact
     bytes already held and already validated, so no reverification under a possibly changed group
     is required or performed.
   - otherwise the head is the selected receipt.
5. Classify: `Rewound` when `self.opening.as_ref() == Some(&losing)`; otherwise `Resumed`.
6. Call `exit_fault_for_repair(head, self.opening.as_ref(), adopting = matches!(Rewound))`, and on
   `Rewound` set `self.adopting = true`.
7. Return. The caller persists. A returned `Held` is a successful state observation that must be
   reported, never an error that discards evidence, exactly as `StudioAdoptionOutcome::Fault` is
   handled at `store/epoch_studio/adoption.rs:103-104`.

`StudioRepairHold` distinguishes `PairMismatch`, `SupersededFault`, `SequenceNotNewer` and
`ScopeMismatch` so the UI can say which, without leaking receipt content.

**C-3. A repaired loser is not an anchor.** Two one-predicate changes in `epoch/adoption.rs`:

- `ingest_adoption`'s anchor search becomes
  `.find(|prior| receipts_conflict(prior, &receipt) && !self.is_repaired_loser(prior))`.
- `verify_adoption_state`'s two `is_none_or` anchor predicates each gain
  `|| self.is_repaired_loser(prior)`.

Justification: `is_repaired_loser` is already the authority on which receipts a signed repair
retired, and it already screens *incoming* receipts on every path. These changes apply the same
authority to *retained anchors*, which is the only remaining place a repaired loser can still block
progress. A third differing baseline is unaffected and still faults (I-9, M10).

**C-4. One shared evidence checker.** In `epoch.rs`:

```rust
impl ReceiptRepair {
    /// Complete structural check of a named conflicting pair against this record. Signature and
    /// shape only: it asserts nothing about present membership or ownership, and mints no
    /// capability. `ResolvedRepair::verify` and every store/app validator call exactly this.
    pub fn check_evidence(&self, a: &Receipt, b: &Receipt) -> Result<(), ReplError>;
}
```

Contents are section 6.2. `ResolvedRepair::verify` is rewritten to call it, so the mutation target
is a single invariant reached by every path (the AG1-TEST-001 lesson).

**C-5. Small accessors.** `ReceiptBook::repair_sequence() -> u64` (owner counter recovery, section
6.4); `StudioEpoch::latest_repair() -> Option<&ReceiptRepair>` and the same on `RegistryEpoch`
(serving, section 5.6); `StudioEpoch::fault_evidence() -> Option<(&Receipt, &Receipt)>` (the
explicit owner decision, section 6.1).

**C-6. Recovery reason on the adoption plan.** `prepare_checkpoint_adoption` gains a private
reason parameter and a public `prepare_repair_adoption(receipt, raw_seed, group, tenure)` wrapper
that passes `RecoveryReason::Repair`. Everything else, including `adopted_successor`, is unchanged;
the successor's book is cloned from the source and therefore carries the resolved repair.

### 5.2 Store: durable owner issuance

`design-epoch-close.md` section 12 already budgets "fault evidence of 2 receipts and the latest
repair" under **owner state per logical document**. That is the designed home, so the repair
decision extends the existing per-document `.owner-receipts` record rather than creating a sixth
storage family that the five-family inventory, budget and cleanup would all have to learn.

In `store/epoch_owner.rs`:

```rust
pub struct EpochRepairDecision {
    repair: ReceiptRepair,
    a: Receipt,          // canonical ascending by hash
    b: Receipt,
    applied: bool,       // durably applied to THIS peer's source; never "delivered"
}

pub struct EpochOwnerReceiptState {
    journal: OwnerReceiptJournal,
    decision_close: Option<([u8; 32], CloseRecord)>,
    repair: Option<EpochRepairDecision>,   // new
}
```

Record encoding appends, after the existing optional `2 | hash | close` section, an optional
`3 | repair.encode() | a.encode() | b.encode() | u8 applied`. Old readers already reject any tag
other than 2 (`epoch_owner.rs:141-143`), so a v3 record fails closed for them, which is the stated
intent of the v2 extension comment. Decoding requires the sections in ascending tag order and
rejects duplicates.

`check_scope` gains: both receipts re-decode byte-exactly, both documents equal this document,
`repair.document` equals it, and `repair.check_evidence(&a, &b)` passes (C-4). A corrupt or
incoherent repair section is an error, never a silent reset, matching the existing rule that
corruption never resets an irrevocable choice.

Bounds: `MAX_RECORD_BYTES` becomes
`MAX_OWNER_RECEIPT_JOURNAL_BYTES + MAX_CLOSE_RECORD_BYTES + 3 * MAX_RECEIPT_BYTES + 1280`, about
11.4 KiB, and `MAX_SEALED_BYTES` follows. This widens the pre-parse cap for this namespace only;
the reader still refuses anything larger before allocating (`epoch_owner.rs:530-540`). These bytes
charge the protocol allowance through the existing owner `storage_record`, unchanged.

New transitions, both through the existing `update_epoch_owner_state_with_writer` so they inherit
its reload, inventory verification, reservation, seal, atomic write and commit order:

```rust
pub fn prepare_epoch_repair(
    &mut self, server: u64, repair: ReceiptRepair, a: Receipt, b: Receipt,
    group: &ServerGroup, issuer_tenure_start: u64,
    rng: &mut impl CryptoRngCore, budget: &mut EpochStorageBudget,
) -> Result<EpochOwnerReceiptState, AppError>;

pub fn mark_epoch_repair_applied(
    &mut self, server: u64, document: &LogicalDocument, repair_hash: [u8; 32],
    rng: &mut impl CryptoRngCore, budget: &mut EpochStorageBudget,
) -> Result<EpochOwnerReceiptState, AppError>;
```

`prepare_epoch_repair` refuses unless: `repair.verify_current_owner(group, issuer_tenure_start)`
passes; `check_evidence` passes; and the sequence rule of section 6.4 holds. An exact retry
(identical repair bytes and pair) re-saves and returns success, repairing a post-rename flush
failure, exactly like the receipt path. A *different* repair while an unapplied one is held is
refused: the pending decision must be resumed, never regenerated.

`mark_epoch_repair_applied` refuses a hash that is not the held pending repair, so a stale
completion cannot clear a newer decision. An exact applied retry still writes before returning.

### 5.3 Store: the Studio repair transaction

In a new leaf `store/epoch_studio/repair.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRepairOutcome {
    /// Fault cleared, source resumed. `installed` is true when no replacement was needed.
    Repaired { installed: bool },
    /// Rewind accepted and durable; the selected checkpoint's seed is still required.
    AwaitingSeed,
    /// Exact repair already applied to this source. Newer progress preserved.
    AlreadyRepaired,
    /// A recovery eviction warning holds the replacement. Work retained, fault retained.
    RecoveryPending,
    /// Storage preflight refused. Work retained, fault retained.
    StorageRefused,
    /// Named pair, sequence, scope or issuer tenure does not permit this repair here.
    Held(StudioRepairHold),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepairWrite { Journal, Source, Recovery, Successor }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepairSync { Source, Successor }

impl ServerStore {
    /// Trusted current durable-owner-snapshot or observed-tenure caller only. A UI-supplied
    /// tenure is not authority, and a raw repair passed by the renderer is not provenance.
    pub(crate) fn apply_studio_repair(
        &mut self, server: u64, group: &ServerGroup, target: StudioTarget, device: &MlsDevice,
        repair: &ReceiptRepair, issuer_tenure: u64, raw_seed: Option<&[u8]>,
        clock: &dyn Clock, rng: &mut impl CryptoRngCore, budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochStudioState), AppError>;

    /// Owner-only: sign and durably record a decision. It applies nothing and serves nothing.
    pub(crate) fn issue_studio_repair(
        &mut self, server: u64, group: &ServerGroup, target: StudioTarget, device: &MlsDevice,
        selected: [u8; 32], pair: [[u8; 32]; 2], issuer_tenure: u64,
        rng: &mut impl CryptoRngCore, budget: &mut EpochStudioBudget,
    ) -> Result<(ReceiptRepair, EpochStudioState), AppError>;

    /// Read-only fault evidence for the explicit decision surface. No mutation, no authority.
    pub(crate) fn studio_fault_evidence(
        &mut self, server: u64, group: &ServerGroup, target: StudioTarget, device: &MlsDevice,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<StudioFaultEvidence>, AppError>;
}
```

`apply_studio_repair` body, in order:

1. `current_member`, channel and document scope checks; `repair.document == target.document(...)`.
2. `repair.verify_current_owner(group, issuer_tenure)` **before** any expensive source work, in the
   same shape as `seal_studio_with_io` (`store/epoch_studio.rs:415-419`).
3. `self.resolve_studio_handoff(...)` (R10), before any journal, recovery or source side effect.
4. `checked_studio_receive_source(...)`, which enters the five-family budget, authenticates the
   actual wrapper and verifies its inventory record. Absence is an error here: a repair never
   creates a source.
5. `unit.apply_receipt_repair(repair, group, issuer_tenure)` (C-2). On `Held`, save nothing and
   return the hold with the unchanged state.
6. Save the source through `save_studio_source_reusing(..., WritePurpose::Settlement, ...)`. This
   is the barrier: Fault has now durably ended. On `Resumed` this is the last write and the
   outcome is `Repaired { installed: true }`.
7. On `Rewound` without `raw_seed`, return `AwaitingSeed`. The persisted state is the accepted
   adoption "Closing, awaiting seed" shape, now additionally carrying the resolved repair
   (book version 5).
8. On `Rewound` with `raw_seed`, run the section 9 recovery-first half and finish with the
   successor write.

`issue_studio_repair` body: read the fault evidence from the actual source under custody; refuse
unless `group.designated_committer() == device.device_id()`; refuse unless the supplied `pair`
equals the held pair's hashes and `selected` is one of them; derive `repair_sequence` per section
6.4; `ReceiptRepair::sign_in_tenure(...)`; then `prepare_epoch_repair(...)`, which is the durable
barrier. The signed bytes are returned only after that write returns. Signing alone grants no IO
authority, which `epoch.rs:1264-1265` already states.

### 5.4 Store: the Registry repair transaction

`store/epoch_registry/repair.rs` mirrors 5.3 exactly, using `RegistryEpoch::apply_receipt_repair`,
`RegistryRecovery` typed validation of every existing slot (as `stage_registry_recovery` already
does) and `RegistryRepairOutcome` reusing the same variants. It is driven only for buckets that
hold a Studio Index or Flipnote pointer; section 12.

### 5.5 Store: head selection carries a durable repair

`ReceiptHeadSelection` (`catcoms-sync/src/receipt_head.rs:157-160`) gains
`pub repair: Option<ReceiptRepair>`. `prepare_studio_head` and `prepare_registry_head` populate it
from, in order:

1. the per-document record's repair section, when present and `applied`; else
2. `unit.latest_repair()` from the saved source book.

and only after the same `reserve_sync` + `sync` flush that the `prove` branch already performs
(`store/epoch_studio/discovery.rs:244-255`). An unapplied pending repair is **not** served: a
decision that has not crossed its own application barrier locally is not yet evidence anyone should
act on. That is I-6.

### 5.6 Sync: distribution

- `serve_receipt_head_with_handoff` passes `selected.repair` into the answer instead of `None`
  (`receipt_head.rs:539-543`). The existing `encode_answer` scope check already requires
  `repair.document == document` and a 32-byte owner key (`receipt_head/wire.rs:143-157`).
- `complete_checkpoint_hint` (`receipt_head/detached.rs:216-229`) stops discarding answers that
  carry a repair. It keeps refusing answers carrying a `proof` (those need the authoritative path)
  and returns `AuthenticatedCheckpointHint { context, receipt, repair }` with a
  `repair()` accessor. A hint's repair is authenticated *member delivery*, not authority; the app
  verifies it under section 6.3 before use.
- `complete_checkpoint_head_scoped` already returns the full answer, so the Selected arm needs no
  change; the app reads `answer.repair` from `ServerCheckpointDiscovery::Hint` and from the
  completion that produced a selection.
- **Naming.** `catcoms-sync/src/lib.rs` already owns an unrelated `repair_outbox` /
  `repair_attestation_transcript` family for MLS delivery repair. Every symbol added here is named
  `receipt_repair` / `fault_repair`, never bare `repair`, and the two must not share a module.

One new bounded sync capability is required for the rewind seed, because the selected receipt is
not the owner's freshly proved head:

```rust
/// Mint a seed-fetch selection from a locally verified repair instead of a fresh owner proof.
/// Fails closed unless the SELECTED receipt itself verifies under the current owner and the
/// independently observed tenure, so a cross-tenure repair can never drive an installation.
pub fn select_repaired_checkpoint(
    &mut self,
    target: CheckpointTarget,
    receipt_repair: &ReceiptRepair,
    selected: Receipt,
) -> Result<RegistrySeedFetch, SyncError>;
```

It reuses the existing selection generation, the four retained slots, the three paced attempts and
the sixty-second lifetime. Section 6.3 explains why failing closed cross-tenure is correct rather
than a limitation to work around.

### 5.7 App: control, runtime and settlement

```rust
// studio/control.rs
pub enum StudioControlAction {
    // ... existing ...
    ReadFault,
    RepairFault(Box<StudioFaultRepairRequest>),
}
pub struct StudioFaultRepairRequest {
    pub receipt_a: [u8; 32],
    pub receipt_b: [u8; 32],
    pub selected: [u8; 32],
}
pub struct StudioFaultCandidate {
    pub receipt_hash: [u8; 32],
    pub closed_epoch: u64,
    pub close_record_hash: [u8; 32],
    pub seed_change_hash: [u8; 32],
    pub inherited_epoch: Option<u64>,
    /// True when the retained local source descends from this receipt.
    pub locally_installed: bool,
}
pub struct StudioFaultView {
    pub target: StudioTarget,
    pub source: Option<StudioSettlementSource>,
    pub candidates: [StudioFaultCandidate; 2],
    pub repair: Option<StudioRepairStatus>,
    /// Only the actual current owner with an observed issuer tenure may decide.
    pub may_decide: bool,
    pub blocked_by: Option<StudioRepairBlocker>,
    /// Operations that a rewind would move into recovery. Not a promise they are lost.
    pub preserved_operations: usize,
}
pub enum StudioControlResponse {
    // ... existing ...
    Fault(Box<StudioFaultView>),
    Repaired { target: StudioTarget, outcome: StudioRepairOutcome },
}

// studio/settlement.rs
pub enum StudioSettlementState {
    // ... existing seven ...
    Repairing,
    StorageRefused,
}
```

Runtime, in `studio/receiver/catchup/`: a new `repair.rs` step scheduled in the same slot as
`rotate_owner`, after discovery, seed and page work, so it can never pre-empt authoritative
progress. Per section 10 it holds one job per turn, rotates round-robin across faulted watched
targets, pays the existing 5 s per-target cadence and backs off to 60 s on any hold. It notes
`RefreshRequired` before the transaction and the actual observed state after it, on both the
success and error paths, exactly as `rotation.rs:66-70` does.

### 5.8 Native, designed and not registered

`apps/desktop/src-tauri/src/studio/fault.rs`:

- `studio_fault_read(server, channel, object?) -> Value`
- `studio_fault_repair(server, channel, object?, decision) -> Value`

Both go through the parent module's one custody and session path, like
`studio/recovery.rs`. `settlement.rs`'s mapping gains `S::Repairing => "repairing"` and
`S::StorageRefused => "storageRefused"`, and its existing exhaustive contract test gains both rows.
The `#[tauri::command]` attributes, the `invoke_handler` registration and the security rows land in
a separate identifiable commit owned by Agent 4, gated on section 13.3.

## 6. Selection and authorization

### 6.1 Explicit selection, no silent policy

The runtime never selects. `studio_fault_evidence` returns both candidates with exactly the fields
a person needs to choose: which epoch each closes, which close and seed each names, which inherited
baseline each carries, which one the local source descends from, and how many local operations a
rewind would move into recovery. `may_decide` is true only for the actual current owner with an
observed issuer tenure; every other caller gets the view with `may_decide: false` and a
`blocked_by` reason.

`RepairFault` echoes both receipt hashes and the chosen one. The actor re-derives the pair from the
durable source under custody and refuses if it differs, so a stale UI view cannot authorize a
decision about a pair that is no longer the live fault. The renderer supplies no receipt bytes, no
tenure, no sequence and no repair record.

There is deliberately no automatic tie-break, not even "prefer the one we installed". An honest
owner equivocated by accident; which branch is correct is a content question only a person can
answer, and the wrong answer costs a rewind.

### 6.2 Exact conflicting-receipt evidence

`ReceiptRepair::check_evidence(a, b)` requires all of:

1. `Receipt::decode(&a.encode()) == a` and the same for `b`. Public Rust fields are not a validation
   boundary; the wire decoder's exact schema is.
2. `a.verify_signature_only()` and `b.verify_signature_only()` succeed. Never
   `verify_current_owner`: the signer of a faulting pair may no longer be the committer, and after
   A -> B -> A it certainly is not the issuer.
3. `a.document == b.document == self.document`, comparing the full `LogicalDocument`: numeric
   server id bytes, doc type tag and logical key. For a Flipnote the channel is checked separately
   by the caller through `StudioTarget::channel()`, because the logical key is only the object id.
4. `a.tenure_id == b.tenure_id == self.tenure_id`.
5. `a.hash() != b.hash()` and `receipts_conflict(a, b)`, that is, same closed epoch **or** a
   differing `TenureSelection` (tenure id, tenure start group epoch, inherited checkpoint, owner
   key). Two successive consistent receipts are progress, not equivocation.
6. `self.receipt_hashes == sorted([a.hash(), b.hash()])` and
   `self.receipt_hashes.contains(&self.selected_receipt_hash)`.
7. `self.issuer_tenure_start_group_epoch.is_some()`. A v1 record fails here and can never authorize
   a live repair.
8. `self.repair_sequence > 0`.

Live authority adds, at the actual custody visit and not from a cache:
`self.issuer_tenure_start_group_epoch == Some(observed_issuer_tenure_start)`,
`observed_issuer_tenure_start <= group.epoch()`,
`group.designated_committer() == DeviceId::from_public_key_bytes(&self.owner_public_key)`,
`group.member_signature_key(&owner) == Some(&self.owner_public_key)`, and the record's own
signature. That is exactly `ReceiptRepair::verify_current_owner` (`epoch.rs:1343-1360`), which is
reused unchanged.

Application adds: `self.repair_sequence > book.repair_sequence`, and the local fault pair's hashes
equal `self.receipt_hashes`.

Rewind installation adds: the fetched seed's Automerge change hash equals
`selected.seed_change_hash`, checked by the existing `verify_checkpoint` before decoding, and the
typed channel and root validation that `prepare_checkpoint_adoption` already performs.

### 6.3 Current owner and issuer tenure

`expected_issuer_tenure_start` comes from exactly one place:
`ChannelSync::observed_owner_tenure_start()` (`owner_tenure.rs:151`), read inside the same
synchronous custody visit that uses it. It is `Option<u64>`; `None` is a **hold**, never a
substitution. The repair path never uses `group.epoch()`, never uses a receipt's carried
`tenure_start_group_epoch` and never uses the repair's own claimed field as its own evidence.

The issuance path additionally requires a `ServerOwnerSnapshot`, so the owner's MLS and tenure
observation are durable before an irrevocable decision, and the snapshot is rechecked by
`with_durable_owner_snapshot` at the moment of use (`receipt_head.rs:242-258`).

A -> B -> A has two distinct identities in one record and they must not be conflated:

- `repair.tenure_id` is the **fault tenure**, the tenure in which both conflicting receipts were
  signed. It is matched against the receipts, never against the current group.
- `repair.issuer_tenure_start_group_epoch` is the **issuer tenure start**, which must equal the
  locally observed start of the *current* owner's tenure.

Two consequences the runtime must state honestly:

1. A returning owner A cannot reuse its first tenure's repair. A v2 record signed in A's earlier
   tenure carries that tenure's start epoch, which no longer matches the observation, and
   `verify_current_owner` fails. The exact-retry shortcut inside `apply_repair` is deliberately
   placed *after* the authority check, which is already correct
   (`epoch.rs:1735-1741`); M4 mutates that ordering.
2. A **cross-tenure repair does not install anything.** When the issuer tenure differs from the
   fault tenure, the selected receipt cannot be current-owner-verified, so
   `select_repaired_checkpoint` fails closed and `from_checkpoint` could not be called anyway. Such
   a repair clears the fault, records the resolved evidence and screens the loser and its
   baseline descendants; convergence then happens through the current owner's ordinary first
   receipt for that document, whose adoption stages an ordinary `Rewound` snapshot. This is the
   designed behaviour of the v2 split, not a gap: a later owner should be able to end a stale
   read-only fault without resurrecting an old tenure's checkpoint.

A newcomer with `Unknown` tenure can verify no repair and applies none. It is not stuck: its
convergence path is the current owner's fresh proof, unchanged. A repair is an optimization for it,
not a prerequisite. Section 15 N14 asserts exactly this.

### 6.4 Retry behavior

**Owner issuance.** `repair_sequence` is strictly increasing per logical document. The next value
is `1 + max(record.repair.repair_sequence, source_book.repair_sequence())`. Seeding from the book
matters when the owner record is absent after a reinstall while peers already hold a higher applied
sequence; without it the owner would mint a sequence peers refuse. An exact retry, meaning the same
pair, the same selection and the same sequence, re-signs deterministically (the signature is over a
canonical transcript with no nonce), re-saves and returns the same bytes. A different selection for
the same pair while a repair is held unapplied is refused: the decision is irrevocable in the same
sense a receipt decision is.

**Local application.** An exact repair already applied returns `AlreadyRepaired` through
`ReceiptRepairIngest::Duplicate` without touching the gate, so newer progress after the repair
survives. If the source was already replaced, `unit.opened_by(selected)` short-circuits before any
recovery or write, mirroring `store/epoch_studio/adoption.rs:89-91`.

**Crash retry.** A rewind interrupted after recovery was staged recomputes an identical
`RecoverySnapshot`: same source bytes, same reason `Repair`, same `selecting_receipt` (the source
opening, which is the loser). `RecoverySnapshot::id()` is therefore identical, `Stage` is
idempotent and the original eviction deadline survives, which `epoch_recovery` already guarantees.

**Pacing.** Section 10.

**What a retry must never do.** It must not restage a recovery snapshot under a new id, must not
clear a *different* active fault (`apply_repair` requires the named pair to equal the held fault),
and must not lower a retained head (C-2 step 4).

## 7. The pipelines

**Flow F, entering fault.** Unchanged and already implemented: a proved receipt conflicts with the
book or with the retained opening; `adopt_studio_checkpoint` returns `StudioAdoptionOutcome::Fault`
after crossing its own source barrier; the receiver notes `RefreshRequired` then `Fault`. Head
service and rotation refuse for that document only.

**Flow I, issuance (owner).**

1. Actor turn selects a faulted watched target; `ReadFault` or the runtime reads
   `studio_fault_evidence` under custody.
2. The user chooses. `RepairFault` arrives with both hashes and the selection.
3. `issue_studio_repair`: current-owner and observed-tenure checks, pair re-derivation from the
   live source, sequence derivation, `sign_in_tenure`, then `prepare_epoch_repair`. **Barrier B1.**
4. Flow A runs immediately on the owner's own source with the freshly signed record.
5. `mark_epoch_repair_applied` after Flow A reports a terminal outcome. **Barrier B5.**
6. Only now is the repair eligible for serving (5.5, I-6).

**Flow A, application (owner and peer, identical code).**

1. `resolve_studio_handoff` (R10, section 11).
2. Checked source under the five-family budget.
3. `apply_receipt_repair` -> `Resumed` / `Rewound` / `Duplicate` / `Held`.
4. Source save. **Barrier B2.** Fault has durably ended. `Resumed` stops here.
5. `Rewound` without a seed: stop at `AwaitingSeed`; the runtime schedules
   `select_repaired_checkpoint` plus the existing paced seed fetch.
6. `Rewound` with a seed: validate all retained recovery slots as typed, verify the recovery
   inventory record, promote a due eviction, hold on a pending warning. **Barrier B3.**
7. Stage the whole-version `Repair` snapshot; hold if that now raises a warning. **Barrier B4.**
8. Build the successor with `adopted_successor` and write it atomically over the source.
   **Barrier B5.**

**Flow D, distribution.** A member's ordinary head query is answered with `{ receipt, repair,
proof? }`. The receiver authenticates the responder as a current member and the response signature
(already implemented), then verifies the repair under 6.2 and 6.3. If its own source for that
target is faulted with the named pair, it runs Flow A. A repair for a document it has no fault for
is discarded without state change.

**Flow X, visible exit.** Every transition notes an observation: `RefreshRequired` before the
transaction, `Repairing` once `apply_receipt_repair` has returned a non-`Held` exit and the source
save has returned, then the actual phase read back from the saved state, plus
`RecoveryEvictionPending` or `StorageRefused` on a hold. The UI re-reads; it never infers repair
from a Restore, an absent error or a displayed projection, because `Repairing` is produced by
exactly one call site and `StudioFaultView` is the only structure that reports a fault's candidates.

## 8. Durable ordering, crash and reopen

| Barrier | Written | Crash immediately before | Crash immediately after |
|---|---|---|---|
| B1 | owner record, repair pending, `applied: false` | no repair exists; the fault is unchanged and the owner may decide again, possibly differently | the exact decision is resumed after restart; a different selection is refused |
| B2 | source, fault cleared, resolved repair in book | source still Fault; re-apply from the pending record or a re-fetched repair; identical result | `Resumed` is complete; `Rewound` restarts at step 5 with an `adopting` source, which is the accepted adoption shape |
| B3 | recovery eviction promotion | warning still pending; the hold is re-reported | staging proceeds |
| B4 | recovery record, staged `Repair` snapshot | nothing staged; recompute gives the identical snapshot id | the losing version is durably readable; replacement may proceed |
| B5 | successor source, atomically | source is still the losing branch, with recovery already holding its copy; retry replaces it | installed; `opened_by(selected)` makes the retry inert; owner marks applied |

Reopen reads only sealed bytes. No in-memory hint, warm source or cached tenure participates in any
of these decisions after a restart. Uncertain IO invalidates the budget and requires full inventory
reconciliation before another attempt, which the existing writers already enforce; the repair adds
no bypass.

Two shapes are deliberately impossible:

- A persisted source whose gate is `Fault` but whose book has no fault. C-1 refuses to leave Fault
  except into a shape `verify_restart_mode` or `verify_adoption_state` accepts, and the book and
  gate change under the one gate lock (I-1).
- A durable repair applied to a source that still descends from the loser with no recovery copy.
  The successor write is unreachable without the section 9 token (I-2).

## 9. Recovery before replacement

Straight-line ordering is not enough evidence for this review, because a reordering mutation would
still compile and might be masked by an unrelated failure. So the replacement is gated by a value
that only a returned durable recovery save can produce:

```rust
/// Minted only here, after the recovery record's atomic write and flush have RETURNED and the
/// snapshot is retained or staged with no pending warning. Not Clone, not Copy, not durable.
pub(super) struct CheckedRepairRecovery {
    plan: [u8; 32],      // blake3 of the plan's source version and snapshot id
    snapshot: [u8; 32],
}
```

`stage_studio_repair_recovery(...) -> Result<Option<CheckedRepairRecovery>, AppError>` returns
`None` together with `RecoveryPending` when a warning holds, and `Err` when the preflight refuses.
`save_studio_source_checked` gains a `repair: Option<&CheckedRepairRecovery>` parameter alongside
the existing `handoff: Option<&CheckedHandoffWrite>` and requires it whenever the unit being written
is a repair successor, matching `plan` against the plan actually used to build that successor.
Removing the ordering therefore becomes a type error; M2 instead weakens the single condition
inside the mint, which is the isolated guard the preamble asks for.

A plan whose encoded snapshot exceeds `MAX_RECOVERY_SNAPSHOT_BYTES` (6 MiB) fails in
`StudioRecovery::snapshot` with `EpochBound`. That is reported as `StorageRefused` with the fault
and the whole branch retained. Adoption has the same property today; the repair does not add a
weaker path around it.

What is preserved, precisely:

- The whole current typed version, including deletions, conflict overflow, seed-only values and
  original insertion gaps, through the same compactor adoption uses.
- Every accepted signed operation of the source, as full author plus envelope pairs in the
  snapshot's operation list.
- Every pending intent: the repair retires **none**. `retire_included_with_io` is never called on
  any repair path (I-5).
- Any retained overlay branch: untouched. Section 11.
- Blob references: `save_studio_source_checked`'s conservative holds run before either barrier, on
  both the old and new unit, exactly as they do for adoption.

## 10. Capacity admission, publication ordering and fairness

**Admission.** Three accounted writes, all through existing reservations:

| Write | Purpose | Pool | Peak |
|---|---|---|---|
| owner record replacement | `Settlement` | protocol allowance, 16 MiB server-wide | old + new, about 11.4 KiB each at maximum |
| recovery stage | `Settlement` | settlement reserve, 48 MiB, staged slot | up to 6 MiB, plus the existing replacement peak |
| successor source | `Settlement` | settlement reserve | old + new source |

No planned deletion is credited before its IO commits; that is `EpochStorageBudget`'s existing rule
and the repair adds no exception. A reservation dropped without commit requires full reconciliation.

The resolved repair also adds about 3 KiB per repaired document to
`StudioEpoch::storage_protocol_bytes` (`studio/epoch.rs:504-518` counts the book delta). At the 16
MiB allowance that is roughly 3,300 repaired documents per server before the allowance is the
binding constraint; the registry's 256 buckets and a realistic Studio document count are far below
it. This is an arithmetic estimate, not a measurement.

**Publication ordering.** Strictly: sign -> durable decision (B1) -> durable local application
(B2, and B5 for a rewind) -> mark applied -> eligible to serve. Serving is availability only. No
step claims remote delivery, and no served answer marks a receipt journal published: the receipt
handoff completion path (`with_receipt_head_handoff`) is untouched and a repair never mints one.

**Fairness.** One repair job per actor turn per server. Faulted targets rotate round-robin using the
same `selection`-index pattern as `rotate_owner` and `registry_runtime`, so one permanently held
fault cannot monopolize the slot. The step is scheduled after discovery, seed and page work, so it
cannot pre-empt authoritative receive or another server's progress. The existing per-target 5 s
cadence applies; any hold backs off to 60 s, matching `RecoveryPending` in `advance_checkpoint`.
Seed fetches for a rewind use the existing four retained selection slots, three paced attempts and
sixty-second lifetimes; no new pool is introduced. Head queries and answers keep the existing
preauth (20), per-service (4) and per-requester (2) rails unchanged: a repair rides an answer that
was already going to be sent.

Costs to measure before the implementation review, none measured in this pass: the whole-version
`Repair` snapshot encode at a maximal accepted source (20,000 operations, 4 MiB), the successor
build, and the total custody time of one rewind turn. Section 15.3 names the harness.

## 11. Interrupted Prepared overlays and the common fences

A repair is a common source writer. It therefore:

1. Calls `resolve_studio_handoff` first, before any journal, recovery or source side effect, so an
   interrupted Prepared transaction is **resolved against actual durable evidence**, never
   overwritten. A `StudioHandoffEvidence::Hold` propagates as an error and the repair does not
   proceed; the fault and the overlay are both retained.
2. Writes only through `save_studio_source_checked`, so `check_studio_handoff_write` and the
   conservative blob holds apply unchanged.
3. Retires no intent, so `retire_included_with_io`'s `handoff_prepared()` refusal is never reached
   and never needs relaxing.
4. Preserves the complete pending ledger including overlay-annotated entries. A rewind changes the
   source the overlay's basis refers to, so the basis becomes stale. The overlay branch itself is
   retained and remains visible; the stale-basis manual path is Agent 2's, and this design adds no
   disposal, no eviction and no automatic rebase. Section 13.2 hands Agent 2 the exact condition.
5. Does not introduce a competing source writer or a second preparation pool. The runtime step
   reserves through the same `CatchupRuntime::prepare` path as rotation and discovery, which uses
   `registry_catchup::preparation_pool()`.

## 12. Registry dependencies for Index and Flipnote

A Studio target's discoverability runs through its registry bucket:
`install_registry_seed_for_studio` (`studio/receiver/catchup/discovery.rs:159`) and
`refresh_studio_registry_pointer` both operate on the bucket derived from the target's
`PointerKey`. A faulted bucket refuses page service (`page_source.rs:235`), refuses owner
maintenance (`epoch_registry/owner.rs:49`) and is skipped with a bounded diagnostic
(`registry_runtime.rs:154-161`). A newcomer therefore cannot find an Index at all while its bucket
is faulted, even though the Index document itself is healthy.

So the Registry repair is a prerequisite, not an extension. Required:

- `RegistryEpoch::apply_receipt_repair` and the `RegistryRepairOutcome` transaction of 5.4.
- `prepare_registry_head` serving a durable repair (5.5), which is how a peer's bucket learns it.
- The `registry_runtime` Fault arm gaining a repair attempt before it gives up on the bucket.

Explicitly not in scope: any other `DocType`, any bucket that holds no Studio pointer, and any
registry behaviour beyond ending a fault. The registry's `MAX_REGISTRY_EPOCH` lineage ceiling
(`epoch/adoption.rs:91-93`) is a registry product rule and stays enforced on every repaired anchor
too.

## 13. Coordination

### 13.1 Agent 1: source and commit custody

What this scope consumes, and must not weaken:

- `resolve_studio_handoff` as the first step of every repair transaction, and the Prepared source
  fence and publication hold across restart.
- `save_studio_source_checked` as the single source writer, with `check_studio_handoff_write` and
  the conservative reference holds.
- The four-slot shared `preparation_pool()`; no new pool, no uncharged cache, no worker-owned
  device or MLS secret.
- Agent 1's proposed `inventory_generation` and `epoch_mutation_guard` (its I-4): the owner record
  write, the recovery stage and the successor write are all five-family durable mutations and must
  rotate that token at the same audited choke point if it lands. If it does not land, this design
  is unaffected: it relies only on the existing `verify_record` plus reservation discipline.

What this scope asks Agent 1 to preserve: `save_studio_source_checked`'s `handoff` parameter shape,
so the parallel `repair` parameter of section 9 can be added without a third mechanism.

Conflict risk: both scopes add variants to `StudioControlAction`, `StudioControlResponse`,
`StudioSettlementState`, `StudioBackgroundJob` and the native settlement mapping. Agent 4 owns the
merge; this design adds only the five variants named in 5.7 and touches no existing one.

### 13.2 Agent 2: live tenure and stale bases

Required contract, in the exact shape this scope consumes:

- **T1.** One accessor returning `Option<u64>` for the start of the **current** owner tenure, with
  no fallback. Today that is `ChannelSync::observed_owner_tenure_start`.
- **T2.** `None` stays a hold. Neither `group.epoch()`, nor a receipt's carried
  `tenure_start_group_epoch`, nor a repair's own claimed field may ever be substituted.
- **T3.** If Agent 2 introduces a new authenticated tenure-proof protocol, it must expose the same
  `Option<u64>` shape plus a freshness binding that this scope can recheck at **every** custody
  visit: verification, application and serving. No tenure value is cached across an await or across
  a store borrow.
- **T4.** Fault tenure and issuer tenure are different values (6.3). Any tenure API must not
  collapse them.
- **T5.** Agent 2's preview path must not render or act on a receipt that the local book screens as
  a repaired loser. `is_repaired_loser` already covers the exact loser and, when inherited
  baselines differ, the losing `TenureSelection`.

Handed to Agent 2 from this scope: a rewind invalidates a retained overlay's Closing basis. The
condition is "the source's opening receipt changed and the branch was not derived from the new
one"; the work stays retained and needs Agent 2's manual path. This design performs no disposal.

### 13.3 Agent 4: integration contract

- Register `studio_fault_read` and `studio_fault_repair` plus their security rows, only after this
  scope's implementation review passes.
- Add `"repairing"` and `"storageRefused"` to the native settlement mapping and to its exhaustive
  contract test.
- `INTERFACES.md`: the receipt-head answer now carries a repair in the serving direction; record
  the bound (1 KiB inside the existing `MAX_ANSWER`) and that an unapplied pending repair is never
  served.
- `FLIPNOTE-UI-HOOKS.md`: replace the "History fault / repair progress" row per section 15.4.
- `BACKEND-IMPLEMENTATION.md` and `design-epoch-close.md` section 8: record that `Repairing` and
  the Registry repair producer are connected. **This design edits no shared contract document.**

## 14. Limits and costs

- One repair record per logical document is retained. An older, no-longer-covered conflict can
  require another repair after that bounded evidence is replaced; this is the accepted v2
  behaviour and the sequence rule makes the replacement monotone.
- A third differing receipt still faults. The repair does not confer ancestry on same-baseline
  receipts, and `is_repaired_loser` deliberately does not invent descendant relationships.
- A cross-tenure repair clears a fault but installs nothing (6.3).
- A newcomer with unknown tenure applies no repair and converges through the current owner's proof.
- A rewind whose whole-version snapshot exceeds 6 MiB is a visible `StorageRefused` hold.
- Serving a repair proves availability, never delivery.
- No measurement was taken in this pass. The maximal-shape rewind cost, the custody time of one
  repair turn, and the protocol-allowance arithmetic of section 10 are all unverified estimates.

## 15. Test and mutation plan

### 15.1 Normal regressions

Core, `catcoms-replication`:

- **N1** `exit_fault_for_repair` produces, for each of the four rows in 5.1 C-1, a unit that
  round-trips through `snapshot` and `restore` and whose restored gate, book and phase match. The
  error row leaves the gate byte-identical.
- **N2** A repaired source persisted at B2 and restored: `verify_restart_mode` accepts it, the
  resolved repair survives, and `receipt_head()` stops erroring.
- **N3** R3: fault via `check_opening_receipt` with a newer sealing receipt retained; repair
  selects the opening; the newer head survives, `previous_until_installed` survives, and the gate
  returns to `Closing` under the newer head.
- **N4** R4: a rewind whose opening is the loser restores through `verify_adoption_state`, and
  `ingest_adoption` of the selected receipt does not re-fault.
- **N5** I-9: after a repair, a third receipt with a differing baseline still faults; a receipt
  carrying the losing `TenureSelection` is `Stale`.
- **N6** `check_evidence` rejects each of the eight conditions in 6.2 individually, with an
  independent positive oracle that the otherwise identical valid pair passes.

Store, `catcoms-app`:

- **N7** Owner record v3 round-trips with journal, close and repair present; an old-format reader
  rejects it; a corrupt repair section errors and does not reset the journal; oversized,
  non-regular and copied records never reset a decision (extending the existing owner tests).
- **N8** `prepare_epoch_repair` is idempotent on an exact retry, refuses a different pending
  decision, and refuses a non-increasing sequence. `mark_epoch_repair_applied` refuses a stale hash
  and rewrites on an exact retry.
- **N9** Sequence seeding from `ReceiptBook::repair_sequence()` when the record is absent.
- **N10** Full rewind on a real store: two conflicting proved receipts produce Fault; the repair
  produces `AwaitingSeed`; the fetched seed produces `Repaired`; the losing projection is readable
  through the recovery control; **the installed source's actual projection, doc id, opening receipt
  and stored bytes** match the selected checkpoint, not merely an enum.
- **N11** `Resumed`: the confirmed case writes the source once, stages no recovery, and leaves the
  pending ledger and any overlay branch byte-identical.
- **N12** Crash at each of B1..B5 through the existing writer seams; reopen from sealed bytes;
  exact retry; assert at each point which of the two legal states holds and that no third exists.
- **N13** Both retained recovery slots full: the repair holds in `RecoveryPending` with the fault
  and the branch retained; after acknowledgement it completes; the original deadline survives.
- **N14** A newcomer with `Unknown` tenure receives the repair, applies none, stays provisional,
  and converges on the owner's fresh proof. Its existing local work survives.
- **N15** A -> B -> A: B's repair of A's first-tenure fault clears the fault and installs nothing;
  a v2 record signed in A's first tenure is refused after A returns; a v1 record is refused always.
- **N16** Mismatched named pair: the repair is held, the fault is unchanged, and an unrelated
  active fault on another document is untouched.
- **N17** Interrupted Prepared overlay on a faulted document: the repair resolves it first; a
  `Hold` evidence state refuses the repair and retains everything.
- **N18** The repair retires zero intents and leaves a mixed ordinary and overlay-annotated ledger
  intact.
- **N19** Registry bucket fault and repair; a newcomer then discovers an Index through that bucket.
  Exercised for both Index and Flipnote targets.
- **N20** `StorageRefused` at a maximal source whose whole-version snapshot exceeds 6 MiB.

Sync and app:

- **N21** A served answer carries the repair only after the record flush; an unapplied pending
  repair is never served; `encode_answer`'s scope check rejects a foreign-document repair.
- **N22** `complete_checkpoint_hint` now returns a hint carrying a repair, still refuses one
  carrying a proof, and the app discards a repair for a document it has no fault for.
- **N23** `select_repaired_checkpoint` fails closed cross-tenure and succeeds same-tenure, spending
  one retained slot and three paced attempts.
- **N24** Two real peers enter Fault from two valid conflicting receipts, the current owner repairs,
  both converge, a third peer joins after the repair, and every peer's **installed state, receipt
  evidence, recovery contents and native events** agree. Native `settlement-changed` carries
  `fault` then `repairing` then the real phase.
- **N25** Fairness: a permanently held fault on one target does not delay another target's repair,
  authoritative discovery, page receive or a second server's progress across repeated turns.

### 15.2 Isolated mutations

Each mutation removes exactly one guard, must make one named executed assertion fail, must restore
the source byte-for-byte, and the restored test must pass.

| # | Guard removed | Intended failing assertion |
|---|---|---|
| M1 | `exit_fault_for_repair` mutates the phase outside the book's commit closure | N1/N2: restore rejects a gate and book that disagree |
| M2 | `CheckedRepairRecovery` is minted without requiring the durable save to have returned | N10 variant: after a crash between B4 and B5, the losing projection is not readable from recovery |
| M3 | `verify_current_owner` in `apply_studio_repair` accepts `issuer_tenure_start_group_epoch` from the record instead of the observation | N15: a record claiming the current start is accepted |
| M4 | the exact-retry shortcut in `apply_repair` is moved before the authority check | N15: a returning owner's old-tenure retry succeeds |
| M5 | `check_evidence`'s `self.receipt_hashes == sorted(pair)` condition | N16: a different pair clears the fault |
| M6 | the assertion that the pending ledger is unchanged across a repair (a real call to retirement is introduced) | N18: intents are retired |
| M7 | the flush before a served repair | N21: a repair is served that the record does not durably hold |
| M8 | the `Duplicate` short-circuit ordering, so a retry restages recovery | N12: a second snapshot id appears and the deadline resets |
| M9 | the `RecoveryPending` hold, so replacement proceeds through a warning | N13: an old retained version is evicted without acknowledgement |
| M10 | `is_repaired_loser`'s baseline arm inside the shared checker | N5: a third differing baseline is silently accepted |

M6 is stated as an inserted call rather than a removed guard because the invariant is an absence;
the mutation makes the absence violable so the assertion can fail rather than vacuously pass.

### 15.3 Harness

New tests live in `crates/catcoms-replication/src/epoch/repair_state/tests.rs` (core), a new
`crates/catcoms-app/src/store/epoch_studio/tests/repair.rs`, a new
`crates/catcoms-app/src/store/epoch_registry/tests/repair.rs`, and
`crates/catcoms-app/src/studio_exchange/tests/repair.rs` for the two-peer actor and native
scenarios, reusing the existing deterministic actor fixtures rather than injecting private state.
Local runs stay serial with `-j 1` on this machine; platform and matrix runs go to GitHub through
the focused `studio-overlay` / `studio-handoff` workflows, with a `studio-repair` job proposed to
Agent 4 so the scenarios cannot remain opt-in. No workflow is edited by this scope.

### 15.4 Proposed UI-hooks row, for Agent 4

Replacing the current "History fault / repair progress" row once the implementation review passes:

> Actual `phase:"fault"` and `fault` invalidations are available. `studio_fault_read` returns both
> conflicting candidates, which one the local version descends from, how many operations a rewind
> preserves, and whether this client may decide. `studio_fault_repair` is available only to the
> actual current owner with an observed tenure; every other client sees `mayDecide:false` and a
> reason. `settlement-changed` adds `repairing` and `storageRefused`. Restore and Copy still save
> ordinary content in an Open target and cannot clear a fault. A repair preserves the losing
> version in recovery before any replacement, and never retires local work.

## 16. Unresolved decisions

**U-1 (blocking, needs the reviewer's design answer). How does an owner learn of a fault it does
not hold?** R6 establishes that an owner's source never ingests a peer's receipt, and
`apply_repair` requires a locally held fault. So the exact case the gate must fix, an owner whose
journal was rolled back, is one in which only peers fault and the owner cannot issue anything.
Three options:

- **(a) Recommended: carry the reporter's conflicting receipt in the head query.** Bump the scoped
  query to version 2 with an optional trailing receipt, raise the scoped query cap from 256 bytes
  to `256 + MAX_RECEIPT_BYTES`, and have the provider validate it with `check_evidence` against its
  own head. Only when the provider is the current owner and the check passes does it durably ingest
  the receipt into its own source (through the existing typed Fault-producing seal, crossing its own
  barrier) before answering. Bounded, authenticated, rate-limited by the existing head rails, no new
  protocol kind. It is still a **new wire boundary** and needs explicit approval.
- **(b)** Restrict Gate 4 issuance to faults the owner independently holds, and document that an
  owner-side rollback leaves peers faulted until a manual out-of-band step. Smallest change,
  leaves a real acceptance case unreachable.
- **(c)** Implement catch-up service of receipts and repairs by record hash, as
  `design-epoch-close.md` section 12 specifies. Largest, and duplicates (a)'s effect for this case.

**U-2. Where does the R3 head-lowering correction belong?** This design puts it in the
`StudioEpoch`/`RegistryEpoch` wrapper (5.1 C-2 step 4) to avoid changing the accepted
`ReceiptBook::apply_repair` contract. Changing core directly is cleaner and removes a way for a
future caller to get it wrong, at the cost of touching tested accepted code. Reviewer's call.

**U-3. Should the owner record hold peer-side unapplied repairs too?** This design says no: peers
persist only *applied* repairs, inside the receipt book, and a peer that crashes before B2 simply
re-fetches. The cost is that a peer which goes offline between B2 and B5 holds a staged recovery
snapshot pinning the settlement reserve until it can re-fetch. Recomputation makes this correct but
not free.

**U-4. Serving policy for a repair a peer applied but whose selected checkpoint it has not
installed.** Proposed: serve it, because the repair's value to another peer is independent of this
one's installation progress. The alternative, serving only from an installed source, is more
conservative and slower to converge.

**U-5. Third-conflict user experience.** After a repair, a third differing baseline faults again
and needs a second repair at a higher sequence, and the first repair's evidence is then replaced.
Confirm that "repair again" is the intended experience rather than an aggregated multi-pair record.

**U-6. Should `Repairing` be emitted before or after B2?** Proposed after, so the label never
claims progress that did not cross a durable barrier. The cost is that a long rewind shows `fault`
until its first write returns.

## 17. Review request

Copyable, with the common contract from
[GATE4-REVIEW-PREAMBLES](GATE4-REVIEW-PREAMBLES.md#common-contract-for-the-four-subsequent-reviews)
sent alongside it.

```text
Review type: design.
Base: 1bcb1bca204d721b848b17c0835faf931ae930e3. Head: [FULL_HEAD_SHA once pushed].
Compare: https://github.com/Thalpy/Mewtual/compare/1bcb1bca204d721b848b17c0835faf931ae930e3...[FULL_HEAD_SHA]
Scope/evidence: docs/GATE4-AGENT-3-DESIGN.md revision 1 and docs/GATE4-AGENT-3-STATUS.md.
Documentation only: no production code, test, shared contract document or workflow is changed,
and no Cargo command was executed for this pass. Every number is a source constant or a labelled
estimate.
Dependencies: repair design verdict is what this request seeks; tenure seam is Agent 2's section
13.2 contract, unimplemented; source/Prepared fence integration is Agent 1's design revision 3 at
this same base, unreviewed; the core signing split at e65bfd8 remains unreviewed.

This is the runtime signed fault repair design: issuance, durable application, serving and
receiving, and the visible exit from Fault. Section 0 separates what the existing ReceiptRepair v2
codec and ReceiptBook already do from what is missing. Please challenge that split first: in
particular R2, that applying a repair to a persisted book while the gate is Fault makes the source
unrestorable, and R6, that no production path ingests a second receipt into a Studio source, which
together decide whether the proposed core additions are necessary or avoidable.

Challenge the explicit winning-receipt decision in section 6.1 and its authorization in 6.3: only
the actual current designated committer with an independently observed issuer tenure, read at the
same custody visit, with the fault tenure and the issuer tenure kept distinct across A -> B -> A,
and with historical v1 evidence and an earlier tenure of the same key failing closed. Check that
the exact evidence in 6.2 binds both full receipts, the selected hash, the sequence and the
complete target, and that a different named pair, an earlier same-key tenure, an old repaired
loser or its descendants, newer valid progress and a third conflicting baseline each behave as
claimed. Confirm an exact retry cannot erase newer progress, restage recovery under a new id or
clear an unrelated active fault.

Trace the durable order in sections 7 to 9: owner decision, source save, recovery stage, successor
replacement, with the CheckedRepairRecovery token as the recovery-before-replacement enforcement
and the crash matrix in section 8. Check that no recovery capacity, an eviction warning or
uncertain IO each produce a visible hold with the accepted work retained, that the repair retires
no intent and discards no overlay branch, that it resolves an interrupted Prepared overlay against
actual durable evidence rather than overwriting it, and that the conservative reference holds and
source-required metadata fences of the common writer are unchanged.

Check distribution in 5.5, 5.6 and section 10: bounded authenticated receipt-head transport,
current authority and exact seed admission before any replacement, save before serve, an unapplied
pending repair never served, duplicate and replay pacing on the existing rails, job ownership and
preserved other-server progress. Section 12 states the Registry repair paths that Index and
Flipnote discovery actually depend on and why they are a prerequisite rather than an expansion.

Section 15 gives the normal regressions and ten isolated mutations, including the current-tenure
and recovery-before-replacement guards the preamble requires, each with the intended executed
assertion. Please say whether those mutations isolate what they claim or whether another failure
could mask them.

Section 16 lists six unresolved decisions. U-1 is blocking: an owner whose journal was rolled back
never sees the fault locally, so it cannot issue any repair, and the recommended fix adds an
optional receipt to the head query, which is a new wire boundary. A design verdict that does not
answer U-1 leaves the main acceptance case unreachable. U-2 asks whether the head-lowering
correction belongs in core rather than in the Studio wrapper.

Return a design verdict for this bounded repair scope, or numbered findings with concrete failure
paths and required corrections. Implementation, integration and full Gate 4 remain separate.
```
