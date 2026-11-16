# Actual private V2 founder producer (source checkpoint; validation paused)

This source slice creates the actual signed V2 genesis data from ONE original newly created MLS
group. It is not a covering save, Current, selected public head, positive projection authority,
root-distribution protocol or App/native activation. No App factory/head/archive format changes
belong here. The source is connected through the actual Sync creation witness and typed codecs;
compilation/tests/builds remain paused and all eight authored regressions are UNRUN. Read-only
design and actual-diff reviews are clear for this private DATA-only boundary. No executed
acceptance follows from this document.

## Actual source and negative custody

`ChannelSync::found` is the sole production caller of the opaque
`VerifiedFreshChatAdmission::from_fresh_creation`, immediately after real `ServerGroup::create`.
`new`, restored epoch-zero groups, raw Profile bytes and current rosters cannot create it. The
existing original founder predicate checks actual group/full signer/active local leaf and original
epoch authenticator. The new producer also requires epoch0, one current member, actual local
designated committer, no v1 owner tail, no documents/ledger/commit/key/debt state, and no active
subscriptions/pending membership/network publication work. Refusal never truncates retained state.

Absent/nonoriginal/Welcome/advanced fresh sources are rejected BEFORE the member-count/committer
helpers' SDK roster observations. Inventory emptiness predicates are O(1); those separate actual
MLS observations may traverse/clone SDK roster state and are not part of the64KiB record invoice.

The original fresh token gains a PRIVATE physical identity and owned negative watch. Its opaque
founder observation keeps only weak identity and receiver, not the cancellation sender. Moving
the original token to its native owner does not renew it; removing it from this Sync makes this
slice's checked freeze refuse. Dropping it cancels the observation. Nothing is serialized and
existing fresh attempt/wire/snapshot bytes remain unchanged. Matching an authenticator alone is
insufficient: a restored byte-identical group has no matching original token/physical identity.

A separate private local-initialization observation captures exact label/revision/required state
and any ORIGINAL reservation identity+negative observer. It accepts only an unmanaged untouched
private source or an Active, unbound, alias/recovery-empty private initialization. A live runtime,
retirement/initializing marker, changed scope/revision, cancelled/replaced reservation or required
missing label refuses. It grants no scope binding, Current or publication permit. The observation
is checked again before signing completes, before freeze and before any host DATA borrow. The
future host order is original creation fee -> actual create -> reserve scope -> stage private Active
label with its ORIGINAL reservation -> prepare/freeze -> archive/current/head barriers. Actual
`PrivateManagedServer::reserve_scope` initially stages Initializing; the existing explicit private
Active step remains necessary before preparation. There is no generic rebind after preparation.

## Exact producer result

`owner_v2::PreparedFounder` is a non-Clone opaque prepared object. Its constructor accepts only
the actual current Sync, never a decoded Profile/snapshot, scalar source/creation label, synthetic
roster or caller-supplied success boolean. The object retains both original negative observations
and original MLS authenticator plus private typed public records:

1. Sign `owner_profile::ProfileSpec` with the SAME actual `MlsDevice`. Its public creation label
   is BLAKE3(raw `mewtual/chat/founder-creation/v2\0` || u32_BE(group length) || group || full
   founder32 || original opaque fresh attempt32). This label is DATA, not fresh provenance; the
   attempt is already a domain-separated hash, no private key/raw provider/epoch secret is exposed.
2. Create the exact `AdmissionBody` with that ProfileCID/full founder and Founder{creation}.
3. Create `Frontier::empty` under (ProfileCID, exact Admission BODY CID). The sole current slot
   has that admission/subject, tip BODY==Admission, count0 and that context's exact empty root.
4. Encode canonical `owner_state` sequence1/Admission/epoch0 with Profile's virtual genesis
   parent, `virtual_genesis_state(Profile)` as previous, and the exact admission BODY CID.
   The image never contains its own future header CID.
5. Sign sequence1 Admission header with the actual SAME device and resulting state digest.
   Re-run full `StateFrame::verify_genesis_binding(Profile,body,header)` and actual-source checks.
   Only then derive the selected initial tip HEADER identity from this actual signed header.

The checked consuming freeze retains these exact originals, encodes Profile/Admission/header DATA
frames and borrows its existing canonical state image, with empty frontier/selected-tip data. The
borrowed host view repeats original source checks and checks each retained encoded representation
against its original, so a same-CID invalid signature cannot be mistaken for that representation.
It returns signed/unsigned DATA only; no saved receipt,
new fresh capability, mutable Server or actual publisher escapes. The eventual actual App host
must prepay before preparation, retain original native/creation custody and cover the exact bytes
before exposing them to the protocol/UI. This slice deliberately supplies no such App facade.
Existing v1 facts/tails/finite checkpoint are never decoded or relabelled as V2.

## Fixed allocation/work invoice

Each group input is bounded1..256 BEFORE copies/signing. Existing public codecs bound each
Profile frame494, Admission490, header634 and state image9244 bytes. Keep one state image in the
prepared object; the frozen view borrows it rather than making a second image. Typed records have
at most three256-byte group allocations; the empty frontier is its fixed64 Option-hash array,
not a peer-sized heap collection. Signing/semantic/body-ID/genesis scratch is independently below
4KiB at any one step. The three other retained public frames total<=1618 bytes, plus original
identity/watch/scope/fixed metadata. A conservative64KiB logical private producer allowance pays
these named simultaneous objects; it is an invoice for the future actual host, NOT an existing
global pool debit, RSS/allocator bound or arbitrary SDK/provider/active Server heap admission.
No key-package parser, current wall clock, new RNG draw, networking or filesystem I/O occurs.

## Authored regressions (UNRUN)

- Real MlsDevice/ChannelSync creation produces signed typed V2 Profile, exact Founder body,
  canonical state/header binding and one empty context frontier with exact selected tip.
- Byte-identical actual SDK snapshot restore/new cannot prepare or borrow an old prepared/frozen
  source, despite matching group/signer/epoch/authenticator and arbitrary copied public DATA.
- Original token take/drop, changed/replaced/retired scope/reservation, different full signer,
  actual Add/epoch/roster change and any retained history refuse before freeze/export.
- V1 staged owner facts cannot be silently converted, and public codec decode/signature validity
  cannot construct the actual producer source.
- Repeated exact freeze/borrow preserves original signed/frame bytes and IDs, never resamples a
  signature/source/RNG or installs runtime authority. Bounded DATA decode/mutation checks remain.

Actual private producer observation is distinct from recipient public replay: historical peers
rely on an independently pinned non-equivocating finalizer's authenticated creation/transition
attestation and public semantics. No public signature proves private MLS execution or global
freshness. Selected-chain/cold archive, contradiction/control reservation, durable negative
closure, positive issuer, key/join lifecycle and native/UI/physical acceptance remain open.
