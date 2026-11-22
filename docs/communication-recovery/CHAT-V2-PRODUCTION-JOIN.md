# Production Chat V2 join: next vertical

Production design, not an implemented owning join or activation approval. The
initial isolated [DATA codecs](CHAT-V2-JOIN-BINDINGS.md) are under validation;
their design/diff reviews are clear after corrections. Chat V2 remains OFF.
PR #128 connects warm local saves to the renderer; it does not
communicate with a second member. The existing joined-source coordination
decisions remain binding.

## Required outcome

Founder A admits B and C using real MLS and a covered governing Add. A then
stops. B saves an ordinary message once and C receives the same signed original
through a permitted real transport path. They recover history after restart,
including edits/deletions, without per-message founder certification. Missing
parents cause bounded catch-up, never invented authority or an automatic fork
winner. The ledger keeps original actions; projection hides replaced/deleted
values while authorized admin audit retains them.

This requires a live founder source as well as a joined source. The current
`UnboundV2GenesisNode`, generation-1 member lane and one-slot/epoch-zero archive
remain unchanged until their explicitly versioned successors exist.

## Signed protocol bindings

Use independent, domain-separated V2 codecs, not a wrapper containing a valid
Legacy `InviteToken`. Legacy invite/request decoders must refuse the new bytes,
and a Native V2 endpoint must refuse every Legacy admission request before SDK
Add. Communication policy (peer-to-peer/dedicated) is not protocol mode.

The proposed small signed **invite binding** contains:

- the complete genesis anchor: group, founder key, Profile, founder Admission,
  tenure, genesis Header and state commitments;
- inviter key, one-time invite nonce and expiry;
- immutable communication-policy commitment and versioned routing-plan commitment.

Policy/routing bodies travel with the pasteable invite but are separately capped
and must match these commitments before use. They contain route descriptions,
not an embedded Legacy invite. No connection hint is permission to scan/dial.
The [initial invitation packet](CHAT-V2-INVITE-ISSUER.md#invitation-packet)
implements the bounded binary envelope for the warm founder. This profile
carries a TCP reservation with zero certified contacts.
The initial accepted out-of-band invite is the trust decision; signature
verification or a downloaded self-signed Profile alone is not one. A nonfounder
inviter carries the already pinned genesis anchor. An existing same-group pin
conflict preserves both inputs and refuses repinning.

The signed **join-request binding** names the exact V2 invite commitment,
genesis-anchor commitment, joiner key, retained request ID, domain-separated
commitment to the exact KeyPackage wire bytes, and the distinct OpenMLS
KeyPackage hash reference. The latter is the existing Add/`V2JoinFacts` field;
it is not `blake3(raw KeyPackage)`. The actual SDK boundary must independently
compute/check BOTH against the same package. Full KeyPackage validation must bind that same key, group and invite
credential. Do not mint another package when a reply is lost. The request's
separate domain/version prevents reinterpreting its signed fields as Legacy.
Also retain the distinct MLS credential digest in the request and compare it to
the Add origin and actual package at the SDK boundary.

The signed **Welcome binding** names the same invite/request/KeyPackage/anchor,
actual governing Add Header and new Admission, exact Welcome bytes, both
KeyPackage bindings, original public routing-plan/policy commitments and a
SEPARATE commitment to the exact post-Add sealed routing-transfer bytes. The
sealed transfer cannot be frozen when minting the invite. Distinct field/hash
domains prevent its substitution or confusion with the public routing plan.
Its signer is the actual authorized admitting
device, which may differ from the inviter or forwarding endpoint. Verify its
authority through the independently pinned complete governing prefix and match
the genuine MLS outcome; a valid device signature alone is insufficient.
The DATA linkage check requires `AdmissionOrigin::Add`, matching subject,
KeyPackage reference and credential, and the exact Admission Header body/kind/
group/Profile/tenure. Welcome signer must equal that Header issuer. Current
Core requires the pinned Profile founder as governing issuer; an ordinary
member Admission does not grant it. Founder-Admission substitution is refused.

Codec framing is length-prefixed byte fields and big-endian integers from
`catcoms-wire`. Semantic domains are `mewtual/chat/join-invite/v2\0`,
`mewtual/chat/join-request/v2\0` and `mewtual/chat/join-welcome/v2\0`, each followed
by u16 version2. The common envelope contains its own distinct
`mewtual/chat/join-binding-envelope/v2\0` tag, semantic field and64-byte signature,
with exact EOF. Each complete envelope is capped at1024 bytes before parsing.
Semantic IDs hash `mewtual/chat/join-binding-id/v2\0` followed by the complete
semantic frame, so signature variants share an ID but never signature validity.

The complete anchor has its own tagged/versioned canonical encoding. Payload
commitments use distinct kind-specific `mewtual/chat/join-<kind>/v2\0` tags,
then u64 big-endian byte length and the EXACT bytes, under BLAKE3. Each body
commitment refuses empty or greater-than512KiB input before hashing. This is a
format/work cap, not a paid network/SDK resource grant. Public routing plan and
sealed routing transfer are different kinds. The OpenMLS KP reference and MLS
credential digest are supplied/checked in their existing SDK domains, never
computed with this wire-commitment helper. Expiry is checked when admitting;
an exact successful Welcome may legitimately arrive after it. Diagnostics must
not disclose bearer nonces, raw request/key material or routing bodies.

First implement these bindings as bounded DATA codecs/signature checks. Their
constructors/decoders expose no Source, Current, native credential or trusted-pin
constructor. The existing Legacy bytes remain unchanged. Do not register or
activate the V2 transport until the actual owning join vertical exists.

## Retained join request and private Welcome candidate

Sync now builds a JoinRequest from an explicitly accepted founder anchor and
one actual MLS KeyPackage. The request contains the exact wire digest,
OpenMLS reference and typed credential digest. The private provider snapshot
and original packet remain together for exact retry. The API leaves the
selected contact, mounted accepted pin and durable pending join to the host.

The wire frames use distinct length-prefixed
`mewtual/chat/join-request-packet/v2\0` and
`mewtual/chat/join-response-packet/v2\0` tags followed by u16 version 2.
The request carries the signed Invite, signed JoinRequest and exact
KeyPackage. The response carries the signed Welcome binding, the MLS Welcome
and the sealed routing transfer. Each field is length prefixed, separately
bounded and decoded with exact end of input. Signed binding verification
precedes use of request identifiers or payloads. The joiner charges the exact
response frame, including tags, length fields and the encoded binding, against
its original exchange limit before entering the Welcome SDK.

On the native founder, `check_received_join_request` verifies both request
signatures and the package wire commitment. It compares the complete
incoming Invite with the linked, verified signature recovered from the
original mounted journal. It does not accept a caller-supplied journal token.
The original Current and file checks bracket authentication. The paid read
gate then validates the exact MLS package, including its credential, MLS
reference, and lifetime against the founder's original clock. This keeps a
signed request with a malformed or expired package from reaching a future
durable reservation. SDK admission repeats validation at entry. The owning
transaction spends the nonce only through durable Add coverage.

The joiner checks the complete verified genesis anchor and the current
replay tip at its own Add. It validates the signed response and both
payloads before spending its one SDK attempt on a private provider restore.
The Welcome must select the original package and produce the same group,
subject, credential and epoch. The strict V2 routing decoder then compares
the original commitments, actual Add parent and epochs, exact policy and
fixed L0 slot. It verifies the policy against the actual joined MLS owner.
Exact completed retries reuse the private candidate after fresh prefix
checks. The candidate exposes public facts and has no Legacy conversion.
The native founder job now drives the retained Add through covered Head2 and
installation. It returns the original response only after a fresh check of
the selected head and archive. A later delivery of the exact same request
reuses the original response. Rejecting a different request still requires
the original nonce fence. Network delivery and the joined native owner
remain required for production.

The response carries a bounded public proof of the original genesis and
first Add. The joiner compares it with the accepted anchor and replays the
exact Admission and state before opening Welcome. A truncated or unrelated
proof leaves the original private KeyPackage available for a valid retry.

The private request owner can seal its exact KeyPackage provider and request
against the original pending join and joined pin. Reopening matches the
complete signed invite from that pin and checks that the retained provider
owns the same package.
An owning journal must still prove whether Welcome entry was possible before
resuming SDK work.
A successful Welcome's private graph, policy, routing secrets and verified
binding can be sealed to that same original request for later native
installation. The completed outcome reuses its original ciphertext on retry.
Its independent reader authenticates the original pending ciphertext, replays
the signed delivery, and checks the restored closed MLS graph and routing
transfer against it. Mounting and installing that graph require separate
native ownership.

The warm founder continues to use its existing ordinary-member lane after
installation. Before signing a new action it checks the selected Head2,
the exact archive and the same verifier's governing Header and epoch.

## Namespace6 and original pin custody

The [combined governance/native constructor](CHAT-V2-GOVERNED-NATIVE-STORAGE.md)
implements the startup-only Selected3 handoff before group or Source creation.
The combined handler requires namespaces 4, 5 and 7. The ordinary native3
handler still refuses governance4. Namespace6 now has a separate joined-pin
record and declaration grammar. The record binds the first signed invite and
pending join to the vault and root. Exact ciphertext identity keeps the
original pin stable across rejoin. The combined catalog parser preserves
namespace6 bytes in existing JOIN rewrites.

Namespace6 needs its own critical handler and versioned authenticated format.
Founder `GroupPinData` and pending Legacy `CapturedOwnerProfilePin` are different
provenance and cannot be relabelled. Store the accepted genesis anchor and its
accepted-invite provenance, bound to the actual vault/group/Profile and covered
original pending join. Invite-specific request/KP facts are separate from the
stable group pin. On same-Profile rejoin/reopen, reuse the SAME original pin
ciphertext and `V2GroupPinId`; resealing identical anchor plaintext changes the
stable outbox context and is forbidden.

The startup declaration prepares a seed over the exact predecessor and
candidate catalogs, their governing, outbox and native files, and the first
empty pin index. Its staging path holds those original files and preserves
them on refusal.
The explicit joined storage opener classifies this declaration before the
older inspectors and carries the same selected file through their checks.
It can finish an authenticated interrupted declaration or open an already
selected joined catalog. The mounted index preserves the first pin ciphertext
for each group and checks the held index when reading pin context. The first
pending-JOIN record covers the private request and the one exact index
insertion. The original warm owner can select the insert and release the
exact signed request bytes while keeping the private package. Cold opening
can complete the same insertion and verify the original request. The warm
owner verifies the delivery before covering possible Welcome entry, then
seals and covers the exact successful outcome. Cold opening independently
recovers that outcome and prepares Core's initial selected-history candidate
from the same signed delivery. Core advances only after the mounted storage
rechecks the original selection. A possible entry with no outcome stays
historical. Other pending-JOIN transitions and native runtime installation
remain open.
The joined storage owner can exchange the exact selected JoinRequest on an
already authenticated connection to the peer named by the signed invite.
It keeps the original connection and vault cancellation through response
receipt. The fixture uses the connected transport harness and the actual
native founder responder. The request kind selects separate transport size
limits before the decoder allocates either body. Certified dialing and
desktop dispatch remain open.

Unknown/ambiguous remnants are preserved and refused, never cleared as debris.
Predecessor tests must use a real prior binary with positive open controls.

## Founder operation and whole-node handoff

The MLS exact-receiver pair handoff and registered completed-result view are
implemented under final validation. They provide no Root save/Source/native
admission. [Ownership boundary and remaining integration](CHAT-V2-ADD-OWNERSHIP.md).

Retain one actual founder Add operation under its original request, selected
parent and native/Source custody. The existing independently paid
`PreparedTextCurrentAdd` candidate is an input, not covering save or Header
authority. Cache the genuine SDK outcome once. Retain separate actual governing
Header signing custody and cover the candidate SDK snapshot, invite ledger,
governing records and selected archive before exposing Welcome. A late refusal
or lost observer resumes the retained operation without another Add/signature.
Crash policy must distinguish unexposed work from any possibly published result.

Move the SAME original founder device/provider/transport/event ownership into a
closed governed runtime. No raw mutable SDK getter, runnable Legacy Server
conversion, second node or replacement transport is allowed. Preserve existing
native key admission, Source closure, SDK/physical drainage and payment order.
The live founder ordinary-member source selects the real governing prefix after
Add; it must not pretend that head is empty genesis. Legacy dispatch must not
write into this runtime.

## Joined source and durable selected history

Keep the source beside its one actual joined node in canonical Native Sources.
Use `take_v2_joined_join_facts` once on a genuine fresh Welcome. Replay the full
pinned prefix before the own Add, then match own Add to that exact consumed
KeyPackage/credential/Welcome. Postjoin authoring additionally follows supported
finalized local transitions, not numeric epoch equality alone. Restored nodes
need a separately reviewed cold reconstruction path; do not fabricate fresh facts.

Use the real `V2GeneralPrefixVerifier` selected-history append data and original
scope proofs. Retain selected records, contradictions and old-parent evidence in
durable paged storage before hot-cache eviction. Never reconstruct an opaque
known-parent witness from decoded IDs. A cache cap is not a lifetime-message cap;
missing/corrupt/busy backing is not authenticated absence. Neutral or multiple
MLS changes need an explicit reviewed rule; current refusal is not completion.

The actual joined fence orders every transition/removal against final issuance.
An admission that wins retains its original end through IO; removal that wins
prevents issuance. A1 saved/uncertain originals stay historical across A2 rejoin;
they cannot become new tokens or be silently discarded. Reuse the existing
two-phase native member save rather than adding a parallel writer.

## Acceptance before enabling

Require deterministic tests for stripped/downgraded/cross-root bindings, exact
lost-Welcome/KP retries, every Add/pin/archive crash cut, both removal/admission
orderings, stale A1/A2/A3 tokens, delayed old-parent siblings beyond4096 records,
missing/corrupt archive pages, persisted contradictions and actual cold reopen.
Use at least three real peers for founder-offline delivery, then the planned
20–50-member bubble/epoch/restart/load matrix. Physical multi-device evidence is
separate. No newly decoded DATA or in-memory mock can stand in for these results.
