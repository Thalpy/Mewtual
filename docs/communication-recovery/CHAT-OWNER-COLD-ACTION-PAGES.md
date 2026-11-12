# Private original-action chunks (5 November; authored, unexecuted)

`chat::cold_actions` retains/assembles exact signed-action DATA through bounded private chunks.
It performs no I/O, enables no namespace, grants no historical/current authority and authorizes
no admin read. Six regressions are authored and UNRUN during the compilation pause. The actual
archive handler, compound journal, source admission and native consumer remain unimplemented.

## Canonical plaintext and original identity

The complete frame is:

1. put_bytes(`catcoms/private-chat-action-chunk/v1\0`), put_u16(1).
2. put_bytes(original vault-domain32), put_bytes(group1..256), put_bytes(ProfileCID32),
   put_bytes(semantic ActionCID32), put_bytes(exact original envelope digest32).
3. put_u32(total original length), put_u16(chunk index), put_u16(chunk count), put_bytes(payload).

Payload is at most61440 bytes; total is positive and at most the existing131072-byte action
envelope cap. Count must be EXACT ceil(total/61440), at most3 conservatively; index<count and
payload length must exactly match that index's remaining range. No caller-supplied offset or
filename occurs. The per-page buffer ceiling is61899 bytes (37-byte domain, max256-byte group).
All framing, group, count, geometry, payload length and exact end are checked using borrowed
slices. No decoded payload/group container is allocated. Syntax alone authenticates no bytes.

OriginalActionDigest is BLAKE3(raw `catcoms/private-chat-original-action/v1\0` || COMPLETE original
canonical envelope, including its chosen valid signature). It is deliberately not ActionCID.
Genuine alternate valid signatures share one semantic action but name different original byte
representations. The archive may retain one independently verified representative per semantic
CID; it need not store infinitely many equivalent signature variants. A later receive must still
verify its own signature before duplicate handling and cannot overwrite the retained original
merely because the semantic CID matches. All distinct actions/edits/deletes remain retained.

## Preparation and assembly

`PreparedActionPages::prepare` verifies the actual signature and owns ONE zeroizing canonical
envelope plus immutable group/ActionCID derived from that SAME action. Page encoding takes only
an index, so another caller action cannot retarget later chunks. Invalid indices refuse before
page allocation. Encoded plaintext uses a zeroizing output buffer; the future Store must seal it
before writing. This preparation is not Saved, an archive reservation or permission to publish.

`assemble_action` requires EXTERNAL expected vault/group/Profile/ActionCID/OriginalActionDigest
from the original selected manifest. A parsed first page cannot select its own trusted request.
Check exact ordered count, every context/total/index and incremental hash of borrowed payloads
BEFORE allocating an assembled output. Missing/reordered/duplicate/mixed originals never yield
partial data. After the digest matches, assemble once, strict-decode SignedAction, verify its real
signature and compare actual semantic CID and group with the requested source. A valid byte
digest cannot replace a signature; a page's declared CID cannot replace the signed payload's CID.

The result exposes exact original bytes and the signature-checked typed action as DATA. Actual
ordinary delegation, semantic dependencies, accepted-before-cut/review evidence, current effects
and privileged native/UI read permission are independently required. Merely assembling a valid
old signature cannot release an unproven late action from a removed member.

## Memory and source limitations

The caller independently pays the original typed SignedAction and its group/text/parent backing,
existing nested `encode` semantic+envelope buffers, `id`/signature verification semantic scratch,
the prepared canonical envelope, copied group, up to3 simultaneous chunk buffers and their raw/
AEAD/sealing owners. Assembly adds its original raw output plus typed decoded action and existing
decode/re-encoding/signature scratch. Each original/semantic envelope can request up to131072
bytes; page encoding requests61899. The actual phase/drop invoice must count overlapping buffers,
not just payload length or a multiplier. This codec supplies no host/process pool, storage fee,
SDK bound, lower-transport accounting or durable retry grant.

Exact original-byte retrieval is distinct from semantic recovery. A valid alternate signed
representation can agree on ActionCID while failing an old manifest's selected original digest.
It must not silently masquerade as those original bytes. A later separately reviewed repair
transaction may retain that independently verified equivalent action with explicit source/coverage
metadata; it may never fabricate an original signature or change an uncertain SDK/reply candidate.
This codec implements no such repair or on-disk original-selection update.

## Authored regressions (UNRUN)

Six tests cover real small/max-text+64-parent signed originals and borrowed payload pointers;
missing/reversed/duplicate/mixed/corrupted chunks; external expectation swaps; correct digest but
invalid signature and payload-CID mismatch; genuine alternate-signature vectors with one semantic
identity and different original digests; and full truncation/extra/oversize/version/geometry
refusal. Large edit fixtures are signature/format data, not proven semantic target dependencies.
The existing alternate signatures are genuine equation-valid fixtures, not a verifier stub.
