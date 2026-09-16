# Gate 4 Agent 3: runtime signed fault repair

Status: **revision 3, design proposal, awaiting re-review. No production code is written.**

Revision 1 (`7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a`) received **REQUEST CHANGES** with
AG3-DES-001 to AG3-DES-008 and AG3-TEST-001 plus decisions on U-1 to U-6; revision 2
(`63a11e1a6451c7ed373c90b0e81b59d8a748a72a`) answered those and the reviewer confirmed the
AG3-DES-003, AG3-DES-005, AG3-DES-007 and AG3-DES-008 corrections stand. Revision 2 then received
**REQUEST CHANGES** with four new blockers, AG3-DES-009 to AG3-DES-012, two non-isolating mutants
(M2, M12) and decisions on U-7 and U-8. This revision answers those and reopens nothing that
passed. Scope is
[Agent 3 of the four handoffs](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair);
progress is in [GATE4-AGENT-3-STATUS](GATE4-AGENT-3-STATUS.md). Review preamble 3.

**Unmet dependencies, unchanged.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at
`e65bfd8` is unreviewed; Agent 1's runtime seams and Agent 2's live-tenure contract are design
proposals. Sections 13.1 and 13.2 state what this scope needs and what it does without each.

No Cargo command was executed for this pass either. Every number is a source constant read at the
base or an explicitly labelled estimate.

## 0. Disposition of the revision-2 findings

| Finding | Disposition in revision 3 | Where |
|---|---|---|
| AG3-DES-009, the report contract makes cross-tenure repair unreachable | **Corrected, and the reviewer's diagnosis is confirmed at the source.** Both `ingest_verified` and `ingest_adoption` return `Fault` before considering anything else, so a repair is the **only** exit and a later owner's first receipt can never clear an old fault. Revision 2's §6.3 convergence story was wrong and is withdrawn. Reports now carry the **complete frozen pair**, are validated historically, and land in an owner-side evidence slot rather than through the live seal. Cross-tenure repair becomes a distinct `Unblocked` transition that installs nothing and lets ordinary discovery of the current owner's checkpoint converge. | 3 R14, 5.1 C-2, 5.2, 5.6 W-1, 6.3, 6.5, 15.1 N17, N26 |
| AG3-DES-010, case 2 can declare terminal success with an unfinished adoption | **Corrected.** Case 2 splits into 2a (ordinary seal mode, no replacement) and 2b (adoption mode, replacement still required), **and** the in-memory plan stops being the terminality oracle: after B2 continuation is derived exclusively from the committed `repair_state()`. A cross-tenure source can never be left claimed by `repair_install_pending()` for a checkpoint that is forbidden to install. | 5.1 C-2, C-5, 5.3 S-2, 15.1 N5 |
| AG3-DES-011, `resolve_repair` still leaves one journal shape wedged | **Corrected.** Clearing a losing `in_flight` is not enough: the head selector and `prepare_verified` both key off the retained high water, so `high_water = R(e-1)`, `in_flight = L(e)` leaves the owner serving `R(e-1)` and rejecting the next receipt as a gap. The journal gains a distinct `reconciled` canonical decision, deliberately not the publication bit. | 3 R15, 5.1 C-7, 5.2, 6.7, 15.1 N7, N28 |
| AG3-DES-012, W-1 is not a complete bounded-wire contract | **Corrected.** Both the Studio and the Registry query codecs are specified, the version tag is an unambiguous `2` with a single canonical form for "no report" (an explicit count of 0 or 2, never 1), and the parse and charge order is corrected against the real seam: the current service decodes the scoped query **before** the per-requester charge, so the report body stays an opaque length-delimited slice until after it. | 3 R16, 5.6 W-1, 12 |
| M2 non-isolating | **Corrected.** Minting on the `Err` path leaks no capability because the caller still receives `Err`. The mutant now falsely converts a failed recovery barrier into `Ok(Some(capability))`. | 15.2 M2 |
| M12 non-isolating | **Corrected.** Core independently returns `Fault`/`Stale`, so the old assertion proved nothing about the report layer. The report layer now owns a unique guard (the frozen-pair replacement refusal in the evidence slot) and the mutant targets it, with a report-boundary side-effect assertion as well. | 15.2 M12 |
| U-7 | **Reviewer's answer adopted, overriding revision 2.** Admit before answering: authenticate and charge, validate, resolve Prepared and durably record, and only then decide the response. No authoritative proof is served for a disputed receipt, and an uncertain fault write fails closed for authoritative head service. | 6.5, 6.6 |
| U-8 | **Confirmed.** No pre-B2 `Repairing`. Progress before B2 is a subordinate field inside the Fault view, not a durable settlement state. | 5.7, 6.4 |

Revision-2 corrections the reviewer confirmed and this revision does not reopen: AG3-DES-003's
positive head justification, AG3-DES-005's B2 serving rule, AG3-DES-007's scoped recovery
capability and AG3-DES-008's custody split.

## 0.0 Disposition of the revision-1 findings

| Finding | Disposition in revision 2 | Where |
|---|---|---|
| AG3-DES-001, U-1 needs a complete fault-report admission contract | **Corrected.** Option (a) adopted. A repair-independent `conflicting_receipt_pair` checker, a report-only wire and store boundary that authenticates, bounds, validates and records through the common source fences without choosing a winner, and an explicit admissibility rule (same-tenure, current-owner tenure only) that removes the historical-versus-live authority confusion. Duplicate, failure, version and rate rules stated. | 5.1 C-4, 5.6 W-1, 5.3 S-3, 6.5, 15.1 N26 |
| AG3-DES-002, C-1/C-2 do not cover existing adoption-fault states | **Corrected.** Both failure paths reproduced and classified. The transition is now computed as a complete validated candidate on a clone, including the existing adoption mode, and committed book-with-gate under one lock; every refusal leaves both unchanged. Five cases, including distant adoption and no-opening sources. | 5.1 C-1, C-2; 7 Flow A; 15.1 N1 to N5 |
| AG3-DES-003, the R3 correction can retain a head that blocks the selected rewind | **Corrected.** The `!receipts_conflict` predicate is withdrawn as unsound: it is not an ancestry proof. A newer head is preserved only under a positive justification, and the correction moves into shared core with explicit source context, per U-2. | 5.1 C-2 case 1a; 15.1 N3, N3b |
| AG3-DES-004, a duplicate repair is treated as a completed transaction | **Corrected.** Durable repair disposition, unfinished continuation and terminal completion are now three distinct things read from the source, not from the book's ingest outcome. Retries flush. The crash matrix gains a separate mark-applied barrier, and the "Fault retained" claim after B2 is withdrawn and replaced by a truthful read-only `Repairing` hold. | 5.1 C-5, 5.3 S-2, 6.4, 8, 9 |
| AG3-DES-005, the publication rules contain a selected-seed deadlock | **Corrected.** Per U-4, distribution is eligible after the durable application barrier B2 (recorded at B3), not after installation, for owners and peers alike, under one shared predicate used by both 5.5 and 10. A B1-only decision is never servable. | 5.5, 6.6, 10; 15.1 N21, N27 |
| AG3-DES-006, repair never reconciles the owner journal | **Corrected.** A new repair-authorized `OwnerReceiptJournal::resolve_repair`, performed inside the B1 write, is the only way an irrevocable decision is replaced. Historical evidence is retained in the record. The reviewer's finding is stronger than stated: the journal is not merely mis-preferred, it is wedged. | 5.1 C-7, 5.2, 6.7; 15.1 N28 |
| AG3-DES-007, `CheckedRepairRecovery` needs an enforceable scope and predicate | **Corrected.** The writer derives "this is a repair replacement" from the authenticated durable predecessor, not from the caller. The token binds storage scope and the predecessor wrapper digest, is minted only after the required save or flush returns, and is reacquired after reopen. Existing Prepared, metadata-link and reference checks are untouched. | 9 |
| AG3-DES-008, shared pool use is not bounded custody | **Corrected.** An explicit capture, detached, revalidate and commit split with the permit carried through, full coordinate rebinding at every visit, weak admission bookkeeping, and the mutation-generation contract made mandatory over every repair write and possible-I/O path. | 5.3 S-2, 10.3, 11, 13.1; 15.1 N25 |
| AG3-TEST-001, the mutation plan does not establish isolation | **Corrected.** All ten mutants reworked per the table's assessments; M10's intended assertion was wrong and is replaced, M3 to M5 are retargeted at the guards that are actually reached, and no useful redundant validation is removed to manufacture a failure. Eleven mutants now. | 15.2 |
| Audit claim: "no production path ingests a second receipt into a Studio source" | **Withdrawn as overstated.** The checkpoint-adoption path does ingest another receipt, produce Fault and durably save it; what is missing is a route by which peer-held evidence reaches the owner. The workspace-wide caller count is also reduced to what was actually verified. | 0.1, R6 |
| Audit claim: the repair signature binds the target | **Qualified.** `LogicalDocument.server_id` is checked against the MLS group id only; the local numeric server and the Flipnote channel need the separate authenticated store and request-target checks. | 6.2 |

Decisions adopted: **U-1 (a)** with the completed contract, **U-2** shared core with source context,
**U-3** no peer owner-record entry provided the saved source carries enough continuation state,
**U-4** serve after durable application for owner and peer alike, **U-5** another explicit repair at
a higher sequence, **U-6** emit `Repairing` after the durable barrier and keep it truthful
throughout unfinished work.

## 0.1 What exists, and what is missing

| Capability | State at the design base | Evidence |
|---|---|---|
| `ReceiptRepair` v1/v2 record, canonical codec, byte-exact re-encode check | **Exists** | `epoch.rs:1230-1445` |
| v2 tenure-separated signing and `verify_current_owner` | **Exists** | `epoch.rs:1267-1360` |
| `ReceiptBook::apply_repair` returning the loser, with authority before the retry shortcut | **Exists** | `epoch.rs:1729-1776` |
| `ResolvedRepair` retention of both full receipts across checkpoint and restart | **Exists** | `epoch/repair_state.rs:8-59` |
| Repaired-loser and losing-baseline screening on every book admission path | **Exists** | `epoch/repair_state.rs:74-84`, `epoch.rs:1620`, `:1690`, `epoch/adoption.rs:30` |
| Book versions 4/5 under the unchanged 8 KiB cap, with restart validation | **Exists** | `epoch.rs:1799-1811`, `:1870-1874`, `:1920-1922` |
| Golden vector, maximal-shape and v1-cannot-authorize tests | **Exists** | `epoch/repair_state/tests.rs`, `tests/epoch_close.rs:274` |
| Ingesting a second receipt into a Studio source, producing and durably saving Fault | **Exists**, through checkpoint adoption, requiring a current-owner discovery selection | `store/epoch_studio/adoption.rs:89-119`, `studio_exchange/discovery.rs:268-296` |
| A gate transition **out of** `EpochPhase::Fault` | **Missing** | `epoch.rs:2522-2563`; `epoch.rs:1727` says so in prose |
| A restart-valid persisted source whose fault has been repaired | **Missing** | `epoch.rs:2352` and `epoch/adoption.rs:115` require `book.fault` under a Fault gate |
| A route by which peer-held fault evidence reaches the owner | **Missing** | the adoption path's provenance is a current-owner proof; there is no reporter-to-owner direction |
| Any exit from Fault other than a repair | **Missing, and impossible by construction** | `epoch.rs:1615` and `epoch/adoption.rs:27-29` both return `Fault` before any other consideration, so no later receipt, from any owner or tenure, can clear it |
| An owner-side home for fault evidence it did not itself observe | **Missing** | `apply_repair` needs `self.fault`, and a healthy owner source has none; nothing else retains a pair |
| Repair-authorized replacement of an irrevocable owner decision | **Missing** | `epoch.rs:1993-2008`: `in_flight` is irrevocable within a tenure, `high_water` demands exact-hash equality at one epoch, and the next receipt must be exactly one epoch beyond that high water |
| Durable owner issuance state for a repair, with a monotone sequence | **Missing** | `store/epoch_owner.rs:29-34` holds a journal and one close |
| Any construction of `RecoveryReason::Repair` | **Missing** | zero matches outside the enum declaration |
| Repair on the wire, in either direction | **Missing** | `receipt_head.rs:541` sends `repair: None`; `receipt_head/detached.rs:223` discards answers carrying one |
| Catch-up service of repairs by record hash | **Missing** | no `ReceiptRepair` symbol in `catcoms-sync` outside `receipt_head/wire.rs` |
| Actor scheduling, control request, native command or `repairing` event | **Missing** | `studio/settlement.rs:11-21` has no `Repairing` |

So: the record format and one-document bookkeeping are done and tested. Everything that makes a
running application exit and distribute a fault is absent, and two absences (R2, R5) mean the
existing pieces cannot be composed without core changes.

## 1. Outcome and boundary

Conflicting owner receipts produce a visible, persistent, read-only Fault. Only the actual current
owner, with an independently observed issuer tenure, can select one of the exact conflicting
receipts and sign a `ReceiptRepair` v2. That decision is durable before it is published. Applying
it preserves the losing work before any replacement, exits Fault visibly, reconciles the owner's
own irrevocable decision, and lets the running app resume. Peers and newcomers obtain the same
bounded evidence and converge.

In scope: issuance, the fault-report route that makes issuance reachable, durable application,
distribution, the visible exit, and the Registry bucket repairs Index and Flipnote discovery
depend on.

Out of scope: local Save and automatic handoff (Agent 1); manual and provisional overlay lifecycle,
preview-local work and the tenure authority protocol (Agent 2); native registration and full-gate
acceptance (Agent 4). Other managed families stay unwired.

Restore and Copy are not repair. A matching projection, an absent error, a receipt hint and an
acknowledged eviction are not repair. Section 15 tests each refusal.

## 2. What was audited

Revision 1's audit list, plus for this revision:
`epoch.rs:1948-2060` (`OwnerReceiptJournal::prepare_verified`, `mark_published`),
`studio/epoch/adoption.rs:53-79` (`begin_checkpoint_adoption`'s adoption-flag assignment),
`studio/epoch.rs:199-222` (`receipt_head`, `checkpoint_bytes_by_hash`),
`store/epoch_studio/discovery.rs:210-265` (the head selector's `own_choice`/`held` rule),
`store/epoch_studio/adoption.rs:89-119` (duplicate versus installed classification),
`store/epoch_owner.rs:408-484` (the owner writer and its test failure seam),
`receipt_head.rs:347-432` (`queue_checkpoint_head`'s length bound and rate rails).

Facts established by this audit and relied on below:

- `begin_checkpoint_adoption` sets `self.adopting = true` whenever the outcome is not `Stale` and
  the source was not already faulted (`studio/epoch/adoption.rs:75-77`). **A fault produced by
  adoption therefore leaves the source in adoption mode.** This is AG3-DES-002's failure path A.
- `ingest_adoption` is reachable with `opening == None` on a source that holds content at epoch
  zero, so an adoption fault can name receipts closing an epoch unrelated to the gate's. This is
  failure path B.
- `checkpoint_bytes_by_hash` calls `receipt_head()` first, which errors while faulted
  (`studio/epoch.rs:199-204`), and serves only the installed opening's exact seed. A faulted peer
  serves no seed at all.
- The head selector prefers `journal.pending().or(published())` for an owner and proves only on
  three-way equality with the held source receipt (`store/epoch_studio/discovery.rs:229-242`).
- `OwnerReceiptJournal::prepare_verified` rejects any different `in_flight` within a tenure and,
  at an equal `closed_epoch`, demands exact hash equality with `high_water`
  (`epoch.rs:1984-2008`). A journal whose decision is the loser cannot be corrected by any existing
  API.
- `queue_checkpoint_head` bounds the request at `MAX_QUERY + 144` and charges the global preauth
  rail before authenticating, then the per-requester rail, then decodes
  (`receipt_head.rs:357-418`).

## 3. Audit observations

### R1: `apply_repair` is a struct mutation with no gate, no disk and no wire

Verified; unchanged from revision 1. Its own comment at `epoch.rs:1725-1728` says settlement
orchestration must move the gate, and that this is deliberately absent.

### R2: a repaired book with a Fault gate is not a restorable source

`verify_restart_mode`'s Fault arm (`epoch.rs:2352`) and `verify_adoption_state`'s
(`epoch/adoption.rs:115`) both require `book.fault` to be `Some`; `StudioEpoch::restore_scoped`
calls one of them on every restore (`studio/epoch.rs:589-593`); `apply_repair` clears the fault.
Persisting only the repaired book under a Fault gate yields an unreadable source.

### R3: `apply_repair` can lower a retained head, and the revision-1 predicate did not fix it

`check_opening_receipt` keeps `latest` intact because it may be the successor's sealing receipt
(`epoch.rs:1702-1704`); `apply_repair` overwrites it. Revision 1 proposed restoring the retained
head when it was outside the named pair and did not satisfy `receipts_conflict` with the selected
receipt. **The reviewer showed that predicate is unsound**: `receipts_conflict` compares same-tenure
epoch and inheritance conflicts and is not an ancestry proof, so a head descending from the loser
passes it. Section 5.1 C-2 replaces it with a positive justification.

### R4: a repaired loser is still treated as a high-water anchor by adoption

`ingest_adoption:37-46` faults on any conflicting `opening` or `previous_until_installed`;
`verify_adoption_state:109-113` requires `prior.closed_epoch < latest.closed_epoch` for a
same-tenure anchor. A repaired rewind's selected receipt closes the same epoch as the losing
anchor, so both reject the only legal repaired-rewind state.

### R5: an adoption-generated fault leaves the source in adoption mode

See section 2. Any transition out of Fault must therefore decide the source's adoption mode
explicitly, not inherit it. Revision 1 did not, which is AG3-DES-002.

### R6: peer-held fault evidence has no route to the owner

Revision 1 claimed no production path ingests a second receipt into a Studio source. That was
overstated and is withdrawn: `adopt_studio_checkpoint` ingests another receipt, classifies
`ReceiptIngest::Fault` and crosses its own durable barrier before returning
(`store/epoch_studio/adoption.rs:89-119`). The accurate statement is narrower and still
load-bearing: **that path's provenance is a current-owner discovery selection**, so evidence flows
owner-to-peer only. An owner whose journal was rolled back holds one decision, the other exists
only at peers, and `apply_repair` requires a locally held fault. Section 5.6 W-1 supplies the
missing direction.

The status document's workspace-wide "zero non-test callers" count for `seal_studio_epoch` is
reduced to what was actually verified: a grep over `crates/` and `apps/` at this base found call
sites only under test modules. That is a search result, not an exhaustive reachability proof, and
nothing in this design depends on it.

### R7: Fault is already durable, visible and refusing, with no exit

`store/epoch_studio/rotation.rs:51`, `store/epoch_registry/owner.rs:49`, `studio/epoch.rs:199-204`,
`store/epoch_registry/page_source.rs:235`, `studio/receiver/catchup/registry_runtime.rs:154-161`.

### R8: `RecoveryReason::Repair` has no producer

Zero constructors. `StudioRecovery::snapshot` accepts it; only `Rewound` has a special epoch-zero
rule (`studio/recovery.rs:127-140`). `RecoverySnapshot::id()` covers the reason.

### R9: the adoption transaction is the right shape for a repair replacement

`store/epoch_studio/adoption.rs:133-230` already validates every retained slot as typed, verifies
the recovery inventory record, promotes a due eviction, holds on a pending warning, stages, holds
again, then builds a separate successor and writes it atomically. A repair replacement differs in
the recovery reason, in clearing the fault first, and in the section 9 token.

### R10: the common source fences a repair writer must honour

`save_studio_source_checked` is the single Studio source writer and calls
`check_studio_handoff_write` with conservative blob holds before either barrier
(`store/epoch_studio.rs:518-596`); `retire_included_with_io` refuses while `handoff_prepared()`
(`store/epoch_intents/retirement.rs:159-161`); rotation and adoption both call
`resolve_studio_handoff` first (`rotation.rs:121`, `adoption.rs:80`).

### R11: the app enum has no `Repairing`, the canonical design does

`design-epoch-close.md` section 13 names `Repairing`, `HeldForStorage`, `StorageRefused` and
`AwaitingTenureReceipt` as specified but unconnected; `studio/settlement.rs:11-21` implements seven
states and the native contract test asserts exactly those seven.

### R14: Fault short-circuits every admission path, so a repair is the only exit

`ingest_verified` returns `Ok(ReceiptIngest::Fault)` on `self.fault.is_some()` **before** the
repaired-loser screen, the tenure comparison and the high-water logic (`epoch.rs:1615-1619`).
`ingest_adoption` does the same on `self.is_faulted()` before its anchor search
(`epoch/adoption.rs:27-29`). `check_opening_receipt` likewise returns early
(`epoch.rs:1687-1689`).

Consequence, and the correction of revision 2's §6.3: **no receipt from any owner in any tenure can
clear a fault.** Revision 2 claimed a cross-tenure fault would converge through "the current
owner's ordinary first receipt". That is false. A repair is the only exit, so the report path must
carry historical pairs and cross-tenure repair must actually be reachable. This is AG3-DES-009.

### R15: the journal's canonical head is its high water, not its pending decision

`prepare_verified` requires a same-tenure receipt to be exactly one epoch beyond `high_water`
(`epoch.rs:1993-1998`), and the head selector's `own_choice` is `pending().or(published())`
(`store/epoch_studio/discovery.rs:229`). Clearing a losing `in_flight` therefore returns the owner
to its **older** high water for both selection and adjacency. With `high_water = R(e-1)` and a
losing `in_flight = L(e)`, a repair selecting `S(e)` leaves the owner serving `R(e-1)`, unable to
prove the repaired source, and unable to issue at `e+1` because that reads as a gap. This is
AG3-DES-011.

### R16: the Registry query uses a different codec, and the per-requester rail is charged after decode

`encode_scoped_query` delegates `CheckpointTarget::Registry` to `encode_query`, whose framing is
`1 | doc_type | logical_key | nonce`, not the Studio channel/object framing
(`receipt_head/wire.rs:70-97`). A Registry report therefore needs its own encoding.

`queue_checkpoint_head` charges the global preauth rail and authenticates, then calls
`decode_scoped_query`, and only afterwards charges the per-requester rail
(`receipt_head.rs:357-418`). Revision 2 claimed the opposite order. This is AG3-DES-012.

### R12: the store cannot validate a conflicting pair today

`Receipt::restore_verified_from_vault` is `pub(crate)` (`epoch.rs:1125`) and `receipts_conflict` is
a private free function (`epoch.rs:1534`).

### R13: the owner journal is wedged, not merely mis-preferred

`prepare_verified` makes `in_flight` irrevocable within a tenure and requires exact hash equality
with `high_water` at an equal `closed_epoch` (`epoch.rs:1984-2008`). If the journal's decision is
the receipt a repair declares the loser, no existing API can replace it, the head selector keeps
preferring it, and `prove` can never succeed because it demands three-way equality with the held
source receipt. This strengthens AG3-DES-006 from a selection bug to a durable wedge.

## 4. Design principles and invariants

**P1. Repair is a decision, not an inference.** No code path selects a winner. The runtime detects,
records, reports and displays the conflict, and refuses until an explicit current-owner action
names both receipts and the chosen one.

**P2. Nothing new where an accepted transaction already has the shape.** The replacement half
reuses the accepted adoption transaction; the decision reuses the accepted persist-before-publish
record; distribution reuses the accepted head route and its rails; recovery reuses the accepted
staged-slot transaction and its warning.

**P3. Every persisted intermediate state is one the code already validates**, after the two anchor
corrections in C-3.

**P4. A transition is computed whole, then committed whole.** Nothing partially mutates a live unit.

Invariants, each with a mutant in 15.2:

- **I-1** Book, gate and adoption mode change together or not at all, under the one gate lock.
- **I-2** The losing branch is durably readable as recovery before any byte of the replacing source
  is written, enforced by a capability minted only by a returned durable save.
- **I-3** Only the actual current designated committer, whose key matches the record and whose
  issuer tenure start was independently observed at this exact custody visit, authorizes a live
  repair. v1 and an earlier tenure of the same key fail closed.
- **I-4** Both full conflicting receipts, the selected hash and the sequence match the locally held
  fault exactly. A different named pair holds and never clears the local fault.
- **I-5** A repair retires no intent, discards no overlay branch and reduces no pending ledger.
- **I-6** A repair is servable only after a durable local application barrier, never from a signed
  but unapplied decision, and serving claims availability, never delivery.
- **I-7** Durable repair disposition, unfinished continuation and terminal completion are distinct.
  A retry resumes outstanding work, preserves newer progress, reuses the same snapshot identity and
  deadline, and clears no unrelated fault.
- **I-8** Every hold retains the complete branch and is truthfully labelled for the state actually
  persisted.
- **I-9** A repaired loser is not a high-water anchor; a covered losing-baseline descendant stays
  stale without re-faulting; a genuine third baseline still faults.
- **I-10** A report records evidence only. It never chooses a winner, adopts a checkpoint, replaces
  a frozen pair with a third receipt or infers an issuer tenure.
- **I-11** An irrevocable owner decision is replaced only by a verified repair naming it as the
  loser, and its historical bytes are retained.

## 5. Concrete APIs

None of these exist. A proposed name is not an implementation.

### 5.1 Core additions, `catcoms-replication`

**C-1. Matched book-and-gate commit.** On `EpochGate`, in the shape `transition_verified_receipt`
already uses (`epoch.rs:2522-2563`):

```rust
/// Leave the read-only Fault phase under the same lock that swaps the receipt book. The caller
/// supplies a COMPLETE validated transition; this only checks that the current phase is the one
/// the plan was computed against and that the target shape is coherent, then commits both.
pub(crate) fn commit_repair<F>(
    &self,
    expected: EpochPhase,
    next_phase: EpochPhase,
    next_receipt_hash: Option<Hash32>,
    commit_book: F,
) -> Result<(), ReplError>
where
    F: FnOnce();
```

It refuses unless the observed phase equals `expected`, unless `next_phase` is `Open` with no
receipt hash or `Closing` with one, and unless the phase is not `Settled`. Accepted operations,
per-device accounting and the bounded quarantine are untouched: a repair changes admission
authority, not the admitted set. On any refusal neither the gate nor the book changes.

**C-2. One validated candidate transition.** On `ReceiptBook`, computed against an explicit source
context so the correction lives in shared core (U-2):

```rust
pub(crate) struct RepairSource<'a> {
    pub opening: Option<&'a Receipt>,
    pub gate_epoch: u64,
    pub phase: EpochPhase,
    pub adopting: bool,
}

pub(crate) struct RepairTransition {
    pub book: ReceiptBook,
    pub phase: EpochPhase,
    pub receipt_hash: Option<Hash32>,
    pub adopting: bool,
    /// Some means the selected checkpoint must replace this source.
    pub install: Option<Receipt>,
    pub losing: Receipt,
}

pub(crate) enum RepairPlan {
    Transition(Box<RepairTransition>),
    /// This exact repair is already the resolved disposition. Says NOTHING about whether the
    /// replacement it requires has happened; the caller reads that from the source.
    AlreadyResolved { losing: Receipt },
    Held(RepairHold),
}

pub(crate) fn plan_repair(
    &self,
    repair: &ReceiptRepair,
    a: &Receipt,
    b: &Receipt,
    group: &ServerGroup,
    issuer_tenure_start: u64,
    source: RepairSource<'_>,
) -> Result<RepairPlan, ReplError>;
```

`plan_repair` takes `&self`, clones internally and mutates nothing. When the source **is** faulted
it calls the existing `apply_repair` on the clone, so live authority, the named-pair match, the
sequence rule and the retry ordering all stay exactly where they already are and are not
reimplemented; `a` and `b` must then equal the held pair. The two full receipts are explicit
parameters because `apply_repair` reads them from `self.fault`, which a healthy owner source does
not have (R6, AG3-DES-009). When the source is **not** faulted the same
`repair.verify_current_owner` and `repair.check_evidence(a, b)` checks run and the clone records
`resolved_repair` and `repair_sequence` from the supplied pair, so the loser and its baseline
descendants are screened on every future admission even on a peer that never observed both.

**Same-tenure versus cross-tenure.** The discriminator is computed in core, from the same inputs:
`selected.verify_current_owner(group, issuer_tenure_start)`. It succeeds exactly when the faulting
tenure is the current one. Cross-tenure repair therefore **cannot install** the selected
checkpoint, because `from_checkpoint` and `select_repaired_checkpoint` both require that same
verification, and R14 means a repair is nevertheless the only way out of Fault. So cross-tenure has
its own single transition.

Classification. Let `S` be the selected receipt, `L` the loser, `H` the pre-application `latest`,
`E` the gate epoch, `O` the opening.

| Case | Condition | phase | receipt_hash | adopting | install |
|---|---|---|---|---|---|
| 1a | same-tenure, `O == Some(S)`, `H` qualifies (below) | `Closing` | `H.hash()` | `false` | none |
| 1b | same-tenure, `O == Some(S)`, `H` does not qualify | `Open` | none | `false` | none |
| 2a | same-tenure, `S.closed_epoch == E`, `!source.adopting` | `Closing` | `S.hash()` | `false` | none |
| 2b | same-tenure, `S.closed_epoch == E`, `source.adopting` | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 3 | same-tenure, `O == Some(L)` | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 4 | same-tenure, `source.adopting` and none of the above | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 5 | **cross-tenure**, any shape: `Unblocked` | `Open` | none | `false` | none |
| 6 | not faulted and `O != Some(L)`: screening only, no transition | unchanged | unchanged | unchanged | none |
| 7 | otherwise | refuse: `Held(UnsupportedShape)` | | | |

`H` qualifies in case 1a only under a **positive** justification, never merely the absence of a
conflict: `H.closed_epoch == E`, `H.tenure_id == S.tenure_id`, `H != L`, and `O == Some(S)`, so the
epoch `H` seals is the one the selected checkpoint opened. That is the only lineage statement
receipts support; they carry no ancestry chain. In every other same-tenure case the head is `S`.
This resolves AG3-DES-003: in the reviewer's counterexample `O == Some(L)`, which is case 3, so the
head is `S` and the adoption planner's `latest == receipt` precondition holds.

Case 4 is AG3-DES-002's failure path B: a distant adoption target with no usable anchor. Case 1b
covers failure path A by setting `adopting = false` explicitly rather than leaving the flag the
adoption attempt set.

**Case 2a/2b is AG3-DES-010's correction.** A fault whose pair closes the gate epoch is reachable
in adoption mode: `ingest_adoption` admits a receipt closing `E`, sets `adopting`, and a second
receipt closing `E` then faults through `ingest_verified` while `was_faulted` was false, so the
flag survives. Revision 2's single case 2 left `adopting` true with `install = None`, which
`repair_install_pending()` reads as continuation while the store reported the terminal `Repaired`.
Splitting the case removes the contradiction at the source, and C-5 removes the whole class by
making the committed state, not the plan, the terminality oracle.

**Case 5 is AG3-DES-009's correction.** A cross-tenure repair sets `fault = None`, records
`resolved_repair`, and returns the source to `latest = opening`, `tenure = TenureSelection::from(opening)`,
`previous_until_installed = None`, phase `Open`, `adopting = false`, `install = None`. Where the
source has no opening (a faulted epoch-zero source) `latest` and `tenure` both become `None`, which
the decoder's `tenure.is_some() == latest.is_some()` rule requires. Both pair members leave
`latest`; they are retained in `resolved_repair`, which is what screens them. Nothing old-tenure is
installed and nothing is claimed for installation. Convergence then happens the only way it can:
the source is no longer faulted, so ordinary discovery of the **current** owner's checkpoint
proceeds and, if that rewinds this branch, stages an ordinary `Rewound` snapshot. A same-tenure
head is discarded with the tenure that is over.

Case 6 keeps a peer that never faulted, but holds descendants of a repudiated branch, from having
to fault first before it can screen them.

`RepairHold` distinguishes `PairMismatch`, `SequenceNotNewer`, `ScopeMismatch`, `NoFault`,
`Settled` and `UnsupportedShape`. A `Held` result is a successful observation the caller reports,
never an error that discards evidence, exactly as `StudioAdoptionOutcome::Fault` is handled at
`store/epoch_studio/adoption.rs:103-104`.

On `StudioEpoch`, and symmetrically on `RegistryEpoch`:

```rust
pub fn apply_receipt_repair(
    &mut self,
    repair: &ReceiptRepair,
    a: &Receipt,
    b: &Receipt,
    group: &ServerGroup,
    issuer_tenure_start: u64,
) -> Result<StudioRepairExit, ReplError>;
```

It builds `RepairSource` from itself, calls `plan_repair`, and on `Transition` calls
`commit_repair`, whose closure swaps the book **and** `self.adopting` in the same critical section.
Any error returns before that closure runs, so the live unit is unchanged (I-1, P4).

**C-3. A repaired loser is not an anchor.** Two predicate changes in `epoch/adoption.rs`:

- `ingest_adoption`'s anchor search becomes
  `.find(|prior| receipts_conflict(prior, &receipt) && !self.is_repaired_loser(prior))`.
- `verify_adoption_state`'s two anchor predicates each gain `|| self.is_repaired_loser(prior)`.

`is_repaired_loser` is already the authority on which receipts a signed repair retired and already
screens incoming receipts on every path; this applies the same authority to retained anchors, the
only remaining place a repaired loser can block progress. A third differing baseline is unaffected
and still faults (I-9, M10, M11).

**C-4. Two layered, repair-independent checkers.** In `epoch.rs`:

```rust
/// Repair-independent: are these two canonical signed receipts a genuine conflicting pair for
/// this logical document? Signature and shape only. It asserts nothing about present membership
/// or ownership, needs no repair to exist, and mints no capability.
pub fn conflicting_receipt_pair(
    document: &LogicalDocument,
    a: &Receipt,
    b: &Receipt,
) -> Result<(), ReplError>;

impl ReceiptRepair {
    /// Adds this record's bindings on top of `conflicting_receipt_pair`.
    pub fn check_evidence(&self, a: &Receipt, b: &Receipt) -> Result<(), ReplError>;
}
```

The split is required by AG3-DES-001: the fault-report path validates a pair *before* any repair
exists, so it cannot call a method on `ReceiptRepair`. Contents are section 6.2.

`ResolvedRepair::verify` is rewritten to call `check_evidence` and **keeps its own additional
checks**: the repair's historical signature, the selected-versus-losing role assignment, the
enclosing book's document, and equality with the stored sequence including the non-zero rule. The
eight pair conditions do not replace `ResolvedRepair::verify`. `ReceiptBook::apply_repair`'s own
sorted-hash comparison is **retained as useful redundancy**; 15.2 M5 tests the checker boundary
directly and labels that comparison redundant for that claim rather than deleting it.

**C-5. Accessors that distinguish disposition from completion.** This is AG3-DES-004's correction
at the core boundary:

```rust
pub struct StudioRepairState {
    pub repair: ReceiptRepair,
    pub selected: Receipt,
    pub losing: Receipt,
    /// The selected checkpoint is the installed opening of this very source.
    pub installed: bool,
    /// A replacement this repair requires has not happened yet.
    pub install_pending: bool,
}

impl StudioEpoch {
    pub fn repair_state(&self) -> Option<StudioRepairState>;
    pub fn repair_install_pending(&self) -> bool;
    pub fn fault_evidence(&self) -> Option<(&Receipt, &Receipt)>;
}
impl ReceiptBook { pub fn repair_sequence(&self) -> u64; }
```

`installed` is `self.opened_by(&selected)`, matching the existing adoption test
(`store/epoch_studio/adoption.rs:89`). `install_pending` is
`resolved_repair.is_some() && self.adopting && self.phase() == Closing && latest == selected &&
!installed`. It is the single eligibility predicate both runtime steps use (10.3).

**`repair_state()` is the sole terminality oracle after B2** (AG3-DES-010). The in-memory
`RepairTransition.install` decides only what the transition itself must do; once the transition is
committed and saved, whether more work remains is read back from the committed source and never
from the plan. This makes the revision-2 contradiction unrepresentable rather than merely fixed in
one case: a source cannot simultaneously be `install_pending` and be reported terminal, because
only one value answers the question.

Because case 5 sets neither `adopting` nor an install, a cross-tenure repair can never leave a
source claimed by `repair_install_pending()` for a checkpoint that `select_repaired_checkpoint` is
forbidden to install.

**C-6. Recovery reason.** `prepare_checkpoint_adoption` gains a private reason parameter and a
public `prepare_repair_adoption(receipt, raw_seed, group, tenure)` wrapper passing
`RecoveryReason::Repair`. The reason is a property of the transaction that performs the
replacement, not a guess about lineage: cases 3 and 4 use `Repair`, ordinary adoption keeps
`Rewound`. `adopted_successor` is unchanged; the successor's book is cloned from the source and
carries the resolved repair.

**C-7. Repair-authorized journal reconciliation.** This answers AG3-DES-006 and R13:

```rust
impl OwnerReceiptJournal {
    /// The ONLY way an irrevocable decision is replaced. Requires a live-verified repair that
    /// names the journal's current preferred decision as its loser. It never creates a new
    /// publication obligation and never invents a decision the owner did not sign.
    pub fn resolve_repair(
        &mut self,
        repair: &ReceiptRepair,
        selected: &Receipt,
        group: &ServerGroup,
        issuer_tenure_start: u64,
    ) -> Result<bool, ReplError>;
}
```

Rules: `repair.verify_current_owner(group, issuer_tenure_start)`;
`selected.hash() == repair.selected_receipt_hash`; `selected.document` matches the journal's
document. If neither `in_flight` nor `high_water` hashes appear in `repair.receipt_hashes`, return
`Ok(false)` and change nothing. If the loser is `in_flight`, clear it. If the loser is
`high_water`, drop it. In **both** cases the winner becomes the journal's canonical decision. It
never sets `in_flight`: reconciliation records what the owner already decided, it does not create a
new thing to publish. The losing receipt's bytes are retained by the caller's record (5.2), so
nothing irrevocable is erased (I-11).

**The canonical decision is a distinct field, not the publication bit** (AG3-DES-011). Revision 2
only cleared a losing `in_flight`, which R15 shows is not enough: with `high_water = R(e-1)` and a
losing `in_flight = L(e)`, the owner falls back to `R(e-1)` for both head selection and
`prepare_verified`'s adjacency, so it serves the wrong receipt and reads the next receipt at `e+1`
as a gap. The journal therefore gains:

```rust
pub struct OwnerReceiptJournal {
    document, tenure, high_water, in_flight,
    /// Set only by `resolve_repair`. The canonical decision after a signed repair. It may never
    /// have been published by this device, so it is deliberately NOT `high_water` and
    /// `published()` keeps its exact meaning.
    reconciled: Option<Receipt>,
}

/// Effective head for selection and for the next decision's adjacency.
pub fn canonical_head(&self) -> Option<&Receipt>;   // reconciled.as_ref().or(high_water.as_ref())
```

`canonical_head()` replaces `high_water` in `prepare_verified`'s same-tenure adjacency rule and in
the head selector's `own_choice` (`in_flight().or(canonical_head())`), and is restored across
restart. `published()` still answers only "what did this device finish publishing", so nothing
falsely implies a completed publication.

The journal's wire format gains version 2 with the extra receipt slot, keeping the existing
`tenure`-consistency and adjacency invariants expressed against the canonical head. Encoded size
grows from at most two retained receipts plus a tenure to three plus a tenure: roughly 3.2 KiB
against the existing `MAX_OWNER_RECEIPT_JOURNAL_BYTES` of 3,328 bytes. That fits, but with only
about 128 bytes of headroom at maximal receipt sizes, so this design raises the constant to
`4 * MAX_RECEIPT_BYTES + 256` rather than relying on the margin. That widens the owner namespace's
pre-parse cap by 1 KiB and is carried into 5.2's record bound.

### 5.2 Store: durable owner issuance and reconciliation

`design-epoch-close.md` section 12 already budgets "fault evidence of 2 receipts and the latest
repair" as owner state per logical document, so the decision extends the existing per-document
`.owner-receipts` record rather than adding a sixth storage family.

```rust
pub struct EpochFaultRecord {
    /// The frozen pair, canonical ascending by hash. Validated by the repair-INDEPENDENT
    /// `conflicting_receipt_pair`, because this slot is written before any repair exists.
    a: Receipt,
    b: Receipt,
    /// Signed only after an explicit current-owner decision.
    repair: Option<ReceiptRepair>,
    /// The local source has durably crossed its repair transition (barrier B2). It does NOT
    /// mean the replacement finished, and never means delivery.
    applied: bool,
}
```

Splitting the pair from the repair is what makes AG3-DES-009 solvable. `apply_repair` needs
`self.fault`, and an owner whose own source is healthy has none, so a historical pair reported by
a peer has nowhere to live. This slot is that home: it is the issuance input, it is written by the
report path before any decision exists, and it survives independently of the source.

Encoding appends, after the existing optional `2 | hash | close`, an optional
`3 | a | b | u8 has_repair | repair? | u8 applied`. Sections must appear in ascending tag order
with no duplicates. Old readers already reject any tag other than 2 (`epoch_owner.rs:141-143`),
which is the stated intent of the v2 extension.

`check_scope` gains: both receipts re-decode byte-exactly; both documents equal this document;
`conflicting_receipt_pair(document, &a, &b)` passes; and when a repair is present,
`repair.document` equals this document and `repair.check_evidence(&a, &b)` passes. Corruption is an
error, never a silent reset.

**The frozen pair is never replaced while unresolved** (I-10). A report naming a different pair
while this slot holds an unresolved one is refused, exactly as a third receipt never replaces a
book's frozen pair. This is a guard the core does not provide on the owner side, because on that
side the book is not involved at all; it is what 15.2 M12 mutates.

Bounds: `MAX_RECORD_BYTES` becomes
`MAX_OWNER_RECEIPT_JOURNAL_BYTES + MAX_CLOSE_RECORD_BYTES + 3 * MAX_RECEIPT_BYTES + 1280`, which
with C-7's raised journal constant is about 12.4 KiB, with `MAX_SEALED_BYTES` following. The reader
still refuses anything larger before allocating. These bytes charge the protocol allowance through
the existing owner `storage_record`.

Transitions, both through `update_epoch_owner_state_with_writer` so they inherit its reload,
inventory verification, reservation, seal, atomic write and commit order:

```rust
pub fn prepare_epoch_repair(
    &mut self, server: u64, repair: ReceiptRepair, a: Receipt, b: Receipt, selected: &Receipt,
    group: &ServerGroup, issuer_tenure_start: u64,
    rng: &mut impl CryptoRngCore, budget: &mut EpochStorageBudget,
) -> Result<EpochOwnerReceiptState, AppError>;

pub fn mark_epoch_repair_applied(
    &mut self, server: u64, document: &LogicalDocument, repair_hash: [u8; 32],
    rng: &mut impl CryptoRngCore, budget: &mut EpochStorageBudget,
) -> Result<EpochOwnerReceiptState, AppError>;
```

`prepare_epoch_repair` performs **three things in one write** (barrier B1): it stores the repair
section with `applied: false`; it calls `journal.resolve_repair(...)`; and it drops
`decision_close` if that close bound the losing receipt, since `close_for` must never bind a
retired decision. Doing the reconciliation here, not after installation, means the owner never
serves the loser as a hint after deciding: the worst intermediate state is that it offers the
winner as an unproven hint until the source agrees, which is strictly better.

It refuses unless `verify_current_owner` and `check_evidence` pass and the sequence rule of 6.4
holds. An exact retry re-saves and succeeds. A different repair while one is held unapplied is
refused; the held decision must be resumed.

`mark_epoch_repair_applied` (barrier B3) sets `applied` and refuses a hash that is not the held
repair, so a stale completion cannot clear a newer decision. An exact retry still writes.

### 5.3 Store: the Studio transactions

In a new leaf `store/epoch_studio/repair.rs`.

```rust
pub enum StudioRepairOutcome {
    /// Fault ended and no replacement is required. Terminal.
    Repaired,
    /// Fault ended, the replacement completed. Terminal.
    Installed,
    /// Fault ended durably; the selected checkpoint's seed is still required. NOT terminal.
    AwaitingSeed,
    /// This exact repair is already the durable disposition AND its replacement is complete.
    AlreadyRepaired,
    /// A recovery eviction warning holds the replacement. Everything retained.
    RecoveryPending,
    /// Storage preflight or snapshot bound refused. Everything retained.
    StorageRefused,
    /// The named pair, sequence, scope, shape or issuer tenure does not permit this here.
    Held(StudioRepairHold),
}
```

**S-1 issuance.** `issue_studio_repair(...)` reads the frozen pair from **either** the source's
`fault_evidence()` or the record's `EpochFaultRecord`, which is what lets an owner repair a
historical fault it never observed itself (AG3-DES-009). It refuses unless the device is the
designated committer with an observed tenure, refuses unless the supplied pair equals the held
pair's hashes and the selection is one of them, derives the sequence per 6.4, signs with
`sign_in_tenure` using the **fault** tenure id from the pair and the **issuer** tenure start from
the observation, then calls `prepare_epoch_repair` (B1). The signed bytes are returned only after
that write returns; signing alone grants no IO authority.

**S-2 application.** `apply_studio_repair(...)`, with the custody split of 10.3:

1. Scope and channel checks; `repair.document == target.document(...)`;
   `repair.verify_current_owner(group, issuer_tenure)` before any expensive source work, in the
   shape `seal_studio_with_io` already uses (`store/epoch_studio.rs:415-419`).
2. `resolve_studio_handoff` (R10), before any journal, recovery or source side effect.
3. `checked_studio_receive_source`, which enters the five-family budget, authenticates the actual
   wrapper and verifies its inventory record. Absence is an error: a repair never creates a source.
4. Read `unit.repair_state()` **before** planning. This is the AG3-DES-004 correction:

| `repair_state()` | Action |
|---|---|
| `None`, or a different repair | plan and commit the transition (step 5) |
| `Some` with `installed` | flush the unchanged source, return `AlreadyRepaired` |
| `Some` with `install_pending` and no seed | flush the unchanged source, return `AwaitingSeed` |
| `Some` with `install_pending` and a seed | skip to step 7, continuing the replacement |
| `Some`, not pending, not installed | flush the unchanged source, return `Repaired` |

   Every row flushes rather than short-circuiting before any write, because a readable successor
   is not proof that its durability barrier returned. This mirrors the adoption retry, which saves
   or syncs the actual source (`store/epoch_studio.rs:556-566`).
5. `unit.apply_receipt_repair(...)`. On `Held`, save nothing and return the hold with the unchanged
   state.
6. Save the source (barrier **B2**). Fault has durably ended. The outcome is then read back from
   the **committed** `repair_state()`, never from the plan's `install` field (C-5, AG3-DES-010):
   `install_pending` continues to step 7 or returns `AwaitingSeed`, and only a committed state
   that is neither pending nor awaiting returns the terminal `Repaired`. The owner then records B3.
7. Replacement, reusing the accepted adoption half with the section 9 capability: validate every
   retained recovery slot as typed, verify the recovery inventory record, promote a due eviction
   (**B4**), hold if a warning is pending, stage the whole-version `Repair` snapshot (**B5**), hold
   again if that raises a warning, build the successor with `adopted_successor` and write it
   atomically (**B6**).

**S-3 fault report admission.** `report_studio_fault(...)`, the owner-only half of W-1, is
specified in 6.5.

**S-4 read.** `studio_fault_evidence(...)` returns both candidates and the repair status, read-only.

### 5.4 Store: the Registry transactions

`store/epoch_registry/repair.rs` mirrors 5.3 exactly, using `RegistryEpoch::apply_receipt_repair`,
`RegistryRecovery` typed validation of every existing slot as `stage_registry_recovery` already
does, and the same outcome variants. Same corrections, same tests, not a weaker mirror. Driven only
for buckets holding a Studio Index or Flipnote pointer; section 12.

### 5.5 Store: head selection, under one eligibility predicate

`ReceiptHeadSelection` gains `pub repair: Option<ReceiptRepair>`. `prepare_studio_head` and
`prepare_registry_head` populate it from a **single predicate shared with section 10**:

```rust
/// A repair is servable exactly when a durable local record shows it applied, or the saved
/// source's own book carries it as the resolved disposition. Both become true at B2/B3 and
/// neither is true for a signed but unapplied decision.
fn servable_repair(record: &EpochOwnerReceiptState, unit: &StudioEpoch) -> Option<ReceiptRepair>;
```

populated only after the same `reserve_sync` and `sync` flush the `prove` branch already performs
(`store/epoch_studio/discovery.rs:244-255`). A B1-only decision is never served (I-6).

Because `prepare_epoch_repair` reconciles the journal at B1, the selector's existing
`own_choice`/`held`/`prove` three-way equality now converges on the selected receipt once the
source agrees, instead of being wedged on the loser (R13). No change to that rule is needed beyond
the reconciliation and the new field.

### 5.6 Sync: distribution and the report direction

- `serve_receipt_head_with_handoff` passes `selected.repair` into the answer instead of `None`
  (`receipt_head.rs:539-543`). `encode_answer` already bounds and scopes it.
- `complete_checkpoint_hint` stops discarding answers that carry a repair
  (`receipt_head/detached.rs:223-225`), still refuses those carrying a proof, and exposes
  `AuthenticatedCheckpointHint::repair()`. A hint's repair is authenticated member delivery, not
  authority; the app verifies it under 6.2 and 6.3.
- `complete_checkpoint_head_scoped` already returns the whole answer; the Selected arm is unchanged.
- **Naming.** `catcoms-sync/src/lib.rs` owns an unrelated `repair_outbox` family for MLS delivery
  repair (`lib.rs:2175`, `:2350`, `:4035`). Every symbol added here is named `receipt_repair` or
  `fault_repair`, never bare `repair`, and must not share a module with it.

**W-1, the scoped head query version 2.** The reporter-to-owner direction, adopting U-1 (a).
AG3-DES-012 required three corrections: both codecs, an unambiguous tag, and the real charge order.

**Both codecs.** `encode_scoped_query` delegates Registry to a different framing (R16), so both are
specified and both gain the same trailing section:

```text
studio query v2   = 2 | u16 doc_type | bytes channel | bytes object | bytes nonce | report
registry query v2 = 2 | u16 doc_type | bytes logical_key | bytes nonce | report
report            = u8 count (0 or 2) | count * bytes Receipt
```

**Unambiguous tag and one canonical form.** The first byte is `2`, not `1`: v1 keeps its byte and
its exact framing, and a v1 decoder rejects a v2 query as it already rejects any other leading
byte. The report is a counted list, never an optional trailing field, so "v2 with no report" has
exactly one encoding. A count of `1` is rejected: the pair is always complete.

**The pair is always complete.** Revision 2 carried one receipt, which suffices only for the
rollback case where the provider already holds the other member. For a historical fault the new
owner may hold neither (AG3-DES-009), so the reporter, which is faulted and therefore holds both,
always sends both. One shape, no guessing about what the provider lacks. The scoped query cap rises
to `MAX_QUERY_V2 = MAX_QUERY + 2 * MAX_RECEIPT_BYTES` (2,304 bytes) and `queue_checkpoint_head`'s
length bound to `MAX_QUERY_V2 + 144`.

**Real parse and charge order.** The current service authenticates, then calls
`decode_scoped_query`, and only afterwards charges the per-requester rail
(`receipt_head.rs:378-415`); revision 2 claimed the opposite. Rather than reorder an accepted
seam, the decode is split:

1. existing length bound, watch check, pending cap (8) and global preauth rail (20);
2. `authenticate_request`, member and epoch checks;
3. **header-only** decode: version, target, nonce, and the report section captured as an opaque
   length-delimited slice that is bounds-checked but not parsed;
4. existing requester dedupe and per-requester rail (2);
5. only now, in the admission step, are the receipts decoded and validated.

So no untrusted party can make the provider parse up to 2 KiB of receipts before paying its
per-requester rail, and the accepted ordering of the existing steps is untouched.

**Version compatibility.** A provider at the old version rejects a v2 query through the existing
strict decode, so the reporter gets no answer and falls back to v1. Older peers never receive
reports, which is safe: a report only ever adds evidence the owner already signed, and its absence
only delays discovery of an owner-side rollback.

One seed-fetch addition, needed because the selected receipt is not the owner's freshly proved head:

```rust
/// Mint a seed-fetch selection from a locally verified repair instead of a fresh owner proof.
/// Fails closed unless the SELECTED receipt itself verifies under the current owner and the
/// independently observed tenure, so a cross-tenure repair can never drive an installation.
pub fn select_repaired_checkpoint(
    &mut self,
    target: CheckpointTarget,
    fault_repair: &ReceiptRepair,
    selected: Receipt,
) -> Result<RegistrySeedFetch, SyncError>;
```

It reuses the existing selection generation, four retained slots, three paced attempts and
sixty-second lifetime.

### 5.7 App: control, runtime and settlement

```rust
pub enum StudioControlAction { /* existing */ ReadFault, RepairFault(Box<StudioFaultRepairRequest>) }
pub struct StudioFaultRepairRequest { pub receipt_a: [u8; 32], pub receipt_b: [u8; 32], pub selected: [u8; 32] }
pub struct StudioFaultCandidate {
    pub receipt_hash: [u8; 32], pub closed_epoch: u64,
    pub close_record_hash: [u8; 32], pub seed_change_hash: [u8; 32],
    pub inherited_epoch: Option<u64>, pub locally_installed: bool,
}
pub struct StudioFaultView {
    pub target: StudioTarget, pub source: Option<StudioSettlementSource>,
    pub candidates: [StudioFaultCandidate; 2], pub repair: Option<StudioRepairStatus>,
    pub may_decide: bool, pub blocked_by: Option<StudioRepairBlocker>,
    /// Operations a replacement would move into recovery. Not a claim they are lost.
    pub preserved_operations: usize,
}
pub enum StudioControlResponse { /* existing */ Fault(Box<StudioFaultView>), Repaired { target: StudioTarget, outcome: StudioRepairOutcome } }
pub enum StudioSettlementState { /* existing seven */ Repairing, StorageRefused }
```

Runtime: a new `studio/receiver/catchup/repair.rs` step, scheduled in the same slot as
`rotate_owner`, after discovery, seed and page work. Section 10.3 gives ownership, pacing and the
shared `repair_install_pending` predicate that keeps it and `advance_checkpoint` from both claiming
one target.

### 5.8 Native, designed and not registered

`apps/desktop/src-tauri/src/studio/fault.rs`: `studio_fault_read` and `studio_fault_repair`,
through the parent module's one custody and session path. `settlement.rs` gains
`S::Repairing => "repairing"` and `S::StorageRefused => "storageRefused"`, and its exhaustive
contract test gains both rows. The `#[tauri::command]` attributes, registration and security rows
land in a separate identifiable commit owned by Agent 4, gated on 13.3.

## 6. Selection, evidence and authorization

### 6.1 Explicit selection, no silent policy

The runtime never selects. `studio_fault_evidence` returns both candidates with what a person needs
to choose: which epoch each closes, which close and seed each names, which inherited baseline each
carries, which one the local source descends from, and how many local operations a replacement
would move into recovery. `may_decide` is true only for the actual current owner with an observed
issuer tenure; everyone else gets `may_decide: false` and a reason.

`RepairFault` echoes both hashes and the chosen one. The actor re-derives the pair from the durable
source under custody and refuses if it differs, so a stale view cannot authorize a decision about a
pair that is no longer the live fault. The renderer supplies no receipt bytes, tenure, sequence or
repair record. There is deliberately no automatic tie-break, not even "prefer the one we installed".

### 6.2 Exact conflicting-receipt evidence

`conflicting_receipt_pair(document, a, b)` requires, with no repair in existence:

1. `Receipt::decode(&a.encode()) == a`, and the same for `b`. Public Rust fields are not a
   validation boundary; the wire decoder's exact schema is.
2. `a.verify_signature_only()` and `b.verify_signature_only()` succeed. Never
   `verify_current_owner`: the signer of a faulting pair may no longer be the committer.
3. `a.document == b.document == *document`, comparing the full `LogicalDocument`.
4. `a.tenure_id == b.tenure_id`.
5. `a.hash() != b.hash()` and `receipts_conflict(a, b)`: same closed epoch, **or** a differing
   `TenureSelection`. Two successive consistent receipts are progress, not equivocation.

`ReceiptRepair::check_evidence(a, b)` calls the above with `self.document` and adds:

6. `self.tenure_id == a.tenure_id`.
7. `self.receipt_hashes == sorted([a.hash(), b.hash()])` and
   `self.receipt_hashes.contains(&self.selected_receipt_hash)`.
8. `self.issuer_tenure_start_group_epoch.is_some()`. A v1 record fails here and can never
   authorize a live repair.
9. `self.repair_sequence > 0`.

**Scope qualification (reviewer's correction).** A receipt's signed `server_id` is checked against
the MLS group id only. The **local numeric server**, the store scope and, for a Flipnote, the
**channel**, are not bound by the signature: `StudioTarget::Flipnote`'s logical key is the object
id alone. Every caller therefore additionally checks `scope_bytes(server, document)`, the
authenticated store record's own scope, and `target.channel()` against the request target, exactly
as the existing Studio store paths do. The repair signature is not a substitute for any of them.

Live authority, read at the actual custody visit and never from a cache, is
`ReceiptRepair::verify_current_owner` unchanged (`epoch.rs:1343-1360`).

Application adds `repair_sequence > book.repair_sequence` and equality with the locally held pair,
both already inside `apply_repair`.

Replacement adds: the fetched seed's change hash equals `selected.seed_change_hash`, checked by the
existing `verify_checkpoint` before decoding, with the typed channel and root validation
`prepare_checkpoint_adoption` already performs.

`ResolvedRepair::verify` keeps, on top of `check_evidence`: the repair's historical signature, the
selected-versus-losing role, the enclosing book's document, and equality with the stored non-zero
sequence.

### 6.3 Current owner and issuer tenure

`expected_issuer_tenure_start` comes from exactly one place,
`ChannelSync::observed_owner_tenure_start()` (`owner_tenure.rs:151`), read inside the same
synchronous custody visit that uses it. `None` is a hold, never a substitution. The repair path
never uses `group.epoch()`, a receipt's carried `tenure_start_group_epoch`, or the repair's own
claimed field as its own evidence. Issuance additionally requires a `ServerOwnerSnapshot`, rechecked
by `with_durable_owner_snapshot` at the moment of use.

Two identities, never conflated: `repair.tenure_id` is the **fault tenure**, matched against the
receipts; `repair.issuer_tenure_start_group_epoch` is the **issuer tenure start**, matched against
the observation.

Consequences stated honestly:

1. A returning owner cannot reuse its first tenure's repair; the claimed start no longer matches
   the observation. The retry shortcut sits after the authority check in core, which is already
   correct (`epoch.rs:1735-1741`).
2. A **cross-tenure repair installs nothing**, and revision 2 was wrong about what happens next.
   The selected receipt cannot be current-owner verified, so `select_repaired_checkpoint` fails
   closed and `from_checkpoint` could not be called. Revision 2 then claimed convergence would come
   "through the current owner's ordinary first receipt". **R14 shows that is impossible**: every
   admission path returns `Fault` before considering the incoming receipt, so no receipt from any
   owner can clear a fault. A repair is the only exit, which is precisely why the v2 tenure split
   exists and why the report path must carry historical pairs.

   The corrected story: the cross-tenure repair itself ends the fault through case 5, records the
   evidence and screens the loser and its baseline descendants. **Only because the fault is now
   gone** does ordinary discovery of the current owner's checkpoint proceed, rewinding this branch
   into an ordinary `Rewound` snapshot if it must. The repair unblocks; discovery converges.
3. A newcomer with unknown tenure verifies no repair and applies none. Its convergence path is the
   current owner's fresh proof, unchanged. N14 asserts this.

### 6.4 Retry, continuation and completion

**Three distinct things** (AG3-DES-004):

- **Disposition**: this exact repair is the book's resolved repair. Read from `repair_state()`.
- **Continuation**: `install_pending` is true; recovery and replacement are outstanding.
- **Completion**: `installed` is true, or the transition required no replacement.

`ReceiptRepairIngest::Duplicate` establishes only the first. The store never maps it to a terminal
outcome; it reads the source (5.3 S-2 step 4). Continuation resumes the same snapshot identity and
warning deadline, because the recomputed snapshot has the same source bytes, the same `Repair`
reason and the same source-opening `selecting_receipt`, so `RecoverySnapshot::id()` is identical
and `Stage` is idempotent.

Every retry row flushes the actual source rather than short-circuiting before a write, because
visible bytes are not proof that a post-rename parent flush returned.

**Owner sequence.** Strictly increasing per document; the next value is
`1 + max(record.repair.repair_sequence, source_book.repair_sequence())`. Seeding from the book
matters when the owner record is absent after a reinstall while peers hold a higher applied
sequence. An exact retry re-signs deterministically (the transcript has no nonce), re-saves and
returns the same bytes. A different selection for the same pair while a repair is held unapplied is
refused.

**Never**: restage under a new snapshot id, clear a different active fault, or lower a retained
head (C-2 case 1a).

### 6.5 The fault report contract

This is AG3-DES-001's required correction, and the only new wire boundary in this scope.

**Reporter.** A peer whose own source for the target is Faulted attaches **both** members of its
frozen pair to its next query for that target (5.6 W-1). It chooses nothing and asserts nothing
about authority. One report per query, paced by the existing four outbound head slots and
ten-second request lifetime.

**Provider admission**, implemented by `report_studio_fault`, reached after the header-only decode
and the per-requester rail (5.6 W-1 step 4), and **before the response is decided** (U-7):

1. The requester is authenticated as a current member and the request is fresh. Already enforced by
   `authenticate_request` and `head_request_current`.
2. Complete target check: numeric server, group id, doc type, logical key and, for a Flipnote, the
   channel (6.2's scope qualification).
3. Both reported receipts decode byte-exactly; `conflicting_receipt_pair(document, a, b)` passes.
   This is **historical** validation only: signature and shape, never `verify_current_owner`,
   because the signer of a historical pair is by definition no longer the committer.
4. **Admissibility is by document scope and genuine conflict, not by tenure.** Revision 2 refused
   anything but the current owner's tenure, on the premise that an older fault would heal through
   the new owner's first receipt. R14 shows that premise is false, so that rule would have made
   cross-tenure repair permanently unreachable (AG3-DES-009). A historical pair is admissible; the
   **repair** it enables still needs live current-owner authority, which is where the authority
   question belongs.
5. **Two disjoint recording paths**, chosen by whether the pair is the current owner's tenure:
   - **Same tenure as the current owner** (the rollback case): the reported receipt genuinely is a
     current-owner receipt, so the existing typed seal is a legitimate admission and the provider's
     own source durably enters Fault through the common source fences, stopping its head service
     and rotation for that document.
   - **Any earlier tenure** (the historical case): **nothing is passed through the live seal.**
     `StudioEpoch::seal` ends in current-owner verification and only handles its own gate epoch,
     opening and adoption shapes, so feeding it an old receipt would be exactly the confusion of
     historical authenticity with live authority that the review warned against. The pair is
     written only into the owner record's `EpochFaultRecord` (5.2). The provider's source is not
     touched and does not fault: its own head under the current tenure is not in dispute.
6. **No-ops and the frozen pair.** A report whose pair is already recorded, or whose members are
   already screened by a resolved repair (`is_repaired_loser`), changes nothing. A report naming a
   **different** pair while an unresolved one is held is refused: a frozen pair is never replaced by
   a third receipt (I-10). On the owner side this refusal lives in the record, not in the book, so
   it is a guard the core does not otherwise provide (15.2 M12).
7. Every write goes through the **common source fences**: `resolve_studio_handoff` first, then
   `save_studio_source_checked` or the accounted owner writer.
8. No winner is chosen, no checkpoint adopted, no issuer tenure inferred, no journal decision
   replaced. Only `issue_studio_repair` may do any of those, and only on an explicit user action.

**Ordering against the answer (U-7, reviewer's decision, overriding revision 2).** A provider that
has authenticated a request and validated a genuine conflict must not mint another authoritative
proof for one side and only then try to persist the evidence. So admission runs **before** the
response is decided, and:

- a successfully admitted report means no authoritative proof is served for **either** member of
  the newly recorded pair in that response, and none at all while the provider's own source for
  that document is Faulted;
- a failed or uncertain fault or record write **fails closed for authoritative head service**: the
  response carries at most an unproven hint, and the reporter retries.

This also removes revision 2's liveness hole, where admission only happened if enough request
lifetime happened to remain after answering.

**Duplicates** are idempotent no-ops by (6), charged against rails already paid. A **failed write**
returns no success and invalidates the budget like every other writer; the next report retries
exactly.

### 6.6 Distribution eligibility

One predicate, used identically by 5.5 and section 10: **a repair is servable once a durable local
record shows it applied, or the saved source's book carries it as the resolved disposition.** Both
become true at B2/B3. A signed but unapplied decision (B1 only) is never servable.

This removes the deadlock the reviewer found. Previously, serving waited for installation, while
installation needed the selected seed, which only a peer holding that checkpoint could serve, and
such a peer refused all seed service while faulted (`studio/epoch.rs:199-204`). Now a faulted peer
that receives the repair crosses B2, stops being faulted, and can serve its installed opening's
seed, which is exactly the selected checkpoint's seed for the peers that had installed the winner.

Two refusals sit alongside this predicate, both from U-7: no authoritative proof is served for
either member of a pair recorded in this same exchange, and none at all while the local source for
that document is Faulted. A repair being servable is never a licence to prove a disputed receipt.

### 6.7 Owner journal reconciliation

Performed inside B1 by `prepare_epoch_repair` (5.2), using `resolve_repair` (5.1 C-7). It is the
only way an irrevocable decision is replaced, it requires a live-verified repair naming that
decision as the loser, it never creates a new publication obligation, and the losing receipt and
its close remain in the record as historical evidence. Without it the journal is wedged (R13): the
owner keeps preferring the loser, can never prove, and cannot prepare the winner at the same
closed epoch.

Clearing the loser is necessary but not sufficient (AG3-DES-011, R15). Both the head selector's
`own_choice` and `prepare_verified`'s same-tenure adjacency read the retained **high water**, so
the shape `high_water = R(e-1)`, `in_flight = L(e)`, winner `S(e)` would fall back to `R(e-1)`:
the owner would serve the wrong receipt, fail the three-way equality that gates a proof, and read
its next receipt at `e+1` as a gap. `resolve_repair` therefore also establishes `S(e)` as the
journal's `reconciled` canonical decision, and `canonical_head()` is what both consumers use.
`published()` is untouched, so nothing claims a publication that did not happen.

## 7. The pipelines

**Flow F, entering fault.** Implemented: a proved receipt conflicts with the book or the retained
opening; `adopt_studio_checkpoint` returns `Fault` after crossing its own source barrier; the
receiver notes `RefreshRequired` then `Fault`. Head service, seed service and rotation refuse for
that document only.

**Flow R, reporting (new).** A peer that detects a conflict with a provider head attaches its
receipt to its next query; the owner admits it under 6.5 and gains a durable Fault. This is the
route that makes issuance reachable after an owner-side rollback.

**Flow I, issuance (owner).** Read evidence, user chooses, `issue_studio_repair` verifies, derives
the sequence, signs, and writes **B1** (repair pending plus journal reconciliation). Flow A then
runs on the owner's own source with the fresh record; **B3** records `applied` immediately after
B2 returns.

**Flow A, application (owner and peer, identical code).** 5.3 S-2 steps 1 to 7, with barriers B2,
B4, B5, B6.

**Flow D, distribution.** Ordinary head answers carry `{ receipt, repair, proof? }` under the 6.6
predicate. The receiver authenticates the responder and the response signature, verifies the repair
under 6.2 and 6.3, and runs Flow A if its own source for that target holds the named pair. A repair
for a document it has no fault for is discarded without state change.

**Flow X, visible exit.** `RefreshRequired` before the transaction; `Repairing` once B2 returns and
while `repair_install_pending` or any hold is true; then the actual phase read back from the saved
state; plus `RecoveryEvictionPending` or `StorageRefused` on a hold. Before B2, holds keep `fault`.
The UI re-reads; it never infers repair from a Restore, an absent error or a projection, because
`Repairing` has exactly one producer and `StudioFaultView` is the only structure reporting
candidates.

## 8. Durable ordering, crash and reopen

| Barrier | Written | Crash immediately before | Crash immediately after |
|---|---|---|---|
| B0 | owner record or source: the reported frozen pair (6.5 rule 5) | no evidence; the reporter retries; no proof was served for a disputed receipt because admission precedes the response | the pair is frozen and cannot be replaced by a third receipt; the owner can decide |
| B1 | owner record: repair signed, journal reconciled to the canonical winner, stale close binding dropped | no repair exists; the fault is unchanged; the owner may decide again, possibly differently | the exact decision resumes; a different selection is refused; the owner offers the winner as an unproven hint |
| B2 | source: fault cleared, resolved repair in book, adoption mode set | source still Fault; re-apply from the pending record or a re-fetched repair; identical result | fault has ended; if `install_pending`, the state is the accepted adoption "Closing, awaiting seed" shape carrying the repair |
| B3 | owner record: `applied` | serving falls back to the source book, which already carries the repair, so eligibility is unchanged | record and book agree |
| B4 | recovery eviction promotion | warning still pending; the hold is re-reported | staging proceeds |
| B5 | recovery record: staged `Repair` snapshot | nothing staged; recompute yields the identical snapshot id and deadline | the losing version is durably readable; replacement may proceed |
| B6 | successor source, atomically | source is still the losing branch with recovery already holding its copy; retry replaces it | installed; `repair_state().installed` makes the retry a flush-and-return |

Reopen reads only sealed bytes. No warm source, in-memory hint or cached tenure participates after
a restart, and the section 9 capability is reacquired through its durability barrier rather than
inferred from readable bytes. Uncertain IO invalidates the budget and requires full inventory
reconciliation; the repair adds no bypass.

Three shapes are impossible by construction:

- A gate out of Fault whose book still holds that fault, or the reverse: C-1 commits both under one
  lock and refuses any incoherent target (I-1).
- A durable repair applied to a source that still descends from the loser with no recovery copy:
  the successor write is unreachable without the section 9 capability (I-2).
- An owner whose live preferred decision is a repaired loser: B1 reconciles or refuses (I-11).

## 9. Recovery before replacement

Straight-line ordering is insufficient evidence, so the replacement is gated by a capability the
**common writer** demands based on the authenticated durable predecessor, not on caller
classification (AG3-DES-007):

```rust
/// Minted only after the exact typed recovery record's required save or flush has RETURNED and
/// its warning state permits replacement. Not Clone, not Copy, not durable, not reusable across
/// custody visits.
pub(super) struct CheckedRepairRecovery {
    scope: StorageScope,          // numeric server + group id
    predecessor: blake3::Hash,    // authenticated predecessor wrapper plaintext
    successor_plan: [u8; 32],     // the plan the successor was built from
    snapshot: [u8; 32],
}
```

The writer's predicate is derived from durable bytes: while decoding the authenticated predecessor
it already holds, `save_studio_source_checked` computes
`predecessor.repair_state().is_some_and(|s| s.install_pending)`. When that is true **and** the unit
being written is not that same predecessor, the write requires a `CheckedRepairRecovery` whose
`scope` matches the entered budget scope, whose `predecessor` matches the digest of the bytes just
authenticated, and whose `successor_plan` matches the plan the successor was built from. Passing
`None` in that situation is a runtime refusal, not merely a missing argument, and the same predicate
also covers an ordinary adoption path that might otherwise replace the same held source.

`stage_studio_repair_recovery(...) -> Result<Option<CheckedRepairRecovery>, AppError>` returns
`None` with `RecoveryPending` when a warning holds and `Err` when the preflight refuses. The
verified empty-source case, where the plan legitimately has no snapshot, mints a capability with a
zero `snapshot` only after the same typed validation of all retained slots and the same inventory
verification, so "no recovery needed" is a checked conclusion rather than a skipped step.

None of this weakens the existing Prepared capability, the source-required metadata link or the
conservative reference checks; they remain separate, mandatory and unchanged.

A plan whose encoded snapshot exceeds `MAX_RECOVERY_SNAPSHOT_BYTES` (6 MiB) fails in
`StudioRecovery::snapshot` with `EpochBound`, reported as `StorageRefused` with the whole branch
retained. Adoption has the same property; the repair adds no weaker path around it.

What is preserved: the whole current typed version through the same compactor adoption uses,
including deletions, conflict overflow, seed-only values and insertion gaps; every accepted signed
operation as author-plus-envelope pairs; every pending intent, because the repair calls no
retirement path at all (I-5); any retained overlay branch; and blob references through the
conservative holds that run before either barrier.

## 10. Capacity, publication ordering, fairness and custody

### 10.1 Admission

| Write | Purpose | Pool | Peak |
|---|---|---|---|
| owner record (B1, B3) | `Settlement` | protocol allowance, 16 MiB per server | old plus new, about 11.4 KiB each at maximum |
| recovery stage (B4, B5) | `Settlement` | settlement reserve, 48 MiB, staged slot | up to 6 MiB plus the existing replacement peak |
| source transition (B2) and successor (B6) | `Settlement` | settlement reserve | old plus new source |
| report fault write (Flow R) | `Settlement` | settlement reserve | old plus new source |

No planned deletion is credited before its IO commits. A reservation dropped without commit
requires full reconciliation. The resolved repair adds roughly 3 KiB per repaired document to
`storage_protocol_bytes` (`studio/epoch.rs:504-518` counts the book delta); against the 16 MiB
allowance that is roughly 3,300 repaired documents per server, an arithmetic estimate, not a
measurement.

### 10.2 Publication ordering

Sign, then B1, then B2, then B3, and only then servable, under the single 6.6 predicate. B4 to B6
are not preconditions for serving. Serving is availability only: no step claims delivery and no
answer marks a receipt journal published. `with_receipt_head_handoff` is untouched and a repair
never mints one.

### 10.3 Custody, ownership and fairness

This is AG3-DES-008's correction. A repair turn is four stages, not one call:

- **S1 capture, sole custody, bounded.** Reserve one slot from
  `registry_catchup::preparation_pool()` **before any body read**. Authenticate the source wrapper
  and read the bounded record. Capture the full context: numeric server, group id, MLS epoch,
  observed issuer tenure, device and membership, actor and sync incarnation, mount, complete target
  including channel, the source wrapper digest and physical size, and the original native request
  and session. Release custody.
- **S2 detached, no Server, no vault key, no MLS secret.** Restore the graph if cold, compute the
  transition plan, the typed whole-version snapshot and the seed verification. Owns the permit, the
  authenticated plaintext, the public context and any transient reference hold only.
- **S3 revalidate, sole custody, bounded.** Recheck every captured coordinate, recheck that the
  authenticated wrapper digest and size are unchanged, recheck live authority and tenure, then
  perform the writes.
- **S4 release.** The permit is carried through queue, worker, ready result and delivery.
  Cancellation never refunds a slot a worker or ready result still owns. Admission bookkeeping keeps
  only `Weak` handles and reaps dead owners, so a dropped native handle releases where the last
  `Arc` drops, matching Agent 1's I-2.

**Mutation generation.** When Agent 1's resumable five-family scanner is used, rotating its
mutation generation is **mandatory** for this scope, before the first possible IO of **every**
repair-related path, not only the headline writes: B1, B2, B3, B4, B5, B6, the Flow R fault write,
temporary siblings, cleanup and every failed operation. `verify_record` and reservation discipline
do not invalidate a parked scan and are not substitutes.

**Fairness.** One repair job per actor turn per server. Faulted and repairing targets rotate
round-robin using the same selection-index pattern as `rotate_owner`, so one permanently held fault
cannot monopolize the slot. The step runs after discovery, seed and page work. The existing 5 s
per-target cadence applies; any hold backs off to 60 s. Seed fetches use the existing four retained
selection slots, three paced attempts and sixty-second lifetimes; no new pool. Head rails are
unchanged, and a report rides an already-admitted query.

**One claim per target.** `repair_install_pending()` (C-5) is the shared predicate: the repair step
claims such targets and `advance_checkpoint` skips them, so ordinary adoption can never stage a
second `Rewound` snapshot of content a repair already staged as `Repair`.

Unmeasured costs to record before the implementation review: the whole-version `Repair` snapshot
encode at a maximal accepted source (20,000 operations, 4 MiB), the successor build, and the
custody time of S1 and S3.

## 11. Interrupted Prepared overlays and the common fences

A repair is a common source writer and a common source reader. It therefore:

1. Calls `resolve_studio_handoff` first on **both** the application and report paths, so an
   interrupted Prepared transaction is resolved against actual durable evidence, never overwritten.
   A `StudioHandoffEvidence::Hold` propagates as an error; the fault and the overlay are retained.
2. Writes only through `save_studio_source_checked`, so `check_studio_handoff_write`, the
   source-required metadata link and the conservative blob holds apply unchanged.
3. Retires no intent, so `retire_included_with_io`'s `handoff_prepared()` refusal is never reached
   and never needs relaxing.
4. Preserves the complete pending ledger including overlay-annotated entries. A replacement changes
   the source an overlay's basis refers to, so the basis becomes stale; the branch is retained and
   visible, and the stale-basis manual path is Agent 2's. This design performs no disposal, no
   eviction and no automatic rebase.
5. Introduces no competing source writer and no second preparation pool.

## 12. Registry dependencies for Index and Flipnote

Studio discoverability runs through the target's registry bucket:
`install_registry_seed_for_studio` (`studio/receiver/catchup/discovery.rs:159`) and
`refresh_studio_registry_pointer`. A faulted bucket refuses page service
(`page_source.rs:235`), refuses owner maintenance (`epoch_registry/owner.rs:49`) and is skipped
with a bounded diagnostic (`registry_runtime.rs:154-161`), so a newcomer cannot find an Index at all
while its bucket is faulted even though the Index itself is healthy.

Required, with the same corrected transaction and the same convergence tests: the Registry
`apply_receipt_repair` and transaction (5.4), `prepare_registry_head` serving a durable repair
(5.5), the Registry half of the report path **including its own v2 query encoding** (5.6 W-1, which
AG3-DES-012 correctly found missing in revision 2 because `encode_scoped_query` delegates Registry
to a different codec), and the `registry_runtime` Fault arm attempting a repair before giving up.
A Registry owner rollback is not a deferrable product extension: a faulted bucket blocks Index and
Flipnote discovery outright, so its reporter-to-owner route ships with the Studio one. Explicitly excluded: any other `DocType`, any bucket holding no Studio
pointer, and any registry behaviour beyond ending a fault. The `MAX_REGISTRY_EPOCH` lineage ceiling
(`epoch/adoption.rs:91-93`) stays enforced on every repaired anchor.

## 13. Coordination

### 13.1 Agent 1: source and commit custody

Consumed and not weakened: `resolve_studio_handoff` as the first step of every repair path; the
Prepared source fence and publication hold across restart; `save_studio_source_checked` as the
single writer with its handoff capability, metadata link and reference holds; the four-slot shared
preparation pool.

Required if Agent 1's proposed `inventory_generation` and `epoch_mutation_guard` land: mandatory
rotation over the full list in 10.3. If they do not land, this design relies only on the existing
`verify_record` and reservation discipline and says so.

Asked of Agent 1: preserve `save_studio_source_checked`'s capability-parameter shape so the
section 9 capability is a second parameter of the same kind rather than a third mechanism.

Conflict risk: both scopes add variants to `StudioControlAction`, `StudioControlResponse`,
`StudioSettlementState`, `StudioBackgroundJob` and the native mapping. Agent 4 owns the merge; this
design adds only the variants named in 5.7 and changes no existing one.

### 13.2 Agent 2: live tenure and stale bases

- **T1** One accessor returning `Option<u64>` for the current owner tenure start, no fallback.
- **T2** `None` stays a hold; no substitution from the group epoch, a receipt's carried field or a
  repair's claimed field.
- **T3** A new authenticated tenure protocol must expose the same `Option<u64>` shape plus a
  freshness binding rechecked at **every** custody visit: verification, application, serving and
  report admission. No tenure value is cached across an await or a store borrow.
- **T4** Fault tenure and issuer tenure are distinct (6.3) and must not be collapsed.
- **T5** The preview path must not render or act on a receipt the local book screens as a repaired
  loser.

Handed back: a replacement invalidates a retained overlay's Closing basis when the source's opening
receipt changed and the branch was not derived from the new one. The work stays retained; the
manual path is Agent 2's. Fail-closed `None` is correct but is not by itself evidence of eventual
progress; 15.1 N14 and N15 demonstrate progress with legitimate evidence.

### 13.3 Agent 4: integration contract

Register the two native commands and their security rows only after this scope's implementation
review passes; add `"repairing"` and `"storageRefused"` to the native mapping and its exhaustive
test; record in `INTERFACES.md` the answer's repair field, the v2 scoped query and its raised cap,
and that an unapplied decision is never served; replace the UI-hooks row per 15.4; record in
`BACKEND-IMPLEMENTATION.md` and `design-epoch-close.md` section 8 that `Repairing` and the Registry
repair producer are connected. **This design edits no shared contract document.**

## 14. Limits

- One repair record per document is retained; an older, no-longer-covered conflict can require
  another repair at a higher sequence (U-5). A genuine third baseline faults; a covered
  losing-baseline descendant stays stale without re-faulting.
- A cross-tenure repair ends a fault but installs nothing; the source then converges through
  ordinary discovery of the current owner's checkpoint, which is possible **only** because the
  fault is gone (6.3, R14).
- A newcomer with unknown tenure applies no repair and converges through the owner's proof.
- A historical pair is admissible as a report and lands in the owner record, not through the live
  seal (6.5 rule 5). Revision 2's tenure restriction here was withdrawn: R14 means it would have
  made cross-tenure repair permanently unreachable.
- A replacement whose whole-version snapshot exceeds 6 MiB is a visible `StorageRefused` hold.
- Older peers never receive reports, by version compatibility; this only slows discovery of an
  owner-side rollback.
- Serving proves availability, never delivery.
- No measurement was taken. The maximal-shape costs and the protocol-allowance arithmetic in
  section 10 are unverified estimates.

## 15. Test and mutation plan

### 15.1 Normal regressions

Core:

- **N1** `commit_repair` accepts each coherent target and refuses each incoherent one, leaving gate
  and book byte-identical on refusal.
- **N2** A repaired source persisted at B2 round-trips through `snapshot`/`restore` for every C-2
  case, and `receipt_head()` stops erroring.
- **N3** AG3-DES-002 path A: a fault produced by `begin_checkpoint_adoption` (which sets
  `adopting`) is repaired by selecting the existing opening; the result is case 1b with
  `adopting == false` and the adoption validator is not consulted; restore succeeds.
- **N3b** AG3-DES-003 counterexample: installed opening L, retained newer sealing receipt H, late
  conflicting opening W with the same `TenureSelection`. Selecting W gives case 3 with head W, and
  `prepare_repair_adoption` succeeds. Selecting L gives case 1a with head H preserved, including
  `previous_until_installed`. Both winners asserted.
- **N4** AG3-DES-002 path B: an epoch-zero source mid-adoption of a receipt closing epoch five,
  faulted by a conflicting receipt for that checkpoint, repairs into case 4 and restores.
- **N5** AG3-DES-010: a fault whose pair closes the gate epoch, built **both** ways. In ordinary
  seal mode it repairs into case 2a and the committed `repair_state()` reports no outstanding work.
  In adoption mode, reached by admitting a receipt closing `E` and then a second one, it repairs
  into case 2b and the committed state reports `install_pending`, and the store continues to
  installation. Asserting only that both restore as `Closing` is explicitly insufficient.
- **N5b** Case 5: a cross-tenure repair on each reachable fault shape, including a faulted
  epoch-zero source, restores with `latest` and `tenure` consistent, is never `install_pending`,
  and permits an immediately following ordinary adoption of the current owner's checkpoint that
  stages a `Rewound` snapshot.
- **N5c** Case 6: a non-faulted source holding a descendant of the repudiated branch records the
  resolved repair, screens the loser, and performs no transition.
- **N6** `conflicting_receipt_pair` rejects each of its five conditions individually and
  `check_evidence` each of its four, with an independent positive oracle for the otherwise
  identical valid pair. `ResolvedRepair::verify` still rejects a wrong role, a foreign enclosing
  document and a mismatched stored sequence.
- **N7** `resolve_repair`, across **both** wedged shapes. (a) losing `high_water`. (b) AG3-DES-011's
  shape: `high_water = R(e-1)`, losing `in_flight = L(e)`, winner `S(e)` held only by a peer. In
  both, assert `canonical_head() == S(e)`, that `published()` still reports only what this device
  published, that `prepare` then accepts the receipt at `e+1` which it rejects as a gap beforehand,
  and that the journal round-trips at version 2. Also: `Ok(false)` on an unrelated journal, never
  setting `in_flight`, and refusal of an unverified or non-naming repair.

Store:

- **N8** Owner record v3 round-trips with journal, close and repair; an old-format reader rejects
  it; a corrupt repair section errors without resetting the journal; oversized, non-regular and
  copied records never reset a decision.
- **N9** `prepare_epoch_repair` is idempotent on an exact retry, refuses a different pending repair
  and a non-increasing sequence, reconciles the journal and drops a close bound to the loser in the
  same write. `mark_epoch_repair_applied` refuses a stale hash and rewrites on an exact retry.
- **N10** Sequence seeding from `ReceiptBook::repair_sequence()` when the record is absent.
- **N11** Full replacement on a real store: two conflicting proved receipts produce Fault; the
  repair produces `AwaitingSeed`; the fetched seed produces `Installed`; the losing projection is
  readable through the recovery control; the installed source's **actual projection, doc id,
  opening receipt, receipt evidence and stored bytes** match the selected checkpoint.
- **N12** AG3-DES-004: rewind without a seed, then retry with a seed. The retry must **continue**
  to installation, not report `AlreadyRepaired`, and must reuse the same snapshot id and deadline.
  Every retry row of 5.3 step 4 flushes.
- **N13** `Repaired` with no replacement: one source write, no staged recovery, pending ledger and
  overlay branch byte-identical.
- **N14** Crash at each of B1 to B6 through the existing writer seams; reopen from sealed bytes;
  exact retry; at each point assert which of the two legal states holds and that no third exists,
  including that the capability is reacquired rather than inferred.
- **N15** Both retained slots full: the repair holds in `RecoveryPending` with the branch retained
  and the state truthfully labelled `Repairing` after B2 and `Fault` before it; after
  acknowledgement it completes with the original deadline.
- **N16** Newcomer with `Unknown` tenure receives the repair, applies none, stays provisional, keeps
  its local work, and converges on the owner's fresh proof.
- **N17** AG3-DES-009's full historical path, with **no injected local state on B**: owner A
  equivocates with valid A1 and A2 and faults peer P only; ownership moves to B; B holds **neither**
  member; P transfers the exact pair over the v2 query; B's record freezes it while B's own source
  stays healthy and untouched by any seal; B explicitly chooses and signs v2 using its
  independently observed B tenure; P applies it, leaves Fault through case 5, and then converges by
  ordinary discovery of B's checkpoint. Also assert that before the repair, B's legitimate first
  B-tenure receipt does **not** clear P's fault (R14), which is the fact that makes this path
  mandatory. Separately: a v2 record signed in A's first tenure is refused after A returns; a v1
  record is always refused.
- **N18** Mismatched named pair holds, leaves the fault unchanged, and does not touch an unrelated
  active fault on another document.
- **N19** Interrupted Prepared overlay on a faulted document: the repair and the report both
  resolve it first; a `Hold` refuses both and retains everything.
- **N20** The repair retires zero intents from a ledger holding an **eligible ordinary
  full-envelope entry** alongside retained overlay entries.
- **N21** Registry bucket fault and repair; a newcomer then discovers an Index through that bucket.
  Run for Index and Flipnote.
- **N22** `StorageRefused` at a maximal source whose whole-version snapshot exceeds 6 MiB.

Sync, app and native:

- **N23** A served answer carries the repair exactly under the 6.6 predicate: not after B1 alone,
  yes after B2/B3, from both the record and the book origins, and only after the required flush.
  `encode_answer` rejects a foreign-document repair.
- **N24** `complete_checkpoint_hint` returns a hint carrying a repair, still refuses one carrying a
  proof, and the app discards a repair for a document it has no fault for.
- **N25** `select_repaired_checkpoint` fails closed cross-tenure, succeeds same-tenure, and spends
  one retained slot and three paced attempts.
- **N26** AG3-DES-001 acceptance, with no injected owner state: roll back the owner's
  `.owner-receipts` record to an earlier sealed copy, let the owner issue a second receipt, let a
  peer holding the first discover the conflict from a head answer, deliver the report over the v2
  query, and assert the **owner's own durable Fault**, then the explicit choice, then convergence.
  Also assert: a v1 query still works and carries no report; `count == 1` is rejected; "v2 with no
  report" has exactly one encoding; a duplicate report is a no-op; a report from a removed member is
  refused; a **historical** pair is admitted to the record without touching the source or the seal;
  a third receipt never replaces a frozen pair; the receipts are not decoded before the
  per-requester rail is charged; per U-7, admission precedes the response, no proof is served for
  either member of a newly recorded pair, and a failed or uncertain fault write fails closed for
  authoritative head service while still permitting an unproven hint.
- **N26b** The same report acceptance for a **Registry** bucket, over the Registry v2 encoding, then
  a newcomer discovering an Index through that repaired bucket.
- **N27** AG3-DES-005: the only holder of the selected checkpoint is itself faulted. Assert that it
  serves no seed while faulted, that the repair reaches it, and that after its B2 it serves the seed
  so the owner and the remaining peers converge.
- **N28** AG3-DES-006 and AG3-DES-011: run the head selector and the next ordinary issuance against
  N7's shape (b), `high_water = R(e-1)` with a losing `in_flight = L(e)` and the winner held only by
  a peer. Assert the **actual served receipt and proof** are `S(e)` and not `R(e-1)`, a newcomer
  installing from that proof, restart, and that the owner then issues at `e+1`. Repeat for Registry.
- **N29** Two real peers enter Fault from two valid conflicting receipts, the owner repairs, both
  converge, a third peer joins after the repair, and every peer's installed state, receipt evidence,
  recovery contents and native events agree. `settlement-changed` carries `fault`, then `repairing`,
  then the real phase.
- **N30** Custody and fairness: pause a real repair reconstruction in S2 while authoritative
  discovery, page receive and a second server complete; assert the permit is still owned, that
  cancellation does not refund it, that a dropped native handle releases it, and that after all
  transient and result owners drop, a fresh scan sees the expected reference lifetime with
  job-owned protection already transferred to the conservative set.

### 15.2 Isolated mutations

Each mutant compiles, removes exactly one guard, must make one named executed assertion fail, must
restore the source byte-for-byte, and the restored test must pass. The boundary each proves is
stated. No useful redundant validation is deleted to manufacture a failure.

| # | Boundary proved | Mutant | Intended failing assertion |
|---|---|---|---|
| M1 | book and gate atomicity (I-1) | `commit_repair` takes the gate lock, commits the book, releases, then re-acquires to set the phase, with a deterministic interleaving hook between | N1: an observation at the hook shows the book unfaulted while the gate reports `Fault`; and a forced failure of the second half leaves both changed |
| M2 | recovery precedes replacement (I-2) | **Corrected:** minting on the `Err` path leaks nothing because the caller still receives `Err`. The mutant instead weakens the "the durable save returned successfully" predicate so a failed recovery barrier is converted into `Ok(Some(capability))`, making B6 reachable | N11 variant with an injected recovery write failure: "no successor bytes were written" fails |
| M3 | the shared live-authority guard (I-3) | remove the `issuer_tenure_start_group_epoch == Some(expected)` clause **inside `ReceiptRepair::verify_current_owner`**, the one guard both layers reach | N17 with a correctly signed v2 record whose claimed start is a genuinely earlier tenure of the same key, against a different observed current start: the refusal fails |
| M4 | core retry ordering | move the retry shortcut in `ReceiptBook::apply_repair` before `verify_current_owner` | the **core** returning-owner retry regression, not the store-level N17, which is separately labelled redundant for this claim |
| M5 | the checker boundary | remove the pair-equality condition inside `check_evidence`, using pairs that **share the selected receipt** so membership cannot mask it | a direct `check_evidence` unit assertion. `apply_repair`'s own sorted-hash comparison is retained and labelled redundant for this claim |
| M6 | no retirement (I-5) | insert a receipted-retirement call into the repair transaction | N20: the eligible ordinary full-envelope entry survives |
| M7 | save before serve (I-6) | remove the required flush before serving | N23 with visible-but-uncertain post-rename state and a failed required reflush, asserted for both origins and for a non-proof peer answer |
| M8 | continuation versus completion (I-7) | map `AlreadyResolved` to terminal `AlreadyRepaired`, which is revision 1's actual bug | N12: after the retry with a seed, the successor is not installed and the losing projection is not readable |
| M9 | the single eligibility guard | remove the `eviction_pending` refusal in `stage_studio_repair_recovery` before the mint | N15: no successor bytes are written while the warning is pending and before its deadline. A due promotion without acknowledgement is a separate positive control |
| M10 | losing-baseline screening (I-9) | remove `is_repaired_loser`'s losing-baseline arm | a covered losing-baseline descendant remains `Stale` **without re-faulting**. (Revision 1's assertion here was wrong and is withdrawn) |
| M11 | no over-suppression (I-9) | make `is_repaired_loser` return true for any same-tenure receipt | a genuine third differing baseline still faults |
| M12 | report admission (I-10) | **Corrected:** the old mutant was masked, because the book independently returns `Fault`/`Stale` whatever the report layer does. The report layer's own unique guard is the **frozen-pair replacement refusal in the owner record** (5.2), which core never sees because the owner's book is not involved on that path. The mutant removes it | N26: after a second report naming a different pair, the recorded evidence and the offered candidates are unchanged, **and** the no-op path reaches no source writer, reservation or seal |

### 15.3 Harness

New tests in `crates/catcoms-replication/src/epoch/repair_state/tests.rs`, new
`store/epoch_studio/tests/repair.rs`, `store/epoch_registry/tests/repair.rs`,
`crates/catcoms-sync/src/receipt_head/tests/` for W-1, and
`crates/catcoms-app/src/studio_exchange/tests/repair.rs` for the two-peer actor, report and native
scenarios, reusing the existing deterministic actor fixtures rather than injecting private state.
N26 in particular must not pre-inject the owner's fault. Local runs stay serial with `-j 1`;
platform and matrix runs go to GitHub, with a `studio-repair` job proposed to Agent 4 so these
scenarios cannot remain opt-in. No workflow is edited by this scope.

### 15.4 Proposed UI-hooks row, for Agent 4

> Actual `phase:"fault"` and `fault` invalidations are available. `studio_fault_read` returns both
> conflicting candidates, which one the local version descends from, how many operations a
> replacement preserves, and whether this client may decide. `studio_fault_repair` is available
> only to the actual current owner with an observed tenure; every other client sees
> `mayDecide:false` and a reason. `settlement-changed` adds `repairing`, emitted only after the
> durable transition and kept while recovery or installation is outstanding, and `storageRefused`.
> Restore and Copy still save ordinary content in an Open target and cannot clear a fault. A repair
> preserves the losing version in recovery before any replacement and never retires local work.

## 16. Remaining open questions

U-1 to U-8 are answered and adopted (section 0). U-7 was decided **against** revision 2's proposal:
admission precedes the response, with no proof for a disputed receipt and fail-closed authoritative
head service on an uncertain write. U-8 confirmed revision 2: no pre-B2 `Repairing`, and pre-B2
progress is a subordinate field inside the Fault view rather than a durable settlement state.

Two smaller questions are opened by this revision:

**U-9. Should a non-owner peer also keep an `EpochFaultRecord`?** Proposed no: a peer's evidence
already lives in its faulted book, and the record exists because a healthy **owner** source has
nowhere to put a pair. The cost is that a peer which is not itself faulted cannot forward a
historical pair it merely observed; only a faulted peer reports. If the reviewer wants any member
to be able to relay evidence, the slot becomes general and its abuse surface needs re-examining.

**U-10. How long may an unresolved `EpochFaultRecord` sit before it is reported to the user as
requiring a decision?** Proposed: it is surfaced immediately in `StudioFaultView` with
`may_decide` true, and there is no expiry, because expiring evidence would silently restore the
ability to serve a disputed head. The cost is one permanently visible prompt per unresolved fault.

## 17. Re-review request

Copyable, with the common contract from
[GATE4-REVIEW-PREAMBLES](GATE4-REVIEW-PREAMBLES.md#common-contract-for-the-four-subsequent-reviews)
sent alongside it.

```text
Review type: design, revision 3, findings re-review.
Base: 63a11e1a6451c7ed373c90b0e81b59d8a748a72a (revision 2). Head: [FULL_HEAD_SHA once pushed].
Compare: https://github.com/Thalpy/Mewtual/compare/63a11e1a6451c7ed373c90b0e81b59d8a748a72a...[FULL_HEAD_SHA]
Earlier revisions: 7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a (revision 1), original scope base
1bcb1bca204d721b848b17c0835faf931ae930e3.
Scope/evidence: docs/GATE4-AGENT-3-DESIGN.md revision 3 and docs/GATE4-AGENT-3-STATUS.md.
Documentation only: no production code, test, shared contract document or workflow is changed, and
no Cargo command was executed in any of the three passes. Every number is a source constant or a
labelled estimate. The commit sits on a branch shared with Agents 1 and 2; it touches only the two
Agent 3 documents.
Dependencies unchanged: tenure seam is Agent 2's section 13.2 contract, with a design of record but
no implementation or accepted review; Agent 1's runtime integration is separately unreviewed; the
core signing split at e65bfd8 remains unreviewed.

This revision answers AG3-DES-009 to AG3-DES-012, corrects mutants M2 and M12, and adopts your U-7
decision against my own proposal and your U-8 confirmation. Section 0 is the disposition table. It
reopens nothing you accepted: AG3-DES-003's positive head justification, AG3-DES-005's B2 serving
rule, AG3-DES-007's scoped capability and AG3-DES-008's custody split are unchanged.

AG3-DES-009 is confirmed at the source and was worse than a contract gap. R14 records that
ingest_verified, ingest_adoption and check_opening_receipt each return Fault before considering the
incoming receipt, so NO receipt from any owner or tenure can clear a fault and a repair is the only
exit. Revision 2's cross-tenure convergence story is therefore withdrawn as false. The corrections:
reports now carry the complete frozen pair, validated historically by the repair-independent
checker; a historical pair lands in a new owner-side EpochFaultRecord and never goes through
StudioEpoch::seal, whose current-owner verification is exactly the confusion you warned about; the
owner's own source is untouched in that case; and cross-tenure repair is a distinct case 5,
Unblocked, that installs nothing and returns the source to its opening so ordinary discovery of the
current owner's checkpoint can converge it. N17 runs A faults P only, ownership moves to B, B holds
neither member, P transfers the exact pair, B chooses and signs under its own observed tenure, P
heals, with no injected local state on B, and it asserts that B's legitimate first receipt does not
clear P's fault beforehand. Please check whether any reachable shape still has no exit, and whether
case 5's restored book and gate satisfy the ordinary restart validator in every shape including a
faulted epoch-zero source.

AG3-DES-010: case 2 splits into 2a, ordinary seal mode with no replacement, and 2b, adoption mode
where installation is still required; and separately C-5 makes the committed repair_state() the
sole terminality oracle after B2, so the plan's install field can no longer contradict
install_pending at all. N5 asserts different committed outcomes for the two modes rather than that
both restore as Closing. Please check that the contradiction is now unrepresentable rather than
fixed in one case, and that a cross-tenure source can never be left claimed for a forbidden install.

AG3-DES-011: your finding was right and the shape you named is the one revision 2 missed. The
journal gains a distinct reconciled canonical decision, deliberately not high_water and not
published(), with canonical_head() used by prepare_verified's adjacency and by the head selector's
own_choice, restored across a version-2 journal. Section 5.1 C-7 also states the size arithmetic
honestly and raises MAX_OWNER_RECEIPT_JOURNAL_BYTES rather than relying on about 128 bytes of
headroom. N7 and N28 both use high_water = R(e-1) with a losing in_flight = L(e) and a peer-held
winner, through the actual head selector and the next ordinary issuance.

AG3-DES-012: both the Studio and Registry v2 encodings are specified, the tag is an unambiguous 2
with a counted report list of 0 or 2 so "v2 with no report" has one canonical form and a count of 1
is rejected, and the parse and charge order is corrected against the real seam. Since the current
service decodes the scoped query before the per-requester charge, the decode is split so the report
stays an opaque bounds-checked slice until after that rail, leaving the accepted ordering of the
existing steps untouched. Section 12 states why the Registry route ships with the Studio one.

U-7 adopted as you decided: admission precedes the response, a successfully admitted report yields
no authoritative proof for either member of the newly recorded pair and none at all while the local
source is Faulted, and an uncertain fault or record write fails closed for authoritative head
service while still permitting an unproven hint. This removes revision 2's lifetime-permitting
liveness hole. U-8 kept: no pre-B2 Repairing; pre-B2 progress is a subordinate field in the Fault
view.

Mutants: M2 now weakens the "durable save returned successfully" predicate into Ok(Some(capability))
rather than minting on an Err path that leaks nothing. M12 now targets the frozen-pair replacement
refusal in the owner record, which core never sees because the owner's book is not involved on that
path, and additionally asserts that the no-op path reaches no source writer, reservation or seal.
Please say whether both now fail at the boundary they claim, and whether the other ten still do.

Section 16 opens two smaller questions: U-9, whether a non-owner peer should also keep an evidence
record so any member can relay a historical pair, and U-10, whether an unresolved record should
ever expire.

Return a design verdict for this bounded repair scope, or numbered findings with concrete failure
paths and required corrections. Implementation, integration and full Gate 4 remain separate.
```

