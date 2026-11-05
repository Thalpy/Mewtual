# Private complete-node page prerequisite (5 November; authored, unexecuted)

`chat::cold_nodes` implements one bounded DATA page for the pending archive. It writes no file,
opens no namespace, selects no owner/root and creates no durable/current/acceptance capability.
The critical catalog still accepts Empty only. Its6 regressions are authored and UNRUN during
the compilation pause; formatting/review is not executable evidence.

## Canonical plaintext

Fields use catcoms-wire fixed big-endian integers and u32-framed byte fields, in this order:

1. put_bytes(`catcoms/private-chat-node-page/v1\0`), put_u16(1).
2. put_bytes(original vault-domain32), put_bytes(group1..256).
3. put_bytes(ProfileCID32), put_bytes(AdmissionCID32), put_bytes(exact AppendBodyCID32).
4. put_u64(before count), put_u64(after count), put_u16(node count).
5. put_bytes(node count*32 packed hashes in actual leaf/carry creation order), then exact end.

Checked after-before is1..64. Node count must equal the existing append formula
popcount(before)+(after-before)-popcount(after) INTERNAL nodes, PLUS(after-before) leaves.
The conservative ceiling is191 total hashes, not an assertion that191 is reachable in one real
append. The page buffer ceiling is6578 bytes, including a maximum256-byte group; actual encoded
pages can be smaller. The domain has34 bytes including its NUL. No ordinal is encoded: derive
the first as2*before-popcount(before) using u128 BEFORE multiplying. There is no u64 position
cliff, self-header-CID cycle, filename, external URL or embedded provider/key material.

`NodePage::decode` checks complete bounded borrowed framing/count/packed length before returning
a view. It copies fixed labels only; group and packed hashes borrow the original bytes and no
parsed Vec/String/map is allocated. A syntactically valid page can contain false hashes/context.
`hash_at_ordinal` does checked wide subtraction/range checks BEFORE usize conversion and returns
only untrusted hash data, never a physical page address or proof of retention.

## Producer and verifier consistency

`encode_node_page` accepts the actual privately constructed `PreparedAppend` and exact Append
body. BEFORE output allocation, check old/new tips and context, complete created-node count,
every derived contiguous ordinal, and EVERY leaf's position/context hash against that body's
actual sorted leaf list. Equal DECLARED old/new roots alone are insufficient: an unverified body
can claim those same roots while naming another leaf list. Its internal nodes come from the
existing private bounded append constructor. Body-ID scratch ends before output allocation.

The encoder requests one6578-byte output buffer, writes the prefix/packed length and appends
hashes directly into that SAME buffer. It creates no separate packed-node Vec. The returned
plaintext uses Zeroizing; the future original Store must seal it before any disk write.
`verify_exact_append` checks the expected original vault/group/context/body/counts, recomputes
the exact bounded append from the supplied Frontier and compares EVERY ordinal/hash. A correct
last root alone cannot conceal a substituted interior node. Success remains math/data consistency,
not proof of a selected signed owner tip, on-disk durability or any action's acceptance.

## Separate resource obligations

The cap is one requested plaintext-buffer allowance. The caller ALSO owns its original encoded
input, actual PreparedAppend Vec (<=191*sizeof(CreatedNode)), fixed64-slot frontier, any separately
recomputed verification nodes/frontier, and existing AppendBody ID scratch (<=3572 encoded bytes
plus<=3072 packed leaf bytes). Body-ID scratch ends before encoder output, but it can overlap the
existing prepared/input owner. There is no claim these labels equal allocator/RSS or pay a native
pool. AEAD raw/cipher/plain buffers, file handles, page/journal reachability, control reserves,
actual host fees and SDK/provider typed heaps are separate required compound-operation invoices.

The actual private page/manifest/journal reader must authenticate the selected original file and
context, then the completed public inclusion/prefix proof against independently selected signed
tips. Parsed page labels or a proof-planner ordinal cannot substitute for those checks.
The broader [archive design](CHAT-OWNER-COLD-ARCHIVE-FORMAT.md) remains draft/unimplemented.

## Authored regressions (not executed)

Six tests cover real multi-batch leaf/carry mathematics and borrowed pointers; synthetic high-u64
frontiers/120 internal carries/184 actual nodes and ordinals above u64; an interior hash substitution
under an unchanged genuine signed Append; vault/group/context/body/frontier/prepared swaps;
same-count/same-claimed-root changed-leaf producer refusal; and complete truncation/extra/oversize/
wrong-version/empty-group/count/packed-geometry rejection. Synthetic frontiers are math fixtures,
not evidence of2^63 actual accepted actions. The signed-header test checks body binding, not
genesis/root/Current provenance. Adversarial review corrected the producer leaf-binding gap.
