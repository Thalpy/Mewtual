# Agent 3 core follow-up: four implementation design corrections

Base: `918ffb9b3034c43ed33574225e04f1be2e87090d`. The accepted revision-14 design remains
the baseline. A read-only adversarial source audit found the four concrete paths below before
the typed transitions were implemented. **These proposed corrections are not yet accepted.**
The first leaf checkpoint implements only C-3/C-4/C-8 and the C-5 sequence getter, unaffected
by these questions. No new gate, journal or typed-source format has been written.

## AG3-CORE-001: reopening with quarantine is not restorable (High)

`EpochGate::admit_inbound_locked` permits bounded rejected hashes during Closing. Moving to
Fault preserves that quarantine. Design C-1 says repair leaves it untouched, but C-2 cases 1b
and 5 reopen the gate. `EpochGate::decode` accepts Open only with an empty quarantine. The
proposed transition therefore creates a source that cannot survive restart. Merely allowing
Open quarantine is insufficient: re-admitting the same envelope leaves a hash in both the
accepted set and quarantine, another restore rejection.

Proposed narrow correction: when repair reopens a Fault gate, clear only the rejected-hash
quarantine under the same gate lock. Preserve every accepted operation, domain id, byte total,
per-device total and signed log entry. These hashes are neither admitted content nor durable
intents; replay/catch-up supplies any actual missing operations through ordinary admission.
Closing repair targets keep the existing quarantine. No gate encoding change is needed.

Required regression: Studio and Registry, Closing -> quarantined inbound envelope -> Fault ->
case 1b or 5 -> snapshot/reopen. Then admit that formerly quarantined envelope through the real
typed path and reopen again. Assert accepted work and accounting are unchanged by repair.

## AG3-CORE-002: screening cannot use C-1 verbatim (High)

C-1 allows only Open/Closing targets; C-2 case 6d must preserve a different Fault. The proposed
typed wrapper unconditionally invokes C-1, so it cannot commit the screening disposition that
releases a pending unrelated repair. This recreates the deadlock AG3-DES-019 was meant to close.

Proposed correction: distinguish a validated transition from an unchanged-gate screening
commit. The screening commit checks the exact expected phase/hash, preserves them and the
unrelated fault pair byte-for-byte, and swaps only the book's repair evidence and typed provenance
under that lock. Settled remains a refusal. It must not become a general Fault-to-Fault mutation
escape hatch; the private planner supplies the validated candidate.

Required regression: N33/N38 through both typed APIs. Apply unrelated repair A while Fault B
stands, snapshot/reopen, exact retry still returns Screened, B remains unchanged and repairable.
Do not assert that `receipt_head()` succeeds for this case; it must continue reporting Fault.

## AG3-CORE-003: a reconciled loser must itself be repairable (High)

C-7 examines `in_flight` and `high_water` only. Publish R1, repair `{R1,R2}` to R2, then before
R2 is published repair `{R2,R3}` to R3. The preferred decision is now `reconciled = R2`, but the
stated implementation returns `Ok(false)` and continues selecting the repudiated R2.

Proposed correction: include `reconciled` among decisions eligible for an exact named-loser
replacement. Verify the live repair and its complete evidence against that retained loser and
the supplied winner before mutating. A second reconciliation replaces the canonical decision
atomically, never invents publication, and retains the existing source/owner-record sequence
checks as prerequisites. This does not add an unbounded journal repair history.

Required regression: two consecutive repairs without intermediate publication, restart, publish
R3, restart, and prepare the next receipt. Test wrong evidence and unchanged state on refusal.

## AG3-CORE-004: canonical selection and publication history can diverge (High)

N7(b) retains published R(e-1), removes pending losing L(e), and reconciles to selected S(e).
The conflict may change inherited baseline. The existing journal decoder requires every retained
receipt's full `TenureSelection` (including inheritance) to equal the journal's single selection.
No one selection can describe both R and S in this valid repaired shape, so v2 as currently
specified rejects its own admitted state.

Proposed correction to the v2 contract: the journal selection follows the canonical head and
any adjacent pending receipt. Historical `high_water` retained behind `reconciled` must match
document, owner key and tenure identity/start, but may have a different inheritance selection.
Its closed epoch must remain strictly below the reconciled decision, preserving C-7's ordering
rule. `published()` continues to report only completed publication. On promotion/supersession,
ordinary canonical adjacency resumes; v1 validation and encoding stay unchanged.

The mutation must preflight the complete resulting v2 shape before changing the journal. Shapes
outside those invariants need an explicit refusal, not silently discarded publication evidence.
Review must check interactions with an unrelated newer pending decision before this is implemented.

Required regression: N7/N28 for same and different inherited baselines, preserving the published
receipt, round-tripping v2, completing the actual canonical publication, and issuing the next two
receipts. Include malformed mixed document/owner/tenure and nonadjacent pending records.

## Review request

Review these four corrections against revision 14 section 5.1 and the actual epoch, Studio and
Registry admission/restart/journal code. Return an explicit bounded design verdict, especially
on clearing rejected hashes and the precise v2 canonical-versus-publication invariant. This is
not runtime repair acceptance, native registration, or full Gate 4 acceptance. C-1/C-2/C-5/C-6/C-7
implementation remains pending this correction; independent C-3/C-4/C-8 verification continues.
