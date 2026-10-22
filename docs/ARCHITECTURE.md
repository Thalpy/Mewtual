# Mewtual; Architecture

## App-owned status bootstrap and session collection (21 October)

Source bac90cfa connects the retained browser channel to application-owned native bootstrap.
The host binds its paid loopback socket before Tauri creates windows and adds only that exact
numeric port to the existing CSP and devCSP. Origin selection follows Tauri custom-protocol
mode and validated main-window configuration, including packaged debug builds. Setup explicitly
enters Tauri's IO runtime; the Exit callback collects the actual task/socket before SDK cleanup
or restart. Bind/start failure disables this channel while the rest of the app can start.

The original IO state now retains a weak native action context. All nine references must
upgrade before any ledger/factory admission. Ordinary IPC and the upgraded context borrow the
same UI gate, including actual mounted-store presence, resumable-to-store lock order and the
post-await lock/generation check. Three existing policy fields now use shared Arc ownership;
no cached authorization snapshot or second commit mutex is introduced. The detached native
worker retains only its actual actor, original selection/grant/commit and paid work owners.

Weak references retain the original inline Arc allocations after their values drop. The fixed
listener payment remains32KiB, up8KiB from24KiB. The original weak-context Windows native
peak was32,256 bytes; retaining one fixed header allocation across stages reduces the current
measured peak to19,984 bytes, including context allocations/control blocks and wrappers. This spends more of the
same2MiB process cap, without changing group or engine limits. Context drops before the IO
credit; completed/uncollected native output retains that same original payment. Other platform
layouts still fail closed if their measured peak does not fit.

Factory admission now accepts the actual original SelectedHandoffOwner Arc, not owner/executor
strings from a request body. Pure early/final Coordinator predicates compare the already-cached
original allocation and current grant/lease/binding/generation. Refusal creates no local snapshot.
Actual channel closure must still be checked at final admission under the same UI commit before
any action opcode is exposed; this context alone enables no byte-bearing operation.

Three main-only commands expose fixed public setup and exact cleanup metadata. They carry no
private key, seed, plaintext, SDP, ICE or action constructor. Ready, Pending and Closed replies
share the original Row's serial projection guard and payment through body handoff. A capped
2,304-byte JSON writer and 2,048-byte hex scratch fit the original 8 KiB Row envelope; arbitrary
retained framework/WebView public-metadata copies after handoff are outside this copy bound.
No private/action data may use this exception. The retained production metadata adapter has
one immutable identity across component and module remounts.

The v9 browser broker retains its selected key, original TX/RX counters and constructor promise.
An original native 8 KiB Crypto credit stays paid through real browser setup/outgoing SDK work.
Separate original control capacity remains within the existing group/process/eight-Crypto and
two-browser-position limits. IO expiry aborts actual reads but never proves SDK/cancel drainage.

Explicit setup-failure and final channel collection first acquire the original shared UI commit
and check the action ledger using the actual retained selected owner. Every matching action
must already be compacted or explicitly closed as a native Pending before routing closes.
Open/Closing Pending or Live work returns Capacity with
Control still open, so receipt ACK, settlement and phase cleanup remain reachable. Factory
admission uses that same commit; the check and closure cannot race a new Pending factory.
Only then does final collection detach native Session/public/setup owners after browser SDK/IO
and buffers drain. Paid weak witnesses track independent headers, completed
replies and public outputs. A slot is reusable only when its original lease is permanently dead,
no setup is pending and every actual owner has drained. Serial IDs never reset. Housekeeping
runs outside Coordinator before new controller/owner/key admission, including late last-owner
drops after the collection reply. No uncertain history is evicted to make capacity.

Physical channel collection transfers an opaque native-only original-Row witness to the action
ledger, after releasing all bootstrap registry locks. It records original grant allocation,
executor and lease provenance without upgrading current authority. The separate bounded32-slot
sweep reclaims only actual compacted Body::Drained or explicitly ClosedPending records with
this proof, a permanently dead
lease and no other Row owner. Evidence persists through late metadata/cleanup handles; every
ordinary collection hook sweeps even without a new channel witness. Resources drop outside all
registry/ledger guards and both serial allocators remain monotonic. Root can independently retain
its own history, lower work and input owners; this collection is not proof of full Workflow
drainage. The inactive Pending closure adapter validates the actual opened Session, exact
header RowId and complete original purpose. It refuses a live worker, publishes Closing while
actual PreparedPhase/selected owners retire outside all guards, then records ClosedPending.
Begin cannot restart a Closing/Closed row. This separate local terminal leaves action/receipt
absent and engine.settled, acknowledged and drained FALSE: Root publication may be uncertain.
Indexed slot/serial/Row pointer are rechecked even for a retained native handle, so orphaned
unstarted records cannot be mutated after removal/reuse. The original physical request excludes
uncollected factory work/output. Real browser SDK/RPC drainage remains a protocol precondition;
the retained broker's authenticated Pending closure and production action dispatch remain open.

Action lookup now produces a native-only ResolvedRow only after checking the actual original
grant allocation, executor, lease and complete purpose. It retains the paid Row through delayed
metadata/cleanup use, without upgrading the grant for historical recovery. All raw ID-taking
operational helpers are module-private. Data admission tests the original lease under State;
this lookup is not current UI/channel/Root effect authority. Actual physical callers still
need to bound simultaneous handles and complete funding/drainage before serving action bytes.
An independent bounded original-purpose lookup also recovers this native handle when a lost
factory reply left the requester without a RowId. It checks the same native provenance and
works after End/grant destruction, without charges, Clock callbacks or factory mutation.
Missing history is not drainage evidence. Current-authority retry remains a separate path:
pure historical lookup must not prevent an idle Pending worker from resuming on its original row.

This service still handles authenticated Status only. Factory/action input, result, exact
receipt ACK, settlement, phase closure and final delivery remain open. Starting the local
listener does not enable V2 signalling, SDP/ICE/RTC or legacy-mode isolation. The action ledger's
separate finite history now has physical collection of Drained or explicitly ClosedPending
records; browser Pending closure remains open.
Relay and combined/physical acceptance remain
open. Earlier checkpoint sections below retain their historical scope; HANDOVER has current
gate provenance and platform limits.

## Internal immutable local-channel bootstrap (21 October)

The disabled ECDH candidate retains Pending before native RNG, validates the actual selected
browser P-256 point, and installs one Session under original UI-generation/selected-Arc checks.
An injective versioned transcript binds both points, device/join, original group/room/policy,
owner/executor and native session/slot. Exact retries share the original immutable paid public
output; no seed is exported and counters cannot be recreated. The64-slot paid index has no
uncertain-history eviction and still needs lifecycle collection before activation.

This candidate has no renderer command, application listener/CSP change, browser Session or live
dispatch. Its native-only computation credit does not pay for future browser crypto or prove
provider drainage. Actual IO/copy custody, setup ACK, action delivery, SDP/RTC, legacy isolation
and media relay remain separate work. Current full-gate provenance is in HANDOVER.md; historical
pending/pass descriptions below do not supersede it.

## Fixed native process infrastructure (21 October)

`Resources::charge_process_retained` prepays fixed listener/router/task overhead against the
existing2MiB process ceiling. Its distinct affine `ProcessRetainedCredit` creates no group
record, work slot or reservation and cannot convert into a group budget. Real group payloads
and bootstrap computation still require their ordinary group bytes and Crypto permits. The
actual last worker/output owner releases this payment; group retirement does not release it.
This primitive alone starts no listener and enables no V2 dispatch.

## Disabled framed handoff continuation (21 October)

The new native channel codec conserves a single paid physical position across header scratch,
authenticated body and reply/completed-task custody. Fresh injected RNG creates directional
keys; no renderer seed can reinstall a native session. Canonical framing and request-bound
responses are pinned against independent WebCrypto vectors. Pure browser metadata helpers
provide no keys, counter allocator, channel authentication or effect authority.

Control/setup payments are independent of the original action Phase. This lets fixed terminal
control replies remain possible after actual data drainage. Session/header authentication is
local transport authority only; every Root action still needs its actual native selected owner,
call, actor, member/room/epoch and original deadline. No listener/bootstrap/dispatch is enabled.
Actual factory registration and the native/browser result protocol remain under integration.

## Validated protected handoff capacity (21 October)

An actual protected CryptoAction lends one chain-wide HandoffLoan from its existing96KiB
Workflow. Retained copy capacity adds no Crypto slot; Clock sampling precedes Engine, followed
by pure original gates and the Workflow fence. Actual loan owners keep an ended Workflow
nonreplaceable. Fixed historical actions share the original paid PlanBinding, never Workflow.
The adapter must retain the original action and separately prove real copy/worker drainage.

The candidate browser reader follows actual BYOB transfers, caps a body at2064 bytes plus one
EOF byte and awaits cancellation/release. It is not a complete flight/copy budget or a live
transport. Focused/static review and a generated-data packaged Windows probe pass; complete
current-source Root suite passes 3,226; native passes 496 library plus five ACL tests, check,
executable build and actual startup. Full frontend (2,297), static/build and rendered
flows pass. No application listener/CSP/dispatch is enabled.

The disabled pilot's process-owned native ledger reserves row and phase metadata before
factory handoff. Pending/live/drained rows share 32 slots, without uncertain-history eviction.
Selected key snapshots keep Weak affine-grant identity; attachment still resolves the original
Action and complete pair binding. Actual native data owners contribute to one shared Phase.
Metadata-only terminal recovery and exact ACK/settlement need no new copy loan after closure.
Compaction removes strong Action/Phase/selected snapshot owners outside native guards. The
32-row lifetime limit is intentional for this disabled pilot. Actual Actor integration, physical
control/reply custody and authenticated browser drainage remain required before enablement.

## Selected native/browser key bootstrap (21 October)

One retained WebCrypto broker selects one nonextractable key for an actual accepted native
owner/executor. Native keeps a bounded paid generation with the original grant identity, join,
permit and frozen public point. Exact replies can be recovered; closed/refused metadata cannot
regenerate or adopt an earlier key. A typed foreign-executor refusal requires no computation or
settlement by that executor. Settlement-first permanently closes a selected generation.

Public-key publication uses the actual ServerActor endpoint API. Its native-owned task retains
the UI commit guard across the actor reply even if the IPC caller disappears. Exact settlement
waits for real publication drainage. Browser work shares the existing bounded queue; one retained
wake identity covers End/remount during failed replies without creating an immediate retry loop.
Private keys, confirmation plaintext, SDP and ICE do not use these three fixed-metadata commands.
This is not the byte-bearing action IPC channel or a stock-framework allocation bound.

Current native/frontend/check/build/startup/flow gates pass; the handover records exact counts
and unchanged-root provenance. Earlier pending sections below are historical. Normal V2/RTC,
protocol-mode isolation, member media relay and combined/physical acceptance remain open.

## Native production pair selection (21 October, full gates pending)

Root selects new pair evidence from the exact participant's implicit Watcher; the request binds
peer device, remote join/key reference and current epoch. A fixed receipt is metadata only.
Retained exact selector lookup precedes new discovery, preserving the independently pinned pair
through presence expiry/renewal/newer-join proof. Final current native call/session/room/member/
epoch/deadline checks remain required. Failed/retired records cannot recreate their allocator.
Cached recovery emits no new revision, preventing a driver's ensure loop from waking itself.

Transcript, original window, paid Pair/packet state and provisional origin pin are prepared before
final Group/Policy publication. An unpublished reservation revokes its actual pin outside final guards; successful
native publication moves both table owners and serial together. This uses existing 16-pair and
eight-confirming limits and no traffic allowance. Protected creation epoch is permanent; a future
driver needs explicit automatic join/phase replacement after epoch invalidation rather than
silently adopting old pair authority. No wire/persistence format, IPC or dispatch enablement.

The actual grant-validation path's source_context now samples Clock before nonblocking Group/
Policy, then rechecks the original native owner. A later busy snapshot must leave earlier source
jobs intact and the blocked packet unspent. Original Clock revocation and contention regressions
pass within426 focused Root voice tests, zero failures (22.49s). All10 actor tests, external API
test and2,268 frontend tests pass. Complete backend gates remain pending. The older frozen full
run was interrupted for this correction.

## Native actor controls and terminal voice retirement (21 October, full gates pending)

ServerActor now carries fixed-size, native-only voice controls through its existing bounded
mailbox. Root remains the only protocol state owner; commands retain original CallGrant,
CryptoAction and watch allocations. The opaque envelope exposes no serialized constructor,
plaintext, ciphertext, renderer projection or normal V2 dispatch. Mailbox capacity bounds
admitted commands, not waiting producers; the future IPC adapter must separately bound/coalesce
requests and account for every serialized copy. Actor loss is an uncertain result, not evidence
that a first effect did not happen. Retry only the original request and owners.

Terminal stop closes Root-local voice installation and queued command replies before its ACK,
even when the Closed event sink is full. Once an open stop waiter is admitted, losing its ACK
does not resume the actor. Already-canceled queued stops and canceled/failed saved freezes keep
the original installation. Successful freeze, explicit shutdown and sync-error terminal paths
retire before awaiting Closed. Shared Runtime groups and actual externally held engine/input/
transport custody remain intact until their owners drain; retirement never refunds that work.

Transport authority samples callback-bearing clocks/effects before its final pure route,
connection and room checks. A clock that retires the Root, replaces a connection or closes a
room cannot leave the pending forward authorized. This does not enable production IPC or RTC.

## Root derivation and final deadlines (21 October, focused checks passed)

Root now produces row zero and admits Derived metadata for its actual affine call, proven pair,
original actor/session and directory room. Input, tag and action construction precede the short
native fence. An outer unpublished History issue is borrowed into Group/Policy and try-History;
refusal destroys its owners after those guards and permits the same pair to retry. Cached factory
lookup returns the actual action allocation, including after ACK/input compaction; it cannot
rebuild browser keys. First results retain the existing preparation loan through settlement and
repeat original chain/deadline checks at the last native time observation. Root supplies the
same original Runtime clock to the pair, action and native origin.

First signed publication now respects the minimum construction/source/original SDP deadline.
A late confirmation output can have construction time remaining after the pair expired; it
must refuse before packet visibility or Finish promotion. Already-published lookup remains
historical and separately requires current source authority to send. No new allowance, wire,
persistence format, IPC command or live V2/RTC enablement is introduced by this increment.
All 409 focused voice tests pass, with unchanged layout guards. Full frontend, strict Clippy,
formatting, ambient and root dependency policy pass; complete Rust/native gates remain pending.

## Prepared native confirmation results (21 October, validation pending)

The next native increment splits sealing into Unsigned, private Prepared and source-visible
Published stages in the original outgoing row. Local refusal retains one exact inert signature;
changed ciphertext refuses before another signing call. All signing, hashing, copies, RNG,
pruning and replacement-owner construction precede Engine and short nonblocking Group/Policy
admission. An affine preparation loan retains the original admitted engine charge through actual
work, including settlement/unwind. Settlement fences first effects immediately; drained status
and permit/input release wait for the loan. No capacity or reusable permit is added.

Root seal/open consumers recognize actual call/action allocations and current native room,
participant and epoch. Final original actor/session checks follow the last Clock callback.
Consumed token/slot/body custody, replaced signatures/actions/attempts and older promotion cleanup
remain outside all final guards. Compatibility result APIs share the prepared packet seam.
All 389 focused voice tests and unchanged layout guards pass; complete gates remain pending; production IPC, normal V2/RTC, authorized SDP phase transfer,
member media relay and combined/physical acceptance remain open.

## Protected browser confirmation (21 October, integration pending)

The selected WebCrypto broker consumes protected native row projections after accepted derivation.
Rows1..9 map the three Hello/Proof/Finish candidates and local role; row0 remains derivation.
Twelve fixed claims and existing byte allowances are unchanged. A bounded private queue shares
one prepaid execution position through actual crypto/result/settlement drainage, retaining it
between flights. Retained input projections plus the maximum serial engine peak must fit before
copying. Owned TypedArrays cannot alias caller subclasses. Exact lost result replies retry the
same computed bytes without another AEAD/counter allocation. Historical admission replies are
metadata and cannot settle earlier work. Tests use real WebCrypto with a fault adapter; production
native IPC/Root/RTC and authorized confirmation-to-SDP key transfer remain open.

## Protected three-flight continuation (21 October, awaiting validation)

The continuation checkout generalizes protected intake to Hello/Proof/Finish and adds a native
response factory. Proof (low endpoint) and Finish (high endpoint) use three independent incoming
role positions, because their authenticated headers do not expose the echoed Hello nonce. A
separate `ProtectedInbox` owner permits bounded opening across attempt replacement; closed
plaintext must still match the current native nonces before any progression. This supersedes
arrival-attempt cancellation for these protected role inboxes; ordinary compatibility is unchanged.

High Proof output belongs to the accepted Hello intake lane (rows2/5/8); low Finish output belongs
to the accepted Proof intake lane (rows3/6/9). First production recognizes the actual retained
decrypt action and accepted terminal result, plus current native trigger/response/attempt. Exact
produced output history precedes those current-trigger checks and recovers its original action
without restoring authority. Unproduced superseded triggers refuse. No plaintext or Workflow is
retained in fixed response history. All original limits remain unchanged; their growth guards
pass in the 365-case focused run. Bounded follow-up links and each exact response pointer live
in their paid input rows, keeping shared metadata within the original 2 KiB allowance. Complete
combined gates remain pending. Normal kind28 dispatch, IPC/RTC and SDP transfer remain open.

## Protected outgoing Hello (21 October, pending validation)

The next worktree adds a native Root/local-join producer for the low side. Outgoing preparation
retains the original call/pair/chain/resource/table owners but publishes no sequence or SDP
identity. An exclusive ledger loan then participates in one local Group -> Policy -> try-History
transaction that publishes the packet, nonce/attempt, crypto action and candidate receipt. No
incoming ticket or origin allowance is manufactured for unsigned local work; signed source
admission remains separate. Exact predecessor IDs distinguish the first request and each of its
two possible successors, preventing lost replies from restarting confirmation.

Three canonical Hello plaintext hashes in the existing Pair allowance prevent A -> B -> A nonce
reuse. Protected high-side distinct IDs repeating an accepted nonce receive a closed rejection
without replacing current progress. Exact signed-ID/action retries still recover original history.

Low and high roles share three history positions, within the original fixed limits. Three pending
crypto leases independently observe packet, action and chain cancellation. Sealed source packets
retain chain cancellation after crypto settlement. Composite queued source checks sample every
constituent's original clock before pure owner checks; deadlines are not moved between monotonic
origins. A busy effect skips its untouched packet without discarding earlier prepared jobs.

Read-only adversarial review is clear and all 352 focused voice tests pass, including the
unchanged layout guards. Complete combined gates remain pending. Proof/Finish, SDP transfer,
production IPC/browser/RTC integration and
normal kind28 dispatch remain open. The following sections record earlier checkpoints.

## Confirmation packet tables (20 October, inactive)

Incoming and outgoing native V2 packets now each retain a position from their independent
32-row pools. Preparing a confirmation workflow protects three candidate lanes within those
same limits: low-side 3 input/6 output, high-side 6 input/3 output. Fixed action history does not
retain these progress claims. Workflow close returns unused positions; actual rows/loans retain
their own positions until retirement, independently of retained decrypt-event bytes. The first
protected native consumer is high-side Hello below. Live V2 dispatch remains disabled.

## Protected native Hello admission (20 October, inactive transport)

High-side `ConfirmHello` now has a private Root intake consumer. Actual packet/token, decrypt
event, inactive action and credited admission history are prepared before a single nonblocking
Group -> Policy -> try-History publication. Clock samples precede the guards; contention refuses
without origin quota, replay, token or candidate effects. Unpublished owners remain outside the
callback so their destruction cannot reenter History while Group/Policy are held.

Three candidate lanes remain spent after malformed decrypted results, independently of accepted
confirmation nonces. A shared Root public/encrypted issuer/MessageId/hash classifier precedes
token preparation; exact retries return original admission history without creating another
decrypt. The fixed receipt owns original room/call/pair/chain/member/effect custody, without an
action, input, engine or Workflow. Original Root reply-route authority independently fences epoch
and connection changes. Consumed tokens and successful confirmation do not erase admission ACKs.
The receipt and its actual effect allocation share the original 1,536-byte action row; the fixed
metadata remains 2 KiB. These structural guards exclude allocator slack and framework copies.

Protected mode is selected only for a pristine native pair and stays selected after a capacity
refusal. Ordinary confirmation factories refuse selected pairs. First confirmation results check
the original epoch before effects; exact accepted/rejected terminal history remains recoverable.
At this checkpoint low-side start/encryption, Proof/Finish, SDP phase transfer, production IPC and RTC remained open.
No normal kind28 dispatch, browser protected confirmation or whole-chain readiness is enabled.

## Chat history and MLS epoch separation (20 October, continuation)

Chat retains the original signed Automerge operations across key epochs; current-key catch-up
merges that history rather than replacing a branch. Edits/deletes change the live projection while
their earlier content remains retained. Causality, not wall-clock timestamps, controls these
operations. The continuation corrects all-visible-list message addressing and failed-page cursor
commit ordering, and binds modern 32-byte writer actors to their verified signer at ingress.
Historical actor compatibility and the existing R6/group-binding/equivocation limits remain.
The [chat history trace](communication-recovery/CHAT-HISTORY-CONVERGENCE.md) distinguishes those
implemented boundaries from the stronger authenticated/moderator-only audit design still needed.

## Protected browser derivation (19 October, inactive)

The surviving browser broker v5 shares one closed derive/confirmation action driver. Protected
derivation uses an opaque broker-generated join key, its exact accepted native join projection,
and a frozen native pair setup. Its inactive plan protects twelve shared row claims (24 KiB),
96 KiB variable custody, 8 KiB control metadata and one actual execution position before publication.
Only derivation row0 has a consumer. Existing published records plus unused claims remain capped
at32; ordinary queued work cannot occupy the protected position. These are conservative bounded
payload/layout allowances, not a measurement or bound on WebCrypto/JavaScript implementation heap.

Lost admission/commit replies reuse the original action and computation. Exact native terminal
history without matching locally retained keys returns a historical receipt with no usable pair.
The empty Derived marker never causes ECDH/HKDF reconstruction. Protected keys cannot enter the
ordinary encrypt/open/confirmation APIs; their protected consumers are still unimplemented.
Close clears key references and unused capacity, while real action and RPC promises retain their
position and variable custody until drainage. Fixed history and closed pair stamps remain separate.
The original browser-request deadline closes an idle plan; native monotonic timestamps are not
compared with browser time. Production native registration/IPC and live V2 remain disabled.

## Prepared confirmation token binding (19 October, transport disabled)

LocalJoin binds an incoming confirmation's actual Hello floor or attempt and original pair
deadline into the affine preparation before replay/token publication. The same binding follows
the final origin checks and retained token. Generic application admission refuses all three
confirmation kinds without that binding. This grants no engine/action or continuation readiness;
the larger native/browser reservation and atomic intake boundaries remain open.

## V2 action input retirement (19 October, transport disabled)

Exchange-backed actions prepare fixed history bytes independently from variable input. Only
input and an actual engine loan retain the exchange's phase owner; the pending engine lender
is cleared permanently under the engine fence on settlement. A direction permit returns only
after unpublished abandonment and is consumed at publication, so compaction never permits a
second seal/open identity. Unstarted retirement also drops unused input; settlement without a
terminal result drops input because the same fence permanently excludes a later first result.
Installed terminal results still require their exact ACK. Borrowed readers retain actual bytes.

Typed crypto actions split fixed history capacity from owned variable input capacity without
changing the total pool charge. Native input readers hold an opaque Arc-backed lease. Under the
original engine fence, exact receipt acknowledgement and actual settlement can remove the
action's input reference; actual readers retain their buffers and credit through last drop.
Engine-to-input is the sole nested lock order, and buffer/credit destruction runs outside both
locks. The fixed action/history row remains through the join; this does not free protective IDs,
complete the engine-to-RTC phase transfer or account for Tauri/browser serialization.

## Private durable Download ownership (19 October, implementation in progress)

`BlobStore` optionally exposes the native `DownloadStore` interface. `ProtectedBlobs` forwards
that capability to the same encrypted retained engine; cache-only stores refuse it. Immutable
Keep(CID) and Download(random saved ID) owners share reservations and persistence machinery,
but have distinct paths, run tokens, public-serving eligibility and cleanup. The actor's
`SavedRun` dispatch preserves purpose through each local/network/fallback/terminal effect.
Native Save acquires/reverifies the whole encrypted copy before plaintext export. `DownloadReady`
observes the ORIGINAL actor generation, Root and mounted storage generations. Native account/
registry-row custody stays separate. Four whole-export slots follow queued plaintext replies and
remain distinct from eight process-wide acquisition jobs/attempts. Renderer metadata exposes no
saved manifests, keys, external destination authority or resource grants. Resume is explicit;
there is no startup plaintext publication or persisted external path. See design-file-reliability.

## Native MLS source catalog and explicit requester (19 October)

`ServerGroup::member_count` now retains a fixed native observation of its committed epoch:
member count, last occupied leaf and whether both copied OpenMLS key vectors have the supported
32-byte shapes. Create, join and restore each have a fresh opaque instance identity. The cache is
not serialized; a committed epoch change makes it unavailable until ordinary native observation.
Staging/aborting a commit does not change the committed roster. Native loading and the first
O(roster) observation remain outside the bounded request-page claim.

An already observed catalog proposes at most four sorted distinct public signing keys while
examining at most64 raw leaf positions, including holes and requester/helper exclusions. Before
OpenMLS's indexed copying accessor, it checks the exact instance/epoch and attested key lengths,
then checks the borrowed credential against4KiB. Empty incomplete pages mean yield, never absence.
Catalogs expose public tree metadata on inactive groups too; they grant no active membership,
Root, native session, file intent, connection or execution authority.

The sync owner's native requester search uses these continuing pages, with independent
positions for at most four helpers by four exact encrypted variants. A nonempty proposed page
stays unconsumed across local pre-admission refusal. Helper replacement cannot reset its cursor;
empty pages advance one bounded local turn. There is no permanent320/512-member source cutoff.
The app's explicit native chunk/range and Keep reads now fall back to this search after direct
providers. The captured native invocation and Root file authority remain mandatory. Root prepares
an exact signed kind29 grant and reserves route custody before the actor's durable Keep debit;
only subsequent commitment consumes that proposal. A helper forwards kind30 to at most four
named sources, without gaining file keys or retaining their ciphertext. Each bridge page stays
at64KiB and has its original12-second Root deadline, independently of B/A's service clocks.
Signed Unavailable observations schedule at most two later helper/variant sweeps after fresh
coverage. Ambiguous transport failures retry the same batch with fresh nonces and paid work.
Stable-helper local pacing is a scheduling hint, not a guarantee about remote arrival rates.
Finite read and Keep allowance can expire before complete coverage; this never proves absence.
Four focused TCP/native end-to-end file scenarios pass; full programme and physical network
acceptance remain separate.

## Connected file source checkpoint (19 October)

The native server actor installs an observing file-service attachment from its exact account/
server-entry scope and current file actor generation. Kind30 is routed into a retained Root
completion queue before any general request await. Authentication, current member/route/policy
checks and signing stay on Root; only an affine bounded storage plan enters the blocking native
executor. The source credit follows worker, pending metadata and actual response independently.
File completions rotate as a fourth input in the existing owner selector. Final Present effects
recheck captured scope, storage generation and the original two-second read deadline, excluding
the reply wait that retires before a driver send. Source tests alone do not establish complete
indirect Keep acceptance. Helper29 uses the same retained
queue for one-hop source requests: its affine chain prepays all four lower attempts and five
route records before admission. Original lower bytes, output bytes and actual driver sends own
the same charge independently, including final-refusal paths. The selected A scope survives
through final Found admission; unused sources do not retire the parent C scope.

## Congested network dispatch (PR 48 revision)

The driver owns bounded application payload and lifecycle reconciliation queues; it never awaits
the application sink from a swarm callback. Actual payload/response/lower ownership carries a
shared process byte credit, while dropped gossip requests ordinary anti-entropy. Automatic cached
dials and reciprocal/probe forwarding prepare immutable exact authority on Root and wait in
bounded detached jobs. Driver admission repeats ownership; completions return to the sole Root
for current retry/accounting effects. Notifications have local ACKs and driver-discarded replies.
See [the interface contract](INTERFACES.md#congested-transport-delivery-and-automatic-repair-submissions-pr-48-revision)
for precise bounds, generation ordering and the still-immediate reconnect/manual paths.

Document anti-entropy also has an owned monotonic 30-second wake in the sync network wait.
It revisits quiet connected sources after a completed search without requiring UI activity,
gossip or a discovery edge. Caller cancellation preserves the deadline and pending requests.
Only active local members with open documents and an actually connected bound current source
arm it; one bounded due turn precedes normal fair input rotation. Queue, cooldown and source
rotation bounds remain unchanged. See [the message trace](MESSAGE-FLOW.md).

Automatic delivery receipts now share detached notification ownership with repair forwarding.
One persistent admission turn alternates successful classes, capped at four new notifications per
owner turn; capacity denial does not skip the owed class. Local ACK waits participate in owned
response/event/submission polling. Neither scheduling nor an ACK is remote delivery evidence.

## Connected discovery maintenance (2026-10-18)

The server actor queues connected-only membership-commit, PEX, member-finalization and
switchboard-offer stages. Quiet discovery first polls the released commit catch-up protocol at
its current epoch: a lost removal broadcast must not depend on future chat to expose the gap.
Empty/failed probes still continue ordinary discovery and do not establish currency or PEX
failure. Applied epochs publish member/role consequences, including equal-count replacement,
and request the existing sealed snapshot worker before subsequent stages prepare fresh authority.
Each stage prepares its authenticated request on the sole sync owner, waits in a detached worker,
then commits only against that originating owner's current epoch, effective policy, descriptor
generation and live connection. The networking generation is independent of UI lock and V2 call
sessions. Legacy PEX remains unbound member evidence; finalization retains its stronger policy
and both-endpoint transcript. Production transport connection leases revoke at aggregate close,
before the sync owner consumes any event; same-peer reconnect cannot revive captured work. Old
disconnect events cannot revoke a replacement lease. Unsupported legacy test transports retain
event-driven fencing; upgraded voice refuses a transport without a live driver lease.
Released wire encodings and public synchronous-owner async wrappers
are unchanged. Explicit finalization commands use this same nonblocking actor scheduler.

Historical dual-key bindings remain conflict pins across a disconnect. They do not establish
that a restarted member remembers our identity. A separate session-only table (at most320 rows)
records verified outbound kind26 completion for the original connection lease, member/device,
epoch and policy. Same-peer reconnect owes a fresh exchange; a delayed old disconnect cannot
erase completion on a replacement lease. Serving alone never settles our outbound obligation.
Known current members on inbound-only connections are eligible without outbound dial evidence;
unknown Noise endpoints are not. Native capture selects at most two fair repair targets per pass,
through bounded connected-only maintenance independently of PEX backoff. Both endpoint orders
pace the second attempt past the one-second serving rate limit. Only saving listener hints still
requires actual successful outbound Noise observations; no repair query grants dial authority.
Unsupported legacy transports retain event-fenced readiness rather than exact driver generations.

Actual aggregate loss of a previously endpoint-bound current P2P member also sets one coalesced
actor scheduling hint. Driver-live stale disconnect events do not clear replacement liveness.
The actor owns two extra local-route retry opportunities, normally at +5s and +20s, inside one
25-second window and a 60-second re-arm cooldown. Further losses do not extend/refill that window.
Its absolute deadline participates in the existing fair timer source. Each due turn rechecks
member-mesh eligibility and uses the existing sealed-route, current-roster and shared endpoint
scheduler gates; unavailable budget stays unavailable. No file invocation grants a dial.
A 250ms bounded local driver-acknowledgement wait prevents an indefinite owner stall. Unqueued
permits may drop; an already queued driver command retains its actual permits and may complete
once after that wait ends. No ambiguous submission is refunded. This starts no PEX, rendezvous,
interface refresh or snapshot work; the native 45–75-second cadence remains the fallback.

Two actual lower flights/server and four/process bound retained answers to 32 MiB/server and
64 MiB/process for the released 16 MiB commit response; PEX remains capped at 512 KiB. Existing
bounded signature/record decoding scratch is additional. Commit bundles have a separate 1024-
record work ceiling (default retained log: 256); larger local logs serve successive epoch polls.
A flight and its per-target occupancy survive timeout,
worker cancellation and queued completion through the transport's actual last keepalive owner.
At most sixteen peer chains and four explicit waiters/chain coalesce discovery; one bounded refresh
handles state supersession without charging peer-health backoff. Queued actors wake on actual shared
capacity release, while idle actors ignore unrelated releases. A sixth fair ready source consumes
completions. Accepted raw signed-record changes request the existing core snapshot worker even in
quiet legacy groups; native restore rebuilds its passive address cache from those saved records.
Connectivity/duplicate records alone do not dirty that revision. Automatic cached dials and mesh
repair now use the detached submission path above. Local reconnect, explicit manual redial and
the bounded rendezvous drain retain separate waits; this is not a universal latency bound.

## Voice V2 foundation in progress (2026-10-18)

An inactive native confirmation plan now protects twelve fixed local history rows/slots and a
provisional 96 KiB variable envelope, one Crypto lane and two Forward lanes. At this checkpoint,
only derivation consumed a protected row. One weak process/group Workflow index remains occupied through
actual input/engine/lower custody after End; fixed history and retained closed control metadata
stay independently charged without pinning progress. LocalJoin permanently stamps successful
preparation per pair, including after close. Original resources/slots, scope/epoch, both join/key
identities, pair pin/generation, role and deadline bind the plan. Fresh native authority is still
required at consumption and first effects. Process housekeeping closes unused capacity outside
resource and action locks. This is not whole-chain/native/browser readiness or live dispatch.

The reviewed [V2 decision](communication-recovery/VOICE-SIGNALLING-V2.md) is being implemented
as separate foundation modules; live calls still use legacy signalling. Strict signed objects
and route roles live in `catcoms-sync::voice_v2`;
the standalone browser crypto broker uses non-extractable P-256/HKDF/AES-GCM keys. Native call
ownership is distinct from optional mappings. A unique allocator claim on the shared native lease
prevents packet-ledger recreation, and dropping that allocator revokes the join. Affine native
resource credits survive uncertain engine work and share a process/group byte pool with packet
caches. Native `Watcher`/`LocalJoin` now implement separate fresh observation/challenge proofs and
immutable active-join renewal. Observations retain exact charges and revoke on replacement or
membership loss; actor-owned origin limits keep key/renewal protection across watcher replacement.
`LocalJoin` now owns initial three-flight confirmation, exact action/reservation Submit and
atomic newer-pair promotion. Packet commit produces an opaque affine signed-flight receipt rather
than a forgeable bare header. Capacity/engine failure preserves validated pending actions without
resetting counters; roster observation refresh does not retire native pair authority.
The desktop now shares one process Runtime/resource pool with attached product actors. Helper
owners retain group limits/caches across actor replacement and
capture irreversible session, actor, policy, member and directory-incarnation gates for deferred
work. Transport RequestAuthority checks those gates synchronously at lower admission; lower
keepalive ownership still survives waiter cancellation. V2 time advances with local monotonic
elapsed time as well as forward wall observations, so rollback cannot outlive pruned protection.
Helper flights share four group/eight process slots and the same declared-byte pool.
An inert kind28 admission seam now enforces bound Noise identity, pre-signature work/byte budgets,
exact frame tickets and role-bound typed ACKs. Prepared replies now retain original verification,
first packet-admission and route custody through the driver's actual serialization/send. A cached
helper packet cannot adopt replacement actor/room authority or a later longer permit to issue a
positive ACK; protective identity and spend remain retained. Detached source/helper requests capture actual driver
connection generations plus Root descriptor/proof/epoch owners, and retain lower flight custody.
Source attempts preserve exact native cached packets/pairs/actions and share the helper flight
pool, with separate typed settlement. Normal request dispatch, production source retry scheduling
and native endpoint/UI delivery remain disabled. Root now owns bounded Watcher/LocalJoin lifecycle
state and observes exact authenticated removals inside the current runtime membership transaction,
before fallible additions or any await. It keeps weak call-grant identity, so losing the last native
owner closes queued source activity. Watcher grants bind native controller/call parent consent before
cloning; abandoned replies revoke observation. A coalesced metadata channel avoids awaited event
sink backpressure. The trusted public-control intake now commits original allowance and bounded
queue custody atomically; its pump derives challenges/proofs from actual native owners, and only
fresh native challenge responses admit presence. Encrypted intake and recoverable payload/
confirmation/negotiation pull remain next. Process and actor Clock housekeeping expire cold caches
while locked without erasing membership, replay or allocator protection. Actual transport/payload
owners retain their credits, and unresolved browser operations have no expiry refund. Confirmation
source work captures the original handshake deadline; exact retries use immutable reservation time.
Native entry custody revokes the exact actor on removal. Lock publishes a newer UI generation
before revocation hooks; IPC requires the exact generation last committed by authenticated unlock.
Unlock prepares fresh retained-actor owners outside native commit, then registers each exact row
before activating the captured session ticket. Optional voice capacity or a stopped actor leaves
that row without V2 authority and does not prevent chat/vault access. Duplicate unlock retains the
same installed owner. Root directory observation sees newly authenticated deletions even within a
compound change or prefix-applying error; duplicate/rejected operations grant no new revocation.
The bounded reader always includes the permanent `general` room. Affine native call grants revoke
on abandoned replies; production activation validates their exact scope and live child lease.
Production grants also retain the exact room allocation and a 256-byte affine charge through
actual last-owner drop; deletion/revocation alone cannot refund retained native work.
No call keys or replayable call state are persisted. A standalone confirmation-only browser
registry retains one computation through lost admission/commit replies under the surviving
broker's combined byte/record/engine limits. It returns closed historical receipts without
plaintext or fresh RTC/pair authority. Owner retirement wakes observers but does not refund
actual pending work. An inert native typed-action queue now gives proven pairs and exact
confirmation inputs continuing identities, immutable charged custody and one shared engine fence.
The desktop registration/admit/settle model now compiles only for contract tests until its
result owner and browser IPC adapter exist. It is not production native enforcement; the sync
typed-action queue remains an inert production Rust seam. That model's registration matches
the actual call lease, scope and process pool. Exact admission is
idempotent, settlement permanently closes delayed admission, and first signing/confirmation
effects use that same fence. End closes unstarted actions but leaves actual admitted work charged.
The combined 32-record queue includes terminal identities; pulls rotate through at most four
without dequeueing. Prepaid capacity can close unused bytes while retaining actual outstanding
loan bytes and slots. The narrow SDP foundation now requires a 224 KiB native exchange reserve
before the first local nonce or incoming positive admission. It prepays one Crypto lane, two
Forward lanes, two action continuation slots and member records for the endpoints and at most
two fixed helpers. One local and one remote SDP of at most 8 KiB share one OfferId and fixed
initiator/responder role; crossed Offers refuse before admission. Packet, Source, verification,
engine/action/result capacity loans retain actual custody. This is capacity enforcement, not
completed SDP crypto-action factories or browser result delivery. Full-chain integrated progress
remains a requirement. The inert native CryptoAction seam now retains small exact accepted and
closed-rejected terminal receipts under the original engine fence, independently of consumed
tokens and actual settlement. SHA-256 is recomputed over the native-input-selected bounded
output; cached history uses the original clock and post-transition pair/attempt owners. History
never grants fresh signalling or RTC authority. Live Endpoint confirmation/presence wiring, native action IPC adapters, helper ingress
and UI V2/legacy separation are not yet enabled or accepted;
the [communication ledger](communication-recovery/CHAT-FILES-VOICE-ACCEPTANCE.md) records the scope.

## Call lifecycle and optional router custody (2026-10-18)

Call RTC callbacks and asynchronous SDP/stats/router continuations require the exact peer object
and call-join lease. Component destruction explicitly leaves the old call, stops captured tracks
and closes peers, including retained-WebView remounts. Native optional router mappings have a
separate process-owned controller/call/grant coordinator: non-destructive prepare, revision-CAS
activation, actor/group/UI-generation binding, exclusive port slots and detached acquisition/
cleanup. The UI never owns a router effect merely by awaiting its command. Lock/leave/remount
revoke ownership; uncertain cleanup stays charged until process exit. IGD cleanup addresses its
original granting gateway. See [custody and limits](communication-recovery/CALL-MAPPINGS.md).
These corrections do not add indirect voice signalling or media encryption; those remain in the
separately tracked [communication programme](communication-recovery/CHAT-FILES-VOICE-ACCEPTANCE.md).

## Studio working edits and five-second saves (2026-10-18)

Production Studio edits update a separate volatile working projection immediately. Native views
and raw registers remain the sole authority/evidence; local frame handles and display sources
never become native CIDs, recovery choices or authoring authority. Ordinary pixels, timeline,
sound, Score, header and Index operations wait for Save, normal close/navigation, or a fixed
five-second deadline measured from the first change. Document creation and explicit historical
recovery are themselves Save actions. Only never-submitted tail edits coalesce. A released Save
contains a fixed set of queue identities; later edits wait for their own deadline. Failed and
uncertain records remain visible and never automatically retry. First delivery preserves its
admitted epoch/trust route across publication/ticket awaits; only explicit reauthor changes it.

Normal document, channel, server, surface and window changes await definite Save receipts,
including a new interrupted stroke drawn while the first revision was saving. Refusal or timeout
keeps the old editor. Window close holds routes even after its Save drains, and every awaited navigation
decision is rechecked in the continuation that changes the screen. Delayed wiki rename results
also bind an exact displayed-page generation, so a same-name reopen or another server's page
cannot become the stale write's target. Playback/export may use complete validated owned working
inputs without writing; exact native group/channel/epoch and linked-Score checks still apply. Semantic media
generation ignores delivery status and an ACK of identical content, so autosave does not stop
playback. Data notices are re-read before content comparison; settlement, explicit invalidation,
membership and lock still revoke playback ownership. A new editor Play begins at frame one.

Orderly lock captures the owned queue in the existing atomic encrypted UI continuity envelope,
then immediately revokes native authority and clears editor models, buffers and rasters. The
existing lock coordinator retains the immutable continuity argument until unlock so a racing
close/retry can reuse its exact bytes; this is not a RAM-zeroization guarantee. Capture failure cannot
prevent lock and is disclosed; the previous sealed record remains intact. Restored work is held
until explicit Save/retry in its original server/channel/group, never automatically replayed by
a new edit. Limits are 512 queued records, 8 MiB of PIX file bytes and 12 MiB serialized Studio data;
admission reserves frozen-request/diagnostic overhead and counts UTF-8 plus outer JSON escaping.
The sealed envelope is capped at 16 MiB with the existing independent 1 MiB allowance for all
non-Studio continuity. Physical reads are bounded before decryption. This is orderly lock/close
recovery, not a write-ahead log or a claim of receipt recovery after a hard process/OS crash.
Unlock restores that queue with no document open. Opening a document is never conditional on its
held work saving first (a document switch saves only the document left), and the sidebar shows a
row with an open button for documents holding unsaved edits (up to eight, then a count; an Index
edit about one entry counts for that document, on every route; pending Creates keep their own
rows), so their Save, Retry and discard controls stay
reachable even for a document the Index does not list. Once open, those controls render whatever
the document read did: a failing read shows its error beside them, never instead of them.

## Studio Save, navigation and playback corrections (2026-10-18)

Native document refreshes and Saves use separate latest-request lanes. A Save revokes older
projection/Save results; a later read cannot hide its durable receipt. Session lock, cancellation
and actor-incarnation checks still apply at delivery. Actual read-admission counters separately
fence an ACK's projection: an overlapping later read preserves its newer state, and reconciliation
finishes before dependent queued work runs. Navigation without a read never suppresses Create's
initial projection. Editor selection flushes the original
object/frame's dirty raster before replacing the document; a synchronous enqueue refusal vetoes
the switch. Pending exact Saves retain their pixels and identity offscreen. Protocol Open phase
and bounded maintenance watches are independent of the one currently open editor.

Looping pictures wrap through long authored sound/release tails without shortening sound. Shared
archival decks render and announce the DJ's actual synth clock after preparation; listeners sample
their room seek after preparation as well as download.

The shared sound shelf retains only sixteen currently eligible recipes. Native reads enforce the
4 KiB patch cap, serialize replacement work and cancel after registration. Scope/listing generations
and direct post-await trust, lock, readiness, close and membership checks reject stale results.

Accepted Closing retries authenticate their scoped intent record and flush its existing ciphertext
before acknowledging it. They neither rewrite metadata nor require an auxiliary owner journal.
Fresh authoring and commits retain complete inventory/admission. Automatic receive supports up to
1,024 cumulative metadata records, matching its directory ceiling, with the existing 8 MiB read and
256 KiB cold-validation rails. Resumable inventory turns retain their deadline; legacy synchronous
complete scans still have no measured custody-latency guarantee.

## Studio UX and ordered sync observations (2026-10-18)

Studio keeps Art, Sound, Music and History separate. Server settings owns fault inspection and
explicit owner decisions through an independent typed target wrapper; it never resets the editor's
shared session or manufactures a winning receipt. Normal owner maintenance and already chosen
repair work continue automatically through the existing actor.

Chat/wiki and file details share one App-owned inline PIXA manager: one serial native read,
two retained encoded packages, 128 observed hosts, one repaint timer and only current rasters.
Pictures loop silently under the existing file trust preference; sound requires a local click and
uses the existing StudioAudioSession/JamEngine with token-owned cleanup. Membership, navigation,
lock, visibility and trust edges retire reads/packages/consent. A retained same-channel host may
renew its scope only on a fresh explicit Play. Package arrival uses the existing chat media-load
capture listener; individual frame paints do not trigger scroll work.

Post to chat exports the displayed composition, streams it through ordinary fileshare upload,
then appends its returned file CID to the existing draft and opens chat. Ordinary Send retains
durable retry ownership. Navigation (including A → B → A), membership, lock and window-close
intent revoke the upload's permission to edit the composer, although a completed file may remain
in Files. This does not create a Studio history reference or claim remote custody.

Receiver pause/resume observations now carry actor-local sequence, bounded actual failure detail
and native server incarnation. Successful whole explicit access emits current state; a failed
access and read-only fault controls cannot resume receiving. StudioSession orders these lossless
u64 labels and caches 128 recent local server observations across navigation, clearing them on
lock/session reset. A cold/evicted scope refreshes Index to obtain current native state. This is
diagnostic state only; no wire, persistence, authoring or settlement authority changes.

## Gate 7 UI completion candidate (2026-10-17)

PIXA now extends the existing Jukebox archival branch and global file details viewer, using the
same JamEngine, call/DJ transport, authenticated file fetch, cancellation debt and local consent.
Encoded package caches retain at most two per call; a canvas expands only the current frame.
The global viewer owns one package, a separate cancellable read generation and explicit Play.

Explicit document/Registry fault repair delegates to the existing detached durable-owner path.
Separate Closing local-draft commands derive the saved signed close from the local sealed owner
journal. Their failed lookup is carried to new-authoring admission, after accepted/terminal retry
classification. A captured plan retains the exact verified close for the unchanged S3 rechecks.
Fresh Closing authoring is generally available only on the journal-holding device; it remains
local-only pending automatic handoff. No second synth, document owner, network protocol or
persistence format is introduced. Current contracts are in [GATE7-UI-HANDOFF](GATE7-UI-HANDOFF.md).

## Gate 7 baseline (2026-10-17)

The owner accepted Gate 6; PR #44 merged at `82631a81`. Gate 7 adds composed lifecycle
acceptance and the [production UI handoff](GATE7-UI-HANDOFF.md), reusing the same actor, vault,
receiver, transport and JamEngine. No wire, persistence or authorization contract changes.
[GATE7-ACCEPTANCE](GATE7-ACCEPTANCE.md) separates adapter/TCP evidence from desktop/audio
release observations. Dated Gate 6 checkpoints below remain historical.

## Gate 6 composition checkpoint (2026-10-14; candidate at that date)

The existing Studio/P1/vault/native/renderer stack now composes typed Score and complete Flipnote
sound state with production Score/Sound/Music controls, actual-byte PIXA publication/readback,
durable export references and local visual-only GIF export. The existing JamEngine/allocator also
serves Studio and local/shared `.jamtake` playback through opaque archival capabilities; no second
synth, media transport or document owner was added. Live raw notes retain the 30s watchdog, while
archival indices carry bounded authored remaining holds and drum deadlines.

Explicit Flipnote SoundRecord recovery is implemented and bounded-reviewed at `1d888287`, with
frontend selectors/custody/retry plumbing reviewed at `cefe0825`. Whole-version recovery walks
select one effective value/removal per slot, not every raw alternative or a preserved branch.
Score historical OBJECT edits stay manual, Unconfirmed drafts stay local/non-transferable,
header-only Index Copy stays projection-level, and direct Closing Save/unfinished native repair
commands remain unavailable. Gate 4's bounded lifecycle P5 promotion is unchanged.

[GATE6-ACCEPTANCE](GATE6-ACCEPTANCE.md) pins the stack, clear cumulative review and passing complete
local suites/startup evidence; hosted/Linux CI remains pending. Older first-slice sections describe their named checkpoints,
not current missing UI/native activation. Gate 7 release/two-real-client acceptance, broader
emoji catalog/chat clicks and shared-deck PIXA consensus/global viewer remain later work.

## Gate 6 native exports (2026-10-14)

Native `publish_pixa` now validates complete actual bytes and reuses the existing persistent
blob stage/promotion/flush path, without writing Studio metadata. Local `export_studio_gif`
reuses the pure-Rust image encoder, bounded detached work, inline leases and guarded safe
Downloads publication. It is visual-only, fixed-name, infinite-loop output, not replicated
audio or synchronization. Command registration, policy/capabilities and review ledger move
together. SetExport validation/custody and production UI integration are now connected in the
candidate; combined local verification passes, while hosted evidence remains pending.
See `GATE6-NATIVE-EXPORT.md` and the ledger.

## Gate 6 Flipnote sound core checkpoint (2026-10-14)

The typed Flipnote projection now retains SFX, reusable recipes, linked-score provenance and
exports through the existing P1 causal/checkpoint/recovery path. Art-only bytes stay unchanged;
an optional canonical sound trailer retains complete source/body records. Effective removal
and visible prefixes are separate from retained evidence and conservative blob holds. Native
DTO/UI activation was not implied by that core checkpoint. The native sound omission has since
been closed by the shared fallible conflict-preserving converter and ordinary/provisional tests;
Production UI composition and explicit SoundRecord recovery are separately bounded-reviewed;
final combined verification remains open. See `GATE6-FRAME-SOUND.md` for the exact contract and review scope. Older
art-only/refusal statements below describe their earlier checkpoints.

## Exact device admin authority (2026-10-13)

Current moderation-record roles also use the verified full signing-device identity.
Legacy origin labels supply attribution only, without inheriting an origin's admin
or owner authority. Historical-role proof and append-only audit completeness remain open.

After a failed role save, the owner must retry a valid role change to complete
the save and unblock delegation; a background snapshot alone does not acknowledge it.
Completed admission recovery remains nonce/KeyPackage-bound, not original-inviter-bound.

The designated owner admits independent invitees using exact current-device grants.
Four-byte fingerprints remain display labels. Companion certification adds membership,
not admin authority. Owner-local grants are rechecked synchronously immediately before
fresh MLS Add, including queued requests. Observed removals prune exact grants and
owner changes clear them; published predecessor rosters never become local authority.

The signed v2 roster carries sorted full identities. The snapshot authority tail is
explicitly versioned, with the old roster section empty. Old fingerprint grants cannot
identify their original grantees and are reset, requiring owner re-approval. No members
or content are removed by this migration. Native role transactions block delegated
admission until their generation is saved. Published stale roster replay can still
affect badges/relay liveness, but cannot authorize owner admission.


This document is the in-repo source of truth for the design. It records the locked
decisions, the corrections that came out of an adversarial design review (run before any
code was written), the honest residual risks, and the phased build plan.

## 1. Locked decisions

| Area | Decision |
|------|----------|
| Stack | Rust core (an ordinary workspace of library crates; the shared `staticlib`+`cdylib`+`rlib` artifact is the **Tauri bridge**, `apps/desktop/src-tauri`, which is what a mobile host links against) + Svelte 5 UI, packaged via **Tauri 2** to Linux/Windows/Android from one codebase. |
| Group crypto | **MLS (RFC 9420)** via `openmls`, ciphersuite `0x0003` (X25519 + ChaCha20-Poly1305 + SHA-256 + Ed25519), `PrivateMessage` wire format only. One MLS group == one server/connection. Per-**device** identity (a human with N devices = N leaves). |
| Channels | NOT separate groups; each channel/wiki/status/calendar/moderation document derives an independent key via the MLS exporter secret + a canonical, injective `(doc_type, doc_id)` context. |
| Delivery | Encrypted **CRDT documents** (`automerge`) synced P2P. Chat logs are append-oriented; policy documents are not assumed append-only without protocol enforcement. New Studio/index/reply/registry types use the owner-receipted bounded epoch-close protocol in [`design-epoch-close.md`](design-epoch-close.md); this is distinct from per-operation delivery receipts. A newly applied remote op queues an authenticated, connected-only delivery receipt for its exact document/change hash; causal descendant evidence remains the compatibility fallback. Delivery receipts prove delivery, never reading. |
| Networking | **rust-libp2p** (QUIC + TCP + WSS for the application mesh). Direct reachability uses stable ports plus best-effort **UPnP IGD, IPv4 PCP/NAT-PMP, and IPv6 PCP firewall pinholes**; zero-knowledge **circuit-relay v2 + DCUtR** hole-punching and authenticated rendezvous cover harder networks; **AutoNAT v2** performs scoped dial-back testing through explicitly enabled relay/rendezvous nodes. PCPv6 binds the exact global listener address to that interface's scoped default router, requests short leases for TCP and UDP/QUIC, and honors the router's assigned lifetime up to 24 hours. AutoNAT serving is experimental and off by default; its pre-socket guard enforces exact source/target matching, direct public-address shape, global/per-prefix/per-peer rate buckets and concurrency caps. Invites embed bootstrap multiaddrs. A 60-second `JoinReply` lets the inviter (or one explicitly authorized current-member helper) dial a joiner's validated public routes back. Separately, an opted-in current member can publish a two-minute signed **switchboard offer**; a fresh, explicitly labelled assisted invite may endorse up to three such members, and the joiner must consent before they are contacted after direct routes fail. A switchboard forwards only the admission exchange to the invite's named inviter and must catch up the exact MLS Add before becoming the joiner's first member path; it never signs/adopts the Welcome or becomes a general circuit relay. An already-admitted but isolated member can create a ten-minute, member-signed `MemberRecoveryCode` containing at most four direct literal-IP, terminal-peer-bound candidates. The recipient verifies its exact current device→transport binding and seals expiring consent before the bounded dial; only a later outbound Noise-authenticated route is promoted, without destroying the last proven contact on failure. |
| Live media | Calls and screen shares use WebRTC separately from the libp2p application mesh. A receiver advertises only its nearest 720p/1080p/1440p/2160p display bucket (with resize hysteresis); a screen sender parks each edge, applies per-peer resolution/bitrate/frame-rate limits, and only then attaches the screen track. Client-wide presets are exposed both in Settings and from the call-stage cog. Screen audio defaults off; the user may request the selected surface's audio or grant multiple independently chosen application/window sources, which are mixed locally into one audio sender and forgotten when sharing stops. Its 160-kbps-per-peer cap is reported as planned unless the WebView applies it. Permission-prompt results carry lifecycle leases so stop, leave, lock, mode changes, and competing requests stop late tracks rather than resurrecting capture. It reports the estimated aggregate mesh upload and prefers H.265/AV1/VP9 with compatible fallbacks when the WebView exposes them. Runtime stats, not preference order, are the source of truth for the negotiated codec. |
| Invites | Strictly **single-use, device-bound** (one device per invite); revocable/expirable. |
| Files | Content-addressed over **ciphertext**. New listings carry a group-bound device signature over author/name/path/file-ref so per-uploader local trust cannot be bypassed by rewriting the CRDT `author` scalar; unsigned legacy listings remain usable but cannot satisfy specific-uploader auto-load. Circulation expiry is metadata; automatic eviction is not wired. Explicit kept copies use a separate quota-reserved sealed local store. Index publication does not prove remote possession. Health checks authenticate storage seals/CIDs and decrypt file refs; repair may overwrite a corrupt local record only with authenticated, CID-valid peer bytes. |
| Moderation | One group-bound, independently signed moderation document per server. Warnings attest to bounded snapshots; kick votes are advisory; only the owner-only MLS removal path changes membership. The log is not yet protocol-enforced append-only (threat-model R7). |
| Local continuity & backup | Drafts/read positions are bounded and vault-sealed. Backup is an opaque, non-overwriting copy of a freshly snapshotted sealed vault. Secret changes atomically rewrap the same DEK; automated restore is not implied. |

## 2. Corrections from the adversarial review (must hold in the implementation)

The Gate 4 completion candidate exposes retained Studio work without promoting it into shared
history. Awaiting-tenure views remain non-authoritative bases: their edits use a ticketed
Unconfirmed overlay, are shown as local-only and follow typed exact-retry instructions. The
lifecycle surface can inspect/export/archive/copy/dispose retained work; copy is preview then one
ordinary provisional Save, and destructive actions require literal confirmations. The registration,
command policy, native conversion, frontend parser and consumer move together. Direct Closing Save
and repair commands stay unavailable. This preserves the existing ownership model rather than
creating a second source, receipt or repair authority in the renderer.

Creative immutable blobs are independent of the P1 document lifecycle. The C0c native seam uses
the existing blob staging/store and authenticated fetch protocol, with PIX1 validation and bounded
reads/response decoding added at the relevant boundaries. Publication returns a real CID only after
promotion and flush; saving a Studio reference is a separate subsequent operation. P1 does not replace
Automerge or sync: it adds permission to retire their retained history into owner-receipted,
verifiable checkpoints with bounded recovery. Existing snapshots alone do not provide that.

Gate 6's leaf export layer is a pure frontend conversion/codec boundary (`studio-export.ts`,
`pixa.ts`). Scores produce the existing validated jam-take type consumed by the jukebox; PIX
frames use the existing PIX decoder. No second synth, media transport or document owner is added.
The candidate composes these helpers with typed persistence, native custody and production
controls; the leaf functions themselves still grant no publication, edit or membership authority.

The merged Gate 4 baseline connects Studio's typed owner decision to the same durable P1 journal,
recovery store and exact-envelope intent retirement used by Registry. Recently watched targets
share the existing native idle worker for owner rotation and Registry pointer/tail maintenance.
An installed, flushed checkpoint with an exactly matching current-owner journal can complete
that journal's pending publication slot locally: it is available through keyed head/seed service,
not claimed delivered to another member. This lets a solo owner keep rotating. A Registry Fault
remains a per-bucket hold; it does not invalidate otherwise authenticated storage accounting.
Registry head adapters therefore authenticate/account the source, persist any independently
authorized B0 report, and only then preserve Fault as a no-head result. The prepared adapter reuses
the verified detached graph rather than reconstructing it for this ordering.
Actual changed Registry wrappers refresh the existing inventory-validation cache before the next
Studio turn. No second mutable document cache, budget owner or network protocol is introduced.

Explicit recovery controls now borrow the same sole Server/vault/session custody. Whole-version
Restore is a sequence of re-previewed domain choices, never a replacement Automerge branch or
an atomic batch. A fingerprint covers the complete current projection; Apply also rechecks all
retained/staged historical deletion evidence and then uses normal Save. Exact accepted own-log
retries remain idempotent after later edits or snapshot eviction. Pending intents do not bypass
that fence. Copy is explicit mutable replacement/deletion, not same-id resurrection. Pointer
restoration is separately retryable with actual saved-target verification and existing Registry
accounting; an incomplete pointer step cannot relabel already saved content as wholly restored.

The same watched worker now paces own-intent replay, with current full-log equality and all
retained recovery snapshots checked before each operation. Stable-id prerequisites use a bounded
topological plan; hashes do not choose between competing mutable edits. A newer current value,
deletion, contradictory historical selection or cold large reference holds automatic replay.
The user-approved manual disposition first flushes the existing full-envelope recovery copy,
then removes only the matching own intents through the accounted ledger writer. This is needs
recovery, not receipt finality. Missing evidence stays pending. Settlement invalidations are
queued under custody but emitted after its release; reads do not emit refresh-triggering events.

Studio's Rust schema layer (`catcoms_replication::studio`) decodes the closed IndexOp, FlipnoteOp
and ScoreOp bodies, preserving stable element ids and the frontend's canonical JSON bytes.
ScoreOp itself supplies only static musical grammar and complete-envelope checks; the separate
typed ScoreTarget now supplies its causal/P1 writer and exact checkpoint/recovery preflight.
Its codec must be selected from trusted root kind; shared title/patch shapes do not identify it.
Contextual decoding checks the whole P1 envelope and expected type/key; Index creation additionally
binds `created_by` to a caller-supplied, independently verified full author. This decoder grants
no membership, causal-mutation, storage or publication authority. The first read-only projection
is StudioIndex: flat immutable insertion/deletion records retain full provenance and mutable
title/expiry registers use Automerge's actual winner with all concurrent alternatives retained.
The smallest insertion op id wins a same-object collision; the first 64 live object ids are
visible, with explicit overflow and deleted content kept for later typed recovery. Every live
root value is checked, including losing headers; primitive and value-byte limits bound the reader.
The Flipnote frame projection likewise retains insertion/replacement/deletion
evidence. Stable insertion-node origins and a right-origin forest preserve sequential placement
and same-gap concurrent ordering without rewiring losing/deleted anchors. All live alternatives
remain available even when a frame is deleted or beyond the 999-frame / 8 MiB playable prefix.
Typed Score and sound/export projection support is present in the candidate; unsupported future
families still refuse rather than disappearing. Projection readers alone do not authenticate
their records. A separate Index delta validator checks canonical record/actor
binding, the sender's dependency-frontier target existence, immutable headers/evidence and exact
same-property predecessors for mutable registers. It is a pure semantic callback: P1 must still
authenticate membership/server/physical scope and preflight the exact checkpoint. The art/frame
callback supplies corresponding checks, deriving left/right insertion origins
from the sender's full causal projection, including hidden nodes. Its private historical reader
uses one frontier for keys, all values and register winners; the public current reader retains
its original pristine-history rule. `StudioTarget` combines these callbacks with existing P1
gated edit/ingest and exact next-checkpoint AND complete recovery preflight. It supports epoch
zero and verified typed seeds; it does not own durable intents, storage, publication or settlement.
The compact baseline is one immutable `_studio/seed` bytes property alongside the original
headers, not a new operation log. It preserves selected values/authorship and up to four values
per conflict field across 1024 fields. Frame origins normalize to the live playable chain and
are explicitly marked `checkpoint`; all original/hidden/losing evidence and current operation
bodies remain in typed recovery. Ordinary post-seed registers override fallback values, with
same-property predecessors only; inherited provenance ids are not reusable operations/markers.
No user operations, markers or tombstones enter the compact seed. Generic P1 hash/receipt
verification and canonical typed rebuilding both run before installation. Recovery uses the
existing bounded envelope with a complete typed payload, not the lossy generic summary arrays.
Historical reads still repeat Automerge clock work; boundedness is not measured production
latency. These projection helpers themselves provide no actor/native or user-interface binding.
`StudioEpoch` now privately owns the Index/art document, signed log, gate, opening receipt and
receipt book. Its bounded vault-only restart format revalidates signatures, causal mutations,
typed seed and gate/log coherence; removed authors remain historical evidence, not current edit
authority. The `ServerStore` adapter saves the exact intent before this whole unit and returns
prepared ciphertext only after both durability barriers. Exact retained-envelope retries flush
unchanged files, including at the ordinary content ceiling. Receipt sealing retains the complete
source and persists faults; it does not settle, prune or install a replacement.
Opt-in five-family inventory adds Studio records and temporaries to the existing storage/intent
budgets. A mount-local generation rejects stale scans and duplicate Studio budget handles. The
coordinator must still exclude interleaved raw registry/recovery/owner writes; the token is not
a global filesystem transaction or blob-retention guarantee.
Explicit Index/art Save/Load now runs through `ServerActor::studio_begin` and five native commands.
The bounded queue carries no vault guard: the actor advertises Ready before native attempts all
locks fail-fast. The transferred lease retains mounted vault, numeric-server persistence, UI
commit and exact registry-incarnation guards. One blocking worker owns the sole live Server
(not a cloned MLS/device) plus that lease; invoke/actor cancellation cannot release its custody
mid-write. Guards drop before reply/event awaits. A worker panic stops the actor rather than
resuming an absent or stale Server. Live channel/member and complete-envelope checks precede
mutation I/O; current MLS/device snapshot persistence precedes the intent/source write.
Local frame references require exact held, validated 192x144 PIX bytes promoted and flushed in
the same vault namespace. Create writes the object before the Index and is exact-retryable, not
atomic across files; a different nonce cannot rename an existing object through Create.
Native projections retain conflict/deletion evidence and three-state expiry. Local update events
invalidate views, while responses explicitly label local publication and provisional edits.
Create keeps the new object in epoch zero but obtains the Index's actual Open epoch under the
same transaction, including after rotation. Automatic Index/art sync, watched-owner rotation,
repeated-tenure recovery and detached Studio/Registry repair are connected. Dense-source latency
remains unqualified. The Studio UI consumes the reviewed local lifecycle and ticketed Unconfirmed
Save boundary; it does not expose direct Closing Save or native repair controls.
Persistent blob handles now share one mount-local deletion guard outside the kept-copy adapter.
The derived reference union is keyed by full group bytes (including numeric-server aliases).
Studio source/intent/recovery writes add holds before I/O; only a complete, generation-current,
exclusive five-family reference scan replaces them. Unknown/corrupt/unsupported/partial state
refuses held-blob deletion. A cheap bounded absence-only filename check can initialize an empty
new mount; a restored P1 mount starts Unknown. Studio access refreshes before pre-holding new
PIX CIDs, then verifies/promotes bytes. Its later budget scan is deliberately reference-neutral.
Source holds include the verified seed-only projection as well as current projection and signed
operations: successor edits can hide a seed's replacement pixels. Pending intents and BOTH
retained and staged typed recovery also hold pixels, including deleted/superseded versions.
The guard stays locked through synchronous unlink; dropping the store revokes old blob handles.
There is no durable pin ledger or expiry engine. Full scans have existing work rails plus a
65,536-reference cache rail; overflow holds bytes rather than truncate. Existing file unlisting
and upload cleanup use the guarded delete, but still do not promise freed disk space. Explicit
kept-copy release and unreferenced staging cleanup retain their separate ownership semantics.
The Gate 6 candidate extends this enumeration to retained sound/export CIDs, including losing,
deleted and overflow evidence. Future doodle/unknown families must extend typed enumeration before
enabling reclamation; unsupported records cannot be interpreted as an empty hold set.
Explicit Studio exchange now reuses current sealed operations, blinded topics, the cancel-safe
subscription reconciler and one-shot publication. A bounded opt-in sync inbox authenticates,
but the existing accounted Studio store remains the only durable admission path. Logical
watch identity excludes channel aliases and retains full-author rate debt across replacement.
Saved-only send matches the entire own operation in the current source before exact retry and
resealing; it cannot become another edit path bypassing native Save's PIX/snapshot ordering.
Server adapters bind channel, source, mount and current membership. These low-level cooperative
calls provide no scheduler, catch-up or discovery themselves. The bounded native runtime below
now supplies scheduling/custody, remote events and one opened-source reuse slot; joining remains open.
Native Save remains local/provisional.
Initial publication is now integrated into that existing Save arm. Its private bounded batch
contains only the packets returned by successful durable edits, exposed after the final view
read; partial Create errors expose no batch. The worker returns the sole Server and lease, and
the actor keeps the same native/source custody through at most two one-shot attempts under one
two-second injected-clock deadline. No duplicate save/reseal pass is performed for this initial
send. Native cancellation/operation accounting follow the actual worker/lease; interruption or
send failure cannot retire an intent or undo Save. Guards drop before reply/event backpressure.
The response remains a local/provisional acknowledgement, not a delivery claim.
Successful access now installs at most 16 recent-target watches from the already-checked source
descriptors; same-watch reuse preserves queued traffic, eviction revokes it. Reconciliation shares
the existing two-second network wait budget. An independent coalesced boolean wakes one native
receiver per exact actor incarnation, never through an actor-awaiting event consumer. One packet
per paced pass gets the same Ready/lease/off-executor custody as Save. Current watch/mount/channel
and membership/MLS are rechecked before disk work; snapshot persistence precedes typed ingest.
Only newly Accepted durable edits emit remote StudioUpdated; events recheck native incarnation.
Automatic inventory uses the existing scanner with local limits (1024 directory entries,
1024 records, 8 MiB authenticated bytes and 256 KiB cold validation bytes). The mount-local
LRU retains every family's pure accounting validation (and an Intents record's inventory facts),
never a document, inventory or write permit. Every hit requires fresh authenticated file bytes, scope/filename binding and
the exact complete-wrapper digest/size. Ordinary complete scans warm it; remount starts cold.
It is indexed and holds up to `MAX_ACCOUNTED_RECORDS` (65 536) entries, so a complete scan never
thrashes it, and every inventory-issuing finish prunes the entries of records the scan did not
find, within the scan's own coverage (`GATE4-AGENT-1-C3-RUNTIME.md` 16.1).
The sole mounted store can also retain one owned verified Studio restart unit, moved rather than
cloned. Explicit view access prepares it; warm receive and timeline reads reauthenticate its exact
complete wrapper and bind current actor/group/MLS before reuse. Index refreshes keep only their
verified footprint when art occupies the slot, so list invalidation does not evict opened art.
The source has an 8 MiB encoded-input rail, not an 8 MiB heap promise. Cold targets retain the
256 KiB inline rail; larger watched sources use bounded detached preparation instead. Ingest uses the same typed
gate, fresh complete inventory/budget and durable save. Failed takes discard the owned graph;
successful unchanged flushes preserve the physical version rather than a normalized snapshot hash.
Failures pause until successful explicit Studio access and emit StudioReceivePaused; peer traffic
cannot repeatedly trigger a failed scan. This is not a reduced document cap or completed large-
vault collaboration: unrelated cold inventory still needs explicit preparation. Unopened-source
service and Index/art discovery now use the bounded coordinator described below.
No new persistence, finality, blob fetch or UI implementation is introduced.
Studio now reuses the registry's private bounded page walk through a typed provider wrapper.
Registry-v1 cursor bytes stay unchanged; Studio adds a distinct HMAC domain and channel binding
alongside full group/type/key/concrete epoch/provider/requester/initial-heads/seed. Both use the
same fixed accepted-prefix continuation, 32-operation / 512-KiB page bounds, monotonic expiry,
current-member resealing and explicit checkpoint/historical-authority holds.
The cooperative store provider serves only its exact prepared source, with authority/MAC checks
before I/O and full authenticated-wrapper matching afterward; it never performs a cold restore
inside a page call. Page admission moves the same checked source as live receive, validates every
entry through the existing typed gate and crosses one accounted save/flush barrier. A bad middle
entry saves no prefix. An uncertain rename may leave the old source or the entire new page; only
successful persistence returns counts/frontier, and exact retry after reconciliation deduplicates.
Empty pages verify the exact Open epoch and flush held bytes; actual epoch-zero absence stays
absent. No source seed, receipt or intent is installed/retired by this seam. Additive Studio kind
23 now uses the existing Registry request pool/rates and bounded typed answer framing with a
separate signed-response domain. The actor owns the cursor/retry state; signed connected-only
attempts detach before network waits, so mutually reconnecting actors can serve each other.
The native receiver wakes on its pending hint or five seconds of injected-clock idle time. This
is outside the actor select loop, avoiding a separate continuously-ready timer arm. Native work
still uses the existing command path; legacy command-versus-outbox cancellation debt is unchanged.
Service/client/gossip turns are bounded; held pages win their persistence pass. A context change
discards/retries instead of becoming a sticky storage pause. Cold watched source verification
also detaches, sharing Registry's four process slots; full-wrapper/mount/member/MLS fences
reattach only a current result to the same one-source owner. Provider cursors remint on runtime/
mount changes. Keyed discovery, unopened-provider service and checkpoint installation still need
joining integration; no native lifecycle permit or finality follows from a page result.
Expiry mirrors FileExpiry's absent/null/timestamp states, not the fixture's numeric-only view.
Jam descriptors are bounded/validated and retain their existing declaration-order identity hash
even inside the sorted-key Studio body. No audio renderer or game/avatar work is added.

P1 owner authority now also has independent local tenure evidence in the MLS synchronizer.
Locally founded epoch zero is known; Welcome joins and legacy snapshots are Unknown. Every
applied owner-changing MLS transition records its resulting epoch, including Adds into recycled
low leaf slots. Same-owner commits preserve Unknown rather than inventing a start. A shared
synchronous mutation seam observes the actual group before propagating a helper result, including
an error after a successful merge. The observation is saved in the same strict snapshot as MLS,
not in a separately advanced sidecar. A received receipt never establishes its own tenure.
This supplies local evidence only: fresh owner proofs require the publication barrier described
below. An upgraded or newly joined owner with Unknown tenure may remain
unable to authorize rotation until independent evidence or a witnessed transition is available.

Explicit owner registry rotation now uses that same durable MLS/tenure snapshot permit. Under
exclusive sync/store borrows it flushes the checked source, checks the owner inventory, and either
resumes the exact saved decision or derives a new close from all current heads. More than 64 heads
refuses without truncation. The actual authenticated closure must meet the lower/upper budgets and
typed checkpoint validation before signing; a live projection alone is insufficient. First-of-tenure
inheritance comes from the installed opening seed, and later decisions repeat the journal baseline.
The owner vault record saves the exact close alongside the selected receipt in one atomic replacement,
so a crash before sealing cannot change the decision when more Open edits arrive. Such later edits
enter recovery through the existing adjacent installer. Legacy pending receipts without a saved close
hold explicitly. Installation returns publication-pending status and does not mark the receipt
published. Kind-21 serving now records exact completion only after a current-owner proof is
accepted by the local reply-forwarding channel. Its private one-shot handoff is consumed under
the same synchronous Server/store borrow with fresh authority and accounted journal persistence.
This is not transport-driver admission or peer delivery. A crash or write failure after handoff
can require exact republication; uncertain disk state blocks accounting until reconciliation.
Automatic scheduling and durable repair remain open.

Keyed registry receipt-head discovery is now a cooperative authenticated kind-21 exchange.
It registers logical buckets, not concrete epochs, and returns provisional hints or a one-shot
current-owner selection proof. Explicit local preparation saves the whole-server MLS/tenure
snapshot and mints a runtime/epoch/mount-bound permit; remote queries never trigger that large
legacy serialization. A proof additionally requires complete inventory agreement, a nonfault
saved registry, and exact equality with the pending-preferred owner journal. Source flush and
journal re-save precede signing. Disagreement/stale preparation remains a hint; lost indexed
files, faults and corruption refuse. A selected receipt is not proof its seed is available or
verified. Expected-seed fetching uses additive kind 22. Explicit recovery-first registry
installation is connected to that scoped selection. The Studio coordinator now schedules Registry
checkpoint bootstrap and Studio joining; automatic Registry tail receive/rotation remains Gate 4.

Checkpoint fetching keeps discovery provenance rather than converting public receipt fields into
authority. The private selection is minted only while checking the fresh kind-21 owner response;
runtime, MLS, requester, owner and a per-bucket supersession token accompany it. Four non-Clone
handles retain at most one verified 2-MiB seed each for bounded attempts. Any newer authenticated
owner selection for that bucket, MLS transition, runtime replacement or receiver timeout revokes
use without refunding memory still held by a handle. This is not a current-head lease.

The independently proven seed provider may be any current member. It serves the installed
opening seed from a checked, inventory-matched vault unit, not the latest receipt's prospective
successor; during Closing the latter is normally unavailable. Fault refuses. Kind 22 binds full
identities, current MLS, actual transports and the exact query/answer. Raw seeds use the existing
512-byte to 1-MiB padding ladder inside group AEAD; above that ceiling the encoded size remains
visible. The receipt's exact Automerge hash is checked before parsing, then the canonical registry
schema. Fetching never writes the receiver's vault, retires intents or replaces provisional work.
The registry core has explicit checkpoint-adoption state: a distant selected receipt seals
the entire held source, with no pruning, and a typed plan preserves the whole previous version.
Retargets keep the same content-derived recovery id; seed-only and terminal-epoch versions count.
The outer restart v2 / adoption-only receipt-book v3 keep nonadjacent selections separate from
ordinary adjacent settlement. Opening/prior-target equivocation is checked before stale filtering.
Fault is a successful typed outcome which the store transaction saves before seed or recovery work;
bad seed or recovery must not suppress that evidence. Constructing a separate successor
does not install it, retire intents, or provide a transferable network-authority permit.

Studio Index/art now consume this same adoption model through a typed whole-version recovery
plan and the existing sole prepared source/five-family budget. Source Closing or Fault is saved
before optional seed work; recovery is durable before selecting the separate successor. Normal
Studio restart v1 is unchanged, with v2 reserved for an in-progress adoption. The shared adoption
verifier no longer mistakes Registry's 4,096-epoch product ceiling for a limit on Studio receipts.
Joined-member tests use the desktop's normal authenticated channel-index bootstrap, then actual
head/seed requests, recovery-first installation, open-tail paging and disk reopen.

Gate 4 now also has Studio's typed owner-close and adjacent settlement preparation, matching
the existing Registry flow without introducing another finality protocol. The privately owned
Studio source validates the exact signed dependency closure and builds its typed seed. An
immutable close/receipt decision resumes unchanged after newer Open edits or restart; only the
future durable owner-journal adapter can make it publishable. Adjacent plans distinguish complete
included/excluded author envelopes and preserve the full source projection when compaction loses
evidence, even without excluded operations (conflict overflow, deletions and original frame gaps).
Whole-restart-unit fingerprints fence plans; the separate successor preserves repair bookkeeping.
There is no new wire/vault format and no runtime rotation, pruning, intent retirement or UI hook
in this core slice. The next connection is the existing accounted journal/recovery/store gate,
then the actor/native driver; do not rebuild those storage primitives.

Additive Studio kinds 24/25 reuse Registry's head/seed queues, rates and retained capacity, while
separately binding type/channel/object and response domains. Detached prepare/I/O/complete phases
leave the actor free; private runtime/MLS/endpoint/attempt context is checked again on return.
The old cooperative Registry wrappers also become connected-only; wire compatibility is preserved,
but they no longer implicitly dial a proven endpoint. Weak generation maps follow the bounded
live handles. Both outbound and retained seed capacity follow cancelled lower-driver work.
Studio service shares the owner snapshot/journal durability barriers; no remote query can cause
whole-server serialization. The existing Studio receiver now drives unopened saved-key service
and automatic checkpoint discovery for recently accessed Index/art targets. Sync prepays each
exact authenticated queued request before source capture; it neither adds a UI watch nor grants
receive authority. Detached work reuses the four-slot preparation pool and the existing owned
Studio/Registry graphs. Registry graphs release their retained slots after a fixed 30 seconds
on the local idle pass, without extending the deadline per query. Verified prepared Registry
sources and exact Studio-bootstrap installation results warm the existing exact-wrapper
inventory LRU, not another cache or budget. Unrelated Registry writes retain their cold behavior.
Local owner snapshot failures retry after 30 seconds; queries cannot accelerate that cadence.

Joining tries the known key's Registry bucket, then the current owner's Studio head, expected
seed, recovery-first install and open-tail pages. Registry cold-request expiry allows at most
three fresh, charged head requests five seconds apart; a hint/refusal never authorizes installation.
A large already-saved Registry is prepared for accounting but its automatic adoption is deferred;
the known Studio key remains directly discoverable. Registry checkpoint bootstrap is not a claim
of current Registry operation-tail state. Rotation, pointer publication/refresh and Registry tail
receive, owner succession, own-intent replay and recovery controls remain Gate 4. Old-owner hints
still grant no authority. Recent-target watches are volatile: ordinary Read after restart resumes
the persisted Closing state, then expiry/retries need no further UI action.

`Server::install_registry_seed_step` now performs this explicit newcomer transaction under exclusive
sync/store borrows. It rechecks runtime, MLS, full local identity, owner/tenure, superseding selection,
receiver-clock expiry and physical mount/server at entry. The accounted store saves the full source
Closing (or Fault) even before a seed is available, then validates and saves whole-source typed
recovery before atomically selecting the successor. Invalid inventory or uncertain I/O fails closed;
an indexed source cannot be replaced by invented epoch zero. Recovery warnings keep their original
ids/deadlines across retargets and restart. Resuming after expiry requires fresh discovery, not a
receipt reconstructed into a permit. Adoption retires no intents; exact installation retries flush
the actual successor and preserve later edits. Old concrete watches/queued pages are not rebound:
the caller explicitly watches the installed epoch before normal paged catch-up. This is cooperative
registry integration, not an actor worker, current-head lease or universal progress at full quota.

The registry's adjacent, locally proved checkpoint transaction orders durable barriers as source flush,
typed recovery, included-only intent retirement, then atomic successor selection. Until selection
the complete Closing source remains the restart proof; afterward the verified seed and preserved
receipt book do. Exact retries flush the actual successor without replacing newer work. Local
Save requests carry a concrete epoch id so retired intents cannot silently reappear on retry.
Delayed opening-receipt conflicts fault even a successor already sealed by a newer receipt,
preserving both accepted content and high-water evidence. This is a tested backend adapter,
not live actor/transport orchestration. A bounded replay step now selects a saved intent by id,
checks its actual local author/current Open epoch and all typed recovery evidence, then uses the
existing durability path. New authoring is held on deletion or a superseding pointer hint; an
exact current-log retry reseals unchanged so failed-flush deletions remain retryable. Holds never
retire intents. Deletion screening is conservative for stable keys and best-effort after bounded
recovery eviction. A cooperative replay pass now snapshots one author's bounded id set and paces
one attempt at a time. It waits for an exact local submission acknowledgement, pauses failures at
the same id, and never retires intents on traversal/submission. The cooperative Server sender now
binds that pass to the exact sync instance and holds exclusive sync/store borrows from checked
preparation through one-shot dispatch. Actual full local identity, MLS epoch and current blinded
routing are checked before sending. Only Submitted advances; Duplicate, errors, cancellation and
unwind preserve the saved id for a fresh reseal without resetting pacing. This is not an autonomous
worker: aggregate scheduling, native lifecycle cancellation, catch-up, settlement-wide
capacity headroom and discovery remain to be integrated. No universal progress at full quota is
claimed yet.

The transport offers one-shot publication, now used by that cooperative backend sender. Legacy
`publish` acknowledges actor enqueueing and can hold ciphertext for later retries. `publish_once`
instead waits for one driver attempt and never enters that application retry queue. Its bounded
commands retain their capacity after caller cancellation until drained. Cancellation observed at
the last driver check suppresses an attempt, but cannot retract work admitted before it. Normal
gossip caches/handler queues may retain bytes even after some refusal results; cache duplicates
and local submissions are never delivery proofs. The sender owns saved-id retries and known-state
checks, not native UI-lock policy, driver deadlines or live actor wakeups. These additive seams do
not change existing chat publication or wire/persistence formats.

Managed registry gossip now has an opt-in receive path separate from the legacy document map.
A synchronous watch captures the checked current concrete epoch (or deterministic epoch zero
when absent), numeric server, physical vault mount and fresh sync/watch generation. Network ticks
authenticate into a 16-packet queue; an explicit Server drain persists at most one packet through
the existing typed gate and accounted store. Only the returned Admission describes the saved
outcome; queuing is not acceptance, and no delivery receipt or legacy accepted-op counter is earned.
Current local/full author membership, MLS scope, bucket and blinded topic are checked; delayed
packets recheck authority at drain. Same-id watch replacement revokes old queued work. Current-MLS
only reception and drop-on-cap/failure require author retry or future catch-up, not silent recovery.
Global pre-auth and per-full-author/document limits bound work; the shared pre-auth allowance does
not promise per-peer fairness. Subscription reconciliation retains a single uncertain topic across
an interrupted subscribe/unsubscribe and establishes it as unsubscribed before retry, including
after the same topic was rewatched. Revoking a watch needs only its exact generation, so an old
mount can discard its own queued traffic without gaining permission to ingest. Desired
watch installation itself never awaits. Actor/native ownership, automatic scheduling of the
cooperative discovery/catch-up/installation paths remain next.

Registry catch-up has a cooperative read-only page provider over checked vault history.
Vault reconstruction checks dependency/duplicate presence through Automerge's applied change-graph
metadata, without reconstructing raw operations merely to test membership. It avoids historical
visibility scans for a change whose dependencies exactly equal the entire current frontier.
The trusted restore loop derives that equality after signature,
actor, scope and dependency checks, with no mutation before semantic validation. Other branches
retain historical reads, and live edit/ingest never asserts this optimization. The exact-frontier
path also skips the semantic validator's already-proven dependency-presence predicate. This changes no
wire/snapshot format, membership rule, cap or request deadline. `P1-PERFORMANCE.md` records measurements;
explicit source preparation now separates capture under store custody, detached worker rebuild,
and reattachment under reacquired custody. The worker owns no vault/device/MLS keys. Four
process-wide slots remain charged through captured bytes, cancelled workers and retained results,
including across remounts. Read-only cache attachment and each page compare the entire authenticated
saved record, so gate/book-only faults invalidate it. Superseded preparation jobs cannot attach.
Cold/stale service returns a local preparation error and never rebuilds inside the request path.
Automatic scheduling and native lifecycle ownership still need integration; the split API must
be driven without holding actor/vault locks through reconstruction.
Its HMAC cursor freezes the provider's dependency-complete accepted-log prefix
and advances by position, so appends and large head sets cannot force the same prefix forever.
Every page freshly seals at most 32 operations / 512 KiB of framed bytes; byte-identical reloads
preserve cursors, changed prefixes and provider restarts do not. Source seeds must already be
verified by a conforming requester; missing removed-author history explicitly needs historical
authorization rather than weakening live operation admission. Already emitted cursor history
counts as claimed held dependencies. Requester claims and prefix completion prove no remote
possession, finality or currency. The app binds provider state to the exact runtime/mount and
validates bounded request/MAC/current membership before source I/O. Kind 20 now routes authenticated
page requests into a bounded opt-in queue, and a mount/watch-bound app drain serves saved history
under independent requester and aggregate source-read limits. Both transport endpoints are bound
in the exchange; a client needs a current bound-member proof before disclosing private identifiers.
Its four outbound permits follow actual transport termination after caller cancellation. Replies
are unadmitted claims, not currency or delivery evidence. A bounded receiver now derives the
frontier from checked durable state, fetches without borrowing the vault, and advances only after
one accounted all-or-none page save. Independent unknown heads permit one empty-head fallback;
verified seed, provider and charged limits stay fixed. Four watch-bound passes each retain at most
one page and expire on the receiver's monotonic clock. Duplicate and terminal-empty pages still
cross the storage/inventory barrier; a receipt seal or MLS advance cannot turn them into stale
success. Automatic actor/native scheduling remains; the adapters
do not move vault ownership into sync or change the legacy document map.

Owner succession over an **already frozen** source now shares one first-new-tenure check
(`crates/catcoms-replication/src/epoch/succession.rs`). `frozen_owner_inheritance` establishes what a
new tenure inherits: the INSTALLED opening, never an uninstalled remote target and never an old
owner's journal. Scope and key checks precede allocation, because a Rust record can bypass the wire
decoder, so every receipt in play is re-decoded from its own encoding and restored-verified from the
vault before anything else is read from it. Epoch zero with no opening, or an opening whose closed
epoch is exactly one below and whose tenure start is strictly below this tenure, are the only two
inheritances; anything else is a receipt conflict. A same-tenure call is permitted **only** to resume
the exact previously journaled decision, never to generate a replacement for a pending seal, which is
what makes the write/seal crash boundary safe: before the journal write the held seal belongs to an
older tenure, and after it the seal may be that first current-tenure decision and no other.
`owner_rotation_needs_adoption` is a local routing hint only; the authority and source checks are
repeated inside `frozen_owner_decision`, which the Studio and Registry owner modules each hold over
the shared helper. This deliberately does **not** reopen a gate or choose a seed: Studio and Registry
still validate their own actual full closure, persist the exact decision, and use whole-source
recovery before checkpoint adoption.

The naive "one group, every device commits, replay old ciphertext to latecomers" design
is broken. The load-bearing fixes:

1. **Proposal/commit split.** Concurrent MLS commits fork the group. Devices replicate
   *proposals* via the CRDT; a single designated committer per epoch packs them into one
   commit; deterministic fork-resolution (lowest commit hash) + loser re-issues. (Design-era
   name; the implementation calls it `commit_id`.)
2. **Snapshot-only catch-up.** Never replicate raw old-epoch frames between members (it
   contradicts forward secrecy). Latecomers receive a self-contained Automerge snapshot
   re-sealed under the current epoch. `max_past_epochs` only covers in-session reordering.
3. **Inner per-op signature.** Every log entry is signed by the author's Ed25519 leaf
   over `(doc_id, deps, mls_epoch, payload, author_device)`, verified independently of MLS
   sealing; so re-sealing is a pure transport re-wrap and history cannot be forged/omitted.
   (Design-era name; the implementation calls the type `SignedOp`, sealed as `SealedOp`.)
4. **Invites bound in MLS.** The invitee's KeyPackage carries a `(GroupId, invite_nonce)`
   extension validated at `Add`; the leaf is reserved in the InviteLedger before commit.
   Stops cross-group KeyPackage replay and partition double-claim.
5. **Routing topic decoupled from content epoch.** Gossipsub topic + rendezvous namespace
   derive from a **separate metadata exporter label**, rotating only on member *removal*
   (with grandfathering); not on every commit.
6. **File crypto.** Per-file wrap nonce (no nonce reuse); re-wrap content keys under each
   new epoch so a removed member cannot lazily fetch post-removal files. Already-fetched
   files cannot be un-shared (documented limit).
7. **One key hierarchy.** Platform-wrapped root → one DEK → HKDF subkeys (SQLCipher DB key,
   openmls value sealing, blob-at-rest). On Android, **two tiers**: a non-auth-bound
   SERVICE key (background sync of ciphertext) + an auth-bound CONTENT key (decrypt
   plaintext for display). `SecureKeyStore` reports a tier enum; a downgrade forces a
   passphrase before the store opens.
8. **Also:** no device key ever leaves a device (multi-device is origin-rooted
   certificates at chain depth 1; `design-multi-device.md` v2; with origin-signed
   revocation; the earlier account-key cross-cert chain design was superseded and its
   module deleted); blob-fetch padding/quantization + per-session outer
   re-encryption; eclipse resistance (≥2 rendezvous + member peer-exchange + roster-size
   check + cached addresses); decorrelated cache eviction (jitter) + holder liveness probe
   + mandatory archive-pin role; bounded deterministic owner-receipted checkpoints for P1
   document types (with provisional edits and two-version recovery, not an all-acked consensus
   claim); resumable chunked anti-entropy under relay caps; panic-revoke (any
   sibling device removes another) and `mint_invite` gated behind the auth-bound key;
   per-server "relay-only / hide my IP" mode; pure-Rust crypto on the hot path;
   metadata-index aging/re-keying.
9. **Discovery egress is peer-bound and process-bounded.** Every peer/invite/switchboard/companion
   grant route
   accepted for dialing uses one canonical supported multiaddr grammar with a non-zero TCP or
   UDP/QUIC socket and a terminal `/p2p/<PeerId>` matching the signed/discovered transport
   identity. Untrusted records use IP literals; DNS and dangerous local/private/link-local,
   multicast and transitional ranges fail closed outside the explicitly local invite case. The
   sync classifier deliberately retains non-routed documentation/benchmark literals as
   deterministic test stand-ins; those can only consume bounded retry tokens. `DiscoveryPolicy`
   charges addresses rather than peers, then
   one desktop-owned `EndpointDialScheduler` applies monotonic per-process, per-server, canonical
   Phase-0-peer, attempt, and IPv4 `/24`/IPv6 `/48` caps before submission. The parser embeds that
   peer principal in each opaque endpoint so cache, rendezvous, and pre-join callers cannot select
   different accounting identities. A direct attempt is keyed by its physical socket, excluding
   the claimed terminal id; a relayed attempt is keyed as the authenticated relay/target circuit,
   while the relay's outer host remains bounded by the prefix and process caps. PeerId/sequence
   rotation therefore cannot reset a direct socket, prefix, server, or process bucket. The transport
   refuses a dial command with no terminal peer rather than falling back to an address-only socket
   dial. Two-way reply retries use the same scheduler for each new socket pass; their proof request
   may continue only over a connection that is live when the network actor handles it, without
   consulting the ordinary recent-peer redial cache.

## 3. Honest residual risks

- **Metadata** is the dominant weakness: who-talks-to-whom, timing, group sizes, and; once
  two members hole-punch; each other's IP, are partly observable. Mitigated, not eliminated.
  AutoNAT additionally reveals a candidate address and probe timing to its configured observer;
  a positive is scoped to that server/address/moment and is not a universal reachability claim.
  No nation-state-grade metadata protection is promised.
- Automatic router mapping and IPv6 firewall pinholes intentionally expose the stable TCP and UDP/QUIC libp2p listeners to
  the internet when the local gateway grants a lease. Noise still authenticates/encrypts libp2p
  sessions and connection limits cap strangers, but a hostile/local gateway can deny mapping or
  report an unusable address. PCPv6 may also disclose a device-specific/privacy address, and a
  granted pinhole does not prove the host or upstream firewall permits traffic. A mapping is
  therefore only a candidate until AutoNAT calls that exact address back.
- Two-way reply codes and member switchboards improve signalling/routing, but cannot create a path
  from nothing. The first two mutually unreachable users still need a public IPv6/manual/router
  mapping, a configured relay, or a reachable third party. Reply punching requires both apps'
  60-second sessions to overlap and is primarily useful for QUIC; TCP simultaneous-open is not
  claimed. Switchboard offers disclose the helper's stable identities/candidate addresses to the
  invite recipient and disclose the joiner's IP/timing to the helper. Signed candidates are not
  proof of address ownership or reachability.
- Post-join discovery is self-healing only while some route survives: the desktop polls a bounded
  set of peers with authenticated PEX, retries cached/current signed address epochs with monotonic
  exponential backoff and jitter, and resets that delay on a new signed epoch or connection
  lifecycle. Native route/interface notifications trigger a debounced re-sample of the kernel's
  route-selected IPv4/IPv6 sources, while the roughly-minute pass remains a fallback. A changed
  sample republishes one address epoch; exact route ownership prevents raw-interface removal from
  withdrawing an identical mapping/manual/relay route. It intentionally does not merge
  withdrawn public IPs forever because an ISP can reassign them; a newer zero-route descriptor
  also removes the prior sealed cache row, so restart cannot resurrect it. Route signatures and matching
  peer ids still do not prove ownership of an IP/port before the bounded first packet is sent.
  Scheduler counters are transient and reset with the process. The scheduler owns the one injected
  monotonic clock used for both reservation and commit. A generation-bound, non-cloneable permit
  moves into the network actor with each discovery dial command; commit rechecks the window deadline
  under the scheduler lock, so queued old-window work cannot start merely because no later
  reservation happened to roll the generation. Actor-side duplicate, already-connected, or
  already-dialling suppression drops/refunds the permit before commit. The peer-wide check covers
  member dials later reclassified as infrastructure; constructor TCP/QUIC routes for one peer are
  grouped into one known-peer address race and enter the pending ledger by exact libp2p
  `ConnectionId`. Constructor peers are protected from remote eviction but are not thereby
  classified as infrastructure, so an inviter reached over a bootstrap/relay route may still race a
  newly learned direct member address. Pending infrastructure attempts are released only by the
  matching connection/error id; an unrelated inbound connection cannot clear them. The actor commits only immediately before the endpoint enters the
  pending/socket-start path. Caller cancellation after enqueue therefore cannot refund work the actor
  will still perform, and an old-window command cannot decrement a replacement window. A failure
  after commit is conservatively spent even if libp2p rejects before a socket completes. A relay circuit
  has its own attempt key so unrelated targets at one relay do not starve each other, but the shared
  outer relay socket is not separately leased at the exact-socket scope; it is bounded only by the
  relay-host prefix and process caps. Libp2p's pending-outgoing cap is still per swarm, so this slice
  does not yet provide a process-wide in-flight/concurrency lease. A fully isolated device whose
  current address is unknown to every peer still needs
  out-of-band signalling, rendezvous/relay infrastructure, or a reachable member; swarm sampling
  cannot manufacture a route from no contact.
- Connected current members now provide a bounded baseline repair control plane. A node pushes at
  most two session-proven active members an authenticated probe for one exact descriptor; each
  helper answers later with a separately authenticated, exact-attempt-bound result. One positive
  observation may queue a short reciprocal request A→C→B; every hop is a connected-only
  asynchronous push, and B revalidates current roster, exact descriptor hashes/sequences, helper
  signature, expiry, replay and rate limits before submitting
  at most one IPv4 and one IPv6 direct route through the shared endpoint scheduler. Two signed
  negatives are suspicion only. Helpers never dial during a probe and never carry application
  traffic; switchboards remain opt-in admission-only, while general circuit relay hosting remains
  a separate opt-in role. HyParView/CYCLON-like active/passive selection and local age/source
  metadata improve which peers are tried but cannot repair a partition with no surviving edge.
  Indirect probing keeps one target per pass. Initial selection honors topology priority; later
  passes traverse eligible full device IDs circularly across the complete bounded peer-record
  set. A scalar session cursor advances immediately after each confirmed local admission, before
  another helper can suspend. Local refusal, negative replies, descriptor refresh and disappearance
  of the cursor's record do not rewind traversal. This is ordering only; every existing live
  membership, descriptor, path, expiry and rate check still applies.
- Pairwise route evidence is a bounded, session-only refinement of aggregate transport liveness.
  Libp2p reports a sorted/deduplicated IPv4/IPv6/DNS/memory + TCP/QUIC/WebSocket/circuit snapshot
  after connection edges, including relay-to-direct DCUtR upgrades and partial closes. The sync
  layer accepts detail only while `PeerConnected` remains live, caps custom transports again,
  retains historical success for at most 24 monotonic hours, and never serializes it. Admission
  paths that temporarily inspect pushed proof/Welcome requests coalesce any lifecycle
  edges they dequeue into a bounded final-state handoff, which the new sync owner adopts once;
  ordinary pre-owner connection waits use a non-consuming watch. Connectivity exposes typed
  claimed-peer health/actions, refreshes on path changes even when aggregate presence
  is unchanged (plus a bounded visible-view refresh for time-derived expiry), and says “no claimed
  path,” never “offline.” Dial counters describe scheduler submissions/cooldown, not
  unobserved per-address failures, and IPv6 candidate shape is not presented as an outbound-route
  test. A
  signed peer record is still a self-asserted device-to-transport binding, so this evidence is not
  proof that the member controls that live transport key or is reachable from another network.
  Reciprocal repair is intentionally not a dual-key ownership proof and does not strengthen that
  binding.
- A **fully compromised device** exposes its current keys and plaintext; PCS only heals
  *after* the device is removed.
- **Already-fetched files cannot be un-shared.**

## 4. Engineering seams (every layer is written against these)

- **`Clock`** (in `catcoms-rt`): all time flows through it; no layer calls the OS clock
  directly. Enforced by `scripts/check-no-ambient.sh` in CI.
- **`MeshTransport`** (in `catcoms-rt`): pub/sub + addressed request/response. Node logic
  runs over the deterministic in-memory `MemNetwork` in tests and over rust-libp2p in
  production, unchanged.
- **Canonical wire format** (`catcoms-wire`): length-prefixed, fixed-width, injective
  encoding; the basis of collision-free key-derivation contexts.

## 4a. Network join handshake (6c)

A fresh device joins over the wire by sending its invite-bound KeyPackage to the
inviter (request/response); the inviter validates and returns a Welcome. Adversarial
review (run before commit) hardened it:

- The joiner's KeyPackage init key is **not secret**, so a malicious inviter/relay
  could otherwise add it to a group *they* control and return a valid Welcome. The
  invite therefore carries the **inviter's public key**; the joiner authenticates
  the invite (`verify_self`), the admitter **signs the Welcome**, and the joiner
  verifies that signature against the invite's inviter and re-checks `group_id`.
- Only the invite's **named inviter admits** over the network (so the joiner can
  authenticate the response). `serve_join` runs cheap invite checks *before* the
  expensive KeyPackage validation, and oversized control requests are dropped.

## 4b. Membership commit propagation (6d-1a)

The Add commit (previously discarded) is now captured and fanned out on a per-group
**control topic**; every member applies it via `process_incoming` and advances to
the same epoch; so a multi-member join converges and a non-admitting member can
decrypt the new joiner's ops. A design+adversarial-review pass (verified against
the openmls 0.8.1 source) showed the "safe by construction" claim was only *assumed*,
so safety is **enforced**: only the **designated committer** (lowest leaf index) may
admit, which prevents concurrent commits from forking the epoch chain. Tested with a
3-member join + a non-committer-admit rejection. That is the **default** configuration
(`SyncConfig::max_committer_rank: 0`), not the only one the code can do; §4c records the
staged fork-resolution path that a non-zero rank enables.

## 4c. Missed-commit recovery + past-epoch key window (6d-1b)

A member that misses a control-topic commit (unreliable delivery / brief offline)
self-heals: an out-of-order commit is buffered (`pending_commits`), a new
`KIND_COMMIT_CATCHUP` request fetches the gap from any member's bounded `commit_log`,
and the missed MLS commits are **replayed in epoch order** through `process_incoming`.
Every retained buffer/queue is hard-bounded so an untrusted peer cannot force
unbounded allocation; a forged future commit is gap- and size-capped and fails MLS
verification at apply time (its only residual cost is one deduped catch-up request).
Separately, a bounded **past-epoch channel-key window** (`snapshot_epoch_keys` before
each advance → `Zeroizing` keys, evicted past `max_past_epochs`) lets an op sealed
just before an epoch boundary still decrypt (`ingest_with_key`, inner signature still
verified) instead of being dropped as `EpochUnavailable`; deeper gaps fall back to
auto-queued document/commit catch-up. Peer discovery started as nothing more than remembering
inbound `Gossip.from`/`Request.from`; there is now a real `DeviceId→PeerId` directory.
`ChannelSync::member_transport_peer(&DeviceId) -> Option<PeerId>` resolves a **current member**
device to the transport identity in its latest self-signed `PeerDescriptor`, the records fed by
member PEX and the sealed address cache. It is deliberately narrow: the record is signed by the
device it describes, so a modified client can sign one naming somebody else's transport peer.
That is harmless for a caller that only *labels* the record's own device, and not harmless at
all for a caller that acts **against** the value, so every such caller carries its own
operation-specific liveness and exact-binding checks (transport eviction is the worked example,
with three checks in three places).

An adversarial review (background `Workflow`) hardened this before commit. The
load-bearing fix: the **catch-up serve endpoints are members-only**. A requester
proves current membership by signing a transcript with its MLS leaf key; the server serves only
if that key content-addresses a current member, the timestamp is fresh, and the
signature verifies; so an outsider cannot harvest a group's id, member device ids,
or history from these endpoints. Also folded in:
hard response-size bounds on the *serving* side, `committer_device` validated against
the designated committer on the inbound apply path, and explicit caps on every
recovery buffer/queue.

The transcript has since grown: of the two residuals the original review recorded, one is closed
and the other is narrowed but still open (see the end of this paragraph).
It is now `(CATCHUP_AUTH_DOMAIN ‖ group_id ‖ u16 kind ‖ inner ‖ requester_pubkey ‖ ts ‖ nonce ‖
req_epoch)`, with the requester's **transport peer** appended for the kinds where
`kind_binds_requester_peer` is true (`KIND_CATCHUP_SINCE`, plus the registry page/receipt-head/seed
and studio page/head/seed kinds). The per-request RNG nonce and the epoch close the
same-millisecond `ts` collision window, so a captured response cannot be replayed against a
different request; the peer binding is rebuilt from where the bytes actually arrived, so an
endpoint cannot forward a request verbatim to a real member and have that member's answer
attributed to itself. Which kinds carry the binding is a compatibility decision, not a security
ranking. **The transcript is what a signature is over, so adding a field to it changes what an
older build computes and breaks both directions of a mixed pair.** `KIND_PEX` and
`KIND_COMMIT_CATCHUP` exist on the released build and keep the released transcript exactly; a
commit catch-up that stopped verifying would be a member stuck at an old epoch, unable to open
anything sealed under the new one, which is a missing-message failure rather than a discovery one.
They instead stop being a way to *prove* anything about a transport peer: their answers still carry
usable records, but only an exchange bound at both ends establishes that the endpoint answering is
the member that signed. Two more are excluded on their own merits: reciprocal forwarding relays a
request on somebody's behalf and re-authenticates the original bytes at the far end, and a delivery
receipt is built once and sent to several targets, so binding either would be wrong rather than
safer. The remaining residual is that there is no server-side seen-nonce log, so a captured
*request* can be re-sent inside `MAX_REQUEST_AGE_MS`; that costs one freshly-signed re-serve into
the peer's own Noise session.

**Document catch-up pages by position.** `export_catchup_since` recomputed the whole difference on
every call, so a serving peer that could only send a prefix sent the same prefix forever; against a
requester whose frontier is too narrow to name everything it holds, that prefix is history it
already has and the operations it needs sit behind a wall of duplicates it can never clear. That is
a livelock, not an inefficiency, and it is reachable: `sync_frontier` caps the hashes a requester
may name, and a serving peer walks its own log in insertion order, so unnameable history sorts
ahead of whatever is actually missing. Two protocol-boundary facts follow.
- `MAX_CATCHUP_SINCE_HEADS` is **512**, raised from 64. The subtraction walk visits each change at
  most once (`have` is a set, and a hash already in it terminates that branch), so it is
  O(document) whether it starts from two heads or five hundred; the real cost is 36 bytes each on
  the wire, and a maximal frontier frames to roughly 18 KiB against the 64 KiB control-request
  bound. What 64 bounded was the requester's ability to *describe itself*, and a frontier that
  cannot say what it holds is not merely incomplete, it is wrong. This moved the threshold past
  any realistic group; it did not remove the failure mode.
- `export_catchup_page` did. It walks the log from a caller-supplied position, skips what the
  requester claimed, stops on a byte budget and reports where to resume, sizing **before** sealing
  so a page never pays to seal operations it will not send. `KIND_CATCHUP_SINCE` carries an
  optional trailing continuation, and a truncated answer comes back under its own marker
  `CATCHUP_SINCE_PAGE` with a 20-byte cursor (its own marker rather than an optional prefix on
  `CATCHUP_SINCE_MORE`, because every twenty bytes decode as a cursor and a requester could not
  otherwise tell one from the front of a bundle). **Both wire directions are additive**: a build
  predating paging sends no continuation field, that frame still decodes and is answered exactly
  as before, and it is never sent the new marker, because only a request carrying the field gets
  one. A position means something only against the log that issued it, so each cursor is stamped
  with a per-runtime provider id and one replayed to the wrong peer, or to the same peer after a
  restart, is recognised as foreign and the walk restarts rather than skipping history. It is
  deliberately **not** authenticated: a forged position can only make a server skip operations the
  forger then does not receive, so the fence exists to stop an honest mistake silently hiding
  history, not to stop a liar harming itself. A round that applies nothing but advances the walk no
  longer counts against the source; a round that repeats with no way to advance still does.

**The dead zone past the commit log, and what is now known about it.** A member behind by more
than a serving peer's `max_commit_log` cannot be chained from that peer, and there are two bands:
inside `max_commit_gap` the records buffer and can never drain, past it they are dropped before
buffering. Both used to return `Verified { applied: 0 }`, which is byte-for-byte what an honest
up-to-date peer returns, so the drain read the exchange as closed and retired the recovery; and
past `max_commit_gap` nothing was logged at all. The state is now typed.
`CommitCatchupOutcome` distinguishes `Verified { applied }` from `Empty` (the peer sent an empty
bundle, which is genuinely what both an up-to-date member and a refusal look like on the wire, kept
as its own variant so the conflation is visible where it is relied on), `Unanswered` (no answer,
oversized, undecodable, or not signed by a current member: a fact about the peer and the link, and
evidence about the commit log in neither direction) and `Stranded { lowest_available }` (the peer
answered with authenticated records that all begin above this node's epoch). A `Stranded` outcome
closes nothing, so the drain marks that source and re-queues to one whose log reaches further back.
`note_membership_chain_gap` keeps the **best** (lowest) offer seen at this epoch in
`MembershipChainGap { current_epoch, lowest_available, observed_at_ms }`, counts
`SyncStats::commit_chain_gaps_observed` once per epoch, and warns; `membership_chain_gap()`
filters on read against the current epoch, so a node repaired through any route stops claiming to
be stranded without that route having to know about it. **The limit is deliberate and worth
stating: this is detectability only. Nothing surfaces it in the UI and there is no repair path.**
A full snapshot rejoin remains the missing recovery, and `docs/MESSAGE-FLOW.md` §11 puts
surfacing the state to the user ahead of deciding whether an automatic rejoin can be made safe.

**Implemented but off by default (not deferred):**
- **Concurrent-commit fork resolution.** `SyncConfig::max_committer_rank` is `0` in
  `Default`, which is strict single-committer: only the designated (lowest-leaf-index)
  member admits, on the synchronous fast path. At `>= 1` the staged path is live: a node
  collects competing same-base candidates for `stage_decision_window_ms` (`PendingResolve`),
  adopts the lowest `commit_id`, and the winner merges its staged commit while the loser
  aborts (`MyStaged`; a staged Add carries a `StagedJoin` whose provisional Welcome is
  pushed on a win and rejected on a loss, and whose invite nonce is consumed only on a
  winning merge). A candidate built on a different base is a deeper divergence and is
  refused outright. `SyncStats::forks_resolved` / `forks_lost` / `forks_too_deep` count it,
  and it is test-gated (roadmap 6d-2a 2b/2c, 6d-2b 1/2). Turning it on is a configuration
  decision, not remaining work.

**Still deferred, with the data model already in place (no rewrite):**
- The rest of **6d-2**: the full RFC 9420 proposal/commit split (the designated committer
  packing replicated *proposals*, rather than each member committing its own), the
  replicated InviteLedger giving single-use across members, and joiner-bound nonces.
- A full snapshot rejoin for a member past every reached peer's commit log.
- Per-peer rate limiting / off-actor offload of join work.

## 4d. Product operations and moderation plane

The desktop keeps operational observations separate from protocol claims. **Storage health** walks
the current file index and validates every manifest and referenced chunk through the storage seal,
CID and file-layer key. One report per server is cached for the process session and augmented with a
deduplicated category/pin/largest-file inventory; category local-byte totals remain estimates, while
the verified ciphertext total is exact. **Repair** is an explicit network action: it re-fetches only
missing or unreadable content from authenticated members, re-runs verification, and replaces that
cache. Upload deduplication also requires complete locally authenticated possession, including
whole-file hashing. A metadata-only re-upload retains fresh staged ciphertext and publishes an
attested repair; only an exact verified local-device listing can be replaced. Other uploaders'
attestations remain intact. Downloads and media share a bounded resolver for at most four encrypted
manifests with identical ordered plaintext chunk identities, lengths and MIME fields. Media cache
identity binds the complete sorted set; every alternative is independently authenticated before
use, with local alternatives tried before any network wait. **Connectivity assistant**
reports the live peer/path evidence already available to the node; it is diagnostic and never a
proof that every remote member or future network path is reachable.

Moderation history lives in `DocType::Moderation` (stable tag 14, document id 0), not in a chat
channel. Each event/vote has a canonical Ed25519 signature binding its semantic fields and group id.
The app also binds the signer device to its certified member origin and interprets authority through
the current owner-signed role state. Warning evidence therefore survives a live message edit/delete
and is attributable to the moderator who observed it. It does not retroactively authenticate the
message author, prove historical role state, or prevent a modified member from deleting a CRDT root
entry; those are the explicit R7 residual. A vote can never invoke membership mutation. The owner
must resolve a case, and removal reuses the protocol-enforced MLS Remove flow.

Frontend continuity state (currently composer drafts and per-channel read marks) is stored as a
bounded vault-sealed record rather than browser plaintext. An offline backup first snapshots live
actors and persists the registry, then copies the already-sealed vault tree to a new destination
without following links or overwriting an existing backup. Restore remains deliberately staged work:
it needs a locked-vault import path, full verification, atomic swap and rollback before it can be
safe to expose. Export creates another permanent offline guessing surface and leaves filesystem
metadata visible. Changing the live secret atomically rewrites only the DEK wrapper; it cannot revoke
an older export and never rotates the server/blob encryption keys.

Normal window close is a native-owned lock/snapshot/destroy transaction. Close attempts serialize
inside `AppState`; once a continuity failure has been returned, another already-queued caller must
also defer until a later request explicitly acknowledges losing that latest screen snapshot. This
prevents duplicate WebView calls from replacing the failure with a newer success and destroying the
only surface capable of warning the user. The decision is bound to the close request's own native
generation because another lock caller may acquire the shared commit mutex first and consume that
newer snapshot. Plaintext jam-sheet export uses the same exact-generation commit guard and holds it
through both Downloads publication and OS reveal.

Bounded whole-file reads may reserve a native cancellation registration before invoking
`download_file`. The signal crosses the Tauri bridge into the server actor's chunk-fetch `select!`,
because dropping a webview promise or reply receiver does not cancel an actor command already in
progress. Registrations are process-capped and generation-bound. A shared opaque keepalive crosses
the sync/transport seam and remains in libp2p's pending-request row until that exact request
responds, fails or times out, even when the JavaScript/native waiter has already been cancelled.
Their begin step commits under the exact UI-session guard. Locking signals active operations and
drops unclaimed permits; transport work stays counted until its terminal event, while an unclaimed begin-only
permit may also be displaced instead of stranding capacity after a webview reload. Legacy callers
that omit a cancellation token receive a reserved native token, so they share the same cap and
lock cancellation without changing the IPC shape.
`MeshTransport::request_cancellable` intentionally has no default: every transport must state the
accounting lifetime. Production libp2p and the in-memory transport transfer the keepalive to their
actual pending/queued owner; wrappers delegate to that implementation, and non-transferring test
fakes own no detached lower work.

## 5. Roadmap (test-gated, block by block)

0. Workspace + `Clock`/`Transport` seams + canonical wire format + CI/lint gate.
1. Device identity + unified key hierarchy (`SecureKeyStore` tiers; desktop impls).
2. MLS `ServerGroup` (local, no network) + channel-key derivation.
3. Invites (MLS-bound, single-use, device-bound) + InviteLedger.
4. Replication engine: inner-signed ops, proposal/commit split, snapshot catch-up. P1 adds
   owner-receipted deterministic checkpoints only for its new document types; see its own staged
   implementation status rather than treating compaction as already shipped globally.
5. Storage & retention: CID blockstore, 3-scope expiry, clock-injected GC, file crypto.
6. Real mesh: libp2p, relay-v2/DCUtR, rendezvous, scoped gossipsub, eclipse resistance.
7. End-to-end local integration over real sockets + consolidated security suite.
8. Product model + Tauri desktop UI (channels, fileshare browser, status, wiki).
9. Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys.
10. Hardening: calendar, cover traffic, supply-chain attestation, security review.

### File reliability and explicit retention

Studio's Gate 6 core continuation extends the existing P1 `StudioTarget` with a score family,
not a new store or transport. Reusable remove-wins musical slots, sender-frontier checks and
explicit-family snapshots/restart units are specified in `GATE6-STATE-DESIGN.md`. Existing art
and Index bytes remain unchanged; explicit app/native/wire family activation is now implemented
and reviewed separately. Final combined acceptance remains governed by the Gate 6 ledger.

[File reliability](design-file-reliability.md) separates actor-owned authorization/storage from bounded
network tasks, projects conflicting first file-index lists consistently, and adds explicit local kept
copies. Kept-copy ownership never comes from replicated expiry. One exact manifest is reserved and
verified before durable visibility; normal cache deletion cannot remove it. UI availability reports
local evidence and leaves remote copies unconfirmed.
New retained transfers use a purpose-key-separated sealed successor journal in `transfers-v2`,
with full native group/device scope, private partial chunks, persistent prepaid execution allowance
and explicit idempotent Retry. Resumable, Completed and Deleting are distinct durable states.
Legacy pending remains cleanup-only behind a permanent format barrier. Actor workers reverify the
saved prefix and whole plaintext before completion; queued network work carries its original
deadline and revocable run owner. An actor-owned paced scheduler resumes saved partial intents
under an exact native unlock generation; persistent user Pause, budget exhaustion and deterministic
integrity stops remain distinct. V2 canonical bytes reconcile before explicit V3 consent migration.
Completed checks retain cancelable inline leases. New V4 multi-variant intents reserve their whole
original compatible set with per-candidate progress and a separate committed winner. Fallback
publishes a selection under a fresh opaque run, resets whole-file hashing and preserves all global
allowance/intent. Public retained service exposes only the committed winner. V2/V3 remain exact-only
with unchanged bytes and charges. Focused integration checks/review are ongoing; indirect blob
service and combined acceptance remain open.


### Native full-source read custody (19 October)

The future connected file service has a separate opaque read task, independent of voice runtime
and small ciphertext page sizes. `SourceReadRuntime` reserves a fixed 40 MiB before bounded store
preparation, with a single process-static pool limiting two actual reads/results total and one per
full group. Replacing a Root, runtime or executor cannot create fresh capacity while old workers
remain alive. A task moves both plan and affine credit into the app's `spawn_blocking` closure;
a completion keeps its result and that same credit. No owned-result extraction bypass exists.

Store/winner fences and exact CID verification survive this handoff. Future service must separately
recheck original group/membership/request/native-session/deadline authority before signing and
transfer any outgoing page into response custody before dropping source completion. The executor
factory starts no work by itself. New request dispatch, helper service, source cache, replay/rate
admission and actor integration remain unfinished; this is no indirect file acceptance claim.


### Existing file work: actor and Root ownership

Detached file requests now observe both their original actor generation and Root/member/policy
generation. The actor retires before stop acknowledgement or a blocking close-event wait. A
reversible freeze retires before native custody waits and resumes with a fresh generation; old
local jobs, retries and completed responses cannot adopt it. Root observations retire on actual
epoch/member/policy changes even without voice attached. Its sole publisher is the first Root
field, so Drop ends it before transport/MLS/store fields. Neither observer owns those publishers.
Actual lower accounting remains held through transport retirement. This supplies no bridge peer
standing or native entry consent; those additional scopes remain separate.

### Native confirmation result ownership (22 October continuation)

The inactive native action path now composes an original buffer with actor delivery without
copying that buffer. A trusted `VoiceConfirmationBody` pins one prepaid owner with
`Bytes::from_owner` outside native/Root guards. The pinned bytes implementation calls the
owner's slice callback once at construction; the actor reads only the cached immutable slice.
One private Flight per body covers queueing and completed-but-uncollected output. A lost caller
does not release the original buffer/payment while the actual command or output remains alive.
The existing Root seal/open methods still validate the actual grant, action and result tag.

Native Phase reserves4KiB actor metadata from its ORIGINAL16KiB loan before taking a result
buffer. Local refusal returns that exact buffer for retry. This payment is independent of a
future detached dispatcher task; neither a queue slot nor a receipt proves Phase drainage.
Explicit confirmation retirement consumes the original HandoffLoan to close its original plan,
outside native locks. Ordinary loan return leaves the96KiB Workflow open. Actual input/lower/
copy owners continue to retain the original Workflow after closure.

Browser broker v10 can lend a Ready channel's existing execution position to its exact selected
key/chain. Provider/digest work and same-channel IO share that lane; RPC waits occur outside
provider ownership to avoid self-deadlock. Every row retains its original native adapter.
These ownership primitives do not enable action opcodes: the application listener still serves
Status only. Framed action dispatch, complete final ACK/collection, proven-pair actor ingress,
SDP transfer, live V2 and physical acceptance remain separate unfinished work.

### Native physical shutdown and private request grammar (22 October continuation)

The application service now closes its listener, latches Registry shutdown under Index, and
closes every original Row's first-effect fence. No later bootstrap or retained Route may create
new work. It then closes header admission on each original Session and waits for that Session's
actual physical position outside Index/Row/UIcommit/Coordinator guards. Retirement witnesses
also find a Session whose only remaining owner is a detached worker or completed reply. A single
in-place Notify waiter observes last-owner collection; socket loss, End and timeouts do not
replace it. The service task/output retains its original process payment throughout this wait.
SDK/actor cleanup follows Host completion. This is native physical collection, not proof that
browser SDK work or the complete96KiB Workflow drained.

An authenticated, opened Data request can transfer its original buffer into the Phase-funded
actor body without copying it, leaving only an empty body placeholder with the same affine
response nonce. Local range/foreign-Phase/capacity refusal restores that exact buffer. The
eventual dispatcher still must resolve the complete decrypted purpose and original header RowId,
fund detached task metadata separately, and retain the unique physical hold through real result
work and output collection. The extraction primitive supplies no such effect authority itself.

Private version1 action plaintext has a fixed full selector prefix and closed per-operation
lengths. Native borrows at most528 output bytes; the browser constructs one prepaid bounded
request. Shared public vectors pin full-width u64 fields. Receipt ACK digests bind the actual
native transition and exclude observational retirement, matching Root's same_transition.
This grammar is inert: production continues to serve Status only. Pending closure, native proof
of final delivery, safe32-row purpose history collection and the complete action adapter remain
required before enablement. FinalACK alone must never remove history from a still-live Session.

The inactive derivation-result executor now consumes the actual opened Incoming and checks
its Session allocation against Route before lookup. It resolves the authenticated header RowId,
complete selector and native serial/version/input tag, and accepts only an empty Derive output
with its actual empty-output hash. Historical receipt recovery precedes current eligibility and
takes no additional Phase contribution, Work or task; the already-paid Data body remains owned.
Current effects repeat original UI/selection/Route checks under UIcommit and call the actual
ServerActor/Root method. Detached Worker and completed Response retain Work/task custody before
Incoming in field-drop order, keeping shutdown behind real publication and output collection.
The measured future/output/command envelope must fit8KiB from the original16KiB loan. Incoming
stays inline; both future and output storage count. Its single fixed4KiB header-scratch backing
allocation is explicitly counted/prepaid by the unchanged12KiB Session control envelope. Moving
that affine scratch handle does not reserve another inline4KiB in each async state; its full
backing storage is counted, zeroized and freed before physical idle can be published.
This narrow empty-marker executor does not construct the separate seal/open body bridge. Its
actor fixtures deliberately require rejection of a synthetic foreign pair; successful normal
production-pair publication, listener dispatch and the browser action adapter remain unproved.
