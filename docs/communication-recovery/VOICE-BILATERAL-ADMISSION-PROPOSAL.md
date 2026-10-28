# Bilateral native V2 admission proposal

Design only, 28 October 2026. No runtime change, wire allocation or activation is
implemented by this note. The native/browser handoff owner is the primary agent
(`/root`). That owner has agreed the four directions below, subject to the concrete
enforcement/inventory review and a final agreement on this proposal:

1. Separate closed network tags and key domains; preserve existing canonical bytes.
2. Actual Root DeviceId orientation, with neutral 6/6 packet backing before advertising.
3. Local authenticated failed-round Forget/ACK, independent of a silent peer's ACK.
4. Sticky operation31/version12 profile selection before setup ACK and worker selection.

This refines `VOICE-MULTIPEER-ADMISSION-NEXT.md`'s reviewed 28 October candidate.
It does not replace the original acceptance requirements. Root/native/browser code
owners and implementation prerequisites are listed below; there is no independently
owned placeholder interface that another agent is assumed to implement.

## Existing enforcement to preserve

- `voice_v2.rs`: `SignedObject` signs exact canonical scope/issuer/key/id/time/body;
  `fixed` adds a four-byte length. Existing Body tags1-6 and `CompleteSdpV1` stay unchanged.
- `presence::FreshPeer` is current challenge-backed presence, not capacity. Native
  `SelectedHandoffOwner` does not supply local DeviceId; the actual Root Actor must.
- `admission::RootAdmissionSession` owns the original unpredictable public incarnation.
  Ordinary generation cancellation is distinct from revoking that registration.
- `RootHighPreclaim` is HIGH only and exposes no consume API. Its packet backing is
  incoming6/outgoing3; LOW requires incoming3/outgoing6. It cannot be relabelled LOW.
- Native `Ledger::reserve_parent`/`ParentClaim` reserves one checked physical row serial
  and the original 2304-byte fee plus PreparedPhase2KiB. Its affine return marker excludes
  all allocators until actual owners/payment destruction. No consume API exists yet.
- Browser ABI50 pairs one original128KiB/twelve-row preclaim with the native generation.
  Its512KiB group default and2MiB process aggregate are separate from Root's512KiB group
  budget. The user permits measured local increases, not unaccounted or peer-controlled ones.
- Existing post-Finish `PairReadyRound` is causal crypto readiness. Availability before
  an agreement must be a different object; neither is a generic RTC permission.

## Profiles and the local initial fence

Proposed closed enums: `AdmissionProfile::BilateralCompleteV1 = 1` and
`SdpProfile::CompleteSdpAdmittedV1 = 2`. Existing SDP profile1 is unchanged. New network
Body tags7/8/9 below are distinct at both ingress and helper selection. New constructors
must use the same retained window/channel symbols with a new compatible broker ABI.

Proposed local operation31 has Control class, the exact physical Session logical ID,
and a three-byte plaintext `[12, kind, profile=1]`; kinds0/Probe,1/Select,2/Cancel.
Its ciphertext is19 bytes. The reply is37-byte plaintext:
`[12, state:u8, profile:u8, support:u16, contract_hash:32]` (53 ciphertext).
`state` distinguishes supported-unselected, selected and permanently closed. Unsupported
requires a NAMED authenticated `UnsupportedProfile` or `UnknownOperation` response whose
defined meaning is explicitly unsupported version/profile/operation. Authenticated Capacity,
Unauthorized, Malformed or generic decoder refusal is NOT version evidence: classify it as
Refused/Unavailable/Unknown, without inferring an old implementation. Bare HTTP refusal,
missing authentication and silence remain unknown/unreachable/uncertain. An existing v11
native may reject unknown31 only through HTTP; that remains unknown, not Unsupported.
Partial implementations must not claim the complete support mask.

New constructor order is29/Register ->31/Probe ->31/Select ->30/noncreating Observe1
-> ACK9. Select is sticky and must precede BOTH ACK and the worker's choice of backing.
31/Cancel before Select, or29/Cancel first, installs a permanent negative profile/mode
fence. It never chooses pilot or revokes an unrelated registration as a substitute for
generation cancellation. A late Select cannot reopen it. Select after ACK refuses,
including on a retained capacity-only admission channel. No old v11 Reserved response
establishes operational row/consume/KDF/RTC support. Lost Select replies coalesce on the
same physical owner, keys, counters and profile; they never replace the channel.

Operation30/version11 can retain its capacity observation bytes. An operational positive
factory additionally requires the selected31 contract and the exact committed agreement.
Capacity-only29/30 clients keep their current HIGH-only, non-operational behavior.

The sticky selected profile identity and its admission-open flag are DIFFERENT fields.
31/Cancel closes fresh admission while retaining the original selected identity for exact
history/negative decoding. Existing indexed R handlers use their immutable RoundGate/profile,
not whether the free admission slot is open. Admission-only cancellation cannot revoke
healthy indexed media. Full End/lock/epoch owns the separate all-round fence. Ordinary
30/Cancel(g) closes only that generation, never the registration as a substitute.

## Capacity and orientation

Introduce `RootRoundPreclaim`, for the newly selected operational profile only. Reuse
the existing ChainCapacity prepayment machinery, but reserve incoming6/outgoing6 once.
It retains ONE group progression claim, the same twelve Action positions, byte reserves,
crypto/lower lanes and actual pool identities. Do not stack another GroupProgressClaim
or invoke `PreparedChain::new`/legacy pair creation at consumption.

The actual Root candidate establishes `local DeviceId < peer DeviceId` for LOW. No peer,
role, cursor or caller label is accepted from the renderer. A closed private
`PacketLayout::{Neutral,Low,High}` can replace the inert boolean; no second packet wrapper
is required. Validate both exact parent pools, pristine state and every fallible binding
condition before one irreversible trim outside Group/Policy/UI/Row guards. Under the
surplus parent's mutex, release only available surplus3 and matching backing credit;
never issued/subreserved positions. Pin actual peer/R/Binding at that transition, not
just role. A later retry returns the SAME neutral owner OR the SAME already-bound
PreparedChain candidate, with its original deadline. It cannot regain trimmed positions,
switch same-role peers or invoke a new pilot/group-claim constructor. Existing
`into_chain(self)->Result` can lose an already-built chain if install_workflow refuses;
the operational builder must instead return/stage that SAME candidate on refusal.

Root preparation outcomes must distinguish `NoCompleteOwnerRemains`, retained-neutral,
retained-bound and validated. A proven unpublished preparation may safely destroy all
actual backing while preserving registration/g; that is not a current HIGH-path defect
or a refund of an advertised agreement. After canonical retention/promise/role binding,
retryable refusal must return SAME ownership. Entered ambiguity or Work retirement alone
never proves no publication. Preserve the existing registration/negative staging rules.

With otherwise idle current32-position packet tables, neutral6/6 leaves26/26 ordinary
positions; LOW binding returns only incoming3 (29/26), HIGH only outgoing3 (26/29).
The twelve Action claims still leave20/32; native ParentClaim leaves31/32. These are
position counts, not a promise that ordinary byte capacity or another native owner exists.

Both endpoints must own current RootRoundPreclaim, native ParentClaim and original paired
browser envelope before signing Request/Grant. One generation retains one intent/agreement.
A new public capability cannot be minted merely because the old observer vanished.
No transcript/Hello deadline starts during inert capacity waiting.

## New signed objects: exact proposed grammar

Body7 is `AdmissionControl`. Its header appends ONE `put_bytes(payload)` to the existing
SignedObject header. All payload integers below are unsigned big-endian; all raw fields
have exact stated widths. Payload common prefix C is11 bytes:
`format=1:u8 | operation:u8 | admission_profile=1:u8 | epoch:u64`.
Scope, issuer, Ed25519 public key, message ID and issued/expires remain in SignedObject.
Targets are derived from the closed operation, never a free forwarding instruction.

| Op | Raw fields after C | Payload bytes |
| --- | --- | ---: |
|0 Query|target32, query_kind1 (protocol or availability)|44|
|1 Support|target32, exact Query message_id32, supported_features:u16|77|
|2 Availability|target32, exact Query message_id32, local_join32, local_key_ref32, public_incarnation32, generation8, advert_serial8, available1, neutral_layout1|189|
|3 Request|LOW/HIGH DeviceIds (64), joins (64), key refs (64), incarnations (64), generations (16), LOW nonce32, capability_length:u16, exact signed HIGH Availability bytes|317 + capability length|
|4 Grant|targetLOW32, exact RequestDigest32, independent HIGH nonce32, commit_by_wall8, validity_contract:u16=1|117|
|5 Commit|targetHIGH32, RequestDigest32, GrantDigest32, RoundRef32, LOW nonce32, HIGH nonce32|203|
|6 Busy|target32, RequestDigest32, bounded retry_hint_ms:u16|77|
|7 Status|target32, RequestDigest32|75|
|8 Terminal|target32, RD32, GD_present1, GD32, R_present1, R32, decision1, reason1, accepted_wall8, result_digest32, observed_state1|184|

Unknown enums/bits, inconsistent presence flags, nonzero absent fields, trailing bytes,
wrong fixed lengths or cross-scope nested capability refuse. Present random incarnations/
nonces may be all zero; zero is not an absence sentinel. All generations/advert serials
are nonzero and checked against overflow. A retry reuses the exact signed original object.

With the current encoder, SignedObject overhead for this single payload field is282 bytes
without a policy pin,318 with one. This includes domain string's length, both version/kind
bytes, Scope's own per-array lengths, three fixed identity fields, wall times, the payload
length, outer unsigned length and signature length/64 bytes. Consequently:

| Object | Maximum bytes, policy present |
| --- | ---: |
|Availability|507|
|Request (317 + exact507-byte capability payload)|1142|
|Grant|435|
|Commit|521|
|Terminal|502|

`capability_length` must equal the exact nested object's actual canonical length, at most512.
The existing one-helper endpoint RoutePermit is392 bytes with a policy pin; a largest
ToHelper/HelperDelivery envelope is2+4+1142+4+392=1544 bytes (direct1148). These are voice
envelope bytes, excluding outer transport framing and simultaneous encoder copies.
Use a closed2048-byte Body7 control-object ceiling; Body8 keeps the existing per-kind
confirmation/8KiB SDP plaintext plus AEAD-tag bounds. Preserve the48KiB envelope maximum.
Vectors must pin all minimum/maximum policy forms before allocating production buffers.

Body8 is `AdmittedEncrypted`: an explicit admission-profile byte followed by the existing
encrypted-header field grammar, with its handshake-width field containing RoundRef32,
then the existing bounded ciphertext field. Body9 is `AdmittedReady`: profile1 plus the
existing ReadyHeader grammar (handshake-width field is R), sequence and SDP profile2.
The new Body tags/profile bytes participate in signed bytes and encrypted AAD. Existing
Body4/5/6 do not become aliases and cannot mutate an upgraded round. Discovery may still
observe legacy presence without authorizing a writer into the admitted round.

## Capability freshness and compatible helpers

Query uses its sanctioned-RNG SignedObject message ID as challenge. Support/Availability
must target and echo that exact live original query. A fresh Availability can be a hint
only until native verifies current presence, scope/policy/epoch and the live query owner.
There is at most ONE current query authority per exact (peer,scope,epoch,profile,query-kind).
It has a checked native monotone issue ordinal. Installing q2 irreversibly supersedes q1
BEFORE q2's Source publication; an old retained q1 owner/response cannot overwrite or revoke
q2's accepted observation merely because q1's signature/TTL still passes. A response must
resolve the exact current query; a missing query never creates one. Support and availability
queries use separate namespaces/gates. Support ordering cannot close an endpoint round or
claim a free slot. Old query/Source owners retain their original payment through actual return.
Random incarnations have no ordering: never prefer lexical order or arrival timestamps.
A different incarnation needs fresh query-backed evidence (or a changed current proven
join/key), and closes the previous remote admission observation. Initial round creation ALWAYS
checks its own current exact incarnation/generation and stored intent after all waits.
Remote replacement after a valid exchange cannot be instantaneously known; any remaining
orphan is bounded by the original deadlines and can never recreate the new local session.

HIGH Availability certifies actually held neutral/native/browser capacity at publication;
it does not promise that every later requester wins. Its target/query/incarnation/generation
and expiry are included in Request's exact nested object. HIGH verifies its own signature,
targetLOW and current local generation, so it need not retain64 signed capability copies.
LOW Request itself is LOW's signed reservation statement, emitted only under its actual
current prepayment fence. Grant and Commit finally validate both endpoint tuples.

A relay-only helper answers protocol Query/Support using current membership and the actual
RoomAuthority. It joins no call, reserves no endpoint slot and receives no endpoint key.
Support is bound to group, exact policy pin, room/profile, epoch, target/query and expiry.
Only authenticated supported features establish compatible new-tag forwarding. Existing
presence/ordinary kind28 support alone does not. No response/timeout means unknown or
unreachable, never authenticated unsupported. Direct connected-roster probes are bounded;
they cannot dial caller addresses or recursively ask another helper.

Endpoint and helper selection intersect support with actual current roster/peer binding,
connected routes, forwarding consent and RoomAuthority. Absent policy pin remains direct-only.
Every new-tag Source/route entry needs a typed profile validator, fresh native authority and
the SAME original attempts/deadline/keepalive through helper replacement. Replacing B with D
cannot mint another Source packet or refill its three submission attempts. No media relay
or endpoint-key distribution to a helper is implied by this control-forwarding contract.

## Agreement, key derivation and deadlines

`RD = SignedRequest.hash()` and `GD = SignedGrant.hash()` use the existing exact canonical
signed-object BLAKE3 hash, including message IDs, wall contracts and signatures. Neither is
a hash of an attacker-supplied logical JSON projection. Grant contains RD and no RoundRef,
avoiding a cyclic hash dependency. Commit must echo both original nonces and exact RD/GD.

Canonical agreement A is EXACTLY503 raw bytes, in this order:

```text
agreement_format1, admission_profile1, sdp_profile2, suite1       4
group32, policy_present1, policy32(always present), room16      81
epoch8                                                         8
LOW DeviceId32, HIGH DeviceId32                                64
LOW join32, HIGH join32                                        64
LOW key_ref32, HIGH key_ref32                                  64
LOW public admission incarnation32, HIGH incarnation32         64
LOW generation8, HIGH generation8                              16
LOW nonce32, HIGH nonce32                                      64
RD32, GD32                                                     64
commit_by_wall8, validity_contract:u16=1                        10
```

Absent policy bytes must be zero. This new fixed agreement encoder does not alter existing
Scope encoding. `R = BLAKE3("mewtual/voice/admission/round/v1\0" || A)`; never caller supplied.
Contract1 preserves packet15s, skew5s, confirmation15s and existing post-Finish AwaitReady30s
bounds. It defines grant expiry/commit time, not an arbitrary remote timer extension.

New Root derivation transcript is existing Encoder applied to:
`put_str("mewtual/voice/pair/admitted/v1"), suite1, profile1, fixed(R), fixed(A),
fixed(LOW Ed25519 key32), fixed(LOW ECDH point65), fixed(HIGH Ed25519 key32), fixed(HIGH point65)`.
Its size is789 bytes. Native must validate both signed endpoint joins/key references and
the exact original local selected point before issuing it. Browser validates the authenticated
new-profile input against its original selected key, committed R and native action/input tag.
Legacy binding/input shapes and KDF are separate decoders/producers, not fallback choices.

Use SHA256 over `"mewtual/voice/v2/admission/v1/transcript\0" || transcript` as HKDF salt;
info is `"mewtual/voice/v2/admission/v1/aes-gcm/{direction}\0" || transcript`.
The longest info is839 bytes, below1024. Direction derives solely from actual Root LOW/HIGH.
Both R and full A enter the keys. Changing only a control nonce cannot renew a legacy key.

Local inert preclaims have no transcript clock. First Request publication freezes its
original admission deadline; remote first observation maps wall freshness to a monotonic
bound at most existing25s remaining, intersected with capability/request/grant deadlines.
`commit_by_wall <= min(Request.expires, Grant.expires, referenced Availability.expires)`.
Exact retries, status/re-signed history wrappers and new Availability cannot refresh it.
Each local Commit consumption starts that endpoint's existing15s confirmation window ONCE.
Waiting for the other endpoint's acceptance consumes that same window. Later accepted
history after expiry never permits a fresh Derive or current RTC use. Existing phase
transfer rules alone permit the original post-Finish AwaitReady window.

## State machine and the final publication boundary

Operational free generation: Reserved -> RequestPending(LOW) OR GrantPending(HIGH) ->
CommittedNegotiating(R) -> existing protected confirmation -> post-Finish AdmittedReady ->
SDP/RTC -> independently indexed media. Both sides keep their SAME capacity until actual
phase transfer or collection. Busy before an admitted intent is only a scheduling hint.
After first signed Request publication, a generation cannot retarget or rebuild its nonce.

Only LOW initiates Request. HIGH signs Grant only for one retained matching Request under
its current capacity/generation. LOW validates Grant, consumes its original agreement once
and publishes Commit; it does NOT emit Hello until HIGH's accepted result is received.
HIGH consumes once on valid Commit and can prepare HIGH's own Derive/receive state; role
checks prohibit it from issuing LOW's Hello. HIGH cannot send Proof or post-Finish Ready
without the actual matching received causal input. This avoids normally generating encrypted
traffic before LOW knows acceptance. A duplicate, malicious or reordered pre-accept encrypted
packet can only enter the original committed R's bounded prepaid inbox; no entry without
that committed local record, no auto-created Workflow, at most the existing role packet
positions and exact duplicate/hash cache. A mismatch refuses rather than evicts current work.

Lost Commit/accepted reply recovers the SAME original committed round, or the original
permanent cancelled/expired refusal. Terminal accepted-versus-rejected is immutable; live/
expired/closing is a separate current observation. A local Capacity try-lock refusal before
commit retains the original preparation and does not become a false permanent rejection.
No expired Grant is reconstructed. Never refund an attempt after an ambiguous lower handoff.

The final Root step follows the existing SdpResultAdmission/VoiceSdpResultPublisher model:
all hashes/signatures/Clock calls and fallible allocation happen before a borrowed synchronous
step. Native then tries UI commit -> Coordinator -> original bootstrap Row/Slot -> Index/row,
validates actual selected/physical/generation/profile/ParentClaim and calls that step there.
The callback waits on nothing and invokes no external code. Heavy original candidates remain
in the enclosing Actor call and drop only AFTER native guards release. Actual Root group/
policy protection surrounds its own publication; native acquisition under that scoped loan
uses try-locks, so contention yields Capacity instead of an inverted blocking lock wait.

Root commit and native row installation are one prepared private publication tail. Prepare
the future native Row outside guards, transfer the SAME ParentClaim serial/preparation/payment
under Index and disarm its SAME marker; no new Begin/reservation/refund/recharge occurs.
Any entered but unstaged panic remains uncertain and recovers the original R/terminal record.
It cannot certify absence or mint another agreement. Exact accepted AND rejected native
terminal results are recovered before attempting a consumed token or a new publication.

The immutable RoundGate binds scope/epoch/endpoints/joins/keys/A/R and its own actual call/
peer revocation. It does NOT consult a later free-slot generation/cancellation floor.
After actual successful compaction transfers a peer into independent media custody, advancing
the free slot leaves that peer and other siblings live. While its original negotiation is
still attached, Cancel(g) closes only that exact pending/committed negotiation. The transfer
fence atomically disarms that cancellation association; a delayed Cancel(g) after indexed
transfer cannot close its healthy peer. Explicit CloseRound(R) closes that peer; End/lock/
epoch replacement closes all relevant rounds. Old finally handlers compare exact ownership.

Successful negotiation collection is different from closing the independent media round.
Its actual original workflow/input/history owners return, while a compact immutable R/
profile/context/revocation identity transfers with the existing media entry. A binds through
R; retain full A only while an actual derivation/source/input owner still needs it. The
media inventory must cover this compact gate (or an explicitly separate paid contribution),
with no Workflow/capacity/native Slot backedge. Old admission-result lookup after its explicit
release may be Unknown even while the separate known media entry still serves that R.

## Explicit local collection and bounded release links

No network peer ACK is required to clear local failed work. Local IPC32/version12 owns
Status/Collect/ForgetACK/linked successor operations on the SAME physical channel/profile.
`IntentRef = kind:u8 + digest32`, kind0/RequestDigest or1/RoundRef: its33 bytes are counted.
Positive committed selectors contain R only, so they do not need this extra discriminator.

| Local op32 kind | Raw request layout | Plain / cipher bytes |
| --- | --- | ---: |
|0 Status|12:u8, kind:u8, g8, IntentRef33|43 /59|
|1 Collect|same43-byte prefix, exact terminal_digest32|75 /91|
|2 ForgetACK|same43-byte prefix, collected_digest32|75 /91|
|3 RegisterNext|12,kind,next_g8,previous_g8,previous IntentRef33,collected_digest32,forget_ack_digest32|115 /131|
|4 LinkACK|12,kind,next_g8,previous_release_digest32,next_link_nonce32|74 /90|
|5 LinkStatus|12,kind,current_g8; generation lookup only, no fabricated IntentRef|10 /26|

Two closed reply shapes avoid pretending an intent exists for an idle successor:

- IntentReceipt (reply_kind0),173 plaintext/189 ciphertext:
  `12, reply_kind, g8, IntentRef33, state1, presence_flags1, terminal_digest32,
  collection_nonce32, collected_digest32, forget_ack_digest32`.
- GenerationLinkReceipt (reply_kind1),140 plaintext/156 ciphertext:
  `12, reply_kind, g8, observed_state1, flags1, previous_release_digest32,
  link_nonce32, applied_link_ack_digest32, stable_receipt_identity32`.

IntentRef kind0/1 is mandatory in IntentReceipt; no absent/zero RoundRef is inferred.
GenerationLinkReceipt has no IntentRef. Its flags independently indicate previous release,
nonce presence and latched LinkACK application; Closing is distinct from that historical
applied result. The stable identity binds exact incarnation/g/previous release/link nonce/
applied ACK hash with a separate domain. It excludes changing present-time observations.
All unknown enum/bits or inconsistent combinations refuse. Absent bytes are zero, but a
present zero random nonce is valid. Unknown has no collection/accepted/applied flags.
Header/class/exact length/physical Session checks precede counter and row admission.

Lost Grant means LOW may never know R. An admitted RequestDigest anchor can retire without
fabricating one. Likewise Commit can have succeeded while no Derive Action was ever issued.
Collection therefore has typed proofs `PreWorkflowCollected` and `WorkflowCollected`.
The first proves zero actual issued Actions AND joins ALL actual preparation/Source/Lower/
IO/SDK/output owners. Zero Actions is not zero transport work: a Grant/Commit lower flight
may still be outstanding. Lost reply does not prove unstarted. The second additionally requires all
actually issued result settlements, Plan-final/ACK, children and Claim/key/SDK/storage/Source/
Lower return. No fake empty Plan ACK, fake media entry, weak-count-zero or changed boolean
substitutes for actual return. Native subtree removal remains affine; rows drop outside
Index/Row guards before Root PlanActionHistory collection and its actual fixed-credit return.

Only after actual collection does Root release a fresh sanctioned-RNG collection nonce in
the authenticated Collected receipt. CollectedDigest binds profile/incarnation/g/IntentRef,
the original terminal result and that nonce. It is not a generation-floor projection.
Browser waits for original negative worker, SDK/IO/timers/cells/unsubscribe/codec frames and
the SAME authenticated receipt, clears its private old graph, then sends ForgetACK.

ForgetACK frees the heavy terminal anchor only after its actual remaining output/owner
destruction. During that destruction state is Closing. A small separately inventoried
`LastRelease` retains exact old g/IntentRef/terminal/collection/ACK identity. It is not
overwritten merely because a successor requested capacity.

RegisterNext(g+1) must carry that exact CollectedDigest and content-stable ForgetACK digest;
it can atomically finish a previously lost ACK. It cannot use a floor as a substitute.
Native reserves the successor under the SAME owner, retains LastRelease and returns a new
unpredictable link nonce. Successor remains ReservedUnlinked: no availability, Request or
Grant until authenticated LinkACK echoes the exact previous release plus that new nonce.
This proves the browser observed the successor reply and relinquished old retry authority.
BEFORE discarding LastRelease, atomically install a paid CURRENT-generation `LinkState`:
current g, original previous-release digest, link nonce, exact applied LinkACK content hash
and immutable Linked receipt identity. These are independently inventoried plain fields,
not a pointer back to the old heavy anchor. Exact LinkACK retries and LinkStatus(g) recover
the SAME Linked result even if its first reply was lost and no new Request/IntentRef exists.
RegisterNext retries also recognize this exact current link; a mismatch refuses.

Before LinkACK, old exact ACK/status retries recover LastRelease. After it, old predecessor
RPCs are Unknown/refusal and can neither affect the successor nor claim collected; current
link recovery uses LinkState instead. LinkState remains through that entire generation,
including later Request/Commit/terminal/collection. It cannot be replaced by an incoming
peer, an unrelated intent or a new link nonce. During a continued call, that generation's
explicit local ForgetACK plus the NEXT linked-successor transfer retires it. Terminal
old-session final-forget below is the separate no-successor path. Thus losing Linked's reply
never requires resurrecting an old LastRelease or unbounded receipt history. Initial unused
generations use their existing phase-typed actual-return path, not a fabricated Request key.

The private browser cannot send LinkACK until old actual frames have joined. It retains
one negative retry descriptor and the current link receipt, not every past handle. LinkState,
LastRelease, old-frame return and new-candidate overlap need a paid executable inventory
before implementation; no unused bootstrap slack is assumed. A lost LinkACK response after
LastRelease destruction and before the first successor Request is a mandatory regression.

A genuinely unused generation, with no published Request/Grant/Commit and joined actual
work, can use its existing actual unused-return path. It never fabricates an IntentRef.
An operational producer exposes one stable App/channel handle, not public per-round
capability handles. Private round cells cannot escape to arbitrary callers; old retained
private handles/owners keep their original payment and block reuse until actual return.
This is necessary for more than32 sequential rounds without growing a lifetime R set or
silently refunding caller-held closure graphs. Existing capacity-only public APIs are not
an operational conversion path.

### Terminal old-session final-forget and physical stop

Full End/lock/shutdown must not need a new generation, live positive grant or Actor restart
to finish. Preserve minimal terminal/LastRelease/current-LinkState history and its payment
through losable negative-history recovery until an explicit original executor final-forget.

The actual existing normal barrier is `RetainedIpcChannel.cleanup()` -> original
`native.collectChannel(owner,executor)` AFTER actual private SDK/IO/timer/frame/history joins.
Its preceding `settleSetup` closes effects while preserving cleanup routing; the browser
clears its original keys only after those joins and before that final collection call.
Status9/`acknowledge_setup` is SETUP acknowledgement only, not an End/round final ACK.
Native `bootstrap::finish(Closure::Channel)`/`Row::retire_channel` and the actual
`Registry::collect_retired` implement the current final trusted executor/physical path.

Extend that NEGATIVE final path to distinguish returned heavy owners from paid minimal
history; otherwise requiring history already forgotten in `admission_may_close` would be
circular. Resolve exact historical owner/executor/selected scope/profile, close first
effects, and verify actual heavy obligations returned. The explicit collectChannel request
cedes all OLD-session retry authority. Detach minimal history into an affine paid return
ticket, release UI/Coordinator/Row/Index guards, then destroy its actual owners/payment.
Only afterward finish physical retirement and actual registry collection. This is not
a native StatusClosed, missing row, socket close, expiry or generation-floor inference.
A successful collection reply does not replace actual last-owner/physical-idle evidence;
independent original Session/public-output holders keep their own fees until actual drop.
Lost collectChannel reply retries the SAME exact idempotent old-executor final-forget;
it cannot require g+1, new native permission, Source work or a network signature.
After an APPLIED authorized final-forget, old detailed R/result lookup may be Unknown;
the retry needs only its idempotent negative final-forget result. It must not reconstruct
ceded detailed history or infer Collected from its absence. This is different from a
physical stop WITHOUT final-forget, where the exact original detailed history is retained.

Service stop WITHOUT that broker acknowledgement has a different result. Existing
`stop_and_wait_for_physical` explicitly does NOT certify browser drainage, Plan compaction
or Workflow refund. It waits actual native physical holds and closes the listener; those
physical bytes/positions may return while a bounded PLAIN paid terminal/link remainder
and separate actual SDK/IO/native-row obligations still exist. The remainder must have
no Session/Row/Actor/Workflow backedge and remains charged; do not call it forgotten.
No receipt recovery over old private HTTP is promised once its listener/keys are gone.

REQUIRED IMPLEMENTATION GATE: provide a scoped negative-only recovery/final-forget handle
through the ordinary native bridge only if its actual original authority can be traced.
It must resolve the exact original executor/profile/selected scope/incarnation, not a
current CallGrant or arbitrary caller R. Existing collectChannel proves the normal final
barrier, but does not presently establish detailed receipt recovery after physical stop;
the additional bounded remainder store/bridge/authority and its Tauri permissions need
implementation/review before any positive producer. It exposes no Actor restart, Source,
signer, new capability, input/key/SDK or live permission. All outstanding actual obligations
keep their OWN original owners and payments. Full remainder capacity may backpressure;
there is no silent TTL/floor/socket-close eviction or uncounted tombstone allocation.

Only a separately proven, explicit trusted executor-destruction/final-abandon ticket may
cede retry authority without client ACK, and only after actual all-owner return. Service
stop itself is NOT that ticket. Do not infer SDK/OS return from WebViewDestroyed unless a
genuine existing lifecycle guarantee proves the exact obligations. If no final-abandon
proof exists, retain the negative obligation. Tests must pin receipt recovery after the
physical stop, exact foreign-executor refusal and final payment return after genuine ACK/
abandonment; physical-idle is tested separately from history-forgotten.

## Scheduling, rate and storage parameters to pin in tests

LOW chooses from actual current higher DeviceIds using a Root-owned cyclic cursor, after
the last attempted peer, plus bounded cooldown. HIGH chooses valid pending lower requestors
by its own cursor. Arrival order alone is not fairness. Only one active intent per generation.
Strictly increasing LOW->HIGH waits prevent cycles; a busy intermediate refuses rather than
holds another incoming agreement while waiting on a higher endpoint.

Maintain an actual Root-owned active-peer index, at least keyed by (scope,peer DeviceId),
with exact profile/join/key/R and pending/committed/indexed/closing ownership in its value.
LOW selection and HIGH final Grant AND Commit recheck it. A healthy indexed peer is skipped;
new R inequality or a fresh advertisement never permits another PC/audio entry for it.
Changing that DeviceId's join/key/profile requires an explicit replacement transaction:
fence the old round first, join its actual Root/native/SDK/Source/media owners and only then
replace its exact entry. Stale finally/removal compares the original R/allocation, never
just DeviceId. Failed or Closing entries also block reuse until actual return/collection;
other peer selection continues. Reserve/update the pending index under actual Actor
publication, so simultaneous preparations cannot both pass a stale free observation.

Candidate parameters: at most64 proven peers (existing presence bound), at most8 pending
incoming Requests/one per DeviceId, each separately prepaid2KiB before its canonical1142-byte
copy; no eviction of an admitted live entry to service a newcomer. The active agreement has
its separate anchor. Refillable admission buckets are separate from existing Source attempts:
process burst8/refill1 per250ms, group burst4/refill1 per500ms, peer burst2/refill1 per2s.
Definite Busy cooldown500ms; timeout cooldown2s; advance the fair cursor on every attempted
peer. These are initial design constants, not presently enforced or a promise under permanent
resource pressure. Fairness tests must establish progress for admitted stable3-8 peer sets,
including one silent peer, and another group's measured progress under sustained load.

Rate debit is at first new signed Request publication; local pre-publication Capacity/probe
failure creates no committed-round allowance. Busy may consume rate work, but no Action/
committed round. An admitted original Request keeps its original deadline/digest; no peer
can refill a spent packet's three attempts by changing helper or querying status. Expired
ungranted queue entries join/drop their actual owners; generation changes make their old
local target unusable. Held actual transport/SDK/Root owners may legitimately block reuse.

These buckets belong to the original Root runtime/group OriginLimits owner, outside
LocalJoin, watcher, room/profile controller, public incarnation and free generation.
Group/process and authenticated original DeviceId keys exclude those replaceable labels;
replacement cannot replenish tokens. Idle eviction cannot recreate an exhausted bucket
before its retained refill horizon. Helper/transport churn cannot reset origin work.
Use Clock-based nonzero refill periods (250/500/2000ms), with bounded wide-integer elapsed/
period arithmetic and preserved remainder; peer1-per2s must NOT call existing integer
Bucket.spend with rate0. Test subperiod steps, backward samples and near-u64-limit jumps.

ALL new controls use the existing cheap pre-auth transport budget plus authenticated
requester and original-issuer Setup limits before retained copies/crypto/sign/route work.
Add shared control-service budgets, independent of Request publication: candidate process
burst64/refill8 per250ms and group burst16/refill2 per250ms, with fair group scheduling.
Query, Support, Availability, Busy, Status, duplicates and re-signed Terminal cannot bypass
them. Responses additionally need a bounded permit for the authenticated exact request,
target/scope/profile, maximum response bytes and SAME original Source submission. Invalid,
nonmember or unsolicited input cannot trigger an amplification response/signing oracle.

Cache at most one current query authority and its exact signed response per key. Candidate
total bounds are64 query records PER GROUP across all rooms/kinds and256 per process, with
separately measured2KiB payments before owning copies. No unbounded map per random ID/nonce/
incarnation. Superseded/expired records stop authorizing first and release only after actual
query/response/Source owners return. Full caches refuse new work without evicting live state.
These occupancy bounds count ACTUAL retained owners, including superseded/deferred records
outside the current lookup map. Their affine count credit follows the record until actual
return, not map removal or observer cancellation. Prepay q2 before superseding q1; if all
records remain owned, refuse q2 without manufacturing another uncounted query/cache entry.
Exact retries coalesce into the SAME paid entry and remaining attempts/deadline, without
re-signing or new Source budget. A new status query may get a fresh signed wrapper only
after all service/response-byte admission; it retains the SAME immutable terminal outcome.
Flood each control-only form and replace watcher/controller/profile/incarnation/g repeatedly;
assert bounded signing/copies/cache/Source work and measured progress of another group.

Exact original local negative cleanup has a different gate. Status/Collect/ForgetACK and
current-link historical recovery remain reachable after End/lock/epoch/registration closure
and after maintenance shutdown, using exact physical/profile/provenance and retained
payments; they must NOT require a live positive CallGrant or observer.is_current(). They
cannot be stranded by exhausted fresh-network tokens. A dead registration permits only
negative/history progress, never a new signature/round/availability. RegisterNext and
LinkACK that enable successor availability are POSITIVE: they require original current,
uncancelled call/watch/epoch/physical ownership. Historical duplicate acknowledgement of
an already-applied link is separately noncreating. All variants remain coalesced/bounded
under their original prepaid Control/task owners, including service shutdown recovery.

Retain the original owner-based retirement/collection handles before the Actor mailbox or
maintenance disappears. Shutdown may finish those SAME negative owners and their actual
drop certificates without recreating a live Actor, registration or workflow. A stopped
mailbox is neither proof that old transport/SDK work returned nor a reason to abandon its
charged cleanup. No fresh network signature under a dead old epoch is implied.

## Proposed ownership APIs and implementation files

All types below are opaque native Rust owners, not renderer DTO constructors. Existing
RootAdmissionSession/ParentClaim APIs stay separate until the operational profile selects
the new branch. The primary must agree final names and signatures before implementation.

```rust
// catcoms-sync::voice_endpoint / voice_v2::admission: actual Actor owns cursor/identity.
fn prepare_voice_round_candidate(&mut self, grant: &Arc<CallGrant>,
    session: &AdmissionSessionObserver, generation: NonZeroU64,
    rpc: &AdmissionRpcPermit) -> Result<CandidateOutcome, VoiceError>;
enum PreclaimRefusal {
    NoCompleteOwnerRemains(VoiceError),
    Retained { original: RootRoundPreclaim, cause: VoiceError },
}
fn reserve_voice_round_preclaim(&self, grant: &Arc<CallGrant>,
    session: &AdmissionSessionObserver, generation: NonZeroU64)
    -> Result<RootRoundPreclaim, PreclaimRefusal>;
enum RootPreparationOwner {
    Neutral(RootRoundPreclaim), Bound(PreparedAdmittedChain),
}
struct RootPreparationRefused { cause: VoiceError, original: RootPreparationOwner }
// Private to the actual LocalJoin/verified agreement. SAME owner on every refusal,
// including after decomposition/install_workflow preparation, never old naked into_chain.
fn bind_round_capacity(original: RootPreparationOwner, agreement: &VerifiedAgreementOwner)
    -> Result<PreparedAdmittedChain, RootPreparationRefused>;
// private actual-Actor methods, after route/membership/profile verification:
fn accept_request(&mut self, input: VerifiedAdmissionRequest<'_>,
    original: &mut RoundAdmissionState) -> Result<PreparedGrant, VoiceError>;
fn prepare_commit(&mut self, original: &mut RoundAdmissionState,
    input: VerifiedCommit<'_>) -> Result<PreparedRoundCommit, VoiceError>;
// Clock/hash/signing/allocation finished; borrowed step cannot cross an await.
trait VoiceRoundCommitPublisher {
    fn publish(self: Box<Self>, step: RoundCommitAdmission<'_>) -> Result<(), VoiceError>;
}
impl RoundCommitAdmission<'_> {
    fn matches(&self, original: &AdmissionSessionObserver, generation: NonZeroU64,
        round: RoundRef) -> bool;
    fn commit(self, native_publish: impl FnOnce(&CommittedRound))
        -> Result<CommitReceipt, VoiceError>;
}
// Native only. Retryable refusal RETURNS the SAME affine original, including any
// already-built candidate. A plain VoiceError must not drop/refund the signed Grant's row.
enum ParentPreparation { Held(ParentClaim), Built(NativeRoundCandidate) }
struct ParentPreparationRefused { cause: VoiceError, original: ParentPreparation }
fn prepare_parent_commit(original: ParentPreparation, round: &PreparedRoundIdentity)
    -> Result<NativeRoundCandidate, ParentPreparationRefused>;
// NativeRoundCandidate installs under the original Slot/Index inside the scoped step.
// Canonical Slot stages returned ownership before exposing retryable refusal. Permanent
// revocation enters Closing and destroys it outside guards; it never silently retries.
// Terminal/collection stay opaque and phase-typed; missing/expired is not collected.
fn terminal_status(&self, original: &AdmissionSessionObserver, g: NonZeroU64,
    intent: IntentRef) -> Result<TerminalObservation, VoiceError>;
fn collect_failed_round(&mut self, original: &AdmissionSessionObserver, g: NonZeroU64,
    terminal: &TerminalObservation) -> Result<RoundCollection, VoiceError>;
```

Owner map: Root (`voice_v2.rs`, new admission codec/state, presence/admission, chain/capacity,
packets, crypto authority/actions, source/helper/endpoint ingress and transport) owns signed
context, role, nonce/RNG, round state, KDF input, Source and terminal/collection enforcement.
App Actor owns typed paid queue/publication loans. Native handoff owns profile pin, exact
Slot/ParentClaim/physical Session, UI/Coordinator fences, compact selectors and subtree return.
Browser owns original key/preclaim, authenticated codecs, distinct KDF consumer, actual
SDK/RTC/timer/frame joins, private history compaction and release-link ACK. Root `/root`
coordinates these interfaces; no separate service or agent implicitly owns a missing bridge.

## Inventory gates before any producer can be enabled

Positive local v12 selectors replace the legacy105-byte selector with version+R32=33 bytes.
Existing child prefixes105/122/123/138/154 become33/50/51/66/82; largest ACK is82+80+16=178
ciphertext bytes under272. Begin uses the actual committed parent RowId, not legacy
logical-zero creation. Header union admission must remain necessary-only; after AEAD it
selects exactly one version/profile and resolves original row/R/selected authority.

Propose an independently paid16KiB agreement/terminal anchor before owning/signing bytes;
this is a candidate allowance, NOT a measured guard pass. It must include retained exact
Request/Grant/Commit/Terminal bytes (max2600), nested capability507, decoded agreement503,
all Arc/Box/map/cursor/round-gate/witness backing, source/control/history and negative/release
overlap, canonical encode/hash/sign scratch and current callback/finally frames. Pending
request2KiB records and helper-support/cursor records have their own measured bounded fees.
Existing3KiB registration,4KiB actual Actor RPC and8KiB native worker are separate; never
borrow them twice or move a backedge into a plain payer. ParentClaim keeps its4352 bytes.

The actual compiled graph tests must compose neutral ReservedSlots/splits and Root2KiB,
Ready4KiB, Sourcebook16KiB, full AppCommand/Envelope and physical Control12KiB. Previous
Sourcebook16,360/16,384 and Control12,244/12,288 have almost no free space. Keep new large
records behind already-paid affine/Arc owners and lend only scoped references; merely
putting a Box behind an uncharged pointer is not a fix. Measure intake/signing/encoding/
queue/final publication, both role consumes, return and successor-link overlap BEFORE
allocating/publishing, including unpolled future, early refusal and unwind paths.

No Root limit increase is assumed. If measured accounting requires one, review that local
change separately and keep per-packet/action/attempt limits fixed. The unexplained hosted
native media-capacity refusal must not be hidden by raising limits.

## Focused implementation/acceptance order

1. Canonical signed/new-tag/profile/503-byte agreement and789-byte transcript vectors,
   exact1142/1544 maxima, rejection variants and unchanged legacy golden vectors.
2. Neutral6/6 reserve and SAME role transfer under incoming-only/outgoing-only/full-byte
   pressure, all allocator identities and actual destruction barriers; no second group claim.
3.31 pre-ACK ordering, cancelled-before-profile, ACK-first, lost replies, old native support,
   capacity-only late upgrade refusal, and unused versus intent-bearing generation cleanup.
4. Actual Actor candidate and Request/Grant/Commit: bidirectional/simultaneous joins,
   crossed intents, LOW-only Hello, lost Grant, lost Commit/accepted/refused replies,
   duplicate/reordered encrypted input before acceptance, wall skew and monotonic expiry.
5. Scoped UI/Actor/native publication barriers at every side of lock/End/epoch/remount;
   forced retryable parent preparation refusal after Grant preserves SAME stamp/4352-byte
   fee/preparation through retry, with no new Begin or recharge;
   same accepted/rejected terminal after consumed tokens; no stale receipt renews Derive.
6. Local preworkflow/workflow collection, including a parked real Grant/Commit lower flight
   with zero Actions: Collected/Returned/stamp/fee reuse stay blocked until actual return;
   held actual SDK/Root/Source/Lower/IO owners,
   lost Collect/ForgetACK/RegisterNext/LinkACK replies, old RPC arrivals after transfer,
   floor gaps, different incarnation restart and more than32 sequential returned rounds.
   Also accept q2/new-incarnation then deliver still-fresh q1/old-incarnation: neither the
   admission observation nor an established round may be replaced/revoked by that stale query.
   Lose LinkACK before/after LastRelease destruction, then End/lock/service shutdown with
   no future g. Service stop WITHOUT final-forget must preserve/recover exact accepted/
   rejected/link history through the reviewed negative bridge. APPLIED final-forget with
   a lost collectChannel reply instead requires exact idempotent NEGATIVE final-forget
   retry; old detailed lookup may be Unknown and is never an absence-based collection proof.
   Hold actual SDK/IO/
   native owners, refuse a foreign executor and assert separate real fee/position return.
7. Real TCP A-C forbidden through B, B joining no call, replacement D, mixed old/new helper
   profiles, fair3-8 peer matching and another group progressing under bulk/flood load.
   Repeated availability after A-B is healthy must create no second A-B round/PC; include
   absent C, simultaneous replacement, changed join/key and parked old media/SDK return.
8. Full integrated suites and existing original two-peer bounds, then real WebView RTC,
   audible capture/playback, member media relay and two-machine/physical acceptance.

No Rust/TypeScript/runtime tests were run for this documentation-only proposal. Its numbers
are source-backed byte calculations and proposed memory gates, not implementation evidence.

## Owner agreement and design review

The primary native/browser handoff owner (`/root`) has approved this proposal for STAGED
implementation under its explicit gates. Pure codecs/neutral backing/profile work may
proceed; no positive producer or activation is approved until cleanup/recovery authority,
closed enum/wire allocation, complete inventories and runtime acceptance pass.

The implementing agent's own read-only adversarial design review/re-review reports no
remaining blocker/high/medium after corrections. It challenged lost LinkACK recovery,
affine owner-preserving refusal, zero-Action transport return, old-query/new-incarnation
ordering, duplicate healthy peers, shared rate/control-cache ownership, negative cleanup
after End and cancellation/media isolation. The primary additionally required precise
Unsupported semantics and the no-successor terminal escape. The short re-review cleared
those directions; its low acceptance-wording ambiguity (physical stop without finalforget
versus lost reply after APPLIED finalforget) is corrected above. This review is not a
memory-layout pass, implemented protocol, physical result or blanket rollout approval.
