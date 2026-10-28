# Native admission integration

Status,28 October: Root registration and browser preclaim are reviewable leaves in
PRs79/80. They are joined at `a44b51b2` on `communication-v2-native-admission`.
The subsequent Actor staging implementation compiles; both new Root preparation tests
and all eight Actor staging regressions pass in that checkout. Complete gates are pending.
Production V2 remains OFF. This document describes the next connection, not completed
native admission, signed readiness, media relay or physical acceptance.

## Implemented Actor boundary

`AdmissionRpcPermit` pays4KiB from the selected ordinary group/pool. Root refuses a
foreign pool/scope before minting. The original3KiB registration is returned even when
HIGH capacity fails; retries using its original observer never mint another incarnation.

`try_queue_voice_admission` admits without waiting for mailbox space. The publisher's
`accepted(work)` installs exact lifecycle custody BEFORE enqueue can wake Root. Full
mailbox and failed custody installation are proven unstarted. A queued command destroyed
before application passes the SAME Work to `refused`; callback return is not retirement.
Actor apply removes the publisher before Root entry. Unwind after that point remains
uncertain; an unused-publisher destructor must never certify publication absence there.

`completed(result, work)` must synchronously stage the exact affine outcome. Root has
released Group/Policy guards. Staging owns quarantine custody only. Positive native
attachment waits for actual callback/argument retirement and the final native fence.
The plain Work-to-payment graph contains no Route, Row, publisher or task backedge.

Root registration3KiB, Actor RPC4KiB and native worker8KiB are separate contributions
within the existing group/process pool. Measure the concrete native Publisher BEFORE
Box construction. Source/intake metadata and physical Control are not free slack.

## Canonical native slot

Keep one small slot inside the original bootstrap Row, paid by that Row's8KiB fee:

- Stable optional Root registration; monotone current ordinal and cancellation floor.
- An exact invocation token, actual Work and one affine staged outcome.
- Optional HIGH capacity, worker/finalizing/reclaiming state and terminal uncertainty.

Store no future, JoinHandle, publisher or Route in the slot: their captures could form
Row-to-worker-to-Row cycles. The actual worker/publisher temporarily retains the Route;
Work retains only its original payment and retirement cell.

Allowed progression is original Pending, queued, Staged, Finalizing, then Reserved or
a proven unstarted capacity retry. Canceled generations go through Closing and actual
return before advancement. Stable registration survives a canceled generation; HIGH
capacity does not. Dead epoch/join/call bindings terminate this physical Session rather
than refreshing the Root registration. Ordinals cannot skip or wrap; validate the exact
permitted ordinal BEFORE calling Root's negative floor, including MAX.

Install a successful new registration independently of HIGH failure, including after
cancellation. Apply the latched floor/close before exposing its capacity. Never overwrite
another registration or silently discard a successful mint and then call prepare again.
An entered but unstaged/uncertain invocation stays quarantined. Retirement alone proves
neither absence nor permission to remint.

## Locks and actual return

Use UI commit, Coordinator, bootstrap Row, then the exact slot. Route's existing
`try_with_open` ALREADY holds Row.state: borrow the slot from that same guard instead
of recursively locking State. Final positive publication checks original selection,
physical Session, invocation/ordinal, Root currentness and capacity ownership/generation.

Quarantine staging and negative cancellation remain reachable after End/effects closure;
they must not require the positive UI gate. NativeWorker and maintenance consume a staged
result through ONE affine Staged-to-Finalizing transition. Recheck exact invocation and
actual retirement after waiting outside every native/Root guard.

Detach heavy outcomes/capacity into outer owners and destroy them outside UI, Coordinator,
Row and Root guards. Closing persists through that destruction and actual old work return.
Do not publish Returned because an index was removed, a receiver disappeared or a boolean
changed. Do not report channel collection while admission work or reclamation remains.

Extend original-owner End, bootstrap finish/retire_channel, registry collection and
stop_and_wait_for_physical. Close first, retain exact cleanup routing, then collect work.
The first close fence must release UI commit before awaiting Actor work or destroying
Root capacity; otherwise the returning worker could wait on that same commit.

## Protocol pinning and browser coupling

The existing pilot starts automatic discovery after authenticated bootstrap Status ACK.
Pinning a new admission mode afterward would race that old producer. A candidate design
is an explicit authenticated short Register operation before ACK, followed by inert
waiting and actual Root work only after ACK. First Register after ACK must refuse an
upgrade. A valid first Cancel before ACK must select the SAME sticky admission mode,
so a delayed ACK cannot turn negative admission debt into pilot discovery. Observe
alone never creates a mode or ordinal. First Register/Cancel on an already ACKed
pilot Session refuses without changing mode or the cancellation floor. Exact retries
and subsequent permitted ordinals on an ALREADY pinned Session may occur after ACK;
the pre-ACK requirement applies to initial mode selection, not every generation.
Old clients that never select that mode retain the existing pilot behavior. Mode
selection is irreversible through cancellation, pressure, lost replies and epoch
changes. Read-only design review accepts these conditions; implementation is pending.

Header admission must select the new short Control grammar by operation and exact physical
Session logical ID BEFORE RowId decoding/counter admission. No renderer peer/role/profile
boolean substitutes for that native selector. Unknown versions fail closed without fallback.

Browser must retain its original preclaim through an ambiguous Register and fence native
cancellation BEFORE releasing paired browser capacity. The currently reviewed bare preclaim
has no native effects; adding that pairing requires new ownership/retirement tests. Do not
hide a new Session coordinator in the nearly full intake8KiB or IPC16KiB metadata. Measure
its independent payment and update the retained broker ABI on the SAME window symbol.

Reserved is capacity only. It is NOT signed Ready, a native32-row ledger reservation,
DeviceId orientation, bilateral Request/Grant/Commit or RoundRef/KDF permission. Those
remain separate requirements before any production V2 activation.

## Required adversarial evidence

1. Park the actual Actor after Register; observe Preparing, Cancel, then release it.
   Preserve the same incarnation and never positively attach canceled capacity.
2. Lose Register's reply after Root publication. Retry/Observe uses the same stored
   outcome, with no second Root mint or RNG/factory call.
3. Drop a native worker before a queued command is destroyed, and unwind after Root entry.
   Preserve exact Work; distinguish proven never-entered destruction from uncertainty.
4. Hold actual publisher return and capacity/output destruction. Preparing/Closing blocks
   advancement until the real boundaries return; every overlapping graph remains paid.
5. Race maintenance with worker finalization, current/next/stale Cancel, skipped/MAX
   ordinals, lock/unlock, room deletion and epoch replacement. Consume one affine outcome.
6. Fill mailbox, ordinary bytes, native position and Controller/Row metadata. Proven
   unstarted pressure retries the same generation without resetting a protocol deadline.
7. Prove pin-before-ACK prevents pilot discovery and rejects a late upgrade. Keep old
   canonical bytes/profile behavior intact for clients that did not select admission.
8. Run real native/HTTP/WebCrypto/TCP composition with A-C forbidden, B joining no call,
   and recovery through D. This still does not establish audible WebView or physical
   network acceptance. Only one physical desktop is currently available.

Native8KiB worker, bootstrap8KiB projection and physical12KiB Control guards must pass
on the assembled source. Root/Native/frontend suites and relevant build/flow/startup gates
remain mandatory before final handoff; only one heavy local Rust run may execute at once.
