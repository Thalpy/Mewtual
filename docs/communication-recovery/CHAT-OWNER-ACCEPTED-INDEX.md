# Private accepted-action index (design only; 5 November)

Recommendation: two scoped immutable copy-on-write B+ trees using one bounded node format.
The outer directory maps AdmissionCID to that admission's action-tree root and exactly closed
OWN accepted tip. The inner tree maps ActionCID to its original acceptance/receipt/source binding.
Vault/group/Profile are mandatory page context; Admission is additional inner-tree context.
There is no lifetime64/4096 entry cache, full-history RAM map, last-write-wins update or deletion
of original manifests/records. This document implements nothing and changes no public wire.
The [archive host](CHAT-OWNER-ARCHIVE-HOST-VERTICAL.md) remains the actual consuming owner.

## Why two scopes

One global tree with (Admission,Action) keys is possible, but replacing one incomplete admission
range then needs range-deletion/rebuild or a partial-global-completeness protocol. An admission
directory allows rebuilding/replacing only A's derived tree while preserving B's root and every
historical incarnation. Remove freezes A's selected accepted tip without changing either tree.
A later same-device admission has a different AdmissionCID and a different inner tree.

An LSM alternative needs many authenticated runs and persistent compaction/coverage reconciliation.
A compressed sparse trie has a256-page worst path and expensive arbitrary-key batches. B+ trees
give smaller fixed reads and one implementation for both scopes. No sibling redistribution is
needed: split33 leaf rows into16+17, or193 branch children into96+97, deterministically.

## Exact initial shape and bounds

Use plaintext pages<=16384, sealed through the private page codec (<=16470 complete bytes), with
a distinct framed `catcoms/private-chat-accepted-index/v1\0` domain. Node context is version1,
vault32, group1..256, ProfileCID32, closed scope Directory/AdmissionActions, optional exact
AdmissionCID32 for the latter only, leaf/branch kind, height, row count, checked u128 descendant
key count, min/max32-byte key and one packed field. All counts/framing/caps precede owned copies.
Directory/inner pages cannot substitute for one another. Fixed tags/unknown versions refuse.

* Leaf: <=32 sorted unique32-byte keys, u16 value length and closed value<=384 per row. Nonroot
  leaves have>=16 rows; root leaf may have1..32. Empty inner roots are explicit None, permitted
  only for an actual closed empty Admission prefix, never merely a decoded count0.
* Branch: <=192 sorted children, each packed lower-key32/complete-ciphertext-ID32/u128 count16.
  Nonroot branches have>=96 children; root branches have>=2. Children have height-1, disjoint
  inferred separator intervals and exact descriptor/count/min/max/height binding when opened,
  including the last child's parent-max boundary. Descendant counts must be<=u64::MAX before
  relying on the depth proof. The
  trusted builder derives summaries from actual children; there is no raw summary/root setter.
* Maximum header is510 bytes including maximum group and inner Admission context. Branch maximum
  is510+192*80=15870, leaf maximum510+32*(32+2+384)=13886, both below16384. The codec must freeze
  exact arithmetic beside its implementation, not rely on allocator capacity or these prose sums.

Each inner tree has at most u64::MAX accepted keys because the public per-Admission accepted count
is u64. Directory entries cannot exceed the original positive u64 manifest sequence's Admissions.
A height10 root would need at least32*96^9=22161087866383368192 keys, greater than u64::MAX.
Thus height<=9 and a path opens<=10 pages. These are derived protocol-space bounds, not a new
lifetime-entry quota. Counter/control reserves must stop ordinary work before space exhaustion;
arithmetic capacity is not a prepaid grant.

## Values and the two result contracts

Directory values retain inner root ID or explicit empty, exact closed accepted LEAF count/root, OWN
tip HEADER ID and body ID, and the original finite manifest-prefix/source labels under which
that closure was actually built. Removing an admission does not overwrite this tip with the
unrelated global Remove header. Its selected frozen cut points to this same pre-cut accepted tip.
Historical directory entries stay on disk. Inner subtree key count is uniqueActionCount, separate
from public acceptedLeafCount; reader recovery below can encounter repeated historical leaves.
A missing entry for a known admission means Incomplete.

An action value binds the following original DATA, capped at384 bytes (initial fixed layout<=334):
AcceptanceRecordCID, zero-based ordinal, channel, accepting header CID, Append body CID,
acceptance manifest ID/sequence, acceptance-record page/row, header-body page, prior original-action
retention manifest, original Action-envelope digest/length, semantic-receipt digest, checked
occurrence count and bounded binding/availability hint. All
references are typed IDs, not filenames, URLs or a caller-selected file service. Field values
must be checked against actual original manifests/pages and their complete context and signatures.
Ordinal is Append.before + the actual (channel,recordCID) leaf position, checked for overflow.

**Canonical semantic receipt:** the first text vertical can return the exact tuple
(Profile,Admission,Action,AcceptanceRecord,ordinal,acceptingHeader). It has no time, randomness,
native incarnation or transport/session observer. Its digest uses a separate fixed domain and
fixed fields. It derives deterministically from already retained original Append/record/header
sources. The original signed header representation remains separately retained. A proof built
from original complete-node pages is reconstruction, not a new signature/MLS Welcome/provider.
No new permanent per-message secret SDK snapshot or transport ciphertext is required by this
tuple. A future public reply encoding requires an explicit wire contract; none is added here.

**Nonreconstructible/exact external results:** native terminal/retry and publication debt retain
their exact original bytes/authority until actual settlement under their existing contracts.
An MLS Welcome or another nondeterministic required result cannot be regenerated from this tuple.
If a required original is unavailable, return ExistingAcceptedResultUnavailable/Incomplete and
repair it. That is NEVER Absent and never permission for a second leaf. A future permanently
nonreconstructible result needs an independently retained original reference; a disposable index
or reclaimable settled journal alone cannot supply that guarantee. Do not alter current manifest
roles or permanently retain already-settled provider snapshots to paper over that separate case.

## Completeness comes from the actual producer, not node metadata

A valid MAC/page ID, matching count/root or decoded "coverage" fields cannot create a complete
lookup. On disk the index authenticates its contents; it does not independently prove that a
program inserted every accepted action. The consuming invariant is concrete:

1. Empty creation comes only from the actual private saved genesis/admission source and its
   selected empty acceptance prefix. No decoded Profile/root/count can initialize completeness.
2. Incremental update consumes the same actual original writer/head and previously closed inner
   prefix, verifies ALL original Append records/leaves/nodes and actual selected successor,
   checks every new Action key for prior acceptance, then closes the new inner prefix and outer
   replacement in the SAME covering head transaction. A decoder cannot construct this producer.
3. Rebuild closes the exact finite selected admission prefix by the replay described below.
4. Cold restoration obtains only the index root referenced by the actual supported current
   canonical head/namespace/key/lineage closure. It retains that original mount/pin/group/scope;
   a root decoded from a page, RPC, stale native task or same path reopened is not a root source.

This is locally sealed actual-producer provenance. Public replicas separately authenticate the
selected owner/finalizer chain and ordinary/action semantics; a private index root is not a public
acceptance proof or a substitute for that trust/verification boundary. The current DATA codecs
and ReplayMath alone do not supply the missing production source/issuer.

An actual lookup captures the current directory root AND an independently selected own-tip/cut
from the governor. It checks outer path, directory entry, inner path and exact context. Absence
is usable only when the closed inner tip/header/count/root matches that selected current or
frozen tip, the path is fully authenticated/available and original negative custody still holds.
Missing/corrupt/unknown/stale path/entry/coverage produces Incomplete and bounded repair. Never
fall back to empty. A positive old hit can establish AlreadyAccepted only after its original
record/header/manifest binding is verified; missing required result sources still forbid a new
leaf. A hit from a stale prefix does not turn an unrelated miss into complete absence.

Every incoming Action representation must pass its own real signature before lookup/duplicate
handling, including invalid-first same-CID variants. With a closed valid historical binding, an
equivalent valid signature variant returns the same semantic receipt; it does not replace the
retained original or consume a new ordinal.
Queries derive Profile/Admission/Action from the actual authenticated action/delegation source,
not supplied context scalars. Same-key rejoin, changed realm/grant/basis/body and old UI sessions
cannot borrow another incarnation's result. Epoch changes do not clear accepted history.

### Repeated historical receipts: writer rule versus reader recovery

Normal source writers MUST return the original result, never append a second leaf for the same
valid action/acceptance binding. Historical read recovery is deliberately more tolerant:
replay EVERY original leaf/header, preserve public acceptedLeafCount/root, and for repeated
EXACT VALID action plus canonical AcceptanceRecord binding choose the EARLIEST selected owner
header sequence, then ordinal. This is canonical chain order, never arrival order. Keep
uniqueActionCount separately and preserve all original receipts/records for admin diagnostics.
It recovers past duplicates without rewriting public state or blocking otherwise valid history.

A shared action key with DIFFERENT record/binding, or missing required signature/ordinary/action
evidence, is not silently validated as an alias. Keep the claim and originals, return
ExistingClaimIncomplete/Conflict or unavailable result, and schedule bounded automatic evidence
repair. It is never Absent/new-leaf authority. A known claim remains a key even while validity
or result availability is unresolved; presence coverage and historical authorization are distinct.
Unknown record payloads prevent closing the target key-presence prefix; missing action bodies
may instead leave that known key incomplete while other-key absence still uses a fully replayed
record prefix. A stored hint is not an authority witness. Actual action/ordinary/signature checks
still gate any positive historical result/projection. Known same-parent owner forks remain the
governor's original negative case and cannot be resolved by this sequential-alias rule.

## Finite per-Admission reconstruction

Rebuild one admission, not all histories or all SDK groups in RAM. Capture finite manifest H0 and
actual selected own tip T0 (current or frozen). The original source must bind H0 to the selected
governing prefix; neither an index field nor a peer assertion can supply that relationship.

Phase1 walks H0's exact immutable manifest-parent chain backwards with strict sequence-1/context
checks. Only target-Admission Append inputs need their acceptance/node/original-header sources;
unrelated action chunks/other admissions' record batches are not dependencies. Existing governing
selection/negative evidence remains the governor's job and cannot be cleared by missing bodies.
Target own-tip links must match the captured T0 and each actual Append.previous_tip, not the latest
global header (unrelated admissions may have advanced it). Close that chain at the actual
Admission, while continuing the finite manifest scan to genesis for original-retention joins.

AcceptanceRecord contains an ActionCID, NOT its original retention-manifest/digest locator. The
reverse pass therefore also owns a separate temporary on-disk Need tree keyed by target action.
After an actual target record reveals a needed key, older RetainAction manifests can join its
original manifest/digest/length without opening unavailable action chunks. Prior covering retention
must precede acceptance. Preserve the first-covered-original local policy: reverse scanning may
replace a tentative newer locator with its earlier canonical one, never choose by arrival or
silently overwrite a different action/acceptance binding. Original retention verifies each actual
representation's framing/signature; this alone grants no ordinary rights or historical authority.
The positive producer/historical action+delegation checks remain mandatory. Later audit still
verifies actual bytes.
An unresolved required locator after a complete scan is Incomplete/integrity evidence, not empty.
This is another actual bounded job/index source dependency, not a pointer invented from the record.

Store a bounded reverse-source spool, not an in-memory lifetime vector: <=64 entries per<=8KiB
encrypted work page, each manifest ID/sequence and accepting-header ID, sorted oldest-to-newest
within the block. Each newly completed older block points to the prior newer block. The final
oldest-block pointer therefore drives forward replay. The job checkpoints its partial block,
next manifest ID/sequence, captured H0/T0 and original spent grant; restart does not rescan from0.
No original manifest or action is moved/deleted. Missing target proof pauses this admission.

Phase2 starts at the actual selected Admission's empty Frontier and walks the spool forward.
For each target Append, check exact previous OWN header/tip; real Header/body/state binding;
actual RecordBatch leaf bijection/context; prepare Append on the internally derived Frontier;
and compare every original complete-node hash/ordinal. Resolve each retention locator from the
Need tree and insert/fold the actual action claim under the exact historical-alias rule above.
This recomputes the entire target accepted count/root from original records, not a metadata count.
Original action-chunk unavailability is audit/result unavailability, not acceptance absence;
the owning signed-finalizer/local-producer acceptance attestation remains a separate assumption.
At finish, derived LEAF count/root/OWN header must equal captured T0; distinct action keys have
their separate count. Conflicting originals stay retained for admin diagnostics; that key's new
acceptance/result stays held pending actual repair. No arrival order resolves a binding conflict.

The checkpoint retains only one Frontier<=64 hashes, current own header/tip, working inner/Need roots,
spool cursor and exact source/grant/counter bindings. It is private authenticated job state with
an actual Store recovery consumer, not a public raw-snapshot-to-Current/ReplayMath initializer.
Both current/stage must be inspected before settlement. Cold resume must validate the same finite
source and restore already spent execution, not manufacture a Fresh/current or retry allowance.
Those job/source/namespace/grant codecs and consumers remain prerequisites, not implemented APIs.

Final reconciliation captures the actual current own tip in a NEW original invocation. If it is
still T0, unrelated global headers/retained messages do not require rescanning them. If it advanced,
capture a finite tail T1 and replay only that tail without discarding completed progress. The
actual sole finalizer must suspend new acceptance for an admission whose lookup is incomplete;
ordinary retention, independently proven control and other admissions/groups continue. A removed
A's unchanged frozen tip can complete; A2 is a different directory key.

Installing the derived A root copies only the current directory path, preserving all other roots.
It uses actual current-head CAS, never adopts a later head after await. A known stale CAS retains
the completed inner candidate; the next invocation obtains its own original writer/scope/negative
capture before rebasing the directory. Derived-only repair changes private head/index generation,
not public state or original manifest history; it needs an explicit private head operation and
control/repair counter admission. The host's canonical format is still a prerequisite.

If the directory root itself is missing, all queries depending on it are Incomplete, not an
empty-directory initialization. Per-admission replacement assumes a readable/authentic directory
path. Whole-directory recovery needs its own finite job: retain the damaged original root as
unresolved reachability debt, reconstruct known directory entries from actual sources, and never
treat an unrepresented historical admission as empty. A partial replacement directory may expose
only independently closed admission roots; absent directory entries STILL mean Incomplete, and
unknown old reachability forbids GC. Its exact partial-directory/debt source contract must be
reviewed before implementation. Independently proven control and other groups still progress.

## Concrete I/O, memory, execution and journal invoice

A negative decision opens<=10 directory +10 action pages (<=329400 sealed input bytes total),
streaming paths; it need not retain20 full pages. The actual read helper must reject file/frame
length>16470 BEFORE raw allocation/unseal. Existing private-page opening can simultaneously hold
raw16470, copied ciphertext16400 and SDK plaintext capacity16400:49270 requested buffer bytes per
open, plus fixed crypto/metadata/path descriptors and output. Actual Vec capacities must be paid
at the Store call; these are not an allocator/RSS claim. Retaining10 ancestor plaintexts during
mutation adds<=163840, and node preparation separately overlaps plaintext16384, ciphertext16400
and final frame16470. Holding dirty/candidate/journal copies adds their FULL enumerated allowance;
do not replace it with a rough multiplier. A receipt hit adds acceptance manifest, record batch, header/body
and retention-manifest reads. Full original audit adds<=3 Action chunks. Public inclusion proof
adds<=64 hashes; with a future<=10-page node-range locator, pessimistic uncoalesced proof work is
64*(10+1) opens, separately budgeted (<=731 opens including the20+4+3 above). That node locator
and privileged proof reader are not implemented by this index proposal.

Insertion into a height-D tree creates<=2D+1 pages including splits/new root. Directory coverage
replacement is fixed-size at an existing key, so it copies<=D pages and does not split. With D<=10,
K accepted keys create<=21K+10 frames:31 for K1,1354 for K64. At16470 bytes/frame these are510570
and22300380 bytes. Coalescing can reduce this, but admission must use the conservative bound until
the real builder proves tighter. New admission-directory insertion is a separate operation and
can split; do not silently count it as fixed-size replacement. Genesis must create its actual
empty coverage entry. The first Create-only host may choose K1; this is batch policy, no new
public64 limit or lifetime cap. It needs a SEPARATE index candidate allowance (at least its exact
framing beyond510570), not the already occupied four-page archive512KiB journal.

The actual journal atomically covers BOTH archive originals and index replacement, but index
candidates have their own bounded inventory/complete-original seed, byte/file counters and fee.
For K64, even one index candidate copy is>21MiB; seed duplication/read/unframe/private SDK/public
records/hot replay/receipt buffers overlap and are not paid by archive512KiB or founder64KiB.
Preparation can stream separately durable seed packs with a bounded root inventory, or retain the
complete funded candidate in RAM for the first implementation. Either choice needs actual exact
partial-seed/current/stage recovery before use. A checksum/list of absent candidate bytes cannot
repair them. No index child write may begin until its original complete seed/allowance closure
is durable. A partial seed before child admission requires its separately proved unstarted
negative transaction; do not invent another index candidate or refund ambiguous work on restart.

Before any work, an actual persistent original operation/job grant binds group/Profile/Admission,
parent head/index, finite H0/T0 or exact Append, request identity, page/byte/read/write/copy budgets
and already-spent execution. Debit a bounded quantum BEFORE its admitted disk/encoding work;
escrow e.g.8 reads/4 page preparations in one durable transition instead of syncing counters for
each individual comparison. Crash-spent unused escrow is not silently refunded. Restore consumption
after crash. Hot reservations pay raw/unframe/plain and candidate ownership independently of
decoded grant numbers. Bound active jobs/requests under the actual group/session scheduler.
Suggested preparation quantum<=8 reads/4 new pages; no ordinary head mutex through an entire
scan/batch. Index encoding itself loads no SDK provider; the host's actual signer/provider/S0/S1,
typed SDK heaps and full-action verification buffers remain separately owned/paid.
Rebuild also retains a temporary Need tree and new inner tree alongside the old pinned inner
tree and paged spool. Their complete disk/candidate/checkpoint roots require separate job quota;
do not hide the second working tree in the ordinary one-Append invoice. Persistent aggregate
work/byte counters use checked wide arithmetic. An exhausted grant stays exhausted; an actual
policy-admitted extension is a separately recorded grant, not a restart/refund or a lifetime
entry limit. Missing sources back off under that same job instead of hammering storage.

Required Remove/retirement/finality must not wait for this ordinary scan or allocate/update the
accepted index: freeze the already independently selected accepted count/root/tip and retain the
last index root. Preempt the ordinary continuation and its unexposed CAS under original negative
custody; preserve exact grant/debt/candidates and all retained actions. Once a physical write was
admitted, close waits for its actual result. Missing essential governing/key proof still holds
the dependent control; a reserved counter alone cannot promise success on a genuinely failed
filesystem. Ordinary bytes/files/head revisions cannot spend actual required-control reserves.

## Reachability and derived garbage collection

Original manifests, acceptance/Action/header/body/node/cut/decision pages are NEVER index garbage.
An old derived page can be reclaimed only after exact current directory, both pending journals/
stages/seeds, durable rebuild/checkpoint roots and active finite-reader roots no longer reach it.
Root pins have actual bounded owner/negative custody, not a lifetime RAM map. Mark/sweep work is
paged/checkpointed; a root-generation barrier registers new roots/pages before releasing old pins.
Uncertain/unknown sources are retained. Actual file identity and final root-generation check
precede unlink under original Store custody; no raw path/BlobStore cleanup escape is permitted.
Compaction only produces another derived root from a closed source and crosses the same barriers.
No-GC prototyping may be safe under explicit ordinary quota, but is not long-lived scalable signoff.

## Proposed private consuming seams and remaining decisions

These are future Store-internal operations, NOT declared/public proof constructors:
capture actual lookup source; advance bounded directory/action path; verify original hit/receipt;
prepare same-writer Append/index replacement; checkpoint/resume per-admission rebuild; and install
closed rebuilt inner root under a new actual current capture/CAS. Read results distinguish
KnownAcceptedReceipt, ExistingAcceptedResultUnavailable, ExistingClaimIncomplete/Conflict and
Incomplete. Only an unconstructible
internal completion from the actual closed source/path may authorize a new acceptance; no public
Verified/Complete/paid bool or generic caller callback substitutes for it.

The concrete boundary must accept the actual owning V2 host plus Store, not a generic verifier
trait. Before await it captures that original supported canonical head/directory root, mounted
key/Profile/full signer, selected admission tip/cut, native/group negatives and real read fee.
The path cursor retains exactly that capture and takes pages only from fixed-name Store reads
derived from its actual opened parent IDs. A miss is consumed inside the same actual actor when
preparing an Append; final admission compares the SAME head/root/scope under existing lock order.
Source movement returns retry/incomplete, never an old task sampling a replacement. Rebuild
installation uses a new invocation's own original capture. Each authenticated reply retry captures
its own original live responder; durable historical receipts do not retain a canceled waiter.

Priorities before implementation: (1) agree the semantic receipt/private value format and actual
head/index source interface; (2) exact node codec and actual closed-producer/update path; (3) separate
index grant/seed/journal crash closure; (4) real per-admission replay/checkpoint/install; (5) exact
GC/reader roots; (6) native/network integration. No public record wire or manifest-role change is
needed for the recommended semantic tuple. A genuinely nonreconstructible result is a separately
reviewed contract, never silently folded into the canonical receipt.

Required adversarial evidence includes genuine Action/Header signature variants; stale/corrupt/
missing/foreign directory/inner paths and coverage; same-key rejoin/epoch/control interleavings;
mixed A/B OWN tips; exact-valid sequential aliases selected by canonical header/ordinal while
retaining every leaf; different binding/missing evidence held per key; original-retention joins;
separate distinct/leaf counts; result unavailable
without second leaf; minfill/root split/height/capacity and maximal batch invoices; exact same/cold
candidate retry; crashes at grant/seed/page/root/head barriers; cursor/counter persistence and
source retirement; changing heads without rescan starvation; active-reader/GC races; missing old
audit chunks with control/other-group progress; histories well beyond4096 and20/50-peer bubbles.
Synthetic layout tests are not actual signed-history acceptance. All code/tests/builds remain
paused; no index implementation, compiled test, crash result or completion claim exists here.
Read-only architecture review is clear for the two-tree/source/replay/result/preemption design.
Before codec source, the exact bounded grammar, actual root/partial-directory recovery interface,
grant/seed invoice and concrete GC ownership must receive their own design and actual-diff reviews.
