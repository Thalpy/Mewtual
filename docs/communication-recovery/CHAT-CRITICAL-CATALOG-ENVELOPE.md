# Shared critical catalog outer DATA (5 November; authored, unexecuted)

`store::critical_namespaces::catalog_envelope` factors only the strict borrowed outer framing and
whole canonical-plaintext hash. It adds no namespace handler, opener policy, declaration writer,
source selection, cleanup permission, key lease or Current grant. ARCHIVE ID2 stays unsupported.

Existing v1 bytes remain exact generation1/count0 Empty with no parent field. V2 retains its
existing tag/domain, generation>=2, exact parent32 and1..16 strictly increasing positive namespace
IDs, nonzero versions, u32 payload framing and exact EOF. Every entry/payload borrows the original
input; the fixed16-slot array never allocates from an untrusted count. The hard total cap remains
JOIN16MiB+reserved ARCHIVE4KiB+outer512; a concrete reader may supply a smaller admitted cap.

Unknown descriptor IDs/versions are parsed as DATA so separate concrete consumers can inspect
their own known set. This is not support: the existing Pending-only `parse` explicitly requires
EXACTLY one ID1/version1, the same payload bound and unchanged JOIN source/transition validation.
ID2/unknown/mixed candidates continue refusing in either source before old cleaners. Default
Empty parsing/opener behavior is unchanged. No generic handler trait or support boolean exists.

The whole plaintext hash domain stays `catcoms/critical-catalog-plaintext/v1\0`, over every byte,
including unrelated descriptors. Matching JOIN metadata cannot establish equality/parenthood
when another payload changed. Actual authentication, both-source relation, original mount/CAS,
file/parent barriers and namespace-specific validation remain the consuming host's obligations.

The configured read invoice now includes the extra live fixed `Envelope` layout before any raw
buffer allocation. Original raw/unframe/plaintext/source metadata fees remain separate and held
through their actual barriers. The wrapper is not a pool debit or complete SDK/process bound.

Five regressions are authored for exact legacy bytes/hash/classification, borrowed unknown/ID2
framing with concrete JOIN refusal, whole-frame hash changes despite unchanged JOIN payload,
strict size/generation/parent/domain/version/ID/EOF/truncation/huge lengths and16/17 entries.
They remain **UNRUN** under the compilation pause. Direct formatting/static review is not compile
or runtime acceptance. The actual archive consumer and its configured policy remain required.
