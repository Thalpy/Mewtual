# Bounded acceptance tree primitives

Concrete proposed hashing/proof contract for the scalable owner-evidence/v2 successor. Read-only
design and actual-diff reviews cover the implemented prerequisites. These primitives do not sign a tip, select an owner/root,
prove historical membership, approve an action or perform storage. Actual pinned owner facts and
covering cold-page/head publication remain required. Existing action/v1 and owner-fact/v1 bytes
are unchanged. Compilation and compiled tests remain paused by the user.

## Exact semantics

All hashes are BLAKE3-256. The immutable context is exactly `ProfileCID32 || AdmissionCID32`.
The leaf record is exactly `channel_u128_big_endian || AcceptanceRecordCID32`; the separately
authenticated v2 acceptance record binds the exact action/signer/grant/basis/Create. Its exact
inclusion under an independently selected signed tip is mandatory; raw records are not accepted.
This does not reinterpret v1 SignedOwnerFact proofs. Hash inputs have fixed
lengths after their domain, with no implicit native integer encoding:

- Empty: `"mewtual/chat/acceptance-empty/v2\0" || context`.
- Leaf: `"mewtual/chat/acceptance-leaf/v2\0" || context || leaf_record`.
- Internal: `"mewtual/chat/acceptance-node/v2\0" || left32 || right32`.

A nonempty tree of n leaves splits at the largest power of two STRICTLY smaller than n. The tip
API carries `(context, count_u64, root32)`. A count-zero tip must have the exact context's empty hash. Node
hashes/tips/context labels are untrusted data until their external signed/current sources verify.

## Append and frontier

A frontier has exactly one subtree hash for each set bit of count, in ASCENDING bit order. No
extra/missing slot is accepted. Folding from low to high uses `node(higher_subtree, accumulated)`;
this is the required right-associated shape for an incomplete tree. A loaded frontier must fold
to its supplied expected tip before append; matching a caller's tip is not tip authentication.

Each batch has 1..64 leaves, strictly sorted by `(channel, AcceptanceRecordCID)` with no duplicate
within the batch. Cross-batch idempotency/absence remains the real owner's independently verified
retained lookup, not a claim this primitive can make. Count addition is checked before mutation.
The primitive prepares a new frontier/result without changing the supplied original on refusal.

For initial n and batch k, the exact new internal-node count is
`popcount(n) + k - popcount(n+k)`. At most127 internal nodes plus64 leaves are admitted; the known
actual maximum internal count for k64 is120. An emitted node record contains its first-leaf index,
power-of-two leaf count and hash, in creation order. These are O(N) append-only node records for
bounded cold-page persistence, not a retained full path copy for every historical root. The actual
host must reserve `k + internal_count` records/bytes and one bounded scratch/frontier before calling.

For a complete aligned node range `[first,first+span)`, let checked `n=first+span` and
`height=log2(span)`. Its logical zero-based creation ordinal is
`2*(n-1)-popcount(n-1)+height`: the nth leaf creates its leaf then all lower carries. Use u128
BEFORE doubling; total retained node count can exceed u64 while leaf count is still valid. Reject
zero/non-power span, unaligned first and overflowing end. This derives a logical position only,
not node presence/hash validation or a physical page address. Future cold-page/manifest encoding
must still prove and durably publish the exact record. No path-copy index is implied.

## Bounded proof lookup plans

The count-only planner produces exact recursive subtree ranges in the verifier's bottom-up inclusion
or SUBPROOF consistency order. It does not read or hash history. Inclusion refuses an empty tree or
an out-of-range index before output allocation; prefix planning refuses reversed counts. Empty/equal
prefix plans contain no ranges, without claiming the verifier's context/empty/equal-root checks ran.

An output range has private construction: its positive count and checked end are bounded by the
requested tree, and its first index is aligned to the largest power-of-two component. This canonical
shape is necessary. An arbitrary interval can need 126 aligned components; it must never be passed
through a future raw constructor/parser assuming the canonical range's 64-component limit.

Each range decomposes without allocation into complete aligned node labels, rightmost-smallest to
leftmost-largest. Their checked u128 creation ordinals are logical lookup positions. Folding exactly
those hashes uses `node(left, accumulated_right)`; short/extra hash lists refuse. The host must check
each loaded committed record's exact range and ordinal, then verify the completed proof against its
independently authenticated tips. Geometry, folding and an empty plan do not establish retained node
presence, grant filesystem access, or authenticate any root or owner.

Inclusion has at most 64 ranges, consistency at most 65; a conservative export invoice is 65 * 64 =
4,160 logical complete-node reads. Plans also report their smaller exact decomposition read count.
The fixed planner stack/output and actual I/O buffers, page-cache behavior, work scheduling and
publication remain separately paid host responsibilities. This prerequisite adds no cold-page
format, network request, storage reader or active export path and performs no full-history scan.

## Inclusion and historical prefix proofs

Inclusion accepts an exact leaf/index/count and BOTTOM-UP siblings. The index must be less than a
nonzero count; the exact path is derived from index/count. At most64 siblings, no trailing or short
path. It must match the supplied expected tip under the same context. It grants no action authority.

Historical-prefix verification FIRST requires identical tip contexts, then follows the count-aware
binary consistency construction for that same split rule. Opaque subtree hashes cannot independently
authenticate those labels: the caller must verify both contexts against their external signed tips.
Zero old count requires its canonical empty root and an empty proof; equal counts
require equal roots and an empty proof. Reversed counts refuse. For unequal nonzero counts, the
proof is the canonical recursive SUBPROOF(m,n,complete) sequence, not a free node bag. Start with
complete=true; if m==n emit nothing when complete, otherwise emit that subtree root. At split k,
recurse left with the same complete flag when m<=k and append the right root; otherwise recurse
right with complete=false and append the left root. Verification consumes that exact path shape.

Consistency needs its OWN **65-hash** maximum: for m=7,n=15 the canonical proof has five hashes,
and m=2^63-1,n=u64::MAX has65. Reusing the inclusion limit64 silently loses a valid u64 boundary.
Frontier exports have at most64 hashes. These different bounds must remain separate in public
codecs, work/memory invoices and tests.

## Required regressions

Compare small 0..257 trees against an independent recursive root/proof generator, including every
leaf and all old/new prefix counts. Test exact/short/extra/swapped/mutated paths, wrong context,
leaf/channel/acceptance/index/count, empty/equal/reversed tips, frontier occupancy/root mismatch,
batch ordering/duplicates/count overflow and byte-order golden vectors. Cover powers of two and
u64 boundaries without allocating their imagined leaves: synthetic independently folded subtree
roots pin path shape, proof65 and checked node-count arithmetic. Verify every prepared node's range,
that a bad batch leaves the original unchanged and that batches partitioning the same ordered
leaves produce identical final roots. No whole-log scan or caller-supplied success flag is involved.

The first implementation is a format prerequisite for the upcoming owner log, whose actual consumer
remains an integration gate. It is not production activation or a claim that cold pages, headers,
journal, lookup or migration are implemented.

## Implementation evidence and remaining validation

The declared `chat::acceptance_tree` module now implements these bounded primitives. Actual-diff
adversarial review found no blocker/high/medium. Ten regressions are authored, including exhaustive
small recursive reference paths/prefixes, mutation/reordering/refusal cases, exact append ranges,
partition invariance, 120 internal carries, 65-hash consistency and 64-hash inclusion at u64 limits,
and independently enumerated creation ordinals extending beyond u64.
They have NOT been compiled or executed during the user pause; only source formatting/whitespace
checks ran. An optional low-priority follow-up remains before format release: independently compute
and freeze literal empty/leaf/internal digest outputs alongside the existing frozen exact input-byte
vector. Do not mistake the currently runtime-hashed expected frame for that independent digest set.

The additional proof planner has seven authored/unrun regressions and clear actual-diff review:
independent recursive small-tree plans, complete-node reconstruction into the actual proof verifiers,
hash substitution, canonical range/fold refusal, exact 64/65 depths and checked near-u64-limit node
positions. Source formatting and whitespace checks are the only new executed checks during the pause.
