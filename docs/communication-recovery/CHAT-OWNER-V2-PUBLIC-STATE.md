# Public owner-state image (5 November; authored, unexecuted)

`chat::owner_state` defines the public data committed by a v2 header's32-byte state field.
It is NOT the production governor, actual body-transition consumer, mounted source or positive
authority issuer. Six regressions are authored and UNRUN while compilation/tests/builds remain
paused. Existing action/v1, owner-fact/v1 and raw v2 header/body bytes remain unchanged.

## Image grammar and history

The canonical frame uses catcoms-wire integers/framed bytes, in this order:

1. put_bytes(`mewtual/chat/owner-public-state/v2\0`), put_u16(2), put_bytes(group1..256).
2. put_bytes(ProfileCID32), put_bytes(full issuer32), put_bytes(tenure32),
   put_bytes(parent HeaderCID32), put_bytes(previous state32).
3. put_u64(positive sequence), put_u8(closed HeaderKind1..4), put_bytes(current BodyCID32).
4. put_u64(current MLS epoch), put_u16(current slot count1..64), put_bytes(packed slots).

Each packed slot is exactly136 bytes: Admission BODY CID32, full subject32, current log-tip BODY
CID32, accepted count u64 and root32. Slots are strictly ordered by AdmissionCID, with unique
full subjects. Current issuer must remain represented: this supported tenure has no issuer
removal/transfer/succession grammar. This structural presence check is not proof that a claimed
body/actual MLS roster was applied correctly. Empty logs require count0, tip BODY exactly that
Admission body and canonical empty root under context(ProfileCID, exact AdmissionBodyCID).
Positive-count slots cannot name their Admission as the append tip.

The current64 bound does not limit lifetime admissions. Removed historical admissions, cuts and
decisions remain retained in the archive; the prior-state/current-body fold commits their ordered
history even if current slots later look identical. The fold is not a historical lookup/inclusion
proof or permission to discard originals. The future cold consumer must prove exact case heads,
closed admissions/cuts, already-accepted bindings and complete selected prefixes separately.

State commitment is BLAKE3(raw `mewtual/chat/owner-public-state-id/v2\0` || complete canonical
frame). Only PUBLIC semantic IDs/data enter it. Private randomized ciphertext/manifest IDs,
local signature representatives, raw provider state and replica-local rendered flags do not.
The maximum image is9244 bytes (35-byte domain, max256-byte group and64 packed slots). Header
size remains634 bytes: it contains only the resulting32-byte state digest, not this whole image.

## Avoiding a self-header cycle

The image contains the known PARENT header CID and current BODY CID, never its own result-header
CID. Initial slot log tip BODY is its Admission body. After verifying/signing the actual Admission
header, the runtime derives that slot's exact selected log-tip HEADER CID. Later Appends update
only their actual admission's tip; unchanged slots keep THEIR OWN earlier selected tip HEADER,
not whichever unrelated global header was most recent. This source metadata must be reconstructed
from the actual selected chain/journal; a decoded BODY label is not a header or a current source.

The virtual pre-genesis state is a separate label: hash the same state-ID domain followed by an
encoded frame containing put_bytes(`mewtual/chat/owner-genesis-state/v2\0`), group, ProfileCID,
tenure, full founder and actual creation label (each put_bytes). Verify the actual v2 Profile
signature BEFORE deriving it. Its scratch ceiling is444 bytes. No seq0 SignedOwnerHeader or
positive empty state can be decoded. First real image/header has sequence1, Admission kind,
the Profile's virtual genesis parent, that virtual prior state, epoch0 and ONE exact empty
Founder admission slot. Actual fresh creation/pin provenance is still independently required.

## Binding versus governing verification

`StateFrame::decode` performs complete bounded borrowed framing/order/slot checks, with no parsed
group/slot Vec or map. At most2016 fixed full-subject comparisons enforce uniqueness. `encode_state`
validates first and requests one output buffer, appending packed slots directly without a second
packed Vec. The caller separately owns any actual slot container/signature/verification scratch;
this cap is not a host-memory invoice, SDK heap bound or cold/control reservation.

`verify_header_binding` verifies the actual header signature, EVERY matching metadata field and
the recomputed state hash. `verify_successor_binding` also checks BOTH actual signatures, exact
successor geometry/context and previous state==parent's signed result. `verify_genesis_binding`
checks actual typed Profile/Founder body/first-header binding and the empty initial image. These
are data/crypto consistency checks. A genuinely signed image can still describe a false body
transition. No cached self-signature/root label supplies mounted authority or selected finality.

Before production, the connected consumer must apply and compare the exact result:

* Founder/Add obtains the actual fresh/selected-KP producer observation, advances only the actual
  epoch, installs a distinct Admission incarnation and verifies the full current roster/slots.
* Append checks that admission's OWN selected prior tip, original complete lookup/idempotency and
  exact frontier/leaf/node mathematics. Frozen admissions cannot silently append beyond a cut.
* Remove checks actual original producer observation, current epoch/roster and exact frozen tip,
  then closes only that admission. Current-issuer removal/transfer remains unsupported.
* Review verifies the exact original target and selected current-case head/closed transition from
  complete retained evidence. Exact saved retries precede new policy; Reject cannot undo Approved,
  a separate authorized Delete removes content and review never rewrites a frozen LOG cut.

Actual private MLS observations are a PRODUCER boundary; a recipient's historical public replay
instead relies on the explicitly pinned non-equivocating finalizer's authenticated transition
attestations and checks public semantics. A signature does not independently prove private MLS
execution or global freshness. Neither path can be replaced by these codec/binding helpers.
Known-parent contradiction handling, current selected source, group/native publication barriers,
critical journal/control fees, checked remaining sequence space and the production semantic/
projection issuer are all still open. This packet enables none of those paths.

## Authored regressions (UNRUN)

Six tests cover max64 current slots/exact size/borrowed canonical roundtrip; genuine signed v2
Profile/Founder/first header and invalid same-CID signature refusal; exact signed parent/prior-root
binding; identical current slots with different retained-history commitment; order/duplicate
subject/current-issuer/empty-tree errors; and truncation/extra/oversize/version/count/signature/
state substitution. Review fixture headers deliberately check data binding, not an approved
case, actual MLS action, mounted Saved/Current or live Chat V2 acceptance.
