# Gate 4 Agent 3 status: runtime signed fault repair

Owner: Agent 3 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair)).
Proposal: [GATE4-AGENT-3-DESIGN](GATE4-AGENT-3-DESIGN.md), currently revision 2.
Review preamble: 3. Current entries override older ones.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a` | design, docs only | **REQUEST CHANGES**: AG3-DES-001 to AG3-DES-008, AG3-TEST-001; U-1 to U-6 decided |
| 2026-09-15 | Design revision 2, findings answered | `7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a` | this commit | design, docs only | re-review requested |

Working checkout: `M:\Git (local)\CatComs`, branch `Create-suite-2`, shared with the parallel Agent
1 and Agent 2 documentation passes. This pass touches only the two Agent 3 documents and commits
with explicit pathspecs. **Implementation must move to a separate branch or worktree before any
code change**; no mutation harness may run against another agent's source.

## Finding ledger

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-001 | P1 | **Answered in revision 2.** U-1 option (a) adopted with the complete contract the reviewer required: a repair-independent `conflicting_receipt_pair` checker (the repair does not exist yet when a report is validated), a report-only wire and store boundary that authenticates, bounds, validates and records through the common source fences, and an explicit admissibility rule limiting reports to a same-tenure pair in the current owner's tenure, which also removes the historical-versus-live authority confusion. Duplicate, failed-write, version and rate rules stated. | Design 5.1 C-4, 5.6 W-1, 6.5, 15.1 N26, M12 |
| AG3-DES-002 | P1 | **Answered.** Both failure paths reproduced from source. The transition is computed as a complete validated candidate on a clone, including the existing adoption mode, and committed with the gate under one lock; five explicit cases; every refusal leaves both unchanged. | Design 5.1 C-1/C-2, 15.1 N1 to N5, M1 |
| AG3-DES-003 | P1 | **Answered.** The `!receipts_conflict` predicate is withdrawn as unsound; a newer head survives only under a positive lineage justification, and the correction moved into shared core with explicit source context per U-2. | Design 5.1 C-2 case 1a, 15.1 N3b |
| AG3-DES-004 | P1 | **Answered.** Disposition, continuation and completion are now three distinct things read from the source, not from the ingest outcome; every retry row flushes; six barriers with mark-applied separated; the "Fault retained" claim after B2 withdrawn. | Design 5.1 C-5, 5.3 S-2, 6.4, 8, 15.1 N12, M8 |
| AG3-DES-005 | P1 | **Answered.** One eligibility predicate shared by serving and publication ordering, satisfied at B2/B3, never by a B1-only decision, per U-4. The deadlock case is an explicit test. | Design 5.5, 6.6, 10.2, 15.1 N27, M7 |
| AG3-DES-006 | P1 | **Answered, and the finding is stronger than stated.** `prepare_verified` makes `in_flight` irrevocable and demands exact hash equality at one closed epoch, so the journal was wedged, not merely mis-preferred. A repair-authorized `resolve_repair` inside B1 is the only replacement, with the losing bytes retained. | Design 5.1 C-7, 5.2, 6.7, 15.1 N7, N28 |
| AG3-DES-007 | P2 | **Answered.** The capability's predicate is derived by the common writer from the authenticated durable predecessor, binds storage scope and the predecessor wrapper digest, is minted only after the required save or flush returns, is reacquired after reopen, and covers the verified empty-recovery case. Existing Prepared, metadata-link and reference checks untouched. | Design 9, M2, M9 |
| AG3-DES-008 | P1 | **Answered.** Explicit capture, detached, revalidate and commit split; permit carried through queue, worker, ready result and delivery; weak admission bookkeeping; full coordinate rebinding; the mutation generation made mandatory over every repair write and possible-I/O path, not three headline writes. | Design 10.3, 11, 13.1, 15.1 N30 |
| AG3-TEST-001 | P2 | **Answered.** All ten mutants reworked at the boundaries they actually reach; M10's intended assertion was wrong and is replaced; M11 and M12 added; no useful redundant validation is deleted to manufacture a failure. | Design 15.2 |

## Audit claims corrected across revisions

| Claim | Correction |
|---|---|
| "No production path ingests a second receipt into a Studio source" (rev 1) | **Withdrawn as overstated.** `adopt_studio_checkpoint` ingests another receipt, classifies `ReceiptIngest::Fault` and crosses its own durable barrier (`store/epoch_studio/adoption.rs:89-119`). The accurate and still load-bearing statement is that its provenance is a current-owner discovery selection, so evidence flows owner-to-peer only and there is no reporter-to-owner direction. |
| "`seal_studio_epoch` has no non-test caller" (rev 1) | **Reduced to what was verified.** A grep over `crates/` and `apps/` at the base found call sites only under test modules. That is a search result, not an exhaustive reachability proof, and nothing in the design depends on it. |
| "The repair binds the full target" (rev 1, implied) | **Qualified.** The signature binds `LogicalDocument.server_id` against the MLS group id only. The local numeric server, the store scope and the Flipnote channel need the separate authenticated store and request-target checks. |
| "Preserve a newer head when it is outside the pair and does not conflict" (rev 1) | **Withdrawn as unsound.** `receipts_conflict` compares same-tenure epoch and inheritance conflicts and is not an ancestry proof; a head descending from the loser passes it and then blocks the adoption planner's `latest == receipt` precondition. |
| "A duplicate repair means the transaction is complete" (rev 1) | **Wrong.** `ReceiptRepairIngest::Duplicate` establishes only that the book's disposition happened. Recovery and installation can still be outstanding. |
| "The repair holds keep Fault after B2" (rev 1) | **Wrong.** B2 clears the fault. The truthful state is a read-only `Repairing` hold with the whole branch retained. |

## Facts established by the revision-2 audit

New this revision, verified in code at the base:

- `begin_checkpoint_adoption` sets `self.adopting = true` whenever the outcome is not `Stale` and
  the source was not already faulted (`studio/epoch/adoption.rs:75-77`), so **a fault produced by
  adoption leaves the source in adoption mode**. Any Fault exit must decide that mode explicitly.
- `ingest_adoption` is reachable with `opening == None` on a source holding epoch-zero content, so
  an adoption fault can name receipts closing an epoch unrelated to the gate's.
- `checkpoint_bytes_by_hash` calls `receipt_head()` first, which errors while faulted
  (`studio/epoch.rs:199-222`), and serves only the installed opening's exact seed: **a faulted peer
  serves no seed at all.**
- The head selector prefers `journal.pending().or(published())` for an owner and proves only on
  three-way equality with the held source receipt (`store/epoch_studio/discovery.rs:229-242`).
- `OwnerReceiptJournal::prepare_verified` rejects any different `in_flight` within a tenure and, at
  an equal `closed_epoch`, demands exact hash equality with `high_water` (`epoch.rs:1984-2008`). No
  existing API can correct a journal whose decision is a repair's loser.
- `queue_checkpoint_head` bounds the request at `MAX_QUERY + 144`, charges the global preauth rail
  before authenticating, then the per-requester rail, then decodes (`receipt_head.rs:357-418`).

Carried forward from revision 1 and unchanged: the Fault-gate restart requirement
(`epoch.rs:2352`, `epoch/adoption.rs:115`), `apply_repair`'s unconditional head overwrite
(`epoch.rs:1765-1767`), the adoption anchor predicates (`epoch/adoption.rs:37-46`, `:109-113`),
the inert wire fields (`receipt_head.rs:541`, `receipt_head/detached.rs:223-225`), the absence of
any `RecoveryReason::Repair` constructor, the `pub(crate)`/private status of
`restore_verified_from_vault` and `receipts_conflict`, and the common source fences
(`rotation.rs:121`, `adoption.rs:80`, `epoch_intents/retirement.rs:159-161`).

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-3-DESIGN.md) section 5. Summary, revision 2:

- Core: `EpochGate::commit_repair` (C-1); `ReceiptBook::plan_repair` with `RepairSource`,
  `RepairTransition`, `RepairPlan`, `RepairHold`, and `StudioEpoch`/`RegistryEpoch`
  `apply_receipt_repair` committing atomically (C-2); the repaired-loser-is-not-an-anchor
  predicates (C-3); `conflicting_receipt_pair` plus `ReceiptRepair::check_evidence` layered on it
  (C-4); `StudioRepairState`, `repair_state`, `repair_install_pending`, `fault_evidence`,
  `ReceiptBook::repair_sequence` (C-5); `prepare_repair_adoption` (C-6);
  `OwnerReceiptJournal::resolve_repair` (C-7).
- Store: `EpochRepairDecision` and the owner record's version-3 section; `prepare_epoch_repair`
  (which also reconciles the journal) and `mark_epoch_repair_applied`; `apply_studio_repair`,
  `issue_studio_repair`, `report_studio_fault`, `studio_fault_evidence`, `StudioRepairOutcome`,
  `StudioRepairHold`, `CheckedRepairRecovery`, `stage_studio_repair_recovery`, `servable_repair`;
  the Registry mirrors.
- Sync: `ReceiptHeadSelection.repair` served instead of `None`;
  `AuthenticatedCheckpointHint::repair`; `select_repaired_checkpoint`; the version-2 scoped head
  query with its optional report and raised cap. All new symbols are named `receipt_repair` or
  `fault_repair` to avoid the existing MLS delivery `repair_outbox` family.
- App: `StudioControlAction::{ReadFault, RepairFault}`,
  `StudioControlResponse::{Fault, Repaired}`, `StudioFaultView`, `StudioFaultCandidate`,
  `StudioFaultRepairRequest`, `StudioRepairStatus`, `StudioRepairBlocker`,
  `StudioSettlementState::{Repairing, StorageRefused}`, and a `catchup/repair.rs` runtime step.
- Native (designed, **not registered**): `studio_fault_read`, `studio_fault_repair`, plus
  `"repairing"` and `"storageRefused"` in the settlement mapping.

**None of these exist yet.** A proposed name is not an implementation.

## Invariants this scope must not weaken

1. **I-1** Book, gate and adoption mode change together or not at all, under the one gate lock.
2. **I-2** The losing branch is durably readable as recovery before any byte of the replacing
   source is written, enforced by a capability minted only by a returned durable save.
3. **I-3** Only the actual current designated committer, with an issuer tenure observed
   independently at this exact custody visit, authorizes a live repair. v1 and an earlier tenure of
   the same key fail closed.
4. **I-4** Both full conflicting receipts, the selected hash and the sequence match the locally held
   fault exactly; a different named pair holds.
5. **I-5** A repair retires no intent, discards no overlay branch and reduces no pending ledger.
6. **I-6** A repair is servable only after a durable local application barrier, never from a signed
   but unapplied decision; serving claims availability, never delivery.
7. **I-7** Durable disposition, unfinished continuation and terminal completion are distinct; a
   retry resumes outstanding work, preserves newer progress, reuses the same snapshot identity and
   deadline, and clears no unrelated fault.
8. **I-8** Every hold retains the complete branch and is labelled for the state actually persisted.
9. **I-9** A repaired loser is not a high-water anchor; a covered losing-baseline descendant stays
   stale without re-faulting; a genuine third baseline still faults.
10. **I-10** A report records evidence only: no winner, no adoption, no third receipt replacing a
    frozen pair, no inferred issuer tenure.
11. **I-11** An irrevocable owner decision is replaced only by a verified repair naming it as the
    loser, and its historical bytes are retained.

Inherited and not weakened: HANDOFF-002's common source-write and reference-inventory fences, the
Prepared source-replacement fence and publication hold, the four-slot shared preparation pool, the
two-retained-plus-staged recovery policy with its warning and seven-day deadline, the 48 MiB
settlement reserve and 16 MiB protocol allowance, the registry lineage ceiling, and the rule that
uncertain IO blocks the budget until full inventory reconciliation.

## Touched files

Both passes: `docs/GATE4-AGENT-3-DESIGN.md`, `docs/GATE4-AGENT-3-STATUS.md`. No production code, no
test, no shared contract document and no workflow has been changed. Planned files are in design
sections 5 and 13.3.

## Executed checks

**None, in either pass.** No Cargo, npm or script command has been run: these checkpoints change no
code, and the local machine keeps checks serial. Every number quoted in the design is a constant
read from source at the base or an explicitly labelled estimate. The maximal-shape replacement
cost, the custody time of the capture and commit stages, and the protocol-allowance arithmetic in
design section 10.1 are **unverified**.

## Dependencies

| Dependency | Owner | State | What happens without it |
|---|---|---|---|
| Design verdict on revision 2 | user / independent reviewer | requested | no implementation starts |
| Core handoff signing split `e65bfd8` | Agent 1 / core | unreviewed | unaffected: no repair path uses it |
| Live tenure contract T1 to T5 (design 13.2) | Agent 2 | design not written | repair verification holds on `Unknown`; fails closed, never substitutes |
| Prepared overlay fence and source custody (design 13.1) | Agent 1 | design revision 3, unreviewed | repair relies only on the existing `resolve_studio_handoff` and `save_studio_source_checked`; if `inventory_generation` lands, rotating it becomes mandatory over the full list in design 10.3 |
| Native registration, UI hooks, INTERFACES rows | Agent 4 | not started | commands stay unregistered and nothing is callable from the renderer |

U-1 is no longer an open dependency: the reviewer chose option (a) and revision 2 carries the
completed contract. Two smaller questions, U-7 and U-8, are open in design section 16 and block
nothing.

## Coordination summary

- **Agent 1**: this scope consumes `resolve_studio_handoff`, `save_studio_source_checked` and the
  shared preparation pool, introduces no competing writer or second pool, and asks only that the
  writer's capability-parameter shape survive so the recovery capability is a second parameter of
  the same kind. If `inventory_generation` lands, every repair write and possible-I/O path must
  rotate it.
- **Agent 2**: T1 to T5 in design 13.2, chiefly that the tenure accessor stays `Option<u64>` with
  `None` as a hold and that fault tenure and issuer tenure are never collapsed. Handed back: a
  replacement invalidates a retained overlay's Closing basis; the work stays retained and the manual
  path is Agent 2's.
- **Agent 4**: registration, the version-2 query and raised cap in INTERFACES, UI hooks, and the
  proposed `studio-repair` workflow job, per design 13.3. The five app enums this scope extends are
  listed under Proposed API seams so the merge preserves both contracts.

## Next actions

1. Push and fill the head SHA into the re-review request in
   [design section 17](GATE4-AGENT-3-DESIGN.md#17-re-review-request). Do not send it with the
   placeholder.
2. Agree the tenure seam (T1 to T5) with Agent 2 and the custody points with Agent 1 in writing
   before the first line of code.
3. Create a separate branch or worktree; do not implement on the shared documentation checkout.
4. Implement in the design's order: core C-1 to C-7 with N1 to N7, then the owner record and its
   transitions with N8 to N10, then the Studio transaction with N11 to N20, then Registry with N21,
   then W-1 and the report path with N26, then distribution with N23 to N25 and N27, then the
   runtime, control and native surface with N28 to N30, then the twelve mutants.
