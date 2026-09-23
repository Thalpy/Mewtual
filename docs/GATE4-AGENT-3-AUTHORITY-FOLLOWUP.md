# Agent 3: historical authority and journal roles (revision 16 follow-up)

Revision 16 independently **PASS at `2b741e7`**: CORE-006/007 and TEST-014's design plan
are closed. The C-7 journal core is now implemented; source/store/runtime integration and
CORE-005's Agent 2 dependency and production-consumer tests remain required. The accepted
contract below is unchanged; [status](GATE4-AGENT-3-STATUS.md) tracks code and verification.

## CORE-005: local historical-owner provenance

The low-level C-4 checker proves canonical shape, self-signatures and conflict. It does not
establish membership or owner history. An authenticated current member can manufacture a pair
for the correct document with its own key, so that checker alone must never allocate durable
evidence, trigger proof suppression or populate owner choices.

Existing snapshots cannot be grandfathered into the missing proof. `catcoms-sync/src/owner_tenure.rs`
retains only current tenure. `receipt_head/detached.rs:169-204` can accept a fresh owner proof when
the start is unknown and take its carried start; saved receipts do not preserve Observed versus
Imported origin. Studio adoption and vault restore therefore cannot establish the required
historical Observed origin merely by finding an old, correctly signed receipt. Owner journals,
archives, repairs, previews, hints and Welcome data have the same restriction.

### Chosen mechanism and Agent 2 dependency

Add **one optional receiver-local archived Observed-tenure witness per MLS group**, in the same
authenticated sync snapshot as MLS and current tenure. This is a proposed extension to Agent 2's
leaf-aware tenure seam, not a claim that its current/verification accessor already supplies it.
Agent 3 will not edit or substitute for that owned seam before agreement and review.

Witness fields are the full owner signing key (32 bytes), Observed start (`u64`), derived tenure
ID (32 bytes), and retirement MLS epoch (`u64`). Version/tag/framing plus these fields must fit
`MAX_HISTORICAL_OWNER_WITNESS_BYTES = 128`. Group identity comes from the enclosing authenticated
sync snapshot and is checked against the application store's full group scope. No receipt list,
history search, remote proof fetch or transferable witness is introduced.

Mint the private witness only from an actual contiguous MLS owner transition:

1. Capture the departing full key before MLS removes it. Its pre-transition leaf/position and
   owner/start must exactly match the saved **Observed** state.
2. Confirm the actual post-transition MLS state establishes a different owner tenure. Repeated
   keys with different starts are distinct; continuous same-owner updates leave the archive alone.
3. Imported/Unknown departures, gaps and unestablished positions cannot mint or replace a witness.
   A previously valid archive survives those cases. A positively observed later departure replaces
   the one archived entry.
4. Follow the existing post-call transition seam even when a helper reports an error after merging.
   An error before any merge changes nothing. This extends the existing `owner_tenure.rs` logic,
   rather than predicting state from the requested operation.
5. Save MLS, current tenure and the archive atomically in the same sync snapshot. Report callbacks
   cannot manufacture this lifecycle write. The application may consume the witness only through
   a private capability covered by the matching durable server snapshot and current custody stamp.
   Failed or unflushed snapshot writes supply no admission capability.

Only the private authenticated-vault decoder restores witnesses. Check canonical framing, key
length, full group binding, derived ID and `start < retired_at <= saved MLS epoch`. Corruption
fails closed, not as an empty archive. Legacy snapshots have **no archived witness**; no old
receipt or caller-provided flag upgrades them. Current-tenure migration still follows Agent 2's
Observed/Imported contract. Snapshot rollback protection remains the existing vault limitation.

### Report admission and exact durable attestation

Studio and Registry use the same order, before **any** owner-record reservation, write, slot
change, source seal, overflow marker or proof-suppression mutation:

1. Authenticate/freshness-check the current-member requester and apply the existing bounded
   transient request-rate rail. Such transient charging is allowed; durable evidence charging is not.
2. Check complete numeric-server, group, document type/key and applicable channel scope.
3. Bound/decode both full receipts and run C-4. This has established no owner authority yet.
4. Establish the provider's current owner authority from a durable Observed-owner capability.
5. Perform a bounded read-only load of the addressed owner record, without reservation or
   mutation. For an exact previously admitted pair, validate its scoped receiver-local
   attestation; that is sufficient pair authority even after archive turnover. Otherwise establish
   the pair signer's exact `(group, full owner key, start, derived tenure ID)` using either the
   current Observed tuple or the one archived witness. Both receipts must match the full tuple.
   Current pairs additionally pass ordinary live receipt verification. Historical pairs do not
   require their signer still to be a member. Neither branch bypasses steps 1-4.
6. Only after all of these checks may the existing capacity/frozen-transaction/source rules run.

Unprovable history returns a generic bounded refusal on the wire and local
`HistoricalAuthorityUnavailable` diagnostics. Do not expose a historical-owner inventory or
which archived tuple is present. There is no operator override that converts a self-signature
into historical authority.

Every admitted pair gets a **private receiver-local admission attestation** in the same atomic
owner-record write as the pair. It binds the existing authenticated record scope, observer device,
complete owner-tenure tuple, canonical sorted pair hashes, durable admission MLS epoch and origin
(current Observed or archived Observed, including retirement epoch when archived). It is a sealed
local assertion that this receiver ran the authority check, not a transferable signed membership
proof. All fields are private; only the validated admission capability or authenticated-vault
decoder can construct it. A generic record writer must not accept an unvalidated new attestation.
Network report encoding remains two receipts; it has no attestation/witness import field.

Each attestation is capped at **256 bytes**; full document/group scope is already authenticated
by the enclosing owner record and checked before access. Exact owner key/start/ID, observer,
pair hashes, origin and epoch framing fit that cap. Restore repeats canonical receipt/signature,
tuple, pair, scope and attestation checks. Current origin requires `start <= admission_epoch`;
archived origin requires `start < retired_at <= admission_epoch`. Admission epoch must be covered
by the restored matching server snapshot; observer must equal the expected local device/custody
identity. No implicit cross-device identity migration is supported. A structural record decode
with only a LogicalDocument yields inert data: the private contextual decoder must receive these
authenticated scope/device/snapshot inputs before yielding an authority-bearing capability.
It does not require the single archived lookup entry still to exist: unresolved admitted evidence
must survive later archive turnover.

Every retry reauthenticates the requester, target and complete pair. An exact already-admitted
pair with a **still-retained** attestation may use it for acknowledgment/reflush after archive turnover.
That attestation is **not** a witness for a new pair sharing one receipt or the same signer.
No raw legacy fault section is silently stamped as admitted. It must acquire a valid current/
archived witness through this admission path, or remain explicitly unavailable for report issuance.
The new fault section is not implemented yet; implementation must use an unambiguous version/tag.

The attestation follows its exact pair through external/reserved/source-bound moves. Inserting
a source-bound pair into an owner record also needs this authority check: finding it in a legacy
source is not an exception. Live overflow accounting similarly runs only after authority succeeds.
An unresolved attested pair already admitted under a former Observed owner remains actionable after turnover;
the **new repair** still requires independently established live current-owner authority.

Every new live report first durably stages its full pair **and attestation in the reserved slot**,
before any source seal. If that slot is occupied, use the existing authority-checked durable
overflow/refusal path; it is not an admitted full pair until both pair and attestation are stored.
After a successful seal/drain, retain the reserved pair and attestation even when the source fault
duplicates its receipts. Only the atomic B1 owner-record write may move them into the inline
source-bound pair. Duplicate cleanup must never clear that sole attestation first. A crash before
the source write leaves a staged report; after it leaves two copies of the pair and one durable
attestation, so archive turnover cannot remove eligibility. B1 failure retains the staged state.
This revises revision 14's post-drain reserved-slot clearing rule, without adding a source codec.

Terminal recycling removes the resolved pair and its attestation together under the existing
source-terminal/recovery barriers. This is not a permanent history archive. After recycling,
an exact re-report must obtain current/archived provenance again; if that history has turned
over, it refuses as unavailable. The source's raw resolved-repair bytes cannot replace the
missing attestation. This explicitly narrows revision 14's unconditional terminal-pair no-op
claim (N31c). It avoids an unbounded completed-pair cache; a later reporter still faulted on
such an old pair may remain unavailable. The independent review of `a751369` accepted this limit.

### Explicit liveness limits and tests

This accepted bounded-design tradeoff narrows revision 14's arbitrary-history admission. A newcomer that knew a
former owner only through Imported/proof-carried tenure cannot admit that owner's later report.
Another positively observed retirement can evict eligibility for an **unadmitted** older pair.
Unresolved admitted pairs do not expire or lose their attestations; completed pairs can be
recycled as above. The old two-peer genesis/Welcome
version of N17 is therefore no longer an unconditional convergence claim. No transferable MLS
history protocol is claimed. The review of `a751369` explicitly accepted these limits; they
must remain visible in the eventual Gate 4 product/integration verdict.

Required regressions (both Studio and Registry):

- N49 / TEST-013(a): authenticate a current non-owner and supply two canonical, self-signed
  conflicting receipts for the correct target. C-4 passes; admission refuses; exact durable bytes,
  reservations, all evidence slots, overflow/source/proof state and writer counters remain unchanged.
- N50: observer sees real A -> B activation, B signs the pair, observer sees B retire to C;
  persist/reopen, then C admits the removed B's report. No injected historical witness.
- Repeat N50 with B known only as Imported, with a snapshot flush failure, and with legacy
  witness absence: refuse before capacity/write. Same key/different start, different key/same
  start, cross-group, cross-device/store/document/channel and tampered witnesses also refuse.
- Retire another Observed owner; exact admitted-pair retry still succeeds, but a fresh pair from
  the evicted tenure (including one sharing its winner) refuses. Confirm fixed bounds and no
  remote callback can populate or rotate the archive.
- Complete and recycle that repair, turn over the archive, then re-report the exact pair:
  refuse before durable mutation. Repeat around a second repair and verify no old attestation
  is silently reconstituted from resolved receipt bytes.

The new primitive limitation test is not N49: there is still no report-admission implementation.

## CORE-004: publication facts, obligations and canonical decisions

Separate journal tenure identity `(document, full owner key, start, derived tenure ID)` from
receipt-specific inherited selection. Version 1 retains its exact existing decode/encode contract;
the proposed version 2 uses the identity without inherited selection. All retained receipts must
match that identity; their roles determine whether their inherited selections must match.

- `high_water` / `published()` is the last completed publication **in the journal's tenure**.
  It is a historical fact, not automatically an authority to override repaired canonical state.
- `in_flight` is the sole outstanding publication obligation.
- `reconciled` is a repair-selected canonical decision, possibly never published.
- `canonical_head()` is `reconciled.or(high_water)`, the adjacency base for a new obligation.
  The **effective publication choice** is `in_flight.or(canonical_head())`. Head selection must
  preserve that precedence; storing a reconciliation cannot quietly outrank unrelated pending work.

`resolve_repair` must receive both full named receipts, not just a selected receipt. It verifies
the live current repair before any no-op shortcut, runs C-4/check_evidence, and checks document
scope. Classify the effective decision next: an unrelated historical pair is a screening no-op,
including when a current C-tenure journal encounters an attested old A-tenure pair. Require the
pair and journal's complete owner-tenure identity to match only for an actual replacement.
Only an effective decision that is the exact loser or a **provable differing-baseline**
descendant may be replaced. Same-baseline unknown ancestry is not proof. A repair naming only
historical high_water while a different canonical/pending choice stands is screening-only and
leaves the journal unchanged. An unrelated higher same-baseline in_flight is likewise preserved;
the journal does not install a competing reconciled value behind it. Source and journal effects
are planned independently and checked for **compatibility**, not classification equality, before
B1 as specified below (CORE-006).

For an exact/provable losing in_flight, retire its publication obligation. Before doing so, retain
its full signed receipt **and its exact bound CloseRecord** in the candidate's provenance; that
receipt need not be a member of the repair pair. B1 atomically persists those bytes with the
decision. A missing, malformed, wrong-scope or wrong-hash close is a refusal, never permission to
discard pending evidence. The API consequently also accepts the optional retiring pending close
from the owner's already-validated record. It is forbidden to clear an obligation first and save
its evidence in a later write.

### CORE-006: compatible independent source and journal effects

The joint planner produces two effects against one exact captured source/journal/custody state:

```text
SourceRepairEffect  = Transitioned | Retargeted | Screened   // existing C-2 disposition
JournalRepairEffect = Replace | RetirePendingAndReplace | Normalize | NoChange
```

These are derived descriptions of complete private validated candidates, not caller-supplied
authority. The source effect includes its exact planned book, gate, adoption state and required
continuation. `Replace` installs an eligible canonical reconciliation; `RetirePendingAndReplace`
also retires the exact/provable losing publication obligation with its bound close evidence.
`Normalize` selects the already-published exact winner without a phantom unpublished head; any
pending retirement still requires the same eligibility and full evidence. It cannot normalize
away unrelated pending work. `NoChange` preserves every journal role and existing close binding.
It does not skip current repair authority, full admitted evidence, sequence or transaction fences.

Define `covered_R(receipt)` using the exact-loser/provable differing-baseline rule already used
by `ReceiptBook::is_repaired_loser`, applied to this repair's complete verified pair. A higher
epoch or absence of conflict alone proves no ancestry. Joint compatibility requires:

1. Both candidates pass their own existing scope, authority, source-shape, sequence, journal-role,
   nonterminal and recovery/evidence checks. Neither effect overrides the other's refusal.
2. The resulting journal's effective publication choice, `in_flight.or(reconciled).or(high_water)`,
   is absent or is not covered by R. An unrelated or positively preserved choice is legal; it
   need not equal R's selected receipt or the source's resulting head. Historical high_water
   behind that choice and retired pending receipts remain non-authorizing evidence.
3. No covered source receipt remains eligible to prove, settle or install. Inspect the resulting
   latest, opening, retained adoption target, gate receipt binding and planned install together.
   This is an authority postcondition, not a ban on retaining evidence: an old opening/branch
   awaiting repair recovery can remain only behind the existing adoption/continuation guards;
   an unchanged unrelated Fault remains blocked; cross-tenure screening/unblocking cannot grant
   old-tenure installation. Every retained covered anchor must have an explicit enforcing guard,
   not merely an effect label. Full losing work and unrelated fault evidence remain preserved.
4. Existing custody, exact-state and publication guards cover B1 through application. B1 saves
   only the owner candidate; it does not pretend the projected source effect has crossed B2.
   The pre-B2 target fence prevents the old source from bypassing the pending repair. B2 rechecks
   the actual current state/authority and commits the compatible source candidate; a stale or
   incompatible candidate holds rather than silently reclassifying a promised replacement.

Thus the following asymmetric outcomes are permitted when those postconditions pass:

| Source effect | Journal effect | Concrete reason |
|---|---|---|
| Transitioned (case 1a) | NoChange | Installed opening S wins `{S,L}`; positively justified sealing H remains source head and journal high_water **or** in_flight |
| Screened (case 6b) | Replace / RetirePendingAndReplace | Source has no covered usable anchor; its separate journal has an effective losing choice |
| Screened (case 6d) | A valid journal effect, including NoChange | Unrelated Fault(B) and its evidence remain unchanged and cannot serve/install through that fault |

Other combinations are checked by the same postconditions; there is no equality requirement on
the effect tags, nor an unconditional equality requirement on their resulting receipt hashes.
If a postcondition fails, return a precise compatibility hold before B1 with no mutation. A
journal `NoChange` still allows the repair record, admission binding and source transaction to
be persisted and applied. It does not mean the whole repair is a no-op.

The existing three-way proof check (held source receipt, head selector's chosen receipt and
effective owner choice) is unchanged. The selector's chosen head is not necessarily the repair
winner: case 1a can keep H in all three positions. Compatible differing heads may leave an ordinary
head proof unavailable; this contract does not force convergence by weakening that check.
Source guards are concrete: ordinary
settlement requires Closing and non-adopting; adoption requires Closing, adopting, exact latest
target and current-owner validation; successor commit rechecks the source version. Fault head/seed
service refuses. C-3 prevents covered retained anchors from blocking later admission. During a
retarget, the losing opening/log is recovery evidence only under the target claim and exact S
install plan. Neither a disposition tag nor `install_pending` alone replaces that claim, especially
before B2; restore and later source/callback writes must retain/recheck the derived durable fence.

### Bounded provenance and repeated repairs

One optional journal provenance slot contains the latest live-validated repair, its complete
selected/losing pair, and at most one retired pending receipt/close. It binds exactly to
`reconciled`, and is written atomically with it. Restore verifies the historical repair signature,
complete canonical signed pair, repair bindings, selected role, journal identity and any retired
pending receipt/close binding. Sealed local provenance plus the live mutation guard establishes
the authority of the earlier commit; a self-signature alone does not grant new live authority.

After R1 -> R2, an exact/provable repair of effective R2 may replace it with R3 before R2 is
published. Replacement validates R2 against the **current** journal, then stores only the new
R3 proof. Historical high_water need not belong to that latest pair. This is a bounded proof of
the current choice, **not an audit chain**. The old provenance may be retired only after its
source-side transaction is durably terminal and all required losing recovery/evidence barriers
have completed. A terminal source with an unpublished reconciliation does not require retaining
an unbounded chain; a nonterminal source still fences the next B1 as before. Do not overwrite
unresolved external pair attestations under this rule; U-10 still applies to them.

### Publication lifecycle and strict restore

1. `high_water = R(4,A), reconciled = S(2,B)` is legal. There is no numeric epoch monotonicity
   comparison across a repair-selected baseline. The provenance must validate S.
2. `mark_published(S.hash())` records actual completion by making S high_water and clearing its
   reconciled state. That may lower the epoch numerically. A completed in_flight may supersede
   reconciled only through its validated same-lineage adjacency; an epoch comparison alone is
   insufficient. Publication callbacks use the exact still-authorized decision hash and the
   ordinary custody/durability fences.
3. Exact high_water retries are inert while a *different* reconciliation exists. When a new repair
   selects the exact already-published receipt, normalize immediately: no phantom unpublished
   reconciliation is retained. Source transaction evidence remains in its owner record until
   its own durable terminal barrier; journal normalization does not erase it.
4. A callback for a retired in_flight hash matches no publication obligation and is rejected.
   Neither the retired-evidence slot nor a historical repair proof is a publishable source.
5. Keep provenance and any retired pending evidence until both publication/normalization and
   required source recovery are durably complete, unless safely superseded by the repeated-repair
   rule above. A journal publication callback alone cannot guess source completion and discard it.
   Publication/normalization may leave a strictly tagged evidence-only provenance slot with no
   reconciled value until the store finalizes the source barrier; it grants no head or publication
   authority. While this slot remains, hold new receipt preparation and further publication
   completions (exact already-published retries remain inert). This is a bounded hold,
   not permission to infer an arbitrary descendant from a shared inherited baseline.
6. Decode derives identity from the effective retained set. A canonical provenance slot requires
   reconciled equal to its selected receipt. An evidence-only slot requires that exact selection
   to be high_water or high_water to be its **immediate** successor under the ordinary verified
   same-tenure, same-selection adjacency rule. The publication guard records that transition;
   the hold in step 5 prevents another high_water advancement from erasing this bounded witness.
   No mixed role is tolerated.
   in_flight must be adjacent to canonical_head and share its selection unless the ordinary
   independently verified tenure-change rule applies. Historical high_water need only match the
   journal's owner-tenure identity, not the repaired inheritance or epoch order.

Source finalization and idempotent provenance cleanup are exempt from the evidence-only hold.
They run from already durable source/repair evidence and never require a new receipt or proof
publication. A failed cleanup remains retryable. Any outstanding live mutation retains its normal
authority/custody checks; removing already-terminal evidence is not a new publication. Tenure
change must first complete required source recovery and cleanup before replacing the journal,
so ordinary tenure reset cannot silently erase retired pending evidence.

**CORE-007: owner turnover can strand a transaction from B1 onward.** An owner change at any
time **after B1 and before the repair transaction is durably terminal/recycled** may leave an
indefinite durable hold. If A's pending repair was saved at B1 and ownership changes to B before
B2, A's repair fails the mandatory live authority guard and B cannot replace it through the
nonterminal fence. B1 may already have reconciled the journal or retired a pending obligation;
the source is still pre-application. Owner rotation fences do not preserve A's MLS authority.

After B2, the same authority/nonterminal conflict may additionally strand committed source
transition, selected-seed, recovery or B6 work. In either interval, retain all source, journal,
retired-close, pair/attestation and transaction evidence. Restart, ordinary tenure reset, proof
service, publication completion and a new-owner retry cannot silently clear or bypass the hold.
An old repair that lost live authority is not servable as a current repair. Already-terminal
idempotent cleanup remains governed by its existing predicates; no new cancellation is inferred.

The durable ownership predicate is the owner record's pending repair, **even when the journal
effect was NoChange and no new journal provenance exists**. Every store path into ordinary
journal prepare, newer-tenure reset and publication completion must check that record claim;
the existing core journal's newer-tenure allowance cannot override it. Under unchanged current
authority, the same transaction's valid continuation, B2/B3 serving and selected publication
remain allowed by their existing rules. Exact completed publication retries may be inert. The
fence blocks competing work and stale authority, not its own otherwise valid progress.

This revision adopts the broader fail-closed limitation rather than adding cancellation. A
recovery/cancellation transition would require a separate reviewed contract preserving losing
work and authority. Finalization is not claimed universally reachable. The ordinary same-tenure
path still fetches the seed, completes B6 and cleanup without a new publication. The expanded
B1 boundary needs independent disposition; this is not full Gate 4 liveness acceptance.

Required tests: winner before/equal/after old publication; two repairs without publication;
historical-only repair screening; each pending classification; missing/wrong pending close;
historical A-tenure repair with an unrelated healthy C-tenure journal;
same-published normalization; stale callbacks; publication followed by source-finalization and
the reverse order; evidence-only hold rejects second publication but permits cleanup/retry;
reserved-first seal and B1 crashes across archive turnover; interrupted B1; restart at every
transition; next two receipts; v1 compatibility;
malformed mixed group/key/start, role and selection. Assert the joint compatibility postconditions
throughout, not equality of independent effect tags.

**TEST-014, required integrated regressions in both Studio and Registry (planned, not executed):**

- Extend N3b with `{S,L} -> S`, installed opening S, fault `{S,L}` and positively justified
  later sealing H. First journal H is high_water; then repeat with H in_flight. B1 succeeds with
  `(Transitioned, NoChange)` and leaves journal bytes, H's role and close binding unchanged;
  B2 leaves Fault with source H preserved. Crash/reopen around both barriers and prove no phantom
  S publication or retirement of H. A mutation requiring equal effects must fail this positive.
- Exercise the other asymmetry: an unrelated healthy source screens while the journal's effective
  loser is replaced, with and without pending retirement. Covered usable anchors or a missing
  retired close refuse before B1; unknown same-baseline H is not invented losing ancestry.
- After a real B1 (reconciliation-only, retired-pending and journal NoChange variants), change the observed
  MLS owner **before B2**, then crash/reopen. The old repair cannot apply, the new owner cannot
  overwrite it, and the exact source/owner evidence remains durable. Proof, publication, ordinary
  rotation and retries must not bypass the hold; before B2 the source remains pre-application.
- Keep the distinct B2-to-B6 owner-churn case. Verify the already-committed source disposition
  and losing recovery/continuation remain retained and truthfully held. Do not accept a test that
  faults for an unrelated scope error or claims an unimplemented cancellation recovered progress.

## Proposed bounds and implementation gate

The journal's maximum is three role receipts, two full evidence receipts, one repair, one retired
pending receipt, its CloseRecord and bounded identity/framing. Use
`MAX_OWNER_RECEIPT_JOURNAL_BYTES = 7 * MAX_RECEIPT_BYTES + MAX_CLOSE_RECORD_BYTES + 1024` (12 KiB).
The owner record has at most four attested pairs: two external, one reserved and one inline
source-bound. Its proposed bound is
`MAX_OWNER_RECEIPT_JOURNAL_BYTES + MAX_CLOSE_RECORD_BYTES + 9 * MAX_RECEIPT_BYTES + 1280
 + 4 * MAX_FAULT_ADMISSION_ATTESTATION_BYTES` (27.25 KiB with 256-byte attestations).
Update the sealed/pre-read/accounting caps together; maximal encodings and one-over-cap failures
must be exercised before any format implementation is accepted. These are proposed constants,
not changes to today's smaller codecs or storage limits.

Request independent adversarial re-review of CORE-006/007 and TEST-014 before implementing
CORE-004/C-7. Preserve the accepted CORE-005 boundary. Agent 2 must agree the archived Observed witness
and durable capability seam; Agent 4 owns shared snapshot/schema integration and documentation.
The report admission/no-write tests remain mandatory production-consumer tests, not helper tests.
