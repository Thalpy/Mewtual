# Original join activation, grants and accepted reply (design and remaining gates)

The first connected implementation now supports only durable initial grants/debits and retained
signed INPUT DATA under an explicit private physical-open policy. See
[execution implementation boundary](CHAT-DESTINATION-JOIN-EXECUTION-SCHEMA.md).
The broader accepted-reply/SDK/retry-grant/Intent/Current design below remains incomplete;
none of those capabilities follows from storing signed DATA. Finite direct OwnerFact-v1 root
kind1 is distinct from production public-owner-v2 joining. Compilation/tests/build remain paused.

## Durable allowance is independent from activation

A new activation lease MUST NOT reset execution allowance. Extend the critical JOIN journal with
an explicit schema version that old Pending-only readers reject BEFORE any cleaner. Keep the
original source identity, ciphertext, root kind, selected package/request and admission basis.
Every network submission consumes one durably saved unit from that same source/grant BEFORE the
transport call. Initial grant comes from trusted local policy when the real source is admitted;
saved counters never mint a resource fee or a new grant. A new mount/task/transport, lost reply,
pause/resume or epoch change cannot replenish it. Exhaustion preserves the keys and allows local
reply/save recovery; it cannot silently create a new package or evict the source. Explicit retry
grants have checked monotonic IDs and persist before use, with the same immutable source binding.

The initial proposed bounded policy is at most64 submissions per grant, with no automatic new
grant on restart. Only an eligible original connected transport route may be armed, so an offline
owner does not consume allowance merely because a timer fires. Per-request timeout/backoff uses
the existing injected runtime seams. Current trusted policy may choose a lower limit, but the
counter ceiling and source execution state must survive all actual write/reopen boundaries.
Manual/new trusted grants remain explicit; no received status/parsed allowance can grant itself.

V1 sources had NO network API on configured mounts. A supported migration to the execution schema
may initialize their first grant only under actual complete-source/vault/catalog barriers and
that compiled V1 contract, never a decoded never-sent flag. Unknown/future versions refuse. The
same-DEK authenticated component rollback residual remains explicit; natural restart must not
reset counters. Join journal schema2 does NOT mean public-owner protocol2/root-kind2.

## Original activation owner

Store keeps a fixed per-source registration whose weak original task observation is bound to
physical mount, source identity, scheduling revision and grant ID. The actual typed task owns
the revoking lifetime; Drop closes it even if a temporary observer still holds its signal.
Runtime registration is not a stored Active bit and cannot keep a dropped worker running.
Only an actual authenticated mounted source capture plus file/root barrier and current original
row comparison may register it. Duplicate activation returns bounded in-progress/refuses a
second worker instead of constructing another provider/transport. No public data constructor
can supply the registration.

Pausing/regranting must close the original worker BEFORE the dependent durable transition can
complete. A failed/uncertain transition keeps old negative closure and blocks new activation
until that same candidate is resolved. Resuming later issues a NEW negative lease while keeping
the exact immutable source and preserved allowance; old queued callbacks cannot adopt it.
A same-mount dropped/cancelled activation cannot be renewed by rereading the same scheduling
revision. Cold explicit recovery starts a new physical-mount lease for the SAME source/counters;
the old mount remains cancelled. Unrelated rows/catalog generations do not retarget this lease.

Negative-only pause must remain possible if the former admission basis is now retired/changed:
it may preserve and stop that exact source but may not finish a join, replace keys, increase a
grant or mint a new capture. Its concrete original mounted-source control needs a separate
negative consumer rather than weakening the positive complete-row admission CAS.

## Synchronous arm, then await without Store/key ownership

Bind ONE actual transport/event stream to the activated source; keep PreOwnerConnectionHandoff
outside each future. Do not seed it from a PeerId/snapshot or exchange the transport on retry.
Cold recovery may bind a new transport with an EMPTY event prefix. The current pre-owner
exchange consumes its transport event stream. It must receive an exclusive/scoped stream or a
real Session dispatcher integration; passing a shared multi-group stream and dropping other
traffic is forbidden. Do not claim native composition until that consumer is connected.
An original supported
connection lease must belong to that exact transport/target before an automatic request is armed.

Arm synchronously with the actual Store. First reserve/preflight local source/response/route
work; then final original activation/admission -> catalog guards persist the next debit and
cross real barriers. Return a non-Clone original submission ticket only after that durable result.
No Store, vault key/session owner or writer is retained across the network await. The typed bound
attempt keeps only the original weak/negative observations, exact source, actual fees, transport
and ticket. Before invoking the transport (even immediate error/cancel) it consumes that ticket
locally. A new request requires a new durable debit; a new activation cannot reset the allowance.

A proven unstarted local preparation may keep its original ticket/deadline for warm retry rather
than spending another unit. There is NO general refund after ambiguous transport handoff. A crash
at the disk/network boundary cannot promise exactly-once submission; conservative persistent
debits are intentional, while exact owner result lookup makes duplicate request bytes harmless.
Each genuine arm has an opaque original invocation identity retained by its bound owner BEFORE
any debit. The persisted record includes checked per-source submission ordinal/arm ID, source/
request/binding, grant ID, exact old/new allowance and the exact durable outcome. Retrying that
SAME retained invocation looks up its result BEFORE considering another debit. A duplicate arm
call must not create a second live ticket. Before returning even a lost/injected-error result,
Store/bound owner retains the original unstarted ticket/result; exact warm lookup transfers it
once. The local first-poll handoff consumes it irreversibly. A new attempt requires an explicit
new invocation and another debit from the preserved grant; ordinary retry cannot resample one.

Cold restart treats every possibly consumed arm as SPENT, even when no network reply survived.
It never decodes unstarted/unused into repeatable authority. Only the genuine retained warm
unstarted ticket may retry with its original deadline/connection; ambiguous transport handoff
gets no refund. Late arm results after pause/regrant/new activation cannot attach to the new
lease. A disk/network crash boundary may conservatively spend a unit without submission; it
cannot reset the source allowance or replay one ordinal forever. Lost arm replies recover the
same debit/candidate/result, never another initial grant. Tests must cover lost return,
duplicate invocation, consumed-ticket crash/reopen loops and late original results.

## Cross-crate boundary, not a generic recovered decoder

`RecoveredDirectJoinSource` must continue exposing NO into-Prepared or standalone finish API.
The actual App mounted-source/submission implementation is the only authorized production bridge.
Because Sync cannot name App-private disk events without a dependency cycle, a narrowly scoped
trusted-host grant trait comparable to existing ManagedChatScopeReservation may be needed. It
must have NO default implementation, retain the actual original App capture/ticket, and check
exact source/request/package/root identity plus original negative before and after await.
Arbitrary Rust implementations are trusted code, NOT cryptographic proof of a disk event; do not
claim type-level unforgeability this boundary cannot enforce. No IPC/wire/serde implementation
or raw alive/paid/accepted bool may construct that host object. Review the exact interface before
adding it. No public handler trait/callback may declare a catalog namespace supported.

The actual operation uses `request_connected_cancellable`, NEVER plain `request()`. Its original
RequestCancellation composes source/task authority, the captured exact T/target connection
incarnation and prepaid lower-work keepalive; driver admission repeats that authority and no
redial is allowed. Existing source cancellation/accounting must be composed, not replaced. Routing eligibility governs the
submission edge. Once the complete reply has authenticated and actually selected the original
KP, later connection loss does NOT invalidate that result. Retain it first; positive handoff
rechecks original SOURCE/activation/arm identity, not temporary connection parity. A late valid
receipt remains DATA if the source itself was paused/closed, without reviving its graph.
Transport eligibility must explicitly include support for that actual connected+cancellable
seam before debit; connection-lease support alone does not prove a wrapper forwards it. A narrow
explicit RT capability (default false, actual MeshService/MemNetwork true, adapters forward) may
be necessary; it is trusted implementation metadata, never remote membership proof. An unsupported
adapter must refuse before spending source allowance, not exhaust attempts locally.

This finite direct checkpoint still trusts the host's inviter endpoint selection. A live peer
alone is NOT authenticated as the invite signer, and verifying the eventual reply signature does
not undo leaking a bearer invite to a wrong peer. The new implementation must either capture the
actual trusted endpoint mapping before activation or keep that choice explicitly within its
trusted Rust caller contract. It must not forward this raw legacy request via arbitrary members.
Production root-kind2/indirect joining needs its actual authenticated endpoint/sealed-helper path;
this client bridge cannot imply that absent protocol is implemented.

The bridge owns source plus original grant for the whole operation. It processes each reply on
a private candidate and may return ONLY a new private durable-result wrapper, never the old
unjournalled OwnerJoinedInitialization or bare runnable Server. After final original checks,
transport and ordered handoff move once without await into that hidden result. Errors/cancel
retain the exact master/request and original transport/lifecycle prefix, as the current bound
in-process path does. A revoked original may not move its successful candidate into a new lease.

## Proposed concrete API packet (requires agreement before implementation)

App-private actual types would be `JoinActivationOwner`, `JoinArmInvocation`,
`OriginalJoinSubmissionTicket`, `RetainedJoinExchange` and `SavedCheckpointJoinReply`.
None has a wire/serde/caller-field constructor. Source/request identity and grant/counter data
stay separate from physical activation and native generation observations. Store registers a
prepaid `Arc` to the concrete non-T retained exchange cell; the typed bound owner separately owns
one T plus its original PreOwner event prefix. There is no Store, vault key or vault-session owner inside that cell; member private keys
remain intentionally private and charged.

The intended trusted-process entry sequence is:

```
Store::activate_journalled_checkpoint_source(preparation, trusted_endpoint, transport)
    -> ActiveCheckpointJoin<T>
ActiveCheckpointJoin::ensure_submission_armed(&Store, rng) -> Result<(), AppError>
ActiveCheckpointJoin::exchange_armed(&mut self).await -> Result<JoinExchangeProgress, AppError>
ActiveCheckpointJoin::persist_held_reply(&Store, rng) -> SavedCheckpointJoinReply
```

`ensure_submission_armed` returns no externally cloneable ticket: the exact invocation/ticket
stays in the same original bound exchange. Duplicate/lost-return calls recover that slot before
debit. After a consumed failed attempt, a distinct explicit next-invocation step is required;
it draws from the same persisted allowance. `exchange_armed` takes no Store and retains success
in the registered cell before returning a status. That status has no candidate/Server getter.
`persist_held_reply` retries only the cell's exact receipt write. Future handoff takes the opaque
saved-reply witness plus the SAME original held completion and moves transport/prefix once,
without await, into a NEW private durable-initialization type. It cannot return the existing
unjournalled OwnerJoinedInitialization or a bare Server before Intent/Frozen/terminal support.

For the finite trusted-host endpoint, prefer an opaque actual Saved/Current inviter endpoint
capture whose full device key matches the original invite signer. It must refresh routing only,
never replace the previously pinned Profile/founder. Merely taking any ready member's peer would
be wrong: that member may know the root but not be this invite's signer. If that actual capture
is unavailable, keep the raw endpoint within an explicitly trusted local caller contract and do
not label it authenticated prejoin routing or production indirect-join support. This choice is
a remaining interface decision, not permission for arbitrary member forwarding.

The narrow cross-crate submission interface under consideration is:

```
trait OriginalDirectJoinContext: Send + Sync {
    type Submission: OriginalDirectJoinSubmission;
    fn context_identity(&self) -> Arc<()>;
    fn source_binding(&self) -> &[u8; 32];
    fn request_identity(&self) -> &DirectJoinRequestIdentity;
    fn source_cancellation(&self) -> RequestCancellation;
    fn check_source_original(&self) -> Result<(), SyncError>;
}
trait OriginalDirectJoinSubmission: Send + Sync {
    fn context_identity(&self) -> Arc<()>;
    fn arm_ordinal(&self) -> u64;
    fn grant_id(&self) -> u64;
    fn source_binding(&self) -> &[u8; 32];
    fn request_identity(&self) -> &DirectJoinRequestIdentity;
    fn target(&self) -> PeerId;
    fn cancellation(&self) -> RequestCancellation;
    fn check_source_original(&self) -> Result<(), SyncError>;
}
```

It has NO default methods or decoded implementation. App's private ticket implementation must
carry the actual durable arm outcome/ordinal, original invocation, source/activation and paid
lower custody. Its cancellation is source/arm-only; the bound executor captures the actual T's
current connection lease at the once-only submission edge and composes that lease without
replacing the original signal, effect authority or accounting. Sync checks its opaque source's exact identity and same T/connection before
invoking connected+cancellable submission. After successful authentication, retain the result
before any source-negative refusal; check_source_original fences any positive handoff while
allowing a now-lost transport path to remain irrelevant to already authenticated DATA. The
actual API must keep the ticket single-use inside its retained exchange; a trait reference alone
must not permit a second request. Arbitrary Rust implementations remain trusted host code, not
cryptographic Store proof. No public `RecoveredDirectJoinSource::into_prepared` or free finish
method is introduced. Saved local reply replay needs a SEPARATE concrete original mounted-reply
capture (no outbound ticket/debit), with the same source/basis/root and negative checks.

These declarations are a review proposal only. The exact state/cell ownership and typed endpoint
must be agreed; do not copy this trait into runtime until its real App consumer exists.

### Selected single-use mechanism: Sync-owned retained exchange cell

Choose the shared Sync-owned opaque cell, not a promise on an observational trait reference.
The App registers one prepaid `Arc<RetainedDirectJoinExchange<ActualAppSourceLease>>` before
network work and retains it independently of the typed bound caller. Sync alone owns its phase:
`Idle`, `Armed(owned ticket)`, `Submitting(exact arm, flight)`, `Spent`, `HeldSuccess`, or
`Transferred`. The source/master remains a separate immutable field; no transition temporarily
removes it before a fallible call. The cell has no T, Store, vault key or vault-session owner. Its member private master/candidate
keys remain deliberately private and paid until persistence/closure.

Proposed core methods (private fields, no serde/Clone source or free Prepared export):

```
RetainedDirectJoinExchange::from_original_local(source: PreparedDirectJoin, context: H)
RetainedDirectJoinExchange::from_authenticated_source(source: RecoveredDirectJoinSource, context: H)
RetainedDirectJoinExchange::install_original_arm(ticket: H::Submission) -> Result<ArmStatus, ReturnedTicket<H::Submission>>
RetainedDirectJoinExchange::bind_original_transport(self: Arc<Self>, original_transport: T, original_target: PeerId) -> Result<BoundDirectJoinExchange<T, H>, ReturnedBinding<T, H>>
BoundDirectJoinExchange::exchange_armed(&mut self).await -> Result<JoinExchangeProgress, SyncError>
```

`H` is an actual trusted App source context with an associated non-Clone production ticket type,
fixed context identity, exact source/binding/request labels and bounded source-negative check.
Cell installation compares the SAME actual context identity (Arc pointer), never labels alone;
a ticket from a later activation cannot attach to the old cell or vice versa.
Submission also carries checked arm ordinal, grant ID and that SAME original context identity.
The constructors authenticate no disk event by themselves: only the real private App mounted
consumer is authorized to implement/issue H in production. Arbitrary Rust implementations are
trusted; they are not IPC or cryptographic proof. These exact trusted-host signatures still need
review before runtime declaration.

The cell binds ONCE to its actual T/target and creates its own empty PreOwner prefix in
`BoundDirectJoinExchange<T,H>`. The cell never offers exchange with per-call transport/prefix
arguments. Store sees only its retained cell/status, not a runnable second handle. A non-resetting
internal bind claim prevents a second bound owner even after Drop; failed pre-claim binding
returns the SAME T/cell. Build/check any fallible local fields before claiming. A dropped whole
bound owner closes its original activation; a different T requires a genuine new activation/cell
from the authenticated original source, never reuse by same PeerId or replacement prefix. Warm
future cancellation keeps the SAME bound T/prefix. Cold/new activation starts an EMPTY prefix.

Installation takes OWNERSHIP of the exact ticket. It checks source/context/ordinal before state
mutation, returns the SAME owned ticket on refusal, and never installs a second arm while Armed,
Submitting or HeldSuccess. Duplicate genuine invocation lookup returns the existing status,
not another ticket. A different source/context, old ordinal or foreign original must refuse.
Inside `exchange_armed`, a per-cell async mutex serializes inspection; concurrent/double calls
cannot race. Local capability/eligibility/resource refusal occurs while still Armed. Immediately
before the lower call Sync itself atomically changes Armed -> Submitting and consumes once.
That transition is not delegated to a trait getter or caller promise. A second call observes
Submitting/Spent/HeldSuccess and cannot reuse the ticket.

The connection incarnation is captured from the SAME owned T/target at that final submission
transition. An Armed ticket may wait for a route on that SAME bound transport under its unchanged
deadline; there was provably no request before Submitting, so a dead connection need not destroy
an unstarted ticket. Once Submitting, its exact captured connection/cancellation cannot retarget
a replacement. Source/arm/allowance remain fixed in both cases. A different T requires an actual
new activation; no PeerId-only identity guess or old event-prefix adoption is permitted.

The PRIVATE cell async-mutex guard remains held continuously across the network/SDK call AND
through the synchronous HeldSuccess assignment. There is NO release/reacquire await after SDK
success. This guard owns neither Store, vault key/session nor catalog/admission writer. Arm and
status paths use bounded try_lock/Busy; no Store lock waits on the cell. Source pause/Drop uses
atomic negative closure without acquiring it. Receipt encoding/Store persistence happens later,
after success is already retained. The before-assignment test seam may only be synchronous and
bounded: a cancellation request cannot preempt that poll and drop the success before assignment.
Add a deterministic contender/status/abort test proving Busy while that actual guard is held and
HeldSuccess survives once the poll resumes; no fallible user sink or unreserved slot allocation
is allowed after actual success.

A flat per-flight RAII signal closes nonblockingly if the submitting future drops/unwinds. It
never waits for the cell mutex. The next cell access reconciles that exact retired flight to
Spent and requires a NEW durable arm. There is no refund after this possible handoff. Local
cancellation before Submitting leaves the original Armed ticket intact. The async mutex has no
poisoned-success assumption; any unsupported partial phase/identity refuses and preserves source.
No source/drop path may restore Armed or reset the original context after Submitting.

On actual SDK success Sync stores the concrete candidate+opaque exact receipt+fees in HeldSuccess
INSIDE that same cell before returning, further await, fallible persistence or source-negative
handoff refusal. The cell is already retained by App, so dropping a caller future/handle cannot
drop the only success. An assignment must not call fallible user code or allocate an unreserved
slot. Save failure leaves HeldSuccess byte-identical and retry does no request/KP/SDK mutation.
The result has no raw getter. Transfer consumes it once only with the actual saved-reply/original
continuation consumer; paused/retired context leaves held DATA, not a fresh graph for a new lease.

Lower request cancellation carries a separate strong prepaid work owner through actual driver
completion, independently of flight/cell/caller Drop. It is composed with existing custody.
Source native-close drain, SDK/global heap and cold receipt reconstruction remain required gates;
this cell is not permission to retain a vault key/session or to issue a ready owner.

## Actual accepted reply before any local admission

Sync must retain an opaque receipt produced ONLY after the actual selected-KP/signature/group/
routing/policy completion succeeds. It owns the exact authenticated reply/binding; no caller
plaintext getter or decoded Validated phase fabricates it. App persists that exact receipt under
the same source/activation and whole catalog CAS BEFORE reserving any local scope. Signed invalid
replies never poison this phase. Native/API reply loss does not clear an accepted result. Immediately on actual successful SDK
return, move the exact candidate+opaque receipt+fees into the SAME retained source owner before
ANY fallible persistence, return or further await. Its state becomes HeldSuccess; a retry first
retries that receipt write without network, KP generation or another SDK mutation of the master.
Signed-invalid input never installs this state. `PreparedDirectJoin.completed=true` is not an
acceptable substitute for retaining the actual output it just produced.

The concrete owner must also survive a caller dropping its future/handle. Proposed shape is a
prepaid shared original exchange/result cell registered by Store at activation, with the actual
bound task owning ONE T/event prefix separately. The cell holds exact source/grant and concrete
non-T SDK candidate/receipt/fees; no Store, vault-key/session owner or generic transport is kept across await.
Store retains that cell independently of the caller until receipt persistence/explicit actual
terminal handoff. Dropping a task closes its negative activation but cannot silently throw away
held success; a paused/closed result remains DATA, never moves its Fresh graph into another
lease. An actual new negative DATA-custody consumer may persist the same verified receipt after
pause, without issuing admission; positive continuation must replay/verify the exact saved
source under the genuine new capture rather than adopt an old graph. Mount close/crash before
receipt barrier still relies on the durable original master/request and owner's exact cached
result; do not promise locally durable receipt before its real file/root barrier. The exact
cell/drop/native-close contract must be approved before code.

Cold ValidatedReply processing verifies the original historical invite signature/binding without
reapplying current expiry, recreates a private candidate from the SAME master and exact saved
reply through the actual SDK, and verifies original owner facts/root. This supplies no saved or
Current witness. Local recovery does not spend a network attempt, even if the grant is exhausted.
Missing finite owner proof remains held and explicit; it is not a downloaded replacement root.

Source-bound ScopeIntent, exact frozen snapshot before canonical I/O, v6 journal-bound member
save/restore and terminal Settled/file/root/Current/head/ready barriers stay separate required
steps. Until they land, the result has no old retry/ready adapter. SDK/global/native/physical
admission and production root-kind2 invitation remain open; no v4 exception is broadened.

## Required actual acceptance

Grant debit/reopen at every stage/rename/root boundary; genuine arm-ID duplicate/lost-return
lookup with one ticket and consumed-ticket crash loops; exact same source after lost arm/reply;
exhaustion surviving pause/new task/mount and explicit retry granting only checked new ID; no
request when owner/route is ineligible; full-budget local pressure before submission; single
activation and original Drop; pause/regrant while a delayed signed Welcome completes; old queued
work after lock/remount; same-T lifecycle prefix after pending cancellation; original versus new
connection incarnation; accepted and signed-invalid reply ordering; pre/post receipt stage/rename/root faults and
caller-future/handle cancellation with same retained result, one owner Add and no request retry; offline saved reply after
expiry/exhaustion; real owner Add once for exact retries; another independent group progressing.
No full-suite/compiler run until the pause is explicitly lifted by the primary/user.
