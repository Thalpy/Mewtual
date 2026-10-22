# Threat model; client integrity & enforcement boundaries

## App-owned bootstrap/status boundary (21 October)

The listener is now reachable during application lifetime. Its fixed process payment and
registry bind precede socket creation; its exact numeric port enters existing CSP before Tauri
creates windows. Mode selection uses Tauri's custom-protocol decision, not Rust debug mode.
Remote assets/windows, changed dev origins and unsupported HTTPS configuration refuse this
channel rather than widen CORS. Startup failure leaves other app functions available. Explicit
IO runtime entry avoids setup panics; Exit drains the actual status worker before SDK cleanup
and restart. An interrupted observer or concurrent finish cannot fabricate completed drainage.

The paid IO state holds nine Weak references to the actual native state, with no strong
AppState/AppHandle or registry/store/Coordinator owner in the process host. Every context upgrade
is all-or-nothing before factory admission; partial upgrades drain outside native fences. This
is lifetime access, not revocation. The same borrowed UI gate as ordinary IPC checks actual store
presence, original flags and generations after awaits. Weak references still retain inline Arc
allocations/control blocks; the measured32,256-byte Windows peak is prepaid by32KiB instead of
24KiB, reducing other available bytes within the unchanged2MiB process cap. Context fields drop
before the original IO payment; uncollected task output retains its payment as before.

Factories accept the original native-selected Arc and use pure early/final checks of actual
cached allocation, grant/lease, generation and binding. A foreign equal-label selection cannot
materialize or pay for a local snapshot. Broad live context drops before detached actor work;
that work alone retains its original actor/selected/grant/commit/registration/credit. A lock
cannot finish ahead of an admitted actor request after caller loss. A future action handler must
also repeat actual channel closure under the same final UI commit; weak context or selection
provenance alone does not authorize effects or claim the channel stayed open.

The three fixed-metadata commands are granted only to main, with remote/restricted surfaces
excluded by the real ACL. Setup also requires unlocked session, selected native owner, original
generation and actor binding. Exact cleanup remains possible while locked; it cannot install
keys, revive an old generation or authorize a Root effect. Cleanup accepts no arbitrary drain
assertion. Its trusted retained broker awaits real setup/SDK/IO promises before calling it.

All successful setup variants borrow one original Row projection guard. Pending/Closed metadata
remain paid even if End and collection race their delayed serialization. Ready checks the exact
original credit. A hard-capped JSON writer prevents serializer capacity growth; combined graph,
hex scratch and allocation fit the unchanged 8 KiB Row allowance. Arbitrary stock Tauri/WebView
public-metadata copies after handoff, SDK/GC heaps, allocator RSS and physical erasure are not
bounded by this payment. This narrow exception cannot carry secrets or action data.

The browser's retained v9 Session uses only its original selected key and executor. Constructor,
adapter identity and TX/RX floors survive remount; generic keys/seeds cannot construct another
channel. Native setup credit and the original control lane remain through actual SDK/read/cancel
work, within existing caps. Header MAC alone, timeout, End and caller loss never prove drainage.
Port takeover is hostile: fixed BYOB response prefix, MAC/AEAD/request binding and permanent replay
floors apply before body admission; redirects, credentials and arbitrary endpoints are refused.

Final closure fences new routing before detaching owners. Row collection requires a permanently
dead lease, no pending setup, unique registry Row ownership and zero strong Session/public owners.
Paid weak witnesses include headers and completed replies that hold no Row. Serial IDs never
reset; stale cleanup cannot affect a replacement. Late drops are reclaimed before new admission,
outside Coordinator. Uncertain history is never evicted for capacity.

The internal action ledger now resolves every cross-module operation against the actual
native-selected grant allocation, original lease, executor and full pair selector before
returning a native-only paid-row handle. Authentication of a channel or equality of public
fields cannot substitute another owner's row. Raw ID-taking operation helpers are private.
Historical resolution and exact receipt ACK/settlement do not upgrade a dead grant; data work
checks the original lease under State before copy/worker admission, including after waits.
This proves row provenance only. The future transport must still check actual channel closure,
current selection and UI generation at final effect admission, with Root's membership/epoch/
deadline checks. Repeated resolution is possible; physical callers must bound concurrent
handles in their existing payments rather than interpreting non-Clone as an admission gate.
An independent original-purpose lookup uses the same bounded index/provenance when a lost
factory reply supplied no RowId. It cannot allocate a row, re-arm a Pending worker, upgrade
a grant or treat missing history as drained. Explicit current-authority retry stays separate
and resumes only the same retained purpose/serial. End still permits historical lookup while
rejecting fresh work. Neither metadata lookup is exposed as a channel action by this increment.

Only Status is served. No factory/action input/result/ACK/settlement/final-delivery chain, live
V2/SDP/ICE/RTC, legacy-mode isolation or member media relay is enabled by this increment. Packaged
Windows committed document/reload, enforced CSP/CORS, public metadata and native shutdown checks
pass as recorded in HANDOVER; the probe requests no unlock and does not establish selected crypto
or action delivery. Linux/macOS WebViews and independent physical audio/network acceptance remain
unverified.

## Internal ECDH bootstrap boundary (21 October)

The candidate checks the actual selected-owner Arc, original UI generation and live lease at
final Session installation. Pending precedes injected RNG; error/unwind permanently closes
that purpose, so lost replies cannot reset counters or request another ephemeral key. Public
outputs share their original paid allocation. Native curve validation precedes secret scalar
work and transcript bindings separate roles/groups/devices/joins/executors/sessions. UTF8
decoding preserves a leading BOM rather than aliasing two owner identities.

ECDH is not remote authentication. Immutable native metadata must eventually arrive through
trusted main-window IPC and the actual selected browser private key. Later compromise of that
retained join key can recover recorded local exchanges; no forward-secrecy claim is made.
Pinned p256 has no independent arithmetic/assembly audit; stored-value Drop zeroization does
not cover arithmetic temporaries, SDK/provider heaps or compiler copies. Detailed source audit
is in communication-recovery/VOICE-NATIVE-RESULTS-NEXT.md. There is no listener or live path;
browser crypto funding, actual physical setup ACK/drainage and legacy isolation remain required.

## Fixed native infrastructure capacity (21 October)

The distinct process-retained credit shares the existing2MiB total with all groups. It is
capacity for trusted fixed native listener/router/task overhead only, with no group, Crypto
slot, forwarding lane, lending or effect authority. Group-scoped input, crypto and outputs
must not use it to bypass the256KiB group cap. Retain it until actual infrastructure/output
drainage, including after cancellation or group retirement. It does not bound OS socket
buffers, SDK heaps or allocator metadata outside the adapter's documented measured charge.

## Disabled local framing candidate (21 October)

The native candidate authenticates canonical fixed-size headers before body admission. Closed
direction/operation/body-class rules, exact HTTP length and native slot binding precede variable
allocation and counter debit. Admitted invalid/truncated bodies and cancellation permanently
spend both physical counters. A consumed reply owner prevents outgoing nonce reuse; response
MAC/AEAD bind the exact original request. A single position stays paid through actual output
collection, independently of caller lifetime. Control replies do not borrow a closed data Phase.

This local MAC does not authenticate a remote member or authorize a Root effect. Pool/group
identity checks are capacity binding only; registered opaque Actions and current native owners
remain mandatory. No browser raw-seed Session constructor was introduced: the experiment's
resettable codec remains fixture-only. Native/browser bootstrap, SDK delivery custody, IO
deadlines and authenticated browser closure are separate open gates. No listener/CSP/ordinary
V2 ingress or RTC call has been enabled by the codec.

## Protected handoff custody prerequisite (21 October)

Waiter loss, End, expiry or engine settlement is not evidence of application-copy drainage.
HandoffLoan retains the original Workflow through its actual last owner; a chain-wide busy
bit covers sibling rows. Borrow samples callback-bearing Clock before Engine, then checks pure
original gates. It conserves the existing96KiB envelope and adds no Crypto slot. Shared paid
historical PlanBinding metadata supplies no progress, key or first-effect authority.

The candidate body reader requires BYOB on the actual fetched stream, with no oversized-chunk
default-reader fallback. Supplied views are bounded by authenticated length plus one EOF byte,
and read/cancel promises drain before return. It does not authenticate a channel, cover the
remaining flight or prove physical SDK/GC heap zeroization.

The disabled native ledger binds registration to the original published selected key and
actual opaque Root action, not equal renderer fields. It prepays fixed row/phase metadata
before factories and retains uncertain purposes across lost replies. Worker/data contribution
retirement precedes the sole phase notification. Bounded metadata-only recovery samples actual
action history outside native locks; reordered observations cannot erase a terminal result,
settlement or retirement. Compacted tombstones retain no strong Action/Phase/Grant. Selected
snapshots use Weak grants so Generation pruning cannot become affine last-drop under Coordinator.
Pending/live/drained rows share 32 positions; saturation refuses further pilot work without
evicting uncertain history. Actual actor/browser integration, authenticated browser closure,
prepaid terminal replies and complete physical copy accounting remain enablement requirements.

## Selected browser key-generation ownership (21 October, complete gates passed)

A new browser broker cannot adopt a still-admitted generation by repeating its native owner ID.
Native selects one executor per actual accepted owner/grant, preserves its original join/permit
and frozen public point, and returns closed history after settlement. Settlement-first closes a
paid selection before a delayed first admission. One record per owner, at most 32 retained
records, existing group/process Crypto limits and paid fixed metadata bound native custody.
Revocation, IPC loss and elapsed time never refund admitted work. A native publication worker
retains UI commit through actual actor completion; cancellation of its outer waiter cannot
release that ordering guard. A stale completion cannot finish a newer publication retry.

The main-window ACL alone exposes these fixed public bootstrap commands; there is no private-
key export/adoption, listener, CSP source or byte-bearing result command. The executor is
lifecycle bookkeeping within the trusted main window, not remote protocol authentication.
Application record charges do not claim physical SDK/GC or stock IPC allocation bounds. Normal
V2 ingress, RTC mode isolation, result/ACK/settlement IPC and SDP phase transfer remain separate
enablement requirements. Current complete-gate evidence and its provenance are in the handover.

## Native pair authority (21 October, full gates pending)

A peer/device ID alone cannot bind an uncertain pair request: it now also names remote join,
key reference and epoch. Root selects new evidence only from its actual implicit Watcher, and
final native admission observes the original call/session/room/member/epoch and immutable window.
Exact history cannot substitute a newly proved join, renew a failed pair or allocate keys/RTC.
Pair protection is selected at creation; later epoch changes cannot migrate old authority into
a fresh chain. A future automatic join/phase replacement must preserve actual consent and mode.

Provisional pins grant protection/capacity only, never traffic/source quota. Unpublished failure
revokes them after guards release. Ledger claim, Pair and serial publish in one native turn.
No callback-bearing membership or Clock function belongs under that final fence. Grant context
sampling now precedes Group/Policy, and source batch Capacity preserves earlier actual lower
jobs. All 426 focused Root voice tests pass, including original-clock revocation sweeps and
native contention/limit cases. No live V2/RTC/media or complete-gate claim follows. Physical
acceptance still lacks a second desktop. Foreign immutable contexts reject before membership
callbacks, preserving the original group's valid presence rather than revoking it.

## Native actor shutdown and voice controls (21 October, full gates pending)

Opaque native actor controls preserve allocation identity and never turn serialized IDs into
authority. They expose no browser/RTC or wire ingress. Reply loss is uncertain; fresh candidates
or replacement grants cannot be inferred from it. Watch reply loss drops its affine observer.
An admitted stop retires original Root-local actor/routes/endpoints and queued replies before
acknowledgment, regardless of Closed event backpressure or later ACK loss. A queued canceled stop
and canceled freeze preserve the original installation. Other groups sharing Runtime remain live.
Actual running preparation, held inputs and lower transport owners retain their charges until
drainage; shutdown provides no resource refund. Original CallGrant and revision owners are revoked.

Final forward authorization observes route/connection/room gates after callback-bearing Clock
and effect checks, including callbacks that retire the native Root. Deterministic regressions
exercise these orderings. A dedicated sync-error fault-injection test remains open. Production
pair orchestration, serialized native IPC/copy/settlement, SDP transfer, live V2/legacy isolation,
media relay and physical acceptance remain enablement blockers.

## Root derivation and late first publication (21 October, focused checks passed)

Derivation factories recognize original call/action/chain allocations. Final native origin
rechecks actor/session, room, member/epoch, original chain leases and deadline after injected
callbacks. Contention consumes neither the row-zero issue nor engine capacity; abandoned action
serials remain burned. A cached Derived marker grants no key reconstruction, signing or RTC
authority. Actual input/preparation custody remains charged until drainage, including settlement
and unwind; idle open-Workflow reservations are independently retained.

Late confirmation outputs must satisfy their original source/pair deadline as well as their
construction window before first signed publication or Finish promotion. The same pure fence
observes original SDP bounds where present. Historical published retries do not regain pending
or send authority. Native IPC/copy/settlement, SDP phase transfer, live isolation and media relay
remain enablement blockers; this increment changes no wire or persistence format.

## Native confirmation result admission (21 October, validation pending)

Private prepared signatures grant no source authority. Final Capacity retains the exact artifact
in its original row, without another signature for changed ciphertext. First Root seal/open
results recognize actual call/action allocations and recheck original actor/session/room/epoch/
pending gates after the last injected callback. Session revocation wins before child revocation
can acquire Policy; a still-live child alone cannot publish. History grants no fresh authority.

An affine preparation loan retains actual engine/input custody while signing, RNG and copies
run, including competing settlement or unwind. Settlement fences first effects immediately;
actual drained status and credit release wait for that loan. Token/slot/phase/credit destruction
and promotion cleanup occur outside result/native guards. No limits are raised. All 389 focused voice tests and unchanged layout guards pass. Complete gates remain pending;
these changes establish no IPC, RTC, media relay or physical acceptance.

## Protected browser computation (21 October, integration pending)

Protected native row projections cannot use ordinary keys or raw encryption/opening APIs.
Role/row/allowance checks precede owned copying, including TypedArray subclasses with aliasing
slice methods. Counters burn before encryption; failed crypto and repulls cannot reset them.
One prepaid serial execution position and fixed rows bound work independently of ordinary load.
Completed output remains charged across uncertain replies. Historical tagged/tagless admission
reports cannot compute, commit or settle another browser's engine. Chain close revokes keys but
retains actual running work through final promise/settlement drainage. Browser bookkeeping is not
protocol authority; native first-effect/IPC, phase transfer, live RTC/media remain unimplemented.

## Delayed protected follow-up classification (21 October, awaiting validation)

Protected Proof/Finish intake deliberately retains original pair/chain inbox authority across
nonce-attempt replacement. This replaces the earlier arrival-attempt binding for those protected
roles only. Three fixed identities per incoming role bound the work. A delayed plaintext echo
must match the current native nonce and response phase to progress; stale/malformed plaintext
consumes only its exact token and retains a closed rejection. Such rejected history has no
attachment to an unrelated current nonce attempt. Accepted progression captures its real attempt.

First response publication requires the actual accepted decrypt action, original native owners,
current accepted signed hash and native response kind. An old unproduced trigger cannot copy new
response state. A previously produced response can recover its exact action, whose original
revocation/deadline still prevents stale encryption/signing/sending. Old response engine retirement
runs after the current decrypt result fence returns; running work and lost-ACK custody remain
charged. All 365 focused voice tests pass and read-only re-review is clear. The original fixed
layout guards pass; bounded links/response pointers live in their paid input records. Complete
combined gates remain pending; no live V2/media claim follows.

## Protected local Hello publication (21 October, pending validation)

The low-side producer prepares actual packet/action/row owners before a dedicated local
Group -> Policy -> try-History publication. It has no fabricated incoming ticket and spends no
peer's incoming allowance. A delayed request names immutable predecessor history; None remains
the original request after every successor. Unknown/cross-pair predecessors and a fourth lane
refuse. Root recognition uses the actual call allocation and room incarnation, so equal join
bytes cannot adopt an older native action. Ordinary-active pairs cannot silently switch modes.

Three bounded canonical Hello plaintext hashes also prevent A -> B -> A nonce reuse from making
an old Proof answer a distinct new start. Protected receivers reject distinct signed Hello IDs
repeating an accepted nonce before replacing the attempt or accepted sequence floor. These
hashes use the existing Pair record allowance; exact original-ID retries remain recoverable.

Attempt replacement closes old packet/action gates before replacement becomes visible. Chain
closure also closes sealed source custody. Running engines and held inputs retain their actual
credit until settlement/release; fixed history holds no engine lender, input or Workflow. The
three independent pending crypto gates and low/high role history pass the unchanged fixed-layout
guards. No resource allowance has been raised.

Clock callbacks precede irreversible observations. Composite source work samples each original
clock rather than interpreting old packet deadlines using another monotonic origin. A busy later
effect must not discard previously prepared source jobs after their attempts were spent. New
regressions target these races, exact retries and original owner substitutions. Static re-review
has no remaining actionable finding; all 352 focused voice tests pass. Complete combined gates
remain pending. These constraints
are not evidence of live IPC, browser confirmation, Proof/Finish, SDP, relay or physical audio.

## Inactive confirmation packet capacity (20 October)

Native V2 packet tables count actual rows and unused workflow claims in the same immutable
32-position limits. Role-specific claims prevent ordinary packet pressure consuming promised
confirmation positions; reservations from a different table cannot be adopted. Close returns
unused capacity, while actual retained rows/loans and their byte custody retire independently.
This capacity prerequisite changes no signed wire/storage format or authorization check.
Private high-side Hello now composes protected packet/event allocation, three candidate history
lanes and atomic origin/token/action publication. The final fence samples native time before
nonblocking Group/Policy/History locks; contention has no replay/candidate/quota effect. Refusal
destruction runs outside those guards. Three malformed admitted Hellos still exhaust their lanes;
successful confirmation nonces are not the admission counter.

Public and encrypted IDs share one classifier before ledger effects. Admission ACK history binds
the original call, room incarnation, pair, chain, member and actor, independently of consumed
decrypt tokens. Root reply authority also enforces the original epoch/connection; the metadata
receipt alone is not a current-epoch authority. Equal join bytes cannot transfer an old receipt
to a replacement call. Room deletion/recreation and End permanently revoke original ownership.
The receipt's conserved row includes its separately allocated effect owner. History owns no
payload, engine or Workflow; actual retained work remains charged after cancellation.

Protection is selected only before ordinary confirmation activity, and selection remains after
capacity refusal. Both first confirmation-result directions reject an epoch change before any
first effect, while exact accepted/rejected history remains recoverable. Production IPC, other
protected roles, SDP transfer and live V2 remain open. Slot reservation or private Hello intake
alone is not whole-chain progress, browser readiness, live signalling or media evidence.

## Chat history admission and retained audit (20 October, continuation)

Legacy operation ingress now refuses a 32-byte Automerge actor that differs from its verified
signed author, before graph/log mutation. Historical actors of other lengths remain compatible,
including actual pre-rebind 16-byte restore history. This prevents another signer occupying a
modern device namespace; it does not repair previously poisoned graphs/snapshots, legacy actor
collisions or same-device equivocation. Failed catch-up pages preserve their prior cursor and
completion state; verified prefixes retain receipts and recovery work before a later error.

The message resolver now addresses all visible first-writer lists and refuses duplicated IDs.
This does not replace R6: raw channel operations still lack protocol-enforced message/moderator
semantics. Legacy signatures also lack original-group/membership-epoch binding. Retained deleted
content is available to members holding the history, not cryptographically restricted to a
moderator UI. See [the trace and adversarial findings](communication-recovery/CHAT-HISTORY-CONVERGENCE.md)
for current guarantees, explicit limitation tests, migration needs and acceptance boundaries.

## Recovery through connection churn (20 October)

Local bridge-connection retirement permits a bounded retry of an otherwise current read, never
acceptance of the retired response. Opaque typed observations repeat original Root, native,
membership/epoch and lifetime checks after authentication and AEAD. Both successful plaintext and
failed-AEAD verdicts are discarded on route change, preventing an obsolete helper from storing
bytes or selecting/blocking a saved variant. Original cancellation or membership retirement wins
over route recovery. Retry sweeps and helper identities remain bounded; durable work is not
refunded, and actual lower owners retain independent capacity after timeout/cancellation.

Disabled V2 may accept a delayed positive ACK after a newer hedge only for the same immutable
packet's exact authenticated target and still-live original flight. This grants no fresh session,
deadline, retry budget or room-wide receipt. Root signature/request/peer/role checks remain
mandatory, while obsolete negative/helper completions cannot lower confirmed evidence.

## Gate7 upload and session replacement boundaries (19 October)

The 20 October native continuation binds streamed-upload tokens to the original UI-session
generation and actual actor/registry instance. Final removal and publication hold the session
commit guard, with a synchronous generation recheck after the uploads-map await. Lock requests
publish their revocation before waiting for commit. Before admission they reject and clean the
staged data; after admission lock waits for the owned worker's actual publication/failure cleanup,
even when its caller disappears. A completed lock/unlock cannot revive an old token. Cleanup uses
the original actor rather than a replacement registry entry. Quota refusal also cleans uploads it
retired. Ordinary persistence follows guard release to preserve leave's existing lock order; the
worker retains that task, but this is not an abrupt-crash transaction or remote-custody guarantee.

Studio/Settlement notifications use a separate short event-emission fence. Their bounded actor
event consumer never waits for the long command commit guard, whose owner may itself need that
consumer to drain before publication can run. It captures the UI generation before any await,
then repeats generation/closed/incarnation checks under registry and event guards through
synchronous emission. Lock revokes call/mapping/read owners before its first await, crosses and
releases the event fence, then waits for command publication. An already admitted emission may
finish after a lock request; lock completion cannot overtake it. Native Studio vault custody must
still be dropped before any EventSink await, as the actor's Ready path already requires.
The pending UI snapshot and nonresumable state precede both fence waits. Cancelling that native
future cannot make a later snapshot-less close silently forget valid bytes or authorize destruction
after malformed state. This establishes the native cancellation boundary; WebView reload cancelling
a Tauri invoke is not assumed or demonstrated.

Browser batch admission bounds retained user-selected File objects to one batch of at most32,
with a separate16-request review-queue bound. These are renderer lifecycle bounds, not a byte
bound on browser media decoders or the operating system. The native file limits remain independent.

An earlier upload/review cannot insert into a reopened wiki pane, a replacement unlocked session,
or a composer after close captured continuity. Currency uses the actual pane identity and original
session generation, not equality of the visible page/server name. Close retires reviews before
awaiting any save. Old leave-server responses cannot force-retire new Studio work, change trust
settings, expose old errors or delete a replacement leave operation. Announcement completion uses
the same session/close boundary. In-flight native transfers still have their own authority checks;
discarding a browser continuation neither rolls back a shared file nor proves native cancellation.

## Inactive browser derivation boundary (19 October)

The browser v5 plan binds the exact owner, broker-generated private-key handle, accepted local
join/key/public point, native chain projection, both device/join/key identities, epoch, handshake,
generation, role, original deadlines and frozen pair setup. Coercible objects are refused for
closed string fields before publication. A serialized ID is not a native capability: eventual
IPC must resolve the original native action and repeat authority checks independently.

Protected derive-only keys refuse every ordinary encrypt/decrypt/confirmation fallback. Lost
native replies cannot rerun ECDH/HKDF; terminal native Derived history cannot reconstruct missing
keys. End/expiry closes pending eligibility but does not settle an actual promise. Ordinary pair
wrappers also repeat their pair fence at the final broker-to-caller handoff and clear withheld
plaintext if it closed in that microtask gap. These browser tests use real WebCrypto and mocked
native replies, with no claim of live native/WebView integration or complete IPC/COM-copy custody.

## Voice terminal history checkpoint (19 October, Rust only)

The inactive confirmation plan is capacity only. Its twelve fixed local rows/slots and 96 KiB
variable envelope cannot authorize an engine, signed packet, browser continuation or RTC effect.
Successful preparation is permanently stamped in the native pair, including after close;
unpublished row abandonment returns only that original unused row. Publication consumes its
identity irreversibly. Exact Resources/Slots owners, scope/epoch, local/remote join/key, call/pair
pin/generation, role and original deadline prevent cross-owner adoption. A first derive result
checks the captured epoch under its existing engine fence; historical lookup stays independent.
An irreversible chain lease also fences already-admitted pending effects after close.

One weak group Workflow index counts actual input/engine/lower custody across End. History and
closed control metadata do not hold it strongly. The shared fixed metadata owner retains its
plain 2 KiB charge until the last actual control/row/workflow owner drops. Quiet collection
snapshots weak indices before entering control/action locks; close visits engine fences only
after releasing control/history locks. `pair_current` inside first-result processing only
revokes the pair pin; collection closes unused capacity outside that engine fence. At this
checkpoint only derive used protected rows; native/browser registration, complete confirmation/SDP intake,
phase/copy accounting and live V2 remain open.

Past accepted and closed-rejected action results remain metadata after End and actual engine
settlement. Exact output recovery rehashes bytes; a separate terminal-status snapshot permits
receipt cleanup after those browser bytes have been destroyed. Status is not output validation
or effect authority. The cfg(test) native registry preserves unacknowledged history across owner
replacement with a combined32-row bound. ACK requires the immutable registered action/input/
output/outcome tuple, ignores changing retirement, and cannot refund an unsettled engine.
Production IPC must add independent account authorization and prepaid finite reply-copy custody;
the Rust receipt allowance does not cover queued Tauri/WebView copies. Live V2 remains disabled.

For an installed terminal result, exact receipt ACK and actual engine settlement are both
required before dropping the action's input reference. Owned input leases preserve real buffers and their original charge across that
compaction. The fixed history row, immutable output bound and permanent engine fence survive;
no action slot, allocator or signing permission is recreated. Ordinary non-Exchange retained
credits support one fixed two-part partition only, conserving the original byte charge.
This is not a refund of serialized replies, browser plaintext or actual unsettled work.

Exchange action history is separately prepaid without live phase custody. Variable input and
actual engine loans retain that custody; a pending engine lender is removed permanently at
settlement. Publication consumes a separate directional identity that compaction cannot return.
Unpublished abandonment may return its unused direction. Unstarted retirement requires both
no actual permit and no terminal result; a settled accepted/rejected action with a lost ACK is
not unstarted. Actual final settlement without a terminal result permanently excludes later
first effects and can release input. No held reader or uncertain engine is refunded by End.

Prepared confirmation ingress carries its actual native Hello floor/attempt and the original
pair deadline through the final origin check into the retained token. The ledger requires a
binding selected from the signed kind, so its generic application-only admission cannot publish
an unbound confirmation. Replacement, a superseding Hello and expiry refuse before replay,
serial or origin debit. This closes the token-binding seam only; full continuation/action/receipt
prepayment and native/browser activation remain unimplemented and V2 dispatch stays disabled.

## Private download/export boundary (19 October, implementation in progress)

Sealed Download ownership has its own domain and canonical random-ID directory, sharing the
retained quota and exclusive mount with Keeps. Purpose and full MLS group/device scope are checked
before pending-journal reconciliation. A CID match grants no cross-purpose cleanup or run access.
Public blob reads/source preparation exclude all Downloads, including completed copies. Private
export requires exact issuing mount AND transfer-generation identity, saved chunk bounds and
whole-file verification. Original actor freeze/thaw, Root/epoch, account/entry, Pause/removal and
remount revoke old effects; cleanup of a committed export cannot remove a later verification run.
Four export owners survive queued plaintext replies independently of their canceled waiters.
After the cleanup await, returning a saved path still requires original native/actor/Root scope.
External file contents are flushed before no-clobber publication, but the destination directory
entry is not durably flushed. Successful export/cleanup therefore does not establish final-name
durability across host power loss; the native restart tests establish process-restart behavior.

Restart retains only encrypted whole-chunk acquisition progress and conservative spend. External
path/offset is never persisted and no startup flow may publish plaintext. Explicit resume resolves
a fresh destination under fresh authority. No-replace hard-link publication protects collisions,
not same-user tampering with the source staging path. A crash after publication but before cleanup
can leave an external file plus private progress; the UI discloses possible duplicate re-export.
This is not an exactly-once external receipt or a physical audio/network acceptance claim.

## Public MLS source-catalog boundary (19 October)

Connected-peer snapshots and evictable descriptor tables are not complete MLS source rosters.
The new catalog has no permanent320/512-member cutoff. Each page examines at most64 raw leaf
positions and retains at most four distinct public signing keys, with no full-roster clone/sort.
An opaque native summary attests BOTH copied key lengths for one exact group instance/epoch;
a borrowed credential-length check precedes the indexed OpenMLS copy. Actual restored oversized
signature/HPKE and credential fixtures pin this ordering. MLS loading and initial native roster
observation can still copy/traverse the complete roster and are not covered by the page bound.

An inactive removed receiver may observe its current public tree; catalog validity must never
be treated as local membership, native account/entry, file intent, helper route or work allowance.
Same-epoch replacement groups reject old views, and normal self-removal advances the epoch and
retires old views. Caller cancellation and local Busy cannot consume an unstarted proposed
nonempty source batch; sparse empty pages must advance and yield. The live native requester now
captures original Root/native ownership. Signed helper and nested exact-source responses are
authenticated before staged ciphertext; both AEAD success and rejection repeat original job,
bridge-effect and KeepWork checks before storage or durable bad-candidate verdicts. Preparation
reserves route custody before durable debit; subsequent commitment consumes source coverage.
Local pacing and at most two later Unavailable sweeps preserve finite work without pretending
that a negative batch proves global absence. Four focused TCP/native bridge scenarios pass;
combined programme and physical network acceptance remain open.
Finite Keep allowance may expire
before a large catalog is exhausted; budget exhaustion must never be reported as global absence.

Streamed external publication now creates an absent destination with a same-filesystem hard link,
refusing collision without an overwriting fallback. Failure cleanup removes only staging, never
a destination another local action may have created. Successful linking remains committed even
if staging cleanup fails. This protects the destination-collision boundary, not against a same-user
process with authority to modify the source `.part`. The separate private Download journal above
implements encrypted acquisition resume, with a fresh external export rather than a saved path/offset.

## Connected file-source activation checkpoint (19 October)

Source kind30 requires current native account/exact entry, actor generation, Root membership/
epoch/policy and explicit authenticated P2P permission. An unknown/unbound sender cannot spend
an original requester's quota or start storage. Finite current-sender ingress debit precedes
signature work; current C/B MLS keys, fixed helper, exact local source, full group/epoch, signed
window and bounded freshness/replay/work checks precede read admission. A receives no claim of
a direct C connection. The captured B descriptor/proof/actual connection incarnation cannot be
refreshed by a replacement; native freeze/unlock likewise creates new work authority only.

The source worker carries no Root, mutable store or signing key. A process-stable source credit
is shared independently by the actual worker, pending metadata and final response. Cancellation
and panic cannot recycle another owner's memory allowance. Only current positive storage plus
native/Root/route scope and the exact two-second deadline authorize a Present send; reply receive
does not cancel that independent final gate. Synchronous final admission is the boundary: later
revocation cannot retract bytes already submitted. Signed negative replies establish observations,
not global absence or durable remote custody. Helper29 now serves exactly one authorized hop,
with one prepaid1MiB chain per original C/group,4/group and8/process. Its five route records are
reserved before admission; four timed-out lower requests cannot strand a later admitted child
or release the charge early. The selected source must equal the current named DeviceId AND
captured MLS key, not merely any signer listed in the grant. Final Found keeps that A's original
two-second scope and parent C scope, excluding reply lifetime. Lower bytes and output inputs
retain independent same-credit owners across final refusal and cloning. No dial, recursion,
URL fetch, cache write or file-wrap key crosses this worker. Requester/Keep and combined bridge
acceptance remain unfinished; none of this implements live voice V2 or media relay.

## Congested driver and queued recovery boundary (PR 48 revision)

A signed current-epoch Since requester can hint that a held document lacks its frontier. This
cannot open a document, create membership/source proof, dial a peer or authorize imported data.
Unknown hashes need not exist. At most one hint/document/full member is admitted each 30 seconds;
256 retained entries refuse new identities when full after expiry pruning, and endpoint rotation
cannot refill the same member's budget. Actual connectivity/local membership are required. Hints
use the existing bounded recovery queue without evicting work; saturation can defer them until
periodic reconciliation. A dirty bit on an older owned wait prevents stale empty completion from
erasing newer admitted intent, even through legacy fallback or hint-table expiry. This adds bounded
retry work chosen by authenticated members; it establishes no universal latency or congestion SLA.

Application congestion cannot suspend swarm responses/connection revocation. Shared affine
credits bound declared queued ingress/response custody (64 MiB/driver, 128 MiB/process), with
separate count and per-peer stream limits. This is not a bound on libp2p codec/socket or total
application memory. Rejected requests fail honestly; local gossip loss requests ordinary signed
reconciliation and grants no authentication or freshness evidence. Old queued input is never
rebound to a same-peer replacement connection. Coalesced lifecycle events retain original issuer
versus carrier distinction, and recovery wakes cannot be starved by path churn.

The released two ordinary per-peer request owners are preserved, with one additional reserved
maintenance owner for kinds 2/4/12/26. The tag is an untrusted capacity hint, not an identity
or service grant; malformed and forged maintenance frames still fail normal Root checks.
All driver/process byte, record and request totals remain unchanged. Classification remains
charged through last actual payload/responder/lower ownership. This prevents ordinary traffic
occupying the membership owner, but does not establish progress for every request under
continuous competing notifications or bulk work; combined-load acceptance remains open.
The three-owner peer bound is an explicit denial-of-service tradeoff: six Noise identities can
fill a driver's sixteen total request slots, compared with eight at the old two-owner bound.
All sixteen still require actual last ownership to retire. Two ordinary owners preserve released
overlapping legacy signalling capacity, but neither classifies voice separately nor provides an
ACK/retry delivery guarantee for that protocol. Aggregate bytes/records can also cause refusal.

Automatic cached/reciprocal/probe work captures current Root, epoch/policy, descriptor and
connected proof/lease authority before transport waits. Driver admission rechecks it; Root
completion checks it again before peer-health effects. Pre-commit refunds bind to an opaque
original policy-window owner. Notification ACKs are local submission evidence only, and driver
reply disposal prevents a hostile helper accumulating unneeded answers in Root completions.
Actual lower custody survives waiter retirement. Local reconnect/manual immediate ACK paths
retain their existing authorization and limits and are outside the detached-wrapper claim.

File searches rotate beyond absent/busy preferred candidates without trusting them as holders.
Actual connected eligibility is separate from preference rank, and each chunk/round has a
320-peer tried cap while existing deadlines, page bounds and four/eight/one attempt limits stay
authoritative. Different wrapped keys for the same ciphertext are authenticated independently;
an opening failure rejects only the exact reference, including across retry rounds.

Quiet document reconciliation uses one owned monotonic 30-second timer, gated by active local
membership, open documents and an actual connected bound current member. It adds bounded
periodic requests to an existing member, without adding dial or membership authority. It
preserves source cooldowns, original request ownership and queue limits. Cancellation and ready
incoming traffic cannot indefinitely postpone a due local turn; deadline overflow disables the
wake. Diagnostics do not create work. This cadence reveals periodic activity to connected
members and does not guarantee progress when every eligible source is unavailable or malicious.

Automatic receipts cannot wait ahead of inbound event polling. Their detached jobs retain exact
member/descriptor/connection/deadline authority and actual lower custody in the existing 32-group/
64-process submission pool. A bounded persistent admission turn prevents receipt backlog from
monopolizing each newly released slot ahead of repair forwards; this does not establish broader
cross-group fairness or progress under all aggregate pressure. Busy intents remain queued, while
admitted best-effort notifications may still fail. Only authenticated remote receipt changes
confirmation evidence; local queue/ACK observations cannot do so.

## Detached discovery boundary (2026-10-18)

Connected maintenance holds no mutable MLS/store owner while waiting on peers. Driver authority
and fresh actor completion reject old root, membership epoch, effective policy, known descriptor
replacement and disconnect/reconnect results before positive evidence or empty-offer deletion.
The first authenticated descriptor can be pinned while reciprocal finalization is outstanding;
subsequent replacement is irreversible. UI lock does not revoke ordinary authenticated discovery.
PEX still proves a current member signed a request-bound response, not the transport endpoint's
identity. No existing peer-bound authentication is relaxed and no private dial authority is added.
Quiet discovery also polls the released membership-commit exchange before data reconciliation.
Its independently paced rotating bound-member selection cannot be suppressed by PEX backoff.
The selector creates no new dial authority and changes neither request authentication nor PEX
health. Coalesced poll/full-discovery/finalization obligations share the existing bounded queue.

Historical endpoint-bound identity proofs remain conflict pins after disconnect; clearing them
would let weaker forwarded legacy evidence replace an offline member's transport binding.
They do not prove that a restarted member remembers this endpoint. Separate bounded readiness
records only verified outbound kind26 completion under its original live connection lease and
member/device/epoch/policy. Serving an old request cannot stamp readiness, and a held old reply
cannot adopt a replacement lease. Delayed disconnect events retain fresh live-generation readiness.
Known current member candidates on inbound-only connections may receive connected-only repair
work; arbitrary Noise peers gain no work, membership, file authorization or dial permission.
Unsupported legacy transports keep event-fenced scheduling rather than exact driver-generation
evidence. Listener persistence still requires actual local outbound Noise observations.

Actual loss of a bound current member may expedite two existing consented local-route dial
passes in the actor. One original 25s window and 60s re-arm cooldown coalesce member flaps without
extension, queued events or quota refunds. Normally the passes occur 5s and 20s after loss; late
owner polling cannot burst missed opportunities. Each due turn repeats current P2P/member
eligibility and uses the released exact-route/roster checks and process-wide endpoint limits.
Unknown Noise endpoints and stale driver-live disconnects cannot arm this hint. File reads
themselves still cannot dial. A 250ms local driver-acknowledgement deadline bounds the owner wait,
not the lifetime of an admitted command: its original driver-owned permits remain charged and
may complete once after caller timeout. Existing submitted sockets cannot be recalled. Budget
denial can still defer progress to normal discovery; this is bounded acceleration, not a promise
of connection or delivery. No extra PEX or persistence work is scheduled by the retry timer.
The membership-commit poll's exact request/nonce/epoch signature remains unbound endpoint evidence. Only existing MLS
authorization applies commits; removed requesters fail the serving peer's current roster gate.
Empty or failed probes cannot prove currency or suppress ordinary discovery. A separate 1024-
record cap precedes per-record allocation, promotion and MLS processing, alongside the existing 16 MiB
response cap. Older default peers retain at most 256 commits; larger configured legacy responses
above 1024 records refuse, while this build serves larger local logs over successive epoch polls.

Actual lower custody bounds two flights/server and four/process, including timed-out streams and
retained completions; per-target occupancy also excludes overlapping stages until actual release.
The worst case is 32 MiB/server and 64 MiB/process of retained answers; bounded signature/bundle/
record decoding scratch is additional, and the signature transcript is released before decoding.
Queues/waiters and one supersession refresh are bounded. Shared capacity retirement wakes only
queued actors. Signed-record mutation requests sealed core persistence independently of chat and
rendered route changes; duplicate/liveness events do not cause disk writes. Transport frames may
allocate up to the pre-existing lower frame ceiling before the narrower per-kind response cap.
Automatic cached dials and mesh repair now use the bounded detached submissions described above.
Local reconnect, explicit manual redial and rendezvous waits retain separate scheduling boundaries.

## Voice V2 implementation boundary (2026-10-18)

V2 foundations are not a live forwarding or endpoint-authentication guarantee. Typed native
resource admission bounds engine work and survives lost UI waiters; it grants no signing or
plaintext delivery authority. Current-member/directory/policy queries are snapshots, never
permissions retained over awaits. The native packet ledger has a one-time allocator claim per
join lease, exact ciphertext retry identity, bounded replay and exact AAD/sequence/ciphertext
delivery binding. Unused native watcher/active-join owners additionally reject stale challenges,
renewal regressions and removal/re-add reuse; a public announcement alone grants no liveness.
Root's native lifecycle registry now calls synchronous member retirement inside the actual observed
membership transaction, before re-add and fallible additions. Retained
presence clones remain revocable and charged after their visible row disappears. Its caller must
still connect the native presence/three-flight confirmation owners, current offer ownership and
actor-turn authority before exposing them through IPC. Native confirmation binds Submit to the
current action/reservation, keeps validated progress through capacity failure, and consumes only
opaque exact signed-flight receipts. Pair pins outlive roster refresh; only successful newer-pair
promotion retires old authority. These unused native paths do not themselves enable live calls.
The standalone WebCrypto broker does not make those decisions. Browser-managed randomness and
secret memory remain browser responsibilities; no RAM zeroization or signalling ratchet is claimed.
Legacy calls remain in use until V2 integration and compatibility acceptance; their existing
recipient/replay and arrival-time presence residuals are not closed by these unused codecs.

Process/helper foundations separately retain original-issuer limits and immutable helper
packet/final-edge replay state across controller and actor changes. Detached effects capture
irreversible actor/session/policy and both issuer/final-member generations; remove/re-add cannot
revive them. Desktop actor attachment now shares the exact process pool and captured session ticket;
native row removal and pre-await lock revoke old owners. A separately committed UI authorization
generation prevents an older unlock from clearing a newer lock while revocation hooks wait.
Voice attachment exhaustion never grants authority and does not block otherwise valid vault/chat
access. The directory-incarnation observer runs for newly accepted authenticated operations,
including compound deletion/recreation, buffered dependencies and prefix-applying errors. Local
no-op, remote duplicate and rejected operations cannot replay deletion authority. Narrow raw-column
budgets precede expanded-change allocation; this does not close legacy CRDT admission or R6
semantic-binding limitations. Driver authority checks are the lower
admission boundary; revocation cannot recall bytes already submitted. Actual lower/worker owners
retain affine flight capacity after cancellation. The V2 logical clock advances through wall
rollback using local monotonic elapsed time, so pruned protection cannot become valid again.
Clock corrections can impair interoperation; this protection is process-local and restart ends
all joins. No helper confidentiality/live-routing claim follows from this unused foundation.

The inert V2 transport seam bounds framing before copying/verification, charges requester/process
work before signatures, and checks the affine exact-frame admission again at final origin commit.
A helper's valid signature cannot claim final endpoint admission: status and responder are bound
to the original envelope's role/target. Typed prepared replies retain verification and exact first
helper/public admission custody through byte clones and the driver's serialized lower copy.
Original requester/actor/policy, route/epoch/descriptor/proof/connection and five-second admission
deadline suppress delayed sends; reply-wait retirement is deliberately excluded. Cached helper
bytes cannot adopt a replacement actor/room or extend their first permit to claim current custody.
Room fan-out receipt validity does not require every original edge to remain live. Driver-owned connection leases revoke before event queues
are drained, so same-peer disconnect/reconnect cannot revive queued helper or maintenance effects.
Native Root descriptor/proof/policy/epoch gates remain additional checks. Source work shares that
driver custody while retaining distinct attempt/ACK settlement, and encrypted source grants require
the exact native cached packet, current pair and current action. Controller/call parent revocation
closes watcher activity before another actor turn. The trusted public-control queue now commits
original allowance with exact native recipient custody; announcements alone never admit presence.
Encrypted endpoint delivery and UI V2 mode remain disabled. Legacy transports without connection-lease support retain event-time
maintenance fencing; upgraded helper work refuses them. Already submitted bytes cannot be recalled.

Affine production call grants retain their exact room allocation and 256 declared bytes through
actual last-owner drop. Synchronous revocation closes effects without refunding those bytes.
Room owns only lease revocation handles, so retained grant custody creates no ownership cycle.
The public Rust `from_lease` adapter is a trusted lifecycle boundary, not membership enforcement;
the production native bridge obtains charged grants from its exact product actor. Endpoint start
additionally recognizes exact actor/policy and room issuer registries. Root keeps only Weak call
custody, so losing the last native owner cannot leave its LocalJoin alive. Native public-key checks
are structural SEC1/key-reference checks; WebCrypto import must validate the curve point before
derivation/confirmation. Prepared encrypted ingress leaves replay and tokens unchanged until the
final origin transaction succeeds, but live Group/Policy endpoint admission is not yet connected.

Clock housekeeping can collect cold caches while locked but cannot reconstruct authority. Group
membership/epoch/revision evidence and native pair/replay/allocator tables survive collection;
live origin pair pins retain exact key bindings. Confirmation source work captures the original
deadline even when no collector runs. Source deadlines are immutable reservation properties,
with wall expiry checked independently. Actual lower/payload owners retain credits beyond cache
expiry, and uncertain WebCrypto operations have no TTL refund. No collector is entered through a
resource charge callback or while holding the process registry across ledger locks.

Pending crypto authority captures native pair/action/token revocation owners rather than trusting
public derivation/flight data. Sign and token consume retire pending engine eligibility; actual
engine/payload credits are independent. Native confirmation restart retires old Proof/Finish
tokens permanently, including attempts to obtain a new grant for that old token. Hello proposes
a new attempt and instead binds its native stored sequence to the pair's accepted floor. Only
closed validated native progress advances that floor; a lower completion cannot revoke a queued
newer Hello. Final token commit repeats the fence. No pending grant converts into historical
success. The future Endpoint queue still needs prepaid exact-result receipts, current OfferId/
attempt binding and native lost-reply recovery before these seams can enable live V2.
The inert browser confirmation registry now keeps exact frozen computation/result custody
through admission/commit uncertainty in the surviving broker, with a combined 32-record cap
and shared conservative byte/engine limits. Engine failure stays failed; temporary queue
pressure stays retryable. Full closed replies bind exact identity/input/result tags; no
confirmation plaintext reaches RTC and historical success grants no fresh pair authority.
Explicit retirement wakes callers while actual admission/engine/RPC promises retain custody.
Native Coordinator action registration/admission/settlement currently compiles only for contract
tests; it establishes no production IPC enforcement. The sync typed-action factories remain
inert production Rust seams. That native model checks exact opaque Root action, native call
lease/scope/process-pool registration and idempotent actual engine admission. Settlement latches
before a delayed admission, and first signing/closed-confirmation effects serialize against the
same fence. End/lock/replacement cannot refund an admitted engine. The combined native queue
retains at most 32 pending/terminal identities. Small terminal metadata now shares the action's
prepaid record and fence: native hashes exact bounded returned bytes, stores accepted and closed
rejected outcomes, and checks the original clock and post-transition owners on retry. A retired
receipt remains historical metadata; compatibility effect APIs reject it. Closed validation and
checked state preparation precede token consumption/signing, followed by infallible publication.
This seam has no browser commands or complete continuation/result dispatch integration.
A capacity reserve releases only unused bytes on close, preserving actual loans and
slots; closing it is not evidence that a workflow or uncertain engine has completed.

The narrow SDP foundation prepays 224 KiB, one Crypto lane, two Forward lanes, two continuation
slots and ordinary endpoint/helper origin records before a nonce or positive incoming admission.
It supports one local and one remote 8 KiB SDP, one Source record and two actual lower holders,
bound to one native pair/OfferId and fixed initiator/responder role. Crossed Offers refuse before
ACK or replay mutation. At most two current helper identities are immutable for that exchange;
changing a helper cannot allocate another protected lane or reset original traffic allowance.
Readiness and work deadlines cannot extend each other; actual lower/engine custody survives them.
A bounded zero-copy untrusted header peek can select only this existing exact capacity owner;
it authenticates nothing and cannot skip signature, membership, scope, replay or origin checks.
Retryable affine ACK completions retain their original request/response/authority and repeat
those checks; a busy reserved lane consumes no requester allowance. Native budget composition
tests do not prove browser-copy ownership, general SDP action integration or RTC delivery.
Full-chain integrated progress and native/browser/RTC result recovery remain requirements.
General Pair.open is not a live adapter path;
SDP/ICE still need native current-offer receipts and charged dispatch/result ownership.

## Pre-upload video editing boundary (2026-10-19)

A video added to the chat or announcement composer, a wiki page or the Files tab opens
`VideoEditor.svelte` before upload, unless Shift was held through the drop or the picker click.
This is a sender-side product feature with no protocol, wire or native-command change: the result
is an ordinary `File` that goes through the existing `begin_file_upload` path, limits and
magic-byte check.

What the honest sender's client guarantees:

- **Removed time is absent from the uploaded bytes.** An edited video is decoded and re-encoded
  from the kept frames only (`copy: false`); no packet is ever copied. A packet copy would have to
  start on a key frame and so could carry seconds of cut footage into the file, hidden from a
  player but present for anyone holding the bytes. A frame is encoded only for the part of its
  display time inside a kept range: a frame wholly inside removed time is never encoded, and the
  frame already on screen at a cut point (it began up to one frame earlier) opens the next range,
  which is also what the preview shows there. Mediabunny's encoder-support probe encodes one frame
  into a discarded null target that never reaches the output. Verified against real codecs in
  headless Edge (MP4, MOV, WebM), not only in unit tests.
- **Descriptive metadata is not carried into edited uploads.** The output container is written
  fresh with an empty tag set (`tags: {}`), so the source's title, comment and the location or
  device tags phone footage often carries are not copied. Checked end to end for title and
  comment; other tag kinds rely on the muxer writing only what it is given.
- **Output is bounded** by the server's upload limit (or the 1 GiB protocol ceiling while that is
  unread): `CappedBytes` refuses to grow past it and the export stops. Assembling the final Blob
  briefly holds about twice the output.
- **Parsing surface.** Decoding uses the WebView's own decoders, as previewing does. Mediabunny's
  JavaScript demuxer is new parsing surface on top of that; its worst case is a hung or crashed
  page, not code execution outside the WebView. The editor opens the sender's own pick, but that
  can be a video previously downloaded from a peer, so peer-authored bytes can reach it. Inbound
  inline media and the native-decode rule for images are not changed by this path.
- **Video with transparency is refused** before editing: Mediabunny would need blob-URL workers
  for it, which the CSP blocks. The test is whether the first packet carries alpha data, not the
  container's alpha flag (Chrome's recorder sets that on opaque canvas captures too). A file whose
  alpha only starts later fails at export, closed, with the generic message.

File drops (2026-10-19, follow-up): the main window now sets `dragDropEnabled: false`, so files
dropped from the OS reach the page's drop zones as HTML5 drops (with Tauri's default, wry handled
drops natively and switched external drops off for the whole webview). Three browser defaults come
with that, and `drop-guard.ts` closes each for drags that start outside the page:

- a file dropped outside every zone would navigate the webview to it; a link dropped anywhere
  that is not a text field would be followed (there is no back button in this window). An
  unaccepted `dragover` gets the drop effect `none` and the `drop` has its default prevented.
  Only enabled, writable text-entry fields keep their native text drop, and visible
  `<input type=file>` controls their native file drop; every other control (a checkbox, a slider,
  a read-only field) is guarded like the page, since the engine's fallback there is to navigate;
- a file released over an embedded third-party player (the `frame-src` sites) would be delivered
  to that site's page. While files are dragged over the window (from outside or from within the
  page) every iframe has `pointer-events: none`, so the drop lands on this page beneath it: in the
  chat that is the chat's own drop zone, which attaches it there; elsewhere it is refused. The
  rule is in `drag-drop.css`, a stylesheet loaded from the app's origin, because the shipped CSP
  carries a Tauri nonce in `style-src` that makes the browser ignore `'unsafe-inline'`: a
  `<style>` made at runtime or a `style=""` attribute would be refused in production while
  working in development and every test. The shield lifts on drop or dragend, on the first pointer
  event after the drag (none fire during one), or after 3 s with no drag event; it does not lift
  on a short silence, since `dragover` may come only every ~550 ms or only on movement.
  **Not covered**: a drag that enters the window directly over an iframe without crossing this
  page first is seen only by that frame;
- the drop zones accept file drags only, with the effect pinned to `copy`, and their drop handlers
  ignore anything that is not files, so text dropped on a composer or the wiki editor is the
  field's own native drop, and the source app never sees a "move".

The page receives only a dropped file's name and contents, never its path (Tauri's handler used to
deliver full paths, though nothing listened). Nothing native listened for Tauri's drag-drop events.
Whether a real OS drag reaches the zones, and whether `shiftKey` is set on such a drop, has not been
exercised by any automated check; unit tests pin the configuration, the guard, the shield and that
no production module builds a runtime `<style>`.

What it does not claim:

- An **unedited** upload sends the original bytes, metadata included, exactly as before this
  feature existed. That covers the editor's one action on an untouched clip, the original-file
  button when the editor cannot open a file or an export failed, and anything added with Shift.
- "Post now" sends the composer only if it still holds what it held when the editor answered
  (text and reply target; for an edited clip that is after the export, while the modal blocked
  typing); otherwise the video is attached and nothing is sent. It is the composer's own send, with
  its own guards, not a separate path.
- Nothing here is enforced against a modified client or verifiable by recipients: they receive a
  normal video and cannot tell whether, or how, it was edited. The original stays on the sender's
  disk; the editor never touches it.
- Re-encoding is lossy and may change codec (QuickTime becomes MP4/H.264; WebM becomes VP9/Opus).

Clip maker (2026-10-20). A clip with anything added (text, stickers, other videos, music, sounds)
or its own sound changed is drawn and mixed frame by frame by `exportClip` (`clip-export.ts`).
The properties above carry over, and for the same reasons:

- **Cut-out time is still absent.** Frames are drawn only at kept output times, from each video's
  kept source times (`layerSchedule`), and sound comes only from kept ranges (`soundPieces`).
  Checked in headless Edge frame by frame and by frequency, not only in unit tests. The file is
  written fresh from pixels and samples; no packet, tag or file name of any source reaches it (the
  output keeps the picked file's stem as its name, as before).
- **Media is local.** The clip maker reads files the person picks or drops from their own disk,
  emoji drawn by this window, sounds synthesised here (`clip-sfx.ts`), and Studio songs read from
  this device's own Studio store. Nothing is fetched and no `catcoms-media:` URL is read; server
  stickers and shared Files media are deliberately not offered yet. SVG and AVIF are refused as
  pictures. Every file is size-checked before decoding (`MEDIA_LIMITS`), and because bytes say
  little about decoded size, each is also bounded by what it decodes to, before decoding: a
  picture's dimensions are read from its header (`imageSize`; for a GIF, its screen grown to hold
  every frame, since the browser decodes a frame larger than the screen at the frame's size) and
  anything over 40 megapixels is refused, then kept at most 2048 px; a sound is decoded only as
  far as the clip can use (its length plus 30 s) and never past ten and a half minutes of samples
  at 48 kHz. Sound files are demuxed by Mediabunny (all its containers) and decoded by WebCodecs,
  the same surface as the video path; a codec WebCodecs lacks (ALAC, ADPCM) is refused, not
  played some other way. The session holds
  at most 64 media items and 1 GiB of decoded media. Counts in the document are bounded (24
  overlays, 32 sounds, 3 videos, 900 keys a path) and composed clips at 10 minutes and 1920 px.
  Those budgets are asked BEFORE anything large is made: decodes and song renders run one at a
  time through one queue shared by every editor (`JobQueue`), each admitted against its bin's
  remaining room before it allocates its output. The bound holds across editors, not only within
  one: closing an editor does not free what its running job holds, so an offline render (which
  cannot be interrupted, and holds its whole output until it ends) keeps its turn to the end
  however its editor went or whatever the person stopped. That rests on an assumption: a render
  always ends, because the engine's suspend callbacks and drum hashing all complete; a render
  that never ended would hold up every later heavy job until restart. A job that holds no media
  yet (waiting on a Studio read, say) is let go of as soon as its editor gives up on it, however
  many hang. A decode holding its output that stalls (a crafted file) can be let go of too, so it
  cannot block every later editor, but only one such job at a time; a second waits until the
  first ends, and since the queue runs in order, two stalled decodes block every later add in
  every editor (song reads and renders too) until restart. At most two jobs
  holding media are therefore unsettled at once, each within `MEDIA_LIMITS`. A
  cancelled editor starts no queued job, and a render, mix or decode cancelled part way never
  reports success, starts no further allocation, and releases the pictures it held. Posting waits
  until nothing is still being added, or until the person stops what is still pending. Added
  videos stay files in the bin (probed, decoded only frame by frame while previewing or
  exporting); limits on their size and length are a recorded follow-up.
- **Studio songs are peer-authored content** (the channel's Studio is shared). They go through the
  same validation and resolution as pressing play in the Studio (`scorePlaybackFromNative`,
  `planJamArchivalPlayback`, the take limits), and a song that cannot be resolved is refused with
  the Studio's sentence, never replaced by another version. Expired songs are not offered. A song
  is checked against the channel's index again when it is loaded and again when the clip is
  posted (`checkSongsOffered`), so one that expired or was deleted while the editor stayed open is
  not posted; one that expires while the clip itself is being exported still is. They are rendered by the real
  `JamEngine` into an `OfflineAudioContext` that is never connected to the speakers; the
  `runningView` proxy only tells that offline engine its context is running. Putting a song into
  a clip posted elsewhere on the same server is the person's own act, like quoting a message.
- **Text effects** are drawn from the app's own stylesheet rules, frozen per frame into SVG
  `foreignObject` images loaded as `data:` URLs (a `blob:` one would taint the canvas). Layout
  happens in an off-screen shadow root outside `<body>`, so the chat effect runtime never adopts
  it and nothing makes sound; the library's previews sit in `.text-fx-key-preview`, the runtime's
  silent picker-preview class. The page itself styles only through CSSOM and constructed
  stylesheets, which the shipped CSP allows. Each snapshot carries its own `<style>` element and
  `style=""` attribute INSIDE the SVG image: an image document is not governed by the page's
  `style-src`. Checked in Edge under the shipped policy with Tauri's runtime nonce added (the
  page's own inline style was refused in the same run, while the snapshot's styling applied).

It does not claim that recipients can tell how a clip was made, that emoji and text look the
same on every machine (they are drawn with the sender's fonts), or that two renders of the same
music are bit-identical (they differ by float rounding, about -140 dB).

## Volatile Studio edits and orderly lock recovery (2026-10-18)

Working editor projections are unconfirmed local intentions. They never replace authenticated
native views, causal registers, recovery evidence, group binding or authorization. Delayed first
delivery checks its original epoch/trust binding again after asynchronous preparation; exact
retries cannot silently adopt a new epoch or ticket. Only never-submitted entries coalesce.
Submitted identities and ambiguous outcomes remain immutable until ACK or explicit discard.

The encrypted UI continuity record may contain a bounded owned Studio queue for orderly lock
recovery: 512 records, 8 MiB of PIX file bytes, 12 MiB serialized Studio data, 16 MiB total envelope and
the original independent 1 MiB non-Studio allowance. Admission includes UTF-8, outer escaping and
future request/diagnostic reserve. Disk reads check physical size/regularity and use a bounded
read before authentication. Restore validates typed requests and original scope/group, holds
them until an explicit action (Save, retry, discard, or leaving the record's own document, which
saves it), and never restores a native view as authority. Opening a document never sends its held
records, and admission re-reads a document it holds no view of before comparing a record's
authoring binding; an unread or superseded read sends nothing and does not count as an attempt, so
a later retry is admitted afresh. No plaintext
browser fallback exists. Lock proceeds immediately even if raster staging/serialization fails;
the failure is disclosed and previous sealed continuity is preserved. A hard crash between a
native commit and continuity capture can still lose the latest receipt: this is not a write-ahead
log or crash-atomic Studio transaction. The existing lock coordinator retains its immutable
continuity argument until unlock for exact racing-close/retry ownership; clearing the editor is
not a RAM-zeroization claim. Older clients may reject envelopes over their old cap.

Local media previews validate the complete working composition and actual native group/channel;
failed/uncertain Save state is checked both before and after asynchronous preparation. An ACK of
the same pixels/music does not revoke playback; changed effective content, actual overlay binding,
settlement, membership and lock do. Data-only update notices re-read before semantic comparison.

## Closing retry and receive cardinality correction (2026-10-18)

An accepted Closing retry can bypass global inventory only to authenticate the actual scoped
intent file, match current local membership/author, typed envelope, basis and retained branch
evidence, then flush that same file and its parent unchanged. Terminal disposal also checks its
own historic provenance. No new acceptance, source mutation, ciphertext replacement or partial
budget is allowed on this path. Failed flushes return no acknowledgement and rotate both intent
and inventory freshness before I/O. New authoring and commits still require complete inventory.

Preserving a native Save receipt must not restore its older projection over a later Read. Frontend
actual-read serials fence that display update independently of acknowledgement, and reconciliation
is awaited before dependent first-frame work. Navigation-only fences are excluded from those
serials. A pre-Apply epoch refusal is known not written; an earlier ambiguous attempt remains
uncertain. Shared sound-file reads separately enforce a native 4 KiB cap, sixteen current recipes,
serial registration-aware cancellation and post-await scope/trust/listing/session checks.

The receive inventory's cumulative record ceiling is now 1,024, matching its existing directory
ceiling; 64 was incorrectly treated as sufficient for a vault containing several records per
document and multiple local servers. The 8 MiB authenticated-read and 256 KiB cold-validation
limits, restart accounting, per-cursor-step bound and detached worker pool remain. Existing
synchronous complete-scan callers may authenticate more small warm records in one custody visit;
they have no 250 ms guarantee. Resumable InventoryRuntime visits retain their existing deadline.

## Studio inline media and sync diagnostics (2026-10-18)

The existing Media preference now admits PIXA to its bounded application-owned decoder; filename
and MIME are screening hints, never proof of content, author or membership. Native authenticated
manifest caps, 9 MiB input limits and complete PIXA/PIX/take validation precede rendering or synth
admission. On-demand and author blocks remain effective. One inline manager serializes native
reads, retains two packages and 128 observed hosts, expands current rasters only and owns one
repaint timer. Silent visual loops do not resume audio. Sound requires a local gesture through
the existing StudioAudioSession. Token-owned teardown cannot stop a newer card's audio; failed
starts do not claim playing state. Late IO/audio results cannot repopulate revoked scopes.

Post to chat preserves draft text and normal durable Send identity. It uses the returned fileshare
CID, never a Studio blob address, after streamed upload. Navigation including ABA, membership,
lock, readiness and close intent fence every asynchronous composer mutation. A late upload can
remain a file without becoming a new unsaved draft after continuity has already been captured.

Sync pause reasons are bounded local diagnostics, cleared on resume and lock, and never authorize
repair or claim corruption/settlement. Whole explicit access must succeed before resume; old Read
ACKs cannot clear newer authoritative pauses. Lossless actor sequence/native incarnation order a
bounded cache of 128 locally selected server observations across navigation. Reset or eviction
forgets the watermark and requires a fresh successful Index read to reconcile current state;
the diagnostic UI does not claim replay protection beyond that cache/session boundary.

Owner repair remains an explicit decision between actual receipt candidates in Server settings.
Normal maintenance and already selected repair work resume automatically; neither the UI nor a
timer invents a winning receipt. Missing owner tenure history cannot be fixed by elapsed time.

## Gate 7 UI completion candidate boundary (2026-10-17)

Main-window repair commands now expose only the existing actor/store owner decision. The
renderer echoes two actual candidate receipt hashes and one selection; it cannot supply tenure,
receipt bytes, sequence or signing authority. The backend rederives the pair under custody and
requires current verified/durable owner authority. Scheduling is not completed repair. UI reads
and choices invalidate across source and Index/Registry notifications, including in-flight reads;
failed discovery never hides the scoped repair controls. Native reply loss after scheduling is
uncertain, not proof of no write. Restore/Copy remain ordinary content Saves, not fault clearing.

Closing Save is a distinct local draft route with exact basis/branch/nonce/body. The actual signed
close comes only from the local sealed owner journal and is rechecked against current source and
Known tenure for new authoring. Lookup failure is carried as data through terminal classification;
it cannot authorize a new capture and cannot strand an accepted retry. Captured plans retain only
that verified close and still recheck membership, tenure, source version, basis, target and media
possession at S3. Missing/legacy/repair-bearing close state refuses fresh work. Ordinary peers
usually lack that journal; UI availability is not universal. Local draft Save grants no receipt,
shared publication or new network authority; automatic reviewed handoff remains separate.

PIXA deck/global playback validates complete bounded packages and reuses the archival synth and
remote provenance. Declared MIME/filename and package labels are decoder hints, never membership
or author authority. Listing≤9MiB, native requested cap against authenticated plan, encoded and
decoded caps, complete codec checks, four native cancellation debts, one active/one replacement
read and at most two encoded packages bound the path. Only the current raster expands. Explicit
local gestures may resume audio; remote heartbeats cannot. Call/session/lock/membership/Deafen
and local suppression retire pending reads and playback; returning to a previous value does not
restore currency. The global viewer cancels on close/navigation and never auto-plays incoming files.

## Gate 7 evidence boundary (2026-10-17)

Gate 6 is owner-accepted and merged at `82631a81`. Gate 7 composes existing enforcement paths;
it adds no wire/persistence authority. [GATE7-ACCEPTANCE](GATE7-ACCEPTANCE.md) distinguishes
actor/vault, real TCP, native conversion and mocked frontend coverage. The composed rotation
test requires actual recovery-before-replacement, retains deleted SFX/export rejection even
under Copy, and exercises ordinary recipe recovery with exact retries and stale-preview refusal.
It does not convert backups into PIXA, publication into peer custody, or test engines into
audible hardware/platform evidence. Current UI obligations are in [GATE7-UI-HANDOFF](GATE7-UI-HANDOFF.md).

## Gate 6 enforcement checkpoint (2026-10-14; candidate at that date)

The candidate connects typed Score and complete Flipnote sound state to the existing causal/P1,
vault, native and UI paths. Parsing alone still grants no authority: explicit document-family
binding, current membership/tenure, exact retry, recovery preflight and durable custody remain
separate enforcement steps. [GATE6-ACCEPTANCE](GATE6-ACCEPTANCE.md) records bounded review pins
and passing complete local backend/native/startup verification, with hosted/Linux CI still pending;
old slice-only claims below describe
their original checkpoints, not current feature availability.

Archival playback reuses the existing synth through engine-minted capabilities bound to an owned,
validated schedule. Authored holds may exceed 30 seconds without weakening the live-call watchdog.
Scope/membership/lock/Deafen revoke admission, including a shared take still awaiting download or
approval; a remote heartbeat/unpause cannot recreate local consent. Natural completion is distinct
from failure, cancellation or revocation. Rendering and schedule expansion retain bounded work.
Native publication validates actual PIXA bytes; reference Save verifies exact held content.
Blob custody, provisional reference Save, a receipt, browser download and a saved local GIF are
different outcomes, not interchangeable success claims.

Frontend and backend SoundRecord recovery are separately implemented and bounded-reviewed;
final cumulative code review is clear and composed local suites/startup pass; hosted CI remains pending.
[GATE6-SOUND-RECOVERY](GATE6-SOUND-RECOVERY.md)
records exact historical selection, current link scope, media custody and permanent deletion rules.
An explicit selector may choose a retained historical alternative by operation id, but a whole-
version walk chooses one effective value/removal per slot, not all alternatives. Raw evidence
retention is not branch preservation; only archive preservation supplies that proof. Score
historical OBJECT replay stays manual and header-only Index Copy remains projection-level.
Gate 4's bounded lifecycle P5 promotion remains accepted, while direct Closing Save/unfinished
native repair commands remain absent and Unconfirmed drafts stay local/non-transferable.
Local emoji-sound import is not a verified shared catalog/chat-click feature. Shared-deck PIXA
consensus/global viewing and audible hardware/two-real-client release acceptance remain later work.

## Gate 6 native animation export boundary (2026-10-14)

Actual-byte PIXA publication validates whole framing, strict canonical take grammar, bounded
source labels/lanes/recipes/events, native PIX192x144 and authored SFX duration before the
existing persistent stage/promotion/flush. It creates no metadata authority. Local visual GIF
encodes one raster at a time under a32MiB writer and detached inline lease. Exact session and
server incarnation are rechecked through safe staged fixed-name publication/reveal; the commit
guard orders an admitted commit before a concurrent lock rather than undoing a committed export.
All ignored audio still validates. See `GATE6-NATIVE-EXPORT.md` for limits and honest durability.

## Gate 6 Flipnote sound core boundary (2026-10-14)

Sound Sets require an existing live frame and effective recipe at their sender's complete causal
frontier; receiver-only state cannot authorize them. Every mutation still uses exact signed P1
record/actor/body/key and same-property predecessor validation. A raw recipe Set winner cannot
override any live concurrent removal. Missing/deleted references after a valid concurrent edit
remain retained unready evidence, with cleanup available. Linked score ids and export declarations
grant no cross-object authority or blob-format/availability guarantee. Complete retained export
CIDs stay held even when deleted, expired, losing or overflow; patch hashes are not CIDs.

An optional sound snapshot trailer preserves complete records; empty trailers and source reuse
reject. Compaction omits whole removed slots, only under the existing recovery-before-replacement
barrier. Omitted historical deletions cannot be interpreted as proof of safe automatic replay.
The earlier native sound omission is closed: the shared fallible converter retains all admitted
sound evidence on ordinary, provisional, draft and recovery reads. A local Save refusal alone
would not have closed that ingress path. `GATE6-FRAME-SOUND.md` records the bounded core and DTO
reviews; connected UI composition and SoundRecord backend recovery have their own bounded reviews.
Final combined acceptance remains a separate obligation in the current ledger.

## Admin collision hardening (2026-10-13)

Current moderation-record roles also use the verified full signing-device identity.
Legacy origin labels supply attribution only, without inheriting an origin's admin
or owner authority. Historical-role proof and append-only audit completeness remain open.

The four-byte admin admission bypass in hardening item 7 is repaired on the separate
security branch: authoritative grants bind full current `DeviceId`s, with no companion
inheritance. Fresh Add rechecks permission after queueing. Legacy grants are discarded
without matching fingerprints; v2 rosters and versioned snapshot records use full IDs.
Native role-write failures leave delegated admissions blocked and report unsaved state.
Removal revokes exact grants and owner changes require fresh delegation.

The earlier R3 claim was overbroad on unpatched builds: a full-member collision twin
could invite an independent member. Updated owners reject that twin even if the genuine
admin has departed. An existing member can still disclose plaintext or certify ordinary
companions under the existing policy. Admission gives ordinary new-epoch capabilities,
not every historical MLS secret or other devices' private keys. A fixed-target search
has expected cost around 2^32 candidates under uniform hashing; no wall-clock benchmark
is claimed. Old builds/backups remain vulnerable; stale valid v2 roster replay remains
a badge/relay liveness issue, never an owner-local authority source.


This document tracks **what a modified ("hacked") client can and cannot do**. Mewtual is
peer-to-peer and open-source: anyone can run a build that ignores the UI's rules. The question
this document answers, action by action, is *which rules are enforced by cryptography / the
protocol (a modified client cannot break them) versus enforced only by the honest client (a
modified client can ignore them)*, and what we plan to do about each gap.

It is a **living backlog**. When a residual is closed, move it to the "Protocol-enforced"
table with the commit that closed it.

## Trust assumptions

- **Gate 6 export bytes are content, not authority.** The initial `studio-export.ts` and `pixa.ts`
  helpers validate resolved inputs, recipe hashes and exact framed derivations, then reuse the
  existing jam-take and PIX validators. They make no native calls and start no audio. `.pixa`
  checks the whole bound before copying one input snapshot, then component/count bounds and
  complete framing before raster decoding or individual frame copies. It rejects noncanonical/
  duplicate JSON and invalid UTF-8 and returns owned encoded frames, including for Buffer,
  typed-array subclass or shared-memory callers.
  Authored score holds and SFX durations are bounded and preserved, not derived from animation
  FPS. Overlapping same-row score holds refuse export. The connected candidate displays retained
  conflicts/readiness and uses opaque archival capabilities for authored holds beyond 30 seconds;
  raw live-call note admission retains its original watchdog.
  Export inputs are snapshotted before asynchronous hashing. Exporter/group labels do not prove
  membership, authorship of source edits, a receipt, publication or persistence. Current production
  callers separately enforce scope/readiness, actual CID/length binding, native validation and
  durability before publishing a reference. A leaf codec does not replace those checks.

- **Typed score parsing is not persistent score admission.** The Gate 6 `ScoreOp` codec reuses
  strict Studio parsing, recipe hash checks and complete-envelope bounds. It requires a fixed
  16-byte object key, but neither that key nor overlapping Flipnote title/patch body shapes prove
  a root kind, member, group or physical epoch. The separate typed ScoreTarget now enables
  stateful edits through causal/P1 validation, exact recovery/preflight, durable ownership and
  explicit native object-family binding; the codec itself is still not an admission capability.
  Numeric/array take session nonces now refuse in the shared recorder/import validator before
  regex coercion; this closes the external schema finding without changing legal take bytes.

- **Renderer access to retained Studio work is local authority, not protocol authority.** The Gate 4
  completion candidate registers the reviewed overlay lifecycle family and a ticketed Unconfirmed
  Save for awaiting-tenure previews. The session gate, command allow-list and actor/store custody
  still enforce the boundary if the renderer is modified. A ticket binds the local target, basis
  and branch; the request also freezes its nonce and canonical body. Exact retry cannot silently
  become a new edit, and a replacement ticket requires explicit re-authoring. The resulting branch
  is local-only and cannot become an installed source, owner receipt, handoff/signing authority or
  publication claim. Copy preview saves nothing; copy apply rechecks and writes one ordinary
  provisional edit. C4 reports a typed `written:false` stale-preview refusal only before entering
  publication. The honest session may explicitly preview again after that first clean refusal, but
  never uses a later clean refusal to erase an earlier may-have-committed Apply. The same exact
  nonce may cancel a first clean refusal locally, freeing only its bounded frontend slot and making
  no native call because the refusal proves nothing was written; a stale nonce cannot cancel its
  replacement. Save re-authoring likewise changes route/nonce only for a typed known-no-write
  attempt; ambiguous identities remain frozen unless the user explicitly stops tracking one
  through the separately worded two-step UI. That uncertain local-only action is also bound to the
  exact Copy nonce, performs no native mutation and does not claim to undo or disprove a Copy that
  may already have landed. Discard and archive release require exact confirmation literals. Direct
  Closing Save and native repair commands remain absent. A malicious renderer can request the
  exposed local actions and exhaust their documented bounded quotas, but cannot bypass their
  membership, tenure, freshness, storage, reference or destructive-confirmation checks.

- **A repair is bound to the repairing owner's tenure, not just their key.** The v2 repair
  transcript adds issuer tenure separately from the fault tenure. Live application checks the
  independently observed current tenure before exact-retry handling; old v1 remains historical
  only. The bounded book keeps full selected/losing evidence and the latest signed repair,
  revalidates them on restart, and screens its named loser across all three receipt paths.
  Different inherited baselines identify a losing branch; identical baselines do not reveal
  descendant ancestry. Third baselines still fault. New progress survives exact retries, and a
  newer fault cannot be cleared by replaying the old repair. The monotonic repair sequence is
  scoped to that authenticated issuer tenure, so a current owner can exhaust only its own tenure;
  checked allocation refuses at MAX before signing or persistence, while a verified successor
  starts at one. Unfinished source/journal provenance remains a cross-tenure barrier. No wire or
  persistence field changed. Latest-only retention is not an
  audit chain: a no-longer-covered old conflict may require another repair. The app-side candidate
  now consumes the one archived Observed witness only from a still-current durable sync snapshot,
  matches the receipts' full owner/start/tenure tuple, and seals a receiver-local exact-pair
  attestation before storage. Unknown, Imported, wrong and evicted unadmitted history remain
  unavailable by design. Registry head service authenticates and accounts its source before this
  B0 write, propagates every failed or uncertain write before classifying the source, and preserves
  Fault as a hard refusal afterward; prepared service does not cold-reconstruct that source. Studio
  head service keeps B0 ahead of its authoritative source read, so a Faulted or unprepared source
  still refuses service only after an independently authorized report is retained. A report
  naming exactly the pair the owner's source already carries a finished repair for is answered by
  that repair in the same response rather than staged again. This is declined only while no
  decision is held, the source is servable and carries the repair, and the repair verifies under
  the current tenure. Studio reads that repair from its warm, byte-verified source through a
  probe that cannot fail or delay B0. Otherwise the report stages as before, so a pair the owner
  can no longer answer stays decidable. Without this, a faulted peer's
  late reports reopened the decided pair: proof of the selected receipt stayed suppressed, so no
  newcomer could install the repaired document, and the view offered the pair for a second
  decision. Native repair remains unavailable.
  Studio fault repair executes only as Agent 3's detached job (design 10.3). Each actor runs one
  job at a time. A shared preparation-pool slot and a per-target live claim are reserved before
  any body read. The source is rebuilt detached, holding no store, vault key or MLS state. At
  commit the rebuild's context and plaintext digest are rechecked against disk and the unchanged
  authority-checked transaction runs. An offered repair is verified against the live owner and
  this device's authoring tenure before anything is captured, so a newcomer with `Unknown` or
  `Imported` tenure, or a repair signed by anyone but the current owner, costs no capture and
  holds nothing else. While a job owns a target, ordinary installs, page receive, owner rotation,
  preparation and foreground Apply into it defer or refuse. Own-operation replay and gossip
  ingest that needs source preparation wait through the same preparation check. Gossip ingest
  into an already warm source and Flow S, H and R writes (including the handoff's commit, H5, and
  Flow R's commit, R3) are not claim-checked; a change from them makes the rebuild stale, and the
  job writes nothing (an automatic job reruns; an explicit decision is reported abandoned, to be
  made again). S3 commits a rebuild only after installing it, and builds its storage budget only
  then, so a cold source over the receive scan's cold-byte limit is never validated inline.
  Registry bucket repair executes only as the same job, scoped to the bucket. This covers the
  explicit decision, Flow D, the owner's resume and the owed replacement. The bucket is captured
  as bounded authenticated bytes and rebuilt detached. At commit the issue/apply transaction
  receives the rebuild and uses it only if all of the following still match: mount, server,
  group, bucket, actor, designated owner, MLS epoch, plaintext digest and physical size, plus a
  fresh read verified against the live budget. Its writer re-reads the bytes once more before
  writing. The authority checks are the custody path's own: V5, channel, durable snapshot, owner
  refusal for Flow A, and `verify_current_owner`. Offered bucket repairs pass the same owner and
  authoring-tenure pre-check before capture as Studio's. Every failure or held outcome of an
  offered repair, Studio or Registry, at any stage (no local copy, a failed capture or rebuild, a
  rebuild gone stale, a held outcome, a failed commit), holds that repair rather than its target,
  so a replayed older repair cannot keep a target's legitimate replacement from being fetched. An
  offered repair that was applied and now needs the user (a recovery warning, a storage refusal)
  holds the document, since that hold is this device's own state. The same repair offered again
  while its job runs may complete that job's evidence; it is never trusted beyond what S3
  verifies.
  The router still classifies a pending replacement only from the exact-current verified source
  already retained by the bounded preparation boundary; a held owner decision is read from the
  bounded owner record. Missing, cold, stale, retargeted or unreadable preparation is unknown and
  defers the pass; it is never treated as proof that ordinary installation is safe. The owner's
  resume uses the same exact classification, falling back to the record's B3 flag only while the
  provider is unknown, so an install that landed just before a crash (its recycle lost) is
  resumed and recycled rather than waited on forever. Nothing on the repair path reconstructs
  the full Registry graph under custody to decide. Classification preparation is scheduled even
  when the local file is small. A custody-checked absent source is tracked separately and
  rechecked by path, so first installation can proceed without turning cold or stale state into
  a false absence claim. While a job owns a bucket, Registry maintenance, pointer refresh, page
  persistence and installs into it are skipped or deferred. A Studio-side pointer write is not
  claim-checked; like any other change, it makes the rebuild stale and the job writes nothing.
  The owner's automatic repair work is paced per target. A hold, a failure, a started
  repaired-seed fetch or a started resume job defers only that Studio target's or bucket's next
  owner-resume visit: by 60 s, doubling while it repeats, up to 15 min. Every persistent hold
  also backs off that target's automatic jobs. A terminal outcome, a new explicit decision, or the person
  acknowledging the document's recovery warning resets it. One held target therefore never
  delays another's resume, and the owner's resume work for a target whose seed no peer serves,
  or that needs the user, costs less and less. That pacing covers the owner's resume visits
  only. A repaired-seed fetch started any other way (from an answer that carries a repair, from
  the router, or after a job) is paced only by the single checkpoint slot and a hold, so a peer
  owing a seed nobody serves refetches it on every reporting discovery of that target, as it
  always has (an open follow-up). Nor is the pacing a global cap: K newly held targets can still
  start K seed fetches in their first minute, each holding the single checkpoint slot, and the
  deferral state lives in memory, so a restart starts every target afresh. A page fetched before
  an S3 that rewrote its source is dropped, never saved against the repaired source. After a
  crash between an owner's replacement install and its record's recycle, the router resumes the
  decision as soon as it finds the source owing nothing behind it. If no fetch can start (an
  owner alone has no peer at all), the owner resumes at once, and if a fetch comes to nothing,
  its next visit resumes, for a Studio source and a Registry bucket alike (including a bucket
  whose only watched document is stuck Prepared, whose provider is never prepared). So
  recovering an install already on the owner's disk never needs another device. The cost of that: an owner alone whose seed really is still owed, with its
  source evicted between visits, reruns a full detached rebuild once per deferral window (down to
  one per 15 min) although nothing has changed. It is bounded by the deferral and the shared pool.

- **Automatic recovery is conservative and author-local.** Studio replay checks the complete
  own envelope, every retained/staged historical selection and the fresh current projection.
  It never uses op-id sorting as causality or lets one older snapshot override a newer conflict.
  Replay shares bounded worker scheduling and ordinary Save authorization/publication. A cold
  large referenced object is held rather than reconstructed on the actor thread. Unsafe edits
  leave pending only after an actual full-envelope recovery record is re-flushed before the
  accounted ledger replacement; exact post-rename retries still sync. This user-approved manual
  disposition is not settlement and has the same two-snapshot eviction limits as explicit
  recovery. No-evidence intents remain pending. Existing accepted current-log operations cannot
  be manually retired by this path. Settlement events are invalidations, not authority or proof
  of receipt coverage; native session/incarnation guards also fence their delivery.

- **Local recovery is a new edit, not historical authority.** Explicit Studio Restore/Copy
  validates all sealed typed slots and current member/channel/epoch before using ordinary Save
  and its blob, intent, storage and one-shot publication checks. Original attribution remains
  separate from the restoring signer. Restore never overwrites; Copy requires an explicit choice,
  cannot replace immutable Index identity fields and cannot resurrect a retained deleted id.
  The complete current projection fences previews, and new historical tombstones are rechecked
  independently. Only an exact own envelope already in the current Open log bypasses re-preview;
  a pending intent, marker, known op id or matching visible value does not. Native JSON/base64
  conversion stays under final session/incarnation guards. Recovery backup export contains
  private historical content, not only visible pixels. Pointer restoration uses a separate
  explicit Registry write and actual saved source, never a renderer-supplied checkpoint. It can
  override valid historical deletion evidence after rotation, not a current tombstone. Partial
  recovery/pointer failure is reported, not silently promoted to complete or settled recovery.

- **Creative blob size bounds are per operation, not retention or automatic-load authority.**
  `publish_pix` validates the bounded PIX1 format and promotes verified bytes before returning a
  referenceable CID. `request_blob_bounded` bounds local/sealed reads, response body copies and
  storage by the caller's declaration (hard ceiling 9 MiB); the transport has already buffered its
  existing globally capped frame. Network responses retain current-member, request-signature and
  CID checks. Native calls share four inline slots with cancellation/transport keepalives and
  suppress stale results after unlock-generation or server-incarnation changes. Consumers must
  enforce exact record length, format and passive-fetch policy; no Studio auto-fetch is wired.
  These primitives neither pin blobs nor enforce a disk quota. An orphan can remain after a failed
  post-promotion flush; deleting a shared held CID to roll back would be unsafe. File flush plus
  Unix directory flush matches the vault seam, not a universal power-loss guarantee. P1 storage
  accounting and creative reference enumeration are not implied by this independent C0c seam.

- **Cryptographic core holds against modified clients.** End-to-end message confidentiality,
  membership authentication, forward/post-compromise secrecy, the owner anchor, and admin-grant
  authenticity are enforced by MLS + signatures, not by client behavior. A modified client
  cannot read messages it isn't entitled to, forge membership, forge an admin grant, or forge
  the owner identity.
- **A colluding malicious *majority* is out of scope** (as elsewhere in the design). We defend
  against a single modified client / member, not against the committer (owner) itself being
  malicious; the owner is the root of governance authority by construction.
- **Governance/policy actions vary.** Some are protocol-enforced; some are enforced only by the
  honest client today. The tables below are the source of truth.
- **Public reachability infrastructure is low-trust, not invisible.** A relay/rendezvous that
  serves AutoNAT v2 learns the requester's source address, candidate addresses, peer id and probe
  timing, and can withhold service or cause a false negative. It cannot forge a v2 positive merely
  by claiming success: the client accepts success only after the fresh callback returns its nonce.
  Ordinary members do not serve anonymous probes. The relay/rendezvous wrapper tags the upstream
  callback and a first-declared pre-socket guard accepts only one direct public TCP/QUIC literal
  whose IP exactly matches the request connection's source; it charges peer, source-prefix, node
  and concurrency limits before opening a socket. Exact-IP matching still permits bounded probes
  of other ports on the requester's shared NAT/CGNAT address, and the service retains metadata and
  egress cost, so it remains **experimental, disabled by default, and operator opt-in**.
  A successful callback proves only that the configured observer reached that exact candidate at
  that moment. Operator configuration also permits LAN/private infrastructure, so the product does
  not turn that result into the broader claim “reachable from the internet”.
- **Member switchboards are consented, bounded admission paths, not new authorities.** A standing
  host explicitly opts in per server; its complete two-minute self-signed offer is carried under
  the invite's named inviter endorsement, so the inviter cannot substitute routes or lengthen the
  helper's consent. A joiner explicitly consents before any helper address is dialled.
  The helper must be a current member with a live record-bound connection to that exact inviter.
  It forwards only bounded `JOIN`/`WELCOME` frames, applies the resulting Add before serving as the
  joiner's first sync path, and cannot forge the inviter-signed Welcome. It already has normal
  member plaintext access; helping grants no additional content authority. The host learns the
  joiner's IP/timing and spends bandwidth, while invite recipients learn the host's stable device
  and transport identities plus candidate addresses. Opt-out refuses new forwards immediately,
  but cached/already-copied signed offers remain dial-visible until their short expiry. A malicious
  member may sign an arbitrary *public* candidate: canonical terminal-peer binding plus endpoint,
  prefix, server and process caps bound the resulting scan surface, but the signature does not
  prove address ownership or live reachability.
- **Two-way replies authenticate possession of the bearer invite, not a person.** Candidates are
  public direct literals, capped at four and live for at most 60 seconds from receipt. Every callback
  contact proves the invite-derived reply channel before seeing the invite/KeyPackage; replacing a
  different joiner needs confirmation. Anyone who obtained the original invite can still form a
  valid reply or redeem it. Both apps must remain open during an overlapping window, and symmetric
  NAT/CGNAT can still make punching impossible. Each callback socket pass spends the shared
  endpoint budget; proof retries use a connected-only actor command, so a proof queued before the
  first connection completes can still use that live connection but cannot consult the ordinary
  recent-peer cache and silently redial after the scheduler denied a new socket attempt.
- **The local router is trusted only for a mapping candidate.** UPnP/PCP/NAT-PMP and PCPv6 firewall
  pinholes can expose this app's stable TCP and UDP/QUIC listeners and can return a wrong or stale
  public socket. PCPv6 accepts only a request-matched Global Unicast result from the exact scoped
  default router, but the pinhole remains open to arbitrary Internet sources because invite peers
  are not known in advance. Noise and connection limits still protect the listener; AutoNAT is the
  independent test before the UI calls the route verified. A malicious gateway can still cause
  denial of service or dead invites. The client requests five minutes but honors a larger
  router-assigned lifetime up to a 24-hour sanity cap; a crash can therefore leave the pinhole
  until that grant expires. Publishing a global IPv6 privacy address also makes that
  device/address visible to invite recipients, peers and the configured AutoNAT observer.
  Optional WebRTC UDP mappings additionally use native mount/call/grant custody rather than
  port-only release authority. Pending native work survives a lost IPC waiter through cleanup;
  lock, call end, remount and server-incarnation retirement revoke its installation authority.
  IGD cleanup retains the granting gateway, and post-Add validation failures retain custody.
  Uncertain router or legacy PCP/NAT-PMP worker results quarantine both port and capacity for
  the process lifetime (64 total slots); repeated End, lock or account change cannot refund
  them. This can exhaust the optional mapping aid while STUN/TURN continue. Client drop is not a
  joined shutdown or physical closure proof; router leases and pre-crash effects can survive
  process restart. The existing interface check does not prove kernel ownership of a named
  WebRTC socket. [The custody contract](communication-recovery/CALL-MAPPINGS.md) records these limits.
- **Discovery retries are availability aids, not proof of presence.** The current member roster and
  self-signature gate every cached record. Every route accepted from PEX/cache/rendezvous is parsed
  by one canonical grammar, must terminate in the exact signed/discovered transport peer, and is
  revalidated immediately before submission; the network actor refuses a peer-less dial instead
  of falling back to `Swarm::dial(address)`. A signature authenticates who chose a public socket,
  not ownership of that socket, so the local policy charges every address and one desktop-owned
  scheduler caps process, server, canonical Phase-0 peer, attempt, and IPv4 `/24`/IPv6 `/48`
  dial-command submissions.
  The parser embeds the principal into the opaque endpoint, preventing cache, rendezvous, and
  pre-join callers from selecting different byte representations for the same transport. Direct
  attempt/prefix/process keys do not include descriptor sequence or terminal PeerId, preventing
  those rotations from resetting the main scanner bounds. Relay attempts are keyed by relay and
  terminal target so unrelated circuits sharing a relay do not consume one two-attempt bucket; the
  relay host is instead bounded by prefix and process caps. Scheduler state is bounded and transient;
  restart resets it. It accounts submitted dial commands rather than actor-confirmed socket starts,
  and relay outer sockets have no separate exact-socket permit. Discovery endpoint permits are now
  actor-consumed, single-use, deadline-checked, and accounting-generation-bound. Reservation and
  commit share one injected monotonic clock; queued work cannot start after its window merely because
  no later reservation rolled the counters. Cancellation after command enqueue cannot refund queued
  work; actor-owned connected, member-duplicate, and pending-infrastructure checks refund before
  commit; and a late old-window drop cannot decrement current counters. A process-wide
  in-flight/concurrency lease remains future hardening.
  Direct routes carried by companion grants use the same canonical invite policy, must name one
  unambiguous contact, and spend this process budget across every grant in a bundle. Pre-join
  rendezvous uses at most two validated seeds so infrastructure routes cannot consume the entire
  per-server window before the actual inviter route.
  The sync classifier refuses DNS and dangerous local/private/transitional ranges but deliberately
  retains non-routed documentation/benchmark literals for deterministic tests; such a record can
  waste only bounded retry tokens and is not proof of a live public route.
  Failed current epochs retry with monotonic exponential backoff and jitter; a newly signed epoch
  is tried immediately. Each discovery pass also asks the local kernel which IPv4/IPv6 source it
  would route toward documentation-only destinations (UDP `connect` sends no packet), then
  republishes a changed raw route set. A process-wide native route/interface monitor normally
  triggers that operation after a short debounce; the discovery-period poll repairs a missed event
  or unavailable monitor. Exact ownership preserves an identical manual/mapping/relay route.
  The cache intentionally replaces rather than permanently unions withdrawn
  public IPs: an ISP may reassign an old address, and Noise authentication prevents impersonation
  but not the metadata/connection cost of probing its new holder. PEX success proves one live
  member connection at that moment; failure means only “not reachable from this device by this
  path,” never offline, removed, or malicious.
- **Typed member-route health describes a self-asserted claimed peer.** A member's device signature
  authenticates the `PeerDescriptor` fields it chose; it does not prove control of the transport
  key named in `peer_id`. A malicious member can claim an unrelated already-connected peer and
  make its own row inherit that peer's current coarse path evidence. The backend therefore uses
  `ClaimedPeer*` health variants, exports `binding=self_asserted`, attaches observations only to
  the signer/member's own row, clears history when the claimed transport identity is replaced or
  removed, and the UI preserves the caveat. A fresher address epoch for the same stable transport
  keeps that peer-scoped history; it is still labelled historical and carries no address. This is
  diagnostic evidence, never authorization, membership proof, “online,” or
  public reachability. The implemented reciprocal-dial repair is not a device↔transport dual-key
  challenge and does not strengthen it; a future explicit proof is required. Raw active snapshots and retained history are capped, deduplicated, session-only,
  monotonic-aged, and hidden after 24 hours. Transport churn can still backpressure the actor's
  bounded event queue; connection limits and duplicate-close suppression bound amplification but
  do not make it free. Unclaimed transport peers do not invalidate the member-only UI projection.
  A dial-scheduler row proves only that a policy-approved batch was submitted, not that every
  candidate reached the transport or failed, and the UI does not infer outbound IPv6 capability
  from inbound/public candidate observations. Current switchboards forward admission only and are
  not presented as a post-join repair path. Operational availability instead uses request-bound
  signed catch-up proof. That transient cache retains the current roster `DeviceId` that answered
  on each peer connection, so a removal prunes the departed signer without invalidating unaffected
  live member paths; every later request still authenticates independently.
- **Member-assisted repair is bounded signalling, not delegation of authority or traffic.** A
  helper answers SWIM-style probes only from its current session-proven connection table and never
  dials the target for a probe. Forwarding requires a current authenticated requester, exact
  current requester/target descriptor references, live proven paths to both, a short expiry and a
  per-device budget. The helper signs the original frame and queues connected-only delivery so an
  A→C→B cycle cannot nest actor waits. The target rechecks roster, descriptors, helper proof,
  signature, replay and rate limits, then submits at most two direct routes through the ordinary
  discovery and process scheduler. A descriptor replacement/removal cancels pending work; already
  submitted transport dials cannot be recalled. Signed negatives create only time-bounded
  suspicion and never change presence or MLS membership. The control plane reveals attempted peer
  pairs/timing to the selected helper, but no plaintext; ordinary participation does not make that
  helper a public listener or general relay.
  A stable highest-priority offline target must not monopolize probe selection after every
  authenticated negative. After initial priority, target order rotates over the full bounded
  descriptor set, retaining its cursor through refresh and local Busy. Each confirmed admission
  commits that cursor before another helper await can be cancelled. The cursor grants no
  membership/path authority, does not refund ambiguous handoffs and does not increase per-pass
  fan-out or shared helper budgets. Fair selection assumes a stable eligible set and local
  admission opportunities; continuous membership/route churn or capacity denial has no fixed
  recovery-time guarantee.
- **The desktop webview is trusted only while the UI session is unlocked.** An explicit lock keeps
  native actors online for background sync but closes all non-bootstrap Tauri commands. CSP and
  the main-window capability reduce injection reach; the native command gate is the enforcement
  layer, not the fact that Svelte hid or cleared a control.
- **Diagnostics have separate local-capture and public-disclosure boundaries.** Safe capture
  destructively replaces literal addresses, arbitrary runtime prose, runtime field names, and
  non-allowlisted targets before bounded ring storage; Enhanced and Full intentionally retain more
  local detail and still require review before sharing. Webview-origin trace ids are normalized
  under the process-local diagnostic salt; a native trace crossing back through the renderer is
  accepted unchanged only with its session/trace-bound proof, which is never stored or rendered.
  The optional raw debug file is a different sink and does not pass through Safe admission. Any
  native tracing or frontend console/error value that passes its target/level filter can reach disk,
  including names, message fragments, paths, URLs, tokens, serialized objects, and stack traces.
  Its size/rate bounds are availability and retention controls, not a privacy filter; the user must
  inspect the actual file before sharing it.
  After Tauri decodes an invoke body, structured UI commands retain at most 256 events and 32 fields
  per event. This does not impose a byte cap on Tauri's already-materialized JSON request: a compromised unlocked
  webview can still spend native parsing time/memory with an oversized request body, so a transport-
  level command payload cap remains an availability hardening item. An unlocked or compromised webview may ask
  native code to open the fixed public-diagnostics issue, but supplies no report, title, or URL:
  native code renders its closed allowlist, validates it, constructs the bounded tracker URL, and
  launches that exact value. A truncated URL does not claim the later clipboard attempt succeeded;
  the UI reports copy success only when the clipboard promise resolves. Ordinary user-authored
  feedback remains a separate exact-tracker launcher path and is not presented as automatic safe
  diagnostics.
- **Delivery receipts prove receipt by a member device, not attention by a person.** Only a newly
  applied, signature-verified op can queue a receipt, and it is sent as a connected-only
  member-authenticated request. The author records it only when its exact document/change hash is
  already in the bounded recent-target set; arbitrary hashes, duplicates and unauthenticated
  frames cannot allocate confirmation state. A recipient can withhold receipts or a modified
  current member can claim receipt without displaying content, so the UI says delivered/held and
  never read. Receipt traffic adds message-timing metadata to peers already participating in the
  encrypted group; it is not broadcast outside the group.
- **Recovery persistence is scoped and crash-retryable, not settlement authority.** P1 recovery
  records are vault-sealed and bind the local server id plus full group/type/logical-key scope;
  moving valid ciphertext between record paths is rejected. Reads are capped on the opened file,
  and a corrupt/oversized record is never replaced as if missing. Each exclusive store update
  saves the complete slot state before returning; a post-rename directory-sync failure reports
  committed-but-not-durable and exact completed-eviction ids permit a safe retry. Generic debug
  formatting excludes recovery content. The seven-day warning uses persisted receiver wall time,
  so local clock changes can affect its duration. There are three logical slots, not three physical
  copies during replacement: temporary ciphertext and crash-orphan siblings still require shared
  accounting/cleanup before network or settlement integration. Records currently remain after
  server removal, like held blobs; their retention lifecycle is not yet wired. No success from this
  standalone API authorizes pruning an epoch or claims a total Studio storage bound.
- **Storage admission relies on complete local inventory, not peer-supplied sizes.** The P1
  accounting boundary caps permanent and peak old/new/scratch occupancy with both reserves inside
  2 GiB. Staged bytes pin the reserve to their logical document. A reservation marks its budget
  unready before exposing a guard: dropping or forgetting that guard cannot restore unproven free
  space. Failed/uncertain I/O requires reconciliation, including all orphan temporary files.
  Accounted recovery saves compare the authenticated old record's full pool split and owner, not
  just length. The coordinator must own one budget per server and exclude unaccounted writers;
  fabricated/partial inventories or independent duplicate budgets are not an enforcement path.
  Complete managed-type inventory, cleanup and multi-record settlement remain unwired, so this is a tested
  admission primitive/adapter, not a claim that all current vault writes obey the Studio cap.
- **Recovery inventory does not turn leftover files into authority.** An exclusive borrowing scan
  authenticates canonical final records one at a time, bounds all directory traversal and retained
  metadata, and never returns a completed result after an error or caught parser panic. Recovery
  filename aliases are recognized case-insensitively and then rejected; symlinks/reparse points
  and redirected scan parents are refused. Orphan temporaries are never parsed, promoted or
  removed. Only an authenticated destination supplies ownership, and unresolved ownership blocks
  per-server inventory composition rather than inventing attribution. Debug output omits scopes,
  paths and content. The recovery-only API covers that namespace only; the explicit combined API
  also covers owner journals. Each result reports its fixed coverage, and its metadata becomes stale
  if the future coordinator permits writes after the scan. A malicious local process concurrently
  replacing filesystem paths is outside the mounted-store exclusion guarantee.
- **An inventory issued from a scan is checked against the vault as it is now, not only as the
  scan saw it.** Every five-family write rotates `inventory_generation` before its first I/O
  (I-4), and an owned scan cursor rechecks that token when it resumes, installs a detached
  validation and finishes, and the Studio budget mint checks it again. The type-level write guard
  does not cover a raw `std::fs` call, so `scripts/check-store-raw-fs.sh` in CI refuses raw
  filesystem mutation in non-test store code. A cursor held across custody visits also holds its
  directory stream while unrelated files in the same directory change, and some filesystems a
  vault can sit on (certain SMB and FUSE backends) do not keep a stream stable under that churn.
  Every finished inventory, budget or reference, is therefore confirmed against one fresh
  names-only listing and refused on any mismatch: an inventory job restarts, and a synchronous
  scan's caller gets an error, which pauses background receive. On such a filesystem Studio
  storage work can be refused, never budgeted from an undercount.
- **Replay's manual move holds a cursor across visits** (C-3 step 2, the first production path to).
  Its shared, turn-based inventory job keeps an owned cursor and its directory stream between the
  receiver's custody visits.
  - **How long:** until the first visit 30 s or more after its owner last asked for a budget,
    when the idle drop runs. That is about 35 s with the native 5 s idle wake.
  - **What is bounded:** a parked record's authenticated plaintext and its preparation permit are
    released at pause, at the lock reset and on context change. The job takes permits from the
    same four-slot process pool as catch-up preparation.
  - **Under writes:** a write by another actor charges the job's restart budget, and an exhausted
    budget backs off; this actor's own writes refresh it uncharged, within a cap. **Except a
    restart whose attempt warmed a record it found cold** (C-3 runtime 16.3): that is progress,
    since the next attempt finds the record warm, so it is not charged. Each record excuses at most
    one restart in a job's life, and a job has at most 64 restarts excused, so a peer writing
    between every turn still drives the job to back off once it has nothing cold left to warm.
    Each excused restart did at most one visit's bounded validation work, which the memo keeps.
  - **Errors:** a detached validation's error pauses background receive only if it is about bytes
    still current. An error about bytes a later write replaced restarts the job.
  - **Fallback:** a move that the job cannot serve within 60 s falls back to the synchronous scan
    it used before step 2. So the move, and its recovery notice, can be delayed by up to that
    long.
- **The other automatic Studio inventory owners remain synchronous, as measured debt** (C-3
  runtime 16.5). After production writers warm every record family, the sampled warm Registry and
  Studio accounting traversals spent about 0-8 ms in release profiling, and a real foreign writer
  overtaking a cold shared job leaves enough memo progress for the restarted job to mint on its
  next turn. These are host measurements, not an `INVENTORY_SLICE_MS` guarantee. A remount makes
  the memo cold: receive/catch-up retain their 1024-entry, 1024-record, 8 MiB and 256 KiB-cold
  fail-closed rails, while full-profile H1 and repair commits can still hold one custody visit for
  cold validation. Replay is the only production owner that keeps an inventory cursor across
  visits; a future latency gate must give H1/repair embedded ownership-aware jobs rather than
  borrowing a second pool permit.
- **A Studio handoff's final commit (H5) runs in one custody visit, accepted as technical debt**
  (decision 2026-10-09, `GATE4-AGENT-1-C3-RUNTIME.md` 15.14). It grows with the document and the
  local draft, both capped (999 frames, 256 draft operations). At both caps it measured about
  0.15 s, and once 0.25 s on a busy host, against a 0.125 s visit target. While it runs, the
  server's background work and Studio requests wait.
  - **What a member can do:** fill a shared Flipnote to the frame cap. The draft is the
    handing-off device's own, and a device hands off only documents it drafted on, so this buys
    at most one such pause per draft, with no amplification.
- **An interrupted handoff is resolved off custody (Flow R, design 6.4.2).** The source restore
  that used to run under custody, twice when the source was cold, now runs on a worker, and the
  commit performs exactly the synchronous resolver's writes behind a stamp.
  - **What a member can do:** send operations to the document while it is Prepared. That changes
    its source under the stamp, and the commit then falls back to the synchronous resolver in the
    same visit. On the actor that costs at most what resolution cost before Flow R, plus a
    discarded detached stage on a worker, once. The resolution completes there, so it cannot be
    repeated to hold the record Prepared.
  - **While the record is Prepared,** catch-up's rotation, client pass, Registry maintenance and
    replay skip that document rather than pausing receive. Replay also drops a pass it had
    already begun on the document, so the others keep replaying. No path resolves a record stuck
    on Hold, so these rails stay suspended for that document indefinitely; the eligibility view
    reports it as `PreparedStuck`.
  - **The suspension stays with the document.** Its pointer bucket is shared with other
    documents, and maintenance's turn is a held bucket decision's only ordinary resume route. So
    before skipping, Registry maintenance resumes that bucket's held owner decision, unless a
    repair job claims the bucket (Agent 3, PR #37). That resume reads only the bucket and its
    owner record, never the Prepared document's source, and an error there is recorded, never
    returned, so the skip still cannot pause receive. Until PR #37, a stuck document that was the
    only watched one in its bucket stranded the bucket's decision.
- **A budgeted scan holds custody for a bounded validation only, and a memoized validation is
  reused only for identical bytes.** Under a deadline, `validation_fits` admits a fresh validation
  inline only in accounting mode, and only for four families:
  - Recovery, to 64 KiB;
  - Intents, to 384 KiB, where the 25 ms cap binds;
  - OwnerReceipts, to 747 bytes;
  - DraftArchive, whose accounting does no size-dependent work.

  It also requires a measured worst rate, times four, to fit in what remains of the slice and in
  25 ms. Every other fresh validation detaches, including every fresh Registry and Studio
  validation; a memo hit is reused instead. The rates are from one host, in a release build, on
  the shapes measured. A peer able to make a much denser record than those shapes, or a much
  slower machine, lengthens an inline hold in proportion. For Recovery and OwnerReceipts the size
  envelopes keep it small; for Intents, the cap does. The validation memo holds every family's
  accounting record (C-3 runtime 15.2, M1), and for an Intents record its inventory facts too,
  keyed by filename hash, physical size and the blake3 digest of the authenticated plaintext, and
  a hit needs all three to match bytes a scan has just read. Validation is a pure function of
  those bytes for every family (OwnerReceipts verifies signatures over keys its own bytes carry),
  so a hit is what a fresh validation of the same bytes returns. **One deliberate
  relaxation:** a detached result refused because a write overtook it is still memoized, unless
  the memo already holds that record, so the restarted scan need not validate it again. The result
  never reaches an inventory, and validation is a pure function of those bytes, so a memo entry
  for bytes since rewritten is only a miss. A read evicts any entry its bytes contradict.
- **Recovery staging cleanup deletes unpublished attempts, never saved recovery versions.** Only
  strict canonical temporary sibling names under the mounted store's fixed parent are eligible,
  with regular/non-reparse checks and exclusive access for the whole bounded pass. No caller can
  supply an arbitrary target path. A first-write orphan without a destination is also unpublished:
  the writer consumes its staging name by rename before reporting success, and callers must keep
  durable source history/intents until that success. Cleanup does not parse/promote those bytes,
  delete logical staged snapshots, or authorize pruning. Errors and caught panics can leave
  partial removals, but never a completed pass or accounting credit. Retry runs the directory
  flush even when no siblings remain; this retains the existing Unix/Windows directory-durability
  guarantee, not a universal filesystem or hardware guarantee. A new inventory is mandatory after
  traversal because
  deletion can affect directory iteration. Observed deleted lengths are not promised reclaimed
  disk space. This API remains unwired to startup and network input.
- **Owner receipt persistence is a publication prerequisite, not proof of current authority.**
  A bounded vault-sealed journal binds the local server id and full group/type/key, retaining one
  high-water receipt and one pending decision. Preparation checks the canonical receipt, current
  owner signature and externally supplied tenure evidence, then charges peak replacement space
  and saves before returning. Exact publication-completion retries preserve a newer pending choice;
  a strictly later verified tenure may replace an unfinished old-tenure decision, never a conflicting
  choice within the same tenure. Corruption does not reset the journal. Failed/uncertain writes and
  caught writer panics block accounting until full reconciliation. Loading visible bytes alone
  grants no publication authority: re-save and recheck current owner/tenure/session at send. The
  registry rotation adapter now validates closure/seed before signing; the coordinator must discover/clean this separate
  namespace before constructing production budgets. The combined storage inventory/cleanup now
  covers that namespace alongside recovery. A temporary's ownership requires a verified destination
  with both the same namespace and digest, and owner bodies keep their own small pre-read cap.
  Cleanup can remove unpublished attempts only; saved pending/high-water decisions are never targets.
  The explicit registry owner driver and checked head-response preparation invoke it, but no
  automatic actor/startup scheduler owns those paths yet. Durability remains file-sync/atomic
  replacement plus Unix/Windows parent sync, not protection from device failure or restoration of
  an older vault backup.
- **Owner issuance resumes exact heads, not a new decision from changed content.** The explicit
  registry driver requires the current physical mount/server and a durable runtime/MLS/full-owner/
  tenure snapshot permit, not merely an observed tenure number. It validates and flushes the source
  plus owner inventory before deriving a close. All current heads participate (over 64 refuses),
  closure eligibility counts real signed user content, and the canonical typed seed derives from
  those heads before signing. Inheritance comes from the installed opening, not caller/UI fields.
  A bounded v2 extension in the owner vault record atomically binds one close to the selected receipt;
  old no-extension records are unchanged and old readers reject new extensions. The physical record
  cap grows to 8,488 bytes, still charged as protocol/reserve space. Decode proves structure/binding,
  not current authority; resume rechecks signature and exact receipted seed against the retained source.
  A crash after journaling but before sealing may leave later Open edits: the saved close is resumed
  byte-for-byte and excluded edits enter recovery. A legacy pending decision without that close holds,
  never regenerates. Exact installed retries preserve newer content. Installation labels publication
  pending rather than claiming a handoff or delivery. Kind-21 serving now records exact completion
  after the local reply channel accepts a freshly authorized owner proof. The private non-Clone
  token binds receipt, scope, runtime, MLS/full owner/tenure, watch and expiry; the Server consumes
  it synchronously with its original mount/server/store/budget. Dropped receivers and hints do not
  count. Channel acceptance proves neither transport-driver admission nor peer delivery; a later
  completion failure may follow an already visible reply. Uncertain writes block accounting and
  exact republication repairs restart. Gate 4 also completes an exact installed Open head locally,
  but only after current durable-owner authorization, matching journal/seed and actual source and
  journal flushes. This is discoverable local availability, not handoff or delivery; solo servers
  can therefore make the next decision without inventing an acknowledgement. The existing idle
  worker now drives watched Studio owner rotations and Registry maintenance. Universal full-quota
  progress remains unfinished; completion never bypasses recovery-first retirement. Valid Registry
  Fault state is accounted independently of its refusal to serve a head or receive a tail, so one
  faulted bucket cannot masquerade as corrupt inventory and pause unrelated Studio documents.
- **Saved local intents are replay data, not completion or impersonation authority.** The
  prepare-only vault adapter binds the local mount id and full group/type/key, checks the actual
  local device's current roster signing key, and saves before reporting success. It bounds public
  envelope fields before encoding and verifies the embedded ledger scope even when empty.
  Type-specific semantic validation remains the coordinator's job. There is no arbitrary-id
  public retirement API:
  neither a received operation nor its marker can delete a pending intent. New writes charge
  ordinary content and full replacement peak. A separate 64 MiB vault cap counts physical final
  intent bytes and all temporaries, including unknown ownership; a scan covering all three families
  is required. Mount/generation provenance rejects cross-vault, stale and duplicated budget use
  after intent write/sync/cleanup attempts. Both budgets fail closed after uncertain I/O.
  Exact retries sync an authenticated unchanged final and parent without a replacement copy,
  so a committed write at the cap can still pass its durability barrier. Cleanup removes only
  unpublished siblings, not saved intents, and still needs a fresh scan before credit is released.
  Parent sync is implemented on Unix and Windows; hostile local path replacement, device failure
  and backup rollback remain outside these guarantees. Storage admission across other record types,
  live settlement scheduling, replay and network/editor integration are still unwired. The checked
  registry store transaction below now performs receipt-covered retirement before source replacement.
- **Registry restart consistency is not settlement durability.** The `RegistryEpoch` coordinator
  rebuilds only a receipt-bound raw seed plus bounded, individually signed and schema-checked
  changes. It accepts no separately saved Automerge state that could introduce unsigned roots.
  Restored gate metadata must match every log operation's hash, id, full author and encoded size;
  lifecycle phase must agree with retained receipts. Local vault restore preserves historical
  authority after succession, never authorizes a newly received old-owner receipt, and refreshes
  the current quota-exempt owner before admission. Repeated late hashes cannot consume extra
  quarantine slots or make a snapshot undecodable. The restart codec assumes authenticated local
  vault bytes; it is not a wire authorization path. The store now attaches scope-bound encrypted
  registry records: inbound admission and receipt sealing reload/validate, reserve physical
  replacement space and save before returning. Exact retries sync unchanged authenticated bytes;
  failed writes/flushes require reconciliation and grant no acknowledgement. Receipt authority is
  checked before disk access; expected missing history is a fetch prerequisite, not corruption.
  Public ciphertext/receipt fields and physical records are capped before expensive processing.
  Peer-writable registry history charges content, not reserved receipt space; only exact receipt
  growth charges protocol, allowing a seal at the content cap while protocol/reserve space remains.
  Four-family inventory validates historical raw snapshots without granting mutable authority;
  cleanup removes only unpublished attempts, conservatively content-charged. This is not yet a
  production all-family budget coordinator, and Registry tail/pointer publication plus automatic
  owner rotation, succession and recovery control remain Gate 4 work. Per-mutation graph
  reconstruction is bounded, and the transport is now integrated behind the bounded, opt-in
  registry gossip inbox and the authenticated page/head/seed exchanges, which impose the queue,
  rate-row and packet caps (`catcoms-sync::registry_ingress`, `registry_catchup`,
  `checkpoint_exchange`); Studio's kinds 23/24/25 share those same queues, rates and capacity
  rather than adding their own. Enqueue is authentication, not document admission: the app still
  drains one packet at a time through its durable typed store.
  Local registry editing now joins the intent and epoch adapters under the exclusive store borrow:
  typed/current-author validation and full retained-id comparison precede intent persistence;
  ciphertext returns only after both intent and epoch saves/flushes. An interrupted publication
  retry reseals the exact signed change, not a new change against newer heads. Ids omit bodies,
  so equality requires the entire canonical envelope even when no local ledger exists yet.
  Post-restore/pre-mutation comparison ignores only normalization such as the current quota owner;
  a retry at the content cap still flushes the authenticated original file rather than copying it.
  A saved intent survives a later edit/save failure and is never retired by a marker. Closing/Fault
  refuses local publication preparation, and an author removed from the roster cannot retry.
  Returned ciphertext is not a permanent send permit: live transport must still recheck session,
  server incarnation, membership, MLS epoch and Open at send. Automatic replay must use the intent's
  original author. Callers must still persist excluded recovery before source retirement.
  Registry settlement preparation now checks the exact held current-owner receipt and named
  dependency closure, rebuilds its canonical seed and verifies the expected hash. Its immutable
  recovery inputs include excluded accepted operations with their verified authors, not replay
  permission or late quarantined content. A whole-source fingerprint distinguishes peers with
  identical receipts but different excluded work. Planning changes no durable state or quota-owner
  metadata and grants no installation/prune authority: the installation transaction must recheck source
  and authority under the gate and persist bounded typed recovery first. Recovery capacity is
  not implied by a successful plan, and stale-owner receipts fail preparation after succession.
  The typed registry staging adapter now recomputes that plan under exclusive store access,
  verifies the actual source footprint and (before a nonempty save) every existing typed recovery
  slot, and uses accounted
  durable recovery persistence. It never accepts a caller's stale plan or replaces the source.
  Recovery contains full pointer keys/tombstones/overflow and excluded author-attributed operations;
  decoding checks the generic wrapper too, but trusts authenticated local provenance rather than
  re-verifying discarded signatures. Snapshot ids exclude quarantine/owner-normalization metadata,
  so late traffic cannot churn slots or reset warning deadlines. Snapshot Debug is content-redacted.
  A successful stage still grants no installation/prune/replay permission; storage refusal or a
  pending third-slot warning leaves Closing history intact. This does not implement Restore,
  repair/rewind-specific typed recovery, automatic intent replay or settlement-wide reservation.
  The registry installation adapter now flushes the full checked Closing source, validates old
  recovery even for empty plans, saves needed recovery and holds any eviction warning before
  retiring receipt-covered intents and atomically selecting the successor. Retirement compares
  full author/domain envelopes, not just nonce-derived ids. Until replacement the durable source
  is the finality proof; excluded/unaccepted intents stay pending. Exact installed retries flush
  current successor bytes and never rerun old retirement or overwrite newer edits/seals. Local
  edit requests carry the captured concrete id, checked before journaling and before editing,
  preventing old Save retries from becoming new operations after source/marker retirement.
  Delayed same-tenure opening-receipt conflicts fault both Open and Closing successors while
  retaining high-water and accepted work. Restart binds historical fault evidence to the exact
  seed-opening receipt. ReceiptBook's bounded local v2 form preserves a newer high-water above
  that fault pair; old readers explicitly reject it, and ordinary v1 records stay compatible.
  Conservative per-record content reserves and physical intent replacement headroom can still
  refuse at full quota. No live actor/discovery, repair or automatic replay is wired. The existing
  file-sync plus Unix/Windows-parent-sync durability and local-path threat boundary apply.
- **Newcomer adoption preserves the whole source before replacement.** The registry core can seal a
  whole source against a distant or new-tenure checkpoint, retaining every signed operation.
  Ordinary restart remains v1; explicit adoption v2 contains an adoption-only bounded receipt
  book v3. Its decoder is not accepted by the ordinary book entry point. Restart still checks
  the actual original seed, signed log and gate metadata. Historical fault pairs must bind to
  the retained opening or prior target, not unrelated spliced receipts. Both anchors are screened
  before stale filtering; Fault preserves high-water evidence and blocks successor construction.
  Ordinary settlement rejects adoption mode even at an equal epoch. A separately constructed
  successor has one verified seed and carries repair anti-replay bookkeeping.

  Whole-source Rewound recovery includes seed-only pointers and terminal epoch 4096; exact outer
  caps still apply. Its identity excludes the destination receipt and quarantine, so retargets
  cannot reset eviction deadlines. The core alone never persists recovery, retires intents or
  replaces the source. The explicit Server/store installer now saves the source and successful
  typed Fault outcome independently of optional seed/recovery work, then saves typed recovery
  before atomic replacement. The physical mount/server, current full local member, MLS/owner/tenure,
  selection supersession and receiver-clock expiry are rechecked at admission under exclusive
  sync/store borrows; this is not a lease. Invalid/missing indexed records or uncertain I/O fail
  closed, never bypassing accounting even to save Fault. Recovery must be typed and inventoried;
  exact installed retries flush the actual successor without replacing newer edits. No intent is
  retired based on seed values. Old concrete watches and queued pages cannot save into the new
  epoch; the caller explicitly watches it and begins a fresh catch-up pass. A seven-
  day warning can outlive the 60-second fetch pass: resumption needs freshly scoped discovery,
  never a reconstructed permit from the saved receipt. Bounded historical evidence is not a
  complete owner audit chain; fault repair, automatic scheduling and universal full-quota
  settlement progress remain unfinished.
- **Registry replay preserves original authorship and exact retry identity.** The store's single-step
  replay accepts only a saved intent id plus a captured concrete epoch, never a supplied body or
  author. It verifies the actual current member is the saved author, the existing epoch is Open,
  both inventories match (including intent freshness), and every retained/staged recovery slot
  is typed and accounted. New authoring is held on current/recovered tombstones or a higher
  admitted/overflow pointer hint; holds retain the intent and are not delivery/finality acks.
  A full-envelope authenticated CURRENT-log match can reseal unchanged despite newer evidence,
  preserving post-rename Tombstone retries without reapplying effects. Markers and same-id/different
  bodies cannot invoke that exception. Both vault barriers still precede returned ciphertext.
  Stable keys and missing intent-origin epochs make holds deliberately conservative, including
  some legitimate later re-puts. Once two-slot recovery eviction forgets a deletion, its absence
  is not proof the key was never deleted; this protection is explicitly best-effort. The seam
  sends nothing, retires nothing, and does not implement a worker, live actor/transport checks or
  Restore. Its result Debug omits ciphertext/content. Per-call ledger/log/recovery work is bounded
  but repeated use still needs live scheduling/rate limits.
- **Replay traversal is not a delivery or lifecycle permit.** The cooperative registry pass keeps
  only a fixed, original-author id snapshot (at most 10,000 / 320,000 id payload bytes), and each
  actual attempt runs the existing checked store replay. Begin validates the ledger, not that the
  captured epoch is current/Open. A per-pass monotonic 100-ms deadline is charged before work;
  checked overflow refuses, errors/unwinds pause, and neither retries nor wall-clock corrections
  reset pacing. Prepared results wait for an opaque exact-attempt ticket; duplicate, stale or
  cross-pass acknowledgements never skip an id. A missing ticket has no timeout-success path:
  dropping/restarting preserves durable intents and exact-log retries. Traversal/submission never
  retires an intent or proves delivery/finality. Later additions require another pass, and missing
  selected ids pause even when normally retired by a receipt. Holds count as visited only.
  A private stable physical-mount token rejects use after reopen but is NOT a native UI-lock or
  server-incarnation permit (native lock may keep the store mounted). No ciphertext queue is held;
  Debug omits scope, ids and content. This is trusted local orchestration, not a peer-facing API.
  Gate 4's live actor integration adds aggregate pass/work limits, lifecycle cancellation and
  bounded wakeups outside this cursor; the cooperative sender adds the checks below.
- **The cooperative registry sender binds known local state, not remote currency.** Its cursor
  captures the exact sync-instance allocation at begin; a same-device/group restore refuses the
  old cursor. Exclusive sync/store borrows cover checked saved-intent preparation and awaited
  one-shot dispatch. The store verifies Open, full current author and both durability barriers;
  sync independently checks current full local identity, signature, MLS scope, canonical registry
  envelope and current blinded routing. No generic document is opened or retry outbox populated.
  The trusted local context/publication APIs do not independently grant Open or durable authority.
  A private exact-ticket guard advances only on Submitted. Duplicate, errors, cancellation and
  unwind preserve the saved id and charged deadline for fresh resealing; every outcome retains
  the durable intent. Cancellation after driver admission cannot retract gossip/cache effects.
  This cooperative backend call is not by itself live actor orchestration, a native lock lease, a
  bound on aggregate passes, a driver deadline or evidence that a remote receipt has already
  arrived. Gate 4 wires managed catch-up and bounded wakeups through separate actor custody and
  scheduling. Cursor/result Debug redact scope and content; the sync token is process-local and
  never persisted or transmitted.
- **Registry gossip authentication is not saved admission.** Explicit concrete-epoch watches use
  fresh sync/watch generations; their app handles also bind physical mount and captured local
  server/bucket. Rewatch/unwatch discards old queued packets without refunding rate debt. Desired
  watch installation is synchronous, and cancellation during subscription reconciliation retains
  one uncertain topic for cleanup/retry rather than losing ownership. Both subscribe and unsubscribe
  track their uncertain side effects before awaiting; same-topic rewatch cannot assume a cancelled
  unsubscribe left the topic subscribed. Revocation requires only the exact sync/watch generation,
  so a reopened mount cannot strand old watches; receive still requires its exact physical mount.
  Traffic must match a watched blinded topic, current MLS epoch, canonical registry domain/bucket,
  local current membership and a verified full current author. Registry-tagged bytes never fall
  into generic ingest/catch-up.
  Oversized frames reject before decode. A global pre-crypto 50/s, burst-200 rail bounds work across
  identities; per verified author/document is 10/s, burst 50, with 4096 debt rows reclaimed only
  when fully refilled. Monotonic arithmetic grants no wall-clock or watch-replacement credit.
  This shared global rail permits denial of receive capacity by one sender; it is not fairness.
  The queue owns at most 16 compact ciphertext vectors, each at most 256 KiB + 20 bytes, plus fixed
  metadata. It keeps no transport backing allocation or decrypted body. The explicit one-packet
  drain rechecks current watch/mount/instance/membership/MLS, then runs the existing typed gate and
  durable store barrier under exclusive access. Only that result reports Accepted/Duplicate/
  Quarantined/RejectedQuarantineFull; no legacy success statistic, network ack or receiver-authored
  intent is created. Errors or unwind consume volatile input only, not saved history. Drops due to
  quota, stale MLS, storage refusal or unknown watches require author retry or future catch-up.
  Past/future MLS reception and actor/native lifecycle ownership remain unwired. Current-MLS
  paged catch-up and registry discovery now have cooperative adapters, not automatic scheduling.
  Watches and rate debt are process-local, not restart-stable security policy or native lock leases.
- **Current owner identity alone is not current tenure.** Sync now independently tracks the
  owner transition it actually observed, saving it with the exact MLS group in one authenticated
  snapshot. Founding is known zero; old snapshots and Welcome joins remain Unknown, including
  a joiner that becomes owner through a recycled lowest leaf. All actual merge paths observe
  the resulting group before propagating even a post-merge helper error. Failed pre-merge work
  and same-owner commits cannot fabricate a new start. A missed hook or panic leaves a position
  mismatch: the getter reports Unknown and snapshot refuses instead of saving stale authority.
  A strict 57-byte versioned tail rejects wrong owner/epoch, partial fields or extra bytes;
  absence alone is legacy Unknown. Unknown may persist indefinitely and prevents fresh owner
  proof/rotation authority until independent evidence or a witnessed transition exists. It is
  never repaired by trusting the candidate receipt's tenure or substituting the current epoch.
  This is local evidence, not a durability grant. Runtime integration must flush the matching
  MLS snapshot/owner decision and recheck current membership, fault and tenure before signing
  a head proof. The cooperative kind-21 adapter implements this boundary; actor scheduling remains
  unwired. Cooperative registry expected-seed fetching is implemented below; explicit installation
  rechecks its private selection and follows the recovery-first transaction above.
- **Keyed head hints are not checkpoint installation or editing leases.** Kind 21 authenticates
  full current requester/provider identities and binds its logical key and internally minted
  request nonce before any source work. Fresh proof requires the current owner's endpoint and
  any independently known tenure; an old proof wrapped in a new member response cannot upgrade
  a hint. Explicit LOCAL whole-server snapshot preparation creates a lifecycle-bound permit;
  remote requests never trigger unbounded legacy snapshot serialization. Serving checks both
  inventories, rejects saved faults/corruption/lost indexed sources, and requires pending-preferred
  journal/source-head equality before bounded flush/re-save and signing. Missing/stale permits
  or disagreement yield hints only. One-shot proof establishes owner selection, not seed availability
  or validity. Source and intent state are unchanged; checked local reply-channel handoff can
  complete the exact owner publication journal, without proving driver admission or peer delivery.
  Eight fixed-lifetime queued requests,
  full-identity rates, four driver-owned outbound slots and source service rails bound resources;
  rate debt is process-local and Sybils still reach aggregate caps. Authenticated current-tenure
  repair bytes may be carried only after the exact replacement source is synchronized and the
  contextual owner record is re-saved durably. For Studio sources and Registry buckets alike,
  receiving-side repair application, owner continuation and repaired-seed installation run only
  as the detached job described under the Studio fault repair entry above, which holds shared
  preparation admission from capture through commit. Registry seed fetching
  and explicit recovery-first installation are implemented; head hints alone still authorize
  neither replacement nor an editing lease.
- **Registry page cursors are continuation claims, not remote possession or currency proofs.**
  Cooperative page serving binds an ephemeral HMAC-SHA256 key to the exact provider/requester,
  full group/logical/concrete scope, initial heads/seed, fixed accepted-log prefix and monotonic
  ten-minute lifetime. App handles additionally pin physical mount and sync instance; bounded
  request/MAC/current-member checks precede source I/O. Actual concrete-id matching follows the
  checked bucket load. At most 32 operations / 512 KiB of framed ciphertext are emitted per page,
  freshly sealed under current MLS without changing authorship. Already emitted positions and
  initial heads count as claimed held dependencies. Missing removed-author history cannot be
  laundered through a current provider; it reports that historical authorization is required.
  No seed is implicitly installed, source mutated, intent retired or receipt issued. Completion
  covers only the frozen prefix, not newer appends. Debug hides scope/content/cursors; the random
  provider secret is zeroized and never persisted or transmitted.
- **Prepared registry sources are local caches, not authority.** Capture authenticates the full
  bounded non-link vault record; a detached worker runs the same signed history, typed projection,
  gate and receipt-book checks as normal restore, without live authority keys. Installation checks
  preparation generation, current runtime/mount/member and exact saved bytes again. Every page
  rechecks the complete authenticated record (including gate/book), then uses current requester/
  provider/remaining-author membership and fresh MLS sealing. Stale, missing and corrupt sources
  never fall back to cached content. Cold/stale refusal is local, not a wire history claim.
  Four process-wide permits cover capture, queued/running work, detached results and retained
  sources; cancelling a future or remounting cannot refund a still-live body/job. Raw captures
  are zeroized on drop; reconstructed Automerge heap is not claimed to be securely zeroized.
  A cancelled blocking task may finish privately before its result is dropped, but never installs
  itself. Runtime integration must release actor/store locks before awaiting reconstruction,
  reacquire lifecycle custody for installation, and drop providers on lock/replacement. Automatic
  scheduling, fairness and hard wall-time/heap-byte guarantees remain outside this cache primitive.
- **Registry network pages are bounded member claims, not saved state.** Kind 20 binds requests
  to the actual requester transport peer and responses to the actual provider peer, whole query,
  group, requester key, request nonce/timestamp/MLS epoch and answer under a separate domain.
  Both full current member keys and current MLS epoch are checked; queued work rechecks before
  source I/O. Clients disclose registry ids/heads/cursors only to an already transport-bound
  current member endpoint. Eight pending responders, one per full requester across all buckets,
  expire for admission after five monotonic seconds. Independent global preauth (10/sec, burst 20),
  per-device (1/sec, burst 2, at most 4096 debt rows) and source-read (2/sec, burst 4) rails limit
  work. Watch replacement does not reset debt. A compromised member can consume those bounded
  resources or withhold data; fairness/proof of currency is not claimed. Response framing is
  capped at 524484 bytes after transport buffering but before body copies/signature work; decoded
  operations still require durable author/DAG/projection admission. No legacy fallback is allowed.
  The client publishes cancellation on drop/ten-second timeout and keeps at most four transport
  requests charged until the driver terminates them. Requester cancellation may not suppress an
  already queued provider read; responder handoff proves no delivery. No source is mutated or
  intent retired by paging. The receiver adapter below performs durable admission; runtime
  scheduling remains a separate integration requirement.
- **Receiver continuation advances only over saved pages, not provider assertions.** Four
  watch-bound passes per sync instance retain at most one 512-KiB page each, pinned to physical
  mount, requester and proven provider full identities. A detached batch validates every op
  through current author/DAG/projection/gate checks before one accounted atomic write. Invalid
  dependencies or a bad middle op persist no prefix. Duplicate and terminal-empty answers still
  verify inventory/current Open target and sync held bytes. A failed or uncertain save retains
  the page and old cursor, invalidates uncertain accounting and requires explicit retry after
  reconciliation. MLS advancement drops unusable pending ciphertext into a restart hold.
  Network cancellation leaves charged Paused state; one-second request/write pacing, a fixed
  ten-minute receiver-clock lifetime, 20001 request/write attempts and received-page ceiling,
  20000 received operations and 16 MiB total framed input bound repeated/duplicate work per pass.
  One initial empty-head fallback permits honest divergent edits without relaxing seed checks;
  repeated Restart or a restart after accepting a page holds instead of refunding work. Revocation
  stops work but retained handles keep their capacity until Drop. Saved progress survives restart;
  provider cursors are volatile claims, never persisted security state. This is not fair scheduling,
  proof of remote currency, owner finality, native lock enforcement or automatic runtime ownership.
- **One-shot publication limits application retries, not gossip lifetime.** `publish_once` waits
  for the driver's single attempt and bypasses Mewtual's legacy ciphertext retry queue. Production
  owns at most 16 compact 512-KiB payloads with 64-byte topics awaiting/entering this path, with
  non-waiting admission and capacity retained on cancelled queued commands until drained. The
  final closed-receiver check is the admission boundary: cancellation observed before it suppresses
  work; a race after it cannot retract the synchronous attempt. Normal libp2p caches and handler
  queues can retain bytes even after NoPeers/QueuesFull, and a lost acknowledgement is ambiguous.
  Submitted/Duplicate are not delivery or finality. This adds no identity, membership, epoch-gate,
  UI-lock or server-incarnation authority itself; the cooperative Server adapter owns its separate
  known-state checks. Native lifecycle policy and live scheduling remain required. Legacy
  publication behaviour and gossip limits are unchanged; input caps do not
  guarantee gossip accepts that size. This is a local API, not a new peer-controlled message kind.
- **Large checkpoints disclose their encoded size above the padding ceiling.** A P1 seed can be
  2 MiB. Once transported through the existing sealed-frame codec, a seed above 1 MiB receives
  no power-of-two padding bucket; group peers can estimate its size. The cooperative kind-22
  registry transport now uses that codec; it has no unpadded fallback below the ceiling.
  Seed parsing rejects compressed
  changes/document chunks before decode and checks the exact owner-receipted hash before loading
  Automerge. Registry pointer epochs remain hints: a large number authorizes no seed or history.
- **Checkpoint transport does not establish durable replacement authority.** Kind 22 requires
  a proven current full-member provider endpoint before disclosing the query. Its complete query,
  group, current MLS, requester and actual provider transport bind the signed response; the seed
  is separately group-encrypted under its concrete document key. A provider is not necessarily
  the selected owner. The client accepts a retained seed only through a private context minted
  within fresh kind-21 owner-proof verification, not public mutable receipt/proof fields. New
  authenticated owner discovery for that bucket, any MLS advance and runtime replacement revoke
  old contexts. Four retained non-Clone handles stay charged until Drop, even when stale; each
  has one 2-MiB seed, three attempts paced at one second, and a fixed 60-second lifetime beginning
  before discovery. Four separate network permits remain charged until the transport terminates
  after cancellation; each request has a ten-second deadline, checked after decode/validation too.
  These are resource/admission lifetimes, not leases or a promise that the owner cannot rotate.

  A provider retains eight metadata-only requests, one per full identity, for five seconds. Its
  independent global preauth (10/s burst 20), requester (1/s burst 2, 4096 debt rows), and source
  work (1/s burst 2) rails bound repeated reads. The source read verifies the complete registry
  inventory including absence and rejects corrupt/faulted units. Only the installed opening seed
  can be returned; a latest head whose successor is not installed is unavailable. No owner record,
  source, intent or receiver file is mutated. The response cap is 2,097,312 bytes after transport
  buffering but before body copies/decryption; it includes both sealed-blob length fields, nonce,
  tag, padding footer and signed-response framing. Temporary crypto/parser copies and one bounded
  source rebuild are additional to retained seed memory. Provider response handoff is not delivery;
  the four client slots do not account for bytes retained by the provider's transport driver.
  Withholding/flooding still harms bounded
  availability; fair scheduling and maximum-source latency remain acceptance work. A successful
  fetch is not finality, delivery acknowledgement or permission to prune. Explicit installation
  follows the source/recovery/replacement barriers above. Native vault-lock scheduling remains.
- **P1 bounds decoded changes as well as wire bytes.** Its v2 codec rejects compressed deltas
  and scans raw RLE columns without expansion before Automerge parsing. Action, cell, predecessor
  and expanded-string caps prevent small signed packets from declaring unbounded parser work.
  New operations require the inner author's current roster key, so resealing with a member's group
  key cannot mint shares for outsider identities. Registry validation binds predecessor ids to
  their causal root property; an allowed header write cannot hide a seed's protected slot through
  a cross-key predecessor. Typed Studio validators now provide corresponding causal semantic
  checks; generic callbacks are not an authorization policy by themselves.
  Studio body codecs enforce static operation grammar, complete envelope size and
  expected type/key; a `put_object` creator must match the supplied full verified author.
  The codecs alone are not live admission callbacks: no proof of membership, group binding,
  causal delta equivalence, aggregate projection caps, blob availability or persistence follows
  from decoding. Production writes use the separate typed causal/P1/store enforcement paths.
  Shared vectors pin valid Unicode-scalar titles, safe JSON integers, lowercase fixed-size ids,
  FileExpiry's three states and the existing declaration-order jam patch hash. Schema checking
  does not make a declared frame length or an indexed creator claim authenticated by itself.
  The read-only StudioIndex projection additionally checks scope against every live root value,
  recomputes record operation ids, rejects visible nonce/body equivocation, preserves insertion
  conflicts and removal provenance, and counts losing/deleted/over-cap content in its reader
  budget. It bounds already-parsed CRDT work (128,192 primitive ops, 6 MiB of visible keys/values),
  not wire parsing or retained signed history. Its asserted authors are still untrusted until
  a delta validator binds them to signed changes or a receipt-verified checkpoint.
  It cannot prove an immutable record was never causally overwritten, that a rename saw its
  target at the author's frontier, or that a missing tombstone was never deleted. The reader
  alone is neither signed admission nor checkpoint-size/recovery completeness verification.
  The original art-only Flipnote frame reader uses the same unauthenticated-claims boundary, with
  caller-supplied server scope and object/channel/epoch/dimension header checks, 188,192 primitive
  and 6 MiB reader bounds,
  and iterative cycle/missing-origin checks. Insertion op-id origins retain losing/deleted nodes;
  the signed causal validator must independently derive those origins and validate every
  property predecessor. Timestamp metadata is author-asserted, not an independent signed clock;
  it never orders frames or grants freshness. Over-cap/deleted frames retain all live CID evidence.
  The candidate adds complete sound/export records and a separate typed Score projection rather
  than treating their absence as empty art. Unknown unsupported state still refuses. A projection
  reader alone does not implement PIX verification, retention, signed admission or durable recovery.
  A separate `validate_index_change` checks Index deltas against the canonical
  domain operation and the author's complete dependency frontier. It binds record bytes to the
  change actor, requires fresh markers and immutable insertion/deletion/header writes, rejects
  deleted-id reuse and unknown targets, and requires exact same-property predecessor sets for
  mutable registers (including empty/partial/duplicate and cross-key predecessor rejection).
  Overflow objects remain valid targets; receiver-only creations do not. Concurrent deletion
  cannot invalidate a mutation justified by its own causal view. This callback assumes already
  authenticated accepted history; P1 still owns the signed actor/member/server/physical binding.
  It accepts checkpoint epochs only with the typed seed at the sender's dependency frontier;
  seedless epochs and mutation/reuse of immutable baseline provenance are rejected.
  The corresponding art callback `validate_frame_change` binds exact frame record
  bytes to the full change actor/domain envelope and derives insertion origins independently from
  a private historical projection at the complete dependency frontier. It includes hidden/deleted
  insertion nodes when selecting the first direct right child and never borrows a receiver-only
  smaller collision winner, creation, replacement predecessor or newly inserted sibling. Frame
  targets must be known and nondeleted at that frontier; observed-id reuse refuses, concurrent
  deletion remains compatible. Title/fps and pixel registers require exact predecessor sets.
  The existing public reader's erased-history rejection remains unchanged; only the private
  empty-head view can represent the empty past of a nonempty receiver, with unknown heads rejected.
  Historical queries retain the same byte/primitive bounds but repeat Automerge clock work;
  maximum-source latency is unmeasured and remains required before production scheduling. This
  callback alone grants no cap-edit policy or publication authority. Over-cap frames remain
  semantic targets so trimming is possible. `StudioTarget` now combines both validators with
  local cap policy and exact aggregate preflight through existing P1 gated edit/ingest. Concurrent
  ingress can retain a deterministic over-cap suffix; local edits cannot grow beyond its cap.
  Both paths encode the actual prospective 2 MiB Automerge checkpoint and complete 6 MiB recovery
  envelope including current operation bodies; refusal leaves document/log/gate unchanged.
  Borrowed value/count/aggregate checks precede cloning caller-mutable projections for topology
  reconstruction or compaction. The seed omits markers/tombstones/history and bounds conflicts
  to four values per field and 1024 fields without changing selected values. Whole-source recovery
  retains all alternatives/deletion authors/original positions and superseded operation bodies.
  Normalized seed positions are labelled, not misrepresented as original authored placement.
  The owner receipt/hash and canonical typed seed verifier remain independent requirements.
  Typed recovery validates source epoch/base-close consistency and uses source-stable opening
  provenance for Rewound, so retargeting cannot reset recovery ids and warning lifetimes. These
  codecs do not authenticate caller-invented provenance, persist evidence, retire intents or
  authorize pruning. The owned `StudioEpoch` now revalidates raw signed history, typed seed,
  opening/retained receipts and exact gate/log coherence on vault restore. Historical removed
  authors remain readable; every new edit requires current membership. The accounted Studio
  store verifies observed source presence OR absence against inventory before journaling an
  intent, then persists the whole source before returning prepared ciphertext. Receipt authority
  rejects before disk reconstruction; sealing/Fault results cross the same persistence barrier
  and never prune history. Uncertain writes/flushes poison accounting; retries compare complete
  envelopes and sync unchanged files rather than create another operation. Five-family inventory
  authenticates bounded Studio files and charges unpublished copies. Its mount-local generation
  fences stale/duplicate Studio budget handles, not interleaved raw writes by other P1 adapters:
  a sole coordinator remains required. Blobs/legacy snapshots are outside this inventory. No
  network snapshot authority, completed Studio settlement, live publication, retention guarantee
  or actor/native Save/Load follows from these store APIs alone.
  Awaiting-tenure overlay authoring is likewise an internal local-draft capability, not source or
  signing authority. New work requires checked source absence plus a current complete prepared
  preview at both authorization and commit; detached plans bind mount, numeric server, complete
  target, member key, owner, MLS epoch and exact authenticated Intents bytes. A parked request also
  binds target, provenance, basis, branch and a domain-separated digest of the full canonical
  operation, including its body. A mismatched request may finish the plan to release the bounded
  actor slot, but it receives only `Busy` and cannot claim another request's outcome.
  Unconfirmed history is excluded from automatic handoff and is capped at three live branches per
  numeric server, 8 MiB of authenticated physical Intents bytes per vault and 64 accepted
  operations per branch. Exact
  retry is classified before preview expiry, while any new operation after expiry refuses.
  Accounting changes only after a durable replacement and terminal disposal releases the live
  quota; fresh complete inventory reconstructs it. The common `EpochIntentBudget` writer tracks
  ordinary and Flow S replacements alike. The 8 MiB share is an admission limit on Unconfirmed
  growth, not a hard vault invariant: an ordinary edit can take the tally over the share, after
  which new Unconfirmed growth refuses until headroom returns. Gate 4 promotes P5 only for the
  reviewed lifecycle family and ticketed Unconfirmed Save: native commands and the Studio UI expose
  those local-authority operations behind the session gate and command allow-list. Direct Closing
  Save and unfinished repair commands remain unavailable; this exposure grants no source, signing,
  publication or settlement authority.
  The new explicit native Index/art transactions lend the sole mounted store only AFTER the
  actor's Ready rendezvous. Numeric-server persistence, UI commit and exact registry-incarnation
  locks are acquired without awaiting; a busy fence drops Ready and writes nothing. The blocking
  worker retains these guards and the sole live Server across caller/actor cancellation, saving
  the current MLS/device snapshot before Studio intents/source. Results recheck UI generation and
  server incarnation; guards drop before awaited events. A worker panic fails the actor closed.
  Current channel/member and complete-domain checks precede mutation I/O. Create accepts an
  existing object only as an exact retained-operation retry, never as a header-overwrite shortcut.
  Frame edits require exact declared-length, CID-valid, validated 192x144 PIX bytes in the mounted
  namespace and re-promote/flush before their reference. This is local byte existence, not remote
  possession. Create is two-file and can leave an unlisted
  object on index refusal. A cancelled operation already running may finish durably; cancellation
  suppresses stale results, not committed content. Finite scans/restores run off-executor but
  retain lifecycle locks and have no accepted-size latency guarantee. Views say local/provisional.
  Automatic receive, watched-owner settlement and detached repair run through their own actor
  authority/custody gates; the UI wiring exposes only the bounded local
  lifecycle/ticketed-Unconfirmed path above. Expiry remains enforced by store/actor checks rather
  than renderer timing.
  Successful native saves now attempt initial publication of at most two actual store-returned
  packets. Complete Save plus final-view success is required; partial Create exposes no batch.
  The same sole Server/store/native guards remain held from save through one-shot dispatch, so
  gate/membership/snapshot changes cannot interleave. Each packet still passes current full-author,
  MLS and routing checks. A shared two-second injected-clock deadline bounds added network wait,
  not save latency. Native cancellation/keepalive travel with the lease, preserving capacity while
  a cancelled worker finishes privately. Cancellation and reply closure are checked before/during
  sending; already admitted bytes cannot be retracted. After lease cancellation, residual driver
  work remains covered by existing transport publication slots. NoPeers/error/timeout/Duplicate
  do not erase local state or retire intents. Local/provisional views remain truthful because no
  delivery is claimed. Two Create packets do not make remote creation atomic or discover objects;
  unwatched objects still need discovery. Lost edits on watched same-epoch sources now use the
  automatic authenticated page catch-up described below; a page does not prove current finality.
  The new mount-shared persistent blob guard separately protects all retained Studio source and
  verified seed-only CIDs, pending intents and retained/staged typed recovery. A seed's hidden
  replacement value remains required even after a successor overwrites it. Writes union holds
  before I/O; uncertain writes never unpin. Only an explicit, complete, exclusive reference scan
  with the same mount/generation can replace the set. Full-group scope unions numeric aliases.
  Unknown/corrupt/unsupported/partial records and the 65,536-reference rail refuse reclamation,
  never authorize from a truncated list. A bounded absence-only P1 filename scan permits empty
  new mounts; restored mounts may defer GC until an explicit or Studio-triggered scan succeeds.
  The PIX pre-hold precedes exact read/promotion, and subsequent budget scans cannot erase it.
  The guard stays locked through actual synchronous deletion; store drop revokes stale handles.
  This covers existing unlist/upload-cleanup cache deletion, not arbitrary external filesystem
  mutation, storage corruption, a new blob quota or remote possession. Expiry/tombstones alone
  do not override still-retained recovery/history. No new durable format, finality or expiry policy
  is introduced; kept-copy release and non-held staging cleanup keep their separate semantics.
  Studio's owner-close and adjacent settlement preparation now reuse the existing P1 validators
  over its private typed-admitted source. Owner/group/full-key/independently observed tenure
  checks precede signing; an exact resumed decision uses the named dependency closure, never
  later live content. Malformed public Rust close/receipt fields are bounded before encoding.
  A settlement plan requires the exact held current-owner receipt in ordinary Closing, not
  adoption or Fault, and fingerprints the whole restart unit including quarantined input and
  repair state. It retains complete included/excluded author envelopes; nonce-derived ids alone
  are not enough to retire an intent. Recovery includes compaction omissions even when every
  operation was included, including deletions, overflow conflicts and original frame gaps.
  Constructing a separate successor preserves repair anti-replay state but grants no durable
  replacement, pruning, retirement or publication permission. The accounted owner journal,
  recovery-before-replacement barrier and runtime driving are still to be connected for Studio.
  No new wire/vault format, live rotation path or native recovery control is added by this slice.
  The explicit Studio exchange adapter now sends only an exact OWN operation already in the
  saved Open source, through current-member checks, both retry durability barriers, fresh MLS
  sealing and one-shot dispatch. It cannot create an unsaved edit or bypass native PIX ordering.
  Its opt-in inbox intercepts both Studio tags before legacy ingestion. It admits no document
  until the existing typed store validates channel-bound roots/causal changes/caps and persists.
  Current full local/author identity and MLS are checked again after queue delay. Logical watches
  are capped at 16, packets at 16 times (256 KiB + 78 bytes); channel/epoch replacement revokes
  old generations without resetting logical full-author debt. The separate Studio pre-auth bucket
  is 50/s burst 200; author/document buckets are 10/s burst 50 with 4096 rows, reclaiming only
  fully refilled rows. These are fixed extra rails, not cross-inbox fairness. No queued frame is
  an accepted edit, blob-possession assertion or delivery receipt. Cancellation/refusal keep
  durable intents and require explicit retry. Native automatic recent-target receive now uses
  the same lease/sole-Server worker, current snapshot-before-ingest ordering and typed store.
  Only checked local Read/Save descriptors add watches; incoming packets cannot choose a source.
  Same-target access preserves the inbox; 16-target eviction revokes it. One coalesced hint and
  one paced worker per exact native incarnation avoid per-packet tasks or event-consumer cycles.
  Current channel/mount/member/MLS checks precede disk work. A delayed old-actor update cannot be
  emitted under a replacement numeric server id; the UI/incarnation fence lasts through emission.
  Automatic inventory uses LOCAL limits of 1024 visited entries, 1024 records, 8 MiB aggregate
  authenticated reads and 256 KiB aggregate cold validation bytes, including unrelated saved P1
  sources. Its mount-local LRU stores every family's context-free accounting validation
  (C-3 runtime 15.2, M1). Every hit still authenticates the actual file and binds its scope/filename,
  full plaintext-wrapper digest and physical size, including gate/receipts/quarantine/history.
  That metadata LRU stores no key/plaintext/doc, completed inventory, membership decision or budget authority.
  **What it does hold beyond accounting:** for each Intents record, its inventory facts. For an
  Unconfirmed branch these include the preview provider's device id, the observed MLS epoch and
  the observation time. The same facts are in that record on this device's disk, and decrypted
  into memory by every scan of it, but the memo keeps them resident for the mount's life, UI
  lock included. That is a residual, not a new disclosure: nothing leaves the device.
  **Admission depends on history** (15.9, MEDIUM-4): warm bytes do not count against the cold
  rail for any family, so a vault receive refuses cold is admitted once its records are warm. A
  refused scan warms what it validated before reaching the rail, so a later attempt does less
  fresh work, and a remount makes the vault cold again. The rail still bounds each scan's fresh
  validation, which is its purpose. Warm bytes still count against the 8 MiB authenticated-read
  rail, since every hit reads and authenticates its file, so a vault admitted warm authenticates
  up to 8 MiB on each receive turn, where it used to stop at the cold rail.
  **Entries outlive their records until a complete covering scan prunes them**, which includes
  another member's Unconfirmed provider id after leaving a server.
  It holds up to `MAX_ACCOUNTED_RECORDS` (65 536) entries, so a complete scan never thrashes it,
  and every inventory-issuing scan, once complete, prunes the entries of records it did not find,
  within its own coverage (C-3 runtime design 15.3). About 300 bytes an entry (an Intents entry
  about 60 more, for its boxed inventory facts), so about 20 MiB at
  the bound, resident for the mount's life, UI lock included. **The bound is the only
  unconditional limit**, and it holds whatever puts an entry. Pruning keeps it near the vault's
  record count only as often as a covering scan completes, and some do not complete: receive's
  scan refuses any vault over 1024 records. Until one does, entries stay for records deleted since
  the last, including any a peer caused this device to store and then remove.
  New/staged files still count; reference scans cannot skip CID enumeration. Remount clears reuse.
  A large cold record refuses before read; a changed candidate above the cold rail refuses before
  reconstruction. Warm footprint metadata alone does not authorize a mutable target above its
  256 KiB cold source rail. Separately, the sole mounted store may retain one owned verified Studio
  graph/gate after explicit access or a successful receive. It is moved, not cloned, and its
  encoded source must be at most 8 MiB; this is NOT a resident-heap cap. Every take checks current
  actor/group/MLS and authenticates the full actual wrapper with exact scope, mount, size and digest,
  including gate/book/quarantine changes even when heads and file length match. Ingest still uses
  fresh inventory/budget verification and the existing typed gate/durable save. Failed takes drop
  the graph; unchanged flushes preserve its actual physical stamp, not normalized in-memory bytes.
  Explicit reads can rebuild a changed source; background receive cannot make that large cold
  fallback. Index refreshes preserve the opened art slot without retaining a second graph.
  No vault key is added to this retained unit; it contains plaintext document state like an open
  document. Remount drops it, and existing locked/busy/incarnation guards still govern access.
  A partial scan never grants storage admission. Scan/snapshot/ingest failure
  pauses until successful explicit access, emitting a non-settlement pause warning; inbound
  traffic cannot refund that hold. Busy/locked native access retains the bounded inbox; consumed
  bad or dropped packets still need retry/catch-up. The one-second pacing cannot be bypassed by
  pending-state churn. This bounds automatic work, not worst-case latency or full-size service;
  only one prepared active source is reusable and cold dense reconstruction remains expensive.
  Explicit Save warms inventory; Read warms only its actual Studio record and resumes a paused
  watch. The pause event has no user-owned UI listener yet.
  No pixels are fetched, no intent is retired, and no update is emitted for duplicate/quarantined
  input. Catch-up/seed discovery and bounded unopened-key service use this same coordinator;
  Gate 4 subsequently integrated automatic owner rotation and Registry tail/pointer publication.
  The Gate 6 candidate extends these existing paths with typed Score and complete sound/export
  validation; it does not grant authority to an untyped or silently truncated projection.
  Studio's cooperative page adapter now shares the existing prefix/ancestor walk rather than
  copying the registry protocol. A distinct cursor domain binds the actual Studio type/key and
  channel as well as full group/provider/requester/concrete epoch/initial heads/seed. Registry
  MAC bytes are unchanged. Page service checks authority/MAC/expiry before source I/O, then
  authenticates the full prepared wrapper without cold reconstruction. The low-level caller
  still owns requester transport authentication, provider mount/runtime lifetime, aggregate
  rates/instances and native custody: this object is not a new network or unlock capability.
  The same 32-op/512-KiB page limits, fixed expiry, current-member resealing and removed-author/
  checkpoint holds apply. A page never transfers a raw seed or authorizes replacement.
  Page ingest requires the exact Open target and current-MLS envelopes, moves the same checked
  graph, validates all entries and saves once. Bad middle input saves no prefix. Write uncertainty
  may leave old state or the complete new page; errors return no progress and require reconciled
  exact retry. Empty/duplicate pages still check accounting and flush actual held bytes; empty
  absence creates nothing. Saved counts/frontier grant no finality or intent retirement. Original
  request heads/seed must remain fixed for continuation; returned heads are for a later pass.
  Studio now uses additive authenticated page kind 23, a separate response domain and the same
  aggregate queues/rates/outbound/receiver slots as Registry pages. Full endpoint/member identity,
  current group/MLS, exact channel/key/watch and fixed request deadline are checked before serving
  and again at completion. Detached connected-only requests retain lower-driver capacity on
  cancellation; no actor/native lease is held while awaiting them. The existing native receiver
  drives paced same-epoch catch-up on a five-second idle wake as well as work hints, with bounded
  gossip/service/client turns and save-before-cursor progress. This is not proof of currency.
  Cold watched source captures are capped at 8 MiB encoded input and verified outside the actor;
  Registry and Studio share four process preparation permits through actual worker completion.
  Installation rechecks exact authenticated wrapper/mount/actor/owner/MLS; healthy supersession
  retries, corruption remains a sticky storage pause. The single source and bounded inventory
  limits remain. Unopened saved-key service and automatic Index/art discovery now run through
  that same unlocked actor/native receiver. An authenticated request remains in its original
  capped queue while one private exact-request interest prepays source work. Cancellation,
  expiry, enable-generation replacement or same-key request replacement cannot reuse that right;
  shared rate debt survives re-enable. Source work creates no receive watch or new authorization.
  Registry source graphs have a fixed 30-second retention deadline to free shared preparation
  slots on idle servers; running/queued workers and held results keep their permits until actual
  release. Prepared Registry sources and exact Studio-bootstrap installation results enter only
  the existing full-wrapper/digest/physical-size inventory LRU; unrelated Registry writes do not
  gain cache credit. A large local Registry can be prepared for accounting while its
  adoption is deferred; no size-only check credits storage or asserts content validity.
  Local owner snapshot failures retry on 30-second Clock cadence, never per query. Discovery
  prefers the proven connected owner before the four-peer cap, rechecks the original UI watch,
  and drops late work after watch/channel replacement. Fixed-expiry proof is reacquired for a
  saved Closing state, not extended. Recently accessed watches themselves remain volatile.
  Registry bootstrap is checkpoint-only; Registry tail/pointer publication, automatic owner
  rotation, succession and recovery controls remain Gate 4. Kinds 24/25 share Registry's authenticated head/seed engine and all
  its pending/rate/outbound/retained limits. Type, channel, object, full endpoint identities and
  exact request are bound separately from Registry; expected Automerge hash precedes typed
  seed decode. Only a fresh current-owner response mints private selection authority, never a
  public Hint or registry pointer. Completion checks runtime, mount/server, MLS, channel and
  attempt/selection freshness. A cancelled job retains both outbound and seed capacity in the
  driver, not just while its caller exists. Generations for arbitrary object keys are weak and
  reclaimed, with sixteen Studio service registrations per head/seed family. These and the older
  cooperative Registry wrappers now use connected-only requests, not implicit endpoint dialing.
  Studio adoption first saves the full Closing/Fault source, then typed recovery, then its
  separate successor. No intent retires from seed values. A staged third snapshot still holds
  settlement, failed I/O requires accounting reconciliation, and exact installed retry preserves
  newer edits. Normal restart v1 is unchanged; adoption v2 accepts legitimate Studio epochs above
  Registry's separate lineage ceiling. Owner proofs additionally require the prepared local MLS
  snapshot, exact saved source/journal agreement and both durability barriers. Warm-only source
  service cannot turn missing indexed files, corruption or a cold source into a valid empty head.
  Exact retry recognition belongs to the retained signed
  envelope/gate, not timestamps or marker-only success.
  Vault restoration tests dependency and duplicate presence using metadata from Automerge's
  applied graph, not a saved or peer-asserted index; unresolved queued changes do not count.
  It can use indexed current-view reads only when an authenticated change's
  complete dependency list exactly matches the actual current frontier. The restore loop derives
  that fact locally immediately before immutable validation; saved metadata or a peer cannot assert
  it. Only that path skips the semantic validator's already-proven dependency-presence check.
  Older/concurrent/proper-subset views still use historical reads. Property-specific predecessor
  checks and marker-only winner semantics are unchanged, as are live edit/ingest authorization.
- **Epoch-managed document operations above 1 MiB disclose their coarse size.** P1 bounds the
  domain envelope to 64 KiB and a whole signed operation to 256 KiB, so conforming creative and
  registry operations stay inside the existing sealed-op padding ladder. The generic legacy
  replication codec still permits larger Automerge operations: above the one-MiB padding ceiling
  they receive only the authenticated length footer, not another power-of-two bucket. A forwarding
  group member can therefore estimate such a legacy bulk edit's ciphertext size. Content remains
  encrypted, the network frame cap still bounds work, and P1 admission rejects the operation.
- **Adaptive stream resolution reveals only a coarse call preference.** A call participant sends
  one of four height buckets, not its exact window, monitor dimensions, or pixel ratio. The sender
  treats it as an advisory upper bound and independently caps each WebRTC encoding. Screen tracks
  stay parked until resolution, bitrate and frame-rate parameters apply; rejection pauses the edge
  or stops sharing if parking fails. A malicious
  receiver can request 4K but cannot raise the sender above the sender's own capture/bitrate cap.
  H.265 and other efficient codecs remain runtime-negotiated capabilities, not security
  guarantees. WebRTC encryption protects media in transit, while endpoint capture, decoding and
  the who-is-watching/timing/bitrate metadata remain trusted-endpoint and traffic-analysis risks.
  Shared audio is off by default. Surface audio is whatever the platform chooser grants and can
  accidentally include Mewtual's own playback; separate sources require a fresh chooser grant per
  source/session, discard the picked picture, and mix only their audio into one WebRTC track. A
  bounded 0--200% master and bounded per-source gains change that shared track only; overlapping
  boosts can clip and never grant a new capture source. At most eight explicit application-audio
  grants can be live, and the graph/output track is released when the last grant ends. The call microphone is a separate sender. The
  WebView/OS decides whether window/application audio is available, so this is not an OBS-equivalent
  native process-capture guarantee.
- **Jam events are untrusted member input rendered inside a receiver-owned, bounded synthesizer.**
  The jam engine does not authenticate peers itself: the identity used to mint its source-channel
  capability must come from the already authenticated call connection, never a sender-controlled
  frame field. Each callback must present the opaque capability minted for that exact generation;
  reopen/removal invalidates older queued callbacks before they can recreate sender state. Every data-channel
  delivery is size- and rate-charged before parsing; valid patches pass the same strict, unknown-
  field-rejecting validator regardless of whether they came from the wire, local storage, import,
  or take playback. A patch selects values inside one fixed topology and cannot introduce graph
  edges, code, samples, assets, or feedback. Per-peer held limits, a global tail-inclusive voice
  limit, release/hold watchdogs, and receiver-owned gates/limiter bound honest-renderer work and
  output. Take playback also releases unmatched held notes when the event log ends before its
  bounded tail teardown, rather than sustaining them for that whole grace period. A malicious
  member can still make bounded noise and a modified receiver can discard or
  reinterpret events. Remote events are not queued into a suspended audio context. A content hash
  promises the same recipe, not identical platform audio. Exact legacy note frames have a separate
  receive-only compatibility domain; they gain no patch, drum, clock, or recording authority.
  Ordered data-channel delivery is extended across asynchronous digests, and outbound wire,
  renderer and recorder events share one immutable published recipe. Unopened edges retain no
  musical history: open establishes only the current patch, then fresh live events begin. This
  chooses an explicit sequence gap over collapsing paced history into a receiver-budget burst or
  losing a release behind an accepted attack. Reconnects cannot reset the call-epoch peer budget or leave
  an old channel's digest blocking its replacement. A fresh channel gets one exact raw recipe
  reannouncement only when that peer/call budget previously observed the engine hash-verify it;
  this avoids another digest/token while distinct recipes still spend the persistent patch bucket.
  Sender publication pacing matches the receiver
  patch budget, while publication/playback preparation coalesce to one running plus one latest task,
  and take scheduler work and pending drum digests have separate per-pass/per-source/global caps.
  Drum work is channel-generation ordered with bounded backpressure: dense takes await it, local
  echoes share one causal event lane, and stale generation work cannot block its replacement.
  Local causal overflow advances a resettable queue generation, cancels queued/active engine work,
  and releases current local voices before a fresh queue admits more input; retained attacks cannot
  emerge later as a local burst.
  Monotonic room/source admission generations advance on both gate edges, and engine render
  generations advance on revocation, making Deafen and mute sticky for work already queued or
  hashing even if the user reopens audio before that work settles. Local input captures its room
  generation before asynchronous patch publication;
  the audible voice count is not treated as a bound on unresolved crypto work. Auto-muted music retains only a tiny separately rate-limited
  `t:"s"` lane so an instrument flood cannot freeze consent withdrawal or call state.
  Deafen and member removal are security-relevant integration points: shipment requires Deafen to
  destroy the instrument room graph (including buffered wet state) and release all voices including
  local previews. Remote-controlled call cues cannot create/resume suspended audio, have a global
  overlap cap and receiver limiter, and are cancelled at Deafen/leave. Roster revocation must close the removed
  member's call/data-channel connection. Recording consent and performer labels are honest-client
  coordination until the planned group-bound, domain-separated durable per-participant aggregate
  lane commitments exist; another endpoint can always record audio or events without announcing it.
  Within an honest renderer, take reconnect lanes attributed to one participant share allocator,
  choke and digest budgets. That owner is an engine-minted playback domain, not the unsigned stored
  label, so a forged label cannot consume or choke a same-named live member's voices. Local live
  seed material is bound to the authenticated self fingerprint rather than the reserved `me` lane.
  A recorder lease exists only for events received while its consent gate is already recording and
  binds the recorder's monotonic uninterrupted-recording generation before the App causal queue;
  withdrawal/restart invalidates pre-withdrawal work, and events received while paused have no
  lease. Disconnect retracts that edge's consent and reconnect must announce fresh consent.
  Shared-take ingress rejects oversize listings before whole-file download, bounds encoded bytes
  before decode, keeps a bounded per-call LRU, and serializes/coalesces downloads. Each take read
  has a native cancellation signal observed inside the detached chunk fetch; cancellation
  acknowledgement preempts the stale JavaScript slot, while a transport-owned keepalive retains
  its native charge until the exact submitted request responds, fails or times out. A four-request
  process cap and exact UI-generation cleanup therefore bound rapid call churn even against a
  withholding peer. Explicit lock cancels every slot, and unclaimed
  begin-only registrations are replaceable at the cap so reload cannot starve playback. Inline
  reads that omit a caller token receive an unforgeable native token under the same cap and lock
  cancellation, while caller tokens include a per-WebView random nonce so reset counters cannot
  collide with a still-active predecessor. The transport trait has no cancellable-request default:
  every implementation must declare the ownership rule. Both libp2p and in-memory queues retain
  accounting at their underlying owner; wrappers delegate, while test fakes that do not transfer
  it own no detached work. Compatibility or alternate transports therefore cannot silently inherit
  an unbounded path. A completion must
  still match the exact call lease/server/channel/deck and pass the current listing and trust policy;
  a fetch started in an old call cannot populate a later call even when both adopt the same CID.
  Tokenized progress is accepted only for that exact call/server/CID/token, and tokenless progress
  is excluded while a take is on deck, preventing queued old-group provider telemetry from
  repainting the replacement deck.
  The current take `group` value is device-local
  scope metadata, not the stable group binding needed by phase 6.
- **Call identity is the full verified device id (Gate 5, C0d).** The sync layer queues the
  32-byte `DeviceId` it derived from the signature key that authenticated a `KIND_CALL_SIGNAL`
  request; the actor, the bridge and the frontend carry it as 64 lowercase hex (`from`), with the
  8-hex fingerprint (`from_fp`) only as a display label of the same id. Outbound signals are
  addressed by full identity and route only through that exact current member's own signed record
  (`peer_for_device`); a malformed or 8-hex target is refused at the actor boundary. Queue fairness,
  frontend call maps, roster revocation, jam capabilities/budgets/voice ownership and draw claims
  all key on the full identity, so a member who grinds a device key sharing a victim's first four
  bytes (about 2^32 work) can no longer receive the victim's signals, take over its call edge,
  share its fairness bucket, alias its jam lane or claims, or shield a revoked device by sharing
  its fingerprint. The legacy fingerprint resolver (DM invites only) refuses an ambiguous
  fingerprint. **Residuals:** profiles, names and presence remain fingerprint-keyed app-wide, so a
  twin renders under the victim's name (call and Studio labels add a longer id slice and "shares a
  fingerprint" when two roster members collide) and an online twin makes the victim look online
  (addressing widens; routing does not); DM invites are still addressed and deduplicated by
  fingerprint, so a twin can replace a victim's pending friend request; jam drum seeds and
  `jam-take:v1` `parts` deliberately keep the published fingerprint label (reproducibility, not
  authority), and the recorder refuses a take whose participants share a label; the call-signal
  transcript binds neither recipient nor a nonce cache, so a member can re-deliver another member's
  signed signal (SDP, ICE, presence or `bye`) to a third member within the 60 s freshness window,
  which can briefly drop a rejoined peer's call edge and claims (DTLS still binds every media and
  data-channel endpoint); and `peer_for_device` inherits the documented peer-id squatting caveat of
  member records.
- **Flipnote frame claims are advisory coordination on the call `draw` channel (Gate 5).** A claim
  frame's authority is only the opaque per-generation capability minted for the authenticated peer
  identity; no frame field names a sender. Every delivery is charged before parsing against that
  identity's call-epoch gate (the jam `inst` limits: 1024-byte cap, 80/s burst 160, auto-mute after
  10 s exhausted in 60 s, which also retracts the peer's claims) and must fit 200 bytes before
  `JSON.parse`; the exact seven-key schema rejects everything else. Sequence numbers are bound to
  (capability, sender session): duplicates, reorders, retired sessions and a fifth session per
  capability are refused, and every check runs before any state changes. Claims and asks are
  bounded (4 live per sender, 2/s burst 10 per identity per call; releases uncharged) and expire
  90 s after their last receipt on the receiver's own monotonic clock. While a peer's connection
  is not "connected", its entries are hidden. Its deliveries are charged as usual and then held: at
  most 32, malformed ones refused, and all discarded on auto-mute. They are applied only when this
  side sees the link recover, so an unreachable peer cannot assert presence. Its entries are
  dropped when its channel closes or is replaced, when it is removed, or when the call ends. One
  claim session exists per call join. Only an explicit release of a live claim hands a
  frame to a live asker. The Studio asserts claims only while open on the call's own server, so a
  Studio browsed elsewhere cannot leak another server's object/frame ids or edit timing into the
  call (which would link per-server identities). Claims never gate an edit or a save, and no
  Studio edit, save or native path consults them. A modified client can claim every frame, never
  refresh or release, or ignore Pass; the cost to receivers is bounded as above and the honest UI
  shows such frames as claimed or contested, never locked. Older builds have no channel 8 and
  neither send nor see claims.
- **A shared file is authenticated bytes, not trusted content.** Explicit save streams into a
  non-overwriting `.part`, checks the declared size and whole-file content address, sanitizes the
  peer-provided name to one leaf, atomically publishes only after verification, and reveals it in
  the file manager without executing it or invoking a shell. This blocks path/argument injection,
  overwrite races, corrupt-provider substitution and automatic execution; it does **not** make the
  payload benign or protect an external application the user later opens it with from its own
  parser vulnerabilities. Inline image/video/audio loads through a CSP-limited, `nosniff`,
  range-bounded custom scheme but is still parsed by the platform WebView/media stack. The scheme
  emits a body only when a bounded container signature matches the exact allowlisted MIME; SVG,
  mismatch and unknown inputs receive a bodyless denial because an octet-stream body may still be
  media-sniffed. Cached heads/chunks bind to the complete sorted set of at most four compatible
  encrypted manifests rather than only the member-claimed plaintext CID. Alternative encryptions
  must agree on total size, MIME and ordered plaintext chunk CID/size/MIME; each alternative still
  passes its own ciphertext CID, wrapped-key AEAD, plaintext CID and length checks. Any incompatible
  claim or excess variant fails closed, and adding/removing a fallback invalidates the cache identity.
  Whole-file downloads retain their final plaintext hash check. Upload reuse requires complete local
  verification, not index presence. Before any decryption, reuse compares the index plan with the
  actual upload's size and ordered chunk identities, preventing a tiny upload from triggering a
  hostile repeated large-chunk plan. Re-upload publishes fresh ciphertext under fresh addresses;
  replacement is limited to a fully signature-verified local-device row at the same name/path/CID.
  Publication confirms local possession and index posting, never remote replication or retention.
  Cache access and the synchronous URI-responder
  publication are generation-gated, so a disk/network read that finishes after explicit lock
  cannot refill plaintext caches, reveal a stale size, or publish its already-built body. Storage
  inventory separately authenticates every exact encoded chunk reference and joins verdicts to an
  atomic manifest/listing snapshot. More than four distinct exact references for one ciphertext
  CID makes that CID and its dependent manifests unreadable without attempting unbounded repeated
  decryption. Native cache publication is additionally bound to the process-local server
  incarnation, while webview continuations require their exact still-unlocked view. A plaintext
  export's verified no-replace publication and reveal hold the same exact unlock-generation commit guard,
  so an older export cannot become visible after lock or a later unlock. Before a
  server is founded/joined, the user chooses local on-demand, specific-member, or everyone trust;
  the bounded per-server policy is vault-sealed. On-demand is the default. It leaves passive shared
  media, custom emoji, card thumbnails, event images and call-jukebox tracks inert;
  specific-member trust checks the file's authenticated full device identity. The short 32-bit
  fingerprint remains display-only. An explicit Load/Play/Open/Download click overrides the
  passive-fetch policy. Bytes fetched into the blob store occupy one authenticated,
  XChaCha20-Poly1305-sealed vault copy; explicit export creates a separate plaintext Downloads copy.
  This reduces unsolicited decoder exposure but does **not** sandbox a decoder or make a trusted
  member's file benign. Automatic whole-share mirroring remains unimplemented because the sealing
  disk store has no quota; enabling it first would permit storage exhaustion even by an opted-in
  trusted member.
  Third-party HTTP(S) images always require a click, even under everyone mode: they have no file
  attestation, disclose the client address, and may target loopback/private-network services.
  **Third-party player frames** (cards in chat for any of the allow-listed embed hosts, and YouTube
  tracks on the jukebox deck) are the same class of exposure and are gated the same way, with one addition. Like
  a remote image they carry no attestation and disclose the client address; a queued video id is a
  claim by whoever queued it, and no device contacts the provider on a peer's behalf to check it.
  Unlike an image they keep running once loaded, so a grant to load cannot be the end of it: a card
  is mounted only while it is on screen in a visible window and reverts to an inert chip otherwise,
  and a deck frame is unmounted when the room leaves the track. Cards are click-only by default. A
  device-wide, vault-sealed **Chat & Media → load these cards without asking** preference (default
  off, and re-read as off whenever the sealed record is unreadable or the app is locked) replaces
  that click, and replaces only that click: it does not relax the on-screen/visible-window
  condition, because that condition is about frames running where nobody is looking rather than
  about permission. The preference is deliberately not per-server and not part of file trust: an
  embed's host set is fixed by the CSP so it cannot be aimed at loopback or a private LAN the way a
  remote image can, and it has no author attestation a per-member policy could act on, so the
  decision is only ever "may this device disclose itself to the allow-listed embed hosts without
  asking". Note the scope honestly: it is **one switch over the whole allow-list**, not a per-host
  or per-service consent, so every host in `frame-src` is covered by it, and adding a host extends
  an answer the member already gave rather than asking again. Deck playback of a linked video stays
  a per-track explicit approval regardless of this preference. `frame-src` admits exactly the
  allow-listed embed hosts in `tauri.conf.json`, **currently seven**: `open.spotify.com`,
  `www.youtube-nocookie.com`, `w.soundcloud.com`, `player.vimeo.com`, `player.mixcloud.com`,
  `embed.music.apple.com`, `embed.bsky.app`;
  no third-party **script** origin is admitted, so every deck frame is driven over postMessage
  rather than by loading a provider's API script into this document. Only three of the seven are
  deck sources at all (`youtube`, `soundcloud`, `vimeo`, enumerated natively as
  `JUKE_LINK_SOURCES`): a queue entry naming any other source is not a playable track and is
  skipped by the document reader, and a call-transport frame naming one is refused at the wire
  edge. A frame is
  sandboxed without `allow-top-navigation`, and replies from it are evidence (position, player
  state) that can move only the local player, never the room's transport. This narrows disclosure
  to a deliberate act per member per entity; it does **not** make the provider's player benign,
  and it does not conceal from the provider that this address watched that video at that time.
- **A sealed local reconnect route is a narrow continuation of a completed direct join, not LAN
  discovery or durable presence.** After direct admission, the joining installation may seal at
  most two literal-IP TCP/QUIC routes to the named inviter that were actually used by an outbound
  Noise-authenticated connection. Inbound ephemeral ports, DNS, relay circuits, untried invite
  candidates, WebSocket wrappers, and admission-only helper/reply/switchboard contacts are not
  retained. Durable `ReconnectPolicy` provenance prevents a new non-direct admission with no route
  from later being mistaken for migration consent. On every later dial the sync layer reparses the
  terminal peer binding, requires exactly one current roster member's signed descriptor to claim
  that transport peer, rejects removed/replaced/ambiguous claims, and charges the shared
  process/server/peer/endpoint/prefix budget. Every completed admission now makes one bounded
  best-effort PEX request over the existing connection, so the first snapshot can retain signed
  member descriptors even after a reply/helper join. This request fails if the connection closes;
  it cannot implicitly redial a temporary contact. Learning records is independent of local route
  authority and does not make the reply/helper path restart-safe by itself.

  The route remains self-asserted at the device-to-transport boundary: Noise proves control of the
  transport key, while the member's signed descriptor is the only link from that key to a device.
  It can be stale or unreachable and does not prove the person is online. The private address is
  vault-sealed, never put into PEX/rendezvous/the UI, and reconnect dial logs expose only address
  family and transport shape. Broader same-LAN discovery remains absent because naive mDNS would
  disclose peer identities and create an unauthenticated address-injection surface. Pre-v3 records
  decode as `LegacyPending` and can learn this hint only after one successful overlap in a group
  with exactly one other member and one unique signed transport claim. That one-time migration
  accepts private or loopback routes only, so public rendezvous and ambiguous/helper-bearing groups
  do not become durable by accident. If both peers are already isolated with no old hint, no local
  state can reconstruct the missing address; one fresh invite, rendezvous/relay path, or future mDNS
  discovery is still needed.

- **An out-of-band member recovery code is narrow dial consent, not new membership or route
  proof.** The code is group-bound, signed by a device that must still be in the current MLS roster,
  expires after ten minutes, and contains at most four terminal-peer-bound literal-IP TCP/QUIC
  routes. DNS, relay, WebSocket, unsafe host classes, foreign peer ids, wrong groups, future,
  expired and self codes fail before dialing; accepted candidates spend the shared endpoint,
  prefix, peer, server and process scheduler. Private/loopback addresses are intentionally allowed
  because the user explicitly transports the code and same-LAN address churn is the recovery case.
  The signer's exact transport peer must also be the unique peer in that device's current signed
  record; re-signing another member's peer is rejected. The signer can still direct bounded attempts
  at sockets it does not own, and the code exposes its candidate addresses to the recipient.
  Applying seals only expiring permission before dialing and preserves the prior proven contact.
  No pasted route becomes durable until outbound Noise authentication proves the peer; bounded
  process-local success evidence survives a short close edge long enough for the vault worker, but
  still requires the pending/authorized peer and a unique current roster claim. Expiry is rechecked
  under the final write lock. A code cannot repair an expired first-contact reply or make two
  NAT-isolated devices reachable without some viable route.

## Protocol- / crypto-enforced (a modified client CANNOT bypass)

| Guarantee | Mechanism | Where |
|---|---|---|
| Message confidentiality / integrity | MLS group encryption; only current members hold the key | `catcoms-mls`, `catcoms-replication` |
| Membership authenticity | MLS commits; joins admitted only via a signed invite + committer | `catcoms-sync::serve_join` |
| **Invite admission** | Two paths, both role-gated. Direct: the admitter must be the **named inviter** *and* an **authorized committer** (leaf rank ≤ `max_committer_rank`; at rank 0 that is exactly the owner). Relayed (Option C): a non-committer admin broadcasts a signed Add-request and the **owner** re-checks, before committing, that the requester **is** the invite's named inviter and that that inviter is owner/admin **right now** per the owner's local authoritative roster. A member who is neither **cannot get anyone admitted**, even with a self-minted invite. | `catcoms-sync::{serve_join_inner, on_add_request, inviter_is_authorized}` |
| Owner identity | Owner = MLS **designated committer** = lowest leaf index; cryptographic, not stored | `catcoms-mls::designated_committer` |
| Admin-set authenticity | Two copies, different jobs. The **authoritative** admin set is the owner's **local** `admin_roster`, which is not in the shared CRDT, so no member can write what the admission gate reads. The CRDT carries one **owner-signed whole-set roster** value (`gen ‖ owner_pk ‖ n ‖ fps ‖ sig`, signed over `ROLE_ROSTER_DOMAIN ‖ len(group_id) ‖ group_id ‖ gen ‖ n ‖ fps`), read fail-closed against the *current* owner's full device id, for **display/propagation and a relaying admin's reject-only pre-flight** - not per-fingerprint grants, and not admission. Scope of the guarantee: a modified client **cannot forge** a roster the reader accepts (that needs the owner's signing key) and cannot alter the set the owner admits from; it **can** delete the published copy or fill it with junk, which is cosmetic (reads fail open), and it **can** stale-replay a validly signed older copy, which is **not** cosmetic: that reads cleanly and denies a newly-promoted admin's relay until the owner republishes. Both are liveness only, because admission never consults this copy. See item 3's residual (no per-reader high-water). | `catcoms-sync::roles::read_published_roster`; `catcoms-sync::inviter_is_authorized` (owner-local `admin_roster`) |
| Member-removal authorization | Removal is **owner-only**: `request_remove` rejects a non-owner, and the committer ignores any inbound remove request whose requester isn't the owner (signature-verified, so a forged owner-claim fails). A modified member cannot get anyone removed. | `catcoms-sync` (on_remove_request gate + `request_remove` Unauthorized) |
| Forward secrecy on removal | A removal is a real MLS Remove commit → epoch advance + routing-secret rotation; the removed member is genuinely cut off | `catcoms-sync` removal path |
| Blob integrity | Content-addressed; served bytes are re-hashed against the requested CID before storing (no cache poisoning) | `catcoms-sync::request_blob` |
| File-at-rest encryption | Per-group file-wrap key; sealed at rest under the vault key | `catcoms-storage` (Phase 9h) |
| UI continuity and backup confidentiality | Drafts/read positions are vault-sealed and bounded; sealed records use destination-specific create-new siblings, file sync, rename and parent-directory sync on Unix and Windows, so concurrent record types cannot alias, pre-planted staging symlinks are rejected, and abrupt termination exposes a complete predecessor or replacement rather than a partial record. A failed post-rename directory flush is disclosed as committed-but-not-durable. Offline backup copies only the already-sealed vault tree without following links. Export creates another offline guessing target and exposes filesystem metadata; it does not weaken record encryption. Catastrophic filesystem/hardware failure and unsupported targets remain outside the directory-durability guarantee. | `catcoms-app::ServerStore`; desktop `create_backup` |
| Vault creation and secret rotation | A non-blocking OS-backed sibling lock serializes first creation and rewrap across processes; contention returns `VaultBusy` for retry instead of hanging behind a suspended process. The current wrapper is authenticated; the same root DEK is published through a unique create-new, file-synced staging sibling with rename and Unix directory sync, so concurrent app instances cannot return mismatched first-run DEKs or both report a conflicting rewrap. New/replacement secrets are capped at 4096 bytes. A v1 wrapper with a legacy 4097..65536-byte secret receives one bounded compatibility open and is atomically migrated to fixed-input v2; larger inputs are rejected. This intentionally loses downgrade compatibility with v1-only builds, not ciphertext confidentiality. Wrapper reads accept exactly 89 bytes and never allocate from a hostile file length. | `catcoms-storage::{open_or_create_vault,change_vault_passphrase}`; desktop `change_vault_secret` |
| Vault single-writer lifetime | `ServerStore::open` acquires a separate non-blocking OS session lock before unsealing and retains it until drop/process exit. A second desktop cannot start duplicate MLS, registry, invite-ledger or transport writers from the same snapshot; it receives `VaultBusy`. Normal exit and abort release the OS lock. Explicit UI lock keeps the native mount but closes IPC; re-unlock performs verify-only authentication against `vault.bin`, and a wrong secret cannot reopen the session. This is same-host installation exclusion, not distributed consensus or protection from malware with the user's OS authority. | `catcoms-storage::{acquire_vault_session,verify_vault_passphrase}`; `catcoms-app::ServerStore`; desktop `unlock` |
| Desktop explicit-lock IPC boundary | Every non-bootstrap Tauri command requires both a mounted vault and an open UI session. Lock atomically saves bounded continuity state then closes the command boundary; actor events are dropped, late frontend/native cache publications are exact-generation gated, and plaintext export publication holds the exact-generation commit guard while actors continue native background network/persistence work. Native window-close attempts serialize, and a duplicate caller cannot overtake an unacknowledged continuity failure to destroy the window. | desktop `require_unlocked_session`; `require_ui_session_generation`; `lock_session`; `close_vault_window`; `forward_events` |
| Moderation-record attribution and field integrity | Each event/vote has a canonical group-bound Ed25519 signature; the reader verifies signer fingerprint and linked-device origin, so records cannot be altered or replayed into another server without detection | `catcoms-app::moderation` |
| Path traversal | Virtual file paths normalized (drops `.`/`..`/empty) so a path can't escape the share | `catcoms-app::normalize_path` |

## Honest-client / product-layer only (a modified client CAN bypass); the residual backlog

| # | Action | Honest-client gate | What a modified member could do | Severity | Status |
|---|---|---|---|---|---|
| ~~R1~~ | **Member-removal requests** |; | ~~The committer honored a removal request from any member without a role check.~~ **CLOSED:** removal is now owner-only at the protocol layer; `request_remove` rejects a non-owner, and the committer ignores any inbound remove request whose requester isn't the owner (verified by signature, so a forged owner-claim fails too). |; | **Closed**; `crates/catcoms-sync` (on_remove_request owner gate + request_remove Unauthorized) |
| R2 | **File deletion** | `Server::delete_file` gates on owner/admin role | A modified member could post a raw `FileIndex` delete op directly, unlisting any file. Low stakes; the content-addressed blob survives on every peer that holds it; nothing is destroyed. | **Low** | **Open**; close with the same committer-side role re-check, or accept (lowest stakes) |
| R3 | **Invite-minting permission** | `require_invite_permission` → `can_invite()` (Owner/Admin) gates `mint_invite` | A modified member can *mint* an invite token, **but it is useless against an updated owner**: admission is both rank-gated and role-gated (see the protocol table), so a non-committer cannot admit the joiner itself, and the owner rejects its relayed Add-request because the named inviter is not on the owner's local roster. | **None today** (single-committer) / **Medium** if multi-committer were enabled, where a second committer reads the shared published roster rather than the owner's local one (see item 3's residual) | **Closed at the protocol layer** for the shipped single-committer config; the committer-side inviter role re-check (`inviter_is_authorized`) is shipped and tested, so minting permission is no longer the load-bearing gate. Do not enable `max_committer_rank ≥ 1` |
| R4 | **Local role display** | `my_role()` drives which controls the UI shows | A modified client can paint itself as "admin/owner" **in its own UI**, but this grants **no real capability**; grants are owner-signed (unforgeable) and admission is rank-gated. Cosmetic only. | **Cosmetic** | Accepted; documented in-app |
| R5 | **Invite rate-limit / server policy** (planned) | An owner-set server-settings doc, respected by honest clients | A modified admin can ignore a mint rate-limit / expiry policy. It is a guardrail against *accidental* over-sharing by honest admins, **not** a control against a malicious admin. | **Soft guardrail** | Document the limitation in the UI when shipped |
| R6 | **Message edit / delete / react / pin** | `edit_message`/`delete_message` gate on `author == self` (own messages; delete also allows owner/admin moderation); `set_pin` gates on the owner/admin role; `toggle_reaction` keys the reaction by the caller's own fingerprint | A modified member could post a raw channel op editing/deleting/pinning **any** member's message, or forging a reaction under another member's fingerprint; the per-op inner signature signs the *delta*, not the semantic `author`/reactor/role (the same property that already lets a member forge a message's author on send). Low stakes; message content is not authenticated, by design. | **Low** | **Open**; accept (same posture as R2), or a later per-message author-binding hardening |
| R7 | **Moderation-log semantic authorization and completeness** | Product APIs gate warning/case creation to current owner/admin, resolution to owner, bind evidence to a currently visible message and same-target warning, and readers ignore invalid/unattributed/currently unauthorized records | A modified member can still submit raw Automerge changes that delete/overwrite moderation keys. A modified current admin can sign an invented evidence snapshot because the original message is not author-signed. Signatures make alteration and attribution failures detectable, but do **not** make the CRDT an append-only audit log or prove the signer's role at the historical instant. Votes never authorize removal; owner-only MLS removal remains enforced. | **Medium** (accountability), **None** for removal authority | **Open and disclosed**; historical role certificates plus a countersigned/hash-chained append-only log are the hardening path |
| R8 | **Flipnote frame claims, Ask and Pass** (Gate 5) | The honest Studio claims on paint, releases on navigation, re-checks the call-server scope and completes a Pass only after an explicit release | A modified call member can claim any frame, never refresh or release a claim, spam Asks, or ignore Pass. Claims are advisory by design: they never lock a frame or gate a save, expire 90 s after their last receipt on the receiver's clock, and are bounded per sender (4 live, 2/s) and per identity per call. | **Low** (coordination only) | **Accepted by design**; the UI says claims never lock a frame |

### Notes on the roles/committer entanglement and how "admin invites" were resolved

Reading the admission code surfaced a structural fact worth recording, because it shaped the
design that shipped:

- **Only the owner ever runs the MLS Add**, and that is deliberate: a second committer is a
  fork. So making admin invites work could not mean promoting admins to committers; it meant
  **connecting the roles model (Owner/Admin) to the admission model** without adding committers.
- That connection is the **committer-side inviter role re-check ("option b")**, and it is now
  **shipped**. An admin who mints an invite no longer strands the joiner: when the joiner
  connects to that admin, `serve_join_inner` sees the admin is above `max_committer_rank`,
  so instead of admitting it broadcasts a **signed Add-request** on the control topic and tells
  the joiner to wait for the pushed Welcome. The owner's `on_add_request` verifies the request
  signature, then re-checks that the requester **is** the invite's named inviter and that that
  inviter is owner/admin **right now** (`inviter_is_authorized`, read from the owner's local
  authoritative roster), and only then commits the Add. "Admins can invite" is therefore a
  working end-to-end path, not a UI affordance: `canInvite` in `App.svelte` is not gated off.
- The admin's own pre-flight check on that path is a **liveness courtesy, not a security gate**:
  it is reject-only against a *positively read* published roster, so a member who overwrites the
  published-roster scalar with junk cannot disable other admins' relays (`read_published_roster`
  returns `None` on anything it cannot verify, and `None` means relay anyway), and it can never
  admit anyone. The owner's local-roster check is the authoritative one. The case that does bite
  is a **stale but validly owner-signed** roster replayed into the CRDT: it verifies cleanly, is
  read as a positive omission, and suppresses a newly-promoted admin's relay until the owner
  republishes. That is liveness only, for the same reason: admission reads the owner's local
  `admin_roster`. It is the missing per-reader high-water named in item 3's residual below.

## Hardening backlog (the fixes)

1. **Committer-side requester/inviter role re-check (option "b")**; ✅ **DONE** for both removal
   (owner-only, see R1) and invite admission (R3). Before the owner commits an Add on behalf of
   an invite, `on_add_request` verifies the request signature, that the requester **is** the
   invite's named `inviter_device_id`, that the invite self-authenticates and targets this group,
   and that the inviter is authorized **right now** via `inviter_is_authorized`. It needed no
   invite-format change: the inviter's identity was already recoverable from the invite. Tested by
   `the_owner_admits_a_valid_admin_add_request`, `a_non_admin_add_request_is_rejected_by_the_owner`
   and `an_admin_relays_the_owner_admit_result_so_the_joiner_accepts`. Under `max_committer_rank ≥ 1`
   a second committer would have to re-check against the *published* roster instead of the owner's
   local one, which reopens the replay surface in item 3's residual; do not enable it.
2. **File-delete protocol gate (R2)**; optional; same re-check applied to `FileIndex` deletes,
   or accept the residual (lowest stakes).
3. **Replay-proof grant revocation**; ✅ **DONE + REVIEWED** (`design-grant-revocation.md`).
   Rather than a CRDT epoch/nonce, the authoritative admin set is kept **owner-local** (a
   persisted `admin_roster`); since only the owner admits (Option C), the admission gate
   (`inviter_is_authorized`) reads that local set, which a malicious member cannot write; so a
   demoted admin replaying or deleting its grant in the shared CRDT can no longer re-authorize
   itself. The CRDT now carries a single **owner-signed** `roster` value, which readers verify
   against the owner's key. It is **not** display-only: it is a reject-only liveness hint, read by
   a relaying admin's pre-flight (`published_roster_omits`) as well as by role display. A tampered
   or deleted copy is cosmetic, because verification is fail-open (`None` → relay anyway). A
   *stale-but-validly-signed* copy is not: it reads cleanly and denies a newly-promoted admin's
   relay until the owner republishes. Adversarial review: no blocking/should-fix findings.
   **Residual (one missing high-water, two consequences):** there is no per-reader high-water on
   the signed published roster. Today that costs **liveness only** (the stale-replay relay denial
   above), because admission reads the owner's local set. Under `max_committer_rank ≥ 1` a second
   committer would re-check against this published copy instead, and the same replay becomes an
   **admission** bypass. Do not enable concurrent committers. This **closes the GA gate** for admin invites (item 4).
4. **Make admin invites functional (R3)**; ✅ **DONE + REVIEWED; live end to end.** Design:
   Option C, "owner-serialized admin invites." An admin who wants to invite
   broadcasts a *signed Add-request* on the control topic (mirroring the R1 remove-request
   pattern); the **owner alone** runs the MLS Add after re-checking the inviter is Owner/Admin per
   the **owner's own local `admin_roster`**. This keeps `max_committer_rank = 0` (single committer
   → **no fork**), reuses the most-tested membership code + the existing two-phase Welcome-push.
   Shipped slices:
   (0) roster/grant logic moved into `catcoms-sync` (`roles::read_published_roster` for the
   display copy; the gate itself reads the owner's local roster, so a member cannot write what
   the gate consults - a local roster is not a zero-staleness read of the shared doc, it is a set
   only the owner edits) ✓;
   (1) the inviter-role re-check at admission ✓; (2) the `CTRL_ADD_REQUEST` op + `on_add_request`
   + the **Welcome-authentication chain** ✓; the load-bearing new crypto passed a focused
   adversarial review with **no blocking findings** (verified: no non-owner commit path; the
   no-substitution property holds; the admin re-signs the *identical* transcript only after
   verifying the owner's signature over it, and the joiner's `group_id` pin + MLS KeyPackage bind
   reject any substituted group; exactly-once admission across the hop). (3) actor/desktop wiring
   ✓: the Add-request queue (`add_request_queue`), the retransmit/expiry driver
   (`drive_outgoing_add_requests` over `outgoing_add_requests`) and the owner-side drain
   (`drain_add_request_queue`) all run on the normal tick, and the UI is **not** gated:
   `canInvite = myRole === "owner" || myRole === "admin"` in `App.svelte` shows the invite
   controls to admins.
   - *Rejected:* **Option A** (admins are committers / `max_committer_rank ≥ 1`); forces
     concurrent committers and the staged fork-resolution path's **I1 is still open**, so two
     admins admitting at once can permanently split the group. **Option B** (owner-admits-on-
     behalf via point-to-point forward); routing is a blocker without a new forward protocol +
     a rewritten Welcome trust anchor.
   - *Chosen variant: Option C + **offline Add-request queuing**.* The admin's signed
     Add-request sits on the control topic until the owner is next online; the joiner waits and
     is admitted when the owner finalizes. So an admin can create + hand out invites **with the
     owner offline** (admin-independent), and admission completes whenever the owner next syncs.
   - *Product implication:* admission finalizes **when the owner is next online**; admins are
     independent for minting/handing out invites, but the final MLS Add is owner-serialized so
     the group can't fork. Not a regression (today only the owner can admit at all). Owner-
     **never**-online concurrent admission is explicitly **out of scope**: it needs safe
     concurrent committers, i.e. closing **I1**, which is fundamental consensus (FLP/CAP); a
     multi-week-to-month redesign whose failure mode is a *permanent group split* (forward-
     secrecy/PCS defeated on the losing branch). Recommended against; do **not** enable
     `max_committer_rank ≥ 1`.
   - *Residual:* demotion **is** replay-proof in the shipped single-committer config: the owner's
     admission check reads its **local** `admin_roster` (item 3), which no member can write, so a
     demoted admin replaying or deleting its grant op in the shared CRDT cannot re-authorize
     itself. What remains is item 3's own residual, not a weaker demotion story: under
     `max_committer_rank ≥ 1` a second committer would have to fall back to the published roster
     and would need a per-reader high-water mark to stay replay-proof. Do not enable concurrent
     committers. Separately, admission still finalizes only when the owner is next online.
5. **Invite rate-limit as server policy (R5)**; owner-set, honest-client-enforced; ship with
   the limitation stated in the UI.
6. **Moderation-log completeness (R7)**; design a monotonic, hash-linked event log with historical
   owner-signed role evidence and a protocol-side append/delete policy before calling the timeline
   a tamper-proof audit trail. Preserve the existing invariant that a tally is never executable
   authority and only the owner removal path can mutate MLS membership.
7. **Fingerprint-keyed inviter authorization (FIXED on the separate security branch).**
   Unpatched Gate 5 owners authorize four-byte collision twins that are actual members.
   Updated owners use full per-device grants and final fresh-Add checks, with v2 signed
   rosters and explicit versioned authority persistence. Legacy grants are discarded,
   never matched to current members. Owner re-approval is required. See the security
   amendment above for impact, compatibility and the remaining relay liveness limitation.

## How to use this document

- When you add a governance/policy action, add a row to the right table and state its
  enforcement layer honestly.
- When the UI implies a rule is enforced, make sure the rule is either in the protocol table or
  carries an in-app note that it is advisory (as the roles panel already does).
- When a residual is closed, move it up with the commit hash.
- When a Tauri command is added or removed, update `apps/desktop/src/tauri-command-security.ts`;
  the frontend suite checks the ledger against the native handler and every literal invocation.

### Detached file reads and explicit kept copies

Gate 6's core score reader checks byte/primitive rails before materializing conflicts and uses
one complete causal frontier for headers, recipes, keys, alternatives and winners. Mutable
coordinates/hashes are not permanent frame tombstones: live removals dominate raw Set winners,
while later exact same-property edits can reuse observed cleared slots. Omitted removals and
conflicts require recovery-before-replacement. Explicit family tags prevent pristine score
restart data from becoming art; shared title/recipe body shapes never identify authority.
Typed reference inspection validates complete score envelopes before returning no CIDs. The
legacy operation inspector selects a static grammar union and fully validates Score-specific
cells/clears/musical headers before returning no CIDs; it grants no authority. Typed art inspection
still rejects Score bodies rather than inferring a document family from common body shapes.
The core codec itself grants no native/wire capability. The candidate connects its separate
typed target to existing native/wire custody; historical Score replay stays manual because
final absence cannot prove no intervening reusable-slot clear.

Network workers own only an opaque signed blob request. Completion rechecks the exact sync owner,
current group/epoch/local membership, expected provider, request nonce and response signature/CID;
the application rechecks its current manifest-set identity and cancellation before storage. Each
attempt owns independent server/process permits until actual transport retirement, even after an
application timeout. Connected-only admission cannot redial from cached routes. Responses and ranges
are bounded, and an abandoned worker explicitly signals cancellation before queued driver admission.

Explicit kept copies additionally reserve an exact full manifest before fetching, authenticate/open
bytes before writing directly into a separate sealed quota domain, and verify the ordered whole-file
CID before durable commit. Partial records remain charged until bounded cleanup succeeds. Exclusive
OS leases prevent two mounts from sweeping active work or forking quota accounting. Ordinary cache
GC and unsigned shared expiry cannot destroy local kept ownership. Startup resets verified evidence;
explicit Check and repair remains separate from media authorization and may use its saved exact
manifest after unlisting. UI lock cancels native leases and prevents stale inventory/results from
crossing the original session/actor-instance boundary. Remote copies remain unconfirmed.

New V2 retained journals have a distinct HKDF metadata key, separate from ordinary blob/chunk
sealing. The blob API accepts chosen plaintext; using its key for native intent would let an offline
attacker copy a legitimately sealed chosen record into the transfer namespace. The dedicated key,
full-group/DeviceId binding and canonical namespace prevent that substitution and quota aliases.
An exact successor journal, conservative two-slot reservation and absorbing Deleting state prevent
crash reconciliation from restoring an older grant or released copy. Legacy pending remains
cleanup-only. Whole-vault rollback by an attacker controlling all persisted state is not prevented.

Persistent Keep allowance is spent before dispatch; unused prepaid batches/slices are lost across
restart. Exhaustion requires an explicit idempotent native Retry CAS. Immutable runtime deadlines
and run revocation reach the transport admission seam, so delayed polling or same-CID replacement
cannot renew queued work. Replies still require the original current actor, manifest and membership
checks before storage. Directory barriers are exercised on this Windows desktop; injected failures
and process reopen do not establish physical power-loss durability. Automatic partial Keep resume
is scoped to a saved explicit intent and the exact unlocked native generation; lock retires old
lower admission even before asynchronous cleanup. One absolute 30-second wake rotates candidates;
providerless partial work does not spend slices or repeatedly hash a prefix. Persistent Pause and
integrity stops require explicit Resume, and exhaustion requires its separate Retry grant. V2
manual-only records remain effectively paused until explicit V3 migration; canonical old bytes
and valid old pending successors are preserved. Separate intent CAS fences stale Pause/Resume
without treating progress or budget grants as user order. Completed checks keep runtime stop
controls. V4 whole-variant fallback owns its immutable original compatible set, private progress
and separate committed winner. App admission/resume validates every saved plan, equivalence and
set digest before work. Selection retires the old run before journal publication, restarts the
whole plaintext hash and preserves all global budgets/intent. Actual lower permits retain their
owners independently. Bad verdicts require failure to open exact authenticated ciphertext; missing
keys refuse before admission and provider absence never excludes a candidate. Winner-only public
service prevents partial alternative leakage. V4's own 400 KiB slots and 512-leaf rail do not
reinterpret V2/V3 charges or enrich old saved plans. Indirect blob service remains unimplemented;
focused V4 integration verification and review are in progress.

These controls do not solve arbitrary inherited Automerge history allocation, Byzantine conflicting
index claims, permanent remote availability, continuous local disk health, or all legacy actor waits.
See [the detailed limits and follow-ups](design-file-reliability.md).


### Owned file-source read and deferred reply boundaries (19 October)

Bounded native read preparation captures only exact public storage sources and weak lifetime/
committed-winner fences. It opens no disk file on the sole sync actor. Explicit limits cover path
capacity, closed error metadata, snapshot capacity, source/parent counts and at-rest input/output;
ordinary allocation/OS overhead is not a process-wide cap. Future service must prepay scratch before
preparation, move that exact owner into actual blocking work and retain it with the result through
actor revalidation/page construction. A timeout cannot cancel disk I/O or refund a still-live owner.
A sealed worker retains a zeroizing native at-rest key; no MLS/device/file-wrap key or store borrow
is needed. Staging/private candidates cannot be served. Store/winner/scope retirement suppresses
stale results, but only a caller's fresh group/request checks can authorize a network signature.
No bridge activation or aggregate worker bound follows from this primitive alone.

Deferred reply authority now revokes synchronously on receiver Drop and follows original ingress.
Actual lower-driver admission can check that same owner, avoiding an asynchronous cancellation
propagation gap. This lifetime ends on response receive/transfer, not actual ResponseSent. Driver
send authority and queued/lower/response accounting remain independent; cancellation grants no
refund and exported lifetime grants no membership. Existing notification handling is unchanged.


The native source executor now enforces actual task/result custody with 40 MiB explicit charges,
one per full bounded group and two process-wide. Its pool survives runtime/executor replacement.
The blocking closure retains the whole affine task across observer cancellation; result, error,
absence and panic unwind release only actual ownership. Results cannot be extracted without their
credit. This closes the worker-custody prerequisite, not service authorization. Root/request/
native-session revalidation, bounded source/helper admission and outgoing page response custody
remain required before activation. An independent transport frame limit is also necessary:
application validation after a legacy-sized response allocation cannot justify a small page charge.


Reserved file-service kinds 29/30 now have transport-enforced 2 KiB request and 72 KiB complete
response frames. The codec inspects length and kind before new-request body allocation, and
remembers the class on the actual request/response substream so even a malicious responder cannot
force a legacy 16 MiB allocation. Limits are value-only per clone and reset for legacy/empty
requests. The pinned libp2p handler must continue pairing write-request/read-response or
read-request/write-response on that same clone. A real TCP hostile writer and concurrent legacy
requests pin this dependency. This enables no helper/source handler or new membership authority.


### Inert connected file-service admission (19 October)

The process-stable policy accepts native scope observations only after the caller authenticates
the actual immediate Noise sender. Precrypto limits charge that sender; a claimed original C
cannot spend C's work budget without a valid original signature. A source separately verifies
helper B and C and must still match both to current native membership. Immutable exact-frame
tickets do not supply current standing or a connection for forwarded C. Work admission debits
C, B and aggregate windows atomically; protective rows refuse capacity instead of evicting live
identities. Root replacement, epoch changes and reconnect cannot reset process rows.

Freshness is strictly below 20 seconds in either direction; replay rows last 40 monotonic seconds.
Serialized clock sampling advances the wall high-water mark with monotonic time, so rollback
cannot reopen an accepted grant at row expiry. Overflow/backward monotonic time and poisoned
admission state fail closed. Large clock corrections can refuse otherwise healthy service.
Process restart loses this service replay history; it is not durable exactly-once authorization
and cannot refill the origin's separately persistent Keep allowance. No handler is active yet.
Native entry/Root/membership/connection revocation, source hop deadlines, capacity custody and
final response checks remain required integration boundaries.


Optional final response authority now closes the queued-reply effect gap: the handler checks it
before budgeting/queueing, the receiver checks it, and the real driver checks it together with
original ingress immediately before copying/submitting. The last synchronous check defines
admission; later retirement cannot retract the send. It must observe native/group/deadline scope
without retaining its publisher or keys, and must exclude the already-retired reply-wait lifetime.
Existing byte clones and actual lower custody remain charged after revocation. This seam alone
activates no file handler or current membership authorization.


Existing direct file workers now observe the sole actor generation and original Root/member/
policy scope. This closes queued lower admission after observed MLS removal and after a stop
acknowledgement while the actor is retained by a full close-event sink. Revocable effects are
independent of actual lower credit custody. Reversible freeze and failed stop acknowledgement
create new actor generations; old local jobs, retries and results retain the retired observer.
Legacy direct provider permission remains unchanged. This Root scope does not cover strict
bridge descriptor/proof/connection identity, native entry ownership, or a new service dispatch.


Requested native reads additionally retain the original account generation and exact registry
row through actor queueing, local and network decryption, multi-chunk media/download/save, and
final plaintext publication. Generation capture precedes the first authorization await; a command
waiting through lock/unlock cannot adopt the new generation. Post-AEAD storage admission and
post-inspection/reveal checks observe the original authority synchronously. An already-admitted
synchronous storage/OS action cannot be recalled; verified files already published are retained
on subsequent refusal. Whole jobs (including peer-busy sleepers and staged range/search state)
are separately bounded4/actor and8/process, not just at actual lower request admission.

## Native action handoff custody (22 October continuation)

The native factory now accepts only an actual retained channel Route and derives selection
from that row. It repeats the row's open-state and original generation/selection checks under
the SAME UI commit used by trusted channel collection. A close which wins before admission
prevents work; admitted detached actor work retains that commit after caller loss. Request
labels, a valid channel MAC, RowId or historical receipt never replace current effect authority.

An opaque authenticated header may select only its original bounded result Phase before
body intake. The header is parsed/authenticated once and remains with its original physical
worker. After AEAD, the future dispatcher must resolve the SAME RowId and complete decrypted
purpose using the Route's selected owner before any operation. That dispatcher is not enabled.
Closed/missing Phase capacity never means native or browser work drained; historical control
recovery has a separate bounded path.

The trusted native result bridge pins the actual paid buffer without a byte copy. Slice-owner
callbacks run once before admission, outside protocol/native locks. One private actor Flight
per body prevents duplicate envelopes and uncollected outputs from sharing one fixed payment.
Actual queued commands/completions retain the original body after caller cancellation. Root's
grant/action/tag checks remain authoritative. This generic Rust constructor is for trusted
prepaid owners; its inline-size cap does not bound arbitrary nested heaps. It is not a renderer
API, authentication primitive, total heap/RSS bound or zeroization guarantee.

Plan closure and actual last-owner release remain distinct. Explicit original-loan retirement
cannot adopt a replacement Workflow or declare held inputs/lower work drained. Browser v10
shares actual channel/provider execution, retains each action's exact adapter, and keeps cleanup
transport until linked SDK/RPC work settles. Framed action/final ACK delivery, safe historical
collection, normal ingress, SDP/RTC, relay and independent physical acceptance remain open.

Process exit now closes new native physical admission and waits for original header/reply and
detached factory ownership before reporting service completion. Shutdown waits outside UIcommit
and native guards, while actors remain alive to complete admitted turns. A retired Session's
weak witness prevents socket loss or Row detachment from hiding live physical owners. This
barrier does not acknowledge browser SDK work, retire uncertain purpose history, or establish
full Workflow drainage; any future result worker must retain the same physical hold until its
actual Work/body/completion owners are collected.

The new private action plaintext grammar is not enabled at ingress. Its fixed selector/action
fields and strict bounds are parsing checks, not capabilities. A valid channel MAC still requires
the same original selected owner, header RowId and complete decrypted purpose/action at every
effect boundary. Receipt ACK identity excludes observational retirement; End cannot invalidate
an earlier exact transition ACK. Conversely, unknown/stale FinalACK cannot establish drainage,
and FinalACK cannot justify deleting purpose tombstones while that Session may still admit Begin.

The inactive derivation executor additionally requires pointer identity of the actual request
Session and Route. A valid MAC from another Session or a changed authenticated header RowId
cannot select the original action. Empty output/hash and the full native action tuple are
checked before Work. Historical recovery still owns a previously admitted Data body, but takes
no additional Phase loan/task or current grant. New result publication holds the original UIcommit
through the actual actor turn after caller loss. Structured drop order keeps Work/task retirement
ahead of physical idle; actor and Root remain responsible for current member/epoch/pair checks.
Synthetic-pair rejection tests do not establish successful production ingress or browser keys.
