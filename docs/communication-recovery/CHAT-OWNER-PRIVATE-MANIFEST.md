# Private bounded manifest segment DATA (5 November; authored, unexecuted)

`chat::cold_manifest` implements this DATA prerequisite, not a supported namespace or I/O path.
ARCHIVE ID2 remains refused. No decoded segment or valid page hash may create mounted/current,
historical or admin-read authority. Actual covering journal/head/CAS and resource consumers remain
required. Original source, SDK replacements and exact replies have separate private custody.

Each immutable segment describes ONE operation and links one prior sealed manifest page ID.
It contains no complete historical page list, filesystem path, URL or mutable index root.
One operation introduces at most3 content pages plus its own manifest; old segments stay immutable.
This is an operation-page bound, never a lifetime history limit.

## Exact fields

Use framed domain `catcoms/private-chat-manifest/v1\0`, u16 version1, vault32, bounded nonempty
group<=256, ProfileCID32, positive u64 manifest sequence, optional previous sealed-manifest ID,
optional original canonical ArchiveHeadId, optional preceding public owner-header ID, resulting
public owner-header ID, a closed operation and <=3 packed content descriptors. All integers are
big-endian; byte fields use strict u32 framing. Optional digests have a closed0/1 tag. Descriptors
are packed in one framed field, each39 bytes: role u8, raw private page ID32, plaintext bytes u32,
logical entries u16. All variable data is borrowed on parse; the complete page is<=1024 bytes.

Initial sequence1 has no parent/source-head/preceding-header and uses Genesis only. Later sequences
require all three exact original labels. `ArchiveHeadId` names the future complete private
PRE-OPERATION canonical head, distinct from a manifest PageId or public header ID. New manifest
references point backward; the new head can then reference the new manifest without a cycle.
Its scalar wrapper is DATA;
only actual mounted writer custody can supply it authoritatively. Governing operations require a
different resulting header. Plain retained-action operations keep the exact preceding header.

Closed operations and their required descriptors, in fixed order:

| Operation | Payload | Content pages |
| --- | --- | --- |
| Genesis | exact Admission body ID |1 GenesisRecords,3 rows: Profile/header/body |
| RetainAction | ActionId, exact OriginalActionDigest, total envelope bytes |1..3 OriginalActionChunk pages, derived complete geometry,1 logical action each |
| Admission | exact Admission body ID |1 HeaderBodyRecords,2 rows |
| Append | exact Append body ID, AdmissionCID, before/after counts | AcceptanceRecords (delta1..64), CompleteNodes (derived leaf/carry count), HeaderBodyRecords (2 rows) |
| Removal | exact Removal body ID |1 HeaderBodyRecords,2 rows |
| Review | exact ReviewDecision body ID |1 HeaderBodyRecords,2 rows |

Action chunk encoded lengths derive from the actual group length and complete original-envelope
geometry, not declared per-page counts. Complete-node lengths/counts derive from checked before/
after counts and the existing node codec. Other batch lengths must be positive and<=65536 until
the separate exact batch codec is connected. Duplicate page IDs, unknown roles, wrong order/count,
inconsistent chunk/node lengths, unbounded totals and trailing bytes refuse before output copying.

Actual inner pages still need full vault/group/Profile/typed-key/digest/signature/body binding.
Descriptor consistency does not establish that any page exists, contains the requested original,
is durable, or is historically accepted. In particular a HeaderBodyRecords descriptor is not proof
that either real signed header or body was supplied. No source absence or finality follows here.

The future private Header row must retain BOTH the exact original public signed header envelope
and its complete canonical public state image, and validate the image against that real signed
header's state digest/fields. It is one composite private record, not a change to the public signed
header wire grammar. The image is<=9244 bytes and the envelope<=634, so the separate bounded batch
format can hold their framed pair. This preserves the3/2 logical row counts above and supplies the
existing ReplayMath complete expected-state comparison during cold replay. The exact private
composite/batch grammar remains a separate reviewed prerequisite; it cannot silently omit state
validation or promote a raw decoded image to Current. Multiple Action chunk descriptors are
fragments of ONE retained original action, not independent logical actions.

A data-link check compares the external selected previous sealed ID and current source-head ID,
same exact vault/group/Profile, checked sequence+1, and preceding-header==previous resulting-header.
It does not authenticate those EXTERNAL labels, source bytes or select the governing chain. The
real reader must first open the previous sealed page against its selected manifest ID; matching
self-supplied hashes or labels cannot authorize a replacement source.

## Required source tests and remaining gates

Seven authored source tests cover strict0/1/3/4 descriptors and full framing/size/truncation,
each operation's geometry, exact action chunk/node counts, duplicate/reordered roles, all
group/vault/Profile/source/parent/header swaps, sequence overflow and header preservation for
ordinary retention after a control transition. A real signed action crosses chunk/actual encryption/
manifest/open/assembly/signature boundaries; actual tree output also matches node geometry.
Synthetic maximum carry/chunk geometry and5000 finite encrypted DATA links cover arithmetic and
absence of a codec lifetime list; they do NOT represent5000 actual signed owner operations or a
production cold accepted-index test. All seven tests remain **UNRUN** under the compilation pause.

The encoder requests one1024-byte zeroizing output; parse borrows group/descriptor bytes and owns
fixed labels only. Actual original/record/node/body encoders, encryption buffers, sealed manifest,
journal/head copies, original file/parent barriers and physical fees remain separately accounted.
No memory/accounting claim follows from this DATA cap. Persistent accepted-lookup/index prefix,
contradictions/cases, essential/audit availability and complete journal repair remain open.
Manifest sequence, page/file and journal allowances also need actual mandatory-control reserves;
checked u64 arithmetic alone is not prepaid Remove/finality/repair capacity. Ordinary retention
must not consume those reserves. No control allowance is created by a decoded field here.
