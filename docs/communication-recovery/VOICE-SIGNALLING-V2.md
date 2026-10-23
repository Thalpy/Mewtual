# Voice signalling v2 decision

18 October 2026. Reviewed design; crypto, codecs and native packet/resource ownership are in
implementation. **The live V2 protocol is not enabled or accepted.** Forwarding stays
disabled until this contract, its implementation and compatibility tests pass review. Existing
call lifecycle corrections are separate in [CALL-MAPPINGS](CALL-MAPPINGS.md).

## Scope and alternatives

Repair A-B-C room discovery and targeted setup with A-C forbidden, including replacement helper D.
A helper can be a current authorized server member without joining the room. It receives no
participant tile, microphone consent, recording authority or endpoint signalling key by helping.
This design protects SDP/ICE from a relay-only member with its legitimate server MLS state.
WebRTC still provides the media route and DTLS-SRTP; no new media encryption, relay service,
post-compromise security or audible/NAT acceptance is claimed here.

| Option | Assessment |
| --- | --- |
| Forward the legacy call payload | Reject: signature lacks recipient/session/replay binding; plaintext SDP/ICE becomes visible to helpers. |
| Encrypt with a server exporter key | Reject: a relay-only member can derive that key. |
| Isolated two-endpoint MLS | Possible, but requires fresh isolated providers, signed identity delegation, exact two-leaf admission, application-only receive, ratchet bounds and idempotent Welcome/confirmation ownership. Existing duplicate/snapshot APIs copy the whole provider. |
| Signed HPKE envelopes through installed Rust backend | Possible with a separately reviewed caller-RNG API. Current high-level sender APIs draw hidden backend randomness; neither HPKE nor isolated MLS supplies the required injected Rust seam unchanged. |
| Signed native delegation plus WebCrypto endpoint keys | Preferred candidate: uses established WebView primitives, avoids a Rust provider fork and MLS Welcome/storage lifecycle. It remains a new authenticated application protocol requiring review. |

The installed backend randomness is not alleged to be weak; its injection API is the limitation.
HPKE also leaves replay, downgrade and application binding to the caller ([RFC 9180, section 9.7](https://www.rfc-editor.org/rfc/rfc9180.html#section-9.7)).
The selected browser primitives are specified in the [W3C WebCrypto Recommendation](https://www.w3.org/TR/2017/REC-WebCryptoAPI-20170126/).
Native challenges, message identities and join incarnations use `catcoms-rt` RNG/Clock seams.
Browser key generation uses WebCrypto's managed randomness. No general native signing oracle,
Ed25519-to-DH conversion, handwritten primitive or test-RNG production feature is introduced.

## Ownership and supported versions

A native voice controller represents the unlocked WebView mount. Prepare/activate revision CAS
and exact End follow the mapping coordinator's lifecycle pattern, with a distinct voice owner:
optional mapping capacity must never become voice-session authority. One active call owner binds
UI/account generation, exact actor incarnation, stable group, room and fresh 32-byte join nonce.
There is no call restoration after lock/restart. Current membership is checked at reservation,
signing/queue admission and final delivery; original issuer authentication is independent of the
helper transport identity. Self-removal ends the call; remote removal retires that peer's pair,
joins, queued work and key-confirmation authority. Re-add requires a fresh challenged join.

V2 uses a new request kind with direct requester/transport binding. The original legacy kind's
authentication is unchanged. New V2 calls accept only V2 endpoint state: every legacy heartbeat,
bye, offer, answer and ICE delivery is excluded before it can mutate presence or negotiation.
There is no automatic legacy fallback on missing capabilities, bridge changes, timeout or disabled
indirect routing. Old clients do not join the protected endpoint session. The local UI reports
V2 unavailable; silence is not proof of an old version. Only an authenticated explicit response
or local engine failure supports an “unsupported” diagnostic. A feature rollback can disable forwarding while retaining V2 direct
handling, or end the call; it cannot convert a live V2 join to legacy. Restart starts a fresh V2
negotiation and never restores old keys, SDP, joins or capabilities. No persistent pair secret or
per-peer requirement is discarded, because none is persisted. A deliberately older binary opens
its own separate legacy session; current V2 peers refuse it. Mixed-version fixtures must pin this
refusal rather than count it as interoperability.

## Canonical objects and endpoint keys

Use the repository's injective fixed-width/bounded-length codec. Decoders consume all bytes,
reject unknown versions/kinds/suites and enforce bounds before allocation or crypto. A stable
group identifier is the hash of the exact native MLS group bytes, not a numeric UI server id.
Room is a 128-bit channel id; device identities are full 32-byte ids. Protocol/suite are explicit.

Native signs a typed `JoinDelegation` containing domain/version, immutable policy digest (or
explicit absence for direct-only LegacyUnverified), group, room, original device and
signature key, join nonce, suite, exact public key, key reference, issued epoch/time and expiry.
One key binding is allowed per join; replacing it requires a new join. Long calls renew the
30-second delegation every five seconds with a fresh native announcement sequence and issue/
expiry, retaining the exact join/key, pair transcript, direction allocators, replay state and
confirmation state. These renewal fields are not part of key derivation. Older renewal sequences
cannot replace a newer observation or extend their own signed expiry. The browser generates a
non-extractable P-256 private key. Public keys must be exactly 65-byte uncompressed points with
prefix 0x04 and pass validated WebCrypto raw import. Invalid points do not activate a peer.

The pair transcript encodes policy/group/room/protocol/suite plus both canonically ordered identities,
joins, exact public keys and key references. Its domain-separated hash identifies the handshake.
P-256 ECDH feeds HKDF-SHA256. HKDF salt/info bind the whole transcript with distinct labels for
each direction and purpose. Derived AES-256-GCM keys are non-extractable and limited to the
required encrypt/decrypt usages. Pin 12-byte IVs and 128-bit tags. IV is four zero bytes followed
by the direction's native 64-bit sequence in big-endian order; sequences begin at one, never
reuse and refuse overflow. Opposite directions have different keys. The suite cannot negotiate
downward. Unsupported WebViews refuse V2 with a useful diagnostic.

The native direction allocator belongs to the exact pair transcript for the lifetime of its
local join. Handshake timeout, failed confirmation, key-cache eviction and re-derivation cannot
reset it. Pair records cannot be evicted and recreated while that join is active; capacity refuses
new pairs instead. Retrying confirmation uses a fresh challenge and continuing sequences (or
cached exact flights), never sequence one. A fresh join/key is required after allocator loss.

Every immutable **pair-encrypted** packet header binds domain/version, policy digest, group, room, original issuer and recipient,
both join nonces, both key references, suite, handshake hash, direction, kind, fresh message id,
sequence, negotiation generation, issue time and expiry. Native returns **exact header bytes**
for AAD; JavaScript does not independently reconstruct them. The original member signs those
bytes plus the exact ciphertext. Pairwise AEAD is not a substitute for this signature: both
endpoints know the shared secret. Helpers/recipients verify the original signature and current
member identity, then exact scope. A helper cannot substitute its SDP/DTLS fingerprint.

## Single-use reservations and browser work

Native admits a typed crypto operation before key generation, import, derive, encrypt or decrypt.
Its opaque token binds controller/call/operation and bounded byte declaration. The process allows
8 active-or-uncertain operations, at most 4 for one group. A window-owned broker survives
component remount and retains permits until the actual WebCrypto promise settles; timeout only
revokes effects. Whole-WebView loss may prevent settlement, so native charges survive controller
retirement until a matching settlement or process exit. Exact stale tokens may settle their own
charge while locked, but cannot read state or refund a replacement. Exhaustion refuses new work
truthfully. This trusts the application WebView's reporting; compromised local UI is outside the
remote-member threat model and already controls plaintext/capture. No process-wide bound is
claimed from a JavaScript counter alone.

`ReservePacket` validates the current pair/owner and burns a new native sequence/message id.
It returns an opaque single-use reservation and immutable header/AAD with a five-second local
construction allowance. The broker freezes that header and the exact plaintext **before** crypto,
admits one encryption attempt, and caches the resulting ciphertext. Failed, cancelled or expired
encryption burns the sequence. Retrying reuses cached exact ciphertext; it never encrypts again
under the reservation. Native Submit atomically pins one ciphertext hash and returns the cached
immutable signed packet/result for exact retries; altered retries reject. Reservation retirement
does not refund unresolved crypto. Plaintext, keys, SDP and ICE are never logged or persisted.
Pending and completed retry entries share the 32-reservation count and group/process byte limits.
Retain exact completed packets/results only through their original 15-second signed expiry plus
five-second skew allowance. Expired/evicted ids reject; they never reconstruct an encryption or
reset a nonce. Live protective entries are not evicted to admit more work; capacity refuses.

Decrypt retains the engine/native/declared-byte charge through the exact native commit, including
its independent plaintext copy. Commit rejection or final owner retirement clears owned returned
plaintext buffers before rejecting the promise. The retained-window broker is versioned; an old
broker requires a page reload rather than allocating a replacement beside unresolved work.

All live V2 owners must share the process Runtime's logical wall clock. It advances with local
monotonic elapsed time and re-anchors on forward local wall observations, including while raw
wall time rolls backwards. Remote timestamps never advance it. Merely freezing at the largest
raw timestamp is insufficient: signed freshness could otherwise outlive monotonic replay/key
protection. A large correction can make our new objects unacceptable to other peers until clocks
align or a fresh process starts; no old call state is restored at restart. A first-seen object
can be issued one skew window ahead and remain valid one window after expiry, so incoming owners
allow at most 25 seconds of packet or 40 seconds of delegation protection from first observation.
No skew extends the local 15-second challenge or confirmation deadline.

## Freshness, confirmation and negotiation

Unpaired room controls use separate closed signed objects, not pair-encrypted headers:

- `Announcement` is the typed JoinDelegation above, with its native renewal sequence. It contains
  no recipient join or handshake and no SDP/ICE.
- `WatchChallenge` binds domain/version, policy digest, group, room, full watcher identity/signature
  key, fresh watcher incarnation, fresh challenge id/nonce, exact target identity/join/key reference
  and original issue/expiry. The watcher is not a participant and supplies no fake call join.
- `JoinProof` binds its own domain/version, policy digest, group/room, original participant identity/
  join/key reference, exact watcher identity/incarnation, challenge id/nonce and bounded expiry.
  Pair-confirmation signatures cannot substitute for it, or vice versa.

The native watcher capability is tied to the unlocked controller, actor/group and known channel
directory; it expires/revokes independently of call participation. WatchChallenges/JoinProofs are
targeted public controls, never admitted under room-broadcast permits. Consume proof only against
a matching live native challenge. There are at most 8 watcher capabilities per group, 32 pending
challenges per group, 64 across the process, 64 room-presence records per group and 512 process-wide.
At capacity reject new entries rather than evict live protection. Fair polling/rotation can revisit
omitted room evidence, but no complete global roster is claimed beyond those coverage bounds.

Signed room announcements expire by their original issue/expiry, not arrival time. They advertise
a join; they do not alone prove current liveness or replace an established peer. A native watcher
challenges the exact announced join with a fresh nonce. Only the native owner of that still-active
join signs the scoped proof. Presence expires relative to the watcher's challenge creation and
original expiry; duplicate/reordered proofs cannot renew it. A watcher is not a room participant.
Challenge state is bounded and never restored. Lock/removal revokes response and delivery rights.
Native observation ordering also survives visible-row expiry: accepting a newer proof retires
every older pending challenge from that issuer. Actor-owned join bindings retain renewal high-water
marks and exact keys through the latest delegation protection and active-pair pins, independently
of the sliding distinct-join rate window. Stale renewals may finish an already-issued challenge;
they cannot create a new one or regress a newer observation. Membership retirement synchronously
revokes issued evidence and removes pending challenges before re-add. Actual retained evidence
clones remain charged; a uniquely held row may transfer its charge atomically during refresh.

Endpoint establishment additionally requires encrypted key confirmation. The lower canonical
identity initiates with a fresh native nonce and transcript hash. The responder's role-specific
encrypted proof includes that nonce, its own fresh native nonce and the same transcript. The
initiator returns the role-specific encrypted final confirmation. Reflections, wrong-direction,
previous-join/handshake and unrelated nonces reject. Cache these exact packets for lost-flight
retries; a duplicate final flight returns the cached confirmation and never creates another pair.
The native join claims one confirmation owner per transcript. Its validated pending action keeps
chosen nonces through response reservation pressure or engine failure; exact input retry resumes
that action without a second decrypt. Outbound Submit checks the exact current action and native
reservation before signing and establishing in the same actor turn. Superseded Proof/Finish work
cannot sign. Native post-decrypt receipts are affine and bind the exact signed object, both leases,
expiry and retained byte credit; a caller-supplied header is never a confirmation receipt.
Confirmation, packet replay/delivery, cached flights and origin key pins share one irreversible
pair lease. An observed removal on any path cannot revive another path on re-add. An expired
unsigned preparation automatically advances action identity and reserves a continuing sequence
for the same validated nonces/plaintext; lost responses cannot strand progress. Its old packet and
uncertain engine charges remain with their actual owners until real retirement.
Initial confirmation has one 15-second monotonic window and at most three fresh Hello starts,
with continuing packet counters. Exhaustion requires a fresh native call join. This is an explicit
bounded initial policy, not automatic same-key reconstruction after timeout.

After fresh discovery admits a candidate, its native pair pin is independent of observation
refresh/expiry. A working established pair remains until a newer admitted pair successfully
confirms. Native call-local admission generations order promotion across watcher replacement;
successful promotion atomically retires older pairs/candidates and their delayed results.
Application SDP/ICE is held until establishment, within the receive bounds below. A valid newer
join replaces old state only after freshness/key proof; old bye names only its exact join.

Native owns outbound sequence allocation and inbound replay accounting per original issuer,
recipient join and handshake, independent of helper/connection. Use a bounded 128-sequence receive
window; duplicate packets may trigger an exact cached handshake response but no application effect.
Retired joins remain tombstoned through their maximum signed lifetime plus clock-skew allowance.
Limit four candidate joins per original identity per minute; exhaustion pauses that origin until
the window expires. New routes cannot reset it. No old packet is replayed after restart.

Negotiation identity is distinct from stable room and participant join: `OfferId` is the full
offer issuer plus its fresh native offer message id. An offer names itself; answers and ICE echo
that exact identity. Simultaneous offers cannot both mean “generation one”. Keep at most eight
early ICE candidates/16 KiB for an admitted offer, with its original expiry. Perfect-negotiation
glare uses the existing canonical polite role; abandoning an offer retires its pending answers,
ICE and continuations. Local ICE is associated with the exact local SDP generation/username
fragment; ambiguous or retired fragments are not reassigned to whichever offer is current. Exact join,
peer object and negotiation generation fence every queued callback and `setLocalDescription`,
`setRemoteDescription`, ICE, crypto and stats continuation before externally visible effects.
After each deferred inbound decrypt, an exact single-use native delivery token must be committed
again with the exact immutable AAD/sequence/ciphertext tuple (or native-agreed canonical digest).
Native compares it with that token's authenticated stored packet, then checks current actor
membership, UI/controller/call ownership, both joins, pair and mode. A token for peer B cannot
authorize successfully decrypted old traffic from peer A in the same local call.
An earlier event or delayed UI roster update cannot supply final native authorization. Only then
may the broker return plaintext for RTC application, which also checks its local peer/offer lease.

## Bounded helper routing

Start with direct delivery or **one authorized member helper** (two transport edges), which covers
the required A-B-C case. Longer live forwarding chains remain unsupported and must be diagnosed.
Never dial arbitrary hints, change immutable group mode or use a recent-peer redial cache here.
Reuse connected-only deferred work, keeping actor commands responsive and accounting work until
the lower transport completes. Membership/control and live setup retain priority over backlog.

The immutable signed endpoint packet is independent of route. Native separately issues a signed
short-lived `RoutePermit` binding packet hash, exact helper, final recipient (or room announcement
scope), group and expiry. A helper accepts only a permit naming itself from the original issuer,
authenticates its direct requester, and forwards only to eligible **currently connected** final
members. A final recipient never forwards it again. A helper cannot mint another issuer's permit
for a second helper. B-to-D retry mints a different route permit around the same immutable packet;
endpoint replay accounting and ciphertext never reset. Announcements can use room-scoped permits,
with finite final-member fan-out. Helpers receive no endpoint decryption key.

Use exactly three nonrecursive outer variants, each peer-bound under the new request kind:

| Outer variant | Required authenticated direct requester | Scope |
| --- | --- | --- |
| Direct | Original signed object's issuer | Exact local endpoint/watcher, or public Announcement |
| ToHelper | Original signed object's and permit's issuer | Permit names this helper and exact packet hash/final scope |
| HelperDelivery | Permit's named helper | Permit and packet name this final identity/join/watcher, or permitted public Announcement |

All branches verify original signature, current issuer/recipient/helper membership as applicable,
permit signature/hash/expiry and full group/room scope. Reject nested envelopes, self/helper/final
role confusion and a captured permit laundered through another requester. Room-scoped permits
admit **only Announcement**, never encrypted traffic or targeted presence controls. Helpers never
treat a final delivery as a forwarding request.

Indirect permit issuance, helper admission, queued forwarding and final forwarded delivery all
require the existing active authenticated `PeerToPeer` policy gate, binding its immutable digest
in objects and permits. Missing, persistence-pending/inactive, LegacyUnverified, Dedicated or
mismatched policy refuses indirect service without downgrade. Direct V2 may retain existing
current-member direct-call eligibility under LegacyUnverified; its absent policy is explicit and
cannot authorize forwarding. Dedicated remains unsupported. Revalidate policy and exact actor
after every deferred step; use `group_policy`'s existing enforcement, not an inferred UI mode.
Changing an absent policy binding to an active authenticated pin ends the old direct-only join
and requires a fresh delegation/pair; it cannot silently change the existing key transcript.

Helper ledgers key original issuer plus immutable packet identity and final recipient; changing
the permit, connection or controller does not restore fan-out/retry allowance. Duplicate admission
returns cached admission status with no new forwarding. Admit at most three total lower attempts
per packet/final edge, and count each fan-out edge. Retain entries through original expiry plus
five-second skew; failed/retired work stays charged through actual lower completion. No live
protective entry is evicted for capacity. Bound 256 helper packet/final records per group and
1024 process-wide, including cached outcomes; their bytes share the queue/cache byte ceilings.
Apply immediate-requester limits before expensive inner verification, then verified-original-issuer
limits and process/group byte/concurrency admission. A malicious sender's own submit counter is
not an enforcement point. The new path uses detached connected-only requests with lower-completion
accounting; legacy `notify` submission cannot be treated as retirement or recipient receipt.

Announcement fan-out uses native bounded rotating batches/cursors over current eligible connected
members, rather than always selecting the first four. Over subsequent fresh announcements, quiet
members beyond the first batch must get a turn while the route survives. Cursor state is bounded
per live issuer/room; expiry/removal retires it without replenishing a still-live packet's allowance.
The one-helper/connected topology and resource ceilings limit coverage; diagnostics must not claim
whole-server convergence where no contemporaneous eligible route or coverage exists.

Process-owned helper packet/edge/cursor records survive actor/controller replacement. One targeted
current-member edge may remain pending without a connected route; it consumes a record, no attempt
or dial, and can start only after a new proven connected-route snapshot. Four forwarding flights
per group and eight per process share the byte pool. A driver/worker/completion Flight is an
affine lower-work owner; cancellation cannot clear its active flag or release the charge. Retry
starts only after its actual last drop. Persistent admission acquires packet, every edge and any
new cursor charge before changing the cache or rotation. A native directory-incarnation lease,
separate from participant/watch consent, additionally fences queued effects. Permit expiry can
stop forwarding before the longer original announcement's replay protection expires.

### Narrow SDP capacity contract (PR 48 revision)

Before the first local SDP nonce or incoming positive admission, the confirmed native LocalJoin
must prepay a 192 KiB exchange owner from the revised512KiB native group / unchanged2MiB
native process pool. This local declared-memory policy does not change signatures, wire
formats, nonce/replay history, existing work limits or actual-last-owner retirement. One
group may now occupy one quarter rather than one eighth of that process pool. The browser's
separate256KiB group ledger is unchanged; native policy is not proof of browser composition.
Ordinary endpoint and at most two fixed helper origin records are retained first, without
spending or resetting traffic buckets. The same owner reserves one Crypto work lane, two
Forward lanes and two continuation slots from the existing combined 32-action limit. Capacity
refusal occurs before a nonce, ACK, replay bit or original-issuer quota commit.

The supported first exchange is deliberately narrow: one local SDP, one remote SDP, one Source
record and at most two actual lower holders, all bound to the native pair and one OfferId.
The first Offer pins initiator/responder role; crossed Offers refuse before positive admission.
This does not implement glare recovery. Helper selection is immutable and at most two current
members; direct-only scopes cannot prepare helpers. Actual lower custody can block a later
hedge but cannot consume protected reply capacity. Readiness expires after 30 seconds; the first
SDP begins an immutable 20-second work window. Exact retries cannot extend either deadline.

At the 8 KiB plaintext ceiling, the declared simultaneous peak is 187,966 bytes: local packet,
Source and two lower holders (74,686), remote packet, decrypt record and serialized engine/
verification scratch (89,666), retained encrypt record (10,286), native result (9,232), and
workflow metadata (4,096). These fit the196,608-byte reserve without refunding actual work.
All phase quotas count retained terminal records. Closing releases unused headroom only;
actual bytes, engine/forward lanes and continuation slots remain charged until custody ends.
Maximum native packet/Source/reply and pressured composition regressions pin this arithmetic.
Each lower holder includes 512 bytes for Root's route/authority metadata, reserved before an
attempt is spent. The route retains that same actual flight credit through cancellation and
completion; Root requires no additional ordinary-pool allocation to submit prepaid SDP work.

The copy reduction preserves both actual Lower lanes and inbound verification. Source work
borrows its original immutable packet instead of retaining another ciphertext clone. Canonical
signed-object/envelope parents allocate exact final backing capacity only after their children
finish; the outer request is encoded directly into its final frame. Source preparation and
transport therefore retain at most two large buffers per Lower, with unchanged fixed wrapper/
route payments. A private original-job completion view reuses that immutable packet's cached
hash; its small ACK verifier cannot hide additional full-SDP canonical hashes. Public ACK and
incoming verification still decode and hash independently. Compatibility/capacity tests compare
the previous canonical bytes and actual backing capacities across boundary and maximum sizes.

This Root-only composition does not pay the separately retained native channel, bootstrap,
confirmation history, browser phase or RTC owners. Readiness must still prove their complete
composition, including discovery/old lower custody; a smaller Root reserve alone is not desktop
activation or evidence that16 live pairs fit the provisional group ceiling.

This is enforced native capacity infrastructure, not completed SDP crypto-action factories,
browser-copy custody or RTC result delivery. Those integrations, exact historical receipts and
complete native/browser acceptance remain enablement gates. Live V2 remains disabled.
The native coordinator must also establish compatible fixed helper selections between endpoints
or refuse readiness/delivery honestly; one endpoint's selection is not the other's authority.

Initial defensive ceilings, not latency SLOs: 16 participants/pairs per local call; 8 pending pair
handshakes; 32 packet reservations; 48 KiB complete inner envelope, 8 KiB SDP, 2 KiB ICE;512KiB
declared native work per group and2MiB process-wide; 4 helper final recipients per announcement and 2 candidate
helpers per targeted packet; 3 total route submissions within a 15-second packet expiry. Presence
delegations last at most 30 seconds; verified liveness at most 15 seconds from challenge creation.
Accept at most 5 seconds wall-clock skew, without extending local monotonic challenge lifetime.
Per-original-issuer token buckets survive helper/connection changes: setup 20/s burst 40; room
announcements/challenges 2/s burst 8. Immediate requesters use the same setup bucket independently
of origin. Retired join tombstones share a maximum of 64 per local call (four per 16 peer slots),
256 per group and 1024 process-wide; replay windows/retired records count toward these ceilings.
Forwarding spends both origin and process byte/work budgets.
Oversize/exhausted work rejects before expensive decoding or crypto. These ceilings need load
measurement and may be lowered; package acceptance needs agreed service/latency targets separately.

Source counting is separate from each helper's final edges: one original Announcement has three
TOTAL source submissions across direct recipients and at most two helpers. A direct room ACK
confirms one recipient and cannot end whole-room work. Direct-recipient and helper cursors rotate
over fresh announcements. Targeted packets likewise have three total source submissions across
direct/B/D routes. A 500ms alternate-helper hedge becomes eligible subject to shared byte/flight
capacity and actual lower retirement; it is not guaranteed overlap for maximum SDP. No route or
capacity spends no attempt, while an already minted source/helper job conservatively spends its
reserved attempt even if later local preparation fails. Each overlapped flight owns a distinct
counter/credit; a late old completion or final drop cannot settle/clear the newer selected flight.

The retained SourceLedger core now implements these counts and protects packet IDs/hashes and
bounded cursors across actor/controller replacement. Only an actual LocalJoin/Watcher can issue
its private activity handle; exact join/key or watcher incarnation is checked even for first
admission and cached return. Source native-origin commit reserves persistent state first and
enforces issuer == this native device. Encrypted source activity additionally requires the exact
native PacketLedger's cached signed packet, current pair and current outbound action. Confirmation
supersession retires its prior action before publishing a replacement, without refunding sequence,
attempt or actual lower custody. The capacity tests use real native packet reservations/cache;
they do not prove browser/WebView crypto or end-to-end room dissemination. Watcher activity keeps
its full typed parent consent, so End/controller retirement can cancel queued work before another
actor turn. Native factories bind that parent before lending clones or constructing a Watcher.

### Connected transport admission and remaining integration

Kind28 admission and detached source/helper preparation/completion now exist as trusted Rust seams;
normal request dispatch and production actor scheduling still do not enable them. The strict
borrowed outer parser caps input before copying/crypto. Requester/process work and shared Crypto
bytes precede signatures/inner decoding. An affine ticket binds exact frame bytes, current actor,
member/policy generations and a five-second construction window, rechecked at final helper commit.
The independent actual-work credit survives ticket consumption through ACK signing. Declared
serialized-buffer accounting reserves five frames plus 4096 bytes, without an allocator-slack claim.
Outer signing transcripts are dropped before decoding. Current member/peer lookup builds one
membership set and unique claim index; unbound endpoints refuse before actor reconciliation.
ACK validation authenticates the expected current peer and the original envelope's responder role:
ToHelper permits HelperQueued/Refused only; final delivery permits EndpointAdmitted/Refused only.
Source preparation signs only the selected helper's permit on the originating Root. Source and
helper completions share exact driver/route/Root authority and ACK verification but retain separate
ledgers and settlement types. A helper ACK cannot settle the source's final edge. Trusted native
public-control intake/pumping now has a bounded queue with exact original native recipients and
atomic original allowance. Replacement watchers may seed fresh challenges from an authorized
candidate but cannot adopt an old cached ACK. Only native fresh challenge/proof validation admits
presence. Production dispatch/source scheduling, encrypted Endpoint delivery and UI V2 remain off.

Cold cache collection runs through a process-owned weak Runtime task and the sole actor's absolute
Clock wake, including while locked. It snapshots groups before taking Source/Helper/Group locks;
no resource charge callback enters a collector. Group membership/epoch/revision evidence and native
pair/replay/sequence tables remain intact. Idle confirming pins expire at the original 15-second
deadline; queued confirmation grants capture that deadline independently of collection. Outgoing
source deadlines are fixed at native reservation, so wall corrections cannot change exact-retry
identity. Actual lower/payload owners remain charged until final release; unknown WebCrypto work
is never collected by TTL. The production source/action scheduler must still consume capacity wakes.

Native endpoint installation must recognize a call grant in both the current actor/policy child
registry and the exact current directory room's child registry. Matching scope and a live foreign
lease are insufficient. Prepared encrypted ingress reserves retained/event bytes and checks replay
without changing it; its final native origin transaction must precede infallible replay/token/queue
publication and repeat exact local/pair owner checks inside Group -> Policy. The primitive and
rejection regressions exist; live endpoint admission still needs this transaction wired.

The new kind is 28; kind 8 retains its released transcript and behavior. Before outer signature
work, resolve the incoming Noise peer through a unique current proven bound device/peer relation
intersected with the fresh transport connection snapshot. Unknown/unbound peers refuse. Spend
cheap retained requester/process work before outer verification, then verify exact peer-bound
outer authentication and require its full device to match that relation. Both request and
response verification count as bounded work. Use Runtime effective time only for this new kind.
Do not reuse `proven_member_path` unchanged: it does not enforce the bound-proof bit or fresh
transport snapshot. No new V2 exchange promotes a self-signed descriptor into route authority.

An affine ingress ticket binds the exact raw inner-frame hash and byte length, actor/member and
policy generations and epoch. A small admitted frame cannot authorize a different envelope.
Pending driver work additionally captures a revocable exact route-generation lease: replacement
of a device/peer proof closes old work even while the old Noise connection remains open.

Validate current scope/routing and classify exact packet ID/hash replay before spending original-
issuer quota. Requester/process work is still charged for duplicates. Pre-reserve every persistent
packet/edge/cursor or endpoint-ingress allocation before committing the single new-origin charge
and its protected seen owner. A capacity refusal with no persistent owner spends no original-
issuer allowance. Nothing fallible may silently lose that owner after the charge; retain a
bounded terminal record through the original lifetime if a later step can fail. This reuses
existing packet/endpoint owners rather than adding a parallel global dedup service.

Native responses use a new domain binding the exact request metadata, original packet hash,
current expected full responder identity, actual responder Noise peer and a closed status.
An authenticated helper-queued ACK records helper admission only. The source's final-recipient
edge remains pending and rotates/hedges through another permitted connected helper after bounded
quiet-network backoff or helper loss, within the original three submissions/deadline. Late old-
helper ACKs cannot retire or replace the selected newer flight. After attempts are spent, keep
delivery unconfirmed and retain replay protection through expiry. Direct exact final ingress
evidence can confirm native admission; neither ACK establishes presence, decryption or media.
Empty/unknown-kind replies remain unavailable, never inferred support or downgrade authority.

## Required implementation and acceptance

Before exposing V2: read-only review this ADR, implement canonical codecs and native authority,
then real WebCrypto/broker/key-confirmation tests, bounded connected-only routing, actor/native
commands/events and UI V2 admission/legacy exclusion. Keep feature rollout independent of file
storage and media relay changes. Server snapshots contain no pair keys or replayable call state.

Pin every field mutation, altered AAD/ciphertext/signature, malformed/off-curve keys, wrong
group/room/identity/join/key/suite/handshake, reflection, duplicate/lost confirmation and nonce
burning/exact retry. Release each deferred crypto/RTC/native continuation after leave, lock,
removal, same-room rejoin, remount and whole-WebView retirement. Flood origin and helper budgets;
provider/connection changes must not multiply allowances. Exercise every legacy signal kind into
V2 state, including stripped capabilities and rollback; none may mutate it.

Real TCP acceptance requires A-C forbidden, helper B not joined, independent A/C joins, late join,
B loss/D replacement, duplicates/reordering and concurrent chat/blob/control. Assert exact
authenticated identities, same fresh participant set, bounded state/work and no forbidden edge.
Give B its legitimate MLS state and forwarded traffic and demonstrate it cannot open targeted
SDP/ICE; trace key ownership as well as observing ciphertext. Test actual native WebView crypto
support separately. Physical TURN/audio requirements in Packages 5A/5B remain gated: TURN carries
media, not room discovery ([RFC 8656](https://www.rfc-editor.org/rfc/rfc8656.html)); this ADR cannot
claim those observations from mocked RTC or local TCP.

Residuals: compromised endpoints/WebViews, browser-managed secret memory, no guaranteed RAM
zeroization, no signalling ratchet/PCS, metadata and ciphertext length visible to helpers,
conservative capacity loss after unsettled WebView work, bounded one-helper topology and failure
when no contemporaneous permitted route exists. These must remain explicit in UI/diagnostics,
canonical contracts and [the implementation ledger](CHAT-FILES-VOICE-ACCEPTANCE.md).

## Recoverable endpoint action contract (partial native/browser foundations)

The pending-authority prerequisite is now implemented as trusted Rust factories. Derive captures
the unchanged pair; confirmation seal captures action+unsigned reservation; Proof/Finish open
captures native token+admitting attempt. Hello open captures token+one stable per-pair accepted-
sequence floor. Advance it only after closed validation and a successful native transition, repeat
the fence at final commit, and keep newer waiters pending when lower progress arrives. Token and
action field owners revoke on consume/replacement, including in-place reservation changes.
Signing closes a distinct crypto lease while source/cached submit retains its original owner.
Five-second pending construction is separate from original packet/history lifetime; established
application packets never inherit the earlier confirmation timeout. There is no historical
conversion API. The combined action/receipt/OfferId queue and browser adapter below remain pending.

Use one bounded combined pending/committed/terminal action budget. Reserve each action's eventual
exact-result receipt and bytes before consuming an inbound token, signing ciphertext or mutating
confirmation/negotiation. A full receipt cache refuses before those irreversible effects; live
protection is never evicted. Pull returns a bounded batch and revision without dequeueing. Subscribe
and revision comparison must close the empty-pull lost-wake window. Actual pulled payload owners
retain their byte credits after token/cache removal; they must not strongly retain call custody.

Each action has private revocable authority for its actual pair pin and applicable outgoing
reservation/action or incoming token. PairSetup/CryptoFlight data alone is insufficient. Pending
checks bind exact native input/token/action serial; historical exact-commit receipts instead bind
the result fingerprint and repeat current call/room/member/pair/negotiation checks. Consuming a
token cannot strand an exact successful retry, and cached success cannot authorize retired RTC
work. Store terminal rejection receipts too. Expired action identities never reconstruct state.

The browser keeps a bounded surviving owner+action+version computation/result record. Unresolved
work is never evicted and changed input under one identity refuses. A lost native commit reply
retries the same computed bytes without another WebCrypto decrypt/encrypt or nonce allocation;
retain plaintext and its actual broker/native charge through that uncertainty. Distinguish IPC
uncertainty from authoritative rejection/retirement, and release no RTC plaintext before native
authorization. The current VoiceCryptoPair.open wipes on any commit-hook rejection, so it needs
this commit-aware recovery before the adapter is enabled. Mount-specific state cannot live in the
broker's captured native adapter across HMR. HMR retires the old owner rather than adopting work.

The confirmation-only browser registry is now implemented as an inert broker-v4 seam. It has
one combined 32-record cap, conservative input/output/hook bytes shared with ordinary work,
one caller-attempt promise separate from actual operation lifetime, and exact retry after
admission/commit uncertainty. Native replies must be closed and bind owner/action/version,
input tag and computed SHA-256 result tag. Decrypt/digest failures stay failed; temporary
queue pressure is retryable. Confirmation plaintext never leaves the broker, and accepted
receipts describe past transitions without granting fresh pair/presence/RTC authority.
Explicit owner/action retirement wakes observers while actual promises keep their credits.
Retired action identities stay bounded until owner retirement. Fresh HMR derivation uses the
retained broker's pair factory; v2/v3 retained brokers refuse rather than creating another pool.

The native IPC adapter is a REQUIRED FUTURE contract, not implemented commands. The inert native
action/engine seam models idempotent exact admission and permanent settlement fencing. Desktop
Coordinator registration/admit/settle currently compiles only for contract tests, with no
production IPC enforcement. Sync typed-action factories remain inert production Rust seams.
Only opaque LocalJoin-issued actions register, matched to the exact native call lease, scope and
shared process pool. Typed derive/seal fingerprints bind native inputs, scope, action serial and
reservation/AAD/plaintext; decrypt binds the exact signed-object hash. The native queue retains
at most 32 combined pending/terminal owners, with rotating non-dequeueing pulls of at most four.
First signing/closed-confirmation wrappers hold the same fence through actual protocol effects.
End closes unstarted actions, while admitted engines remain charged through actual settlement.
Native commit still must prepay/store its exact
success/rejection receipt and enqueue follow-ups before returning. Node tests simulate that
adapter and use real WebCrypto; they do not prove native/WebView integration. The general
Pair.open path above remains unused by calls and must not be used as the future adapter path.
SDP/ICE still require a separately charged result/dispatch lease, rather than exported copies.

Confirmation outcomes are closed typed protocol results and never go to RTC. Native enqueues
follow-up work and returns without awaiting crypto that needs the same two broker slots. Browser
RTC dispatch records its promise before awaiting and checks exact peer/offer generation after
each await. A native receipt proves authorization, not RTC/media completion. Encode native u64
sequences/action serials as canonical decimal JSON strings, converting directly to bigint.

Required regressions include one decrypt/native transition/eventual RTC dispatch after a committed
but lost reply; full receipt capacity with lost successful and rejected replies; stale cached
success after End/pair promotion/same-room replacement/remove-readd; private queued-action
revocation; saturated metadata consumers; and held actual pull payload after token retirement.

### Endpoint inbox/action integration proposal (next design review)

The first integration is restricted to confirmation. An authenticated small ConfirmHello may
wait for actual proof/derivation, but its header cannot create a pair. Limit initially to one
unconfirmed chain/group, including retired unsettled custody. Before ACK, prepay the workflow's
conservative peak transient buffers/action/result bytes and continuation slots. Keep long-lived
pair/allocator records and source cursors separately funded from the ordinary pool. Proof/Finish
must match actual native progress. Unknown SDP/ICE buffering is deferred until it has its own
reviewed progress bound; refusal alone with three retries does not solve delayed derivation.

The capacity primitive is implemented and reviewed: Resources::reserve holds one prepaid charge;
ByteBudget::Reserved loans acquire actual slots and return bytes only after actual final release.
Every loan retains the original reserve. Closing permanently rejects new loans and releases only
unused bytes; closed loan drops release their own remaining backing bytes and actual slots.
Closing is capacity-only, not evidence of settled work. Charges validate both the exact process
pool and group. Idle engine slots are never reserved. Fifteen focused tests cover full ordinary pressure, exact
group/process slots, atomic refusal, wrong pool/group, zero-byte engine custody and concurrent
charge/drop/close. Native action factories accept that explicit budget; one focused proven-pair
test exercises it under full ordinary pressure. Whole confirmation workflows do not use it yet,
and the saturated pre-proof full-chain test and inbox
starvation finding remain open. Reserve capacity alone does not prove crypto/packet authority.

Reuse the bounded endpoint inbox's original issuer+MessageId/exact-SignedObject classification for
encrypted originals too, rather than introducing an independent dedup/origin table. A retained raw
encrypted original would mean native custody only. Capture its exact current local join/key, native
call/room/member/policy owners and original expiry; neither raw intake nor ACK admits a pair, proves
decryption or permits RTC. Reserve all inbox/response bytes before committing original allowance.
This would let a Hello arriving before the recipient finishes native proof/derivation, or ICE before
its Offer, wait in bounded native custody rather than immediately exhausting three source attempts.
No received header may manufacture a pair/OfferId or allocate an engine action.

The sole Root would fairly revisit retained input. Actual native fresh proof admits a pair, with a
prepaid Derive action; acknowledged derivation permits native confirmation progression. Only when
pair/token/negotiation prerequisites exist and a combined action/receipt slot is reserved would
prepared PacketLedger replay/token publication occur. The exact inbox owner must authorize that
transaction without charging the original a second time. Known replay/invalid pair state should
refuse before first original charge where it can already be classified. Unknown prerequisites may
remain bounded raw custody through expiry; terminal rejection retains its existing dedup owner.
This refinement requires review against the atomic ingress contract before implementation.

Actions now have unique continuing native serials, opaque owner-bound identities, private pending
capabilities, bounded immutable Arc input custody and fixed original deadlines. Pull returns at
most four items without consuming them. The coalesced action revision/result owner is still pending. Reserve
eventual terminal/success receipt capacity before derivation completion, seal or token consumption.
Store exact input/result fingerprints; post-transition confirmation history captures the accepted
attempt, while SDP/ICE history also captures the current native OfferId generation. Native follow-up
actions enqueue without waiting for browser crypto. Action expiry never refunds uncertain engine
operations, resets a nonce/replay/serial, or permits an expired action identity to be reconstructed.
Production dispatch stays disabled until the queue, browser retry adapter and compatibility tests
are integrated. This proposal is not current implementation or acceptance evidence.
