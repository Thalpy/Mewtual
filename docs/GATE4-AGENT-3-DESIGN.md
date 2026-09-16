# Gate 4 Agent 3: runtime signed fault repair

Status: **revision 11, design proposal, awaiting re-review. No production code is written.**

Revision 10 (design body through `11ce6f1`) received **REQUEST CHANGES** with AG3-DES-045 to
AG3-DES-049 and AG3-TEST-009. The reviewer **closed AG3-DES-040 in 6.3 and 13.2** and **closed
AG3-DES-043**, and accepted AG3-DES-044's write order. All five new findings are internal
contradictions in revision 10 itself, and all five are confirmed. Disposition table is section 0.



Revision 9 (`3b6a4b40462ae83a341f8f6741c93edff55b5ef7`) received **REQUEST CHANGES** with
AG3-DES-039 to AG3-DES-044 and AG3-TEST-008. The reviewer **closed AG3-DES-038** and accepted the
owner-side half of AG3-DES-034. Revision 10 answers the six and reopens none of that. It also
absorbs a dependency change this scope had missed: Agent 2's accepted design **removes**
`observed_owner_tenure_start` in favour of a verification/authoring split with a fail-closed
`Imported` state, which sections 6.3 and 13.2 were still written against. Disposition table is
section 0.



Revision 8 (`b8bb5f3a3db6d0b8e450c82119f95e2cb929bce6`) received **REQUEST CHANGES** with
AG3-DES-034 to AG3-DES-038 and AG3-TEST-007. The reviewer **closed AG3-DES-030** at the mechanism
level and kept AG3-DES-023, AG3-DES-024 and AG3-DES-029 closed, with M14 and M1 to M11 accepted.
Revision 9 answers the five and reopens none of that. The reviewed commit predates
`76b8544`, which consumed Agent 2's `DraftArchive` seam; that change is additive and touches none
of these findings. Disposition table is section 0.



Revision 7 (`6d498c2e901e5071104a2533e0a632dc5676b2a7`) received **REQUEST CHANGES** with
AG3-DES-030 to AG3-DES-033 and AG3-TEST-006. The reviewer **closed AG3-DES-029**, kept AG3-DES-023
and AG3-DES-024 closed, accepted the source-bound encoding shape, M14 and M1 to M11. Revision 8
answers the five and reopens none of that. Its disposition table is section 0.



Revision 6 (`135766ca9f290ab96d3133b771bc42da74fe7825`) received **REQUEST CHANGES** with
AG3-DES-026 to AG3-DES-029 and AG3-TEST-005. The reviewer **closed AG3-DES-023 and AG3-DES-024**,
accepted the fresh-path half of AG3-DES-025 and the AG3-TEST-004 corrections, and confirmed M2 and
M1/M3-M11 stay accepted. Revision 7 answers the five and reopens none of that. Its disposition
table is section 0.



Revision 5 (`3737f1d4fff6f2c08302b0ecb1b024008818a1cf`) received **REQUEST CHANGES** with
AG3-DES-022 to AG3-DES-025 and AG3-TEST-004. The reviewer closed AG3-DES-021, accepted case 6c and
N32 for AG3-DES-017, accepted the monotone sequence concept subject to an ordering fix, and kept M2
and M1/M3-M11 accepted. Revision 6 answers the five and reopens none of that. Its disposition table
is section 0.



Revision 4 (`ad023d2e5b514a8f9598b1fe433fd2b080ecad6c`) received **REQUEST CHANGES** with
AG3-DES-017 to AG3-DES-021 and AG3-TEST-003, and agreement on U-11. The reviewer accepted C-8's
epoch-zero fix, the reconciled publication lifecycle, the healthy-source half of AG3-DES-013 and the
now-unmasked M2. Revision 5 answers the five new findings, realigns the three tests that contradicted
revision 4's own state machine, and reopens none of the accepted work. Its disposition table is
section 0.



Revision 3 (`a62178b94f20cd60a5363e6a3d6d6216edb9e516`) received **REQUEST CHANGES** with
AG3-DES-013 to AG3-DES-016 and AG3-TEST-002, and decisions on U-9 and U-10. The reviewer accepted
the historical-pair report direction, the 2a/2b split and the committed-state terminality oracle,
the Studio and Registry v2 framing and parse-order correction, U-7, U-8 and the retargeted M12, and
confirmed AG3-DES-003, 005, 007 and 008 stay closed. Revision 4 answers the five new findings and
reopens none of that. Its disposition table is section 0.



Revision 1 (`7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a`) received **REQUEST CHANGES** with
AG3-DES-001 to AG3-DES-008 and AG3-TEST-001 plus decisions on U-1 to U-6; revision 2
(`63a11e1a6451c7ed373c90b0e81b59d8a748a72a`) answered those and the reviewer confirmed the
AG3-DES-003, AG3-DES-005, AG3-DES-007 and AG3-DES-008 corrections stand. Revision 2 then received
**REQUEST CHANGES** with four new blockers, AG3-DES-009 to AG3-DES-012, two non-isolating mutants
(M2, M12) and decisions on U-7 and U-8. This revision answers those and reopens nothing that
passed. Scope is
[Agent 3 of the four handoffs](GATE4-AGENT-HANDOFFS.md#agent-3-runtime-signed-fault-repair);
progress is in [GATE4-AGENT-3-STATUS](GATE4-AGENT-3-STATUS.md). Review preamble 3.

**Unmet dependencies.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at `e65bfd8` is
unreviewed and Agent 1's runtime seams remain a partly implemented proposal with I-4 and C-3 not
started. **Agent 2's live-tenure design has now passed adversarial review**, correcting the stale
"design proposal" wording carried here through revision 7, but none of it is implemented, so the
section 13.2 contract is accepted on paper rather than available. Sections 13.1 and 13.2 state what
this scope needs and what it does without each.

No Cargo command was executed for this pass either. Every number is a source constant read at the
base or an explicitly labelled estimate.

## 0. Disposition of the revision-10 findings

| Finding | Disposition in revision 11 | Where |
|---|---|---|
| AG3-DES-045, the overflow hold has no wire bytes | **Corrected.** The field was added to the struct and the proof gate and never to the codec or `check_scope`, so the mechanism described as surviving restart could not be written at all and N43 was impossible as stated. It now has a canonical encoding, strict `0/1` canonicality, validation, and corruption failing the record rather than clearing the hold. | 5.2, 15.1 N44(a) |
| AG3-DES-046, the fallback contradicts the active-pair rule | **Corrected.** For `{R1,R3}` after `{R1,R2}` was repaired, "must drain before any other repair may be issued" and "cannot be drained, repair it directly" both applied and there was no legal next step. A `pair_is_materialisable` predicate, computed on the checked source **without mutating it**, splits drainable from direct-repair-only. | 5.2, 15.1 N45(a)(b) |
| AG3-DES-047, `repair_kind 3` has no coherent lifecycle | **Corrected.** Migration now happens before any B1, so a migrated pair is `repair_kind 1` and kind 3 applies only while the pair is still reserved; the two are never both applicable. Recycling is binding-specific, so a kind-3 pair is cleared from `reserved` instead of being stranded there and becoming decidable again. | 5.2, 15.1 N45(c)(d) |
| AG3-DES-048, the hold collapses distinct conflicts into one bit | **Corrected.** Clearing a pairless marker on one reporter's retry forgets every other known conflict, which is the safety half of AG3-DES-032 returning. The hold now keeps up to four pair fingerprints cleared individually, plus a sticky `unknown` flag for the overflow-of-overflow case that only a tenure change clears. It still stores no receipts, so it can only refuse. | 5.2, 6.6, 15.1 N44(b)(c) |
| AG3-DES-049, the tenure fix is inconsistent for the hold | **Corrected.** The stale `observed_tenure_id` pseudocode is gone, and both liveness predicates compare against one `tenure_id` derived in the same custody visit from the **authoring** accessor, the group id and the committer key. The hold stores that derived id rather than a bare start epoch, so two tenures sharing a start cannot be confused. | 5.2, 6.6, 15.1 N44(d) |
| AG3-TEST-009 | **Corrected.** N44 covers the hold's codec, multiple distinct overflows and tenure identity; N45 covers materialisability, the migration and binding choice, and terminal kind-3 clearing. | 15.1 |

## 0.0 Disposition of the revision-9 findings

| Finding | Disposition in revision 10 | Where |
|---|---|---|
| AG3-DES-039, a historical reserved pair has no encodable repair binding | **Corrected.** `repair_kind` had values for none, external index and source-bound inline, so a pair sitting in the reserved slot could be selected as active work and then not be nameable at B1 at all. A fourth kind binds the repair directly to that slot, which is the cleaner of the two options you offered because the migration alternative has no guaranteed capacity. | 5.2, 15.1 N37(g) |
| AG3-DES-040, liveness consumes a tenure identity the accepted seam does not provide | **Corrected, and the dependency change absorbed.** Agent 2's accepted design removes `observed_owner_tenure_start` for `verification_owner_tenure_start` and `authoring_owner_tenure_start` with a fail-closed `Imported`, which sections 6.3 and 13.2 were still written against. Mutation and drain now derive the expected tenure id through the existing `tenure_id(...)` derivation from the **authoring** accessor, with `None` a hold rather than `false`; the verification accessor may only add refusals. | 3 R32, 6.3, 6.6, 13.2, 15.1 N42 |
| AG3-DES-041, demotion leaves the only slot occupied | **Corrected.** Leaving a demoted pair in the slot solves one owner change and recreates AG3-DES-032 on the next. Demotion now migrates the pair into the historical list when there is room, and where there is not, a new live conflict sets a durable evidence-free hold marker that suppresses proof while the reporter retries, which is the marker you suggested. | 5.2, 6.6, 15.1 N43 |
| AG3-DES-042, a shared-receipt pair cannot be drained into the same fault | **Corrected.** `is_repaired_loser` screens before conflict handling, so feeding `R1` yields `Stale` and feeding `R3` against a head of `R2` yields `{R2,R3}`, never the reported `{R1,R3}`. Report admission therefore keeps an exact-pair fallback: when the live seal cannot reproduce the reported pair, the pair stays owner-side under the proof gate and is directly repairable through the new binding rather than being forced through `ReceiptBook::fault`. | 3 R33, 6.5, 15.1 N39 |
| AG3-DES-043, the claim is invisible for a pre-B2 peer job | **Corrected.** A non-owner has no owner record by U-9 and no `resolved_repair` before B2, so the durable predicate could not represent an in-flight Flow D repair and ordinary settlement could win the race into a `Settled` hold that makes the repair permanently unappliable. A runtime target claim acquired at S1 and owned through S4 joins the durable predicate. | 10.3, 15.1 N37(h), M18 |
| AG3-DES-044, the drain has no crash-safe write order | **Corrected.** The source fault write and its durability return first; only then is the slot cleared. A crash between leaves duplicate evidence, which is valid and idempotently cleaned, and an uncertain source write leaves the slot and the proof suppression intact. | 5.2, 8, 15.1 N37(f) |
| AG3-TEST-008 | **Corrected.** N39 asserts exact pair identity, N42 and N43 cover the tenure states and repeated demotion at capacity, N37 gains (h) and mid-drain failures, and M18 and M19 cover the peer claim and the settlement and rotation fences. | 15.1, 15.2 |

## 0.0 Disposition of the revision-8 findings

| Finding | Disposition in revision 9 | Where |
|---|---|---|
| AG3-DES-034, the ownership fence does not cover ordinary adoption | **Corrected.** Section 10.3 still named `repair_install_pending()` as the one-claim predicate, which is exactly what AG3-DES-031 had just shown starts too late; `advance_checkpoint` calls `install_studio_seed_step`, a real source-mutating install, so a B1-persisted case 6c repair was unclaimed against it. `repair_transaction_nonterminal` is now **the single target-claim fence for every source-mutating path**, with `repair_install_pending()` demoted to the post-B2 replacement capability it actually is. | 3 R31, 10.3, 15.1 N37(e), M17 |
| AG3-DES-035, the live slot exists on the wire but not in the state contract | **Corrected.** Revision 8 added `has_live_conflict` to the codec and `record.live_conflict` to the proof gate while the displayed struct and `check_scope` still knew nothing about it, so an implementation could lose durable proof suppression or admit unvalidated evidence into it. The field is now explicit with full codec and validation invariants, and the alias rule deliberately permits a shared receipt, since the accepted three-receipt case proves distinct legitimate pairs can share one. | 5.2, 15.1 N37(c), N41 |
| AG3-DES-036, the live slot is absent from active-pair priority | **Corrected.** After recycling cleared a terminal repair, a crash before the drain left the derivation choosing a historical external, so an owner could start a new B1 transaction on old evidence and postpone the dispute over the *current* source indefinitely. The live pair now outranks all historical work, and the repair field cannot be reused for an external while it waits. | 5.2, 15.1 N37(f) |
| AG3-DES-037, the live slot has no tenure-change lifecycle | **Corrected, by making liveness derived rather than stored.** "Live" was a judgement made at admission against the then-current tenure; if ownership changed before the drain the pair became historical with nothing to reclassify it, so it would have been sealed under the wrong owner or blocked proof forever. The slot now stores the pair and liveness is recomputed from freshly observed tenure at every custody visit, which also makes demotion free and removes the capacity question entirely. | 5.2, 6.6, 15.1 N37(g) |
| AG3-DES-038, the successor contract contradicts itself | **Corrected.** C-5 required every path carrying `resolved_repair` to carry the disposition and C-6 still said `adopted_successor` is unchanged. Since the disposition deliberately lives outside `ResolvedRepair`, following C-6 literally reproduces AG3-DES-033(C). The sentence is gone and every successor constructor now states what it clones. | 5.1 C-6 |
| AG3-TEST-007 | **Corrected.** N37 gains (e), (f) and (g), N41 adds direct owner-record corruption tests for the live slot, M17 isolates the adoption fence, and N36(b)'s stale "seven-receipt" wording is fixed to nine. | 15.1, 15.2 |

## 0.0 Disposition of the revision-7 findings

| Finding | Disposition in revision 8 | Where |
|---|---|---|
| AG3-DES-030, source-bound validation becomes impossible after B2 | **Corrected.** `apply_repair` sets `fault = None` on success, so revision 7's "inline hashes must match the live `ReceiptBook::fault`" rule necessarily fails after B2 and stranded exactly the resumes it was meant to protect. Binding is now **two-phase**: before B2 against `fault_evidence()`, after B2 against the pair named by the committed `repair_state()`, with a live fault expected absent for transitioned cases. Anything else, and a missing or corrupt source, refuses. | 3 R29, 5.2, 15.1 N36 |
| AG3-DES-031, the report fence misses the whole ownership interval | **Corrected.** `repair_install_pending()` only starts after B2, so a B1-persisted repair was unfenced: a current-tenure report could seal the source into a different fault and silently degrade a pending case 6c replacement into a terminal 6d screening, leaving the source on the repudiated branch. One predicate, `repair_transaction_nonterminal(record, source)`, now spans B1 through recycling and is what the fence and M13 use. | 6.5, 10.3, 15.1 N37, M13, M15 |
| AG3-DES-032, deferred live evidence is not capacity-closed and proof suppression is not durable | **Corrected.** At the legal maximum, two externals plus a pending source-bound repair, a current-tenure report could neither fault, nor be stored, nor replace anything, and section 6.6's refusal named only the same-exchange pair and a faulted source, so nothing durable kept proofs suppressed across a restart. A dedicated **live-conflict slot** is reserved outside the two history slots, and one durable `authoritative_proof_allowed` predicate is consumed on **every** head request. | 5.2, 6.6, 10.1, 15.1 N37, M16 |
| AG3-DES-033, `RepairDisposition` is not a complete state contract | **Corrected on all three points.** Cases 6a and 6c fitted neither documented value, so the disposition is now defined by the operation performed and has three values covering every C-2 case. Snapshot v3 carries the adoption mode explicitly, since a bare leading `3` would have destroyed the one bit that decides `decode` versus `decode_adoption`. And `RepairTransition` carries the disposition, which every successor and clone path that carries `resolved_repair` must carry too, bound to the exact repair: `adopted_successor` copies only the book, so a screened source that later adopted would otherwise have lost its provenance and reproduced the AG3-DES-028 bug. | 3 R30, 5.1 C-2, C-5, 15.1 N38 |
| AG3-TEST-006 | **Corrected.** N36 gains the post-B2 resume states, N37 gains the pre-B2 interleaving, a maximal-capacity variant and a restart-then-unrelated-request proof assertion, N38 gains 6a/6c and the successor lifecycle, M13 is widened to the full ownership fence, and M15 and M16 are added. | 15.1, 15.2 |

## 0.0 Disposition of the revision-6 findings

| Finding | Disposition in revision 7 | Where |
|---|---|---|
| AG3-DES-026, a source-bound repair cannot be encoded or restored | **Corrected.** Revision 6 said the source's own pair is never stored, then left a codec requiring `pair_count` of 1 or 2 and a repair matching a retained pair, so the very path the fix was for could not cross B1. The section now has a tagged repair that is either external-indexed or **source-bound with its pair inline**, so the owner record stays self-validating with only the logical document (which is all `decode` has), and the runtime separately binds those hashes to the authenticated source's actual fault pair. The bound follows to seven receipt-sized values. | 3 R26, 5.2, 10.1, 15.1 N36 |
| AG3-DES-027, a same-tenure report can destroy an active repair continuation | **Corrected.** `repair_install_pending()` requires `Closing`, and a current-tenure report driven through the live seal moves a Closing source to `Fault`, which the core explicitly permits, so the sole continuation predicate would silently go false and an outstanding recovery and replacement would be abandoned. A nonterminal repair now fences report-induced source mutation as well as discovery: the pair is retained as evidence, the source is not faulted until the repair is terminal, and authoritative proof stays suppressed meanwhile. | 3 R27, 6.5, 10.3, 15.1 N37, M13 |
| AG3-DES-028, `Screened` is not persisted so retries lie | **Corrected.** `StudioRepairState` carried no provenance, so an exact retry after a screening application reported the terminal `Repaired`. A disposition tag is persisted in the **Studio restart unit's own snapshot** (version 3), not in core's already-tested `ResolvedRepair` codec, and every retry returns the semantic outcome of the durable disposition rather than one inferred from the current phase. | 5.1 C-5, 5.3, 15.1 N38 |
| AG3-DES-029, the report no-op strands a distinct pair sharing a repaired loser | **Corrected.** `is_repaired_loser` is the right guard for **new receipt admission** and far too broad as proof that a different frozen pair is resolved: with three genuine receipts, repairing `{R1,R2}` would have permanently stranded a peer frozen on `{R1,R3}`, which is exactly the third-conflict case U-5 preserved. The no-op is now the **exact** pair only. | 3 R28, 6.5, 15.1 N39, M14 |
| AG3-TEST-005 | **Corrected.** Boundary tests added for the source-bound codec, a current-tenure report during `install_pending`, `Screened` retry equality and the distinct-pair-sharing-a-loser convergence, plus two mutants on the genuinely new guards. | 15.1 N36 to N39, 15.2 M13, M14 |

## 0.0 Disposition of the revision-5 findings

| Finding | Disposition in revision 6 | Where |
|---|---|---|
| AG3-DES-022, the two-pair record is not closed under a third source fault | **Corrected by removing the coupling entirely.** The record now holds only **external** pairs, never the source's own. A source fault already lives in `ReceiptBook::fault`, so it needs no slot, can never be crowded out by retained historical evidence, and is always derivable. A repair may name the source's fault pair or a record pair. The missing terminal-pair recycling is also specified: when a repair becomes terminal its named pair is removed from the record and the repair field cleared, so a resolved pair can never become active again. | 3 R25, 5.2, 15.1 N31 |
| AG3-DES-023, contradictory owner-record bounds | **Corrected.** Revision 5 left three different figures in the document: the new five-receipt formula, the old three-receipt formula later in the same section, and 11.4 KiB in section 10.1. One canonical formula now appears once, the stale paragraph is gone, 10.1 agrees, and the displayed struct is the actual multi-pair representation rather than the superseded singular one. | 5.2, 10.1 |
| AG3-DES-024, retry-first ordering can bypass the core live-authority check | **Corrected.** Core deliberately verifies current owner **before** its retry shortcut (`epoch.rs:1735-1741`, "a returning key is not its old tenure"), and revision 5's "sequence and retry classify before anything else" inverted that one layer up, in an authority-bearing API. Authority and evidence now precede every retry, sequence and in-progress outcome, and store-side redundancy is explicitly not what makes the core safe. | 5.1 C-2, 15.1 N35 |
| AG3-DES-025, case 6d's outcome contract claims the fault ended | **Corrected.** Case 6d deliberately leaves a different source fault standing, so B2's "Fault has durably ended" and the terminal `Repaired` were false for it. A distinct `Screened` outcome, and pair-specific wording in the barrier and crash prose, separate "this signed disposition is terminal" from "this document is no longer Faulted". | 5.3, 8, 15.1 N33 |
| AG3-TEST-004 | **Corrected.** N5c narrowed to the deliberately unclassifiable same-baseline higher head, since a provably losing baseline now takes 6c; N18 split into malformed evidence, which refuses, and a valid unrelated pair, which screens; M12's fixture qualified as historical throughout, because a current-tenure report must still fault the source even when the record is full; Flow D's "lands in case 6b" replaced by a deference to the C-2 classification. | 7 Flow D, 15.1 N5c, N18, 15.2 M12 |

## 0.0 Disposition of the revision-4 findings

| Finding | Disposition in revision 5 | Where |
|---|---|---|
| AG3-DES-017, case 6b can leave the repaired loser as the live target | **Corrected.** Revision 4 equated "not faulted" with "safe to leave unchanged", which is false: neither `prepare_checkpoint_adoption` nor `prepare_settlement` screens the receipt it already holds, so a source sitting on the loser would install or settle it after the repair. A new case 6c retargets a source whose own `latest`, opening or adoption target is the loser or a provably losing baseline; 6b now applies only when none of them is covered, and the same-baseline higher-head limitation is preserved rather than papered over. | 3 R21, 5.1 C-2, 15.1 N32 |
| AG3-DES-018, the active/deferred record has no codec or bound | **Corrected.** Revision 4 introduced `active`/`deferred` but left the singular encoding and a bound sized for three receipt-sized values, so the very state N31 requires could not be persisted. The extension is re-specified for two pairs plus one repair, the bound is raised to five receipt-sized values, and canonical ordering, duplicate and alias rejection, and the rule that only the active pair may carry a repair are stated. Active status is **derived** on load from the source and the record rather than durably stored, so no atomic source-plus-record transition is implied. | 3 R22, 5.2, 15.1 N31 |
| AG3-DES-019, a later source fault can deadlock a durable repair decision | **Corrected.** Once B1 exists that repair owns the target until its committed state is terminal. A repair naming a pair the source is not faulted on is now screening-only rather than `PairMismatch`, which is what dissolves the deadlock: the historical repair completes locally without needing the source, freeing the slot. Separately, a different repair or a competing pair is refused while `repair_install_pending()` is true, so the single `resolved_repair` slot cannot be overwritten mid-replacement. | 3 R24, 5.1 C-2, 5.2, 10.3, 15.1 N33 |
| AG3-DES-020, the screening path bypasses the repair-sequence guard | **Corrected.** The non-faulted path skipped `apply_repair` and therefore the `repair_sequence > self.repair_sequence` check, so a delayed lower-sequence repair could overwrite a newer one now that Flow D applies repairs regardless of fault status. Sequence and retry classification now runs **before** the fault-status cases and applies uniformly to every path. | 5.1 C-2, 15.1 N34 |
| AG3-DES-021, the reconciled-only journal restore shape is implicit | **Corrected.** Journal decode derives its document from `high_water.or(in_flight)`, so the shape C-7 permits and N7 requires would fail its presence check. Version 2 derives identity from the effective retained set including `reconciled`. This is the journal analogue of C-8. | 3 R23, 5.1 C-7 |
| AG3-TEST-003 | **Corrected.** M2 accepted unchanged. M12 retargeted, since revision 4 made a second distinct pair legitimately land in the deferred slot, so its old assertion would fail on the restored design. N24 and N5d rewritten: N24 still claimed repairs are discarded without a local fault, and N5d asserted unchanged stored bytes where case 6b durably adds repair evidence. | 15.1 N5d, N24, 15.2 M12 |
| U-11 | **Agreed by the reviewer and adopted.** The deferred slot accepts evidence only from a reporter that is itself faulted. | 5.2, 16 |

## 0.0 Disposition of the revision-3 findings

| Finding | Disposition in revision 4 | Where |
|---|---|---|
| AG3-DES-013, cross-tenure case 5 can destroy a healthy current-tenure source | **Corrected.** Revision 3's case 5 matched "cross-tenure, any shape", so an owner applying an old-tenure repair to its own healthy source would reopen it and replace a legitimate current head with its opening. **Fault status now classifies first**: a source not faulted on the named pair can only ever reach the screening-only case, whatever the tenure. The contradiction the reviewer also spotted, that Flow D discarded repairs for non-faulted documents and so never reached case 6, is fixed in the same place. | 5.1 C-2, 7 Flow D, 15.1 N5d |
| AG3-DES-014, case 5's epoch-zero shape is not restorable | **Corrected, and revision 3's claim was simply wrong.** `decode_mode` derives its document from `latest` alone, and `ResolvedRepair::verify` then requires that document to equal the repair's, so `latest = None` with a resolved repair fails `ReceiptConflict`. A repair-bearing book now derives its document from `latest` **or**, when absent, from the resolved repair, then checks every retained receipt against it. | 3 R17, 5.1 C-8, 15.1 N2b |
| AG3-DES-015, `reconciled` fixes adjacency but has no publication lifecycle | **Corrected.** Revision 3 defined only the creation of the reconciled decision. `mark_published` accepts only the existing high water or the exact pending decision, so completing a proof of the reconciled winner had no valid path, and a stale reconciled head would keep overriding a newer high water. The full lifecycle is now specified: promotion on completion, retirement on supersession, clearing on tenure change, inertness of stale retries, and decode constraints. | 3 R18, 5.1 C-7, 6.7, 15.1 N7, N28 |
| AG3-DES-016, the source fault and the evidence record can deadlock on two pairs | **Corrected.** Two durable locations could hold different unresolved pairs, and neither repair could then cross B2. A single-active-pair rule now spans both, with a deterministic priority (a source fault is always the active one, because it is what blocks the local document) and one bounded deferred slot so sequential repair remains possible in both orderings. | 5.2, 6.5, 6.8, 15.1 N31 |
| AG3-TEST-002, M2 is masked by the storage budget | **Corrected, and the masking is confirmed at the source.** `EpochStorageBudget::reserve` sets `ready = false` and a failed write never commits, so after an injected recovery failure the successor write fails on `Reconcile` whatever the mutant mints. The mutant now bypasses the recovery save **before** it reserves, leaving the budget valid so the successor writer is genuinely reachable. | 3 R20, 15.2 M2 |
| U-9 | **Reviewer's answer adopted.** `EpochFaultRecord` stays owner-only. After AG3-DES-013's correction a non-faulted peer retains ordinary resolved-repair evidence through the screening case, so no second owner-style journal is needed. | 5.2, 16 |
| U-10 | **Reviewer's answer adopted.** No automatic expiry: an unresolved frozen pair lives until a verified repair resolves it or the document is destroyed by its normal lifecycle. Expiring it could strand faulted peers whose only evidence was deliberately retained there. | 5.2, 16 |

## 0.0 Disposition of the revision-2 findings

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

### R17: a receipt book derives its document from `latest` alone

`decode_mode` computes `let document = latest.as_ref().map(|receipt| receipt.document.clone());`
(`epoch.rs:1877`) and then calls `resolved.verify(document.as_ref(), repair_sequence)`, whose first
condition is `document != Some(&self.repair.document)` (`epoch/repair_state.rs:43`). So a book with
a resolved repair but no `latest` fails `ReceiptConflict` on restore. Revision 3's epoch-zero
cross-tenure shape, `latest = None` and `tenure = None` with a resolved repair, is therefore not
restorable, and its N5b claim was wrong. This is AG3-DES-014.

### R18: `mark_published` has no path for a decision that is neither high water nor pending

`mark_published` returns early on an exact `high_water` match, then requires `in_flight` to be
present and to match; anything else is `ReceiptConflict` (`epoch.rs:2043-2058`). After revision 3's
reconciliation the journal can hold `high_water = R(e-1)`, `in_flight = None`,
`reconciled = S(e)`. The head selector can now prove `S(e)`, and `complete_studio_head` then calls
this API for `S(e)`, which has no valid path. Separately, `reconciled.or(high_water)` keeps
returning a stale `S(e)` after a later epoch becomes high water. This is AG3-DES-015.

### R19: an unresolved pair can exist in two durable places at once

A faulted source holds its pair in `ReceiptBook::fault`, and revision 3's owner record holds a
reported historical pair. Nothing related them. `apply_repair` requires the book's held pair to
equal the repair's, and the record refuses to replace an unresolved pair, so a source faulted on
one pair and a record frozen on another admits no repair that can cross B2 in either direction.
This is AG3-DES-016.

### R20: a failed accounted write leaves the budget unready, which masks an ordering mutant

`EpochStorageBudget::reserve` sets `self.ready = false` before returning
(`store/epoch_budget.rs:376`), and a writer error returns without `commit()`. So after an injected
recovery write failure every later `reserve` fails with `BudgetError::Reconcile`. Any mutant that
merely lies about the recovery barrier's result is caught by that independent guard rather than by
the intended assertion. This is AG3-TEST-002.

### R21: neither installer screens the receipt it already holds

`prepare_checkpoint_adoption` requires only `adopting`, `Closing`, not faulted, and
`latest == receipt` (`studio/epoch/adoption.rs:89-95`). `prepare_settlement` requires not
`adopting`, `Closing`, not faulted, and then takes `receipts.latest()` and verifies **current
owner** authority (`studio/epoch/settlement.rs:71-79`). Neither asks whether the receipt it is
about to act on is a repaired loser: `is_repaired_loser` screens receipts arriving at admission,
never one already sitting in `latest`.

So a peer that saw only the losing receipt never faulted, and revision 4's case 6b would leave it
Closing or adopting under that loser after the repair arrived. It would then install or settle the
repudiated branch. This is AG3-DES-017, and it is why "not faulted" cannot mean "safe to leave
unchanged".

### R22: the record extension and its bound were never updated for two pairs

Revision 4's `EpochFaultRecord` gained `active` and `deferred`, but §5.2 still specified the
singular `3 | a | b | has_repair | repair? | applied` encoding and still sized `MAX_RECORD_BYTES`
for three receipt-sized values. The maximal state is five: two active receipts, one repair, two
deferred receipts. Since `read_epoch_owner_plain` caps the file before unsealing
(`store/epoch_owner.rs:530-540`), an under-sized contract means the maximal valid state cannot be
written or reopened at all. This is AG3-DES-018.

### R23: the owner journal derives its identity from `high_water` or `in_flight` only

`OwnerReceiptJournal::decode` sets
`document: high_water.as_ref().or(in_flight.as_ref()).map(..)` and then requires
`journal.document.is_some() == journal.tenure.is_some()` (`epoch.rs:2084-2094`). C-7 permits, and
N7 requires, a restored shape with both absent and only `reconciled` present, which fails that
check. This is the journal analogue of R17, and it is AG3-DES-021.

### R24: a receipt book has exactly one resolved-repair slot

`ReceiptBook.resolved_repair` is a single `Option` (`epoch.rs:1575`) and `apply_repair` overwrites
it. So applying a second repair to a source whose first repair still has an outstanding recovery or
replacement would erase the evidence that `repair_install_pending()` is derived from, abandoning the
in-flight transaction. This is the second half of AG3-DES-019.

### R25: a receipt book has exactly one fault slot as well as one repair slot

`ReceiptBook.fault` is a single `Option<(Receipt, Receipt)>` alongside the single
`resolved_repair` (`epoch.rs:1571-1575`). The source therefore holds exactly one unresolved pair,
and it holds it whether or not any owner record mentions it. Revision 5's derivation nevertheless
required a source fault's pair to be "retained here" before it could be selected, so two retained
historical pairs could crowd out the very pair making the document read-only, and a repair for it
could not be written because the record's single repair had to validate against a retained pair.

The resolution is that the source's own pair never needed a record slot: it is already durable, in
the book, and always available. This is AG3-DES-022.

### R26: the owner record validates itself with only the logical document

`EpochOwnerReceiptState::decode` takes `(bytes, scope, document)` and `check_scope` takes
`document` (`store/epoch_owner.rs:66-124`). There is no `StudioEpoch` in scope, so the record cannot
validate a repair against a pair it does not itself hold. A source-bound repair therefore has to
carry its evidence inline if the record is to stay self-validating and fail closed on restart. This
is AG3-DES-026.

### R27: a Closing source moves straight to Fault, collapsing the continuation predicate

`transition_verified_receipt` permits `Open | Closing | Fault -> Fault` and clears `receipt_hash`
(`epoch.rs:2539-2549`). `repair_install_pending()` requires `phase == Closing`, so a report driven
through the live seal while a repair is awaiting its seed or recovery turns that predicate false and
the outstanding transaction stops being visible to the one oracle that governs it. This is
AG3-DES-027.

### R28: `is_repaired_loser` is an admission guard, not a resolution proof

It answers "should this arriving receipt be screened", covering the exact loser and, where
inheritance differs, that losing baseline (`epoch/repair_state.rs:74-84`). It does not answer "has
this peer's frozen pair been resolved". With three genuine conflicting receipts, a repair of
`{R1,R2}` makes `R1` a repaired loser without saying anything about a peer frozen on `{R1,R3}`,
whose `apply_repair` still demands its own exact pair. This is AG3-DES-029.

### R29: a successful repair clears the source's fault

`apply_repair` sets `self.fault = None` and installs `resolved_repair` in the same mutation
(`epoch.rs:1765-1774`). So after B2 the source is, correctly, no longer carrying the pair. Any rule
that validates a persisted source-bound repair by matching the live `ReceiptBook::fault` therefore
fails exactly when a resume is needed. This is AG3-DES-030.

### R30: a successor carries the book but no Studio-layer provenance

`adopted_successor` builds a fresh unit with `from_checkpoint`, then copies `self.receipts` and
marks the latest installed (`studio/epoch/adoption.rs:140-150`). Nothing else crosses. Since the
disposition tag deliberately lives in the Studio restart unit rather than in `ResolvedRepair`, a
screened source that later adopts would keep the resolved repair and lose its disposition, which is
the AG3-DES-028 bug again one transition later. This is AG3-DES-033(C).

Relatedly, `StudioEpoch::snapshot` writes `if self.adopting { 2 } else { 1 }`
(`studio/epoch.rs:432`) and that byte is the only thing telling restore whether to call
`ReceiptBook::decode` or `decode_adoption`. A version 3 that merely replaces it would destroy that
bit, which is AG3-DES-033(B).

### R31: ordinary checkpoint advancement installs into the same source

`advance_checkpoint` calls `install_studio_seed_step`
(`studio/receiver/catchup/discovery.rs:189`), and for the paired bucket
`install_registry_seed_for_studio` (`:159`). Both reach `adopt_studio_checkpoint`, which seals and
can replace the source. So ordinary discovery is a source-mutating path with exactly the same
ownership hazard as report admission, and gating it on `repair_install_pending()` leaves the whole
B1-to-B2 interval unclaimed. This is AG3-DES-034.

### R32: the tenure accessor this scope was written against is being removed

Agent 2's accepted design removes `observed_owner_tenure_start` rather than repointing it, so that
no call site inherits the wrong semantics by default, and replaces it with
`verification_owner_tenure_start()` and `authoring_owner_tenure_start()` over an
`Observed | Imported(u64) | Unknown` state. `Imported(S)` yields `Some(S)` for verification and
`None` for authoring, deliberately fail-closed for this scope's issuance and application
(`GATE4-AGENT-2-DESIGN.md` 9.3 part 5, 9.4 V1/V5/V6). At this head only the single accessor exists
in the tree (`catcoms-sync/src/owner_tenure.rs:151`), so this is an accepted design contract, not
yet a compile-time obligation. Sections 6.3 and 13.2 were still written against the single accessor.

The identity also matters: `tenure_id` is `H("catcoms-tenure:v1", server id, owner public key,
group epoch)` (`epoch.rs:178`), not the start epoch, so a pair's tenure cannot be compared against a
bare `u64`. This is AG3-DES-040.

### R33: a repaired loser is screened before conflict handling, so an exact pair cannot be re-derived

`ingest_verified` returns `Fault` on an existing fault, then `Stale` on `is_repaired_loser`, before
any conflict comparison (`epoch.rs:1615-1622`). After `{R1,R2}` is repaired for `R2`, feeding `R1`
through the typed seal yields `Stale`, and feeding `R3` against a head of `R2` yields a fault of
`{R2,R3}`. Neither reproduces the reported `{R1,R3}`. So "drain the reserved pair into a real source
fault" is not always possible, and an exact pair a peer is blocked on can only be preserved
owner-side. This is AG3-DES-042.

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
- **I-12** A `DraftArchive` record is a preserved local draft, never repairable history: no repair
  path reads, decodes, replaces, retires or reclaims one, and none is ever evidence (12.1).

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
    /// Which C-2 case this is, persisted with the source (C-5). Revision 7 omitted it, so
    /// cases 6a and 6c had no representable provenance at all (AG3-DES-033A).
    pub disposition: RepairDisposition,
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

**Live authority comes first, then sequence and retry** (AG3-DES-024, AG3-DES-020). Core already
gets this order right and says why: `apply_repair` calls `verify_current_owner` **before** its exact
retry shortcut, because "a returning key is not its old tenure" (`epoch.rs:1735-1741`). Revision 5
said sequence and retry classified "before anything else", which inverted that one layer up inside
`plan_repair`, itself an authority-bearing API taking `group` and `issuer_tenure_start`. An exact
old repair could then reach `AlreadyResolved` without the core's issuer-tenure recheck and, worse,
resume a replacement when `install_pending` was true. The store's own `verify_current_owner` in S-2
is defence in depth; it is explicitly **not** what makes the core safe. So `plan_repair` runs, in
this order:

1. `repair.verify_current_owner(group, issuer_tenure_start)`;
2. `repair.check_evidence(a, b)` and the caller's scope checks;
3. the retry, sequence and in-progress table below;
4. the fault and tenure classification.

15.1 N35 asserts step 1 precedes step 3 at the **core** boundary, with an exact previously resolved
repair, the same device key, an independently observed different current tenure and an outstanding
continuation: it must fail authority, not return `AlreadyResolved`.

Step 3, uniformly, for faulted and healthy sources alike:

| Precondition | Result |
|---|---|
| exactly this repair is already the resolved disposition | `AlreadyResolved`, continuation decided from the committed source (C-5) |
| a different repair, `repair_sequence <= book.repair_sequence` | `Held(SequenceNotNewer)`, nothing changes |
| a different repair with `repair_install_pending()` true | `Held(RepairInProgress)`, nothing changes (R24) |
| strictly newer | continue to classification below |

`Held(RepairInProgress)` is what protects the single `resolved_repair` slot: while a repair's
committed state still owes recovery or a replacement, no other repair may overwrite the evidence
that transaction is driving. It is released when the state becomes terminal, not when `applied` is
set.

**Fault status classifies next** (AG3-DES-013). Revision 3 put the tenure test first and let case 5
match "cross-tenure, any shape", so an owner applying a historical repair to its **own healthy
source** would reopen a correctly sealed epoch and replace a legitimate current head with its
opening, losing unrelated newer progress. A source that is not faulted on the named pair is never
reshaped by a repair **unless it is itself sitting on the repudiated branch** (case 6c).

| Case | Condition | phase | receipt_hash | adopting | install |
|---|---|---|---|---|---|
| 6a | **not faulted**, same-tenure, `O == Some(L)` | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 6c | **not faulted**, same-tenure, an own anchor is covered by the repair (below) | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 6b | **not faulted**, no own anchor covered: screening only | unchanged | unchanged | unchanged | none |
| 6d | **faulted on a different pair**: screening only | unchanged | unchanged | unchanged | none |
| 1a | faulted on the pair, same-tenure, `O == Some(S)`, `H` qualifies (below) | `Closing` | `H.hash()` | `false` | none |
| 1b | faulted on the pair, same-tenure, `O == Some(S)`, `H` does not qualify | `Open` | none | `false` | none |
| 2a | faulted on the pair, same-tenure, `S.closed_epoch == E`, `!source.adopting` | `Closing` | `S.hash()` | `false` | none |
| 2b | faulted on the pair, same-tenure, `S.closed_epoch == E`, `source.adopting` | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 3 | faulted on the pair, same-tenure, `O == Some(L)` | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 4 | faulted on the pair, same-tenure, `source.adopting`, none of the above | `Closing` | `S.hash()` | `true` | `Some(S)` |
| 5 | **faulted on the pair**, cross-tenure: `Unblocked` | `Open` | none | `false` | none |
| 8 | otherwise | refuse: `Held(UnsupportedShape)` | | | |

Dispositions, one per case, with no case unmapped (AG3-DES-033A): 1a, 1b, 2a, 2b, 3, 4 and 5 are
`Transitioned`; 6a and 6c are `Retargeted`; 6b and 6d are `Screened`. Cases 7 and 8 refuse and
record nothing.

**Case 6c is AG3-DES-017's correction.** An "own anchor is covered" when any of `latest`, `opening`
or the retained adoption target is the exact loser, or satisfies `is_repaired_loser`, which also
catches a head on a provably losing inherited baseline. Such a source is not healthy; it is sitting
on the repudiated branch and, per R21, both installers would happily act on it because neither
screens the receipt it already holds. Case 6c therefore retargets it with the same adoption shape
as case 3: `latest = S`, `Closing` under `S`, `adopting = true`, `install = Some(S)`, so the whole
current version is preserved as `Repair` recovery and the selected checkpoint replaces it through
the accepted transaction. Retargeting uses the adoption shape even when the source was not
adopting, because a peer must fetch `S`'s seed; the owner's own close-bearing settlement path is
reached only from a faulted source, so case 2a is untouched.

**The same-baseline limitation is preserved, not papered over.** When the selected and losing
receipts carry the same `TenureSelection`, `is_repaired_loser` deliberately classifies only the
exact loser, because receipts carry no ancestry chain and a higher same-baseline head cannot be
shown to descend from either side. Such a head falls to case 6b and is left alone. This design
invents no ancestry for it; 15.1 N32 asserts that explicitly so the limitation is visible rather
than accidental.

Case 6b is now the outcome only for a source with **no** covered anchor: it records
`resolved_repair` and `repair_sequence` so the loser and its baseline descendants are screened from
then on, and touches no gate, head, adoption mode or retained progress. Case 6a keeps the
descendant-convergence path, but only same-tenure, because a cross-tenure selected receipt cannot be
installed at all.

**Case 6d is the first half of AG3-DES-019's correction.** Revision 4 refused a repair whose pair
the source is not faulted on with `Held(PairMismatch)`, which deadlocks the sequence the reviewer
found: a durable repair for pair A, a crash, then the source faulting on an unrelated pair B, after
which A could never be applied and B could never be issued. A repair for a pair this source is not
blocked on is not a mismatch, it is simply not about this source's blocker: it is recorded as
screening, becomes terminal, and frees the record's repair slot so the source's own pair can be
decided next. The source's own fault is retained untouched. `Held(PairMismatch)` is therefore
removed as an outcome; a genuinely malformed pair is still caught by `check_evidence`.

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

**Case 5 is AG3-DES-009's correction, now scoped to a genuinely faulted source.** A cross-tenure
repair of such a source sets `fault = None`, records `resolved_repair`, and returns the source to
`latest = opening`, `tenure = TenureSelection::from(opening)`, `previous_until_installed = None`,
phase `Open`, `adopting = false`, `install = None`. Where the source has no opening (a faulted
epoch-zero source) `latest` and `tenure` both become `None`, which the decoder's
`tenure.is_some() == latest.is_some()` rule requires **and which C-8 makes restorable**; revision 3
asserted that shape already restored, which R17 shows was false. Both pair members leave `latest`;
they are retained in `resolved_repair`, which is what screens them. Nothing old-tenure is installed
and nothing is claimed for installation. Convergence then happens the only way it can: the source
is no longer faulted, so ordinary discovery of the **current** owner's checkpoint proceeds and, if
that rewinds this branch, stages an ordinary `Rewound` snapshot. A same-tenure head is discarded
with the tenure that is over, which is safe precisely because this case requires the source to have
been faulted: an unfaulted source keeps everything through case 6b.

`RepairHold` distinguishes `SequenceNotNewer`, `RepairInProgress`, `ScopeMismatch`, `Settled` and
`UnsupportedShape`. `PairMismatch` and `NoFault` are gone: both of those shapes are now the
screening cases 6d and 6b, which is what removes the AG3-DES-019 deadlock. A `Held` result is a successful observation the caller reports,
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

**C-8. A repair-bearing book is restorable without a current head** (AG3-DES-014, R17).
`decode_mode` derives its document from `latest` alone, so case 5's epoch-zero shape fails. The
derivation becomes: the document is `latest`'s when present, and otherwise, **only in the
repair-bearing versions 4 and 5**, the resolved repair's. Every retained receipt is then checked
against that document independently, exactly as today, and the existing
`tenure.is_some() == latest.is_some()` rule is preserved. Versions 1 to 3 are unchanged, so a book
with no head and no repair still decodes to the empty book and nothing else gains a new
representation.

This is a decoder-side widening of one derivation, not a new state: the state already had to exist
for case 5, and revision 3 simply asserted it round-tripped without checking. 15.1 N2b is a direct
`ReceiptBook::encode`/`decode` regression for the shape, run before any Studio or Registry
round-trip depends on it.

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
    /// What the ORIGINAL durable application did, persisted so an exact retry cannot
    /// report something else (AG3-DES-028).
    pub disposition: RepairDisposition,
}

/// Defined by the OPERATION PERFORMED, not by "did a fault end", so every C-2 case maps to
/// exactly one value (AG3-DES-033A). Revision 7 had only two values and cases 6a and 6c fitted
/// neither: they mutate a source that was never Faulted.
pub enum RepairDisposition {
    /// The source was Faulted on this pair and this repair ended that Fault.
    /// Cases 1a, 1b, 2a, 2b, 3, 4, 5.
    Transitioned,
    /// The source was NOT Faulted but was itself on the repudiated branch, so it was moved
    /// onto the winner. A replacement is required. Cases 6a and 6c.
    Retargeted,
    /// The source was left untouched; the disposition is terminal for this pair only, and any
    /// different Fault still stands. Cases 6b and 6d.
    Screened,
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

**Where the disposition tag lives.** In the **Studio restart unit's own snapshot**, not in core's
`ResolvedRepair` encoding, because book versions 4 and 5 are existing, accepted and tested code and
changing their byte layout for a Studio-layer concern would reopen them for no benefit.
`RegistryEpoch` mirrors this in its own restart format. The tag is never inferred from the current
phase: after later transitions the phase is not reliable provenance, which is precisely why revision
6's retry returned the wrong outcome.

**Snapshot version 3 keeps the adoption bit explicit** (AG3-DES-033B, R30). The existing leading
byte is `1` or `2` and is the **only** thing telling restore whether to call `ReceiptBook::decode`
or `decode_adoption`, so revision 7's "version 3 adds the tag" would have destroyed it. The layout
is:

```text
v1 / v2 : <1|2> | channel | ...            unchanged, byte compatible
v3      : 3 | u8 adopting | u8 disposition | u8 repair_bound | repair_hash[32] | channel | ...
```

`repair_bound` and `repair_hash` bind the disposition to the **exact** resolved repair, by its
record hash together with the book's `repair_sequence`; restore rejects a disposition that names a
repair the book does not hold, a `Transitioned` or `Retargeted` tag with no resolved repair, and a
`Retargeted` or `Transitioned` tag whose book and gate cannot have come from that case. A v3 unit
with no resolved repair is rejected outright, so the tag can never float free.

**Every path that carries `resolved_repair` carries the disposition** (AG3-DES-033C, R30).
`RepairTransition` gains a `disposition` field, and `adopted_successor`, `checkpoint_successor` and
any other constructor that clones `self.receipts` copies the matching tag and its repair binding.
Revision 7 said `adopted_successor` was unchanged, which meant a screened source that later adopted
kept the resolved repair and lost its provenance, reproducing AG3-DES-028 one transition later.

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
`Rewound`. Cases 6a and 6c use `Repair` too, since they also perform the replacement.

`adopted_successor` **is changed** (AG3-DES-038). Revision 8 said it was unchanged while C-5
required every path carrying `resolved_repair` to carry the disposition, and because the disposition
deliberately lives outside `ResolvedRepair` those two sentences could not both be implemented: the
literal C-6 reading reproduces AG3-DES-033(C)'s successor-provenance loss. Every successor
constructor that clones the receipt book, `adopted_successor` and `checkpoint_successor` included,
clones three things together: the book, the matching `RepairDisposition`, and its exact repair hash
and sequence binding. A successor carrying a resolved repair with no disposition, or with one whose
binding names a different repair, is rejected on restore.

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

**The full lifecycle, not just creation** (AG3-DES-015, R18). Revision 3 defined only how
`reconciled` appears, which leaves two live defects: `mark_published` accepts only an exact
`high_water` match or the current `in_flight`, so completing a proof of the reconciled winner has
no valid path at all; and a stale `reconciled` keeps overriding a newer high water forever. So:

1. **Promotion.** `mark_published(h)` additionally accepts `h == reconciled.hash()`: it moves that
   receipt into `high_water` and clears `reconciled`. This claims nothing that did not happen; it
   records the completion of a publication this device actually performed.
2. **Retirement on supersession.** Whenever `high_water` is set, a `reconciled` whose
   `closed_epoch <= high_water.closed_epoch` is cleared, so a newer decision always wins and
   `canonical_head()` can never regress.
3. **Tenure change.** `prepare_verified`'s `changes_tenure` branch, which already drops
   `high_water`, also drops `reconciled`: an old tenure's canonical decision is not this tenure's.
4. **Stale retries are inert.** The early `high_water` match returns `Ok(())` without touching
   `reconciled`, and a hash matching neither `in_flight`, `high_water` nor `reconciled` is still
   `ReceiptConflict`, so an old completion retry can never erase a newer reconciled decision.
5. **Decode constraints.** Version 2 rejects a `reconciled` that is not same-tenure, or whose
   `closed_epoch` is not strictly greater than `high_water`'s, and expresses the existing
   `in_flight` adjacency invariant against `canonical_head()` rather than `high_water`.
6. **Identity derivation** (AG3-DES-021, R23). `decode` currently derives its document from
   `high_water.as_ref().or(in_flight.as_ref())` and then requires document presence to agree with
   tenure presence, so the shape this section permits and N7 requires, both absent with only
   `reconciled` present, would fail. Version 2 derives document and tenure identity from the
   **effective retained set**, including `reconciled` when the other two are absent, and validates
   every retained receipt against that identity. Version 1 is unchanged. This is the journal
   analogue of C-8, and it makes the codec contract match the test rather than the other way round.

15.1 N28 follows this through: prove `S(e)`, complete it, restart, publish `e+1`, then prepare
`e+2`. Revision 3 stopped at "can issue at e+1", which is exactly where the missing lifecycle
would not yet have shown.

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
    /// EXTERNAL unresolved pairs only: pairs this document's own source is not faulted on.
    /// Each is canonical ascending by hash and validated by the repair-INDEPENDENT
    /// `conflicting_receipt_pair`, because a slot is written before any repair exists.
    /// At most two. The source's own pair is never stored here (AG3-DES-022).
    pairs: Vec<FaultPair>,          // 0..=2, ordered by pair id
    /// The reserved slot of 5.2 below. Holds at most one pair deferred behind a nonterminal
    /// repair. Whether it is LIVE is derived from freshly observed tenure, never stored
    /// (AG3-DES-037), so an owner change demotes it without a write.
    reserved: Option<FaultPair>,
    /// Durable hold for conflicts known but not storable (AG3-DES-041). It keeps no receipts,
    /// only enough identity to clear precisely (AG3-DES-048) and to stop being live when the
    /// tenure it belongs to stops being current (AG3-DES-049).
    overflow: Option<OverflowHold>,
    /// Signed only after an explicit current-owner decision. It may name the source's own
    /// fault pair, the reserved pair, or one of `pairs`.
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

Encoding appends, after the existing optional `2 | hash | close`, an optional section for **one or
two pairs plus at most one repair** (AG3-DES-018; revision 4 left the singular form here while the
prose had already moved to two pairs):

```text
3 | u8 external_count (0..=2) | external_count * pair
  | u8 has_reserved (0 or 1) | reserved_pair?
  | u8 has_overflow (0 or 1) | overflow?
  | u8 repair_kind | repair_binding | repair? | u8 applied

overflow        = tenure_id[32] | u8 fingerprint_count (0..=4) | count * fingerprint[32] | u8 unknown
fingerprint     = H("catcoms-fault-pair:v1", lower receipt hash, higher receipt hash)

pair            = two canonical Receipts, ascending by hash
pair id         = the smaller receipt hash
externals are ordered by ascending pair id; equal pair ids are rejected, and two externals
sharing any receipt hash are rejected

repair_kind 0 = none            repair_binding = empty, repair absent, applied = 0
repair_kind 1 = external        repair_binding = u8 index into the external list
repair_kind 2 = source-bound    repair_binding = the pair itself, inline
repair_kind 3 = reserved        repair_binding = empty; binds to `reserved`, which must be present
```

**A source-bound repair carries its pair inline** (AG3-DES-026). Revision 6 said the source's own
pair is never stored and then left a codec that demanded `pair_count` of 1 or 2 with the repair
matching a retained pair, so the one path that correction existed to unblock, an owner faulted on
`C` with zero or two externals, could not be written at all. Inlining is the right shape rather than
a lazy source lookup, because R26 shows `decode` and `check_scope` see only the logical document:
with the pair inline the record stays self-validating and fails closed on restart.

**Binding the inline pair to the source is two-phase** (AG3-DES-030, R29). Revision 7 said the
inline hashes must match the authenticated source's `ReceiptBook::fault`, which is right only before
B2. `apply_repair` clears `fault` on success, so that rule fails exactly when a resume is needed and
strands both an `AwaitingSeed` or `RecoveryPending` replacement and a terminal transition waiting on
B3 and recycling. The runtime therefore binds by phase:

| Source state | Required binding |
|---|---|
| no resolved repair equal to this one | inline pair **must equal** `fault_evidence()`; this is the pre-B2 case |
| `repair_state()` names exactly this repair | inline pair **must equal** that state's `(selected, losing)`; a live fault is expected **absent** for `Transitioned` and `Retargeted` dispositions; resume, finish or recycle |
| anything else | refuse |

A missing or corrupt source is a refusal in every phase, never an assumption.

**A dedicated reserved slot** (AG3-DES-032). The two external slots hold historical evidence. A
**current-tenure** pair deferred behind a nonterminal repair (6.5) gets its own reserved slot, so
the state is closed at the legal maximum: revision 7 allowed two externals plus a pending
source-bound repair, and a current-tenure report arriving then could neither fault the source, nor
enter the record, nor displace frozen evidence. That was not merely a liveness gap, because 6.6's
refusal named only the same-exchange pair and a faulted source, so nothing durable kept proofs
suppressed after the response or across a restart. The slot holds at most one pair and is filled
only by the deferral in 6.5.

**Liveness is derived, never stored** (AG3-DES-037). Revision 8 called the slot `live_conflict` and
treated that classification as a durable fact, but "live" was only a judgement made against the
tenure current *at admission*. If ownership changed before the drain, nothing reclassified the pair:
it would either be sealed into the current source under an owner it does not dispute, or block
`authoritative_proof_allowed` forever even though it disputes an old tenure, contradicting the
adjacent rule that historical externals do not suppress the current owner's proof. So the record
stores only the pair, and

```text
reserved_is_live(record, expected_tenure_id)      // defined precisely below
```

is recomputed from **freshly observed** tenure at every custody visit, consistent with T2's rule
that tenure is never cached. Promotion is not possible in the other direction, since an old tenure
cannot become current again with the same id.

**One derived identity for both slots** (AG3-DES-049). Revision 10 left stale pseudocode naming an
`observed_tenure_id` the accepted seam does not expose, and stored the overflow hold as a bare
`u64` start, which cannot distinguish two tenures that share a start epoch and differ in owner key,
exactly the confusion N42 exists to rule out. Both predicates now compare against one value derived
in the same custody visit, from the **authoring** accessor per 6.3:

```text
expected = tenure_id(group.group_id(), current_committer_signature_key, authoring_start)

reserved_is_live(record, expected) = record.reserved.is_some_and(|p| p.tenure_id == expected)
overflow_is_live(record, expected) = record.overflow.as_ref().is_some_and(|o|
        o.tenure_id == expected && (o.unknown || !o.fingerprints.is_empty()))
```

`authoring_start` being `None`, including `Imported`, is a hold: neither predicate is evaluated to
`false` on missing evidence, and 6.6's proof gate refuses outright.

**The overflow hold keeps identity, not receipts** (AG3-DES-048). Revision 10 collapsed every
unstorable conflict into a single tenure value, so two distinct reports became indistinguishable and
clearing on one reporter's successful retry silently forgot the other, which is the safety half of
AG3-DES-032 returning. The hold therefore carries:

```rust
struct OverflowHold {
    tenure_id: Hash32,           // the tenure this knowledge belongs to
    fingerprints: Vec<Hash32>,   // 0..=4 distinct pairs, hashed, no receipts retained
    unknown: bool,               // a fifth distinct pair arrived; sticky within this tenure
}
```

A fingerprint is removed **individually**, and only when that exact pair becomes stored in `pairs`
or `reserved`, or is resolved by a repair. `unknown` is never cleared by a retry: it is cleared only
by the whole hold ceasing to be live when its tenure stops being current. That is deliberately
pessimistic in the rare five-distinct-conflicts case and safe, which is the right direction for a
fact the owner has already authenticated. It stores no receipts, so it can only refuse, never
authorize.

The hold adds about 170 bytes at maximum and no receipt-sized values, so the nine-receipt bound
below is unchanged.

**Demotion migrates; it does not squat** (AG3-DES-041). Revision 9 left a demoted pair in the slot
and called that free, which closes one owner change and reopens AG3-DES-032 on the next: under a
later tenure, with both history slots full and a repair nonterminal, a fresh current-tenure conflict
could neither fault the source, nor take the occupied slot, nor displace anything, and after the
response nothing durable kept proof suppressed. So:

1. at the next write after demotion, the pair **migrates** into `pairs` when there is an
   **admissible** slot, freeing the reserved slot for a future live conflict. Migration happens
   **before** any B1 for that pair, so the binding is deterministic (AG3-DES-047): a migrated pair
   is issued as `repair_kind 1` by index, and `repair_kind 3` is used while the pair is still in
   `reserved`. The two are never both applicable.

   **Admissible, not merely numerically free** (AG3-DES-051). The reserved slot's alias rule is
   deliberately weaker than the external one: a reserved pair may share a single receipt with an
   external, which the three-receipt case requires, while two externals sharing any receipt hash are
   rejected. So `pairs[0] = {R1,R2}` with `reserved = {R1,R3}` is a legal state, and on demotion
   with one numerically free slot, revision 11 would have demanded a migration the validator forbids
   while also saying kind 3 applies only when there was "no room", leaving the pair with no legal B1
   path at all. Room therefore means *a slot this pair may legally occupy after every external-set
   invariant*, so a pair whose migration would violate the alias rule stays in `reserved` and is
   issued as kind 3 even though a slot is numerically free;
2. where `pairs` is full it stays put, derived-historical, and the slot is unavailable;
3. a live conflict arriving with no slot records its pair **fingerprint** in the `OverflowHold`
   under the derived current `tenure_id` instead
   of being dropped. That is the evidence-free hold: it suppresses proof on the same derived-live
   rule while the reporter retries, and it is cleared when the pair can be stored, when the source
   itself faults on it, or when its tenure stops being current.

The marker deliberately carries no receipts. It cannot authorize anything, only refuse, which is the
correct asymmetry for a fact we know but cannot yet substantiate.

**Not every live pair is drainable** (AG3-DES-046). Revision 10 required a live reserved pair to
become a source fault before any other repair could issue, while the AG3-DES-042 fallback said an
unreproducible pair stays owner-side and is repaired directly. For `{R1,R3}` after `{R1,R2}` was
repaired, both rules applied and there was no legal next step. So resolution is split by a predicate
computed on the already-checked source **without mutating it**:

```text
pair_is_materialisable(source, pair) =
    a DRY RUN of the exact typed admission, on a clone of the book and gate, ends with frozen
    fault evidence byte-for-byte equal to `pair`
```

**Why a dry run and not an analytical test** (AG3-DES-050). Revision 11 asked whether a member was
"the source's current head or its opening", which is broader than what the opening path will
actually accept. `receipts_conflict` admits a pair that shares a closed epoch **or** differs in
`TenureSelection`, so `{O, R}` can be genuine conflicting evidence with *different* closed epochs;
but `check_opening_receipt` additionally requires `receipt.closed_epoch == opening.closed_epoch` and
`opening.closed_epoch + 1 == gate.epoch` (`epoch.rs:1678-1685`). Such a pair would have been called
materialisable, the drain would fail with `EpochScope`, and direct kind-3 issuance would be
forbidden because of that classification: the live pair wedges while proof stays suppressed.

The dry run runs on a clone under the same exclusive custody, mutates nothing durable, performs no
I/O, and answers the only question that matters, which is whether the real transition reproduces
*this* pair rather than some other one. An analytical predicate would have to restate every
precondition of both admission paths and stay in step with them; the reviewer's exactness
requirement is cheaper to meet directly.

- **materialisable** -> drain into a source fault, under the crash-safe order below, then repair it
  as the source's own pair through `repair_kind 2`;
- **not materialisable** -> the pair stays in `reserved`, proof stays suppressed, and it is issued
  directly through `repair_kind 3`. It never passes through `ReceiptBook::fault`, because the seal
  demonstrably cannot produce it (R33).

The decision is made before any durable source mutation, so the impossible case is never reached by
first faulting the source into some *other* pair. 15.1 N39 exercises the selection and issuance
boundary, not only the retained pair.

**The drain is its own crash-safe transaction** (AG3-DES-044). A live pair lives in the owner record
and its eventual fault lives in the source: two durable objects with no atomic transition, and
revision 9 gave no order between them. Clearing the slot first would, on a crash or a failed source
write, leave a healthy source, no evidence and no proof suppression. So the order is fixed:

1. write the exact source fault and **wait for its durability to return**;
2. only then clear `reserved`.

A crash between leaves the pair in both places, which is valid: the existing rule that a record slot
duplicating the source's current fault pair is ignored for derivation and dropped at the next write
cleans it idempotently. An uncertain or failed source write leaves the slot untouched and proof
suppressed, and the drain is retried.

**Validation** (AG3-DES-035). Revision 8 put `has_live_conflict` on the wire and read
`record.live_conflict` in the proof gate while the struct and `check_scope` knew nothing about it,
which left an implementation free either to lose durable proof suppression or to admit unvalidated
evidence into a slot that gates proof service. The reserved pair is validated exactly like an
external: byte-exact receipt decode, both documents equal this document, canonical ascending
ordering, and `conflicting_receipt_pair` passing. `has_reserved` is canonically 0 or 1. Corruption
rejects the whole record and never resets it.

Its alias rule is deliberately **weaker** than the external-external one: the reserved pair must not
be the *same pair* as an external or as an inline source-bound pair, since that would be double
storage, but it **may share a single receipt** with either. The accepted three-receipt case is
exactly that shape, and unlike a historical pair, which can wait a turn, a live pair has nowhere
else to go and gates proof service, so a blanket share rejection would be wrong here even though it
is a tolerable heuristic between two historical externals.

Sections appear in ascending tag order with no duplicates. Old readers already reject any tag other
than 2 (`epoch_owner.rs:141-143`), which is the stated intent of the v2 extension.

`check_scope` gains: every retained receipt re-decodes byte-exactly; every document equals this
document; `conflicting_receipt_pair(document, &a, &b)` passes for each pair, external or inline; the
externals are distinct and share no receipt hash, so an aliased or half-duplicated pair is rejected;
a repair, if present, has `repair.document` equal to this document and passes `check_evidence`
against **the pair its `repair_kind` binds it to**. All four bindings are enumerated, since
revision 11 added kind 3 to the wire and left this contract describing only the older two
(AG3-DES-052):

| `repair_kind` | Required |
|---|---|
| 0 | no repair present, `applied` false, `repair_binding` empty |
| 1 | index in range for `pairs`, and `check_evidence` matches that entry exactly |
| 2 | inline pair present, and `check_evidence` matches it exactly |
| 3 | `reserved` is `Some`, and `check_evidence` matches it exactly |

Illegal combinations are rejected outright rather than tolerated: kind 3 with `reserved` absent,
kind 1 with an out-of-range or absent index, kind 2 with no inline pair, any kind above 3, a repair
present with kind 0, and `applied` true with kind 0. So a valid repair for some pair Q can never be
coupled to a reserved pair P by encoding it as kind 3.

**Overflow canonicality** (AG3-DES-053). The hold has one encoding per logical state: fingerprints
are **distinct and canonically ordered**, `has_overflow` and `unknown` are strictly `0` or `1`,
`fingerprint_count` is at most 4, and `has_overflow = 1` may not encode an empty, not-unknown hold.
A hold whose last fingerprint is cleared and whose `unknown` is false canonicalises to
`has_overflow = 0` at the next write. Without that rule the inert shape
`has_overflow=1, count=0, unknown=0` would decode to something on which `overflow_is_live` is false,
which is precisely a malformed state quietly releasing proof suppression.

Corruption is an error, never a silent reset.

**Bound, stated once and nowhere else.** The maximal state is **nine** receipt-sized values: two
external pairs (four receipts), the reserved pair (two), a source-bound repair's inline pair
(two) and the repair itself. So `MAX_RECORD_BYTES` becomes
`MAX_OWNER_RECEIPT_JOURNAL_BYTES + MAX_CLOSE_RECORD_BYTES + 9 * MAX_RECEIPT_BYTES + 1280`, which
with C-7's raised journal constant is about 18.5 KiB, and `MAX_SEALED_BYTES` follows. This matters
beyond accounting: `read_epoch_owner_plain` caps the file before unsealing, so an under-sized
contract would make the maximal valid state that N31 and N36 exercise impossible to write or reopen.
Revision 5 carried three contradictory figures, revision 6 fixed them at five and revision 7 raised
them to seven for the inline pair; the reserved slot makes nine. Section 10.1 repeats this
number and nothing else does.

**Active status is derived, never stored** (AG3-DES-018). A source fault and this record are
separate durable objects with no atomic transition between them, so revision 4's talk of
"demoting" a pair implied a write that does not exist. Instead, on every load the active pair is
derived, in this order:

1. a persisted repair that is not yet terminal names the active pair (AG3-DES-019: once B1 exists,
   that transaction owns the target);
2. otherwise the source's own fault pair, read from `ReceiptBook::fault`;
3. otherwise the reserved pair **if `reserved_is_live`**, which must be **resolved** before any
   other repair may be issued, by the drain when it is materialisable and by direct issuance when it
   is not (below);
4. otherwise the lowest retained pair id in `pairs`;
5. otherwise the reserved pair when it is no longer live, ranked with history.

**A live reserved pair outranks all historical work** (AG3-DES-036). Revision 8 omitted the slot
from this order entirely, which left a crash-visible hole: once recycling cleared a terminal repair,
a crash before the drain meant no pending repair and no source fault, so the derivation picked a
historical external, the owner could open a fresh B1 transaction on old evidence, and the dispute
over the **current** source could be postponed for the whole length of that transaction. Rule 3 also
forbids reusing the repair field for an external while a live reserved pair waits, which is what
makes the postponement unreachable rather than merely unlikely.

**The source's pair is never a record slot** (AG3-DES-022, R25). Revision 5 required it to be
"retained here" before rule 2 could fire, which left a reachable dead end: with two unresolved
historical pairs already retained, a local source fault on a third pair could neither be selected
nor be given a repair, because the single repair field had to validate against a retained pair and
both slots were full. Since `ReceiptBook.fault` is itself durable and holds exactly one pair, the
record never needed a copy. Rule 2 now always fires when the source is faulted, whatever the record
holds, and the repair field may name that pair directly. The combined state is therefore closed: at
most two external pairs, plus whatever single pair the source itself is blocked on, plus one repair.

A record slot that duplicates the source's current fault pair is redundant; on load it is ignored
for derivation and dropped at the next write, so a pair that was external when reported and later
became the source's own does not consume capacity twice.

**Terminal-pair recycling** (AG3-DES-022, second hole). Revision 5 said the other retained pair
"becomes active" when a repair becomes terminal, but never removed the pair that just terminated,
so the lowest-pair-id rule could make already-resolved evidence active again. So: when a repair's
committed state becomes terminal, one durable transition removes the pair **its binding names** and
then clears `repair` and `applied`. The removal is binding-specific (AG3-DES-047), which revision 10
left as a single `pairs`-only rule that silently stranded a kind-3 pair in `reserved` where it could
become active again:

| `repair_kind` | Removes |
|---|---|
| 1, external | the indexed entry in `pairs` |
| 2, source-bound | nothing in the record; the pair lives in the source |
| 3, reserved | clears `reserved` exactly |

Its evidence is not lost, because the book's
`resolved_repair` now holds both receipts and `is_repaired_loser` screens them. The transition is
idempotent: re-reporting the **exact** pair this repair named is a no-op (6.5 rule 6, as narrowed by
AG3-DES-029, which deliberately does not extend to a distinct pair merely containing its loser), and
an exact retry after a crash re-performs the same removal. Only after it completes may the repair field be used for
another pair, which is what stops a single field silently changing owners.

Crash at any point leaves at most two external pairs and one repair on disk, and the derivation is
a pure function of what survived plus the source's own book.

**The frozen pair is never replaced while unresolved** (I-10). A report naming a different pair
while this slot holds an unresolved one is refused, exactly as a third receipt never replaces a
book's frozen pair. This is a guard the core does not provide on the owner side, because on that
side the book is not involved at all; it is what 15.2 M12 mutates.

**One active pair across both durable locations** (AG3-DES-016, R19). An unresolved pair can live
in the source's `ReceiptBook::fault` or in this record, and revision 3 related them not at all, so a
source faulted on `B1/B2` with a record frozen on a historical `A1/A2` admitted no repair that could
cross B2 in either direction: `apply_repair` demands the book's own pair, and the record refuses
replacement. The rule, with a deterministic priority:

- **A pending repair owns the target.** Once B1 has persisted a signed repair, that transaction is
  the active one until its committed state is terminal, which is derivation rule 1 above. Revision
  4 gave a later source fault priority instead, which is the deadlock AG3-DES-019 found.
- **A source fault is never suppressed.** Refusing to record a genuine local equivocation would
  leave a disputed head servable, which is worse than a wait. It simply may not preempt a pending
  repair; it waits its turn through rule 2.
- The record holds at most two pairs, so a report arriving while a different pair is unresolved
  takes the free slot rather than being lost or replacing anything. If both slots are occupied by
  different pairs, the report is refused with a defer reason and the reporter retries later.
- `issue_studio_repair` only ever decides the derived active pair. When that repair becomes
  terminal, the other retained pair becomes active by the same derivation, so sequential repair is
  always possible and the record stays constant-sized.
- `StudioFaultView` reports which pair is currently decidable and that another is waiting, so the
  user is never asked to choose between two faults at once.

**Why this no longer deadlocks.** The reviewer's sequence was: historical pair A active, B1 persists
repair RA, crash, then the source faults on an unrelated pair B. Under revision 4, RA could not
cross B2 because the book's pair was B, and RB could not be issued because RA was held unapplied.
Now RA applies through case 6d as screening only, which is terminal for a repair this source is not
blocked on, so the slot frees and the source's own pair B becomes active and decidable. The source
keeps its fault throughout and nothing is discarded.

Issuance of ordinary receipts is **not** blocked by an unresolved record pair. A new receipt cannot
manufacture a fault on its own; only equivocation can, and an honest owner does not equivocate. The
reviewer asked this be decided explicitly, so: the owner keeps making progress in its own tenure
while a historical decision is pending, and the cost is that peers faulted on the old pair stay
faulted until it is decided, which is visible in the fault view.

The bound is stated once, above. Revision 5 left this paragraph behind with the superseded
three-receipt formula, which is AG3-DES-023: an implementation following it would have recreated
exactly the failure AG3-DES-018 closed. These bytes charge the protocol allowance through the
existing owner `storage_record`.

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
    /// This document's own Fault ended and no replacement is required. Terminal.
    Repaired,
    /// This signed disposition is terminal and durably recorded, but it was NOT about this
    /// document's own blocker: the source's own Fault, if any, still stands (case 6d), or the
    /// source was healthy (cases 6b). Never read as "the document is usable again"; the
    /// caller re-reads settlement state (AG3-DES-025).
    Screened,
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
3. `checked_studio_receive_source`, which enters the six-family budget, authenticates the actual
   wrapper and verifies its inventory record. Absence is an error: a repair never creates a source.
4. Read `unit.repair_state()` **before** planning. This is the AG3-DES-004 correction:

| `repair_state()` | Action |
|---|---|
| `None`, or a different repair | plan and commit the transition (step 5) |
| `Some` with `disposition == Screened` | flush the unchanged source, return `Screened` |
| `Some` with `installed` | flush the unchanged source, return `AlreadyRepaired` |
| `Some` with `install_pending` and no seed | flush the unchanged source, return `AwaitingSeed` |
| `Some` with `install_pending` and a seed | skip to step 7, continuing the replacement |
| `Some`, `Transitioned`, not pending, not installed | flush the unchanged source, return `Repaired` |

The `Screened` row is AG3-DES-028's correction: revision 6 had no provenance here, so an exact retry
after a screening application fell through to the terminal `Repaired` while the document was still
Faulted on another pair. The durable state was safe; the reported outcome was false.

   Every row flushes rather than short-circuiting before any write, because a readable successor
   is not proof that its durability barrier returned. This mirrors the adoption retry, which saves
   or syncs the actual source (`store/epoch_studio.rs:556-566`).
5. `unit.apply_receipt_repair(...)`. On `Held`, save nothing and return the hold with the unchanged
   state.
6. Save the source (barrier **B2**). **The signed disposition is durable; whether this document's
   own Fault ended is a separate question** (AG3-DES-025). B2 ends the fault only for the
   transitioning cases 1 to 5; in the screening cases 6b and 6d the source's own state is
   deliberately untouched, so a different fault may still stand. The outcome is read back from the
   **committed** `repair_state()` and the committed phase, never from the plan's `install` field
   (C-5, AG3-DES-010): `install_pending` continues to step 7 or returns `AwaitingSeed`; a screening
   case returns `Screened`; and only a transitioning case whose committed state is neither pending
   nor awaiting returns the terminal `Repaired`. The owner then records B3.
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

**Which accessor, and for what** (AG3-DES-040, R32). Agent 2's accepted design removes
`observed_owner_tenure_start` rather than repointing it, so every call site must choose explicitly
between `verification_owner_tenure_start()` and `authoring_owner_tenure_start()` over an
`Observed | Imported(u64) | Unknown` state. Revisions 1 to 9 of this design were written against the
single accessor, which is superseded. The mapping for this scope:

| Use | Accessor | `Imported(S)` | `Unknown` |
|---|---|---|---|
| issuing a repair (B1) and every mutation or drain it authorizes | **authoring** | hold | hold |
| applying a repair to a source (B2 and the replacement) | **authoring** | hold | hold |
| deriving `reserved_is_live` for the drain decision | **authoring** | hold | hold |
| deciding whether a repair or receipt is refused | verification may add refusals | may refuse | may refuse |
| suppressing authoritative proof | fail closed when unclassifiable | suppress | suppress |

`None` from the authoring accessor is a **hold**, never a `false` that silently permits something.
That distinction is the whole point of `Imported`: it is deliberately fail-closed for this scope, so
treating its absence as "not live" would demote a real live conflict and release proof suppression
using verification-only evidence. The verification accessor may only ever add refusals; it can never
authorize a live drain, a seal or an issuance.

**Tenure identity, not start epoch.** A pair's tenure is compared by `tenure_id`, derived through
the existing `tenure_id(server_id, owner_public_key, group_epoch)` (`epoch.rs:178`), not by
comparing a bare `u64` start against a pair's field. The expected current id is derived from the
authoring start, the current designated committer's signature key and the group id, all read in the
same custody visit.

The repair path never uses `group.epoch()`, a receipt's carried `tenure_start_group_epoch`, or the
repair's own claimed field as its own evidence. Issuance additionally requires a
`ServerOwnerSnapshot`, rechecked by `with_durable_owner_snapshot` at the moment of use.

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
     and rotation for that document. **Unless a repair transaction owns this source**
     (AG3-DES-027, AG3-DES-031): `transition_verified_receipt` permits `Closing -> Fault` and clears
     the receipt hash (R27), so sealing here mid-transaction abandons work this peer has already
     promised. The fence is:

     ```text
     repair_transaction_nonterminal(record, source) =
         record holds a repair whose committed state is not terminal      // B1 persisted, incl. pre-B2
      || source.repair_install_pending()                                  // AwaitingSeed, RecoveryPending
      || an unfinished replacement or recycling is outstanding
     ```

     Revision 7 used only `repair_install_pending()`, which **starts after B2**, so a B1-persisted
     repair was unfenced. That is worse than a lost predicate: a healthy source on the loser of
     pair A needs case 6c and a replacement, and if a current-tenure report seals it into `Fault(B)`
     before B2, the resumed repair sees a source faulted on a different pair, takes case 6d instead,
     and becomes **terminal without ever performing the replacement** that would have moved it off
     the repudiated branch. While the predicate holds, the reported pair goes to the reserved
     slot (5.2) and the seal is deferred until the repair terminates, at which point the runtime
     drains that slot into a real source fault **if it is still live**; if the tenure changed
     meanwhile it is retained as historical evidence and never sealed under an owner it does not
     dispute (AG3-DES-037). Safety is not deferred with it: 6.6's durable proof gate covers the
     interval.

     **The exact pair survives even when the seal cannot reproduce it** (AG3-DES-042, R33). The live
     seal is not a general way to materialise a reported pair: `is_repaired_loser` screens before any
     conflict comparison, so after `{R1,R2}` is repaired for `R2`, feeding `R1` yields `Stale` and
     feeding `R3` against a head of `R2` yields `{R2,R3}`, not the reported `{R1,R3}`. Where the seal
     cannot produce exactly the reported pair, admission keeps it **owner-side** in the reserved slot
     under the proof gate, and it is repairable directly through `repair_kind 3` without ever passing
     through `ReceiptBook::fault`. This is what actually lets the three-receipt case converge: N39
     asserts the owner's durable evidence is exactly `{R1,R3}`, not merely that some fault occurred.
   - **Any earlier tenure** (the historical case): **nothing is passed through the live seal.**
     `StudioEpoch::seal` ends in current-owner verification and only handles its own gate epoch,
     opening and adoption shapes, so feeding it an old receipt would be exactly the confusion of
     historical authenticity with live authority that the review warned against. The pair is
     written only into the owner record's `EpochFaultRecord` (5.2). The provider's source is not
     touched and does not fault: its own head under the current tenure is not in dispute.
6. **No-ops, the frozen pair and the deferred slot.** A report is a no-op when **this exact pair**
   is already recorded, or when the resolved repair's own named pair **is** this pair. It is
   **not** a no-op merely because one of its members is `is_repaired_loser` (AG3-DES-029, R28).
   That predicate answers "should this arriving receipt be screened", not "has this peer's frozen
   pair been resolved": with three genuine conflicting receipts, repairing `{R1,R2}` makes `R1` a
   repaired loser while saying nothing about a peer frozen on `{R1,R3}`, whose `apply_repair` still
   demands its own exact pair. Treating that as covered would strand it permanently, since neither
   the `{R1,R2}` repair nor any ordinary receipt can clear its fault. Such a pair stays admissible
   and needs its own higher-sequence repair, which is exactly the third-conflict behaviour U-5
   preserved.
   A report naming a **different** pair while an unresolved one is held never replaces it (I-10):
   it goes to the record's `deferred` slot if that is free, and is otherwise refused with a defer
   reason for the reporter to retry (5.2, AG3-DES-016). On the owner side this refusal lives in the
   record, not in the book, so it is a guard the core does not otherwise provide (15.2 M12).
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

**One durable proof gate** (AG3-DES-032). Revision 7's refusals named only the pair recorded in
*this same exchange* and a source that is *itself Faulted*. A live pair deferred behind a repair
leaves the source deliberately un-faulted, so once the response was sent, or once the process
restarted, nothing durable told the next unrelated request to keep refusing. The gate is now a
single predicate, evaluated from durable state on **every** head request, not only on the request
that admitted the report:

```text
authoritative_proof_allowed(source, record, authoring_start) =
    !source.is_faulted()
 && authoring_start.is_some()                        // Imported/Unknown fail closed
 && { let expected = tenure_id(group_id, committer_key, authoring_start?);
      !reserved_is_live(record, expected) && !overflow_is_live(record, expected) }
 && the receipt about to be proved is not a member of any pair this record retains
```

The `authoring_tenure.is_some()` term is AG3-DES-040's fail-closed rule: when current tenure cannot
be authoritatively classified for authoring, including `Imported`, proof is refused rather than
permitted on verification-only evidence. The `overflow_is_live` term is AG3-DES-041's: a conflict we
know about but could not store still suppresses proof, and per AG3-DES-048 it stays suppressed while
**any** of its retained fingerprints, or its sticky `unknown` flag, is outstanding, not merely until
one reporter's retry succeeds.

Historical pairs do not suppress proof by themselves: they dispute an old tenure's head, not the
current one. A **live** reserved pair does, which is why the slot exists rather than sharing
capacity with history. Because liveness is derived from freshly observed tenure (5.2), a reserved
pair that stops being current stops suppressing proof at the next request, with no write and no
window in which it suppresses forever. A repair being servable is never a licence to prove a
disputed receipt.

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
predicate. The receiver authenticates the responder and the response signature, then verifies the
repair under 6.2 and 6.3 and runs Flow A **whatever its own fault status**. Revision 3 discarded a
repair for a document with no local fault, which contradicted the screening case's existence and
meant it was never reached in practice (AG3-DES-013). Which case a verified repair lands in is
decided by C-2's classification and nothing here: a healthy source with no covered anchor screens
through 6b, one already sitting on the repudiated branch is retargeted through 6c, and one faulted
on a different pair screens through 6d. Revision 5 said "lands in case 6b", which overrode the
classification it had itself just refined (AG3-TEST-004). Only a repair that fails verification, or
that names a document this peer does not hold, is discarded.

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
| B2 | source: resolved repair in book, and for a transitioning case the fault cleared and adoption mode set | source unchanged; re-apply from the pending record or a re-fetched repair; identical result | the disposition is durable. For cases 1 to 5 this document's fault has ended and, if `install_pending`, the state is the accepted adoption "Closing, awaiting seed" shape carrying the repair. For the screening cases 6b and 6d the source's own phase is untouched and any different fault still stands (AG3-DES-025) |
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
| owner record (B1, B3) | `Settlement` | protocol allowance, 16 MiB per server | old plus new, about 18.5 KiB each at maximum (5.2's single canonical bound) |
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

**Mutation generation.** When Agent 1's resumable scanner is used, rotating its mutation generation
is **mandatory** for this scope, before the first possible IO of **every** repair-related path, not
only the headline writes: B1, B2, B3, B4, B5, B6, the Flow R fault write, temporary siblings,
cleanup and every failed operation. `verify_record` and reservation discipline do not invalidate a
parked scan and are not substitutes.

This scope adds **no writer** to the `DraftArchive` family (12.1), so I-4 imposes nothing new on it
from here; Agent 2's `write_studio_draft_archive_with_io` and `release_studio_draft_archive_with_io`
are already on I-4's audited participant list in `GATE4-AGENT-1-DESIGN.md` 9.2. Were a repair path
ever to gain such a writer, it would take `epoch_mutation_guard` on the same terms as every other
write above.

**Fairness.** One repair job per actor turn per server. Faulted and repairing targets rotate
round-robin using the same selection-index pattern as `rotate_owner`, so one permanently held fault
cannot monopolize the slot. The step runs after discovery, seed and page work. The existing 5 s
per-target cadence applies; any hold backs off to 60 s. Seed fetches use the existing four retained
selection slots, three paced attempts and sixty-second lifetimes; no new pool. Head rails are
unchanged, and a report rides an already-admitted query.

**One claim per target, for every source-mutating path** (AG3-DES-034, R31).
`repair_transaction_nonterminal(record, source)` from 6.5 is **the** target-claim fence. Revision 8
still named `repair_install_pending()` here, which is precisely the predicate AG3-DES-031 had just
established starts too late: `advance_checkpoint` calls `install_studio_seed_step`, a real
source-mutating install, so a B1-persisted case 6c repair was unclaimed against ordinary discovery
and could resume against a source whose shape no longer matched the decision B1 was made on, losing
its replacement obligation exactly as the report path did.

**The durable predicate is not enough on its own** (AG3-DES-043). It derives ownership from the
owner record and from `resolved_repair`, and a **non-owner** peer applying a distributed repair has
neither: U-9 keeps `EpochFaultRecord` owner-only, and before B2 its source carries no resolved
repair. Flow A is identical for owner and peer, so a peer's job has a real detached window in which
S2 runs while the actor continues and no durable state represents the job. S3's digest recheck
refuses to commit a stale plan, but that only detects the race after the fact: if ordinary settlement
wins, C-2's `Settled` arm holds and the repair becomes permanently unappliable. So the claim has a
runtime half:

```text
target_is_claimed(record, source, target) =
    repair_transaction_nonterminal(record, source)   // durable, owner side
 || live_repair_job_claim(target)                    // runtime, any peer
```

`live_repair_job_claim` is acquired at S1 before any body read, owned by the queued, worker and
ready-result bundle through S4, and released where the last `Arc` drops, with the same weak-handle
bookkeeping as the admission token in 10.3. Cancellation never releases it while a worker or result
still owns it. Every path below consults `target_is_claimed`, not just the durable half.

Every path that can mutate or install into the source consults the same fence and defers while it
holds:

| Path | Deferred while a repair transaction is nonterminal |
|---|---|
| report-induced sealing (6.5) | yes |
| `advance_checkpoint` and ordinary checkpoint adoption | yes |
| ordinary settlement and owner rotation for that target | yes |
| the repair step itself | it **is** the owner |

`repair_install_pending()` keeps its narrower job: it is the post-B2 predicate for whether a
**replacement** is still outstanding, and it is what stops ordinary adoption staging a second
`Rewound` snapshot of content a repair already staged as `Repair`. It is not the ownership
predicate, and 15.2 M17 removes the fence from ordinary advancement specifically.

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

## 12.1 The `DraftArchive` family is not repairable history

`EpochRecordKind` gained a sixth variant, `DraftArchive`, at `705d44b`
(`store/epoch_recovery/inventory.rs:48-69`). It is a closed shared enum, so this scope consumes the
variant rather than inventing one, and any `match` it adds over that enum must handle the new arm.
That is a compile-time obligation, not a judgement call: the existing arms are exhaustive at that
commit, so a missed arm fails to build rather than silently defaulting.

Physically it is its own family: a `.draft-archive` suffix, the
`catcoms/epoch-draft-archive-store/v1` scope domain, its own inventory key, its own sealed cap of
about 6 MiB plus 35 KiB, and a bounded authenticated reader in `store/epoch_draft_archive.rs`.
Accounting is **shared with Intents** through `EpochRecordKind::intent_class()`, charged against
`MAX_VAULT_INTENT_BYTES`, so it is a new physical family but not a new budget family. Coverage
gating matches Intents, which is safe because `collect_creative_references` refuses any coverage but
the complete scan.

**The rule for this scope: an archive is a preserved local draft, never a signed document record.**

- No repair path reads, decodes, replaces, retires, rewinds or reclaims an archive. The seam itself
  decodes nothing inside one, and neither does anything here.
- An archive is never evidence. It cannot supply a conflicting receipt, a pair, a repair, a seed,
  a close or a tenure, and it is never consulted when classifying a source under C-2.
- A repair's whole-version `Repair` recovery snapshot is computed from the source, exactly as
  today. An archive is not part of that computation and is not folded into it.
- Case 3, 4, 6a and 6c replacements leave archives untouched, on the same footing as the retained
  overlay branch in section 11: a replacement changes the source, not a preserved draft, and
  disposal of a draft is Agent 2's manual lifecycle.
- The bytes still exist for admission purposes, because they are in the shared Intents class and
  therefore in the budget a repair's writes are checked against. Counting them is the only
  interaction this scope has with the family.

This is **I-12** and 15.1 N40 asserts it. At the reviewed head the seam ships no writer and no
guard, by Agent 2's deliberate I-4 sequencing decision, so there is nothing for this scope to guard
today; section 10.3 records what would change if that stopped being true.

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

- **T1** *(restated for the accepted split, AG3-DES-040.)* The seam is
  `verification_owner_tenure_start()` and `authoring_owner_tenure_start()` over
  `Observed | Imported(u64) | Unknown`. This scope binds every mutation, drain and issuance to the
  **authoring** accessor and never to the verification one; the mapping is the table in 6.3.
- **T2** `None` from the authoring accessor, including `Imported`, stays a **hold**. It is never a
  `false` that permits a demotion, a drain or a release of proof suppression, and there is no
  substitution from the group epoch, a receipt's carried field or a repair's claimed field.
- **T3** Any further tenure work must keep both accessors' shapes and a freshness binding rechecked
  at **every** custody visit: verification, application, serving, drain and report admission. No
  tenure value is cached across an await or a store borrow, and identity is compared as a derived
  `tenure_id`, never as a bare start epoch.
- **T4** Fault tenure and issuer tenure are distinct (6.3) and must not be collapsed.
- **T5** The preview path must not render or act on a receipt the local book screens as a repaired
  loser.

Handed back: a replacement invalidates a retained overlay's Closing basis when the source's opening
receipt changed and the branch was not derived from the new one. The work stays retained; the
manual path is Agent 2's. Fail-closed `None` is correct but is not by itself evidence of eventual
progress; 15.1 N14 and N15 demonstrate progress with legitimate evidence.

**Consumed from Agent 2 at `705d44b`:** the `EpochRecordKind::DraftArchive` seam. This scope takes
the variant rather than adding one, handles it explicitly in any match it introduces, adds no writer
to the family, and treats an archive as a preserved local draft rather than repairable history. The
contract is section 12.1, the invariant is I-12 and the regression is N40. No other Agent 2 contract
is needed here beyond the tenure seam T1 to T5.

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
- **N2b** AG3-DES-014, at the core boundary and before any Studio or Registry round-trip depends on
  it: a `ReceiptBook` with a resolved repair, no `latest` and no `tenure` encodes at version 4 and
  decodes back equal, with its document derived from the resolved repair and every retained receipt
  checked against it. Assert that versions 1 to 3 gain no new representation, that a version-4 book
  whose retained receipts disagree with the derived document is still rejected, and that the
  `tenure.is_some() == latest.is_some()` rule still holds.
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
- **N5b** Case 5: a cross-tenure repair on each reachable **faulted** shape, including a faulted
  epoch-zero source, restores with `latest` and `tenure` consistent, is never `install_pending`,
  and permits an immediately following ordinary adoption of the current owner's checkpoint that
  stages a `Rewound` snapshot.
- **N5c** Case 6b, narrowed per AG3-TEST-004: the source's head is a **same-baseline higher
  receipt** whose ancestry receipts cannot express, so it is deliberately unclassifiable. It records
  the resolved repair, screens the exact loser, and performs no transition. A source on a *provably
  losing* baseline is N32(c)'s case 6c, not this one; revision 5's wording conflated them. Case 6a
  rewinds only when the repair is same-tenure and the opening is the loser.
- **N5d** AG3-DES-013, with AG3-TEST-003's correction to the assertion: owner B holds a **healthy
  Closing source with newer B-tenure progress**, none of whose anchors is covered by the repair, and
  applies a historical A repair. Assert the exact phase, head, `previous_until_installed`, adoption
  mode, accepted operation set, projection and **all non-repair source content** are unchanged, and
  that the source's receipt-book bytes differ **only** by the expected repair evidence and the
  advanced `repair_sequence`. Revision 4 claimed the stored bytes were unchanged, which case 6b
  makes impossible. Run the same for a healthy Open source and for a peer reached through Flow D.
- **N32** AG3-DES-017, four sources, none faulted, each given the same repair:
  (a) `adopting` with `latest == L`; (b) ordinary `Closing` with `latest == L`; (c) a head on a
  provably losing inherited baseline. All three must take case 6c, retarget to `S`, preserve the
  whole prior version as `Repair` recovery, and **must not subsequently install or settle the
  repudiated branch** through `prepare_checkpoint_adoption` or `prepare_settlement`, which is the
  assertion revision 4 would fail. (d) a same-baseline higher head whose ancestry is unknowable
  must take case 6b and be left alone, asserting the limitation explicitly rather than inventing
  ancestry.
- **N33** AG3-DES-019, both halves. (a) B1 persists repair RA for a historical pair, crash before
  B2, then the source faults on an unrelated pair B: assert RA still applies as screening and
  becomes terminal **and, simultaneously, that the persisted source still reports Fault on pair B**
  with the outcome `Screened` rather than `Repaired` (AG3-DES-025), that the source keeps its own
  fault byte-unchanged, and that a repair for pair B can then be issued and applied. (b) repair RA is at `AwaitingSeed` or `RecoveryPending` with
  `repair_install_pending()` true when a second pair arrives: assert the second pair is retained in
  the free record slot, that a different repair is `Held(RepairInProgress)`, that the single
  `resolved_repair` slot is not overwritten, and that RA's replacement still completes afterwards.
- **N36** AG3-DES-026, direct owner-record round trips, since revision 6's codec could not express
  these at all: (a) a source-bound repair with **zero** external pairs; (b) a source-bound repair
  with **two** external pairs, asserted to encode, decode and survive crash and reopen; the maximal
  shape including the reserved slot is nine receipt-sized values and is N37(c)'s; (c) a repair whose inline hashes **differ** from the live source's fault
  pair, which must be refused at use; (d) an external-bound repair whose index is out of range,
  refused at decode; (e) a source-bound repair reopened while the source is **missing or corrupt**,
  which must fail closed rather than assume. Plus AG3-DES-030's **post-B2 resume states**, which
  revision 7 could not express: (f) crash after B2 with `install_pending` true, reopen, and assert
  the inline pair binds against the committed `repair_state()` and the replacement resumes through
  B6 even though the live fault is legitimately gone; (g) crash after B2 on a terminal transition
  awaiting B3 and recycling, reopen, and assert it completes rather than refusing.
- **N37** AG3-DES-027, AG3-DES-031 and AG3-DES-032, four variants, all forcing the report-to-seal
  path that N33(b) never reaches. (a) **Post-B2**: RA sits at `AwaitingSeed`, a current-tenure
  report arrives, the source is not faulted, `repair_install_pending()` survives crash and restart,
  RA completes through B6, and the evidence becomes actionable only afterwards. (b) **Pre-B2**, the
  case revision 7 left unfenced: a healthy source on the loser of pair A, RA persisted at B1 needing
  case 6c, crash before B2, then a current-tenure report. Assert the source is **not** sealed into a
  different fault and that RA resumes as **6c with its replacement**, not as a terminal 6d
  `Screened`. (c) **Maximal capacity**: two external pairs plus a pending source-bound repair, then
  a current-tenure report; assert it lands in the reserved slot rather than being lost or displacing
  frozen evidence, and that the full **nine**-receipt record round-trips. (d) **Durable proof
  gate**: after (c), crash and reopen, then let an **unrelated member** request the head; assert no
  authoritative proof escapes even though the source is deliberately not Faulted and the admitting
  exchange is long over. (e) **AG3-DES-034, ordinary adoption rather than a report**: a B1-persisted
  case 6c repair, then `advance_checkpoint` becomes ready with a selected checkpoint for the same
  target. Assert it **defers**, that a crash and restart leave RA still applying as `Retargeted`
  case 6c, and that it reaches B6 with its replacement performed. (f) **AG3-DES-036, the recycle
  seam**: a terminal repair plus a live reserved pair plus a waiting external; recycle the repair,
  crash **before** the drain, reopen, and assert the live pair is handled before the external
  becomes issuable and that no new B1 transaction can open on the external meanwhile. (g)
  **AG3-DES-037, tenure change before the drain**: same setup, but ownership changes across the
  restart. Assert the pair is **retained**, is **not** live-sealed under the new owner, no longer
  suppresses authoritative proof, and is then actually **repaired as historical evidence** with a
  full B1 encode and reopen (AG3-DES-039): revision 9 asserted repairability that its codec could
  not express. The binding follows N45(c) rather than being asserted here: `repair_kind 1` when the
  pair migrated into an admissible slot, `repair_kind 3` when it stayed in `reserved`. Revision 11's
  wording combined migration with kind 3, which AG3-DES-047 makes incoherent (AG3-DES-053).
  (h) **AG3-DES-043, a non-owner pre-B2 job**: a peer receives a repair through Flow D and is paused
  in S2 while ordinary adoption and then ordinary settlement try to mutate the same target. Assert
  both defer on the runtime claim, that the claim survives cancellation of an unrelated job, that it
  is released where the last owner drops, and that the repair still commits at S3 rather than being
  refused as stale or stranded behind a `Settled` hold.
  (i) **AG3-DES-044, failure during the drain**: inject a failing and then an uncertain source-fault
  write mid-drain. Assert the slot and proof suppression survive both, that a crash between the
  source write and the slot clear leaves valid duplicate evidence, and that reopening cleans the
  duplicate idempotently without a second fault.
- **N42** AG3-DES-040, the tenure states, for issuance, application, drain and proof in turn:
  `Observed` permits, `Unknown` holds, and **`Imported(S)` holds for authoring while remaining
  `Some(S)` for verification**. Assert an `Imported` server never drains a reserved pair, never
  issues or applies a repair, and never releases proof suppression, and that the refusal comes from
  the authoring accessor rather than from a coincidental later check. Assert identity is compared as
  a derived `tenure_id`, by constructing two tenures that share a start epoch but differ in owner
  key and showing they are not confused.
- **N44** AG3-DES-045 and AG3-DES-048, the overflow hold as a durable object. (a) Encode, decode and
  reopen a record where **only** the overflow hold is suppressing proof, with strict `0/1`
  canonicality on `has_overflow` and `unknown`, a `fingerprint_count` above 4 rejected, and
  corruption failing the whole record rather than silently clearing the hold. (b) Two **distinct**
  current-tenure pairs overflow; one is later admitted and repaired; assert the other's fingerprint
  still suppresses proof and is cleared only when that exact pair is stored or resolved. (c) A fifth
  distinct pair sets `unknown`; assert no retry clears it and only the tenure ceasing to be current
  does. (d) AG3-DES-049: two tenures sharing a start epoch but differing in owner key produce
  different `tenure_id`s, and an overflow hold from one does not suppress or release under the
  other. (e) **Canonicality negatives** (AG3-DES-053): duplicate fingerprints, out-of-order
  fingerprints and the inert `has_overflow=1, count=0, unknown=false` shape are each rejected at
  decode rather than decoding into a hold on which `overflow_is_live` is false; and a hold whose
  last fingerprint is cleared canonicalises to `has_overflow=0` at the next write.
- **N45** AG3-DES-046 and AG3-DES-047, the reserved lifecycle. (a) The `{R1,R3}` pair after
  `{R1,R2}` was repaired: assert `pair_is_materialisable` is false, that **no** source mutation is
  attempted, that the active-pair rule permits direct issuance rather than demanding a drain, and
  that `repair_kind 3` carries it to terminal. (b) A materialisable live pair takes the drain and is
  then repaired as `repair_kind 2`. (c) The three migration cases, asserting the two bindings are
  never both applicable: **admissible room**, the pair migrates and B1 uses `repair_kind 1`; **no
  slot at all**, it stays and B1 uses `repair_kind 3`; and **numerically free but inadmissible**
  (AG3-DES-051), `pairs[0] = {R1,R2}` with `reserved = {R1,R3}` and one empty slot, where migration
  would create two externals sharing `R1`, so the pair stays and B1 uses `repair_kind 3` rather than
  being left with no legal path. (d) A terminal `repair_kind 3` clears `reserved` exactly, and after
  restart that pair can never become decidable again, the kind-3 analogue of N31c. (e) **The
  AG3-DES-050 negative**: an opening `O` and a same-tenure `R` that closes a **different** epoch with
  a differing `TenureSelection`, so `conflicting_receipt_pair` accepts it but `check_opening_receipt`
  would reject it. Assert the dry run reports it not materialisable, that no drain is attempted, and
  that it takes direct `repair_kind 3`. (f) Restore negatives for AG3-DES-052: a valid repair for
  pair Q encoded as `repair_kind 3` beside a reserved pair P, kind 3 with `reserved` absent, kind 1
  with an out-of-range index, and kind 0 with a repair present or `applied` true, each rejected.
- **N43** AG3-DES-041, repeated tenure at capacity: a live pair under tenure 1, an owner change
  demoting it, both history slots already full so it cannot migrate, a nonterminal repair under
  tenure 2, and then a **new tenure-2 conflict**. Assert `live_overflow` is set, that proof stays
  suppressed across a restart and for unrelated later requests, that the reporter's retry is
  accepted once a slot frees, and that no evidence was discarded at any point.
- **N41** AG3-DES-035, direct owner-record negatives for the reserved slot: malformed receipts, a
  foreign document, a non-conflicting pair, a non-canonical `has_reserved` byte, and a reserved pair
  that duplicates an external or the inline source-bound pair. Each must reject the whole record
  without resetting it. Assert the **positive** case that a reserved pair sharing a single receipt
  with an external is accepted, since the three-receipt shape requires it.
- **N38** AG3-DES-028 and AG3-DES-033, the complete disposition contract. (a) Apply through 6d,
  crash after B2, assert the exact retry returns `Screened` and **not** `Repaired`, with the source
  still Faulted on pair B; repeat for a healthy 6b source. (b) The same for **6a and 6c**, which
  revision 7 could not represent at all: assert `Retargeted`, that the required replacement is still
  outstanding after restart, and that they are never reported as `Screened`. (c) **Snapshot v3
  layout**: a v3 unit round-trips with `adopting` true and false and the decoder picks
  `decode_adoption` correctly, while v1 and v2 stay byte-identical. (d) **Successor lifecycle**:
  a `Screened` source undergoes ordinary adoption, and assert the successor carries both the
  resolved repair **and** its matching disposition, so an exact old repair retry against the
  successor still returns `Screened`. Revision 7's unchanged `adopted_successor` would drop the tag
  here. (e) Restore rejects a disposition naming a repair the book does not hold, a v3 unit with no
  resolved repair, and a `Transitioned` or `Retargeted` tag whose book and gate could not have come
  from that case.
- **N39** AG3-DES-029, three genuine conflicting receipts and two peers: P1 freezes `{R1,R2}`, P2
  freezes `{R1,R3}`. The owner repairs `{R1,R2}` selecting R2, so R1 becomes a repaired loser. P1
  heals. Assert P2's re-report of `{R1,R3}` is **admissible**, that the owner can issue a
  higher-sequence repair for it, and that P2 then heals. Run for both winner choices, and assert the
  record still refuses two externals sharing a receipt hash so P2 simply waits its turn rather than
  being told it is already resolved. Per AG3-DES-042, assert the owner's durable evidence is
  **exactly `{R1,R3}`**, not merely that some fault occurred: feeding R1 through the live seal yields
  `Stale` and feeding R3 against a head of R2 yields `{R2,R3}`, so the test must show the exact pair
  was preserved owner-side and repaired through `repair_kind 3` rather than reconstructed from the
  source.
- **N35** AG3-DES-024, at the **core** boundary on `StudioEpoch::apply_receipt_repair`, not only on
  `ReceiptBook::apply_repair`: an exact previously resolved repair, the same device key, an
  independently observed **different** current tenure, and an outstanding continuation
  (`install_pending` true). It must fail the live-authority check, not return `AlreadyResolved` and
  not resume the replacement. Assert the store's own `verify_current_owner` is not what produced the
  refusal, by driving the core API directly.
- **N34** AG3-DES-020: a **healthy** source holds resolved repair sequence N; a valid, correctly
  signed, current-tenure repair with sequence N-1 for a different pair arrives through Flow D.
  Assert `Held(SequenceNotNewer)` and that the complete book and source are byte-unchanged. Repeat
  with the source faulted, where `apply_repair` already enforced this, to show both paths agree.
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
- **N18** Split per AG3-TEST-004, since `PairMismatch` no longer exists. (a) **Malformed or wrong
  evidence** (a pair that fails `conflicting_receipt_pair`, a repair whose `receipt_hashes` do not
  match the supplied pair, a wrong document or channel) must refuse and change nothing. (b) A
  **valid repair for an unrelated pair** must take case 6d: it screens, becomes terminal, returns
  `Screened`, and leaves this source's own active fault byte-unchanged. Neither touches an
  unrelated document.
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
- **N24** `complete_checkpoint_hint` returns a hint carrying a repair and still refuses one carrying
  a proof. Per AG3-TEST-003 the discard claim is **withdrawn**: a verified repair for a document
  with no local fault is now applied as screening through case 6b, 6c or 6d, and only a repair that
  fails verification, or names a document this peer does not hold, is discarded.
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
- **N28** AG3-DES-006, AG3-DES-011 and AG3-DES-015, carried through the **whole** lifecycle rather
  than stopping at the first issuance. From N7's shape (b), `high_water = R(e-1)` with a losing
  `in_flight = L(e)` and the winner held only by a peer: assert the **actual served receipt and
  proof** are `S(e)` and not `R(e-1)`; complete that proof through `complete_studio_head`, which
  revision 3 left with no valid path, and assert `S(e)` becomes `high_water` with `reconciled`
  cleared; restart; publish `e+1` and assert `canonical_head()` does not regress to `S(e)`;
  then prepare `e+2`. Also assert a stale completion retry for `R(e-1)` is inert, and that a tenure
  change clears a reconciled head. Repeat for Registry.
- **N29** Two real peers enter Fault from two valid conflicting receipts, the owner repairs, both
  converge, a third peer joins after the repair, and every peer's installed state, receipt evidence,
  recovery contents and native events agree. `settlement-changed` carries `fault`, then `repairing`,
  then the real phase.
- **N31** AG3-DES-016, both orderings, each ending in convergence rather than deadlock.
  (a) The owner source faults on a current-tenure pair, then a peer reports a historical pair: the
  reported pair lands in `deferred`, the source's pair stays active and is repaired first, the
  deferred pair is then promoted and repaired. (b) The record holds an unresolved historical pair,
  then the owner source faults: the historical pair is demoted to `deferred` without loss, the
  source's pair becomes active, and both are repaired in turn. In both, assert no evidence is
  discarded, the record stays at two pairs plus one repair, a third distinct pair is refused with a
  defer reason, `StudioFaultView` names which pair is decidable, and ordinary receipt issuance is
  **not** blocked by an unresolved record pair. Also assert the maximal shape, two distinct external
  pairs plus one repair, encodes, decodes and survives crash and reopen at every barrier, and that
  an aliased or half-duplicated pair is rejected (AG3-DES-018).
- **N40** I-12, the `DraftArchive` seam (12.1). A document with a populated archive goes through a
  full repair, including a case 3 rewind that replaces the source and stages `Repair` recovery.
  Assert the archive file's **bytes are unchanged** at every barrier and after restart, that no
  repair path opened, decoded or removed it, that it contributed nothing to the recovery snapshot or
  to C-2 classification, and that its bytes were nonetheless counted in the Intents-class budget the
  repair's writes were admitted against. Repeat with the archive absent, which must behave exactly
  as a vault with no archive file does today. Also assert every `match` this scope adds over
  `EpochRecordKind` handles `DraftArchive` explicitly rather than by a catch-all arm.
- **N31b** AG3-DES-022's closure, the state revision 5 could not represent: two unresolved
  historical pairs are already retained **and** the local source then faults on a third,
  current-tenure pair. Assert the source's pair is selected as active without occupying a record
  slot, that a repair for it can be issued and applied, and that after restart all three are
  repaired sequentially with no evidence lost.
- **N31c** Terminal-pair recycling: repair A becomes terminal, its pair is removed and the repair
  field cleared, repair B is then issued for the remaining pair, and after restart **A never becomes
  decidable again**. Assert the removal is idempotent on an exact retry and that re-reporting A is a
  no-op because the book already screens it.
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
| M2 | recovery precedes replacement (I-2) | **Corrected twice.** Revision 2's mutant leaked nothing because the caller still received `Err`; revision 3's was masked by R20, since a failed accounted write leaves `ready = false` and the successor `reserve` then fails on `Reconcile` whatever was minted. The mutant now **bypasses the recovery save entirely, before it reserves**, returning `Ok(Some(capability))` with the budget untouched and valid, so the successor writer is genuinely reachable and the missing recovery is the only thing that can fail the assertion | N11 variant: the losing projection is **not** readable from recovery after a successor write that should have been unreachable |
| M3 | the shared live-authority guard (I-3) | remove the `issuer_tenure_start_group_epoch == Some(expected)` clause **inside `ReceiptRepair::verify_current_owner`**, the one guard both layers reach | N17 with a correctly signed v2 record whose claimed start is a genuinely earlier tenure of the same key, against a different observed current start: the refusal fails |
| M4 | core retry ordering | move the retry shortcut in `ReceiptBook::apply_repair` before `verify_current_owner` | the **core** returning-owner retry regression, not the store-level N17, which is separately labelled redundant for this claim |
| M5 | the checker boundary | remove the pair-equality condition inside `check_evidence`, using pairs that **share the selected receipt** so membership cannot mask it | a direct `check_evidence` unit assertion. `apply_repair`'s own sorted-hash comparison is retained and labelled redundant for this claim |
| M6 | no retirement (I-5) | insert a receipted-retirement call into the repair transaction | N20: the eligible ordinary full-envelope entry survives |
| M7 | save before serve (I-6) | remove the required flush before serving | N23 with visible-but-uncertain post-rename state and a failed required reflush, asserted for both origins and for a non-proof peer answer |
| M8 | continuation versus completion (I-7) | map `AlreadyResolved` to terminal `AlreadyRepaired`, which is revision 1's actual bug | N12: after the retry with a seed, the successor is not installed and the losing projection is not readable |
| M9 | the single eligibility guard | remove the `eviction_pending` refusal in `stage_studio_repair_recovery` before the mint | N15: no successor bytes are written while the warning is pending and before its deadline. A due promotion without acknowledgement is a separate positive control |
| M10 | losing-baseline screening (I-9) | remove `is_repaired_loser`'s losing-baseline arm | a covered losing-baseline descendant remains `Stale` **without re-faulting**. (Revision 1's assertion here was wrong and is withdrawn) |
| M11 | no over-suppression (I-9) | make `is_repaired_loser` return true for any same-tenure receipt | a genuine third differing baseline still faults |
| M12 | report admission (I-10) | **Corrected twice.** Revision 2's mutant was masked by core's independent `Fault`/`Stale`; revision 3's assertion was then invalidated by AG3-DES-016, because a second distinct pair now legitimately fills the free slot, so the restored design would fail its own named assertion. The fixture starts with **both record slots occupied by unresolved HISTORICAL pairs and a third HISTORICAL report arriving**, which AG3-TEST-004 requires: a current-tenure report must still durably fault the source even when the record is full, so suppressing that write would be unsafe and must not be what this mutant appears to test. The mutant removes the refusal so the third pair displaces retained evidence | N26 and N31: with both slots occupied, a third distinct historical report leaves both retained pairs byte-unchanged **and** reaches no source writer, reservation or seal |
| M13 | a repair transaction fences report-induced source mutation (AG3-DES-027) | remove the **whole** `repair_transaction_nonterminal` fence from `report_studio_fault`, not just its `repair_install_pending()` arm as revision 7 proposed | N37(a): the predicate is still true after the report and RA completes through B6. Under the mutant the phase becomes `Fault`, the predicate goes false and the replacement is abandoned |
| M15 | the fence covers the pre-B2 interval (AG3-DES-031) | remove only the **owner-record-pending** arm of `repair_transaction_nonterminal`, leaving the post-B2 arm intact | N37(b): the B1-persisted RA still resumes as `Retargeted` case 6c with its replacement. Under the mutant the source is sealed into `Fault(B)` and RA silently degrades to a terminal `Screened` 6d, which M13 alone cannot show |
| M16 | the durable proof gate (AG3-DES-032) | remove the `reserved_is_live(...)` term from `authoritative_proof_allowed`, so head selection consults only the source's own fault | N37(d): after crash and reopen, an unrelated member's head request obtains no authoritative proof. Under the mutant it obtains one for a disputed receipt |
| M17 | the claim fence covers ordinary adoption (AG3-DES-034) | remove `target_is_claimed` **only** from `advance_checkpoint`, leaving the report path and the repair step fenced | N37(e): ordinary adoption defers and the B1-persisted repair still reaches B6 as `Retargeted` case 6c. Under the mutant the source is installed into from under the transaction and the replacement obligation is lost, which M13 and M15 cannot show because neither touches this writer |
| M18 | the runtime half of the claim (AG3-DES-043) | remove the `live_repair_job_claim(target)` term from `target_is_claimed`, leaving the durable half | N37(h): a non-owner repair paused in S2 still commits at S3. Under the mutant ordinary settlement mutates the target first and the repair is refused or stranded behind a `Settled` hold. M17 cannot show this, because a peer has no durable claim to remove |
| M19 | the fence covers settlement and rotation too | remove `target_is_claimed` **only** from ordinary settlement and owner rotation for the target | N37(h) second half: settlement defers while a repair job is claimed. Revision 9 asserted these paths consult the fence but proved it only for `advance_checkpoint` |
| M14 | the report no-op is the exact pair only (AG3-DES-029) | broaden the no-op from "this exact pair" back to "contains any `is_repaired_loser` member" | N39: P2's `{R1,R3}` report is admitted and P2 eventually heals. Under the mutant it is silently treated as covered and P2 is stranded |

M13 to M19 target guards unique to this scope: core knows nothing about report admission, owner
records or head-proof gating, and `is_repaired_loser` deliberately cannot distinguish a peer's
frozen pair from a screened arriving receipt (R28). M15 exists because M13 alone cannot reach the
pre-B2 interval, M16 because N37(d)'s proof assertion otherwise has no mutated guard behind it, and
M17 because neither M13 nor M15 touches the ordinary adoption writer. The disposition codec and the
reserved slot's priority and lifecycle are covered by N38, N37(f), N37(g) and N41's direct
round-trip, crash-seam and negative assertions rather than by further mutants, since their failures
are representation and ordering errors rather than removable checks.

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

U-9 and U-10 are decided as the reviewer answered. **U-9**: `EpochFaultRecord` stays owner-only;
after AG3-DES-013's correction a non-faulted peer retains ordinary resolved-repair evidence through
case 6b, so no general member-controlled evidence surface is introduced. **U-10**: no automatic
expiry; an unresolved frozen pair lives until a verified repair resolves it or the document is
destroyed by its normal lifecycle, because expiring it could strand faulted peers whose only
evidence was deliberately retained there.

**U-11** is agreed by the reviewer and adopted: the second record slot accepts evidence only from a
reporter that is itself faulted, consistent with U-9. A member that merely witnessed an
equivocation without installing either side cannot forward it, which keeps the durable
member-controlled evidence surface exactly as narrow as U-9 decided. U-10's no-expiry rule still
applies once a report has been accepted.

No new questions are opened by this revision. Every U-question is now decided.

## 17. Re-review request

Copyable, with the common contract from
[GATE4-REVIEW-PREAMBLES](GATE4-REVIEW-PREAMBLES.md#common-contract-for-the-four-subsequent-reviews)
sent alongside it.

```text
Review type: design, revision 11, findings re-review.
Base: 11ce6f1b58288e44d6ff14dc2a98f42f7cc5e13b (revision 10 design body). Head: [FULL_HEAD_SHA].
Compare: https://github.com/Thalpy/Mewtual/compare/11ce6f1b58288e44d6ff14dc2a98f42f7cc5e13b...[FULL_HEAD_SHA]
Note: like revision 10, this revision is committed in several parts because a parallel session
repeatedly resets this shared working tree and discards uncommitted edits. Every part touches only
the two Agent 3 documents; f5ac522 between the two revisions is status-only.
Earlier revisions: 3b6a4b40462ae83a341f8f6741c93edff55b5ef7 (revision 9),
b8bb5f3a3db6d0b8e450c82119f95e2cb929bce6 (revision 8),
6d498c2e901e5071104a2533e0a632dc5676b2a7 (revision 7),
135766ca9f290ab96d3133b771bc42da74fe7825 (revision 6),
3737f1d4fff6f2c08302b0ecb1b024008818a1cf (revision 5),
ad023d2e5b514a8f9598b1fe433fd2b080ecad6c (revision 4),
a62178b94f20cd60a5363e6a3d6d6216edb9e516 (revision 3),
63a11e1a6451c7ed373c90b0e81b59d8a748a72a (revision 2),
7efc9c2aba0a37d9aec57e268d9ff63edaca1b8a (revision 1), original scope base
1bcb1bca204d721b848b17c0835faf931ae930e3.
Scope/evidence: docs/GATE4-AGENT-3-DESIGN.md revision 11 and docs/GATE4-AGENT-3-STATUS.md.
Documentation only: no production code, test, shared contract document or workflow is changed, and
no Cargo command was executed in any of the eleven passes. Every number is a source constant or a
labelled estimate. The commit sits on a branch shared with Agents 1 and 2, so a literal base-to-head
comparison may again contain intervening Agent 1 production commits; only the two Agent 3 documents
are mine.
Dependencies, corrected per your note: Agent 2's tenure design has now PASSED adversarial review,
though none of it is implemented, so the section 13.2 contract is accepted-on-paper rather than
available; Agent 1 remains partial with I-4/C-3 not started; the core signing split at e65bfd8
remains unreviewed.

This revision answers AG3-DES-045 to AG3-DES-049 and AG3-TEST-009. All five were internal
contradictions in revision 10 and all five are confirmed. Section 0 is the disposition table. It
reopens nothing you closed: AG3-DES-040 in 6.3 and 13.2, AG3-DES-043, AG3-DES-044's write order,
AG3-DES-038, 030, 023, 024 and 029's dedupe rule all stand, and M1 to M19 are unchanged.

AG3-DES-045 is the one you called sharpest and it was exactly that: I added the hold to the struct
and to the proof gate and never to the codec, so the mechanism advertised as surviving restart had
no bytes, and N43 as written was impossible. It now has a canonical encoding with strict 0/1
canonicality, an explicit count bound, validation and corruption failing the record rather than
silently releasing the hold. N44(a) reopens a record where only the hold is suppressing proof.

AG3-DES-046: the two rules really did leave no legal next step for {R1,R3}. A
pair_is_materialisable predicate, computed on the already-checked source WITHOUT mutating it, splits
drainable from direct-repair-only, so the impossible case is never reached by first faulting the
source into some other pair. N45(a) asserts no source mutation is attempted and that issuance is
permitted without the drain.

AG3-DES-047: both halves were wrong. Migration now happens before any B1, so a migrated pair is
kind 1 and kind 3 applies only while the pair is still reserved; N37(g)'s "migrates and then kind 3"
was incoherent and N45(c) picks one per case. Recycling is binding-specific, so a terminal kind-3
repair clears reserved instead of stranding its pair there, which N45(d) asserts across restart.

AG3-DES-048: clearing a pairless marker on one retry forgets every other known conflict, which is
the safety half of AG3-DES-032 returning one tenure later. The hold keeps up to four pair
fingerprints cleared individually, plus a sticky unknown flag for the overflow-of-overflow case that
only a tenure change clears. It still stores no receipts, so it can only refuse, never authorize.
N44(b) and N44(c) cover both.

AG3-DES-049: the stale observed_tenure_id pseudocode is gone, both predicates compare against one
tenure_id derived in the same custody visit from the authoring accessor, the group id and the
committer key, and the hold stores that derived id rather than a bare start. N44(d) constructs two
tenures sharing a start epoch with different owner keys and shows they are not confused.

AG3-TEST-009: N44 and N45 are new and cover the codec, multiple overflows, tenure identity,
materialisability, the migration and binding choice, and terminal kind-3 clearing.

No U-questions remain open.

Superseded text below is retained for the earlier round it answers:

This revision answers AG3-DES-039 to AG3-DES-044 and AG3-TEST-008, and absorbs a dependency change
this scope had missed: Agent 2's accepted design REMOVES observed_owner_tenure_start for a
verification/authoring split with a fail-closed Imported, which sections 6.3 and 13.2 were still
written against. Section 0 is the disposition table. It reopens nothing you closed: AG3-DES-038
stays closed alongside AG3-DES-030, 023, 024 and 029's dedupe rule, and M14 and M1 to M11 are
unchanged.

AG3-DES-039: a reserved pair could be selected as active work and then not be nameable at B1, since
repair_kind covered only none, external index and source-bound inline. A fourth kind binds directly
to the slot, which is cleaner than migration because migration has no guaranteed capacity. N37(g)
now runs a full B1 encode and reopen for a historical reserved pair.

AG3-DES-040: correct, and I had missed the contract change entirely. Section 6.3 now has an explicit
per-use accessor table: issuance, application and the drain decision all bind to the AUTHORING
accessor, where Imported and Unknown are holds rather than false; verification may only add
refusals and can never authorize a drain, seal or issuance; and identity is compared as a derived
tenure_id rather than a bare start epoch. T1 to T3 are restated for the accepted split and N42
asserts the Imported case fails closed for authoring while staying Some for verification.

AG3-DES-041: leaving a demoted pair in the slot closes one owner change and reopens AG3-DES-032 on
the next, exactly as you show. Demotion now migrates into the history list when there is room, and
where there is not, a new live conflict sets a durable evidence-free hold carrying the tenure it was
seen under. It suppresses proof on the same derived rule while the reporter retries and cannot
authorize anything. N43 runs the repeated-tenure-at-capacity path.

AG3-DES-042: confirmed at the source. is_repaired_loser screens before any conflict comparison, so
after {R1,R2} is repaired for R2, R1 yields Stale and R3 against a head of R2 yields {R2,R3}. The
reported pair therefore cannot always be re-derived, and admission keeps it owner-side under the
proof gate, repairable directly through the new binding. N39 now asserts the durable evidence is
exactly {R1,R3}.

AG3-DES-043: also correct. A peer has no owner record by U-9 and no resolved_repair before B2, so
the durable predicate cannot represent its in-flight job and ordinary settlement could win into a
Settled hold that makes the repair permanently unappliable. target_is_claimed adds a runtime claim
acquired at S1 and owned through S4 with the same weak-handle discipline as the admission token.
N37(h) pauses a peer job in S2 against both adoption and settlement, M18 removes the runtime half,
and M19 covers settlement and rotation, which revision 9 claimed but proved only for
advance_checkpoint.

AG3-DES-044: the drain is now its own crash-safe transaction. The source fault write and its
durability return first, the slot is cleared only afterwards, a crash between leaves valid duplicate
evidence that the existing redundancy rule cleans idempotently, and an uncertain write leaves both
the slot and proof suppression intact. N37(i) injects failures between the two writes.

AG3-TEST-008: N39 gains exact-pair identity, N42 and N43 are new, N37 gains (h) and (i), and M18 and
M19 are added.

No U-questions remain open.

Superseded text below is retained for the earlier round it answers:

This revision answers AG3-DES-034 to AG3-DES-038 and AG3-TEST-007. Section 0 is the disposition
table. It reopens nothing you closed or accepted: AG3-DES-030 stays closed alongside AG3-DES-023,
AG3-DES-024 and AG3-DES-029, and M14 and M1 to M11 are unchanged.

AG3-DES-034 is confirmed at the source: advance_checkpoint calls install_studio_seed_step, and the
paired bucket's install_registry_seed_for_studio, so ordinary discovery is a real source-mutating
install with exactly the ownership hazard the report path had. Section 10.3 was still naming
repair_install_pending() as the claim predicate, which is the very predicate AG3-DES-031 had just
established starts too late. repair_transaction_nonterminal is now THE single target-claim fence,
consulted by report sealing, ordinary checkpoint adoption, ordinary settlement and rotation, and the
repair step itself; repair_install_pending() is demoted to the post-B2 replacement capability it
actually is. N37(e) drives the adoption path rather than a report and M17 removes the fence from
that writer alone, which neither M13 nor M15 can reach.

AG3-DES-035: correct, and the inconsistency was entirely mine. The slot was on the wire and in the
proof gate while the struct and check_scope knew nothing about it. It is now an explicit
reserved: Option<FaultPair> with full codec and validation invariants. Taking your warning: its
alias rule deliberately permits a shared receipt, because the accepted three-receipt shape requires
exactly that; what it forbids is the same PAIR appearing twice. The stricter external-external rule
stays, since a historical pair can wait a turn while a live one gates proof service and has nowhere
else to go. N41 covers the negatives and the positive shared-receipt case.

AG3-DES-036: also correct. The slot was missing from the derivation, so after recycling cleared a
terminal repair a crash before the drain let the derivation pick a historical external, and a fresh
B1 transaction on old evidence could postpone the dispute over the current source for its whole
length. A live reserved pair now outranks all historical work and the repair field cannot be reused
for an external while it waits. N37(f) runs the exact crash seam.

AG3-DES-037: the sharpest of the five, and the fix is to stop storing the classification. Liveness
was a judgement made at admission against the then-current tenure, so an owner change before the
drain left the pair either sealable under an owner it does not dispute or suppressing proof forever,
contradicting the adjacent rule about historical evidence. The record now stores only the pair and
reserved_is_live is recomputed from freshly observed tenure at every custody visit, consistent with
T2. Demotion therefore needs no write, cannot fail, and raises no capacity question at all: the pair
simply stops being live and remains retained and repairable as history. N37(g) asserts retention,
no live seal under the new owner, proof no longer suppressed, and eventual repairability.

AG3-DES-038: the contradiction was real and dangerous as guidance. "adopted_successor is unchanged"
is gone; every successor constructor that clones the receipt book clones the book, the matching
disposition and its exact repair hash and sequence binding, and a successor carrying a resolved
repair with no disposition or a mismatched binding is rejected on restore.

AG3-TEST-007: N37 gains (e), (f) and (g), N41 adds the direct owner-record negatives, M17 is added,
and N36(b)'s stale seven-receipt wording is corrected to nine.

No U-questions remain open.

Return a design verdict for this bounded repair scope, or numbered findings with concrete failure
paths and required corrections. Implementation, integration and full Gate 4 remain separate.
```

