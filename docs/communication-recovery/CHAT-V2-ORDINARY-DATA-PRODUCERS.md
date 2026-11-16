# First Create and Append DATA producers

Declared library DATA producers; all11 focused ordinary-data tests pass. The refined raw
signing, consuming negative handoff and Core issuer contract is recorded in
[the current boundary](CHAT-V2-FOUNDER-PROJECTION-ISSUER.md), which supersedes older
full-sign/replay descriptions below. This is not an operational Current, network decoder or
complete text-chat checkpoint. Root owns the real operation capture, payment, WAL, selected head,
complete indexes, cold continuation and projection/publication. Existing wire encodings and
default paths remain unchanged; the new encrypted format is not dispatched.

## Original source and ownership

`UnboundV2GenesisNode::prepare_create_data` accepts only a local stable token32, channel and
borrowed text1..65536 bytes. It derives the actual group, Profile, founder Admission, full
signer, epoch0, Realm and ordinary Grant/Basis internally. The supported existing P2P policy
must already be present; nothing initializes policy or enables member-mesh service here.
The operation uses the SAME retained runtime Clock/RNG and the closed SDK's read-only
`device.sign` and `group.channel_secret`. It never invokes an MLS application ratchet,
resets a decoder guard, changes provider rows or samples another S0/S1 SDK snapshot.

The original pending owner is retained before work. Private phases precede Clock, nonce,
signing, exporter and sealing calls; time/semantic nonce are sampled once. The actual signature,
canonical action and complete ciphertext are cached before later source checks. An ambiguous
partial stage refuses instead of silently restarting. A complete retry invokes none of those
five operations. Changed token/channel/text and equal-restored/foreign node/scope refuse
without replacing the pending owner. All loans recheck the same seed identity, complete original
scope/reservation observation, source context and initial graph shape. These are negatives,
not App authority or proof of payment.

One successful `take_for_handoff` moves non-Clone `DetachedV2CreateData` and records its exact
private packet ID. Preparation/borrow/take then remain terminal; a unit-only transferred check
cannot reopen them. Detached DATA retains no node/graph, key, observer, sender or Store. It
can retain exact bytes after node Drop for already-admitted negative repair. Root must move
the complete packet AND its actual fee into original retained custody before TxnA effects,
keep both on registration/IO refusal, and fund both residual metadata and payload until drain.
DATA permits neither a new signer, IO grant, accepted result nor native/lock-resume authority.

## Frozen encrypted grammar

Canonical plaintext, then existing `pad(512, 1048576)`, then XChaCha20-Poly1305:

```text
bytes("mewtual/chat/ordinary-envelope/v2\0")  # domain length34
u16(2)
bytes(actual group, at most256)
bytes(actual Profile ID32)
bytes(actual Admission ID32)
u64(actual epoch0)
u128(actual channel)
bytes(canonical SignedAction)
EOF
```

`bytes` is the existing u32-length-prefixed wire field; integers use the existing canonical
wire encoding. The action remains action/v1, Create with no reply and empty parents. Private
BootstrapSourceId, native request token, local scope nonce and vault identity are not placed
in this transferable plaintext. Root binds local provenance separately.

The actual channel exporter secret keys a BLAKE3 hash of the distinct29-byte domain
`mewtual/chat/ordinary-key/v2\0`, followed by fixed Profile32, RealmId32 and epochBE8.
The32-byte result is the AEAD key. It is never returned. A future recipient must bind EVERY
decoded field back to its independently selected source/action; this draft adds no receiver.
This format is never reinterpreted as `SealedOp`, Automerge or an MLS application message.

Create-only maxima: canonical action66057; plaintext66457; padded131076 including footer4;
ciphertext131092 including tag16; nonce24 gives131116 before Root debt framing. The NEW1MiB
logical producer invoice names872793 buffer bytes plus actual layouts and control overhead.
It includes requested Encoder growth, typed/request text copies, semantic/encoded overlap,
envelope, padding, AEAD growth/tag and fixed signing/export scratch. Source/SDK/host runtime,
Root WAL, index, page and additional output copies remain separately paid. Capacity/layout
assertions are source checks, not a caller-supplied payer flag or a whole-process/RSS bound.

## Unsigned intent before Append signing

Planning and signing are SEPARATE. `prepare_create_append_data` performs zero private signs:
it verifies each actual action representation, requires this initial Create/no-reply shape,
derives the exact AcceptanceRecord/one leaf/body, and uses the mathematical parent's OWN tip
and frontier. `ReplayMath::prepare_append_data` returns an opaque plan derived through the SAME
private delta/image helpers as existing signed replay. No caller HeaderSpec or current-snapshot
initializer chooses fields. `borrow_unsigned_data` exposes exact canonical signing bytes,
complete image, record/body and an intent commitment. The signing-byte encoder is shared with
the existing Header implementation; App must not duplicate serialization.

Root MUST durably reserve ONE exact unsigned child intent per actual selected parent BEFORE
`sign_reserved_append_bytes_data`. The original selected source, full complete-index result, original
A representation, exact signing bytes/image and intent identity are all bound. Parent-only
labels and CAS after signing are insufficient. Entered/ambiguous signing cannot replace its
child; retry/restart reuses that exact intent. A new parent may get a new paid attempt while
the original A/action/cipher stays unchanged. Native Stop/lock admission orders at this
irreversible boundary and cannot falsely report an already admitted action as unaccepted.

The node signs ONLY its stored opaque plan under an exact original weak plan observation.
The raw cut checks only original seed/scope/context identity and caches actual raw64 before
later failure. Header materialization/verification happens outside writers using that same
signature. Core's opaque candidate derives full signed `derive_successor` ONCE afterwards;
Sync no longer allocates and discards another next model. The resulting admission tip-header
is the NEW actual signed header ID. Raw-signed/complete retries cannot re-sign. Consuming
negative handoff preserves raw64 even Failed/HeaderNone, with no source/key callback or
fabricated missing output. The separate2MiB Append invoice covers model/plan,
verification, image, record, carry and retained output overlaps; the decoder/Create fee is
not reused. Math/signature DATA does not prove selection, cross-batch absence or acceptance.

## Required actual Root handoff and evidence

The real operational owner must retain the same source/SDK/node/scope/task/writer and separately
paid producer buffers. Validate actual Current and selected source first, release temporary
node-backed SourceRef/host locks before mutating the node's runtime RNG, then rebuild the SAME
source loan and recheck originals after work. No raw node/key/signer adopter is authorized.

TxnA's authenticated reachable token -> original action/cipher locator survives caller Drop
and restart. The accepted index's ActionID key alone cannot recover that native request.
Missing/corrupt/stale committed objects mean Incomplete, never Absent or new signing. TxnB
selects the complete accepted index, own frontier/Append/header, manifest and publication debt
under the exact parent/CAS before any positive projection/publication. There is no64/4096
lifetime message cap, historical fullmap, decoder-to-Current shortcut or implicit owner refresh.
Membership/epoch changes, live20/50-writer SDK admission and later-history governance remain open.

All11 genuine SDK-node DATA regressions pass, including actual Node-to-Core issuer composition;
the two independent unsigned/signed math controls pass in the Core focused run. They cover
maximum pad/cipher, independent KDF
roundtrip/separation, original SDK bytes, all five late boundaries, exact replay, changed/equal
sources, one-time/missing handoff, no SDK keepalive, two successive Creates, wrong signature,
foreign parent/scope, post-sign unwind/cancellation, missing result and interleaved own tips.
Their receiver-only reservation/resource fixtures are explicitly NOT App Current or operation
payment proofs. Actual Root durable-intent/sign faults, fee custody, native Stop, TxnA/B/index,
cold reopen and complete backend/frontend suites remain mandatory before production activation.
