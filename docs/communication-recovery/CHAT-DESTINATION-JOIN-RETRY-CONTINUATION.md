# Exact checkpoint RetryGrant and same-transport continuation (private source; UNRUN)

This follows 1b858c75's connected initial grant/arm/signed-input DATA packet. It remains finite
OwnerFact-v1 protocol1/root-kind1, never SDK acceptance or public-owner-v2 joining. No compiler,
tests or builds are authorized. Design review cleared the original reservation/actual SavedData
boundary; final actual adversarial static review is clear for the connected source and ten
regressions. All ten remain UNRUN; static review/formatting are not compilation/readiness proof.

## One explicit retained local retry invocation

Add to the caller-owned `CheckpointJoinNetwork<T>` a private continuation state with four
phases: Requested (exact target Metadata), Prepared (same target + exact prepared new task +
opaque Sync continuation + new Shared), Registered (the SAME actual installed task/cell), and
Completed (exact saved target/result). The Store
may retain immutable target/result DATA and fees, but NEVER either task sender, T or prefix.

`ensure_retry_grant(store, new_limit, rng)` starts one explicit local request only if this
object has none, retaining the desired limit/target before any NEW RetryGrant candidate I/O.
Preliminary same-source old DATA repair and current-file/root confirmation remain allowed. All retries, including calls
after a lost successful return, recover that SAME target/result; changed limit refuses. A
separate explicit `next_retry_grant()` may release only a Completed invocation so another
intentional local retry action can start. No timer/decoded status/reopen/ensure-submission calls
this API or initializes another grant. Cold opening keeps the actual saved grant/units; it
cannot reconstruct a runtime unstarted invocation from a stored arm.

The target changes exactly grant_id+1, scheduling_revision+1, new trusted limit, spent=0.
The DATA grammar preserves Active/Paused, lifetime ordinal, latest arm, any existing received
input, immutable source/request/KP/cipher/root/basis and other rows/descriptors. This executable
slice is ACTIVE-ONLY: a paused/newer scheduling source refuses without RNG, renewal or byte/unit
change. It additionally REFUSES while an input exists (warm Held or cold input summary); input cleanup/Accepted/Intent
must be implemented separately. Two explicit zero-submit regrants may retain a latest arm from
ANY earlier grant. No refund after ambiguous transport handoff.

Close old task and old cell atomically/nonblocking BEFORE the dependent retry write. Reconcile
old Armed/Submitting ownership outside all Store/cell writers; the actual lower driver alone may
still hold its original capacity. Recover any already-admitted old counter candidate byte-for-
byte, with no new ticket. If a locally proposed old arm was NEVER admitted (current is still the
original metadata and Store has no candidate), discard only that proven unstarted proposal. If
current equals its genuine retained target, cross its covering barrier and update metadata to
that exact target. Any foreign/later row refuses rather than adopting a new capture. New retry
source writes use the existing exact original admission basis, never a later admission row.

## Prepare once and reserve BEFORE exposing the successor revision

A real competing coalesced caller must not steal r2 after A saves it but before A registers.
Therefore new task/context/target/candidate are prepared OUTSIDE the writers and BEFORE first
RetryGrant admission. New sender lives only in the caller-owned prepared continuation. Control
preparation/input fee become shared original strong owners, so a new cell cannot copy private
keys or issue another resource fee. New metadata/registry fee is reserved separately.

The new context has its own physical identity, exact target source/revision, prepared new task
negative observer, SAME mount/pin/request/binding and floor equal to the saved lifetime ordinal.
Narrow private PreparedOriginalPredecessor identity/observer getters supply negative lifetime
only, never registration proof. Source/control/data owners in Store cannot retain the sender.

At the SAME final admission->catalog point that takes the immutable RetryGrant candidate,
validate the whole original parent, exact current source/r1 row and exact old CLOSED task identity,
then validate the candidate as exactly one checked RetryGrant (no other source/namespace change).
Preflight every fallible check/allocation before either mutation. Atomically replace the old r1
registry slot with ReservedContinuation(new identity,r2,source,original basis,candidate digest,
original outcome cell) AND assign Store.pending. No await/fallible/drop gap lies between them.
Old registry metadata/fees leave the writers before dropping; no second scarce slot is needed.

The caller's prepared owner is updated to the original reserved claim in that same transition.
A first-admission error returns/retains its same owner/source untouched. An error AFTER admission
preserves that exact owner/claim/candidate through repair. Reserved is private runtime state,
never serde or a persisted live/unstarted flag. Generic activation and execution-observation
validation refuse Reserved for EVERY other task identity, even after sender Drop/weak expiry.
The old bind remains closed, and r2 cannot be first-come reactivated in this mount.

The concrete reservation/finalization methods live only in the admission family and receive
actual original capture, old task identity and the mutable SAME prepared new owner, plus the
actual retained candidate. They do not accept decoded reservation labels or a callback/status
bool. Any private metadata needed by the candidate is prepared/charged before writers.

## Actual saved outcome, repair and independent control progress

Reserved entry and retained candidate share ONE private paid outcome cell carrying immutable
candidate/source/identity binding and no task/T/key/session. ONLY the actual same-byte physical
writer marks that exact outcome SavedData AFTER its file/rename/root barriers, synchronously
before Store clears the candidate. Ordinary repair after caller Drop can mark SavedData but
NEVER Active; cold bytes do not reconstruct that cell or an unstarted owner. A lost return is
looked up through the SAME original cell/claim rather than inferred from matching payload labels.

Independently admitted other-group/control/catalog operations may proceed after repair. Such
legitimate complete-envelope successors need not strand finalization: the original SavedData
cell proves this exact candidate crossed its actual barrier, and finalization separately opens
and crosses the actual CURRENT whole-file/root barrier and checks its unchanged exact source
Metadata (r2/counters/request/KP/cipher), current actual policy and original pin. It cannot merely
adopt a matching JOIN subpayload under an unknown/changed parent; the constructor/writer must
have admitted every complete current envelope and the caller supplies its original SavedData
claim. A changed source/r3, foreign descriptor/unknown current, remount or closed task refuses.

Caller Drop leaves Reserved weak exclusion and durable DATA/debt; it never retains a sender or
renews r1. An explicit genuinely later saved pause/resume revision may supersede CLOSED Reserved
using the existing strict-later rule. The exact same r2 remains unrenewable. On cold reopen no
old transport/sender survives; source opening preserves all saved grant/ordinal units and must
never decode a runtime unstarted ticket. The still-open cold partial-stage limitation remains.

## Selected narrow Sync API

The concrete methods are on the already-bound owner, with no per-call transport/prefix:

```text
BoundOriginalJoinNetwork<T>::prepare_original_continuation(
    &mut self, new_context: Arc<dyn OriginalJoinNetworkContext>
) -> Result<PreparedOriginalJoinContinuation, SyncError>

PreparedOriginalJoinContinuation::retained_cell_data(&self)
    -> Arc<RetainedOriginalJoinNetwork>

BoundOriginalJoinNetwork<T>::attach_original_continuation(
    &mut self, original: PreparedOriginalJoinContinuation
) -> Result<(), ReturnedOriginalJoinContinuation>

ReturnedOriginalJoinContinuation::into_parts(self)
    -> (PreparedOriginalJoinContinuation, SyncError)
```

The opaque prepared value is non-Clone/nonserialized, has no source getter or exchange method,
and is constructed only by the SAME actual Bound. Its original physical bound identity is a
private Arc minted at the first bind, not a peer ID/label. The new cell's bind claim is RESERVED
true immediately during preparation; returning its inert Arc for Store DATA retention therefore
cannot let any caller bind a different T before attachment. Old and new bind claims never reset.
Only successful exact attachment changes Bound.cell; its original T/target/prefix stay put.
All checks precede source.take in preparation and precede cell swap in attachment. Failure
returns the same source untouched or the same opaque prepared continuation, respectively.
No failure path hands out an unbound cell with the moved private source.

## Finalize ONLY the originally reserved owner

Finalization takes the original retained reserved claim and SAME prepared owner. Under actual
admission->catalog it requires the matching Reserved entry/identity/source/basis/r2 and original
SavedData outcome, plus actual current whole-file/root/source checks above. Only then may it
publish the SAME Shared and transition that entry Active. Generic execution capture requires
Active, so reservation cannot arm/send before its real barrier. Sender remains caller-owned.
No fallible ownership movement may erase the new owner after entry installation: record the
Registered phase before returning, and retry NEVER installs/remints the same r2 entry again.

After actual Active registration, attach the SAME opaque Sync continuation to the original Bound.
Attachment checks physical bound identity and returns SAME continuation on failure; Registered
phase retains SAME task/cell for retry. Successful attachment changes only the cell, with T and
ordered prefix intact. No await follows. Replace caller task/shared after registration/attachment;
old sender/cell/shared/fee owners drop outside writers. Neither old/new bind claim is reset.
New Control shares only original preparation/fee, not an unbounded predecessor-cell chain.

This is private trusted-host Rust choreography, not a serialized grant capability. Root's
future native actor must retain this typed owner before exposing any retry reply. Source
caps/SDK/global host and production root-kind2 remain separate gates.

## Required authored adversarial regressions

- Real grant1 arm -> cancel/spent -> exact RetryGrant2/new saved revision -> same T/prefix
  submits the SAME request/KP; exact owner cache replies without another Add.
- Warm lost retry return and changed-limit retry: one target/grant/nonce, never another save;
  explicit second zero-submit grant preserves the original latest arm from grant1.
- A genuine competing coalesced caller between r2 Saved and completion cannot steal its
  Reserved revision; faults/owner Drop keep exact weak reservation/debt, other-group work flows.
- Old task/cell cannot submit or attach after new registration; no same-revision renewal.
- Every retry write pre-stage/partial/sync/rename/root fault; old arm candidate before vs after
  first admission; same ciphertext repair and no ticket from cold/admitted bytes.
- Held/cold input refuses regrant, preserving all bytes; foreign/later source/head/row refuses.
- New task/cell preparation Busy/capacity/registration/attachment failure preserves same owner;
  caller Drop cancels both old/new senders without Store retaining one or reviving old work.
- Actual consumed lower slot remains charged through driver completion despite regrant;
  no resource refund or durable-unit reset from task/future/transport/mount/epoch changes.
- Original prefix consumed before a cancelled request is still present after SAME-T continuation;
  another T or arbitrary caller prefix cannot be supplied. Bounded harness only; no tests run now.

## Current authored source and evidence limits

The concrete paths are predecessor/join_continuation.rs, producer/activation/network/continuation.rs
and Sync prepared_direct_join/network_exchange.rs. The first candidate admission reserves the
same next owner AND Store output before any fallible I/O. The actual writer marks only SavedData;
only same-owner finalization after real current file/root/source checks can mark Active. A
prospective owner is closed by CheckpointJoinNetwork.close as well as by whole owner Drop.

The original preparation/input fee is shared, not regenerated; each continuation's named fixed
metadata/layout fee remains in its new Control after cached response Drop. No parent-cell chain
is retained. First candidate plaintext/sealed buffers retain their separate actual WRITE_FEE.
All strong resource/candidate/context last owners retire outside original writers, including an
error in Sync preparation where a last host context may own quota/destructors.

Ten regressions are authored: seven App tests and three Sync tests. They remain UNRUN. The Sync lifecycle-prefix input is explicitly a local synthetic fixture;
real Mem requests verify exact request bytes and physical Bound refusal, not native/TCP/audio
acceptance. Tests cover paused/received-input refusal, same-owner close, real owner saved-reply
loss/cache retry, coalesced r2 stealing, warm I/O boundaries/owner Drop, two zero-submit regrants,
wrong-bound/floor/bind/no-source-fork behavior and context Drop after old cell guard drains.
Actual SDK/Accepted/cold INPUT authentication, ScopeIntent/Frozen/Current/ready, root-kind2 invite,
full typed SDK/global native admission and deployment remain outside this private source slice.
