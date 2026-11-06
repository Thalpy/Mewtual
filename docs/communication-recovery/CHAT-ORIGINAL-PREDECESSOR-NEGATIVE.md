# Original predecessor exclusion and typed join custody (authored, unexecuted)

This is a narrow negative prerequisite for the configured finite OwnerFact-v1 JOIN source.
It authorizes no scope advance, request, current state, publication or ready-owner handoff.
The execution schema/allowance/cell/receipt path remains a separate required implementation.

## Exact runtime owner

Add a fixed five-slot private predecessor registry to the existing admission `Runtime`.
Each slot retains a bounded key, COMPLETE original optional Record, immutable source ID,
scheduling revision, closed kind, inert Arc identity, weak revoker and negative observer.
It owns no source keys, sender, transport, Store or vault session. The actual non-Clone
`OriginalPredecessorTask` alone owns the revoking sender; its Drop closes the original.
Observers may retain identity/Notify but never that sender. Registry expiry is NOT proof that
persisted source/debt is absent: the slot remains an exclusion until an exact supported source
transition/terminal consumer resolves it. No opportunistic expiry eviction.

The only current runtime kind is `JoinCheckpoint`, with its actual JOIN consumer connected.
The planned `ArchiveFounder` kind/constructor is deliberately deferred until its concrete
source/debt consumer exists. No placeholder caller or warning suppression stands in for it.
Existing catalog support is unchanged; ID2 remains unsupported by the Pending opener.

Source scheduling and lifetime are distinct. One original scheduling revision can register
once per physical mount, even after task Drop. A later SAME-source scheduling revision may
replace its already-negative slot only after the actual current catalog proves that exact
revision and original admission basis. Other source/kind/basis never adopts the old slot.
A negative revision/source control closes only its captured registration, never a later one.
Five retained slots do not grow on network churn; no automatic execution grant follows.

## Registration API and actual consumer

New private `chat_admission::predecessor` owns the registry and task/observer wrappers.
Preparation reserves a fixed 4096-byte logical metadata allowance from the SAME caller-shared
pool BEFORE copying the bounded key/original basis or constructing sender/identity/observation.
The inert registry Entry retains that original non-Clone fee after task Drop, until exact
replacement or physical-mount destruction. The actual JOIN wrapper separately retains its task/
source fees. These are named logical inventories, not full allocator/RSS or SDK heap bounds.

```
PreparedOriginalPredecessor::join_checkpoint(capture, exact_source, scheduling_revision, shared_pool)
    -> PreparedOriginalPredecessor
ServerStore::register_original_predecessor(
    capture, &mut PreparedOriginalPredecessor,
    confirm: FnOnce(&JournalAdmissionPermit) -> Result<(), AppError>
) -> Result<(), AppError>
```

The callback is the existing concrete internal original-admission/catalog inspection boundary,
not a namespace support callback. Registration holds the original admission serializer while
the actual Pending consumer takes catalog AFTER it, repeats the complete source/schedule row,
original pin, negative and whole parent/candidate checks and crosses actual file/root barriers.
After that callback releases catalog, the SAME admission writer remains held, preventing any
independently admitted catalog mutation from overtaking final registration. Recheck the exact
original permit/task before installing into its fixed slot. Allocation, quota Drop and source
destruction happen outside these guards. No sender is temporarily transferred into Store.

The existing journal preparation gains one consuming `activate_original_transport(Store,T,
trusted_target)` method. It first confirms that its exact source is durable and Active, then
registers its original task and returns `ActivatedCheckpointJoin<T>`. The failure result returns
the SAME preparation and T; no keys or transport are replaced. The returned object owns that
ONE T/target, an initially empty lifecycle prefix, original preparation, task and fees.
It exposes negative close/cancellation plus cheap route eligibility DATA ONLY. It has NO
network, old-initializer, recovered-source extraction or ready conversion. Future execution
must consume this original owner, preserve its same-T prefix and install the real persistent
arm/retained-success consumers before making it runnable.

## Shared exclusion and row changes

`prepare_chat_admission` refuses a protected key BEFORE both its issued-candidate retry/dirty
flush branch and its RNG/owner replacement branch. This covers fresh AND legacy wrappers, not
only UI factories. The registry is negative exclusion, never a matching-call scope permission.
The future explicit durable source/ScopeIntent adapter must prove its actual original identity
and intent rather than passing the old generic function through a kind or raw bypass flag.

Every ACTUAL row mutation revokes only a registration whose COMPLETE captured basis equals the
old row, immediately before the in-memory change: new initialization, Initializing-to-Current,
or non-Retired-to-Retired. It retains the inert exclusion. An unchanged AlreadyCurrent or issued
retry does not revoke by key. Critically an old already-Retired retry must not cancel a NEW
source registered against that exact unchanged Retired row. Custody repair/re-reading the same
record never renews an original source signal. Retirement/control remains allowed; unrelated
keys continue to use existing paths. Constructors load inventory only before exposure.

Pending pause/resume closes the matching old task BEFORE admitting its dependent durable
scheduling change. If that write fails/uncertain, the old worker remains closed; it cannot
renew from a read of the same scheduling revision. Retrying the exact candidate preserves
keys, allowance and negative closure. Another source's unchanged control work remains usable.

## Required authored regressions

Actual saved Pending source activation and exact original transport return on refusal;
duplicate same-revision and dropped-task rebind refusal; real pause/resume admits only a new
original task; stale revoker cannot close it; retained Store metadata cannot keep it alive;
absent and Retired basis generic issuance blocked before RNG/dirty retry; actual row retirement
or Current mutation closes only the invalidated original; no-op old Retired retry preserves a
new same-basis task; unrelated group control progresses; fixed five-slot bound; and a held
admission/catalog contention seam proving final negative recheck before registration.

All code/tests/build execution remains paused. This design makes no native-global pool, full
SDK heap/work, durable ScopeIntent/Frozen/saver or production public-owner-v2 join claim.

## Required scope/terminal adapter remains absent

The registry is a fixed bound on UNRESOLVED runtime exclusions, not the number of saved groups
a vault may retain. Existing saved-group control needs no new slot. There is no weak-expiry or
raw release API. The future actual intent-backed installer must recognize its own exact source
and preserve the SAME original sender/reservation through Current; invoking old generic prepare
or treating its own Current transition as an unrelated invalidation would revoke that live scope.
Only an exact actual persistent terminal/debt handoff may remove the scarce registry entry, while
the original sender stays owned by the active consumer until eventual Drop. This needs its real
source/canonical/Current/head consumer, not a kind/status/saved boolean or success callback.

Seven actual JOIN custody regressions and six admission-family negative regressions are
authored UNRUN. Direct source formatting/whitespace checks do not replace production compilation
or the primary-owned required final suites. The separate Sync retained-exchange draft remains
undeclared; persistent grants/arms/receipts and runnable recovery have not landed in this slice.

## Deferred archive consumer contract

The reviewed proposed `PreparedOriginalPredecessor::archive_founder(capture,source,revision,pool)`
and `OriginalPredecessorTask::identity()` will be restored only with an actual non-test consumer.
The archive-only draft test was intentionally removed with that unsupported runtime API, not to
weaken a current JOIN assertion. That future actual packet must test archive exclusion before
old generic issuance, no automatic renewal even at a larger revision after creator close,
zero row mutation from negative registration, and exact source/debt/terminal handoff. Current
JOIN tests remain unchanged apart from direct test-private identity inspection.
