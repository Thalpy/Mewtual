# Agent 3: historical authority and journal roles (proposed revision 15)

Review base: `8d4cc53a529c86bdad76170f7825f7dbd682dfa9`; the user's verdict was REQUEST
CHANGES. This is the concrete follow-up for CORE-004/005. **These new persistence and authority
contracts are proposed, not implemented or independently accepted.** CORE-001/002/003 were
accepted subject to the conditions in the supplied review. IMP-001 is a separate codec fix.

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
such an old pair may remain unavailable. This liveness limit requires independent acceptance.

### Explicit liveness limits and tests

This deliberately narrows revision 14's arbitrary-history admission. A newcomer that knew a
former owner only through Imported/proof-carried tenure cannot admit that owner's later report.
Another positively observed retirement can evict eligibility for an **unadmitted** older pair.
Unresolved admitted pairs do not expire or lose their attestations; completed pairs can be
recycled as above. The old two-peer genesis/Welcome
version of N17 is therefore no longer an unconditional convergence claim. No transferable MLS
history protocol is claimed. These limits require the user's independent design disposition.

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
the journal does not install a competing reconciled value behind it. Source and journal planning
must agree on this classification before B1, or return an explicit hold without mutation.

For an exact/provable losing in_flight, retire its publication obligation. Before doing so, retain
its full signed receipt **and its exact bound CloseRecord** in the candidate's provenance; that
receipt need not be a member of the repair pair. B1 atomically persists those bytes with the
decision. A missing, malformed, wrong-scope or wrong-hash close is a refusal, never permission to
discard pending evidence. The API consequently also accepts the optional retiring pending close
from the owner's already-validated record. It is forbidden to clear an obligation first and save
its evidence in a later write.

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

**Mid-replacement owner turnover is an explicit unresolved liveness limit.** If A departs after
B2 but before selected-seed recovery/B6, A's old repair fails the mandatory live retry guard and
a new repair cannot bypass the existing nonterminal fence. The proposed journal cannot discard
that transaction to reset tenure. It holds durably with evidence retained; finalization is not
claimed universally reachable. A recovery/cancellation transition would require a separate
reviewed contract that preserves losing work and authority. Test this churn sequence, alongside
the ordinary same-tenure path that fetches the seed, completes B6 and cleanup without any new
publication. This proposal does not claim full Gate 4 liveness acceptance.

Required tests: winner before/equal/after old publication; two repairs without publication;
historical-only repair screening; each pending classification; missing/wrong pending close;
historical A-tenure repair with an unrelated healthy C-tenure journal;
same-published normalization; stale callbacks; publication followed by source-finalization and
the reverse order; evidence-only hold rejects second publication but permits cleanup/retry;
reserved-first seal and B1 crashes across archive turnover; interrupted B1; restart at every
transition; next two receipts; v1 compatibility;
malformed mixed group/key/start, role and selection. Assert source/journal agreement throughout.

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

Request independent adversarial review of these precise authority, lifecycle, boundedness and
liveness contracts before implementing them. Agent 2 must agree the archived Observed witness
and durable capability seam; Agent 4 owns shared snapshot/schema integration and documentation.
The report admission/no-write tests remain mandatory production-consumer tests, not helper tests.
