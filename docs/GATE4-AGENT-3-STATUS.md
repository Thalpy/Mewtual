# Gate 4 Agent 3 status: runtime signed fault repair

Owner: Agent 3 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair)).
Proposal: [GATE4-AGENT-3-DESIGN](GATE4-AGENT-3-DESIGN.md), currently revision 8.
Review preamble: 3. Current entries override older ones.

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a` | design, docs only | **REQUEST CHANGES**: AG3-DES-001 to AG3-DES-008, AG3-TEST-001; U-1 to U-6 decided |
| 2026-09-15 | Design revision 2, findings answered | `7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a` | `63a11e1a6451c7ed373c90b0e81b59d8a748a72a` | design, docs only | **REQUEST CHANGES**: AG3-DES-003, 005, 007, 008 corrections **accepted**; new AG3-DES-009 to AG3-DES-012; M2 and M12 non-isolating; U-7 and U-8 decided |
| 2026-09-16 | Design revision 3, second-round findings answered | `63a11e1a6451c7ed373c90b0e81b59d8a748a72a` | `a62178b94f20cd60a5363e6a3d6d6216edb9e516` | design, docs only | **REQUEST CHANGES**: historical-pair direction, 2a/2b split, v2 framing, U-7, U-8 and M12 **accepted**; new AG3-DES-013 to AG3-DES-016; M2 still masked; U-9 and U-10 decided |
| 2026-09-16 | Design revision 4, third-round findings answered | `a62178b94f20cd60a5363e6a3d6d6216edb9e516` | `ad023d2e5b514a8f9598b1fe433fd2b080ecad6c` | design, docs only | **REQUEST CHANGES**: C-8 epoch-zero fix, reconciled lifecycle, healthy-source clobber fix and M2 **accepted**; new AG3-DES-017 to AG3-DES-021; AG3-TEST-003 on M12/N24/N5d; U-11 agreed |
| 2026-09-16 | Design revision 5, fourth-round findings answered | `ad023d2e5b514a8f9598b1fe433fd2b080ecad6c` | `3737f1d4fff6f2c08302b0ecb1b024008818a1cf` | design, docs only | **REQUEST CHANGES**: AG3-DES-021 **closed**; case 6c/N32, the sequence concept and M2 accepted; new AG3-DES-022 to AG3-DES-025; AG3-TEST-004 on N5c/N18/M12 |
| 2026-09-16 | Design revision 6, fifth-round findings answered | `3737f1d4fff6f2c08302b0ecb1b024008818a1cf` | `135766ca9f290ab96d3133b771bc42da74fe7825` | design, docs only | **REQUEST CHANGES**: AG3-DES-023 and AG3-DES-024 **closed**; fresh-path AG3-DES-025 and the AG3-TEST-004 corrections accepted; new AG3-DES-026 to AG3-DES-029; AG3-TEST-005 |
| 2026-09-16 | Design revision 7, sixth-round findings answered | `135766ca9f290ab96d3133b771bc42da74fe7825` | `6d498c2e901e5071104a2533e0a632dc5676b2a7` | design, docs only | **REQUEST CHANGES**: AG3-DES-029 **closed**; source-bound encoding shape and M14 accepted; new AG3-DES-030 to AG3-DES-033; AG3-TEST-006 |
| 2026-09-16 | Design revision 8, seventh-round findings answered | `6d498c2e901e5071104a2533e0a632dc5676b2a7` | this commit | design, docs only | re-review requested |

Working checkout: `M:\Git (local)\CatComs`, shared with the parallel Agent 1 and Agent 2 sessions,
which are now doing implementation and design work respectively. Agent 1 has the checkout on its
own branch, so this pass commits there rather than switching branch, touching only the two Agent 3
documents with explicit pathspecs. `Create-suite-2` was fast-forwarded once, at revision 2, to keep
these documents off Agent 1's branch alone; it has not been moved since. **Agent 3 implementation
must move to a separate branch or worktree before any code change**; no mutation harness may run
against another agent's source.

## Finding ledger, revision 7 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-030 | P1 | **Answered in revision 8.** `apply_repair` clears `fault` on success, so "inline hashes must match the live fault" could only hold before B2 and stranded every resume afterwards. Binding is now two-phase, against `fault_evidence()` pre-B2 and against the committed `repair_state()` pair post-B2, with a missing or corrupt source refusing in either phase. | Design 3 R29, 5.2, 15.1 N36(f)(g) |
| AG3-DES-031 | P1 | **Answered.** `repair_install_pending()` starts after B2, so a B1-persisted repair was unfenced and a current-tenure report could degrade a pending case 6c replacement into a terminal 6d screening. One `repair_transaction_nonterminal` predicate spans B1 to recycling. | Design 6.5, 15.1 N37(b), M13, M15 |
| AG3-DES-032 | P1 | **Answered.** A reserved live-conflict slot closes the maximal-capacity state, and one durable `authoritative_proof_allowed` predicate is consumed on every head request rather than only on the admitting exchange. | Design 5.2, 6.6, 15.1 N37(c)(d), M16 |
| AG3-DES-033 | P1 | **Answered on all three points.** Disposition is defined by the operation performed with three values covering every C-2 case; snapshot v3 keeps the adoption bit explicit and binds the tag to the exact repair; and `RepairTransition` plus every successor path carries it, since `adopted_successor` copies only the book. | Design 3 R30, 5.1 C-2, C-5, 15.1 N38 |
| AG3-TEST-006 | P2 | **Answered.** N36 gains post-B2 resumes, N37 becomes four variants including pre-B2 and maximal capacity, N38 covers the full disposition lifecycle, M13 is widened and M15 and M16 added. | Design 15.1, 15.2 |

Closed in the revision-7 round: **AG3-DES-029**. Accepted and not reopened: the source-bound
encoding shape, M14, and M1 to M11.

## Finding ledger, revision 6 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-026 | P1 | **Answered in revision 7.** Revision 6's own headline fix could not cross B1: the codec required a repair to match a retained pair while the correction said the source's pair is never retained. The repair is now tagged, kind 2 being source-bound with its pair **inline**, which keeps the record self-validating given that `decode` sees only the logical document. Bound follows to seven receipt-sized values. | Design 3 R26, 5.2, 10.1, 15.1 N36 |
| AG3-DES-027 | P1 | **Answered.** `Closing -> Fault` is permitted and `repair_install_pending()` requires `Closing`, so a current-tenure report through the live seal would silently abandon an outstanding recovery and replacement. A nonterminal repair now fences report-induced source mutation; proof stays suppressed meanwhile. | Design 3 R27, 6.5, 15.1 N37, M13 |
| AG3-DES-028 | P2 | **Answered.** A disposition tag is persisted in the Studio restart unit's snapshot (version 3), not in core's tested `ResolvedRepair` codec, so an exact retry after a screening application returns `Screened` rather than the false `Repaired`. | Design 5.1 C-5, 5.3, 15.1 N38 |
| AG3-DES-029 | P1 | **Answered.** `is_repaired_loser` is an admission guard, not proof that a different frozen pair is resolved; the old rule would have stranded a peer frozen on `{R1,R3}` after `{R1,R2}` was repaired. The no-op is now the exact pair only. | Design 3 R28, 6.5, 15.1 N39, M14 |
| AG3-TEST-005 | P2 | **Answered.** N36 to N39 cover the four new boundaries; M13 and M14 mutate the two guards this revision adds. | Design 15.1, 15.2 |

Closed in the revision-6 round: **AG3-DES-023** and **AG3-DES-024**. Accepted and not reopened: the
fresh-path half of AG3-DES-025, the AG3-TEST-004 corrections, M2 and M1/M3-M11.

## Finding ledger, revision 5 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-022 | P1 | **Answered in revision 6.** `ReceiptBook.fault` is itself one durable slot, so the source's own pair never needed a record copy; requiring it to be "retained here" is what let two historical pairs crowd out the pair actually blocking the document. The record now holds external pairs only and the derivation falls back to the book. The missing terminal-pair recycling is specified as an explicit, idempotent, crash-safe removal. | Design 3 R25, 5.2, 15.1 N31b, N31c |
| AG3-DES-023 | P1 | **Answered.** Three different bounds coexisted: the new five-receipt formula, the superseded three-receipt paragraph in the same section, and 11.4 KiB in section 10.1. One canonical formula now, stated once, with 10.1 agreeing and the struct display corrected. | Design 5.2, 10.1 |
| AG3-DES-024 | P1 | **Answered.** Core verifies current owner before its retry shortcut on purpose; revision 5 inverted that inside `plan_repair`, an authority-bearing API. Authority and evidence now precede every retry, sequence and in-progress outcome, with the store check named as defence in depth rather than the safety property. | Design 5.1 C-2, 15.1 N35 |
| AG3-DES-025 | P2 | **Answered.** Case 6d leaves a different fault standing, so a distinct `Screened` outcome and pair-specific barrier and crash prose separate a terminal disposition from a usable document. | Design 5.3, 8, 15.1 N33 |
| AG3-TEST-004 | P2 | **Answered.** N5c narrowed to the same-baseline higher head; N18 split into malformed-refuses and valid-unrelated-screens; M12's fixture qualified as historical throughout; Flow D's stale "case 6b" wording replaced by deference to the classification. | Design 7, 15.1 N5c, N18, 15.2 M12 |

Closed in the revision-5 round: **AG3-DES-021**. Accepted and not reopened: case 6c with N32 for
AG3-DES-017, the monotone sequence concept subject to AG3-DES-024's ordering fix, M2, and
M1/M3-M11.

## Finding ledger, revision 4 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-017 | P1 | **Answered in revision 5.** "Not faulted" is not "safe to leave unchanged": neither `prepare_checkpoint_adoption` nor `prepare_settlement` screens the receipt it already holds, so a source sitting on the loser would install or settle it. New case 6c retargets any source whose own `latest`, opening or adoption target is covered; 6b now requires no covered anchor; the same-baseline higher-head limitation is asserted rather than papered over. | Design 3 R21, 5.1 C-2, 15.1 N32 |
| AG3-DES-018 | P1 | **Answered.** The extension is re-specified for two pairs plus one repair with canonical ordering and alias rejection, the bound raised to five receipt-sized values (the old one made the maximal state unwritable, since the reader caps before unsealing), and active status is **derived** on load rather than stored, so no atomic source-plus-record transition is implied. | Design 3 R22, 5.2, 15.1 N31 |
| AG3-DES-019 | P1 | **Answered.** A repair for a pair the source is not blocked on is screening-only (case 6d), not `PairMismatch`, so the historical repair completes and frees the slot. `Held(RepairInProgress)` protects the single `resolved_repair` slot while a replacement is outstanding, released on terminal state rather than on `applied`. | Design 3 R24, 5.1 C-2, 5.2, 15.1 N33 |
| AG3-DES-020 | P1 | **Answered.** Sequence and retry classification moved ahead of the fault and tenure tests and applies to every path; the non-faulted path had skipped `apply_repair` and its monotonicity guard while Flow D began applying repairs regardless of fault status. | Design 5.1 C-2, 15.1 N34 |
| AG3-DES-021 | P2 | **Answered.** Journal v2 derives document and tenure identity from the effective retained set including `reconciled`; the existing derivation from `high_water.or(in_flight)` would reject the shape C-7 permits and N7 requires. | Design 3 R23, 5.1 C-7 |
| AG3-TEST-003 | P2 | **Answered.** M2 accepted unchanged. M12 retargeted to both slots occupied plus a third report, since a second distinct pair now legitimately fills the free slot. N24's discard claim withdrawn; N5d's "stored bytes unchanged" replaced with an assertion that can actually hold. | Design 15.1 N5d, N24, 15.2 M12 |
| U-11 | decision | **Agreed by the reviewer and adopted**: only a faulted reporter may fill the second slot. | Design 5.2, 16 |

Accepted in the revision-4 round and not reopened: C-8's epoch-zero restore fix, the reconciled
publication lifecycle, the healthy-current-tenure half of AG3-DES-013, and M2.

## Finding ledger, revision 3 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-013 | P1 | **Answered in revision 4.** Case 5 matched "cross-tenure, any shape", so an owner applying a historical repair to its own healthy source would reopen a sealed epoch and lose newer progress. Fault status now classifies before tenure; a source not faulted on the named pair reaches only the screening case. Flow D no longer discards repairs for non-faulted documents, which is what made case 6 unreachable. | Design 5.1 C-2, 7 Flow D, 15.1 N5c, N5d |
| AG3-DES-014 | P1 | **Answered, and revision 3's claim was wrong.** `decode_mode` derives the document from `latest` alone and `ResolvedRepair::verify` requires equality, so the epoch-zero shape failed `ReceiptConflict`. C-8 widens that derivation for versions 4 and 5 only. | Design 3 R17, 5.1 C-8, 15.1 N2b |
| AG3-DES-015 | P1 | **Answered.** Revision 3 defined only the creation of `reconciled`. Promotion on completion, retirement on supersession, clearing on tenure change, inert stale retries and decode constraints are now specified; without promotion, `complete_studio_head` had no valid path for the reconciled winner at all. | Design 3 R18, 5.1 C-7, 15.1 N28 |
| AG3-DES-016 | P1 | **Answered.** A single active pair now spans the source fault and the owner record, with a source fault always active and one bounded deferred slot, so sequential repair works in both orderings. Ordinary receipt issuance is explicitly not blocked. | Design 5.2, 6.5, 15.1 N31 |
| AG3-TEST-002 | P2 | **Answered, and the masking is confirmed at the source**: `reserve` sets `ready = false` and a failed write never commits, so the successor `reserve` fails on `Reconcile` regardless. M2 now bypasses the recovery save before it reserves. | Design 3 R20, 15.2 M2 |
| U-9 | decision | **Reviewer's answer adopted**: `EpochFaultRecord` stays owner-only. | Design 5.2, 16 |
| U-10 | decision | **Reviewer's answer adopted**: no automatic expiry. | Design 5.2, 16 |

Accepted in the revision-3 round and not reopened: the historical-pair report direction, the 2a/2b
adoption split with the committed-state terminality oracle, the Studio and Registry v2 framing and
parse-order correction, U-7, U-8, and the retargeted M12.

## Finding ledger, revision 2 round

| Finding | Severity | Status | Where answered |
|---|---|---|---|
| AG3-DES-009 | P1 | **Answered in revision 3, and the diagnosis is confirmed at the source.** `ingest_verified`, `ingest_adoption` and `check_opening_receipt` all return `Fault` before considering the incoming receipt, so a repair is the only exit and revision 2's "the new owner's first receipt converges it" was false. Reports now carry the complete frozen pair, historical pairs are admissible and land in an owner-side evidence record rather than through the live seal, and cross-tenure repair is a distinct `Unblocked` transition that installs nothing. | Design 3 R14, 5.1 C-2 case 5, 5.2, 5.6 W-1, 6.3, 6.5, 15.1 N5b, N17 |
| AG3-DES-010 | P1 | **Answered.** Case 2 splits into ordinary-seal and adoption modes, and the committed `repair_state()` becomes the sole terminality oracle after B2, so the plan can no longer contradict `install_pending`. A cross-tenure source is never claimed for a forbidden install. | Design 5.1 C-2, C-5, 5.3 S-2 step 6, 15.1 N5 |
| AG3-DES-011 | P1 | **Answered.** Clearing a losing `in_flight` returns the owner to its older high water for both selection and adjacency, so the journal gains a distinct `reconciled` canonical decision and `canonical_head()`, deliberately not the publication bit. The size arithmetic is stated and the journal bound raised rather than relying on ~128 bytes of headroom. | Design 3 R15, 5.1 C-7, 6.7, 15.1 N7, N28 |
| AG3-DES-012 | P2 (P1 for Registry) | **Answered.** Studio and Registry v2 encodings both specified, tag is an unambiguous `2` with a counted report list of 0 or 2, and the parse and charge order corrected against the real seam by splitting the decode so the report stays opaque until after the per-requester rail. | Design 3 R16, 5.6 W-1, 12, 15.1 N26, N26b |
| M2 non-isolating | P2 | **Answered.** The mutant now weakens the "durable save returned successfully" predicate into `Ok(Some(capability))`; minting on an `Err` path leaked nothing. | Design 15.2 M2 |
| M12 non-isolating | P2 | **Answered.** Retargeted at the frozen-pair replacement refusal in the owner record, a guard core never sees, plus a report-boundary side-effect assertion. | Design 15.2 M12 |
| U-7 | decision | **Reviewer's answer adopted over my proposal**: admit before answering, no proof for a disputed receipt, fail closed on an uncertain write. | Design 6.5, 6.6 |
| U-8 | decision | **Confirmed**: no pre-B2 `Repairing`. | Design 5.7, 6.4 |

Accepted in the revision-2 round and not reopened: AG3-DES-003, AG3-DES-005, AG3-DES-007 and
AG3-DES-008.

## Finding ledger, revision 1 round

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
| "A cross-tenure fault converges through the current owner's ordinary first receipt" (rev 2) | **Wrong, and the most serious error in revision 2.** Every admission path returns `Fault` before considering the incoming receipt, so no receipt from any owner or tenure can clear a fault. A repair is the only exit. |
| "A report naming an older tenure may be refused" (rev 2) | **Withdrawn.** Combined with the above, that rule would have made cross-tenure repair permanently unreachable, which is the case the v2 record format exists to serve. Historical pairs are admissible; the authority question lives with the repair, not the report. |
| "One reported receipt is enough" (rev 2) | **Wrong for the historical case.** A new owner may hold neither member, so the reporter, which is faulted and holds both, always sends both. |
| "The per-requester rail and pending cap precede the v2 decode" (rev 2) | **Wrong.** `queue_checkpoint_head` decodes the scoped query before charging the per-requester rail, so the decode is split instead. |
| "Clearing a losing `in_flight` reconciles the journal" (rev 2) | **Insufficient.** Both the head selector and `prepare_verified` key off the retained high water, so the owner falls back to an older receipt and reads its next one as a gap. |
| "Cross-tenure case 5 applies to any shape" (rev 3) | **Wrong.** It would let a historical repair reopen a healthy current-tenure source and drop its legitimate head. Fault status must classify first. |
| "Case 5's epoch-zero shape satisfies restart" (rev 3) | **Wrong.** `decode_mode` derives the document from `latest` only, so a resolved repair with no head fails `ReceiptConflict`. Asserted without checking the decoder. |
| "A repair for a document with no local fault is discarded" (rev 3) | **Contradicted its own case 6.** Distribution has to reach the screening path or the case is dead code. |
| "`reconciled` plus `canonical_head()` finishes the journal fix" (rev 3) | **Incomplete.** Creation without promotion, retirement, tenure clearing or decode constraints leaves the reconciled winner impossible to complete and eventually stale. |
| "M2 is isolated once it mints on a lie" (rev 3) | **Still masked.** The failed accounted write leaves the budget unready, so the successor write fails on `Reconcile` before the intended assertion can fail. |
| "A non-faulted source is safe to leave unchanged" (rev 4) | **Wrong.** Neither installer screens the receipt it already holds, so a source sitting on the loser installs or settles the repudiated branch. Not-faulted is not the same as not-on-the-loser. |
| "`active`/`deferred` is specified" (rev 4) | **Only in prose.** The persisted encoding stayed singular and the bound stayed sized for three receipt-sized values, so the maximal two-pair state could not be written or reopened. |
| "A source fault always becomes the active pair" (rev 4) | **Wrong once a repair is durable.** It strands a persisted B1 decision behind a later unrelated fault. A pending repair owns the target; a source fault waits its turn. |
| "A repair for a different pair is `PairMismatch`" (rev 4) | **Wrong.** It is simply not about this source's blocker. Screening-only application is what removes the deadlock. |
| "The non-faulted path only needs authority and evidence checks" (rev 4) | **Wrong.** It bypassed `apply_repair`'s `repair_sequence` monotonicity guard, so a delayed lower-sequence repair could overwrite a newer one. |
| "N5d asserts stored bytes are unchanged" (rev 4) | **Self-contradictory.** Case 6b durably adds repair evidence; the assertion had to be narrowed to non-repair content. |
| "A source fault's pair must be retained in the record to be selected" (rev 5) | **Wrong, and the record could not represent the result.** Two retained historical pairs could crowd out the pair actually blocking the document, and its repair had nowhere valid to bind. The source's pair is already durable in the book and needs no slot. |
| "The other retained pair becomes active when a repair terminates" (rev 5) | **Incomplete.** Nothing removed the pair that just terminated, so already-resolved evidence could become active again. |
| "MAX_RECORD_BYTES ... 3 * MAX_RECEIPT_BYTES" (rev 5, second statement) | **Stale duplicate.** Revision 5 added the correct five-receipt formula but left the old one later in the same section and 11.4 KiB in section 10.1. |
| "Sequence and retry classify before anything else" (rev 5) | **Unsafe ordering.** Core deliberately verifies current owner before its retry shortcut; inverting that inside an authority-bearing API reintroduces the M4 bug one layer up. |
| "B2 means the fault has durably ended" (rev 5, after case 6d) | **False for the screening cases.** A terminal disposition is not the same as a usable document. |
| "The source's pair is never stored, and the repair may still name it" (rev 6) | **Unrepresentable.** The codec required a repair to match a retained pair, so the source-bound case the correction existed for could not be encoded at all. A source-bound repair now carries its pair inline. |
| "A nonterminal repair owns the target" (rev 6) | **Only against discovery.** The report path could still seal a current-tenure pair, move the phase to `Fault` and collapse `repair_install_pending()`, abandoning an outstanding replacement. |
| "`Screened` separates terminal from usable" (rev 6) | **Only on the fresh path.** No provenance was persisted, so an exact retry reported `Repaired`. |
| "A report whose members are already screened is a no-op" (rev 6) | **Over-broad.** `is_repaired_loser` governs receipt admission, not whether a different frozen pair is resolved; it would strand a peer on `{R1,R3}` after `{R1,R2}` was repaired. |
| "The inline pair must match the live `ReceiptBook::fault`" (rev 7) | **Only true before B2.** `apply_repair` clears `fault` on success, so the rule necessarily failed exactly when a resume was needed. |
| "`repair_install_pending()` is the ownership fence" (rev 7) | **Starts too late.** A B1-persisted repair is already nonterminal and owns the target, so a pre-B2 report could degrade a required replacement into a terminal screening. |
| "Deferred live evidence keeps proofs suppressed" (rev 7) | **Not durably.** There was no capacity for it at the legal maximum and no persistent term in the proof refusal, so a restart resumed proving a disputed head. |
| "`Transitioned` and `Screened` cover the dispositions" (rev 7) | **Incomplete.** Cases 6a and 6c mutate a source that was never Faulted and fitted neither value; the tag also had no place in the snapshot layout and was dropped by every successor path. |

## Facts established by the revision-8 audit

New this revision, verified in code:

- `ReceiptBook::apply_repair` sets `self.fault = None` and installs `resolved_repair` in the same
  mutation (`epoch.rs:1765-1774`), so a persisted source-bound repair cannot be validated against a
  live fault after B2.
- `adopted_successor` builds a fresh unit, copies `self.receipts` and marks the latest installed
  (`studio/epoch/adoption.rs:140-150`). Nothing Studio-layer crosses, so a disposition tag held
  outside the book is dropped by that path unless it is copied explicitly.
- `StudioEpoch::snapshot`'s leading byte is `if self.adopting { 2 } else { 1 }`
  (`studio/epoch.rs:432`) and is the only value telling restore whether to call
  `ReceiptBook::decode` or `decode_adoption`.

## Facts established by the revision-7 audit

Verified in code and still relied on:

- `transition_verified_receipt` permits `Open | Closing | Fault -> Fault` and clears the receipt
  hash (`epoch.rs:2539-2549`), so a report driven through the live seal collapses
  `repair_install_pending()`, which requires `Closing`.
- `EpochOwnerReceiptState::decode` and `check_scope` take only `(bytes, scope, document)`
  (`store/epoch_owner.rs:66-124`): no source is available, so a source-bound repair must carry its
  evidence inline to stay self-validating.
- `StudioEpoch::snapshot` already carries a version byte distinguishing ordinary from adopting
  (`studio/epoch.rs:432`), so the disposition tag extends the restart unit rather than core's
  already-tested `ResolvedRepair` codec.

## Facts established by the revision-6 audit

Verified in code and still relied on:

- `ReceiptBook.fault` is a single `Option<(Receipt, Receipt)>` alongside the single
  `resolved_repair` (`epoch.rs:1571-1575`), so the source holds exactly one unresolved pair and
  holds it whether or not any owner record mentions it.
- `ReceiptBook::apply_repair` calls `verify_current_owner` **before** its exact-retry shortcut, with
  the comment "Authority must precede the retry shortcut: a returning key is not its old tenure"
  (`epoch.rs:1735-1741`).

## Facts established by the revision-5 audit

Verified in code and still relied on:

- `prepare_checkpoint_adoption` requires only `adopting`, `Closing`, not faulted and
  `latest == receipt` (`studio/epoch/adoption.rs:89-95`), and `prepare_settlement` requires not
  `adopting`, `Closing`, not faulted and then acts on `receipts.latest()`
  (`studio/epoch/settlement.rs:71-79`). **Neither screens the receipt it already holds** against a
  resolved repair; `is_repaired_loser` only screens receipts arriving at admission.
- `OwnerReceiptJournal::decode` derives its document from
  `high_water.as_ref().or(in_flight.as_ref())` and then requires document presence to agree with
  tenure presence (`epoch.rs:2084-2094`), so a reconciled-only journal fails to restore.
- `ReceiptBook.resolved_repair` is a single `Option` (`epoch.rs:1575`) that `apply_repair`
  overwrites, so a second repair can erase the evidence an in-flight replacement depends on.

## Facts established by the revision-4 audit

Verified in code and still relied on:

- `ReceiptBook::decode_mode` derives its document as
  `latest.as_ref().map(|receipt| receipt.document.clone())` (`epoch.rs:1877`) and then calls
  `resolved.verify(document.as_ref(), ..)`, whose first condition is
  `document != Some(&self.repair.document)` (`epoch/repair_state.rs:43`). A resolved repair with no
  `latest` therefore fails `ReceiptConflict`.
- `OwnerReceiptJournal::mark_published` returns early only on an exact `high_water` match and
  otherwise requires `in_flight` to be present and equal (`epoch.rs:2043-2058`). A receipt that is
  neither has no completion path.
- `EpochStorageBudget::reserve` sets `self.ready = false` before returning
  (`store/epoch_budget.rs:376`) and a writer error returns without `commit()`, so every later
  `reserve` fails with `BudgetError::Reconcile`.

## Facts established by the revision-3 audit

Verified in code and still relied on:

- `ingest_verified` returns `Ok(ReceiptIngest::Fault)` on `self.fault.is_some()` before the
  repaired-loser screen, the tenure comparison and the high-water logic (`epoch.rs:1615-1619`);
  `ingest_adoption` does the same before its anchor search (`epoch/adoption.rs:27-29`); and
  `check_opening_receipt` returns early too (`epoch.rs:1687-1689`). **A repair is the only exit
  from Fault. No receipt from any owner or tenure can clear one.**
- `prepare_verified` requires a same-tenure receipt to be exactly one epoch beyond `high_water`
  (`epoch.rs:1993-1998`), and the head selector's `own_choice` is `pending().or(published())`
  (`store/epoch_studio/discovery.rs:229`), so clearing a losing pending decision returns the owner
  to its older high water for both selection and adjacency.
- `OwnerReceiptJournal::encode` is version 1 with `high_water`, `in_flight` and `tenure`
  (`epoch.rs:2061-2068`), and its decoder enforces a `(high_water, in_flight)` adjacency invariant
  (`epoch.rs:2105-2116`). Adding a third retained receipt leaves roughly 128 bytes of headroom
  against `MAX_OWNER_RECEIPT_JOURNAL_BYTES = 3 * MAX_RECEIPT_BYTES + 256`, so the design raises the
  constant rather than relying on that margin.
- `encode_scoped_query` delegates `CheckpointTarget::Registry` to `encode_query`, a different
  framing from the Studio channel/object one (`receipt_head/wire.rs:70-97`).
- `queue_checkpoint_head` charges the global preauth rail and authenticates, then calls
  `decode_scoped_query`, and only afterwards charges the per-requester rail
  (`receipt_head.rs:378-415`).

## Facts established by the revision-2 audit

Verified in code at the base and still relied on:

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

- Core: `EpochGate::commit_repair` (C-1); `ReceiptBook::plan_repair` taking the two full receipts
  explicitly, with `RepairSource`, `RepairTransition`, `RepairPlan`, `RepairHold`, and
  `StudioEpoch`/`RegistryEpoch` `apply_receipt_repair` committing atomically (C-2); the
  repaired-loser-is-not-an-anchor predicates (C-3); `conflicting_receipt_pair` plus
  `ReceiptRepair::check_evidence` layered on it (C-4); `StudioRepairState`, `repair_state`,
  `repair_install_pending`, `fault_evidence`, `ReceiptBook::repair_sequence` (C-5);
  `prepare_repair_adoption` (C-6); `OwnerReceiptJournal::resolve_repair`, `canonical_head`, the
  version-2 journal with its `reconciled` slot and that slot's promotion, retirement and
  tenure-clearing lifecycle (C-7); the repair-bearing book document derivation (C-8).
- Store: `EpochFaultRecord` holding up to two canonically ordered **external** pairs plus at most
  one **tagged** repair, either external-indexed or source-bound with its pair inline, with active
  status **derived** on load (pending repair, else the source's own fault pair from the book, else
  the lowest external pair) and an explicit terminal-pair recycling transition; the owner record's
  version-3 section carrying a reserved live-conflict slot and bounded for nine receipt-sized
  values, stated once; `prepare_epoch_repair`
  (which also reconciles the journal) and `mark_epoch_repair_applied`; `apply_studio_repair`,
  `issue_studio_repair`, `report_studio_fault`, `studio_fault_evidence`, `StudioRepairOutcome`,
  `StudioRepairHold`, `CheckedRepairRecovery`, `stage_studio_repair_recovery`, `servable_repair`;
  the Registry mirrors.
- Sync: `ReceiptHeadSelection.repair` served instead of `None`;
  `AuthenticatedCheckpointHint::repair`; `select_repaired_checkpoint`; the version-2 **Studio and
  Registry** scoped head queries, each with a counted report list of 0 or 2 receipts, a raised cap
  of `MAX_QUERY + 2 * MAX_RECEIPT_BYTES`, and a header-only decode that keeps the report opaque
  until after the per-requester rail. All new symbols are named `receipt_repair` or `fault_repair`
  to avoid the existing MLS delivery `repair_outbox` family.
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

**None, in any of the eight passes.** No Cargo, npm or script command has been run: these
checkpoints change no code, and the local machine keeps checks serial. Every number quoted in the design is a constant
read from source at the base or an explicitly labelled estimate. The maximal-shape replacement
cost, the custody time of the capture and commit stages, and the protocol-allowance arithmetic in
design section 10.1 are **unverified**.

## Dependencies

| Dependency | Owner | State | What happens without it |
|---|---|---|---|
| Design verdict on revision 2 | user / independent reviewer | requested | no implementation starts |
| Core handoff signing split `e65bfd8` | Agent 1 / core | unreviewed | unaffected: no repair path uses it |
| Live tenure contract T1 to T5 (design 13.2) | Agent 2 | design **PASSED** adversarial review; no implementation | accepted on paper but not available; repair verification holds on `Unknown`, fails closed, never substitutes |
| Prepared overlay fence and source custody (design 13.1) | Agent 1 | design revision 3, unreviewed | repair relies only on the existing `resolve_studio_handoff` and `save_studio_source_checked`; if `inventory_generation` lands, rotating it becomes mandatory over the full list in design 10.3 |
| Native registration, UI hooks, INTERFACES rows | Agent 4 | not started | commands stay unregistered and nothing is callable from the renderer |

U-1 through U-11 are all decided and carried. No U-question remains open.

At the reviewed head, Agent 1's runtime implementation is only partial and its I-4 and C-3 work has
not started. Agent 2's tenure design has since **passed** adversarial review, correcting the stale
statement carried in earlier revisions, but none of it is implemented. Neither blocks this design
checkpoint; both are real prerequisites before Agent 3 implementation integrates.

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
   then W-1 and both report paths with N26 and N26b, then distribution with N23 to N25 and N27,
   then the runtime, control and native surface with N28 to N30, then the twelve mutants.
5. Treat N17 as the gating acceptance case: it is the one that proves a fault is reachable, is
   otherwise permanently unexitable, and is actually healed without injecting state on the new
   owner.
