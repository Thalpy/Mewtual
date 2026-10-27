# Agent 3 core follow-up findings and dispositions

Implementation base: `918ffb9b3034c43ed33574225e04f1be2e87090d`.
Latest independent verdict: **revision 16 PASS at `2b741e7`**. CORE-006/007 and TEST-014's
design plan are closed. The C-7 journal leaf is implemented alongside C-3/C-4/C-8 and C-5's
sequence getter; report admission, typed transitions and runtime integration remain pending.
See [status](GATE4-AGENT-3-STATUS.md) for execution and implementation-review findings.

| Finding | Current disposition |
|---|---|
| CORE-001 High | User accepted: clear rejected quarantine only on repair to Open; preserve accepted work/accounting |
| CORE-002 High | User accepted with exact-state rebinding: private unchanged-Fault screening commit |
| CORE-003 High | User accepted only for the effective canonical reconciled decision |
| CORE-004 High | Bounded design PASS; C-7 journal leaf implemented, integration pending |
| CORE-005 High | **Bounded design PASS**, including stated finite-history/convergence limits; Agent 2 seam and production tests still required |
| CORE-006 P1 | CLOSED at design: independent effects with a joint compatibility postcondition |
| CORE-007 P1 | CLOSED at design: turnover may strand repair from B1 through durable terminal/recycling |
| IMP-001 Medium | **PASS/CLOSED**; implemented at `f16e1e5`, regression isolation at `27e1b99` |
| TEST-013 | Same-document headless negative implemented; real report/no-write negative remains unimplemented |
| TEST-014 P2 | Design plan CLOSED; N3b/N51 asymmetries and N52 churn execution remains pending |

## CORE-001: reopening with quarantine is not restorable

Closing can retain bounded rejected hashes, and Fault preserves them. Open decode requires an
empty quarantine; leaving it intact either breaks restore or later places the same hash in both
accepted and quarantined sets.

Accepted correction: clear only rejected quarantine under the same gate lock when repair chooses
Open. Preserve accepted operations, IDs, byte totals, per-device totals and signed log. Closing
targets retain quarantine. Required typed Studio/Registry regression: Closing -> rejected inbound
-> Fault -> reopen by repair -> snapshot/reopen -> ordinary re-admission -> snapshot/reopen.

## CORE-002: screening cannot use the ordinary transition commit

C-2 case 6d must keep unrelated Fault(B) while recording repair A. C-1's normal Open/Closing targets
cannot represent that. A phase-only equality would also admit a stale plan after fault evidence or
its receipt binding changed.

Accepted correction: a distinct private screening candidate/commit, rechecking the exact planned
gate phase, relevant receipt hash, fault pair and book repair state under lock. Preserve B's bytes
and gate while changing only validated screening evidence/provenance. Settled refuses; this is not
a generic Fault-to-Fault writer. N33/N38 must run through both typed APIs and restart; receipt head
still reports Fault(B), exact retry is Screened, and B remains repairable.

## CORE-003: an unpublished canonical reconciliation can be repaired again

R1 -> repair to R2 -> repair of effective R2 to R3 before publication cannot ignore `reconciled`.
The accepted correction applies only to the **effective canonical decision**. Historical high_water
must not override unrelated canonical or pending work. Validate complete evidence and live repair
before replacing R2 atomically. Keep source sequence and nonterminal/recovery fences.

Required regression: consecutive repairs without publication, restart, publish R3, restart and
prepare the next receipts; malformed evidence leaves all state unchanged.

## CORE-004: roles, cross-baseline rewind and bounded provenance

The old contract conflated journal tenure identity with receipt inheritance, allowed historical
high_water matches to clobber newer canonical decisions and demanded increasing epochs across a
repair-selected baseline. It also failed to preserve a retired pending descendant outside the
repair pair.

The concrete [journal proposal](GATE4-AGENT-3-AUTHORITY-FOLLOWUP.md#core-004-publication-facts-obligations-and-canonical-decisions)
separates identity, actual publication, pending obligation and canonical repair choice. It allows
published R(4,A) with reconciled S(2,B), preserves complete retired pending receipt/close at B1,
retains bounded latest proof, rejects stale callbacks and gives evidence-only cleanup a publication
hold. The independent review accepted that direction and its bounds/provenance, but found
CORE-006/007 below. No v2 journal implementation has begun; re-review remains required.

## CORE-005: self-signatures are not historical owner authority

C-4 correctly proves bounded canonical shape, self-signature and conflict. A current authenticated
member can sign a conflicting pair for the right document despite never being owner. The old
report contract would admit it into permanent evidence capacity, owner choices and proof gates.

No report consumer exists yet, so this is a confirmed future contract defect, not a demonstrated
present network-reachable exploit. The C-4 API remains narrow and documents this boundary.
The new primitive member-forgery test is a limitation demonstration, **not** the missing
report-admission/no-write security regression.

The [accepted authority contract](GATE4-AGENT-3-AUTHORITY-FOLLOWUP.md#core-005-local-historical-owner-provenance)
uses one receiver-local archived Observed-tenure witness in the matching durable MLS snapshot,
then private exact-pair admission attestations. Every authority check precedes durable capacity,
source, overflow and proof changes. Reserved-first staging retains attestations until B1 transfers
them atomically. Unresolved pairs retain eligibility; terminal recycling removes their attestations.
Agent 2's tenure seam and Agent 4's snapshot integration are dependencies, not implemented APIs.

The independent review of `a751369` explicitly accepted the finite-history limits: newcomers/
Imported history, evicted unadmitted history, re-reported completed pairs after recycling and
device-local attestations. Universal peer convergence is not claimed and the limitation must
remain in the eventual product/integration verdict. Owner-turnover timing is the separate
CORE-007 correction below. This acceptance is design-only, not production N49/N50 evidence.

## CORE-006: compatibility is not classification equality (P1)

Case 1a can clear source Fault on `{S,L}` by selecting installed opening S while positively
preserving later sealing H. A journal whose effective choice is H must stay unchanged, whether
H is high_water or in_flight. Requiring equal effects rejects this accepted path at B1.

Revision 16 instead computes source and journal candidates independently. Their joint check
requires no resulting covered journal choice or usable source proof/settlement/install anchor,
with explicit guarded retention of losing recovery and unrelated Fault evidence. All ordinary
authority, exact-state, sequence and recovery rules remain. Different effect tags and different
non-losing heads can be compatible; the existing three-way proof equality is not weakened.

N3b's integrated extension must exercise `(Transitioned, NoChange)` with both H roles, B1/B2
and restart. N51 adds `(Screened, Replace)` and pending retirement, plus covered-target/missing-close
refusals. These are planned tests, not implementation evidence.

## CORE-007: turnover can strand the transaction before B2 (P1)

After A saves B1, its repair already owns the target and may have changed the journal. If B
becomes the MLS owner before B2, A fails live verification and B cannot overwrite the nonterminal
repair. No B2 source transition exists to finish. A journal NoChange does not remove that record
claim, and the ordinary newer-tenure journal path cannot bypass it.

Revision 16 chooses the explicit fail-closed limitation from **B1 until durable terminal/recycling**,
as the reviewer offered, rather than adding cancellation. Pre-B2 source state stays unchanged;
post-B2 can also retain committed source/recovery work. N52 must cover both intervals through
real MLS turnover and restart, including Replace, retired-pending and NoChange, refusal of old/new
owner repair attempts and ordinary prepare/reset/proof/publication bypasses. Retain all evidence;
same-transaction valid progress and already-terminal inert cleanup keep their accepted rules.

## IMP-001 and implementation re-review

`ReceiptBook::decode_mode` now rejects `previous_until_installed` whenever `latest` is absent,
even with valid same-document resolved evidence. A signed same-document predecessor regression
covers both v4/v5, with valid headless controls. The ninth mutation removes precisely this guard.

The independent review at `a751369` explicitly **closed IMP-001** based on the actual code,
isolated regressions and green final 228-test/nine-mutation CI on both platforms.

Read-only implementation review found no BLOCKER/HIGH/MEDIUM. Its LOW scope-negative masking gap
was fixed at `27e1b99`: an otherwise valid **headed** repaired book accepts a same-document
predecessor and rejects a re-signed foreign predecessor, independently in both modes. The short
re-review closed that gap. Local focused suite: **20 passed, 0 failed**. Complete check/run evidence,
including failures and pending work, is in [status](GATE4-AGENT-3-STATUS.md).

## Historical revision-15 internal review

The read-only design reviewer inspected the actual proposal and neighboring snapshot, tenure,
source and journal paths. It found and then re-reviewed corrections for:

- exact attestation retry ordering after archive turnover;
- cross-tenure healthy-journal screening before replacement-only identity checks;
- lost attestation on source drain and terminal recycling;
- evidence-only provenance after subsequent publications;
- explicit restore epoch/device checks and mid-replacement turnover limitations.

That internal verdict did not replace independent review, which subsequently found CORE-006/007.
Revision 16's new read-only adversarial review inspected the actual corrections, test matrix and
surrounding enforcement paths. No BLOCKER/HIGH/MEDIUM remains preventing independent re-review;
one LOW CORE-005 accepted/proposed label mismatch was corrected and re-reviewed. This static
verdict is not execution or independent acceptance. The requested independent verdict is limited
to CORE-004's CORE-006/007 and TEST-014;
preserve the accepted CORE-001/002/003/005 and closed IMP-001. Report/no-write, historical-owner
positive and typed lifecycle implementation regressions remain mandatory future work.
