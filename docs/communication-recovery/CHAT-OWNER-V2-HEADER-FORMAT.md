# Scalable owner successor headers

Concrete format for the successor header prerequisite in
[the scalable archive](CHAT-OWNER-SCALABLE-ARCHIVE.md). The declared source is implemented after
independent design review; actual-diff review and validation are recorded below. This is not a
v2 Profile/genesis codec, current-owner verifier, cold archive or live
dispatch. Existing action/v1 and owner-fact/v1 bytes remain unchanged. Compilation remains paused.

## Exact unsigned and envelope grammar

All framing uses catcoms-wire: integers are fixed-width big-endian; byte fields have u32_BE length.
The unsigned semantic frame contains, in this exact order:

1. Bytes `mewtual/chat/owner-header/v2\0`, u16 version 2.
2. Group bytes, length 1..256.
3. Profile CID, full issuer key, owner-tenure CID and original parent-header CID: four byte fields,
   each exactly 32 bytes.
4. Successor sequence u64, strictly positive; no saturation/wrap or zero/genesis interpretation.
5. Closed kind u8: 1 Admission, 2 AcceptanceAppend, 3 Removal, 4 ReviewDecision.
6. Exact body CID and resulting public-state commitment: two byte fields, each exactly 32 bytes.

The complete envelope is bytes `mewtual/chat/owner-header-envelope/v2\0`, bytes unsigned semantic
frame, bytes signature (exactly 64). HeaderCID is BLAKE3 of the raw domain
`mewtual/chat/owner-header-id/v2\0` followed by the complete canonical unsigned frame. The issuer
signs that exact unsigned frame. Signature bytes never contribute to HeaderCID. Body CID and
public-state commitment are untrusted labels here; their exact v2 body/state grammars remain a
separate format/consumer gate. A closed kind does not claim its body is available or valid.

All metadata is fixed-size except the bounded group. Decode borrows and validates the complete
outer and semantic framing before copying that one group field. Unknown version/kind, wrong fixed
lengths, empty/oversized group, zero sequence, duplicate/trailing/short framing refuse. Encode/sign
validate before key custody. Signing checks the returned signature against the exact supplied full
issuer key, without exporting private key material or introducing another key/RNG/time source.

## Authentication and contradiction boundary

Decode creates only untrusted signed data. A valid self-signature does not choose a Profile/root,
establish tenure, grant membership, prove an actual MLS Add/Remove, or authorize finality. The active
consumer must independently authenticate the original mounted Profile/full founder and supported
tenure, and select a known authenticated parent before treating a header as a governing successor.

The codec exposes pure signature verification and checked successor geometry only. Geometry checks
the SAME group/profile/full issuer/tenure, the exact parent's semantic CID, and checked parent
sequence + 1. It neither accepts a downloaded root nor creates a runtime/publication capability.
A genesis/tenure transition needs its separate reviewed producer; this successor helper does not
silently authorize a new issuer/tenure. All input signatures must be checked before duplicate or
contradiction handling; an invalid representation with an equal semantic CID cannot poison a valid
later representation.

Two valid successor headers with different semantic HeaderCIDs can contradict even when body bytes
are missing or invalid. Only the actual pinned, supported-tenure consumer may install that negative
state; this codec performs no selection, caching or state mutation. Do not choose a winner by time,
CID, first arrival or whether its body happened to arrive first. Exact duplicate semantics normalize
to one HeaderCID even when their genuine valid signatures differ.

## Resource invoice and required regressions

The maximum semantic/envelope size is the compile-time sum of the fixed grammar and 256-byte
group: exactly 520 / 634 bytes. These are per-buffer encoded caps, not a whole SDK/allocator/RSS guarantee or
prepaid contradiction escrow. A consumer must reserve two complete signed headers plus its actual
metadata/encoding/verification overlap before ordinary work can consume control capacity. Public
state/body/page/manifest/journal fees and sequence-control reserves remain separate gates.

Required authored tests use real keys/signatures: all four closed kinds round-trip at maximum group;
exact byte/domain/integer layout and complete maximum lengths; every scope/commitment/kind/sequence
field changes the semantic CID and signed bytes; corrupted signatures before/after valid duplicates;
genuine independently signed equivalent representations; foreign group/profile/issuer/tenure/parent,
wrong successor and u64 exhaustion; short/trailing/oversized/wrong-length/version/kind/zero frames;
wrong key callback/refusal and invalid grammar before callback. Confirm legacy frames refuse both
directions. No executed test or live producer is implied while compilation remains paused.

## Source evidence

`chat::owner_header` now implements the strict envelope, exact signing callback, unsigned-semantic
ID and both-signatures successor check. Nine regressions are authored but uncompiled/unrun during
the user pause. The frozen maximum-size fixture contains two genuinely different signing-nonce
Ed25519 signatures over identical semantics. Offline fixture preparation checked both through the
installed Python cryptography verifier; the authored Rust regression must still exercise the real
application's strict verifier. That fixture preparation adds no production dependency or crypto
implementation. Source formatting and whitespace checks pass. Actual-diff adversarial review is
clear; it is not a substitute for the deferred Rust checks or production consumers.
