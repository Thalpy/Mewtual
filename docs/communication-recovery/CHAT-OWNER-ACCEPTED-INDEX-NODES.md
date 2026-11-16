# Exact private index node/query DATA (5 November; authored, unexecuted)

This narrows the [reviewed index architecture](CHAT-OWNER-ACCEPTED-INDEX.md) to the authored codec/search
prerequisite in `chat::cold_index`. No Store/I/O/namespace, balanced-tree writer, actual closed
source, Complete/Current/positive authority or GC/journal producer is added. An expected root is
external DATA; checking it cannot authenticate the caller's source selection.

## Node framing

Framed domain `catcoms/private-chat-accepted-index/v1\0`, u16 version1, framed vault32, group1..256,
ProfileCID32, scope u8 (Directory1 or Actions2 followed by framed AdmissionCID32), kind u8
(Leaf1/Branch2), height u8, rows u16, descendant-keys u128, framed minKey32/maxKey32 and one framed
packed field, complete input. Maximum plaintext16384; maximum complete private sealed frame16470.
All frame/count/length/known-tag checks precede owned group or row copies. Node parsing borrows
group/packed/value bytes and owns only fixed metadata. Descendant count must be1..u64::MAX.

Leaf rows are packed key32, value length u16, exact closed value bytes<=384. Count1..32; height0;
descendant count==rows; actual first/last key==header min/max; keys strictly increasing. Root
placement permits1..32; selected child placement additionally requires>=16. Branch rows are
lowerKey32/pageID32/subtreeCount u128 (80 bytes). Count2..192; height1..9; lower keys strictly
increasing, first lower==header min and last lower<=header max; page IDs unique; positive checked
child counts sum EXACTLY to header count<=u64::MAX. For child placement require>=96 children.
Each descriptor count must meet minimum nonroot subtree occupancy for height-1. No list sort or
attacker-count allocation occurs. Root exceptions are untrusted placement DATA, never completeness.

Minimum root keys for height h>=1 is2*16*96^(h-1). Height10 needs22161087866383368192 keys,
above u64::MAX; enforce height<=9 before exponent/child work. Fixed maximum header510,
branch15870, leaf13886 bytes stay below16384. These are per-node logical caps, not host fees.
Fixed256-bit distance checks additionally reject claimed key counts that cannot fit inclusive
min/max or a child's inferred separator interval.

## Closed directory values

Directory value starts state u8: Unknown0, Empty1, Present2. Present adds root pageID32,
height u8, descendant count u128, minKey32/maxKey32 (113 bytes). All states then carry
acceptedLeafCount u64, uniqueActionCount u64, acceptedRoot32, OWN tipHeader32/tipBody32,
prefixManifest32, positive prefixSequence u64 and prefixPrivateHead32. Maximum298 bytes.

Unknown is ALWAYS incomplete even if counts happen to be0. Empty requires leaf/unique counts0,
acceptedRoot==TreeContext(Profile,leaf Admission key).empty_hash and tipBody==that Admission.
Present requires root count==uniqueActionCount>0 and unique<=acceptedLeafCount, known height/min/max.
These are DATA consistency checks, not actual prefix closure. Empty labels cannot supply root,
actual Admission, selected owner-header/cut, membership or source provenance. Directory-key miss
is always NoDirectoryEntry/Incomplete; it NEVER implies the action is absent.

## Closed action claim values and semantic receipt

Actions-scope payload, in order: recordCID32, ordinal u64, channel u128, acceptingHeader32,
positive ownerHeaderSequence u64, AppendBody32, acceptanceManifest32, positive manifestSequence u64,
recordPage32, recordRow u8(<64), headerBodyPage32; retention tag Unknown0 or Known1 followed by
retentionManifest32/originalActionDigest32/envelopeLength u32(1..MAX_ACTION_BYTES);
semanticReceiptDigest32, positive occurrenceCount u64, hint Canonical1/Incomplete2/Conflict3.
Known-retention length343; Unknown length275, both<=384. Canonical requires Known retention.
Hints are disposable DATA, never actual historical validity; incomplete/conflict stays a present key.
The first-source fields describe the earliest summarized CLAIM even for incomplete/conflict;
they do not silently select a later valid claim or present its receipt as authorized. Canonical
valid-alias selection remains the actual producer/replay's separate contract.
Unknown/equivalent/missing evidence does not become a miss. When reached from a directory entry,
ordinal must be<acceptedLeafCount and occurrenceCount<=acceptedLeafCount; inner root count must
match that entry's uniqueActionCount before following the path.

Receipt digest hashes `catcoms/private-chat-accepted-receipt/v1\0` plus fixed raw Profile32,
Admission32, Action key32, recordCID32, ordinalBE8, acceptingHeader32. Recompute against actual
scope/key; no actual signature, record, ordinary rights or result source follows from equality.
Full actual source validation remains mandatory before any real accepted receipt/projection.
Original headers/records/tree leaves retain all aliases. Only actual producer/replay can choose
earliest canonical valid alias, detect different bindings or close a prefix; this codec cannot.
No opaque transport/native/SDK result, secret provider or public wire encoding is introduced.

## Selected-path API and ciphertext binding

Use bounded public DATA specs/refs, not proof callbacks or `complete=true`. Authored interfaces:

* `NodeFrame::decode` borrows complete structural/plain data; root/child placement consistency is
  separate from actual source authenticity. Header/value constructors validate this same grammar.
* `open_query_data(dbkey, QueryRootData, frame)` rejects frame>16470 and invalid expected bounds
  before `open_private_chat_page`, uses the SAME expected complete ciphertext ID, then parses and
  compares context and ALL expected root metadata. The owned output keeps one zeroizing plaintext
  and fixed expected/query metadata; the group is borrowed from that owned immutable plaintext,
  avoiding another group allocation. `node()` borrows from it without a retarget parameter.
* A query owns its original fixed key/root-ID DATA and one opened node. `search_data()` takes no
  replacement key/root and returns fixed DATA: child step; directory entry; action claim; NoDirectoryEntry;
  or NoActionKey. The latter is structural nonmembership consistency, NOT complete absence.
  A directory Unknown never returns Empty, and a claim hint Incomplete/Conflict never returns NoKey.
* Child step is privately constructed from that node's selected packed row, with inherited context,
  SAME query key/root ID, expected ID/count/height, exact lower key, optional next-lower exclusive bound and parent-max.
  `ChildStepData::open_data` checks complete frame ID/context and actual child height==parent-1, count==row,
  min==row lower; max<next lower for nonlast, or max==parent max for last. Child occupancy rules
  apply. The opened child continues the same fixed query, with no replacement query parameter.
  Shared immutable children can legitimately appear under multiple roots; the codec checks this
  path's exact relation, not global root provenance. A plain caller-supplied root is never authority.
* A directory hit derives an Actions root DATA ref for its own actual key, never a replacement A
  argument. Unknown/Empty/Present remain distinct. Its selected leaf/unique counts are carried
  through the Actions path so reached claim ordinal/count bounds can be checked. Root/open helpers
  cannot authenticate the claimed OWN tip or producer provenance; actual captured source must do it.

Branch search chooses greatest lower<=key and opens that child. Gaps within inferred intervals
are checked against actual child content; do not infer a nonexistent key just from two separator
labels. Outside the node's authenticated min/max can return structural miss DATA. Nonmembership
requires the actual captured root/closed prefix/path at the consuming Store; no constructor here
creates that capture. Wrong expected root, profile/vault/group/A/scope/page/kind or child edge fails.

## Encoders, ownership and work

Leaf encoder takes<=32 ordered typed key/value DATA refs. Branch encoder takes<=192 bounded
child DATA refs (context, page ID, exact summary); verifies all contexts/height/order/intervals,
duplicate IDs and sums before output, deriving its header max from the actual last supplied ref.
Refs are DATA, not evidence that those bytes exist. No RNG is used by node encoding; existing
private page preparation handles caller-funded sealing/RNG separately. A retry must retain its
same prepared frame at the actual producer boundary, absent here.

One encoder output<=16384 plus one value scratch<=384 and fixed metadata. Borrowed parse allocates
no full decoded row vector; fixed-size values are streamed one at a time. Duplicate page
check is bounded<=18336 fixed comparisons. Open overlaps original raw<=16470, copied ciphertext
<=16400 and SDK plaintext capacity<=16400 (49270 requested buffers). During child open, the step
borrows its parent's group in the parent's retained plaintext: add that parent plaintext capacity
<=16400, for65670 requested buffer bytes for the transition, plus fixed descriptors. Iterative
callers can drop the old query after the child opens; retaining more ancestors adds EVERY retained
plaintext separately. A directory-derived Actions root borrows its directory query likewise.
node()/search_data() reparse bounded nodes; they do not promise zero scans.
Crypto state, allocator/actual capacities, encoded original input, source/fee,
public signature/Action data, journal/candidates, dirty pages and SDK/global heaps are separate.
Debug must omit group/raw payload/claim identifiers and expose only kind/height/count/state/length.

## Evidence and remaining gates

Ten authored regressions use real sealed root/child/directory/Actions nodes with supplied
deterministic RNG. They cover
all32/192 maxima,16/96 child minima and root exceptions; full truncation/trailing/length/tag/order/
duplicate/sum/overflow/height/interval/min/max validation; expected-ID/context/summary substitution;
last-child parent-max; wrong root candidates; Directory Unknown versus Empty, claims incomplete/
conflict versus absent; receipt scope/key/hash changes and ordinal/occurrence bounds. Synthetic
tree/layout nodes are DATA tests, not actual accepted-history/Store evidence. Parent grants,
balanced source producer, captured root/prefix closure, WAL/index seed, root repair/GC, native
projection and publication remain unimplemented. All compilation/tests/builds stay paused.
The optional stricter rule ordinal+occurrenceCount<=acceptedLeafCount is enforced with checked
addition for ALL claims in every reached leaf, even an unselected row, before returning query DATA.
Direct rustfmt/diff checks only; all ten regressions remain **UNRUN**. Read-only concrete design
and final actual-diff reviews are clear for the stated DATA codec/query checkpoint, with no
remaining blocker/high/medium. This is not compile or physical/source evidence.
Pending focused selector once execution is permitted:
`cargo test -p catcoms-replication chat::cold_index::tests`. Required full backend/frontend suites
remain the primary's later serial validation; no compiler/test/build ran for this checkpoint.
