# Operational cohort: concrete implementation split

Interface proposal at pushed base `6f8c7c6f765b992d995cf0d7cd40361e6867e06a`.
Shared source-only worktree: `target/chat-v2-operational-implementation-worktree`.
No runtime, module declaration, test execution or native activation is supplied by this table.
Root owns all builds and final integration. Implementations remain coupled to their real consumers.

The reviewed HOST owner/issuer contract and the physical author's current FORMAT are the
design inputs. The older HOST text in this base is not a substitute for those reviewed
amendments. This table fixes interoperation; it does not silently approve proposed fee classes.

## File ownership

| Owner | Exclusive source family | Real output consumed by the other owners |
| --- | --- | --- |
| Factory | `current/operational/{owner,negative}.rs`; `chat_admission/v2_current/text.rs`; `owner_evidence/canonical/current_vacancy/text.rs`, `v2_registration/text.rs`, `text_lane.rs`; `critical_namespaces/current_layout/text.rs`; query/close custody and new cold operational task/scope/writer adapters | Closed actual owner loans, same-row Current admission, installed-generation proof, running/commit/query/retirement custody and new cold ownership |
| Physical | `persistence/v2_text/` codecs, head/promotion, selected queries/COW consumers, transaction work, WB/WD/WC and cold selection | Authenticated selected physical closure, frozen work, cached saved outcomes, exact retired-path plan and new cold physical receipt |
| Root | Core verifier/issuer/candidate/index DATA edits; Sync producer refinements; `current/operational.rs` orchestration and narrow old-family call-throughs | Actual node producer/sign call, owned verifier/math/candidate, top-level phase dispatch and final result |

Parent module/State/constructor edits are composed by Root only when all actual consumers
exist. No owner edits another's files without a narrow agreed hunk. No public callback exposes
a node, SDK, Runtime, mutable registration map, raw key, caller-selected Current role or permit.

## Shared names and ownership

All App names below are Store-private. Scope/namespace/Root guards are borrowed and cannot
escape HRTB callbacks. Receipts are opaque, non-renewable historical objects; only a fresh
concrete Current guard proves present selection. Non-Clone owners return unchanged on an
owning error. Mutable workers cache their actual outcome before fallible final proofs.

| Interface owner | Exact contract for the paired caller | Allocation, error and authority boundary |
| --- | --- | --- |
| Factory | `V2TextOwnedCurrent<T,R>::from_warm(actual)` / `from_cold(actual)` / `from_repaired(actual)`; private closed core stores the WHOLE original wrapper. `borrow_current() -> Result<V2TextOwnerRef<'_>>` | No extraction from Drop types. Repaired A recovery is the actual Repaired family. Only these constructors and the actual new cold-text output construct an owner. Root's host keeps this owner through every error. |
| Factory | `V2TextOwnerRef::{source, task, writer_observation, require_original_negative}`; family-specific cached Scope borrower remains private | Source is a node-backed borrowed `GenesisSourceDataRef`; never stored in inert debt or across mutable node RNG use. Negatives perform no FS/account/SDK/Root/Runtime acquisition. No mandatory genesis-seed validator after operational admission. |
| Factory | `ServerStore::with_v2_text_current(owner, callback)` where callback is `for<'w> FnOnce(&V2TextCurrentAdmission<'w>) -> Result<T,AppError>` | Same loaded clean row2, nonce, actual funded owner, exact Current binding and installed writer observation. No new row/revision/nonce/sender or per-message scope write. Dirty inventory maintenance precedes this constructor; it is not permission. |
| Factory | `store.lock_v2_text_owner_namespace(&admission)`; `.require_installed_writer(&V2WriterObservation)`; `.lock_archive_root(&V2TextReadResources)` -> `V2TextCurrentGroupGuard` | AD -> pins -> canonical -> Root. Kernel map is family-private and readonly; pointer/count/cancellation checks never upgrade Weak. Guard getters: `scope_admission`, `archive_root`, `resources`, `validate_source_only`. Physical selected-source proof occurs under this guard. |
| Physical | `SelectedV2TextHead::validate_under_current_guard(&V2TextCurrentGroupGuard)`; `PreparedV2TextParentCapture::capture(&guard, request) -> CapturedV2TextParent` | Capture is exact O frame/generation, H/public parent, accepted/request/publication roots and closed pending transaction. Resources are prepaid outside AD. Missing/dirty/unknown is never Empty/miss. Full proof order ends source -> ID3 -> held/named objects -> cached negatives. |
| Factory | `PreparedV2TextLane::prepare(store, owner, &V2TextSourceNegativeObserver)` outside writers; `namespace.claim_text_lane_into(&mut prepared, &mut outside_slot) -> Result<()>` | Separately paid fixed 16 active slots per ServerStore, one lane per mount/group/full signer, distinct from genesis claims. Claim transfers an already allocated strong NEGATIVE entry and caches its unique caller in the outside phase before postchecks. Some retry accepts only the prepared original observation. No physical O/IO/sign authority follows from registration. No new lane when O advanced but original work still drains. |
| Physical | `PreparedV2TextTransaction::prepare(captured, detached_create, paid_resources)` -> owning result; `.admit_a(&guard, &lane) -> AdmittedV2TextWork` | Cell owns original complete A candidates, immutable source binding and real shared producer/resource payments BEFORE effects. Root retains actual paid verifier in this same pending custody before signing. No key/node/SDK/Store/session/sender in the cell. Preparation errors return all unique inputs unchanged. This admission permits immutable staging only, never selector CAS. |
| Factory | `V2TextRunningWork::start(&mut lane, AdmittedV2TextWork, actual VaultPhysicalWorkLease)`; `.run()` / `.join() -> ReadyV2TextSelection` | Stage runner holds exact original work and real lease until Files/results/syscall cleanup drain. The lease drops LAST. Ready is cached exact child/WAL/stage completion, NOT selected O or a Saved A/I/S/B result. No physical lease in dormant debt. Drop requests cancellation; explicit close joins actual completion and does not block in Drop. |
| Physical | `ReadyV2TextSelection` has private exact A/I/S/B candidate kind, original parent and held staged objects. `V2TextCommitOutcome::{A(Arc<SavedV2RetainedAction>), I(Arc<SavedV2UnsignedAppendIntent>), S(Arc<SavedV2RawSignature>), B(Arc<SavedV2AcceptedCreate>)}` | Work/candidate kind is fixed by its private constructor, not a caller-supplied enum label. No ordinary stage/exact-repair runner can select O. Saved outcomes require the distinct completed commit work, full barriers and final proof. Receipts do not retain a predecessor Arc chain. |
| Factory | `admit_v2_text_commit(&guard, &lane, &ReadyV2TextSelection) -> AdmittedV2TextCommitWork`; `V2TextRunningCommit::start(&mut lane, AdmittedV2TextCommitWork, actual VaultPhysicalWorkLease)`; `.run()` / `.join() -> V2TextCommitOutcome` | Reacquire actual Current/kernel/selected-parent/close proof AFTER immutable staging and before EVERY O mutation, including S. The original commit admission is consumed by this distinct finite running selector. Cancellation before admission refuses; after it, the SAME effect finishes or stays uncertain. The runner caches its actual outcome before lost returns; close joins it. An OS lease or historical SavedI cannot stand in. |
| Factory + Root | `guard.admit_unsigned_signing<'g>(&'g self, &'g lane, &'g SavedV2UnsignedAppendIntent, &PreparedV2AppendData) -> SavedUnsignedV2TextSigningGuard<'g>`; `owner.sign_admitted(&signing_guard, &mut PreparedV2AppendData)` | The Prepared borrow explicitly does NOT have lifetime 'g. Guard borrows Current/Root/lane/SavedI and stores only its already-paid fixed original-plan observation/identity; the temporary Prepared borrow ends before the mutable raw cut. Actual selected O_I and exact I/preimage plus current/close proof are necessary. The private owner call independently compares the SAME immutable prepared plan, writes only its raw slot and reaches only its original node's narrow Sync signer. No parsed I/arbitrary-byte callback/lifetime laundering. Heavy Header/model work follows guard release. |
| Physical | `PreparedV2TextWork::into_inert_debt(self) -> FrozenV2TextDebt`; `FrozenV2TextDebt::prepare_exact_repair(...)` | Negative owning extraction moves only existing immutable frames/raw signature/optional Header/candidate state and payments. No validation callback, key, source export, Clock/RNG, math reconstruction, new seal or selecting CAS. Missing data remains missing. Positive selection needs a new actual commit admission. |
| Factory | `PreparedV2TextQuery::prepare(store, resources)`; `.open(&guard, &SelectedV2TextHead) -> V2TextQueryLease`; physical bounded query borrows this lease | Lease names exact private O/root generation. Finite lookup/locator IO drains before transport/ACK. New queries cannot open retired roots. Projection tokens are not lookup-root leases. Lease metadata/payment remains through readers, without retaining Host/Store/SDK. |
| Root + Physical + Factory | Root's private actual `VerifierAdvancedPendingFinal` path yields `V2TextRetirementBasis` only after same-verifier advance and final selected-B proof. Core COW output supplies `V2TextRetirementPlan`. Factory `lane.close_old_private_roots(&guard, &basis, &plan) -> V2TextPrivateRootsDrain`; OUTSIDE all writers `.join() -> V2TextPrivateRootsDrained` | Selected B plus exact advanced basis/plan is necessary before closing old roots; a COW DATA plan alone cannot close a currently selected root. Close stops new readers of exactly the replaced roots, then joins actual reader/work Files and cleanup. Counts release only after that physical drain. Parsed IDs, Weak expiry, caller counters or elapsed time cannot construct Drained. |
| Factory + Physical | `guard.admit_wd(&lane, &V2TextRetirementBasis, &V2TextRetirementPlan, &V2TextPrivateRootsDrained, &PreparedV2TextRetirement) -> AdmittedV2TextRetirementWork`; distinct running retirement consumes this plus the actual physical lease -> `SavedV2TextDeletion` | This is a concrete NEW Current-backed WD selection admission, with exact selected B, advanced verifier, precomputed WB/WD/WC chain and actual old-reader/work drain. Only its finite running work can select durable WD and unlink its exact authenticated closed list. Durable WD permits listed NotFound/idempotent retry; it does not supply fresh Current/sign/publication. |
| Physical + Factory | `SavedV2TextDeletion` plus actual page-parent/File/delete-work drain yields `AdmittedV2TextRetirementCompletion`; its finite leased runner selects exact precomputed WC -> `SavedV2TextRetirement` | WC requires actual durable WD, completed listed deletion, page-parent barrier and cleanup drain. Precomputed WC bytes, parsed WD, a raw plan or generic Drained cannot issue this completion. Every work step caches outcome before fallible final proof/return. |
| Factory | `lane.retire(&SavedV2TextRetirement, &V2TextWorkDrained)` or `lane.suspend(&DurableV2TextContinuation, &V2TextWorkDrained)` -> retired owner | Exact lane/source/intent identity; consume only after actual WC/work drain, or a separately validated quiescent durable suspension. Return retired metadata for drop OUTSIDE all writers. No Drop/Weak expiry/owner death/B visibility/network ACK release. Persistent pending transaction survives suspension. |

`V2TextReadResources`, transaction resources, query resources and their actual fees must be
acquired BEFORE allocation or waits. Immutable resource loans retain the SAME paid body
through guard/path/File destruction. Account validation precedes selectors/full proof; after
the final source/ID3/name proof only cached negatives and infallible moves may intervene.
Borrowed guards do not own a Store/key/session or become new permission when cloned elsewhere.

## Root Core and Sync interfaces that can start independently

| Root interface | Required signature/owning behavior |
| --- | --- |
| Genesis anchor | `V2FounderExpectation::from_trusted_host(FounderGenesisAnchorData)` is explicit HOST selection trust. Core verifies full Profile/root signature and exact genesis/Admission/Header/image before creating the paid `V2FounderPrefixVerifier`. |
| Derive once | `verifier.derive_verified_append(exact_header_body) -> V2VerifiedAppendCandidate` contains OWNED StepData and private parent/source identity. `advance_host_selected(expectation, candidate)` returns old ReplayMath or an owning Err carrying the SAME unchanged candidate. All fallible predicates precede consume. No current-image setter/replay/clone. |
| Private index edit | Packed-row descriptor/rewrite preserves unchanged rows, validates the actual selected child range/count/height and parent max. Edge insertion descends real first/last paths. No selected-source/miss authority and no fabricated sibling max. |
| Create detach | `PreparedV2CreateData::into_handoff_data(self)` returns existing `DetachedV2CreateData` when materialized even Failed/cancelled, otherwise the SAME owned prepared value/refusal. Cache packet identity before late negatives. Extraction invokes no source/key/Clock/RNG/export callback. |
| Append key cut | Narrow opaque prepared-plan raw-sign operation stores returned raw64 IMMEDIATELY before hooks/late checks. Under Current/exact selected I/close guard it performs only cheap identity/preimage checks and device sign. Header reconstruction/full verification and derivation run outside global writers. |
| Append detach | Consuming negative DATA handoff carries original unsigned plan/raw64/optional Header/optional owning candidate; no fabrication. Shared original Append fee funds BOTH residual and detached tails. Store's SAME paid parent verifier is already owned before sign. |
| Projection | Private V2 semantic constraint/origin/lease issuer uses actual SavedB/current adapter. Same GroupLife, target ledger ViewLife and original source/scope/writer negatives; historical semantic authority does not itself acquire a cancellation lifetime. A/I/S issue nothing. |

## Top-level phases and error ownership

Root's `PreparedV2TextHost<T,R>` owns the closed whole owner from its first constructor.
Its actual cached sum dispatches before old genesis validators:

`GenesisSettled -> PromotionPrepared -> OperationalSelected -> APrepared/RetainedA ->
UnsignedI -> RawSignature -> BPrepared -> SavedBPendingVerifierAdvance ->
VerifierAdvancedPendingFinal -> BSelectedRetiring -> RetirementComplete`.

These are private owners, not serialized status authority. Persisted pendingTxn is separately
None/RetainedA/Unsigned/RawSignature, with one stable Txn/token/public parent and only matching
B clearing it. Capture-to-final-drain lane excludes other A and publication-ACK O changes.

Cache the real worker receipt/candidate BEFORE any final proof/return. After B, retry proves
selected B rather than an old signing parent. After consuming the candidate, infallibly cache
VerifierAdvancedPendingFinal BEFORE old math Drop/refunds/hooks. Drain old model/resources
outside writers, then repeat actual Current/source/B proof. Never re-advance a consumed model.
Only then may the host expose accepted/projection/publication eligibility. Retirement must
still retain its active lane until actual cleanup, even though pendingTxn cleared at B.

Root's physical worker/commit outcome stays discoverable after callback errors; errors do not
mean absence, unstarted IO or permission to generate another action/signature. An owning
constructor/transfer Err returns the same inputs; `&mut` retry never removes them into an
unfilled Option. Inert metadata retains only independently paid DATA/weak negative controls,
not original ReadCapture/SDK graph/Host/Store/model chains (the explicitly paid parent verifier
is finite transaction DATA). The owner/close revokes before payloads drain; fee owners are LAST.

For owning fallible transfers, the signature convention is
`Result<Output, (OriginalInput, Refusal)>`; two-input transfers return BOTH original inputs.
This is an ownership contract, not permission to copy opaque values. `&mut` work methods instead
retain their cached candidates/outcomes on Err and expose only borrowed historical receipt data.

## New cold family

Factory provides task-free `ColdV2TextReadAdmission` from the actual unowned loaded row2,
same full group/signer/source/nonce, group predecessor/JOIN/pin/old-v1 negatives and explicit
Scope basis. It grants no Some1, writer, task, missing-seed inference or genesis-head cast.
Physical `PreparedColdV2TextCapture::capture(&ColdV2TextGroupReadGuard)` freezes exact O,
MAIN/stage precedence, pendingTxn/WAL/dependencies and original files under its new paid loan.
Finite paid replay from genuine genesis/selected prefix produces the actual verifier; it
does not create a new SDK/nonce/source grant or use a cached image constructor.

`Arc<ColdV2TextHead>` then justifies a distinct shared-16 genesis/cold ownership claim and
funded new row2 owner/OnceLock handoff, plus distinct text writer registration using the same
private observation kernel. These shared-16 ownership claims are NOT the separate 16 text
active-work slots. Cold Current/selection and dirty existing-owner proof remain distinct.
Physical pending work resume obtains one NEW active lane only after actual prior mount/IO
quiescence and exact durable continuation, never because the old Weak owner expired.
The output owns this new cold family whole; no manufactured Warm/ColdGenesis/Repaired wrapper.

## Remaining blocking decisions versus independently authorable pieces

1. FORMAT's revised two-phase read/decrypt-drain/seal invoice must include all earlier
   A/I/S candidate caches, exact native/Path/File/guard/control/weak layouts and actual
   checked capacities. Proposed 16 MiB work + 1 MiB metadata is not a resource proof until
   those code assertions exist. Source/SDK/Create/Append/resident credits do not overlap.
2. Persistent prospective byte/object quotas, authenticated pending/selected counters and
   physical quota spending/retirement rules must be fixed. No remount reset or refund from
   merely parsing WD/WC. Public Actions/cipher/headers/H/manifest/records remain history.
3. Complete-stage precedence and scratch disposal need concrete source/work-quiescence
   issuers. Complete authenticated S/B stage is inspected before fresh sign; torn bounded
   nonauthoritative versioned stage is not decode-error-as-absence. Unknown/foreign/later
   or nonregular stages and partial MAIN corruption stay preserved.
4. New WD cleanup admission requires exact selected B + advanced verifier + actual drain.
   Initial implementation should require actual Current/close admission for NEW WD selection;
   a durable selected WD is the exact idempotent source for physical deletion retry. Any
   broader owner-dead NEW-cleanup permission needs a separate explicit review.
5. Cold physical replay/capture and the new cold owner/binding are real consumer dependencies.
   Nothing is declared as supported until a genuine new-mount A/I/S/B/WD/WC matrix works.
6. The whole original owner retains at most two obsolete initial controls: activation.seed
   <=8,232 bytes after cleanup unlink and original genesis head <=4,136 after O_G promotion.
   Account a separate persistent 12,368-byte/two-unique-inode baseline alongside the six
   mutable text-control slots; the original SourceSaved nine-file grant does not cover it.
   ColdAbsent has head only. Ordinary Warm Scope tails retain seed; ordinary ColdPresent
   and Repaired tails retain seed+head. A-born Repaired uses resource-only 64 KiB custody,
   so its Scope-negative tail alone does not retain those FDs after real owners drain.
   New operational cold ownership must not retain another obsolete mutable O inode in each
   historical Scope/observer: drain ephemeral control custody before replacement and keep
   immutable identity/frame DATA in its negative metadata. This must be tested on real IO.

Root may start the Core/Sync DATA/issuer primitives above; Factory may start the closed owner,
cached Scope borrower and readonly kernel pieces; Physical may start reviewed codec/immutable
work construction. Such pieces remain private/undeclared until this connected cohort consumes
them. Effect-capable lane retirement, stage disposal, selection and cold resume wait for their
actual paired receipts/invoices, not placeholder constructors or allow-dead-code call sites.

Required paired tests include Stop before/after sign and every selector admission, rawSome/
HeaderNone and owner-dead detach, A-only/ACK contention during I, B-visible lane exclusion,
lost B return and both verifier post-CAS phases, actual reader drain before WD, partial delete/
WC crash, 16-active-slot fairness/suspension, retained fee notices while detached debt survives,
same-token changed-body refusal, and two Creates -> remount/relocation -> exact token result.


## Required consuming WC-to-resident handoff (design; not yet implemented)

This is the required end of the first ordinary transaction, not permission derived
from B visibility. The concrete names below remain proposed until the Physical WC,
Root advance and Factory lane consumers are paired. None may be introduced as a
parsed-receipt adopter, generic callback, caller boolean or unused effect constructor.

The SAME actual verifier must advance once through the exact owned Core candidate
only after fresh genuine SavedB selection. Before any candidate take, all fallible
checks complete. The callback then infallibly caches `AdvancedPendingFinal` and the
actual returned retired-math owner in outside slots, before any late check. Candidate
absence after this cut means advanced pending final proof; it must never fall back to
Header reconstruction, derivation, signing or a verifier rebuilt from G. Old math is
drained outside all writers while its actual resident/work payments remain alive.

Physical must provide an original-bound final WC issuer. Its full Current proof must
include the actual selected WC, accepted public parent and Directory/Actions roots,
private request/publication roots, same C/source/pin/lineage, completed exact WD list,
page-parent barriers and actual work cleanup. A historical SavedB/WC, model Header/state
label or parsed retirement plan is insufficient. Replaced-root closure requires that
same advanced/final B basis and actual COW replacement plan. Unchanged/shared roots
must remain available and must not enter deletion. Query owners retained inside the
whole A/I/S/B graph must be transferred to an outside disposal slot and actually
dropped before joining old logical readers; finite control-IO zero alone does not
mean those readers drained. No transport or projection ACK owns a private reader.

Proposed Factory consuming edge:

```text
Current.retire_text_create_lane_into(
    genuine SavedWC, same AdvancedFinalBasis, actual WorkDrained,
    &mut outside_unique_lane, &mut outside_retired_lane)
```

It proves the exact original lane/transaction/control/source under fresh Current,
permanently closes that control, and removes only the exact registered Arc. The
outside retired slot owns BOTH the unique lane and removed registry entry before any
late Source check. Drop/Weak expiry, owner death, elapsed time, B visibility or a naked
count cannot release a lane. Failed admission preserves the original outside lane.
A late error after removal leaves the real retired owner cached outside, never lost.

Proposed Root consuming edge, only after the actual WC/lane finish proof above:

```text
PreparedV2TextResidentProjection.finish_after_wc_into(
    borrowed FinalizedCurrentGuard, genuine SavedWC, same RetiredLane,
    &mut outside_whole_completed_append,
    &mut outside_resident, &mut outside_disposal)
```

Physical needs a narrow consuming adapter which can transfer its WHOLE original Root
unsigned custody plus completed physical DATA into those owning slots. It must not
lend a mutable SDK/model, node, key, Origin, View, payload slot or public image adopter.
All account waits, allocations, same-original checks and full physical proofs precede
owning takes. A refusal before the take preserves every original slot unchanged.
After the take, resident AND old disposal ownership are cached infallibly before any
late check; a late error therefore retains both actual owners and never reconstructs
either. No File, native cleanup, old-math drop or SDK destruction happens under writers.

Inside Root's closed custody family the consuming split moves:

- the SAME advanced `V2FounderPrefixVerifier`, SAME domain and original real resident
  model1MiB payment into a private resident owner;
- the completed SDK child, raw/Header/prepared cache, old original-plan observations,
  original SDK2MiB payment and completed physical debt into outside disposal ownership.

The resident contains only its actual verifier, flat bounded selected-parent DATA,
domain and original model debit, with payload fields before their funding tails. It
retains no Saved/WAL/predecessor/SDK/transaction1MiB chain. Extend and check the actual
independent small-domain invoice for the resident and simultaneous transfer wrappers
before allocation; model/work credit does not substitute for metadata funding. The
old transaction's actual fees may refund only after all its real disposal, query,
registry and observation tails drain. The original whole native owner stays in Host.

A later request freshly proves the actual selected accepted parent and accepted
Directory/Actions roots plus private request/publication roots under Current against
this SAME resident verifier/source/domain. Header/state labels alone are insufficient.
It pays its own new SDK/work/transaction/query obligations before copies or allocation,
and derives the next original plan from the advanced resident. It never repeats the
first-request genesis constructor. Domain cancellation remains independently negative;
new-mount recovery requires its separate authenticated replay/bind path, not reuse of
old receipts. First Create -> WC -> second Create -> remount must receive genuine
end-to-end regression coverage before this lifecycle can be described as complete.
