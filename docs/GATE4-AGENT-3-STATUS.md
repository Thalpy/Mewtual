# Gate 4 Agent 3 status: runtime signed fault repair

Owner: Agent 3 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair)).
Proposal: [GATE4-AGENT-3-DESIGN](GATE4-AGENT-3-DESIGN.md), currently revision 1.
Review preamble: 3. Current entries override older ones.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `1bcb1bca204d721b848b17c0835faf931ae930e3` | this commit | design, docs only | review requested, no verdict yet |

Working checkout: `M:\Git (local)\CatComs`, branch `Create-suite-2`, shared with the parallel Agent
1 documentation pass (which committed revision 3 at `1bcb1bc` during this audit). This pass touches
only the two Agent 3 documents and commits with explicit pathspecs. **Implementation must move to a
separate branch or worktree before any code change**; no mutation harness may run against another
agent's source.

## Finding ledger

No review findings yet. This is the first Agent 3 checkpoint.

## Dependencies

| Dependency | Owner | State | What happens without it |
|---|---|---|---|
| Repair design verdict | user / independent reviewer | requested | no implementation starts |
| Core handoff signing split `e65bfd8` | Agent 1 / core | unreviewed | unaffected: no repair path uses the signing split |
| Live tenure contract T1..T5 (design 13.2) | Agent 2 | design not written | repair verification holds on `Unknown`; fails closed, never substitutes |
| Prepared overlay fence and source custody (design 13.1) | Agent 1 | design revision 3, unreviewed | repair relies only on the existing `resolve_studio_handoff` and `save_studio_source_checked` at the base |
| Native registration, UI hooks, INTERFACES rows | Agent 4 | not started | commands stay unregistered; nothing in this scope is callable from the renderer |
| U-1 wire decision (design 16) | reviewer | open, blocking | an owner whose journal was rolled back can never issue a repair, so the main acceptance case is unreachable |

## What exists versus what is missing

The one-line summary the assignment asks for: the `ReceiptRepair` v2 record, `ReceiptBook`
bookkeeping, repaired-loser screening, book versions 4/5 and their golden, maximal-shape and
v1-cannot-authorize tests all **exist and pass**. Everything that makes a running application
enter, survive, exit and distribute a fault is **missing**, and two of the gaps mean the existing
pieces cannot be composed at all without core changes. The full table is
[design section 0](GATE4-AGENT-3-DESIGN.md#0-what-exists-and-what-is-missing).

## Facts established by the revision-1 audit

Verified in code at the design base and relied on by the design:

- `EpochGate` has **no transition out of `EpochPhase::Fault`** (`epoch.rs:2522-2563`), and
  `epoch.rs:1727` says the recovery-dependent transition is deliberately absent.
- `verify_restart_mode`'s Fault arm (`epoch.rs:2352`) and `verify_adoption_state`'s Fault arm
  (`epoch/adoption.rs:115`) both require `book.fault` to be `Some`. `apply_repair` clears it. A
  repaired book persisted under a Fault gate is therefore **unrestorable**.
- `ReceiptBook::apply_repair` sets `latest = selected` and drops `previous_until_installed`
  unconditionally on `Applied` (`epoch.rs:1765-1767`), while `check_opening_receipt` deliberately
  keeps a newer sealing receipt in `latest` (`epoch.rs:1702-1704`). The existing
  "preserves newer head" test covers only the `Duplicate` arm
  (`epoch/repair_state/tests.rs:189-206`).
- `ingest_adoption` faults on any conflicting `opening` or `previous_until_installed`
  (`epoch/adoption.rs:37-46`), and `verify_adoption_state` requires
  `prior.closed_epoch < latest.closed_epoch` for a same-tenure anchor
  (`epoch/adoption.rs:109-113`). A repaired rewind's selected receipt closes the **same** epoch as
  the losing anchor, so both predicates reject the only legal repaired-rewind state.
- `serve_receipt_head_with_handoff` hardcodes `repair: None` (`receipt_head.rs:541`) and
  `complete_checkpoint_hint` discards any answer carrying a repair
  (`receipt_head/detached.rs:223-225`). `wire.rs:11` documents this. No catch-up route serves
  repairs by record hash; the only other `repair*` symbols in `catcoms-sync` belong to the
  unrelated MLS delivery repair outbox (`lib.rs:2175`, `:2350`, `:4035`).
- `ServerStore::seal_studio_epoch` has **no non-test caller**. The only production receipt-ingest
  path for a Studio source is `adopt_studio_checkpoint` through `install_studio_seed_step`, which
  needs a `ServerCheckpointFetch` minted from a verified `ReceiptHeadProof`. So a peer faults only
  on the current owner's proven equivocation, and an owner's source never ingests a peer's receipt.
- `RecoveryReason::Repair` has **zero constructors** anywhere in the workspace.
- `Receipt::restore_verified_from_vault` is `pub(crate)` (`epoch.rs:1125`) and `receipts_conflict`
  is a private free function (`epoch.rs:1534`), so the app store cannot validate a conflicting pair
  today.
- Fault already refuses rotation (`store/epoch_studio/rotation.rs:51`), registry owner maintenance
  (`store/epoch_registry/owner.rs:49`), head service (`studio/epoch.rs:199-204`) and page service
  (`store/epoch_registry/page_source.rs:235`), and is reported with a bounded diagnostic
  (`studio/receiver/catchup/registry_runtime.rs:154-161`). The state is correct and scoped; it
  simply never ends.
- `rotate_studio_owner` (`rotation.rs:121`) and `adopt_studio_checkpoint` (`adoption.rs:80`) both
  call `resolve_studio_handoff` before any journal, recovery or source side effect;
  `retire_included_with_io` refuses outright while `handoff_prepared()`
  (`epoch_intents/retirement.rs:159-161`).
- `StudioSettlementState` implements seven states and the native mapping's contract test asserts
  exactly those seven (`apps/desktop/src-tauri/src/studio/settlement.rs:22-58`). `Repairing` and
  `StorageRefused` are named in `design-epoch-close.md` section 13 but unimplemented.
- The owner record is `scope | journal | [2 | hash | close]` and old readers reject any other tag
  (`store/epoch_owner.rs:110-152`). `design-epoch-close.md` section 12 already budgets "fault
  evidence of 2 receipts and the latest repair" as owner state per logical document.

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-3-DESIGN.md) section 5. Summary:

- Core: `EpochGate::exit_fault_for_repair` (C-1); `StudioEpoch::apply_receipt_repair` and
  `RegistryEpoch::apply_receipt_repair` with `StudioRepairExit` / `StudioRepairHold` (C-2); the
  repaired-loser-is-not-an-anchor predicates in `ingest_adoption` and `verify_adoption_state`
  (C-3); `ReceiptRepair::check_evidence` as the single shared evidence checker (C-4);
  `ReceiptBook::repair_sequence`, `StudioEpoch::latest_repair`, `StudioEpoch::fault_evidence`
  (C-5); `prepare_repair_adoption` carrying `RecoveryReason::Repair` (C-6).
- Store: `EpochRepairDecision` and the owner record's version-3 section;
  `prepare_epoch_repair`, `mark_epoch_repair_applied`; `apply_studio_repair`,
  `issue_studio_repair`, `studio_fault_evidence`, `StudioRepairOutcome`, `RepairWrite`,
  `RepairSync`, `CheckedRepairRecovery`, `stage_studio_repair_recovery`; the Registry mirrors;
  `ReceiptHeadSelection.repair`.
- Sync: `ReceiptHeadSelection.repair` served instead of `None`;
  `AuthenticatedCheckpointHint::repair`; `select_repaired_checkpoint`. All new symbols are named
  `receipt_repair` / `fault_repair` to avoid the existing MLS delivery `repair_outbox` family.
- App: `StudioControlAction::{ReadFault, RepairFault}`,
  `StudioControlResponse::{Fault, Repaired}`, `StudioFaultView`, `StudioFaultCandidate`,
  `StudioFaultRepairRequest`, `StudioRepairStatus`, `StudioRepairBlocker`,
  `StudioSettlementState::{Repairing, StorageRefused}`, and a `catchup/repair.rs` runtime step.
- Native (designed, **not registered**): `studio_fault_read`, `studio_fault_repair`, plus
  `"repairing"` and `"storageRefused"` in the settlement mapping.

**None of these exist yet.** A proposed name is not an implementation.

## Invariants this scope must not weaken

1. **I-1** Gate and receipt book change together, under the one gate lock, in both directions.
2. **I-2** The losing branch is durably readable as recovery before any byte of the replacing
   source is written, enforced by a token minted only by a returned durable save.
3. **I-3** Only the actual current designated committer, with an issuer tenure observed
   independently at this exact custody visit, authorizes a live repair. v1 and an earlier tenure of
   the same key fail closed.
4. **I-4** Both full conflicting receipts, the selected hash and the sequence must match the local
   fault exactly; a different named pair holds and never clears the local fault.
5. **I-5** A repair retires no intent, discards no overlay branch and reduces no pending ledger.
6. **I-6** A repair is served only after the record proving it is durably flushed, and serving
   claims availability, never delivery, and marks no receipt journal published.
7. **I-7** An exact retry preserves newer progress, restages no recovery under a new id and clears
   no unrelated active fault.
8. **I-8** Every hold retains the complete branch and is visible.
9. **I-9** A repaired loser is not a high-water anchor; a third differing receipt is still new
   equivocation and faults.

Inherited and not weakened: HANDOFF-002's common source-write and reference-inventory fences, the
Prepared source-replacement fence and publication hold, the four-slot shared preparation pool, the
two-retained-plus-staged recovery policy with its warning and seven-day deadline, the 48 MiB
settlement reserve and 16 MiB protocol allowance, the registry lineage ceiling, and the rule that
uncertain IO blocks the budget until full inventory reconciliation.

## Touched files

This pass: `docs/GATE4-AGENT-3-DESIGN.md`, `docs/GATE4-AGENT-3-STATUS.md`. No production code, no
test, no shared contract document and no workflow has been changed. Planned files are in design
sections 5 and 13.3.

## Executed checks

**None.** No Cargo, npm or script command was run for this pass, by deliberate choice: the local
machine keeps checks serial and this checkpoint changes no code. Every number quoted in the design
is either a constant read from source at the base or an explicitly labelled estimate. In
particular, the maximal-shape rewind cost, the custody time of one repair turn and the
protocol-allowance arithmetic in design section 10 are **unverified**.

## Coordination summary

- **Agent 1**: this scope consumes `resolve_studio_handoff`, `save_studio_source_checked` and the
  shared preparation pool, and asks only that the `handoff:` parameter shape survive so the
  parallel `repair:` parameter can be added. It introduces no competing source writer and no second
  preparation pool. If Agent 1's `inventory_generation` lands, the three repair writes must rotate
  it at the same choke point.
- **Agent 2**: the live tenure contract is T1..T5 in design 13.2, chiefly that the accessor stays
  `Option<u64>` with `None` as a hold and that the fault tenure and issuer tenure are never
  collapsed. Handed back: a rewind invalidates a retained overlay's Closing basis, and the manual
  path for that stale basis is Agent 2's; this design performs no disposal.
- **Agent 4**: registration, UI hooks, INTERFACES and the proposed `studio-repair` workflow job,
  per design 13.3. Shared enums this scope adds five variants to are listed under Proposed API
  seams so the merge preserves both contracts.

## Next actions

1. Send the design review request in [design section 17](GATE4-AGENT-3-DESIGN.md#17-review-request)
   once this commit is pushed and its head SHA is filled in. Do not send it with the placeholder.
2. Obtain the reviewer's answer to U-1 before any implementation: it decides whether a new wire
   boundary is in scope, and without it the primary acceptance case is unreachable.
3. Agree the tenure seam (T1..T5) with Agent 2 and the source/commit custody points with Agent 1 in
   writing before the first line of code.
4. Create a separate branch or worktree; do not implement on the shared documentation checkout.
5. Implement in the design's order: core C-1 to C-6 with N1 to N6, then the owner record and its
   transitions with N7 to N9, then the Studio transaction with N10 to N18, then Registry with N19,
   then distribution with N21 to N23, then the runtime, control and native surface with N24 and
   N25, then the ten mutations.
