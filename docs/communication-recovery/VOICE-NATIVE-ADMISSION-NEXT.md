# Native admission integration

The current code checkpoint (`6245af85`, draft PR81) adds native capacity Control30/version11, strict shared
browser codecs and retained capacity-only browser pairing.38 focused native tests and the
assembled Control graph guard pass.17 codec/frame tests,37 actual-WebCrypto pairing tests
and nine cadence units pass. Browser re-review has no remaining blocker/high/medium;
the complete Root/native/frontend suites pass4,002/1,049/3,635 tests respectively, with
25 existing Root ignores. Svelte check, frontend build, all ten Edge flows, strict
Root/native Clippy, explicit native check, formatting, ambient dependency checking and
the fresh dependency audit pass. Native build (3m44s) and actual startup also pass. Control29 stays
mode-only. Native ledger reservation, signed Ready/bilateral
admission, operational SDP/RTC/media composition and production activation are unfinished.

Earlier pushed checkpoint (`e14f3996`),28 October: Root registration and browser preclaim are reviewable leaves in
PRs79/80. They are joined at `a44b51b2` on `communication-v2-native-admission`.
The subsequent Actor staging implementation compiles; both new Root preparation tests
and all eight Actor staging regressions pass in that checkout. The complete Root suite now
passes4,002 tests with25 existing ignores. The complete native suite passes1,032 library
tests plus six ACL tests, with zero failures/ignores.
The subsequent native mode-only Control29/version10 fence has seven passing focused
tests. The canonical Slot continuation has thirteen passing regressions;
strict native Clippy and explicit check/build/startup pass. Production V2
remains OFF. This document describes implemented native custody and the remaining connection,
not completed native admission, signed readiness, media relay or physical acceptance.

## Next native ledger and agreement boundary (design only)

The next reviewed slice is an inert affine native parent-row claim, retained by the
SAME Slot alongside RootHighPreclaim. Reserve one of the existing32 positions without a
dummy selector, Pending row, work projection or signature. Pay the existing2304-byte Row
fee plus the SAME PreparedPhase2KiB contribution (4352 bytes). Every allocator, including
SDP children, must exclude reserved serial stamps. Eventual private upgraded consumption
moves the SAME serial/payment/preparation into the real Row; no refund/recharge or
deadline reset. Slot reclamation/emergency recovery/Returned/Closed must include actual
claim return. Measure Index against its1024-byte guard and all original native envelopes.
One parent claim does not promise descendant capacity.

Do not call the pilot producer to consume this claim: its legacy PreparedChain constructor
requests another group progression claim already held by RootHighPreclaim. RootHighPreclaim
has no operational consume API and reserves HIGH layout only; LOW requires actual Actor
orientation/evidence and its own correct backing. Existing signed Ready authenticates an
actual post-Finish transition. It is not a pre-agreement capability advertisement. Complete
bilateral binding, RoundRef/KDF domain, role-specific consume, immutable RoundGate,
Request/Grant/Commit, lost-Commit recovery and failed-round Forget remain required.
The separately charged browser8KiB remains a real deficit in the previously full
configured/audio composition; removing only the old2KiB lifecycle does not make it fit.
This design review authorizes neither an unspecified wire protocol nor activation.

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

## Canonical native slot (local implementation draft)

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

The current draft extends original-owner End, bootstrap finish/retire_channel, registry
collection and stop_and_wait_for_physical. Close first, retain exact cleanup routing, then collect work.
The first close fence must release UI commit before awaiting Actor work or destroying
Root capacity; otherwise the returning worker could wait on that same commit.

## Protocol pinning and browser coupling

The existing pilot starts automatic discovery after authenticated bootstrap Status ACK.
Pinning a new admission mode afterward would race that old producer. The implemented
mode-only Register occurs before ACK, followed by inert waiting. The local Slot draft can
start its same paid Root continuation after ACK; the current draft adds the capacity wire
adapter and the capacity-only browser pairing described below.
First mode Register after ACK must refuse an
upgrade. A valid first Cancel before ACK must select the SAME sticky admission mode,
so a delayed ACK cannot turn negative admission debt into pilot discovery. Observe
alone never creates a mode or ordinal. First Register/Cancel on an already ACKed
pilot Session refuses without changing mode or the cancellation floor. Exact retries
and subsequent permitted ordinals on an ALREADY pinned Session may occur after ACK;
the pre-ACK requirement applies to initial mode selection, not every generation.
Old clients that never select that mode retain the existing pilot behavior. Mode
selection is irreversible through cancellation, pressure, lost replies and epoch
changes. This mode-only boundary and both ACK orders now have focused native/real-TCP
coverage and read-only adversarial review. It accepts only ordinal1 until the canonical
Slot and actual return machinery below exist. This restriction applies to mode29 only;
distinct capacity30 now acknowledges canonical generations.

Header admission must select the new short Control grammar by operation and exact physical
Session logical ID BEFORE RowId decoding/counter admission. No renderer peer/role/profile
boolean substitutes for that native selector. Unknown versions fail closed without fallback.

The browser draft retains its original preclaim through ambiguous Register and fences
native cancellation BEFORE releasing paired browser capacity. Its immutable admission
constructor pins29 before ACK9, installs paired kind before publication/subscription and
rejects pilot conversion/work/lifecycle. An independent prepaid8KiB Session coordinator
owns one flight/ordinal and sticky remembered incarnation. Public retirement detaches its
cell; the private negative continuation joins exact native certificates AND actual
SDK/IO/task/cell/unsubscribe return. Broker ABI49 uses the SAME retained window symbol.
Unknown/cancel-failed timer registrations remain owned until actual return; inline or
invalid cadence cannot drive an outer poll loop. Clock faults close positive admission,
including before ACK, then retain negative debt with bounded/manual availability recovery.
The later already-full configured/audio composition still needs a measured solution.

Reserved is capacity only. It is NOT signed Ready, a native32-row ledger reservation,
DeviceId orientation, bilateral Request/Grant/Commit or RoundRef/KDF permission. Those
remain separate requirements before any production V2 activation.

The current candidate uses a DISTINCT Session Control30/version11 for canonical capacity
Register/Observe/Cancel, keeping mode-only operation29 unchanged. First capacity admission
requires an uncanceled mode pin and completed setup ACK. Mode29 Cancel closes negotiation
admission for that physical Session, including existing Slot attachment; it neither reports
capacity returned nor ends independently held media. Per-generation cancellation belongs
to operation30. The current uncommitted implementation and shared browser grammar are in
INTERFACES; the actual browser pairing is capacity-only and grants no signed readiness.

Actual Work retirement proves callback/argument teardown, not destruction of every Work
clone or return of its4KiB fee. The reviewed continuation uses ONE persistent8KiB task and
actual native position per physical Session, rather than reacquiring a task/position for
each generation. Native may advance only after its generation's owned Work/capacity are
detached and destroyed. Per-generation Returned does not claim task/position or fee return;
that stronger boundary is required at final physical channel collection. Future positive
Ready/SDP work must explicitly compose with this same owner; a second worker is not free.
Residual plain Actor payer
metadata can remain independently charged; neither a Returned reply nor retirement may
claim its refund. No residual payer may contain a Slot/Route/effect backedge. Acquire the
actual NativeHold outside Row, then validate that owned hold; checking global idle AFTER
acquisition would incorrectly block the producer on its own position.

Unexpected task destruction transfers its SAME native position into Slot when original
work remains. That Session has no Row backedge. The original plain8KiB funding remains;
terminal-only exclusive recovery creates no task/credit/Root attempt. It preserves the
position through actual owner destruction, including unwind, and only then permits final
collection. Shutdown must actively recover and await SAME Work retirement outside guards
before physical idle; maintenance has already stopped and cannot supply that cleanup.

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
