# Agent 3 core follow-up findings and dispositions

Implementation base: `918ffb9b3034c43ed33574225e04f1be2e87090d`.
Independent review: REQUEST CHANGES at `8d4cc53a529c86bdad76170f7825f7dbd682dfa9`.
Revision 14 remains the baseline with the corrections below. The leaf checkpoint implements
C-3/C-4/C-8 and C-5's sequence getter only; no report, typed transition or journal format exists.

| Finding | Current disposition |
|---|---|
| CORE-001 High | User accepted: clear rejected quarantine only on repair to Open; preserve accepted work/accounting |
| CORE-002 High | User accepted with exact-state rebinding: private unchanged-Fault screening commit |
| CORE-003 High | User accepted only for the effective canonical reconciled decision |
| CORE-004 High | Concrete revised journal-role/provenance proposal; independent verdict pending |
| CORE-005 High | Concrete local Observed-owner provenance proposal; independent verdict pending |
| IMP-001 Medium | Implemented at `f16e1e5`; regression isolation refined at `27e1b99` |
| TEST-013 | Same-document headless negative implemented; real report/no-write negative remains unimplemented |

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

The concrete [revision-15 proposal](GATE4-AGENT-3-AUTHORITY-FOLLOWUP.md#core-004-publication-facts-obligations-and-canonical-decisions)
separates identity, actual publication, pending obligation and canonical repair choice. It allows
published R(4,A) with reconciled S(2,B), preserves complete retired pending receipt/close at B1,
retains bounded latest proof, rejects stale callbacks and gives evidence-only cleanup a publication
hold. No v2 journal implementation has begun. Independent design acceptance is still required.

## CORE-005: self-signatures are not historical owner authority

C-4 correctly proves bounded canonical shape, self-signature and conflict. A current authenticated
member can sign a conflicting pair for the right document despite never being owner. The old
report contract would admit it into permanent evidence capacity, owner choices and proof gates.

No report consumer exists yet, so this is a confirmed future contract defect, not a demonstrated
present network-reachable exploit. The C-4 API remains narrow and documents this boundary.
The new primitive member-forgery test is a limitation demonstration, **not** the missing
report-admission/no-write security regression.

The [proposed authority contract](GATE4-AGENT-3-AUTHORITY-FOLLOWUP.md#core-005-local-historical-owner-provenance)
uses one receiver-local archived Observed-tenure witness in the matching durable MLS snapshot,
then private exact-pair admission attestations. Every authority check precedes durable capacity,
source, overflow and proof changes. Reserved-first staging retains attestations until B1 transfers
them atomically. Unresolved pairs retain eligibility; terminal recycling removes their attestations.
Agent 2's tenure seam and Agent 4's snapshot integration are dependencies, not implemented APIs.

Independent review must explicitly assess the liveness limits: newcomers/Imported history,
evicted unadmitted history, re-reported completed pairs after recycling, and ownership turnover
during unfinished replacement. No full Gate 4 liveness claim is made.

## IMP-001 and implementation re-review

`ReceiptBook::decode_mode` now rejects `previous_until_installed` whenever `latest` is absent,
even with valid same-document resolved evidence. A signed same-document predecessor regression
covers both v4/v5, with valid headless controls. The ninth mutation removes precisely this guard.

Read-only implementation review found no BLOCKER/HIGH/MEDIUM. Its LOW scope-negative masking gap
was fixed at `27e1b99`: an otherwise valid **headed** repaired book accepts a same-document
predecessor and rejects a re-signed foreign predecessor, independently in both modes. The short
re-review closed that gap. Local focused suite: **20 passed, 0 failed**. Complete check/run evidence,
including failures and pending work, is in [status](GATE4-AGENT-3-STATUS.md).

## Internal design review and requested independent verdict

The read-only design reviewer inspected the actual proposal and neighboring snapshot, tenure,
source and journal paths. It found and then re-reviewed corrections for:

- exact attestation retry ordering after archive turnover;
- cross-tenure healthy-journal screening before replacement-only identity checks;
- lost attestation on source drain and terminal recycling;
- evidence-only provenance after subsequent publications;
- explicit restore epoch/device checks and mid-replacement turnover limitations.

No BLOCKER/HIGH remained preventing submission for independent **design** review. This is not
implementation acceptance, a substitute for the user's review, or acceptance of the liveness limits.
The actual report/no-write, historical-owner positive and typed lifecycle regressions remain future
mandatory work. Request a finding re-review of IMP-001 and a separate design verdict on CORE-004/005;
CORE-001/002/003 need no repeated approval for their already accepted corrections.
