# Cold genesis Current2 scope ownership

## Integrated checkpoint (6 November; Chat V2 remains disabled)

The complete genesis Current2 scope, task/owner handoff, distinct installed
writer and owning cold caller are now connected in the ordinary library build. The public
`PreparedColdV2GenesisCurrent` / `UnexposedColdV2GenesisCurrent` wrappers retain the same paid
source, SDK, mounted owner, writer, receipt and node across retry; only recovery and immutable
DATA escape. Actual current diff and neighboring lifecycle paths passed read-only adversarial
review with no remaining blocker, high or medium finding.

Primary reports **5 actual cold owning-caller tests passed in107.60s** and **2 scope-inventory
resync tests passed in33.87s**. The separate production all-features App check passes without
warnings; strict all-target/all-feature App Clippy also passes. These are focused backend
results. Separately, the full frontend suite passes3,434 tests with no skips. Complete backend
suites remain outstanding; primary owns the serial validation pipeline. See
[HANDOVER](../HANDOVER.md) for the current ledger.

The original milestone retained complete activation.seed. Its Windows-only seed-absent
extension now independently verifies exact genesis metadata and a closed two-folder inventory,
then requires a new covering scope write EVEN CLEAN. The private NeedsCovering state survives
both actual write cuts and final physical-proof failure, even when the loaded table is now
clean. It clears only under the same admission writer after the actual write and full final
proof; read-only validation/extraction refuses it while pending. All4 new actual capture/owner
tests and4 inventory-helper tests pass. Initializing1 repair, actual seed cleanup,
later operational heads, governor issuance, runnable/native activation
and live20/50-writer text SDK admission remain unfinished. The implementation below does not
establish those capabilities or upgrade a decoded receipt into a mounted owner.

## Inputs and the second-cold-task gap

The inputs are one actual `GenesisSourceDataRef`, Root's new
`Arc<ColdV2GenesisCurrentHead>`, and Factory's actual newly registered cold group task. The
receipt proves complete matching seed (or the separate strict Windows absence role) + immutable group pin + exact genesis head + no stage,
same source/read/SDK/mount and file/parent barriers. It carries the opaque full
`ColdV2GenesisScopeBasis`; decoding labels never constructs this receipt or the scope owner.

Factory's separate `from_cold_current(store, source, &Arc<ColdHead>)` must claim its original
registry slot before Scope installs an owner. It retains that claim while the row is unowned;
the preexisting permanent directory is not a release event for this cold role. Otherwise a
second cold task can pass the same unowned row before either installs its own scope owner.
The private cached `require_registered_cold_current_claim` checks the exact registered task,
source/read/SDK and SAME receipt pointer/basis. Final namespace guards also inspect the actual
entry under pins. Cancellation, weak expiry and equal frames never renew the claim.

Only after the actual new scope owner is cached may Factory perform
`task.handoff_cold_current_owner(&ColdOwnedCurrentAdmission)`. It validates actual scope/source/
head/task/owner and the original permanent directory, stores the exact owner association in a
private preallocated OnceLock BEFORE removing the registry entry, then drains retired inert
fees outside all guards. The association has no sender, Store, graph, key or session. The
task keeps the same strong lifetime; no caller boolean can mark it owned. Retained scope owner
custody then fences later cold readers, so this is not a lifetime16-group cap.

## Actual preparation, not a warm Init1 adapter

New private `ColdV2CurrentPreparation` records actual Current/revision2, not an invented
Initializing/revision1. Its Managed reservation says Some(1) only for the historical source
plan. It caches the exact receipt, basis, task observation and real mounted owner before any
owner insertion. It exposes no raw ChatAdmissionPreparation, Server, source setter, public
constructor or binding before the separate actual canonical writer is installed.

The private connected owning method is:

```text
prepare_cold_v2_current_scope(
    &ServerStore, &GenesisSourceDataRef, &PreparedV2GenesisGroupNamespace,
    Arc<ColdV2GenesisCurrentHead>, &mut Option<ColdV2CurrentPreparation>
) -> Result<(), AppError>
```

None requires managed CLEAN v6, exact Current2 row/full source nonce/Fresh historical attempt,
no runtime owner (including cancelled), no foreign signer/owner, no same-group predecessor,
and the shared JOIN group fence before pins. It requires the actual still-registered cold
claim and full physical receipt. Setup, observer/custody allocation and the real new reservation
are followed by another original source/task/basis + owner-absent check immediately before
cache -> `Runtime.owners.insert`. No recoverable fallible work or callback separates those
two steps; BTree insertion retains its ordinary infallible/OOM-abort allocation behavior.
No table key, phase, revision, dirty bit, RNG or head is written.

Some requires SAME receipt Arc/read/SDK/task and SAME newly cached mounted owner. Before task
handoff it validates its exact retained registered claim; after handoff it validates Factory's
actual OnceLock owner association, never the now-retired slot or matching scalar labels. It
cannot evict/revive an existing owner through generic reservation_locked. Every failure after
cache retains that exact preparation and owner; a separate invocation cannot adopt it.

`ColdOwnedCurrentAdmission` is an !Send HRTB borrow of the real Runtime/source/task/preparation.
Its getters expose the actual owner identity, original negative observation, full basis and
head to concrete Factory/Root guards, not Runtime or a trait/callback proof substitute. Its
own-row negative comparison may inspect the SAME dirty table, but its concrete owned lineage
role requires CLEAN before writer installation or handoff. Dirty-table persistence uses the separate paid DATA maintenance or installed-writer proof
consumer described below; the clean owned role itself grants neither permission. It never rewrites Current2 as Init1.

## Pay both the Runtime owner and its weak tails

Root's actual cold metadata debit covers preparation/basis/key/observer/setup layouts, but
the fee must follow every retained allocation it claims. Runtime keeps AdmissionOwner.life
and its watch after preparation/observers drop. An Arc<ColdHead> only in the escaped observer
does NOT pay this residue. The shared-family funded constructor supplies actual resource custody to that real
owner life explicitly;
no global owner allowance is substituted for the actual receipt.

The shared-family implementation keeps optional strong resource custody LAST in AdmissionOwner,
default None for old constructors. custody_from_owner takes the actual AdmissionOwner and
explicitly supplies its SAME custody to observer/Authority and revoker Handle; no weak resource
upgrade can silently create an unfunded tail. A narrow funded reservation helper serves the
real warm SavedSeed and later cold Head callers. Custody is the SAME resource-only receipt Arc: readonly files,
bounded frames/context and actual metadata fee, no Store/key/source graph/creator/session/
sender. Escaping observer and revoker wrappers also retain that same custody through their
weak-only tails. Runtime removal revokes/drops the old life/Sender at its original point and
detaches ONLY its resource into an outer holder declared before Runtime; the holder drains
after all guards on success or error. No custody points back to a sender, avoiding a cycle.
Actual ABI/layouts and fixed setup are included in Root's named invoice; old global-table and
ordinary legacy owner accounting remain their existing separate contracts.

The group-wide predecessor query and shared funding changes passed actual read-only review
and primary's focused tests: six funded-owner lifecycle tests and three revocation controls.
They cover Runtime-only retention, authority/keepalive/revoker tails, immediate native receiver
closure while resource is deferred, and mismatch/replacement/retirement last-drop lock probes.
A genuine SourceSaved -> namespace task/SavedSeed -> scope1 test pins the real warm receipt's
512KiB charge after caller holders drop. The connected cold owner invoice includes
`COLD_V2_CURRENT_SCOPE_METADATA_BYTES` within its actual1MiB class, independently of warm512KiB,
source/SDK and temporary scope serialization payments.

## Canonical writer and final proof

The cold Current writer is a distinct actual constructor, not the warm Init1/SavedSeed
PreparedWriter. It installs under Factory's concrete cold-owned canonical guard. Because
the original ColdHead predates this installation, Root issues a separate private
`ValidatedColdV2CurrentWriter` only from the actual installed writer plus Root guard after
the complete cold head/source proof. It retains the SAME ColdHead Arc, funded original writer
observation and exact owner/task association. Observation alone is not installation proof.

Scope's connected private binding phase latches that actual receipt before any binder allocation and
never repeats the installation callback on retry. It then uses the exact nonreplacing binder
and retains the whole funded writer observation. Subsequent pair/resync/final validation
must repeat the concrete installed-writer/full cold source/head proof; receipt status or a
bare negative observation is insufficient. The actual receipt validator, concrete Factory cold guards and owning caller are connected
and covered by the focused results above. There is no Head callback, row rewrite or
regenerated frame in the cold path.

The binding refinement passed read-only design review with three concrete requirements:
the receipt proves the SAME installed registration generation, owner, task and ColdHead Arc
under canonical plus Root; its Arc/Box allocation precedes the final physical proof, followed
only by infallible moves/clones; and binder allocation is followed by another complete proof.
Dirty resync needs its own existing-owner/installed-writer read role, distinct from CLEAN
installation/binding. That role must be available to a cached WriterValidated attempt as well
as CurrentBound: a binder refusal followed by another group's dirty write cannot strand the
already latched callback. Unbound cannot enter it. After real flush and temporary-credit
refund the final outward proof is CLEAN. No dirty install or replayed callback is authorized.

Earlier gaps also precede ColdHead capture or owner insertion. A source/phase-specific repair
cannot cover those cases. The earlier Unbound-only helper was replaced by the
smaller private DATA maintenance method `resync_v2_current_scope_inventory`. It requires the
actual same-pool temporary scope credit before/after admission waits, this SAME loaded managed
format6 Runtime.table, the original live mount and actual ID3 namespace proof. It releases the
brief read-only Root guard before the existing dirty-only latest-table flush, then repeats
actual namespace/mount checks. It never reloads disk or changes a row, owner, phase, nonce,
claim, source or receipt, and returns no binding or source permission. Normal cold/warm
source and artifact admissions remain unchanged; installed proof stays strict.

This deliberately changes the superseded helper's cancellation contract: previously
authorized dirty rows MAY be persisted after their source/owner is cancelled. That DATA
settlement does not revive the publisher or permit activation/renewal. A cancelled mount,
lost/foreign original namespace object or foreign/insufficient credit still refuses before
the scope write. The two passing genuine tests exercise actual debt before ColdHead capture,
registered claim before owner, Unbound owner, whole-table/owner identity preservation,
real credit pressure/refund, actual handoff, and actual source Drop plus a new equal-byte
read that cannot adopt the old owner/task. Namespace corruption, equal-byte foreign marker
replacement and mount cancellation retain the debt. Root owns the separate lost-cached-receipt gap regression, actual wrapper callers and serial
execution. The helper's exact
design and actual revised source/tests passed read-only adversarial review with no new
findings. Actual wrapper/declaration integration and the two focused maintenance tests are complete;
full-suite validation remains separately required.

Final flow is runtime pair -> funded latest-table resync if required -> drop temporary scope
credits and drain retired metadata outside writers -> final read-only CLEAN Current proof ->
field moves only. The approved warm `validate_v2_genesis_current` is a separate new child;
cold has its own bound-phase validator. Dirty preserves the same node and requires a
new explicit real resync attempt. Original cancellation must occur before payload cleanup;
cheap cancel_original targets only the actual original scope owner.

The connected tests cover real task/owner admission, exact cached handoff, installed writer
and binding, late cancellation, unrelated dirty rows, Runtime-retained payment and complete
seed-present row2 cold reopen. Broader fault/capacity and full suites remain primary-owned.
No fake watch, paid flag, warm Init1 proof or missing-seed fallback substitutes for the actual
source, registry, writer and original mounted owner. Seed settlement requires its own covering
latest-table write EVEN CLEAN and distinct cleanup/seed-independent proof; dirty-only DATA
maintenance and this seed-present milestone do not provide that barrier.
