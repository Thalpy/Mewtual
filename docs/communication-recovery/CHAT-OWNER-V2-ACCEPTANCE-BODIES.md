# V2 acceptance records, appends and removal bodies

Concrete format prerequisite subordinate to the scalable archive and its successor-header codec.
The declared source is implemented after design review; source evidence is below. No Profile/genesis, governing chain,
owner-state commitment, cold store or active authority is created by these data codecs. Existing
action/v1 and owner-fact/v1 bytes remain unchanged. Compilation remains paused.

## Acceptance record and authentication

V2 uses a bounded canonical acceptance RECORD. Unlike a v1 SignedOwnerFact, this new record has no
standalone signature: its exact CID is authenticated by inclusion under an independently verified
selected owner's signed log tip. A record, its CID, a matching action signature or arbitrary caller
root alone never establishes acceptance. This explicit v2 proof rule must be selected by the real
pinned v2 Profile/chain consumer; no v1 proof is reinterpreted.

All fields use catcoms-wire fixed big-endian integers / u32-prefixed bytes. The record frame is:
bytes `mewtual/chat/acceptance-record/v2\0`; group bytes1..256; ProfileCID32; AdmissionCID32;
channel u128; ActionID32; full subject32; GrantID32; BasisID32; create-presence u8 (0/1), and
CreateID32 only if present. All32-byte fields are length-prefixed. No trailing/unknown fields.
CID is BLAKE3(raw `mewtual/chat/acceptance-record-id/v2\0` || complete canonical frame).
Maximum encoded record567bytes. Its tree leaf is the exact existing `(channel, CID32)` geometry,
under the SAME profile/admission tree context. The use of a fixed CID label supplies no proof.

A pure `matches_action` check verifies the real action signature, exact group/channel, ActionID,
full signer, Grant/Basis and `Body::create()` target. This does NOT validate historical membership,
ordinary rights, causal dependencies, accepted-before-removal or an exact cross-author Delete
approval. Reply targets are already committed by the complete ActionID; they are not confused with
the CreateID edited/deleted/joined. Genuine alternate action signatures share semantic acceptance.
Action/v1 does not directly sign Profile/Admission IDs: changing those labels can leave this
matching check true while changing the record CID and labelled tree. The separate delegated
Grant/Basis verifier must prove the incarnation relationship. Matching also uses the actual
bounded action's signature/semantic buffer and work; the567-byte record cap does not pay it.

## Append body

The exact body frame is bytes `mewtual/chat/acceptance-append/v2\0`; group1..256; ProfileCID32;
AdmissionCID32; previous per-incarnation tip HeaderCID32; old count u64/root32; new count u64/root32;
leaf count u16 (1..64); ONE byte field containing count packed48-byte records `(channel_u128_BE,
AcceptanceRecordCID32)`, strictly sorted/unique. Maximum3572bytes, independent of prior count.

The previous tip header is a dependency label, not a root selection. The real verified chain must
bind it to the exact old context/count/root (initial empty tip comes from the independently verified
Admission/genesis), prove this is the current unfrozen tip, and make the new tip governed. The body
does not repeat an old frontier or duplicate complete acceptance records. New count MUST be checked
old+leafcount without overflow. Decode checks all framing, count, packed length and sort BEFORE its
owned leaf Vec or group is copied. No unsigned frontier is retained per append.

`prepare_on_frontier` is pure mathematics: supplied frontier tip must equal the exact old labelled
tip, bounded append produces the claimed new labelled tip, and the returned nodes are the existing
bounded leaf/carry output. It does not authenticate frontier, bodies, old tip dependency or owner.
Cross-batch idempotency is still actual retained lookup, not a duplicate check in this body codec.

## Removal body

The exact body frame is bytes `mewtual/chat/removal/v2\0`; group1..256; ProfileCID32;
AdmissionCID32; full subject32; actual Remove CommitCID32; before epoch u64; after epoch u64;
frozen per-incarnation tip HeaderCID32; frozen count u64/root32. Maximum528bytes regardless of the
number of previously accepted actions. After epoch must be checked before+1, without wrap.
The constant body replaces v1's complete accepted-ID cut list ONLY in the separate v2 profile.

The real governing consumer verifies the actual observed MLS transition, exact selected Admission/
subject and frozen tip/header relationship, state commitment and save-before-publication. Merely
decoding this body does not remove membership or establish that a late action was accepted before
the cut. That later claim needs an exact record/inclusion proof under the authenticated frozen tip.

## Header binding and budgets

Each body CID is BLAKE3(raw `mewtual/chat/owner-body-id/v2\0` || complete canonical body frame).
A pure header-binding helper checks the actual header signature, exact kind/group/profile/bodyCID;
it does not verify owner/tenure provenance, previous governing chain or resulting state commitment.
Missing/invalid bodies do not suppress a separately authenticated pair of contradictory headers.

These caps are per encoded buffer, not a complete runtime admission or physical allocator/RSS
invoice. A host must separately reserve leaf/carry output, every original record/body, scratch,
both contradictory headers, cold pages/manifests, journal/receipts, mandatory control sequence slots
and all retained uncertain candidates before signing. No code here opens files or spends future
removal capacity. Exact state, persistence and scheduling invoices remain implementation gates.

Required authored tests: full-size strict codec/byte-layout round trips; all truncations/wrong fixed
fields/options/counts/packed sort/duplicates/foreign labels/trailing data; exact action field and
signature binding; unchanged v1 refusal both directions; actual frontier append and deliberately
wrong roots/counts; u64 overflow; same-key A1/A2 separation; constant removal length at1 vs1,000,000
accepted leaves; epoch wrap; actual signed header kind/scope/body mismatch and invalid-signature
refusal. No test or compilation is claimed during the pause.

## Source evidence

`chat::owner_bodies` now implements these three strict data codecs and their pure action/header/
frontier binding checks. Actual-diff adversarial review is clear. Thirteen regressions are authored
but uncompiled/unrun, including exact maxima, every truncated prefix, malformed fixed/packed fields,
genuine action signature variants, A1/A2 separation without a false incarnation claim, actual tree
append math, signed header binding and overflow/epoch refusal. Count1 vs1,000,000 tests use explicit
data labels to pin constant encoding; they do not claim a million actual accepted actions. Only
source formatting and whitespace checks ran. Profile/genesis, ordinary delegation, governing state,
cold storage, covering producers and live native/UI integration remain separate incomplete work.
