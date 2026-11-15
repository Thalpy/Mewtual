# Durable local edit, delete and exact-token Stop

Design checkpoint, 29 October 2026. No implementation or test evidence yet. The
primary owns integration and complete test suites. This slice does not migrate
raw chat CRDT history to a typed ledger or close the documented R6 limitation.

## User-visible contract

A local edit/delete is accepted only after the same signed operation, retry
receipt and publication obligation have passed the existing encrypted server
snapshot barrier. An identical-text edit instead saves an explicit no-op receipt:
it creates neither a signed mutation nor an edited timestamp. A lost reply retries
the same sealed caller intent and native preparation. Stop saves an exact-token
terminal disposition before freeing its channel reservation. Stop cannot retract
an operation that was already saved, or pretend a post-rename write did not exist.

The existing `DurableSendRequest`, post binding/id/signing domains and post wire
bytes stay unchanged. The storage and sealed UI formats require a versioned
extension; the current unconditional `ChangeHash` cannot honestly represent an
unstarted Stop or no-op. Calling those states a fake change would be incorrect.

## Requests and target identity

The application adds these distinct requests; none accepts a caller authorization
boolean or a caller-supplied signed operation:

```rust
enum DurableMutationAction { Edit { text: String }, Delete }
struct DurableMutationRequest {
    token: [u8; 16], expected_context: [u8; 32], channel: u128,
    message_id: String, target: [u8; 32], action: DurableMutationAction,
}
enum DurableChatRequest { Send(DurableSendRequest), Mutation(DurableMutationRequest) }
struct DurableStopRequest { original: DurableChatRequest }
struct MutationContext { context: [u8; 32], target: [u8; 32] }
```

`ServerActor::durable_mutation_context(channel,id)` resolves the same unique visible
message as the reader. Empty/duplicate/idless targets fail. The target digest binds
the MLS group/document, winning message id, exact containing list and exact map
object. Canonical object bytes are actor bytes and operation counter; Automerge's
local actor-index hint and current list index are excluded. Text, edited time,
reactions and unrelated fields are excluded: an intervening edit on this same map
must not invalidate the cached operation. A different object reusing the same id,
ambiguous id, or removed target cannot adopt the old preparation.

The mutation request binding has a new domain and includes original context,
channel, action tag, length-prefixed message id, exact target digest and exact edit
text (delete has no text). Send binding is untouched. Every lookup checks token
and this entire binding before returning anything. Native captures the original
Actor and server instance before awaiting context/Ready; the final lease repeats
the existing persist -> UI commit -> mount/store -> exact-instance checks.

Saved terminal lookup precedes current target/role checks: replaying a saved
delete must work after the row disappears or the caller is demoted. Every
UNCOMMITTED retry rechecks current membership/context and current action
permission, even in the same MLS epoch. It also checks the exact target identity;
it never resolves a different equal-id row. Ordinary concurrent edits remain
mergeable because these checks do not bind mutable text/edited fields.

The existing own-author/admin gate remains the legacy compatibility boundary.
Where a unique original signed creator change is available, verify its signature,
scope, actor/counter range and full signer; contradictory/ambiguous modern proof
must refuse. Historical evidence that is genuinely unavailable remains explicitly
legacy, not invented historical authorization or an R6 fix. The creator lookup
must not equate raw ObjId actor bytes with verified authorship.
Legacy signatures bind document type/id, not their original MLS group: this
evidence establishes a signer for a stored creator, never historical group
membership or a new authorization certificate.

## Record extension and restore

Decode existing durable-chat v1 unchanged. Encode v1 byte-for-byte while all rows
remain legacy signed sends. Once this server has a mutation/explicit Stop/no-op,
encode sticky v2 (first byte 2); old v1 readers reject before records are admitted.
Retain all existing count/pending/byte/duplicate-token/channel checks.

V2 keeps the original token/binding/context/channel/message-id fields and adds:

* closed action kind: legacy send / edit / delete;
* optional exact target digest for mutations;
* closed effect: Signed(real ChangeHash, original signed bytes, original sealed
  bytes), NoOp(real bounded document heads), or NeverAuthored;
* separate admission and terminal facts: prepared / committed, plus explicit
  stopped. NeverAuthored is valid ONLY for a saved exact-token Stop;
* stopped Signed retains its actual change identity/history; publication bytes
  may be cleared only after the stopped snapshot succeeds.

NoOp has no publication and no fake change. Its heads are real, capped before
allocation (candidate limit 256; refuse rather than truncate). A failed save keeps
the same no-op preparation and reservation. Replay never reevaluates whether its
text happens to match a later state. NoOp cannot become a later signed edit.
The receipt records the sampled no-op decision, not a promise that concurrent
edits leave the currently displayed text equal to the requested text.

For new Signed rows retain the exact verified local SignedOp alongside its sealed
packet, within one combined 256 KiB payload cap. This permits additive historical
repair of a post-rename operation even if the MLS epoch changes; it does not reseal,
reauthor or grant current publication authority. A narrow replication method may
apply only this saved local-device, exact-scope signed operation after checking
signature and change hash. Legacy v1 pending rows continue their original sealed
restore path; before their first storage attempt cache the corresponding signed
operation so an uncertainty cannot lose it to epoch rotation.

Prepared signed/no-op rows consume the same shared 32 pending slots. All terminal
receipts consume the existing 65,536 non-evicting receipt cap. Stopped rows discard
payload only after the stopped snapshot succeeds; bindings/tokens are never evicted.

## Save, install, uncertainty and Stop

Replace the String-only save callback with a typed internal failure:

```rust
enum ChatSaveError { NotReplaced(String), ReplacedNotDurable(String) }
```

Map the actual `AppError::CommittedButNotDurable` to the second arm at the store
boundary. The first arm restores the private preparation with no live/exportable
change. The second also withholds acceptance/export, but latches the ENTIRE same
candidate record as `replacement_uncertain`: binding, effect, possible admission,
and irreversible Stop intent. This includes metadata-only NoOp and NeverAuthored
Stop, not only signed operations. It is not proof that the old file survived.
Once latched, later failures cannot downgrade it to NotReplaced or token-absent;
a possible saved Stop fences every later positive call for the same token.

Ordinary snapshots must refuse while any uncertain record is hidden, rather
than overwrite a possibly installed saved history with a rollback-only graph.
Only the synchronous durable-chat transaction may construct a reconciliation
snapshot. It merges each uncertain original signed operation into the LATEST
live graph, preserving received operations, and carries every exact uncertain
no-op/stopped receipt. Then it saves before installing the merged graph/terminal
state or returning success. No receive/await occurs in temporary swap,
save or rollback. This uses the existing single Actor storage barrier, not a WAL.
Uncertainty and possible Stop intent survive in process until repair succeeds;
an uncertain Stop cannot be bypassed by an epoch change or positive retry. After abrupt loss, either
the old file remains (no original accepted/exported) or the renamed candidate
restores its exact committed receipt and operation. No unsaved candidate is claimed
recoverable after a crash that erased all of its bytes.

Unwind safety is part of this boundary. If the synchronous snapshot/save callback
panics, restore the prior live documents before unwinding and conservatively
latch the ENTIRE possible candidate (including Stop/no-op), never NotReplaced.
An actor that terminates cannot export the temporary graph; an internal caller
that catches the panic still sees hidden uncertainty requiring repair. This is
not permission to catch arbitrary peer/SDK effects inside the storage callback.

### Automatic negative repair without a renderer

Uncertainty cannot strand all server persistence until somebody clicks Retry.
Add private `ServerActor::prepare_durable_chat_repair()` returning
`Result<Option<DurableRepairReady>, String>`. The Actor returns None with no store
access when no repair is needed; otherwise it retains the same bounded original
records and offers Ready. A `DurableRepairReady::commit(DurableSendLease)` repairs
all uncertain original effects/terminals and writes their union with latest
received history. It admits no new user action, token, authorization or publication.
The saved-local-operation helper must accept only an opaque original preparation
owned by those records, not a generic caller-provided raw SignedOp admission API.

Native `persist_captured` invokes this check while holding its SAME per-id
persistence serializer, BEFORE ordinary snapshot capture. On Ready it transfers
that original owned serializer guard plus mounted store and exact server-instance
registry guard into a negative repair lease. Store/registry acquisition AFTER
Ready uses try-locks, not awaits; failure drops Ready and leaves dirty/retry state.
This negative authorization deliberately does not require an unlocked UI, new
session grant, current role or current MLS context. It checks the actual mounted
store and original Actor incarnation; replacement/unmount stays Superseded/Pending.
The existing persistence worker/discovery retry drives it while UI is locked,
even at full pending/receipt capacity. A successful repair snapshot covers the
captured dirty ticket; an unsuccessful one remains dirty. No repair means the
normal coalesced snapshot path continues, so no extra full write is added.

Stop is another Ready/lease transaction with the full original request binding:

1. Existing saved operation: retain it additively, save stopped publication state.
2. Existing ordinary private preparation: save stopped state, then remove its
   active channel reservation. On failure restore preparation/reservation.
3. Existing uncertain replacement: FIRST reconcile the same original operation
   additively in the stopped snapshot; never discard it based on a failed reply.
4. Token not yet seen: save a NeverAuthored stopped tombstone with the exact
   original binding before freeing anything. A late original commit sees that
   token and returns the saved stopped outcome; it cannot author after Stop.
5. Foreign binding: refuse without changing either record.

An ordinary uncommitted positive retry must pass current permissions. Historical
reconciliation of an already possibly installed operation is a distinct negative
repair path, not fresh mutation admission. Epoch/context obsolescence must not
clear an uncertainty before repair. Stop after a known saved operation does not
wait for a peer receipt and never removes its document change.

The result is typed: `committed` (signed or no-op), `stopped-before-commit`, or
`stopped-after-commit`, together with durable/replayed and a pending storage reason.
No PERSISTED diagnostic or UI accepted state is emitted for Pending/Superseded.
The low-level legacy edit/delete helpers remain documented non-durable APIs.

## Frontend and native API

Edit/delete Tauri calls require token, expected context and target digest. All
production callers, including moderation deletion, use the shared pending queue;
there is no missing-token fallback that can orphan a private preparation. The
native context RPC returns only context/target metadata, no capability.

Extend the existing bounded pending-intent union with explicit Edit/Delete
variants. Posts remain their existing shape. Mutation matching includes exact
kind/target/text; composer matching excludes mutations. Delete permits empty text
ONLY in its closed variant. Reactions and pins remain unchanged in this slice.

The first mutation seals at least sticky continuity v2 BEFORE dispatch. Native load/save
accepts v1/v2/v3; old native `version == 1` rejects v2 rather than letting its old
frontend strip mutation fields and replay edit text as a post. The new sanitizer
rejects unknown action/version instead of coercing to Send. Preserve every other
continuity field and saved copy on rejection. Ordinary post-only v1 remains valid.

The editor captures original server/channel/id/text before its first await and
keeps the draft until sealed intent admission succeeds. Dispatch first seals the
same token's ambiguous state. Lost replies never make another token. Retry uses
the same shared queue/timer/caps. A newer editor/composer cannot be cleared by an
older completion. Pending UI labels edit/delete and omits meaningless recovered
delete text. Stop FIRST seals `resolution: cancel | recover` on the same caller
intent; restart/auto-retry then repeats Stop, never positive commit. It receives
the durable native stopped/committed terminal, then saves continuity removal or
the recovered draft; failure/lost reply retains the original intent and its
resolution, and retries Stop with the same token. UI lock/unlock cannot revive
a stopped token. No native exception is treated as proof of nonacceptance.

Frontend admission validates the complete mutation before queue insertion, including the
64 KiB UTF-8 text bound, 128-byte target id bound and well-formed Unicode required by native
JSON decoding. Lone UTF-16 surrogates are refused without normalization. User payload validation runs before
context lookup, and rejection leaves the editor intact. Permanent `CHAT_MUTATION_TARGET_CHANGED`,
`CHAT_MUTATION_AUTHOR_CONFLICT`, `CHAT_MUTATION_NOT_AUTHORIZED` and `CHAT_MUTATION_INVALID`
refusals persist an automatic-retry pause. They never erase an earlier ambiguous attempt;
manual retry and ordinary Stop retain the exact original request identity.

An older frontend could seal an oversized edit or a target ID longer than 128 UTF-8 bytes
before native rejected it. Its queue checked only aggregate storage; old ID hydration's
128-code-unit check was already lossy. The target-ID bug affects both edit and delete.
Native Stop also validates that original payload. Continuity v2 therefore reads the old
sealable payload within aggregate storage bounds and retains these known invalid cases
within the existing 32-row/256 KiB per-shelf bounds, with every original custody, acceptance,
retry-disposition and resolution field. Quarantine is derived from the original invalid bytes,
not a saved authorization flag. Automatic/manual retry, direct submission and Stop all refuse
those rows. Other invalid custody or unknown kinds/versions remain fail closed.

The manager offers Copy and an explicit saved-text/request recovery for such rows, distinct from Stop.
Recovery atomically seals removal from pending and the complete unchanged original in the
nonretry recovered shelf, then commits both in memory. Failed writes or a full shelf retain the
pending original; Copy remains available. Reload accepts either shelf and ordinary chat/continuity
work continues. This local move establishes neither native nonacceptance nor a durable Stop.
An invalid-ID deletion retains its full request identity with a visible action/message ID, without
empty-text composer controls. Recovered text never publishes automatically and any later new submission must pass ordinary
admission. These mutation recovery changes preserve native payload validation and request/receipt
identity; the related Send retention below separately requires a critical envelope version.

### Critical v3 for lossless Send retention

The related old Send path also sealed text above 64 KiB or reply IDs above 256 UTF-8 bytes.
Existing v1/v2 hydration silently dropped these Send rows, and even dropped native-valid reply
IDs above 128 UTF-16 code units. Therefore retaining enlarged Send fields uses critical continuity
v3, which old native v1/v2 readers reject before their lossy frontend can touch the payload.
New Send admission checks the native byte bounds, Unicode and full fixed custody before queue
insertion; payload checks precede context lookup. Invalid input stays in the composer. A new
native-valid reply above the old 128-unit read limit also seals v3 before frontend dispatch.

Read grammar follows the old queue's sealable payload: recognized kind, valid fixed custody,
nonempty text/ID where applicable and the closed delete shape, bounded by the existing
32-row/256 KiB shelf. Native-invalid byte lengths derive non-dispatch quarantine for sends,
edits and deletions; neither positive IPC nor Stop receives those originals. Known invalid Send
text/replies use the same lossless Copy/recovered shelf; native-valid long replies retain exact
token/context/payload and normal retry, including earlier acceptance uncertainty. Malformed fixed
custody, count/byte overflow, present unsupported collection shapes (including null), and unknown
kinds/versions fail closed with the sealed copy retained,
instead of dropping rows or truncating the queue. No positive or Stop payload validation is relaxed.

Loading a record which needs v3, or already has v3, drains previously admitted continuity writes,
reads the sealed root, then queues a covering save of that complete original root with only its
version changed to 3. UI readiness and new frontend dispatch stay blocked until acknowledgement.
Even an already-v3 root is re-saved: observing bytes after a failed post-rename parent sync is not
proof of durability. Failure retains the full original payload and blocks readiness until retry.
Generation and per-load attempt checks surround reads, queued save admission and completion;
superseded same-session loads cannot expose stale state. All version assignments are monotonic,
including mutation/Stop conversion, legacy read-mark migration and empty queues. Unknown v4 is
refused. The native envelope extension accepts v3 without changing request or receipt bytes.
This metadata barrier does not retract native transport work already admitted under a valid saved
Send; existing native membership, session, token and publication guards still own that obligation.
Compatibility assumes a supported fresh startup with matching bundled frontend/native versions and
exclusive mounted-vault ownership. The new frontend never downgrades its envelope. The native
envelope reader still supports old versions; this is not a universal ban on a stale independently
running frontend writing an older envelope. Mixed old/new development or HMR runtimes sharing an
already open UI session are unsupported, as is a rollback of the authorizing stored bytes themselves.

### Vault entry ownership

Unlock is a locked-to-open entry, never a refresh of an already open UI. Its synchronous guard
rejects duplicate Enter/button submissions and closing/unlocked surfaces. A separate attempt ticket
owns busy cleanup; the captured UI generation and locked/closing checks gate both the continuation
after the prior native lock and the native unlock reply. Successful restoration advances UI generation,
so first-run setup continues only with that attempt's matching success receipt. Stale errors, secret
resets and finally blocks cannot mutate another attempt. Restoration itself also refuses an open UI.

Each authentication joins its prior lock and checks current ownership. A present result must have a
valid actual generation before it can be retired; rejected settlement or malformed proof never falls
through to a replacement. The attempt then establishes one fresh null-snapshot native barrier and
checks ownership/proof again before resetting that fulfilled result and dispatching unlock without
another await. A cold attempt needs only that one fresh barrier. A cached result can be stale after
a surviving native close, whose DTO intentionally supplies no reusable unlock generation; the fresh
barrier avoids a needless correct-secret rejection. It also joins a newer pending exact JS snapshot
before rotating custody, rather than blindly resetting the coordinator on a confirmed close.
A locked-screen cancellation can then submit a fresh null-snapshot native lock; cancellation
while settling instead joins the original exact lock/snapshot. Rejected bridge settlement keeps JS
snapshot custody. Fulfilled valid-JSON write debt is retained by native and retried before native unlock
opens IPC; malformed JSON retains its existing explicit warning/acknowledgement limitation. Native
command admission independently orders queued authentication against lock; JS call order alone
does not prove the order in which spawned native command futures first run. Every real lock result
therefore carries its registered `generation` as a canonical decimal u64 string. Unlock must forward
that exact result as required `expectedUiGeneration`; a cold entry first establishes a null-snapshot
native lock. The generation is cancellation context; passphrase authentication is still required.
Missing, malformed or out-of-range results refuse authentication before coordinator
reset, without a local counter or current-generation fallback. Native checks the supplied generation
before its first await/KDF and at every final admission, so a future first polled after cancellation
cannot adopt the newer generation. Old unlock requests missing the field are refused. The optional
TypeScript outcome field is solely for legacy non-authentication mocks, never a runtime fallback.

Initial resume and gate reads capture the original attempt/generation and cannot restore or change
the gate after a manual attempt, lock or close. A canceled initial read or surviving failed/deferred
close starts a fresh rightful gate query rather than inferring vault existence or remaining stuck.

## Required verification

Design review precedes runtime edits. Focused deterministic tests must cover real
store failure before rename and parent-sync failure AFTER actual rename; hidden
uncertainty cannot be overwritten by ordinary snapshots; Stop repairs the same
operation and preserves foreign received history. Save/Stop races in both orders,
unknown-token Stop/no-op/stopped-signed rename followed by parent-sync failure,
late positive calls and epoch movement must retain the exact possible terminal.
reply loss and fresh restore, exact duplicate/foreign token, same-epoch demotion,
object substitution/duplicate id, concurrent edit/edit and edit/delete, role/epoch
change, stopped late retry and lock/unlock/server incarnation all need coverage.

Frontend tests pin sticky v2 old-reader rejection, seal-before-dispatch, no token
remint on ambiguity, action-aware recovery, exact durable Stop ordering, newer
editor preservation and combined pressure caps. Real signed-op identity/log count
and stored fresh reopen are assertions; literal helper success booleans are not
durability evidence. Primary runs complete Root/native/frontend/lint/build/flows
gates after integration. None has run for this design checkpoint.

## Implementation clarification (29 October)

The no-op receipt retains at most 256 real heads and refuses larger frontiers. Automerge's public
accessor materializes the existing frontier before that retention check; transient snapshot/frontier
work still scales with the existing graph. The earlier "before allocation" wording above is a design
aspiration, not a property of this implementation. Decoding still checks the count before allocation.
Retired Stop additionally uses globally bounded vault fences keyed by numeric server and exact token,
with the canonical original request binding. Its historical-unknown disposition is `stopped-retired`.
Pre-registration restore custody remains inside the actual Actor future through cancellation until
real task completion, and exact matching saved receipts are stopped before restored publication.
