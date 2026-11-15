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

## Namespace6 and original pin custody

The prerequisite combined governance/native constructor now has a separate
[implementation design](CHAT-V2-GOVERNED-NATIVE-STORAGE.md). It is not built;
existing native3 still refuses governance4. Its startup-only Selected3 handoff
must complete before group/Source creation, and its exact native tuple and
first-torn recovery proofs must be paired before runtime use.

Namespace6 needs its own critical handler and versioned authenticated format.
Founder `GroupPinData` and pending Legacy `CapturedOwnerProfilePin` are different
provenance and cannot be relabelled. Store the accepted genesis anchor and its
accepted-invite provenance, bound to the actual vault/group/Profile and covered
original pending join. Invite-specific request/KP facts are separate from the
stable group pin. On same-Profile rejoin/reopen, reuse the SAME original pin
ciphertext and `V2GroupPinId`; resealing identical anchor plaintext changes the
stable outbox context and is forbidden.

Freeze the critical catalog declaration, first-declaration fence/claim/seed,
pin-index records and pending-JOIN grammar together before writing them. The
combined configured opener must handle governance4, member outbox5, joined pin6
and native registry7 before any old inspector/cleaner runs. The present native
v3 `CURRENT_LAYOUT` parser does not provide that composition. Preserve all
unrelated namespace bytes in sorted declarations and every JOIN rewrite.
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
