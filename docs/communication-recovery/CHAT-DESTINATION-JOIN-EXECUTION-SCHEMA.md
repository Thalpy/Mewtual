# Checkpoint join execution schema and private network/DATA consumer

The first connected implementation follows original negative task custody. Its source and
regressions are under static review; compilation/tests/build remain paused. It remains the finite
OwnerFact-v1 direct checkpoint (protocol1/root-kind1), not public-owner-v2 invitation/routing.
The first runtime packet stops at durable arms and retained RECEIVED INPUT. It does not create
an Accepted receipt, SDK membership, old initializer, scope intent, Current, or ready owner.
The implementation has an explicit trusted physical-open policy; ordinary/Pending/archive-only
readers remain strict. No native feature activation or successful compilation/testing is claimed.

## Explicit configured support

Ordinary Store open remains Empty-only. Existing Pending policy remains JOIN descriptor1/v1
only. A new explicit trusted Rust execution policy supports JOIN1 descriptor version2 together
with its actual reader/producer/retained network cell; no decoded field or public handler callback
enables it. Unknown versions/progress/root kinds refuse before cleaners and preserve both files.
Root owns the shared outer envelope, concrete combined ARCHIVE policy and whole-parent handling.
Execution changes only the JOIN payload and must preserve every other declared descriptor.

The v1 journal's never-network-exposed contract permits one explicit migration under actual
original source/file/root/admission/catalog barriers. Migration preserves ALL source rows,
immutable bytes, encrypted original masters and schedule revisions. It does NOT grant execution.
V2 initially represents each old row as NotGranted. A distinct actual trusted admission event
persists its first grant before a request can be armed. No restart/task/mount/transport/epoch,
pause/resume, decoded "unused" or lost reply may initialize another grant.

## Complete bounded row grammar

JOIN payload v2 has a distinct tag, count<=5, sorted original source IDs, and complete EOF.
Each row contains a framed ORIGINAL v1 source row followed by one framed execution record.
The original source frame keeps its exact protocol/root-kind, immutable basis/binding and
encrypted source. Only its existing scheduling revision/Active-or-Paused field may change.
Five total/at most one Active/all five Paused remain valid. Source absence/retirement is not
inferred from input, counters, task expiry or a missing lower reply.

Execution record version1 has these fixed/data fields:

```
grant_id       u64          0 only for NotGranted
grant_limit    u32          0 for NotGranted; otherwise 1..=64 from trusted local policy
grant_spent    u32          <=grant_limit; monotonic within this grant
last_ordinal   u64          checked lifetime submission ordinal, never reset by a new grant
latest_arm     0 | 1 + framed Arm
received_input 0 | 1 + framed original encrypted input
```

NotGranted has all zeros and neither optional field. A granted source may have zero spent units
and no latest arm; otherwise latest_arm is mandatory and its ordinal equals last_ordinal.
Ordinal must be at least the current grant's spent count; all additions/grant IDs use checked
arithmetic and refuse exhaustion. No wrapping, guessed refunds or implicit new grant.

Arm contains exact source-binding/request/KP digests, grant ID, ordinal, original scheduling
revision, activation nonce and invocation nonce (both32 bytes), and original finite deadline.
The nonce identifies the durable invocation, not a current physical runtime lease. Complete
framing/fixed lengths/identity equality precedes copies. An arm is ALWAYS possibly SPENT on
cold reopen. Persisted "armed" is never decoded into an unstarted runtime submission ticket.

Received input contains the SAME source/request and its ORIGINAL consuming-arm labels plus a
bounded sealed exact normalized response body, with its length/digest. Its arm may precede the
current latest arm: ordinal<=last_ordinal and grant<=current grant, with exact immutable source
digests. Equal ordinal requires the ENTIRE original Arm equal to latest; an older ordinal
must have grant and schedule revision no later than latest (even after zero-submit regrants).
The actual producer must retain that genuine saved original arm outcome, not reconstruct
it from labels. Local arm labels describe processing custody, not an exactly-once causal claim
about a delayed/deduplicated remote response. It is DATA. Parser/decryption/hash labels do not establish
the SDK actually selected this package, routing/policy validity, Accepted, Fresh or a save witness.
An input may survive pause/new mount without reviving the earlier task or moving its graph.
The first supported input cap is2MiB/body, with a separate1024-byte maximum inner source/arm
framing allowance before the16-byte AEAD tag; larger valid responses refuse without truncation or
discarding the original source. This closed first-Add checkpoint does not promise large-roster
or production public-owner-v2 joins. Unknown future phases (Validated, Intent, Frozen, Settled)
stay unsupported until their actual producers/consumers land.

## Closed transitions under whole original catalog CAS

Exactly ONE row changes per ordinary successor; other source/descriptor bytes and metadata stay
exact. The explicit V1?V2 migration is the sole deliberate multi-row exception below.
The outer generation is checked successor and parent is the COMPLETE previous plaintext hash.

1. V1→V2 migration: all original source frames unchanged; all records NotGranted. No ticket.
2. First grant: NotGranted→grant1, trusted limit, zero spent/ordinal, no arm/input. Preserve keys.
3. Debit: SAME grant/limit, spent+1, ordinal+1, exact new arm, no input. Must fit remaining units.
4. Receive input: counters/latest-arm unchanged; add only exact sealed DATA under the same
   source and genuine original consuming arm. A late older arm may preserve DATA; it cannot
   retarget its task/graph or reset any current counter.
5. Schedule: original checked schedule revision/toggle only, execution record byte-identical.
6. Explicit retry grant: old task closed FIRST; checked grant_id+1, trusted new limit, spent0,
   last_ordinal unchanged and historical last-arm DATA preserved. ALSO checked-increment its
   saved scheduling revision while preserving Active/Paused (this one event need not toggle).
   This is the exact durable incarnation which permits replacement in the existing predecessor
   registry. Never weaken general same-revision renewal or reset total ordinal.

The implementing grammar must distinguish historical last arm from a current-grant debit after
retry grant; a latest arm can name ANY earlier genuine grant (1<=arm.grant_id<current grant_id) only
while new grant_spent==0, including repeated regrants with no intervening submission. The next
debit atomically replaces it with its new grant/ordinal. A received input is preserved; a new
network debit is refused while input awaits actual bounded validation. Signed-invalid input
must never occupy Accepted: future validation rejects DATA without mutating the master and
uses a separately reviewed exact-input negative transition, not an arbitrary clear flag.

Until that negative/SDK consumer lands, the first received input deliberately stops execution.
There is no loop which treats a parse failure as permission to resample a package/root/grant.
No terminal/source removal or invocation reuse transition is supported by this packet.

## Exact warm arm ownership and persistent debits

The actual typed owner retains a genuine private `JoinArmInvocation` BEFORE debit. Its original
identity and nonce, old/current counter outcome and candidate belong to one source/activation.
`ensure_submission_armed` looks up that SAME invocation before choosing a new debit. It returns
no externally cloneable ticket. Lost/error return after disk work leaves the SAME exact pending
candidate/invocation and, once saved, unique unstarted ticket retained by the original cell.
No second live ticket is issued, even if the caller repeats the method concurrently.

The cell preflights route/capability/real lower fee before preparing a debit. Only actual saved
catalog/file/root barriers install its unique ticket. A private bounded try-lock arm reservation
may remain held during synchronous Store IO, but no Store lock ever WAITS on that cell: inspection
is try-lock/Busy, negatives atomic, and pause never takes it. Exact candidate retry does not
re-seal/redebit/re-sample nonce. A cold arm is spent, regardless of whether its warm ticket was
ever invoked. Repeated crash/reopen cannot reuse one ordinal indefinitely.

A retained warm ticket which provably never crossed Submitting may wait for eligibility on
the SAME T/target under its unchanged deadline. Immediately before the lower call the Sync-owned
cell consumes Armed→Submitting itself. A flat per-flight negative marks future cancellation
as spent without locking the cell. There is NO refund after ambiguous transport handoff. Another
request requires a distinct durable invocation/debit; new activation keeps the same counters.

## Network-only cell and received DATA

The actual App registers its prepaid cell independently of caller handle/future BEFORE any
request. The typed activated task continues owning its sole sender and ONE actual T/target/
empty-to-ordered lifecycle prefix. The cell holds original private source and fees but no T,
Store, vault key/session, or sender which could keep the task alive. Binding never resets.

Only `request_connected_cancellable` is permitted. Explicit actual support and SAME T's live
connection lease are checked before debit, then captured/rechecked at Submitting. Cancellation
COMPOSES original source/task/arm/connection/flight authority and actual lower keepalive. Never
replace the original authority/accounting, redial, infer a connection from PeerId labels, or
transfer a prefix onto replacement T. The stream must be exclusive/scoped; shared multi-group
Session dispatch/native integration remains a separate actual consumer.

The cell holds its PRIVATE async mutex across the exchange and synchronous input assignment.
No Store/writer/key/session crosses await. The normalization/framing/pinned-invite-signature
check is bounded and precedes retention; it does not run the selected-KP Welcome SDK yet.
After a complete signed body arrives, retain its exact owned bytes BEFORE any fallible IO,
return or further await. Source/path loss cannot destroy good received DATA; source-negative
only prevents positive continuation. Busy inspection never means Empty. Drop/close uses the
actual atomic/weak negative pattern, not a lock/wait on the cell.

The existing unjournalled finish API is still forbidden on configured mounts. There is no
public recovered→Prepared/finish export. Any required cross-crate interface is a narrow trusted
host context with the actual App source/arm consumer, no defaults/serde/raw accepted flag;
arbitrary Rust trait implementations are trusted code, not authenticated disk proof.

Persisting held input requires an original mounted SOURCE-DATA capture, not a new authoring
capture or task revival. It checks original physical mount, exact immutable source/cipher/
binding/request and known whole current catalog under admission→catalog. Its producer can
ONLY add the exact encrypted input/carry unchanged counters/arm/other rows. It cannot change
membership basis, grant units, replace a source, mint Fresh/Current, or clear unknown debt.
This negative DATA consumer may preserve a late input after the old task paused/closed; any
future positive SDK/ScopeIntent consumer still needs its actual original source/basis/intent.
Cold source/input recovery authenticates both exact files and historical invite bindings
without reapplying invite expiry; it issues no ticket and spends no network unit.

## Named allocation/persistence inventory

Retain the existing WHOLE16MiB JOIN payload cap, not16MiB per row. Five source ciphertexts at
<=640KiB+40 each plus five input ciphertexts at<=2MiB+1024+16 each fit that cap with fixed metadata;
encoders still checked-add every actual framed length and refuse rather than truncate.

Let P=maximum complete catalog plaintext (16MiB JOIN + fixed outer allowance), U=P+16, F=P+40.
Both-source read classification stays sequential: raw<=F+1, unframed<=U and decrypted<=U plus
two fixed summaries and outer/parser metadata. Writer retains the separate existing P+U+F
invoice while read scratch is live; no large byte/cipher/key scratch survives into writers.
Two fixed disk files may each retain F, so configured persistent allowance remains at least2F.
No saved size constitutes a filesystem quota grant or actual free-space reservation.

Network preflight additionally reserves actual request serialization allowance, current lower
RR maximum response (16MiB, NOT just the2MiB application input limit), own input2MiB and signature
transcript/normalization scratch, fixed authority/flight metadata, and retained sealed-input
overlap. The original source/master/provider fee remains separate. Lower response/request fee
follows driver completion, while input/cell fee follows its DATA custody independently. Exact
implementing constants/drop order must be reviewed against actual helper allocations; these
named logical buffers do not close SDK typed heap, allocator/RSS or global native admission.

## Required regressions

Actual V1 migration without ticket/grant; grant/debit/all root barriers; one unit short/local
pressure/unsupported or offline route before debit; exact lost-arm return lookup with one ticket;
same invocation concurrent calls; consumed-ticket crash/reopen loop; all counters preserved across
pause/new task/mount and explicit checked regrant; final original negative after catalog wait;
single same-T bind/prefix retention and replacement refusal; actual connected+cancellable positive
delivery; future Drop before/after Submitting; received bad signatures rejected BEFORE retention versus inviter-signed but SDK-invalid DATA;
caller abort/handle Drop after synchronous retention; pre/post input rename/root failures with
same held bytes/no request; exhausted grant still permits local DATA save/read; stale old task/
late arm/input after replacement; another group's independent existing control progresses.

Actual selected-KP SDK validation, opaque Accepted result, ScopeIntent/Frozen/terminal handoff,
the supported invalid-input negative transition, production root-kind2 and native/physical
acceptance remain explicit later gates. No active production UI dispatch is enabled here.

## Explicit retry-grant/task handoff

Retry grant includes a new saved scheduling revision, even if Active remains Active. It closes
the old sender before IO; write uncertainty keeps it closed and retains the SAME original
regrant invocation/candidate. Warm lookup recovers that exact saved grant/revision outcome before
considering another grant. The typed owner can preserve its SAME T and ordered prefix through
a consuming exact regrant continuation; it must not expose T/source extraction or adopt the old
cell/task identity. New negative task/cell registration follows only the exact saved newer
revision. A new physical mount uses a new T with empty prefix and reads preserved counters.
Test real grant1 arm?close/regrant2?new original task on the same owned T, old-cell refusal,
repeated grants with no submissions, and warm lost-regrant return with one candidate/outcome.
This is network/DATA custody only; no old Fresh graph crosses that task replacement.

## Persistent arm outcome versus lower capacity ownership

The actual App retains genuine `SavedArmOutcome`/invocation DATA custody separately from the
submission ticket's lower-work keepalive. After a consumed request terminates, retaining that
fee indefinitely in Spent/HeldInput would strand the next reservation when only one lower
slot fits. Retire the consumed ticket OUTSIDE the private cell guard on normal terminal paths.
A canceled Submitting phase reconciles to Spent and yields the retired ticket for out-of-guard
drop BEFORE trying to reserve the next arm. Driver clones remain independently charged until
ACTUAL completion; dropping the upper copy does not refund driver-owned work. Persistent spent
units/ordinals never refund, even when lower capacity is free again.

Held input retains its separately prepaid DATA buffers and genuine original saved-outcome
witness, not an accidental long-lived network capacity owner. The exact host/cell interface
must preserve that witness after disposing of lower custody without reconstructing authority
from arm labels. Test one-slot failed request?real completion?next reservation, cancellation
while the lower driver still owns the first fee, repeated status/arm reconciliation, and held
DATA/late-input proof survival after upper ticket retirement. No pool lock or fee destructor
may run under admission/catalog or the cell's private mutex when it could invert ownership.

## First connected implementation boundary

`CheckpointJoinExecutionPolicy` and explicit archive policy composition choose version2 before
both current/stage classification and old cleaners. The actual `CheckpointJoinNetwork<T>` owns
ONE original transport/task; an independent Store-registered Sync cell retains immutable source
DATA and signed input when the caller/future drops. `ensure_initial_grant` saves only the first
configured allowance, `ensure_submission_armed` saves one exact invocation/debit before ticket
construction, and repeated warm calls recover that SAME ticket/outcome. `next_submission` is
explicit and cannot reset the saved grant. Cold arms remain possibly spent. Exhaustion refuses;
a changed configured limit cannot create another grant.

The connected request requires the actual no-redial connected+cancellable seam and original
connection incarnation. The original lower keepalive is composed into that request and retires
only with actual lower completion. Spent/Held DATA retains a separate saved-arm witness, not
lower capacity. A Sync-private mutex spans network/signature validation through synchronous
HeldInput assignment, with no Store/session/writer held across await. The retained body cap is
2MiB; the lower response allowance is separately16MiB. The lower invoice assumes the repository's actual bounded RR implementation; merely returning
true from a generic transport capability method does not prove its whole-buffer/RSS admission.
These named logical buffers, fixed type layouts and original fees are not a process-global/RSS/
allocator or SDK bound.

`persist_received_input` and Store's bounded `persist_retained_checkpoint_join_inputs` seal the
actual signed DATA once and retain the original ciphertext across errors/lost returns. A distinct
`ReceivedJoinInputPermit` may operate after pause or admission retirement, on the SAME physical
mount/pin/source. Under admission then catalog it proves EXACTLY one Input change, unchanged
all counters/latest arm/schedule/source/other rows/archive descriptor, then hands off the immutable
candidate before file I/O. It cannot enter any grant/task/scope path. Existing unknown-source,
whole-parent, root barriers and warm same-handle partial repair remain mandatory. Cold malformed
partial stage preservation/refusal remains unchanged.

This packet does NOT implement an executable explicit RetryGrant/same-T continuation, cold
input-body authentication/SDK reconstruction, Accepted receipt, ScopeIntent/Frozen/Current/ready,
public-owner-v2 invitation/root distribution, global native admission or complete typed SDK heap.
The RetryGrant grammar is inert retained DATA for its next actual consumer; no renewal API is
exposed. The older undeclared `retained_exchange.rs` SDK draft remains excluded.

Authored UNRUN coverage: eight execution-state DATA tests, seven nested payload DATA tests,
thirteen App Store/network tests and three Sync lower/once-bound tests. This is source evidence,
not runtime acceptance. The primary agent owns all compilation/focused/full gates when resumed.
