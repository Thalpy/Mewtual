# ID3 physical declaration, group exclusion and historical DATA reader

ARCHITECTURE DESIGN review CLEAR after group-wide JOIN, explicit paid read-only canonical
inventory and serial transient-fee corrections. The physical inspector/declaration, opaque
historical DATA reader, complete catalog preservation and constructor/index maintenance are
now authored source. Full actual-diff static review is CLEAR after the declaration-identity,
policy, visibility, metadata-accounting and documentation fixes; primary serial compiled
validation remains required.
Preliminary review corrected late marker/seed/stage identity ordering and an old opener's
missing constructor argument. The namespace codec now has the real opt-in reader consumer;
pin/head/activation codecs remain test-only. No active application Current is claimed.
This implements the namespace/presence part of `CHAT-OWNER-V2-GENESIS-ACTIVATION-FORMAT.md`,
not its Current pin/head/scope/node activation, governor, operation or native path.

The present implementation slice has the physical namespace-only declaration/opener,
opaque historical DATA reader and paid old filename/index maintenance. It has NO group mkdir
task, strong pending-group registry, saved group-directory receipt or typed vacancy/Initializing/
Current guard producer yet. Those required next boundaries remain specified below and must not
be inferred from the metadata reader. Fifteen regressions are authored UNRUN (eleven namespace,
four index); the further group-writer/race matrix below is future required coverage.

## Ownership and concrete vertical

Factory owns `critical_namespaces/current_layout*`,
`critical_namespaces/archive_current_namespace_codec.rs`, the exact physical child
`store/persistence/current_layout.rs`, narrowly required Store policy/state/constructor hooks,
the fixed negative group registry under pins and all old-protocol group-presence checks.
Authority owns private pin/head/activation DATA codecs. Root owns the real paid historical
source reader and subsequent SourceSaved/seed/pin/head/scope/Current coupling.

Public shape:

```text
CanonicalFilenameInventoryPolicy::new(max_files, max_file_bytes)
    -> Result<CanonicalFilenameInventoryPolicy, AppError>
V2CurrentLayoutPolicy::new(ArchiveBootstrapCatalogPolicy,
                          CanonicalFilenameInventoryPolicy, persistent_capacity)
    -> Result<V2CurrentLayoutPolicy, AppError>
PreparedV2CurrentLayoutStore::open(path, passphrase, rng, policy)
    -> Result<PreparedV2CurrentLayoutStore, AppError>
prepared.retry(rng) -> Result<(), AppError>
prepared.into_current_layout_store() -> Result<V2CurrentLayoutStore, SAME prepared>
current.read_saved_bootstrap_source_data(group, signer)
    -> Result<RetainedBootstrapSourceData, AppError>
```

The Current policy includes Root's actual saved-source DATA-read configuration before opening.
There is NO public raw ServerStore getter/conversion, Deref, arbitrary callback, Current
constructor or group-write method. Root's actual private factories consume a checked
crate-private Store borrow. The historical reader is a real existing paid DATA consumer,
not a fake caller to suppress dead code. Its policy is installed before common opening.
Default, Pending and bootstrap-only constructors retain their old supported grammar and
refuse ID3 current OR stage, the new seed and marker BEFORE older cleaners. No exposed Store
flag upgrade occurs. Existing ID2-only source work closes before a new physical declaration;
new captures are paid and cannot adopt its old creator/arm/SDK payment.

## Formats and bounds

The Authority handoff codec is copied byte-for-byte before edits (SHA256
E8645E70DBA109535596A5B67A3D63EC54660674DEC4A10263761F30305B9E48).
`CurrentLayoutData {vault,root}` lives in `archive_current_namespace_codec`, shared by the
actual opener and the pending private metadata codecs. Preserve the reviewed exact ID3/v1
descriptor122B, marker174B/cap1024/frame1064, fixed declaration-seed grammar/cap C+4096,
frame C+4136 and separate KDF domains. ID2 root identity/marker never changes.

Use the existing full catalog cap C=16MiB+4096+512+40. The combined ID2+ID3 descriptors fit
the existing4KiB reserve. Parsed/Summary include immutable ID3 DATA and original borrowed
payload; all JOIN re-encoders append its exact bytes. Strict Pending/bootstrap parsers reject3.
The actual Current policy alone chooses the closed combined parser before BOTH current/stage
classification. Adding3 requires exactly one checked-generation/whole-parent successor with
UNCHANGED complete JOIN version/payload and unchanged ID2. Later transitions preserve3 forever.
No field of any JOIN source/execution counter is reconstructed, truncated or reset.

## Constructor and declaration ordering

The new physical opener retains the EXISTING4096-byte base-path cap of the ID2 opener;
unsupported larger paths refuse before effects, never clean/reset. New child selectors still
use the reviewed checked32KiB+256 hard ceiling, but this physical wrapper does not widen the
old ID2 declaration/mounted-metadata invoice. The actual declaration selectors use the smaller
4096+256-byte ceiling; the broader bounded filename-walk/native conversion scratch is paid
separately. Root approved the compatibility/profile choice.

The declaration metadata assertion names five simultaneous fixed selectors, two additional
nested Root selectors, a32KiB native path-conversion phase, and the actual parsed/summary/File
layouts within its256KiB allowance. Mounted marker ownership is declared before Root acquisition
so every refused competing install drains its last fee only after that writer has dropped.
The new physical opener reserves only separately named small wrapper/index metadata before
its initial boxes, and sets archive, source-reader and Current policies before the common Store
constructor. ID2 and ID3 LARGE declaration debits are SERIAL: finish/classify the ID3 pre-cleaner
probe and drop its transient allowance before entering the old ID2 gate; finish/release the ID2
private declaration owner before reserving ID3 preparation. Each phase reserves its own full fee
BEFORE its buffers/candidate/box/RNG/effects. Retain only mounted metadata and independently paid
index/source objects between phases. Do not silently hold both ~192MiB fees while claiming one.
The existing OS session excludes old physical writers. No Store escapes while incomplete.

Before wrapper/new-DEK creation, include ANY presence of
`chat-critical-namespaces.current.seed` in the fixed orphan guard. Dangling/nonregular/partial
presence is not absence. After wrapper authentication and before old cleanup, inspect complete
catalog current AND stage, the new seed and fixed `chat-owner-v2/current-layout.bin` presence.
Unknown version/namespace/malformed source refuses without deleting it. A missing/old catalog
cannot hide a newer fixed marker/seed. An authentic old-wrapper/catalog-component rollback
remains the documented same-DEK residual, not monotonic protection.

First creation is two serial transactions on the SAME still-private physical owner: complete
existing ID2 declaration/root/parent and settle its seed, then prepare ID3. Simultaneous ID2
and ID3 declaration seeds refuse. No founding, group mutation or source capture precedes ID3.
If ID2 is already present, retain exactly its original root and actual marker/directory.

Prepare complete exact descriptor/marker/catalog/seed ciphertext ONCE before first effects.
Order: independent seed file+vault-parent -> actual marker file+Root-parent -> shared catalog
stage/file -> catalog current+vault-parent -> exact seed settlement+parent. Cache every
opened created File before its first fallible write/identity query. Warm partial repair uses
the SAME held handle, original name/session identity and exact candidate-prefix check, never
truncate/reopen/reseal/RNG. Foreign/replaced/wrong-prefix sources refuse. Cold repair requires
the complete independently authenticated seed; unknown/partial first seed remains preserved.

Use distinct private classification roles: actual selected original parent, or actual current
already equal to the seed's COMPLETE candidate ciphertext. The second may re-sync and settle
that metadata but MUST NOT synthesize an old Summary and call it prior authority. First-add3
current/stage with no complete matching seed refuses. Once3 is fully current, ordinary legal
JOIN successors may settle only with exact unchanged ID2/ID3 and whole-parent checks.

The mounted namespace owner holds the actual marker File, SessionId, complete frame digest,
same Root/mount identity and original negative through Store Drop. Root's actual directory and
ID2 marker stay independently held. Marker disappearance/replacement/corruption refuses;
neither decode success nor matching path is a new mount/Current grant.

## Group negative registry and disk fence

Group path is exactly `g-<64 lower hex>` under the SAME ID2 Root using the reviewed hash over
vault/root/full group. Signer is NOT part of it. Any kind of group-directory presence fences
all V1 signers, including empty/unreadable/nonregular/reparse/link-shaped entries or missing pin.
Never enumerate lifetime group directories. Check this group name even on a new-code ID2-only
mount, so a rolled-back catalog/marker cannot turn a known group reservation into absence.

A fixed16-slot unresolved-work registry under the actual pins serializer stores independently charged inert
negative debt, keyed full group/vault/root/original mount. The actual owning task holds its
non-Clone identity; each inert slot gets a separate actual metadata debit (exact fixed Entry/
Arc layout plus bounded256B group, no source graph/key/task sender) before writers; Store stores no creator sender, source graph, transport, key or session.
Owner Drop NEVER erases an entry: a blocked admitted mkdir may still complete. Registration
runs under actual admission -> pins BEFORE directory effects, after final source/mount checks.
Prepare all fallible metadata/payment outside writers; install entry+return same task with no
fallible ownership gap. Same labels/weak expiry never renew or replace it.

Retire a scarce unresolved slot only from the actual same held permanent directory+parent
barrier result (or a separately proved no-effects path), not a caller boolean or Drop. The
permanent directory is never removed and takes over negative fencing. This bounds simultaneous
unresolved work, not lifetime groups. Real directory I/O additionally requires original source
custody; the registry token proves only exclusion and is not a SourceSaved/Current permit.

Actual input to group preparation is `&RetainedBootstrapSourceData` plus
`&UnboundV2GenesisSeed` and the concrete mounted-layout/admission/pins vacancy guards. Compare
complete source ID/context/verified public genesis using Sync's new NEGATIVE
`seed.check_opened_source_data(read.data()) -> Result<(),SyncError>`: borrowed exact retained
Profile/Admission/Header including signatures, image and frontier, no encode/toVec/SDK/pool wait
or mutation. It returns no SourceSaved/Current token. Then call read.validate_original
in admission -> pins -> canonical -> Root order, then register. Root's eventual stronger
GenesisActivationAdmission separately authorizes pin/head/scope effects. For I/O already holding
Root, use Root's concrete
`read.validate_under_archive_root(store,&MountedRoot)` against that already-borrowed Root;
never recurse `read.validate_original`, which takes Root itself.

Old capture/read/positive/unexposed-negative paths check the group registry+directory and actual
mandatory marker before the old per-signer target test. Pin-only entry points add admission
before pins where needed; callbacks already holding admission use a borrowed helper rather
than relock it. Ordinary deliberate retirement/control and unrelated groups keep their existing
negative semantics. Saved-label equality or a dirty-inventory exception cannot bypass the fence.

## Startup-only old-head attribution index

The present old canonical filenames/HeadCell keys hash(group,signer) and store no group. Direct
pin lookup plus a group-prefix BTree admission lookup O(logN) excludes legitimate other-signer
lineage but cannot attribute an orphan .bin/.replace.tmp after missing row/pin. Root approved a
separate actual shared-pool index reservation BEFORE guards: requested capacity
`MAX_CHAT_ADMISSION_IDENTITIES * size_of<[u8;32]>()` (65536*32=2MiB), plus exact index/Arc/fixed
metadata. The new Current policy owns this separate prepaid allowance through mount/index Drop;
it is not hidden in the64KiB mounted-marker invoice.

On the still-private physical opener, build sorted unique canonical identifiers from the ALREADY
loaded non-evicting admission rows using one exact-capacity Vec. Hash every original group/full
signer key, then `sort_unstable` (no hidden stable-sort allocation). This is a one-time bounded
startup traversal, not a per-group/lifetime hot scan. Hold actual admission while reading keys;
refuse dirty, missing or unsupported inventory. No decoded flag issues the index. Keep its
original physical identity and data/fee lifetime. Positive V2 group reservation later requires
that same installed index, current loaded clean inventory and its key-count/generation invariant.

The existing separately bounded canonical inventory classifies each encountered filename by a
binary lookup in this index. ANY unmatched old canonical/stage key closes NEW V2 genesis vacancy
globally until actual classification/reopen; unrelated already-proven V1 communications do not
consult this global unknown closure. Never manufacture empty canonical inventory when budget
or classification is missing. No extra unbounded directory enumeration or per-action scan.

Every new admission key enters the fixed-capacity sorted index BEFORE any first corresponding
head side effect. Existing row phase/revision changes do not remove membership; Retired identities
remain non-evicting. All actual head-registration/restore paths must require matching known key
under their already-held admission guard when Current policy is active. Legacy policyNone behavior
is unchanged. Refreshing a table/index invalidates any prior canonical association witness until
real bounded inventory reclassification. Use private constructor-only index/association objects,
not an externally supplied complete/dirty boolean. No stale/missing index may prove vacancy.

The trusted constructor policy supplies explicit `max_files` (1..2*65536) and positive file
size cap no larger than the existing144MiB canonical maximum plus its fixed wrapper/frame.
Use a separate read-only fixed-name/metadata pass; do NOT call configure_owner_evidence_budget
or create HeadCells/BTree inventory just for this negative proof. No old owner-state mkdir,
cleanup, content decoder, checkpoint issuer or resource default is enabled. Every actual
encountered filename is checked against the sorted keys; count every `.bin` AND `.replace.tmp`
entry, so no unique-head map/dedup allocation is needed. Malformed/nonregular/orphan/overflow or
incomplete traversal gives an opaque Unavailable association, never empty/complete. A KNOWN
well-formed .replace.tmp key belonging to unrelated B may remain preserved without blocking A;
only unmatched/unsupported/unattributed stages close new vacancy globally (Root clarification). Proven
unrelated V1 operations do not consult that global closure. The actual SDK budget supplied by
Root's retained-source/seed wrapper is independent of this policy.

Pay the named temporary filename-proof scratch separately before guards: at most three checked
path capacities (<=32KiB+256 each), two bounded filesystem-component copies (<=64KiB each), and
exact iterator/identity/codec/fixed metadata. Source audit must confirm actual platform `ReadDir`
Rust copies; OS/C iterator buffers/allocator/RSS remain outside this logical invoice. No retained
per-file collection is made. The completed private association retains only original directory/
mount identity, same row-index identity and its actual complete-traversal provenance, under the
mounted metadata allowance. No budgetNone/runtime empty map proves directory absence.

Maintain that invariant for legal writes rather than rescanning lifetime history on each save:
only actual admitted known-key effects may advance the opaque association's stamp; each new
key is inserted BEFORE the corresponding first head effect. Scope phase/revision changes retain
keys. Missing calls/unsupported table replacement/index inconsistency/overflow latch V2 vacancy
Unavailable; no raw status can reinstall it. Known unrelated V1 operations may proceed while it
is Unavailable. The actual row/permit/head filesystem call paths and each mutation hook must be
listed and reviewed; a generation comparison alone is not permission to skip classification.

Under the SAME admission and BEFORE pins, inspect current + Store-admitted pending.expected/
pending.next fixed-five JOIN summaries for ANY retained source of this group, irrespective of
signer, scheduling/task/weak lifetime or execution phase. Existing per-signer JOIN exclusion is
insufficient. Such a source can predate all local rows/pins/heads and must not be stranded by a
new permanent V2 group fence. Negative/control/exact repair and other groups remain unchanged.

The final group predicate is: no retained/admitted group-wide JOIN source; exact clean group-prefix admission absence; no existing V1 group
pin; complete SAME-index canonical association with no unknown heads; no own fixed old canonical
current/stage; no active/inert group registry entry; original ID3 marker/Root and fixed new group
path absent. The admission/pins/canonical guards remain owned through registration and the Root
writer before directory effects. Existing v1 pins/rows of ANY signer or orphan hash files refuse
new V2 genesis rather than reset history. Whole-directory deletion remains the explicit external
rollback residual.

## Resource and lock contract

Actual declaration fee `12*(C+4136)+256KiB`, separate64KiB mounted marker/namespace fee,
independent2MiB row-key index+fixed layout and the named read-only filename scratch come
from the SAME provided CriticalCatalogReadPool before work. Existing catalog JOIN reads and
ID2 declaration/source/SDK fees stay independent; serial ID2/ID3 phases should release their
completed transient allowance before acquiring the next, while retaining mounted files/debt.
Persistent content reserve covers3*C+4136+1064 plus existing ID2 marker and ordinary capacities.
Actual fixed layouts and simultaneous raw/copy/cipher/plain/seed/candidate/return copies require
source-level assertions. Use checked complete PathBuf capacity, never base.clone().join growth;
keep retained paths out of mounted metadata and bound transient overlaps explicitly. New group
debt fees outlive its task until real directory/no-effect handoff; drop files/bytes before fees.

No resource lock spans allocation/I/O/callbacks. Semantic writes follow admission -> pins ->
canonical -> Root -> local debt; no pins guard reacquires admission and no Root guard reenters
itself. Actual OS session custody lasts through each admitted physical write and its repair.
No declaration/group callback stands in for an original source or a real file/parent barrier.
First Current later reuses the reviewed paid synchronous scope writer. Scope I/O fairness stays
OPEN; no new persistent scope grant or dirty-table bypass belongs to this packet.

## Required authored regressions

Default/Pending/ID2-only unknown3 current OR stage before cleaner, fixed marker/seed rollback
presence, PRE-DEK dangling/orphan seed, full JOIN v1/v2 preservation, unchanged Root2 ID,
all declaration cuts/lost replies/zero+short writes/sync/root faults/no RNG retry, unknown cold
partial preservation, exact original vs promoted repair, stale physical mount/session, cap
refusal before reads/effects and fees after all bytes/files. Real read via the public opaque
wrapper requires SourceSaved plus its three paid artifacts and yields DATA only.

Actual V1 A/V2 B race across signers, V1 capture BEFORE V2 registration then positive refusal,
pending owner Drop while mkdir is blocked, permanent empty/unknown dir and missing pin,
inverse existing V1 pin/head vs V2 first reservation, cold Paused/admitted-unwritten JOIN from
a DIFFERENT signer with no row/pin/head, original task/foreign identity, unknown
old canonical bytes, unrelated group control/progress and negative cleanup preservation.
The primary owns the resumed serial compiled pipeline; this agent starts no competing Rust/npm
suite. All fourteen new regressions remain UNRUN here. The required future group-writer tests
above are NOT yet authored. No production activation or complete Chat V2 acceptance follows
from design/static review.


## Pinned standard-library filename copies (design audit)

The installed toolchain did not contain local rust-src. The exact Rust1.94 primary source
was inspected read-only: [Windows fs](https://raw.githubusercontent.com/rust-lang/rust/1.94.0/library/std/src/sys/fs/windows.rs)
and [Unix fs](https://raw.githubusercontent.com/rust-lang/rust/1.94.0/library/std/src/sys/fs/unix.rs).
Windows DirEntry retains the fixed native find-data and a shared root path; file_name performs
a separate wide-string conversion. Linux-family DirEntry first owns a name copy and file_name
adds another. entry.path joins/clones the root, so the new pass must avoid that API and build
its checked path directly after validating the filename. The named scratch reserves both
component copies and path/conversion overlap; it does not claim to meter OS/C directory buffers
or allocator/RSS. Actual implementation/platform helper inspection remains required before
calling the inventory invoice complete.
