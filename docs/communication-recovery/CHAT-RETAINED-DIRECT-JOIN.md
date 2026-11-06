# Retained direct join (5 November; in-process only, authored/unexecuted)

This slice corrects CLIENT private key-package loss across an error or cancelled join future.
It does not implement crash recovery, a durable join namespace, production v2 invite/root
distribution, media/voice activation, complete SDK heap/work admission or remote-save evidence.
The critical v6 catalog still accepts EMPTY only. All new tests remain UNRUN during the user's
compilation pause; source formatting and review are not executable validation.

## Actual connected source

`prepare_owner_evidence_checkpoint_join` captures the original destination root and reserves its
configured canonical creation fee as before. Its private factory now consumes that SAME fee and
prepares an opaque `PreparedDirectJoin` before any network await. The original pending creation
validates its mount, canonical pool identity, negative observation and active ownership before
exposing immutable snapshot limits; another Store's fee cannot pay for this source.

Preparation first bounds every variable invite field and count, then materializes a new owned
canonical invite so caller String/Vec spare capacity cannot survive into an asynchronous task.
It verifies the actual signed invite, mints ONE invite-bound KeyPackage, retains its exact TLS
bytes in the exact kind-1 request and retains the selected reference. Its distinct tagged bare-
device snapshot holds the actual provider including private init keys. The original mutable
device is dropped after preparation. No decoded-snapshot -> Prepared or generic old-proof/new-
capture constructor exists. The MLS snapshot restore returns device DATA only, never a group,
fresh admission, Current or Saved witness.

The exact wire remains `/catcoms/rr/1`, kind1 followed by the original invite and KP u32 frames.
Legacy `Server::join` / `request_join_tracking` and all helper/grant/reply protocols retain their
existing behavior. This slice changes only the private canonical destination checkpoint consumer.

## Response and retry ordering

The inactive Rust API deliberately changes to two steps: consume the unbound attempt with
`bind_direct_transport(transport, inviter)`, then call the resulting typed
`OwnerDirectCheckpointJoinAttempt<T>::finish_direct(&mut self, rng, clock, name)`.
The bound object owns ONE actual transport and its consumed `PreOwnerConnectionHandoff` outside
the future. No replacement-T retry or snapshot/lease-based prefix reconstruction exists.
The exact request is reused after rejection, transport error, timeout or a dropped pending future.
The bound App consumer retains the existing whole-exchange timeout and lifecycle handoff.
JOIN_PENDING still waits for an inviter-matching authenticated pushed Welcome; unrelated requests
are acknowledged without admitting their content. After sixteen dispatched events it yields once
before continuing, so a ready-event flood cannot starve parent timeout/cancellation polling.

Each response is byte-capped, copied into exact owned client backing and detached from the lower
adapter's potentially larger Bytes backing before SDK work. Complete borrowed framing and the
original invite's signer verification occur BEFORE restoring a candidate provider. Only then is
the SAME immutable master restored into a NEW private device. Existing `finish_join` still checks
actual SDK-selected package, exact group, routing decryption, immutable policy and real fresh
completion. A signed but invalid routing/policy/Welcome can consume that candidate's keys, but
cannot mutate the master. Fresh admission is attached only by that real successful selected-KP
join. Exact request replay is not permission to admit a different package from a spent invite.

Only actual successful Sync candidate handoff marks its prepared source completed. A second
success is refused. The App then repeats the ORIGINAL capture/pin negative check and immediately
moves its private Server/capture/pin without an intervening await. Revocation in this interval
still prevents later original scope/save/ready gates; an old queued task cannot adopt a reopened
Store. A cancelled original scope is not a reusable new admission. The successful private candidate
is retained by `OwnerJoinedInitialization` across later proof/save failures as before.

On success the master encoding drops BEFORE its original real fee transfers into canonical
initialization. On error or cancellation both source and fee remain in the attempt. On ordinary
attempt Drop, the secret source field drops before the fee. No uncharged master is left behind
in the now-empty outer attempt. The canonical Current/Saved/head/writer requirements and freeze
semantics are unchanged; neither network success nor this client source releases a runnable actor.

## Resource boundary

The private source uses limits from its actual existing pending-creation reservation. Its existing
fee covers four snapshot/exchange buffers with framing slack and two explicit provider table/
retained-payload inventories. The encoded master fits the configured MLS limit; request/response
fit the configured snapshot/exchange limit (also no more than the existing16MiB control ceiling).
The bare-device encoder preflights the actual provider backing BEFORE its first scan and retained
Vec capacities before output; the borrowed decoder preflights complete outer frames plus map
count/length/backing BEFORE map/payload allocation. The actual signer must equal its full32-byte
encoded identity after SDK loading. KP encoding length is checked before its SDK clone/serialize
and reference hash. AFTER Welcome processing writes new provider rows, the final candidate must
pass that same raw backing/payload/encoding preflight before handoff. This retained-state postflight
can refuse an oversized candidate while preserving the original source; it does not bound SDK
transient allocations that already happened. Provider and network byte limits are independent.

These bounds do not meter SDK KeyPackage generation, typed signer JSON, Welcome/tree/group heaps,
allocator overhead, runtime tasks or the lower transport's own retained request/reply backing.
Those require their actual separately paid admission/ownership boundaries before activation.
The current raw-slot fees do not establish the complete simultaneous native/SDK scratch or
global host invoice; that remains an independent production gate.
Encoded caps are not a general heap guarantee. The existing creation fee does NOT pay a new
durable pre-join source: the future join journal needs its own persistent/hot invoice and bounded
active/paused slot policy, original mounted recovery issuer and terminal Saved+Current settlement.

## Authored regression evidence (all UNRUN)

Two MLS tests exercise the distinct bare-device DATA format/provider limits and recreate the SAME
selected private keys after independent candidates consume them. Seven Sync tests exercise actual
SDK consumption on signed-invalid routing before a valid exact retry; older private-package traps;
forged/truncated/oversized/signed-invalid Welcome; cancellation after an actual legacy owner Add
followed by identical request/cache recovery and one successful handoff; pending/foreign pushes
and bounded event-turn progress; final post-Welcome provider growth; and independent exchange limits.
The legacy in-memory owner proves no covering remote disk write or complete production first-Add.

Three new App tests pin actual foreign-fee refusal, dropped-future retention/original mount refusal,
and actual App pending lifecycle drain -> cancellation -> cached inline READY adoption. A different
transport registration under the same PeerId inherits none of its prefix. Existing App factory
tests retain the SAME actual creation fee and preserve their semantic assertions. Native fixtures
must explicitly bind the transport once and call mutable finish without a replacement transport.

Read-only design and actual-diff reviews are clear for this inactive in-process boundary. The
broader durable client design remains open in
[CHAT-DESTINATION-JOIN-CUSTODY](CHAT-DESTINATION-JOIN-CUSTODY.md); do not treat this in-process
checkpoint or the owner-side result cache as crash-safe destination recovery.
