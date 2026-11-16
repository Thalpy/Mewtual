# Private bounded record batches and composite Header DATA (5 November; authored, unexecuted)

`chat::cold_records` adds a content codec to the existing private page/manifest DATA prerequisites. It does
not enable ARCHIVE2, perform I/O, authenticate a selected source or issue Current/read/action
capabilities. Actual governing replay, source selection, journal repair, control reserves and
accepted-index completeness remain separate consumers. Public header/body encodings are unchanged.

## Exact grammar

A batch uses framed domain `catcoms/private-chat-record-batch/v1\0`, u16 version1, framed vault32,
framed group1..256, framed ProfileCID32, shared closed PageRole u8, framed exact operation bodyCID32,
u16 row count and ONE framed packed-row field, followed by no bytes. Total plaintext<=65536 and
count1..64. Only GenesisRecords(5), HeaderBodyRecords(4) and AcceptanceRecords(2) are supported.
Each packed row is kind u8, framed semantic CID32, framed OriginalRecordDigest32 and framed payload.
Rows are strictly increasing by (kind tag, semantic CID); duplicate typed keys refuse. Closed tags
are Profile1, Header2, Admission3, Acceptance4, Append5, Removal6 and Review7. There is no generic
unknown-kind escape. Sorting/copying untrusted row inventories is forbidden.

A Header payload is a composite using framed domain `catcoms/private-chat-header-row/v1\0`,
u16 version1, framed exact signed public header envelope and framed complete canonical public
state image. Its maximum is `4+domain.len()+2+4+MAX_HEADER_BYTES+4+MAX_STATE_BYTES`. It decodes the
actual header, verifies its signature and calls StateFrame::verify_header_binding on the actual
image. The semantic key is the public header CID. The image is part of the original payload.

The original digest hashes `catcoms/private-chat-original-record/v1\0`, the closed kind tag and
COMPLETE exact payload bytes. It is distinct from every semantic CID and OriginalActionDigest.
Genuine signature variants can share semantic identity but have different originals. Every
supplied signed Profile/Header representation is verified before order/duplicate handling;
invalid-first or invalid-second same-CID representations cannot be normalized into a valid row.

## Explicit expected DATA and content checks

The reader/encoder take `BatchBinding` containing expected vault, typed externally supplied
SignedOwnerProfile, shared PageRole, actual ArchiveOperation reference and an optional actual
Append body. These inputs remain DATA, never a source/proof trait or status boolean. The expected
Profile itself must verify. The full outer vault/group/Profile/role/body must match it and the
operation. All rows decode under their CLOSED kind, recompute semantic CID and original digest,
and match the expected group/Profile. Header additionally matches Profile founder and tenure.
All complete framing/count/per-kind payload caps are checked borrowed BEFORE typed row decodes.
Only after every representation is verified do sorted unique keys and role semantics run.

Genesis requires exactly Profile/Header/Admission, Header sequence1, actual Founder origin,
Profile.verify_genesis_binding and StateFrame.verify_genesis_binding. OPBODY is that Admission CID.
HeaderBody requires exactly Header plus the operation's Admission/Add, Append, Removal or Review
body, sequence>1, exact body/kind/group/Profile/issuer/tenure and actual image binding. It does NOT
apply body transition math or establish parent selection. Append metadata must also match the
ArchiveOperation Admission and before/after counts; arithmetic is not an acceptance proof.
AcceptanceRecords requires1..64 Acceptance rows and a mandatory actual Append body. That body's
CID, group/Profile/Admission and before/after counts must match the operation. Rows must form an
exact bijection with its leaves by channel AND recordCID and share the exact tree context. No
missing/extra/duplicated rows; matching may take bounded O64^2 because leaves and typed CID rows
use different orders. No record/action authority, cross-batch uniqueness or lookup completeness
follows. An unauthenticated Append remains DATA despite this content consistency check.

## Work/ownership invoice

The encoder accepts already ordered borrowed typed RecordSource variants; count<=64 is checked
before any iteration/encoding. It encodes one capped row scratch at a time, validates signed
representations/context, and stores only fixed metadata for at most64 rows. Checked total byte
size is admitted before allocating/copying the batch output. After all signatures pass, ordering
is checked; the final output is re-encoded from the same immutable references and fully checked
with the reader before return. Invalid role-level data can allocate only the admitted fixed page.
No arbitrary count sorting or retained collection of decoded rows is needed.

The reader borrows group/packed rows/payloads. Its complete framing pass allocates no row Vec and
precedes all typed copies. Streamed typed decode owns at most one bounded group copy at a time;
role checks may retain one Header (group<=256), one Header composite borrowed view and one body,
not64 decoded records. The actual Append's existing <=64 leaf labels are borrowed for the
Acceptance bijection, rather than copied as a second record inventory. Genesis reuses the
already verified equivalent external Profile after checking its real retained row representation.

Caller invoice includes original input, one <=65536 zeroizing output, one <=HeaderCompositeMax
row scratch, a <=634 Header encode (while forming composite), at most64 fixed row metadata,
public signature/semantic buffers (Profile<=494, Header semantic/envelope<=634), Append encode
<=3572 plus its <=3072 packed leaves, and fixed small decoded metadata/group copies. Header state
parse borrows its <=9244 image; complete-state validation is bounded to64 slots and2016 subject
comparisons per Header row. This is a logical simultaneous inventory, NOT a paid physical/RSS fee.
The host must separately pay actual Vec capacities, AEAD/file buffers, manifest/journal/head
copies, full replay hot state/frontiers, nodes/actions, SDK/global heaps and control reservations.
No founder64KiB or other unrelated allowance is reused by this codec.

## Regressions and limits

Ten authored regressions cover genuine signed genesis and each HeaderBody kind, max64 exact Acceptance
bijections,0/65 counts, complete truncations/trailing/oversize/unknown tags, typed ordering and
cross-kind/ID/vault/group/Profile/body swaps, composite state substitution and invalid same-CID
signature handling before duplicates. Existing genuine Profile signature variants will pin shared
semantic CID/different original digests; no artificial signing algorithm is introduced. Public
ReplayMath remains the separate complete expected-transition consumer. All tests remain UNRUN
under the explicit compilation/test/build pause. Direct rustfmt parsing/formatting and whitespace
checks are the only executed verification; these do not establish compiled or runtime correctness.
Read-only design and final actual-diff reviews are clear for this DATA-only checkpoint. The review
found and corrected a test fixture that initially violated the public image's issuer-present
invariant before reaching the intended Profile mismatch; the corrected fixture first proves its
actual HeaderRow is internally valid. No remaining blocker/high/medium was found by static review.

Pending focused selector once execution is permitted:
`cargo test -p catcoms-replication chat::cold_records::tests`. The complete backend/frontend suites remain
the primary agent's later serial validation; none were run for this checkpoint during the pause.
