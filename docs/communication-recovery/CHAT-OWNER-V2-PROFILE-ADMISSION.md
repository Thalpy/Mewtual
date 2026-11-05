# V2 Profile, genesis anchor and ordinary Admission labels

Concrete design for the next scalable-profile prerequisite. Review precedes source implementation.
This creates no mounted pin, actual MLS observation, governing selection, Current, Saved or live
publication authority. Action/v1 and owner-fact/v1 bytes remain unchanged. Compilation stays paused.

## Signed fixed-founder Profile

The canonical unsigned frame is bytes `mewtual/chat/owner-profile/v2\0`, u16 version2, group
bytes1..256, full founder32, actual creation label32, then these CLOSED policy fields:
u8 trust1 (non-equivocating original finalizer); u8 succession0 (unsupported); u16 ordinary MEMBER
rights23; u8 member-created-realms1; u8 acceptance-rule1 (unsigned record authenticated by inclusion
under a selected signed tip); u8 tree grammar2; u8 body grammar2; u8 header grammar2; u16 maximum
CURRENT members64. No lifetime-admission or accepted-action count is introduced. Every32-byte field
is length-prefixed; no unknown policy/trailing fields are accepted.

The envelope is bytes `mewtual/chat/owner-profile-envelope/v2\0`, bytes unsigned frame, bytes
signature64. The founder signs the exact unsigned frame. ProfileCID is BLAKE3(raw
`mewtual/chat/owner-profile-id/v2\0` || unsigned frame), excluding signature bytes. Maximum unsigned
379 / complete494bytes. A raw Profile/self-signature never chooses its trusted root. Actual fresh
creation or the explicit trusted root captured BEFORE selected-KP join must install the immutable
expected (group, full founder, ProfileCID). Existing v1 Current cannot silently convert to this
different Profile by relabelling its snapshot or current roster.

The initial tenure label is BLAKE3(raw `mewtual/chat/owner-tenure/v2\0` || ProfileCID32 || founder32
|| creation32). The virtual genesis parent label is BLAKE3(raw `mewtual/chat/owner-genesis/v2\0` ||
ProfileCID32 || tenure32 || founder32 || creation32), represented as HeaderCID DATA. It is not a
decoded sequence-zero header. The first successor must have sequence1, kind Admission, exact
derived parent/tenure and the same group/profile/full founder, with a valid signature. A separate
founder Admission body must bind that exact Profile creation/subject. This closes the first-header
geometry without letting a downloaded Profile create a runtime or authorize owner succession.

Resulting public-state commitments MUST NOT include the current header's own CID recursively.
Initial empty tips derive from Profile/Admission body IDs; header provenance is checked separately.
Exact public state grammar remains a reviewed implementation gate.

## Admission body and stable owner-offline labels

The unsigned body is bytes `mewtual/chat/admission/v2\0`, group1..256, ProfileCID32, full subject32,
origin u8 (0 Founder, 1 Add). Founder has creation32. Add has exact Add CommitCID32, before epoch u64,
after epoch u64, original selected-KP reference32 and credential32; after must be checked before+1.
Finally u16 MEMBER rights23 and u8 member-created-realms1 are mandatory. Maximum490bytes (Add).
AdmissionCID is its existing domain-separated owner-body CID. The unsigned body alone is not an
Admission witness: it needs its exact selected signed Admission header under the independently
pinned Profile/tenure and governing chain, plus the real producer's MLS/source/save checks.

The v2 Profile must explicitly select the fixed delegation rule. For the exact realm, derive IDs
with catcoms-wire framing, as already specified in the scalable archive:

- GrantID: bytes `mewtual/chat/ordinary-grant/v2\0`, group, ProfileCID32, AdmissionCID32, full
  subject32, RealmID32, u16 MEMBER bits23, u8 member-created-realms1.
- BasisID: bytes `mewtual/chat/ordinary-basis/v2\0`, the SAME group/profile/admission/subject/realm,
  derived GrantID32, u16 MEMBER bits23, u8 member-created-realms1.

These are public labels, not historical authority. No per-post owner signature or parent-coverage
refresh is needed to sign a new-channel action/reply while the owner is offline. Same-key A1/A2
produce different Admission/Grant/Basis IDs from their actual fresh Add/KP incarnations. A pure
action-binding check must verify its real signature, exact group/full subject and these derived
labels, without claiming current membership, selected acceptance or cross-author Delete approval.

The later ordinary-authority consumer must verify the Profile/Admission/header against ORIGINAL
mounted provenance and the common nonconflicting governing prefix before issuing a semantic
witness. It derives the canonical minimal coverage set from the actual <=64 direct parents plus
target, independently verifying each selected Admission under that same root, with no recursive
coverage expansion or arbitrary caller extras. An acceptance record cannot bypass that step.
Projection admission/negative scopes remain separate: historical ordinary rights do not prove an
unproven late action preceded removal, and a detected owner fork must not leave a stale valid
projection/approval active. No new opaque authority constructor is proposed in this codec slice.

## Bounds and regressions

All framing is borrowed/preflighted before owned group copies. Policy flags/rights are fixed by
the codec; callers cannot sign a delegated DELETE_ANY variant. These are per-buffer caps only, not
complete private SDK/active host, historical disk, control or allocator fees. Old lifetime64/4096
checkpoint limits remain nonactivatable; later cold archive/current-state/sequence reserves must
actually replace them before production use.

Required authored regressions: real signature/strict frame maxima and all fixed policy rejection;
unsigned-semantic identity/signature checks before duplicate handling; trusted-root distinction;
first-header positive and swapped issuer/profile/group/tenure/parent/sequence/kind/body refusal;
actual founder-vs-Add body bindings and epoch overflow; independently framed ordinary ID vectors;
same-key fresh A1/A2 and cross-realm/group/Profile separation; member-created realms/offline replies
without per-post owner facts; real action altered Grant/Basis/subject/SIG rejection; wrong/truncated/
trailing/foreign-v1 frames. No execution is claimed while tests/compilation remain paused.
