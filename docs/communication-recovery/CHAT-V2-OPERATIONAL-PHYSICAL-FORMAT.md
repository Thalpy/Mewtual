# Proposed physical operational FirstCreate contract

7 November implementation accounting correction: the shared A/I/S transaction
metadata reservation is now **1.25MiB**. The compiled next-request peak is 1,145,090
bytes, so the previous 1MiB class was insufficient. The same original reservation
is acquired before allocation and retained through cleanup; all actual layout
assertions stay enabled. This changes no wire/disk format or global pool limit.
The completed-parent accepted-query invoice is 1,082,516 bytes; its independent
reservation is now **1.125MiB**, before any copies, with its full assertion retained.
Bmetadata2MiB, work16MiB, completed-parent1MiB and separate model/domain/SDK/read/token-query
payments remain additional. Older 1MiB references below describe the earlier draft.
This accounting correction supplies no member outbox, cold recovery or activation.

DESIGN DRAFT, based on primary `4cb7f43a` and the reviewed, undeclared eight-file
Create/Append DATA packet. This adds no handler, decoder, module, permission or test.
It specifies one connected implementation cohort, not a format-only readiness claim.
The first profile is the actual sole founder, epoch0, one OWN admission, Create without
reply/parents. Other members, epochs, operations and settlement-incomplete owners refuse.
Existing genesis, SourceSaved, v1 and negative-v4 grammars remain unchanged.

## Source and ownership boundary

The actual host retains the WHOLE original settled Warm/Cold/Repaired wrapper in a closed
private owner; it does not extract a raw node or adopt a decoded Current. Cache
`Original -> PromotionPrepared -> OperationalSelected` BEFORE any new effect and dispatch
that actual phase before old genesis inventories. Cold operational reopening is a new
actual receipt/claim/scope-owner/writer family, not a GenesisHead cast. The existing small
S0 control/name proof and paid installed-writer observation kernel are reusable; the old
whole genesis-current proof is not. Source S0, SDK graph and scope2 nonce/owner remain exact.

Two exclusions are mandatory. A durable pending transaction excludes another semantic
child; the active writer lane for `(mount,group,full signer)` excludes ALL selected-O
mutations from capture through physical cleanup, even after B has selected a new parent.
There are sixteen NEW separately paid text ACTIVE slots per ServerStore, distinct from
the sixteen genesis/cold claims. Quiescent durable debt is paged and independently paid.
Only actual drained work plus complete durable continuation permits suspension/reuse;
Drop, weak expiry, selected-B visibility and network ACK do not prove that.

## Encoding and identity rules

Use `catcoms_wire`: big-endian integers; every byte field has a u32 length. `b32` costs36,
`opt32` costs37 at maximum, `bytes(n)` costs4+n. All parsers check total outer cap, every
borrowed field, tag/version and EOF before copying variable data. No successful decoder
mints a physical receipt. New control frames use separate derived keys and existing
XChaCha nonce24/tag16 framing (`plain+40`); existing private pages retain their86 overhead.

For kind K, the literal plaintext tag is `catcoms/private-chat-text-K/v1\0`, followed by
u16 version1. The key domain is `catcoms/private-chat-text-K-key/v1`; the complete-frame ID
domain is `catcoms/private-chat-text-K-id/v1\0`. K is exactly `context`, `head`, `archive`,
`retain`, `intent`, `signature`, `seed`, `wal`, or `publication`. Existing private-page
encryption/IDs remain unchanged for manifest, content and index pages. New token/index
plaintext domains are separately `catcoms/private-chat-text-token-index/v1\0` and
`catcoms/private-chat-text-publication-index/v1\0`; they are not new accepted-index scopes.

Canonical context C has its context tag/version, thirteen b32 fields in this order:
vault, archive-root, target-marker, SourceSaved-terminal, SourceId, canonical-source-context
digest, Profile, OWN Admission, full signer, scope nonce, group-pin ID, tenure, original
genesis-head complete-frame ID; then group bytes<=256, scope revision u64=2 and epoch u64=0.
Scope domain equals the actual vault in this profile. Maximum C is
`43+13*36+260+16=787`. Cid is the context-domain hash of complete canonical C. It is a label,
not source authority: Root derives/checks it from its actual source/pin/scope originals.
The source-context digest covers the existing canonical <=692-byte encoder, not Debug/JSON.

These are persisted identities, not native resource identities. `vault/archive-root` are
the exact immutable `CurrentLayoutData::{vault,root}` bytes, agreeing with the original
mandatory Archive2 descriptor; `root` is NEVER a directory File/session ID. `target-marker`,
terminal and SourceId are the actual typed `SavedSourceExpectationData::{marker_id,
terminal_id,source_id}` labels (`TargetMarkerId`, `IntentId`, `BootstrapSourceId`). The
source-context digest uses `founder_codec::encode_context` on that same expectation's
original `BootstrapSourceContextData`. Profile/Admission/tenure are their canonical public
semantic IDs; signer/group/scope nonce are the same immutable original source/public labels.
Pin is the original `V2GroupPinId` of its COMPLETE existing encrypted frame, not a reseal.
Genesis parent is the new domain-separated complete ORIGINAL frame commitment retained in
seed. No mount/read/SDK allocation identity, OS File ID/path, writer/UI generation, process
counter or cancellation identity enters C, request hashes, token keys or any persisted ID.
Those observations stay only in actual live receipts/work/custody.

A genuine new paid source read/SDK/writer reconstructs identical Cid and request/token
identity. A supported backup relocation preserves these vault/layout/source/pin/frame bytes
and therefore identities while obtaining NEW mount/File/read/SDK/writer observations. Test
two Creates then real remount AND supported relocation, finding exact original token/action/
cipher/receipt without Clock/RNG/sign/export; a foreign vault or re-sealed substituted pin
refuses rather than silently creating a new request namespace. This is no rollback guarantee.

New auxiliary BLAKE3 domains are exactly `catcoms/private-chat-text-source-context/v1\0`,
`catcoms/private-chat-text-genesis-parent-id/v1\0`, `catcoms/private-chat-text-request/v1\0`,
`catcoms/private-chat-text-transaction/v1\0`, `catcoms/private-chat-text-token-key/v1\0`,
`catcoms/private-chat-text-cipher-original/v1\0` and
`catcoms/private-chat-text-index-capture/v1\0`. Hash each literal domain then canonical
fields: source-context uses bytes(existing canonical context); genesis-parent uses the complete
raw old head WITHOUT an extra length word, as complete-frame ID hashing does; request uses
Cid/token/channel u128/bytes(text); transaction uses Cid/token/request
digest; token-key uses Cid/token; cipher-original uses bytes(nonce+cipher); index-capture
uses Cid/role u8/canonical selected root descriptor. Fixed digests use b32 framing. Cid uses
the context ID domain on complete canonical C, not a newly sealed context frame. These are
DATA commitments, not trusted-HOST/native permits. Exact request comparison also decodes
the selected original Action; a digest never skips signature or representation checks.

| Object | Ordered payload after its tag/version | Encoded plaintext maximum / allocation cap / complete-frame maximum |
| --- | --- | --- |
| H, archive | Cid; manifest ID; manifest sequence u64; signed public header bytes<=634; public image bytes<=676; optional previous H; operation u8 | `43+36+36+8+638+680+37+1=1479` /2048 /1519 |
| O, head | bytes(C); generation u64; parent-kind u8+parent ID; H ID; accepted-directory, token and publication root descriptors; queue front/next u64; pending transaction; live byte/object u128 baseline counters; attempt u64 | `40+791+8+37+36+3*118+16+363+32+8=1685` /2048 /1725 |
| A, retain | Cid, Txn, token, request digest, ActionId, original-action digest, nonce+cipher digest; action/cipher lengths u32; channel u128; action descriptors<=2; cipher descriptors<=3 | `42+7*36+8+16+81+121=520` /1024 /560 |
| I, intent | twelve b32 bindings listed below; public-parent sequence u64; exact unsigned signing bytes<=520; image<=676; acceptance record<=567; one-leaf Append body<=548 | `42+12*36+8+524+680+571+552=2809` /4096 /2849 |
| S, signature | Cid, Txn, I ID, pre-I O, public parent, unsigned commitment, ActionId; exact raw signature64 | `45+7*36+68=365` /512 /405 |
| P, publication | Cid, Txn, A ID, public HeaderId, acceptance-receipt digest, H_B ID; queue ordinal u64 | `47+6*36+8=271` /512 /311 |
| promotion seed | bytes(C); old genesis-frame ID; original genesis frame<=4136; exact H_G, O_G and initial Directory frames<=1519,1725,16470 | `40+791+36+4140+1523+1729+16474=24733` /32768 /24773 |
| WORK WAL | variant u8; Cid/install-parent O/Txn/public parent; operation u8; exact parent O; candidate O ID/frame; cleanup kind+carry or final plan+exact WD; count u16+packed bytes of immutable artifacts | B1078322 /2097152 /1078362; absolute frame cap2097192 |
| WD deletion admission | variant u8; Cid/Txn/O_B/PlanCommit; bytes(plan); exact O_B frame; exact WC frame |6261 /8192 /6301 |
| WC retirement complete | variant u8; Cid/Txn/O_B/PlanCommit/public HeaderId; reclaimed byte/object totals u128; bytes(plan) |2278 /4096 /2318 |

A descriptor list is `u8 count`, then `(b32 completeFrameId,u32 plaintextBytes)` per item.
A root descriptor is `u8 Empty|Present`, then for Present `(b32 page,u8 height,u64 keys,
b32 min,b32 max)`, at most118 bytes. Empty is legitimate only for an actual saved initial
root or its proven unchanged successor; missing/unknown/corrupt never means Empty.

O.pendingTxn is a CLOSED sum None / RetainedA / Unsigned / RawSignature. Maximum is
`u8 +8*b32 +opt32 I +opt32 S=363`: Txn, original token, request digest, public parent,
A ID, captured pre-intent O, captured pre-intent H, accepted-index capture commitment.
RetainedA uses its original pre-A capture; I replaces only the capture fields with exact
pre-I O_A/H_A. I's twelve b32 bindings are Cid, Txn, A, token, request digest, pre-I O_A,
pre-I H_A, accepted-root commitment, token-root commitment, public parent, full issuer,
and original opaque unsigned-plan commitment. The actual verifier's OWN frontier stays in
its paid math custody; this digest is not an alternative ReplayMath constructor.

O_A sets RetainedA, O_I extends that SAME transaction, O_S records its existing signature,
and only matching O_B clears it. Serialize ALL other O mutations, including another A and
publication ACK roots, through this lane. Exact eligible cipher retry/read/serve may proceed
under their separate permissions. No B rebase is implemented in this first profile.

No self cycles: H selects no index/request/publication roots. Directory.prefix_head=H_B;
manifest.source_head=previous H_A. O_B selects H_B and those roots. Accepted token/P values
name public header, record/receipt/Txn or H_B, NEVER O_B. I names PRE-I O_A/H_A, NEVER O_I.
The seed contains H_G/O_G but neither refers to seed ID. WAL contains candidate O but O
never names WAL ID. O.previous is a CAS commitment, NOT a permanent traversed history edge.
WAL retains exact parent/candidate O bytes; no O is separately materialized in text.pages.
Cold governing replay follows the permanent H/manifest/public chain, not old private O/index
snapshots. I's pre-I source commitments remain checked against its selected pending closure.

## Existing page shapes and complete output geometry

Source constants, not expanded allowances: private page overhead86; index plain16384,
frame16470; leaf32/min16, branch192/min96, height<=9. Insertion uses deterministic16+17 and
96+97 splits. A <=10-node READ path may emit<=21 new nodes including a new root. There
are no tree deletion/merge operations in this profile. Packed branch rows intentionally omit
child maxima: a splitting level may need ONE additional actual boundary-child read to learn
the new left branch's last unchanged child's maximum. This is at most9 extra reads per tree
(36 across A's token tree and B's Actions/token/publication trees), in addition to descent.
Authenticate its exact selected old row/page/context/height/count and range; never substitute
next_min-1. Retain only one such child's buffers through a rewrite, then drop before the next
level. Root's updater supplies real descriptors/packed preservation; App does not duplicate
its private parser. None of these reads occurs inside the short Current/parent-capture guard.

| Source | Profile bound |
| --- | --- |
| Original Create | <=66057 bytes, two existing60KiB action chunks; complete chunk bytes sum<=67147 including headers/sealing |
| Nonce+cipher | <=131116 bytes; three60KiB chunks with NEW inner tag `cipher-chunk`, Cid/Txn/digest, total/index/count; header168, complete sum<=131878 |
| RetainAction | existing manifest<=1024+86; exactly2 action descriptors at maximum, not cipher/index descriptors |
| One acceptance record | <=567; its real one-row batch<=1062, sealed1148 |
| One-leaf complete nodes | <=64 hashes for a successful u64-count increment; plaintext<=2514, sealed2600 |
| Header+Append batch | header-row<=1359 plus Append<=548; actual two-row batch<=2479, sealed2565 |
| Append manifest | exactly3 existing content descriptors; maximum1024+86; no index/WAL/cipher debt is covered by these3 |
| Directory | ONE genuine founder-admission root leaf, not a general21-node directory allowance |
| Actions/token/publication trees | independently up to21 new index frames each; lifetime paged state, not64/4096 message maps |

The cipher-chunk inner domain is EXACTLY `catcoms/private-chat-text-cipher-chunk/v1\0`
(42 bytes), u16 version1, then Cid b32, Txn b32, complete nonce+cipher digest b32,
totalBytes u32, index u16, count u16, bytes(payload<=61440). Thus its prefix is
`4+42+2+3*36+4+2+2+4=168`. Count must equal ceil(total/61440)<=3; indices are consecutive
and final payload size exact. No raw chunk is dispatched as SealedOp or MLS application.
The cipher is assembled only from descriptors selected by the original A, checks the
complete digest/length, and remains UNACCEPTED until the selected publication record.

New token/publication nodes use the same bounded leaf/branch geometry with their own
domains/context Cid. Their maximum headers180/186 plus192*80 or32*(32+2+384) fit16384.
Token key is a domain-separated Cid+token digest. Value stores phase, Txn/token/request
digest/A/Action/original digest and optional accepted locator `(header,recordPage,row u16,
ordinal u64,receiptDigest)`; maximum336<=384. A hit also checks exact original action
signature/representation and requested channel/text. Publication key is monotonically
assigned u64 ordinal in a fixed32-byte key; value `(Txn,Action,receiptDigest,Header,P ID,
state)` is181<=384. O selects front/next; ACK advances front and retains history. Missing
selected rows are Incomplete. Never scan all historical request rows to drain pending work.

The CLOSED WAL variant byte is WORK=1, WD=2 or WC=3. WORK's operation byte is
Retain A=1, Unsigned I=2, Raw signature S=3 or Accept B=4. Its cleanup byte is
None=0, Carry=1 or Final=2; A/I/S use Carry and B uses Final. Unknown values refuse
without an older-parser fallback. The object-kind byte is private page=1 or private
control=2, with the closed role values already used by the exact artifact set.
Index-capture roles are Directory=1, Token=2 and Publication=3. These assignments
freeze the previously symbolic u8 fields; they grant no authority to parsed DATA.

WORK WAL entries are `u8 role,u8 objectKind,b32 ID,bytes(completeFrame)` =42+frame bytes.
Roles derive closed paths; no filename/path string is accepted. Each unique immutable
frame appears once; candidate O is a dedicated field, written only to head.stage/head.bin.
The first-A consumer derives Action/Cipher/Manifest/Token pages as
`p-<64 lowercase hex ID>.bin`, Archive as `h-<64 lowercase hex ID>.bin`, and Retain
as `a-<64 lowercase hex ID>.bin`, all beneath the SAME held `text.pages` directory.
There are no alternate names; other control roles require their own declared consumer.
The unsigned-I consumer adds exactly `i-<64 lowercase hex Intent ID>.bin` under that
same held directory. The ID uses the existing Intent kind-separated full-frame domain.
This filename freezes no signing permission; selecting O_I requires its own fresh commit.
Canonical order and duplicate-ID checks precede allocation/IO. Signed/public-history objects
are retained; superseded private lookup/control snapshots use the finite plan below.

| Transaction | Immutable objects / selected head.bin selector | Max total written artifact bytes, excluding WAL slots |
| --- | --- | --- |
| Promotion | H_G, Directory plus fixed seed /1 O_G |44487; one pages-directory creation |
| A |2 action+3 cipher+retain+manifest+H+21 token =29 /1 O_A |549809 |
| I |intent=1 /1 O_I |4574 |
| S |signature=1 /1 O_S |2130 |
| B |21 Actions+1 Directory+21 token+21 publication+3 content+manifest+H+P =70 /1 O_B |1065058 |

The immutable B frames sum1063333. Its WORK WAL fixed fields cost3686:
tag39, variant1, four b32 bindings144, operation1, bytes(parent O)1729,
candidate ID+bytes(O)1765, cleanup kind1, count2, packed-length4. Final cleanup adds
PlanCommit36+plan2022+bytes(WD)6305. Thus B WORK WAL is
`3686+36+2022+6305+70*42+1063333=1078322`, below the NEW2MiB cap.
This is the largest first-profile output, not a claim
that the generic multi-admission directory has one leaf. Configured byte/object quota is
independent of source-creation grants. O carries live charged immutable byte/object BASELINES;
exact pending WAL outputs are charged without remount reset or counting an already-selected
candidate twice. Reserve the WHOLE remaining finite A/I/S/B class before A: at most101 objects/
1614671 bytes; after A the remaining class is72/1066587, after I71/1063738, after S70/1063333.
A later ordinary quota shrink must honor this admitted remainder instead of stranding I.
This is not a renewed source grant. The fixed six control slots reserve4268272 bytes separately:
`2*(32768+40)+2*(2097152+40)+2*4136`. WC completion can progress at a full dynamic quota.
Add a NEW explicit conservative ORIGINAL-genesis baseline of12368 bytes/TWO unique obsolete
inodes: original activation.seed<=8232 plus original genesis head<=4136. Warm, Present Cold
and ordinary Repaired whole wrappers/Scope negative tails retain these actual Files after
settlement/O_G promotion; Absent Cold retains only the head. A-born Repaired keeps the same
two-inode maximum while its actual host/repair/settlement owners remain, but its independent
resource-only64KiB Born Scope custody does NOT itself retain RepairSeed/Files after those
owners drain. Pin both actual lifetime chains. Arc/File clones of the SAME inode count
once. These are finite original-lineage custody, not repeated per-operation history, and
SourceSaved's nine-file grant does NOT pay the activation seed. Memory fees do not pay disk.
Thus trusted fixed text space is4280640 bytes/eight conservative live inodes before dynamic
immutable-object quota; this deliberately increases the earlier proposal by12368, not by
silently borrowing Source/SDK credit. Existing named pin/source/group controls stay in their
independent permanent original baseline. Do not refactor/extract whole original Drop owners
to pretend these Files vanished. New operational cold Scope must retain fixed original
selected-O bytes/IDs without accumulating a newly obsolete O File each reopen/successor.
These quotas count encoded/logical complete-file bytes and unique objects, NOT filesystem
allocated clusters, allocation rounding, filesystem journal/metadata or a physical disk-size
guarantee. Those costs and actual ENOSPC remain pressure; neither raises an acceptance limit
nor permits double credit or dropping signed history.

The NEW control-space slots count live inodes, not merely names: escaping text SavedA/I/S/B metadata, publication debt
and query-root leases retain frozen original O/WAL facts/bytes in their paid RAM, NEVER an
obsolete OPERATIONAL control File. Exclusive operation custody closes and joins replaced MAIN/head
handles, named-reopen temporaries and result cleanup OUTSIDE writers BEFORE admitting the
next control stage. Old private-index reader Files remain separately paid until actual drain.
A retained replaced control File must block the next-stage permit until actual cleanup; a
Weak/Arc count is no proof that kernel work or handles drained. Any future receipt retaining
obsolete control FDs must explicitly enlarge the independently reviewed live-inode reservation.
Actual lifetime tests retain original genesis Scope/observer tails after host Drop, count the
seed/head unique inodes once, and prove only their real File cleanup releases this baseline.
No page output is allowed
before a complete WAL reservation; an incomplete stage cannot hide new page allocations.
The actual host must provide a persistent text-byte/object budget; these u128 counters impose
no new64/4096 lifetime ceiling. Disk exhaustion is pressure, never acceptance or refund.

## Finite private retirement, separate from signed history

Never retire original Actions, publication nonce/cipher chunks, signed headers, facts,
H/manifest history, public records/acceptance-node pages, A or P records, current token/pub
rows, or unchanged shared index siblings. Their permanent history is not an old lookup
snapshot. Root's concrete COW constructor alone emits replaced-private-path IDs; no caller
list, parsed index or arbitrary page-kind label can construct a deletion admission.

A replaces<=10 old token path nodes. B replaces<=10 Actions+1 Directory+10 token+10
publication path nodes. Carry A's ten through every I/S WAL replacement. Add now-unreferenced
I and S controls only after selected B: at most41 private index nodes+2 controls=43 IDs.
The closed entry kinds are AcceptedIndex, TokenIndex, PublicationIndex, Intent and Signature;
Directory is AcceptedIndex. No public-history/object/control kind is a retirement kind.
The actual updater proves replaced nodes are not reused by the new root and never lists
shared siblings. Constructors prohibit reintroducing retired IDs from an older source.

Retirement carry is rootCount u8<=4, stable old selected-O IDs (b32 each, at most144),
retireCount u16<=43, packed bytes length u32, then `(u8 kind,b32 ID,u32 actualFrameBytes)`
entries<=1763: maximum1914. Final plan adds Cid/Txn/O_B_ID (108), total2022. Roots are the frozen pre-A,
A, I and S generations, NOT OS/process allocation IDs. PlanCommit is BLAKE3 over
`catcoms/private-chat-text-retirement-plan/v1\0` followed by complete canonical plan.
It is independent of WB/WD/WC IDs; O_B refers to none of them.

Prepare/pay/cache the complete acyclic chain BEFORE original WB effects: seal WC FIRST,
then WD containing complete WC, then WB containing complete WD. WC names only Cid/Txn/
O_B/PlanCommit/public HeaderId, exact reclaimed byte/object totals and the full plan, never
WB/WD IDs. WC plaintext is `39+1+180+32+2026=2278`. WD contains Cid/Txn/O_B/PlanCommit,
plan, complete selected O_B and WC. WD plaintext is `39+1+144+2026+1729+2322=6261`.
WB holds all B outputs plus WD; WD needs only the selected NEW B closure, not old WB output
copies or deleted old paths. Thus WB -> WD -> WC has no backedge. Cold WB supplies exact
WD; cold WD supplies exact WC. Inert debt never needs fresh sealing/RNG/key calls.

After actual selected B and same-verifier advance, latch BSelectedRetiring BEFORE any
retirement effect. Join ACTUAL older private-root query/work leases; no weak-expiry shortcut.
New queries use current O_B, and after admission no API may create a reader from retired
roots. Public view/token support is not a private lookup-root lease. Release a private
lookup lease when bounded locator/query work drains, BEFORE transport/ACK waiting. Cold
independent capture also requires actual old-mount OS-session/work quiescence. A live
original Store dropping while IO runs is not that proof.

The concrete original cleanup admission may promote the EXACT precomputed WD through the
same stage/MAIN barriers only after that drain. Parsed nested bytes/PlanCommit are not this
admission. Only actual durable WD allows listed NotFound/unlinks on retry. Unlink exact
closed-kind named objects under immutable plan/current-B/work proof, sync text.pages parent,
and join all delete/file cleanup outside global writers. Then actual SavedRetirement permits
exact WC stage/MAIN promotion. Keep WB/WD and original lane until WC barrier+physical cleanup,
or a separately reviewed genuinely quiescent durable suspension. WC cannot bypass actual
deletion/drain merely because its precomputed bytes exist.

Cold complete-stage precedence is phase-specific: WB stage requires exact install parent;
WD stage requires exact selected B+same plan and actual cleanup admission; WC stage requires
actual admitted WD plus completed deletion/directory/reader-work drain. Full WD MAIN makes
its exact list idempotent after partial unlink; partial WC stage cannot erase that source.
Complete WC MAIN proves this finite control transition was selected by the actual handler;
it supplies no Current/projection/native permission. Next operation may replace it only
after actual lane/work drain. Every O mutation, including ACK, uses the same WAL protocol.

Reclamation credit follows the actual named durable WC receipt, NEVER decoded nested bytes.
Validate every listed size against its held original object and closed kind cap, sum with
checked u128 arithmetic, and require byte/object totals agree exactly. Effective live usage is
the immutable selected O_B BASELINE minus this exact O_B/PlanCommit's reclaimed totals. Replay,
lost WC reply and repeated reopen recalculate from that SAME baseline; they never subtract
from an already discounted mutable counter. The next selected O absorbs the discount once;
an old WC cannot apply to another O or a new WAL/plan. Wrong size, underflow/overflow or
missing selected WC refuses credit. Deletion never refunds an irreversible execution/attempt
allowance. Permanent public history and unreachable unadmitted objects do not gain free space
by pretending to be this plan. Actual accounting/receipt tests must pin all these cases.

Without retirement, conservative retained private index output alone is
`(21+64)*16470=1399950` bytes per accepted operation; that is O(history*path), not scalable
lookup storage. The paired plan reclaims superseded paths and keeps current trees O(history),
plus one bounded recovery plan/worker per unresolved operation. Worst permanent A history
is202214 bytes and B history9253 (total211467 at maximum text); current live indexes and
normal filesystem overhead are additional. These maxima are caps, not typical costs.
The implementation MUST NOT claim this footprint if it omits the paired retirement worker.

## Fixed layout and the temporary-control availability contract

Under the SAME original local target, keep existing `head.bin` and `head.stage`; add fixed
`text.seed`, `text.seed.stage.v1`, `text.wal`, `text.wal.stage.v1` and `text.pages/`.
The pages directory is not enumerated for history: open only authenticated complete IDs.
Unreachable objects remain untouched. Its format-aware opener is distinct from strict
genesis inventory; unknown top-level names/nonregular objects remain preserved/refused.

Cold dispatch is ordered, not decrypt-error fallback. Inspect fixed seed MAIN/stage and
the selected head FIRST. Absent MAIN with only the known first-stage cut requires a full
independent settled-genesis parent proof. Complete seed selects the operational/promotion
family: the old genesis head is permitted ONLY when its complete bytes equal the exact
parent frame retained in that seed. Otherwise require a valid operational O and its actual
closure. Unknown/partial MAIN or a later/unknown head cannot become the old genesis role.
Only after this selection may a known nonauthoritative stage be classified for disposal.
An older reader already refuses these new names under its unchanged strict inventory.

A complete parent-bound A WAL is a retained transaction source even BEFORE O_A selection.
Cold capture must import/retain that pending closure before exposing a new-request path;
an empty token tree cannot erase it or authorize a second draft. Until matching completion,
it excludes another selected-O mutation just like a selected pending transaction. Its
actual resume obtains a fresh paid active turn/current permission without replacing A.

MAIN seed/WAL are NEVER first-written or truncated in place. Every attempt writes only its
explicit versioned stage using create_new; verifies the full held frame/MAC/context/parent;
syncs the file AND pre-rename parent; atomically renames in the SAME directory/volume; then
syncs/rechecks the parent and actual held named object. Only that last edge yields SavedMAIN.
No dependent child page, head.stage, selected-head CAS, positive effect or acknowledgement
follows an unconfirmed MAIN barrier. RAM candidate sealing is allowed under its own actual
paid admission. Create's action sign/export/seal precedes its A WAL and supplies no accepted
effect; Append's owner-header key call specifically waits for selected O_I and its barrier.
S WAL contains that already-produced result. An ambiguous rename/barrier retains its phase/IDs and independently
classifies named MAIN/stage; it never treats missing as new work.

Finish EMPTY O_G promotion before any Create Clock/RNG/sign call. Its complete seed fixes
the original head, H_G, O_G and Directory ciphertext candidates before pages-directory or
dependent-object creation. A complete seed repairs exact bytes and nonces. A torn first
seed STAGE has no permitted dependents and contains no admitted user action.

Only a NEW concrete `UncommittedTextScratch` role may dispose a torn stage. It derives from
the actual known selected O/workspace policy, or the distinct actual settled-genesis EMPTY
promotion role; proves original source/mount/directories, active-work quiescence, exact known
stage name, held regular object and its bounded size. It classifies PresentUncommitted,
NOT Absent; decode failure supplies no authority. Keep its held bytes/digest until classified.
Disposal is exact-name/identity unlink -> parent barrier -> strict create_new, NEVER truncate
or overwrite an unknown existing file. No general cleaner, directory recursion or head.stage
disposal follows. Complete authenticated foreign/unsupported controls, unknown extras and
nonregular/oversized objects preserve/refuse. Future handlers must durably select an old-reader
refusal version BEFORE giving old scratch names new meaning, or use new versioned names.

COMPLETE authenticated stages have a different lane. If stage context/source/parent exactly
matches the selected parent, and MAIN is absent at the genuine first-write role or is the
demonstrably already-selected predecessor, retain/promote/resync the SAME complete stage
frame under fresh concrete parent/current admission; NEVER discard it to call crypto again.
This applies to seed/A/I/S/B, including a valid stage left before file sync or rename. A
complete S stage is inspected BEFORE any new signing admission and supplies its exact raw
signature. If stage and MAIN are identical, resync that original, then dispose only the
redundant exact stage with a real parent barrier. If MAIN still contains a different live
unselected candidate, or complete stage is stale/foreign/unsupported/conflicting, preserve
both and refuse; arbitrary choice is not recovery. Unseal-success but unsupported or invalid
authenticated inner grammar is preserved too. The disposable-stage lane is only bounded
uncommitted unauthenticated/truncated scratch under the independent prospective role.
An ambiguous rename cannot turn a previously observed complete stage into a fresh attempt.

Fresh scratch-disposal/resealing requires a fresh actual parent/current work admission and
paid attempt; dead-owner DATA repair alone cannot mint it. Selected A action/cipher and
selected I/S/public child NEVER become scratch-disposable. After B, only the now-redundant
private I/S containers may retire under the separate authenticated plan; the exact public
signature/header/body and original A remain retained. A retained MAIN is an exact source,
not scratch. MAIN partial/missing together with dependents contradicts this write profile:
preserve/report inconsistent storage, not a decode-failure fallback. This explicitly assumes
cooperative writers and supported atomic-rename/file+directory-sync crash semantics; external
rollback, torn media despite successful sync and arbitrary file corruption are not repaired
by discarding controls. No portable-filesystem durability claim follows from fs::rename alone.

| Cold cut | Classification and permitted continuation |
| --- | --- |
| Original settled genesis +torn seed.stage.v1, no MAIN/dependents | actual EMPTY-promotion scratch role may unlink+sync stage and generate NEW private empty-promotion candidates; source/grants/nonce/history unchanged |
| Complete valid seed/A/I/S/B stage before sync/rename/reply | classify original parent and MAIN precedence, then resync/promote EXACT stage; S raw is never re-signed |
| Complete seed, partial pages or head.stage before O_G | exact seed source repairs its SAME frames; no Create has run |
| Selected O_G +torn first WAL stage | selected parent proves known temporary workspace; scratch retry; no new A was selected or permitted to publish |
| Selected O_A +torn I WAL stage | A/token/action/cipher stay exact; discard only classified stage, build fresh PRIVATE I candidates for SAME A under new actual admission; no key has been called |
| Selected O_I +raw signature only in lost RAM/torn S WAL stage | preserve exact unsigned child; new actual Current/native-Stop signing permission may sign ONLY same preimage; unavailable permission leaves pending, not another child |
| Complete S WAL before O_S | recover exact raw signature from WAL without key call; actual Current needed for selector CAS |
| Selected O_S +torn B WAL stage | S fixes exact signature/public child; actual paid math source and new private B candidates may be prepared; lost NEVER-admitted private B seal nonces are not recoverable or promised exact |
| Complete B WAL +partial immutable page/head.stage | repair SAME complete candidate bytes/nonces; no reseal/resign |
| head rename/parent/reply ambiguity | independently select actual parent or exact candidate, resync barrier; no acknowledgement until exact selection+dependencies are durable |
| Selected O_B | token/accepted/publication closure remains exact; later scratch cannot downgrade it; ACK waits for old active-work drain |
| Complete WD stage with MAIN WB | exact nested candidate plus actual selected-new-B/verifier/old-root-drain admission, then same-frame MAIN barrier; never discard or merely trust its phase tag |
| Complete WC stage with MAIN WD | repeat exact admitted-list deletion/NotFound+directory barrier and actual reader/work drain, then same-frame terminal barrier; stage bytes alone cannot skip cleanup |
| WD MAIN and partly deleted old private nodes | resume ONLY its exact admitted list under new-selected-B proof; no old-parent-node proof or generic missing-history fallback |
| WC MAIN | new-current proof plus physical lane drain permits next WAL; no old private snapshot/IO-plan history remains required |

Before SavedA, process death can lose a locally drafted action/cipher that reached neither
selected O nor a complete durable control. No accepted/publication permission existed; no
exact-random-byte recovery is promised. A same-token trusted retry is a fresh admitted draft,
not recovery of nonexistent durable bytes. After A/I/S selection the corresponding exact
facts persist. This is intrinsic operational temporary-workspace recovery, not a permission
to reinterpret old torn genesis activation.seed or other unknown journals.

## Payments, handles and real receipts

NEW work-buffer allowance16MiB is proposed, not an enlargement of source/SDK/512KiB genesis
credit. A separate NEW1MiB A/I/S transaction-metadata allowance funds their layouts and tails.
First B additionally uses the independently paid2MiB complete-proof profile below.
Create1MiB, Append2MiB and resident ReplayMath1MiB remain independent. Move the actual paid
parent verifier/math into Store BEFORE signing. The NEW V2 founder-prefix verifier derives
the append ONCE into its opaque owning verified candidate; the coupled Sync producer does
not also derive/clone Step. After actual HOST-selected B, the verifier checks all fallible
candidate/origin/expectation predicates before consuming, moves candidate Step.next into
the SAME verifier and returns old math. Old parent drains outside writers and the resident
fee follows that installed next cursor. Verifier/view metadata needs its own retained invoice.
Cache `SavedBPendingVerifierAdvance` with actual SavedB and SAME owning candidate before
calling that fallible advance. Its retry dispatches exact selected B rather than an old
parent proof or signing path. No next capture/projection/publication is exposed until
the same verifier advances, old math/temporary fees drain outside writers, and final actual
Current/source/selected-B proof succeeds; the logical lane remains exclusive through this.
Do not clone the model or restore it from cached image. Shared real fees fund BOTH detached
payload and residual observations until BOTH drain; no unpaid weak/control tail.

| Conservative simultaneous work-buffer inventory | Bytes |
| --- | ---: |
| Retained B candidate frames including separate exact WD/WC caches |1073677 |
| Conservative earlier A immutable frames548084 plus I2849 and S405 |551338 |
| WAL plaintext Encoder requested capacity |2097152 |
| WAL AEAD growth request, `2*(2097152+16)` |4194336 |
| New WAL frame |2097192 |
| Old WAL raw full verifier input |2097192 |
|31 old index paths raw+(plain+16 retained AEAD capacity); two open buffers; one new node/seal/frame overlap |1117424 |
| One additional authenticated split-boundary child raw/cipher/plain overlap, drained per level |49270 |
| Two record encoders65536, node encoder6578, manifest1024, control4096 |142770 |
|24 path/native-conversion buffers5120 plus native/name/error scratch65536 |188416 |
| Total before separately paid actual metadata layouts |13608767 |

This leaves3168449 bytes within16MiB; there is no RSS/allocator/OS-heap claim. Enforce
actual requested capacities and phase/drop order; grow this proposal only by reviewed
arithmetic, never silently reuse frozen allowances. An individual public cipher131116 does
not fit one private64KiB page. Each control/file parser and retry checks its cap BEFORE
allocation/IO. Acquire fees outside writers and validate the SAME account after waits.
Drop old-WAL decrypted plaintext and unframed/decryption ciphertext scratch BEFORE allocating
the new WAL Encoder or sealing it; only its charged raw frame and bounded metadata remain.
The separate read/open phase is bounded by
`4*(2097152+40)+1073677+551338+1117424+49270+142770+188416=11511663`, below the sealing-phase
total. Retained decrypted index Vec capacity is p+16,
even though its successful length is p; the invoice deliberately distinguishes them.

### Actual complete-proof host resource profile (7 November)

This resource profile intentionally REPLACES the earlier proposed eight-owned-path,
one1MiB streaming-metadata layout. It does not change the byte grammar, IDs/domains,
private disk quota/reservation, selector ordering or the catalog pool's global limit.
The actual implementation keeps complete bounded current-source proofs rather than
claiming an eight-path layout that its concrete Rust ownership does not have.

A/I/S retain their original independent1MiB metadata debit in FULL. First B acquires a
NEW separately reserved2MiB metadata debit from the SAME catalog pool BEFORE B context,
frames, paths, Arc or Box allocations; every B/Ready/Saved/observer/advance tail retains
that SAME debit through its actual cleanup. The initial base-B Windows test-target
measurement is1,232,709B:772,549 persistent plus460,160 current scratch. It includes75
persistent closed-path slots,80 current-path slots and TWO80-file proof backings,
whole-cell/Ready/Saved/observer layouts, the complete flat first-B selected closure and
the actual preallocated Root advance state. The original A/I/S1MiB, model/Core1MiB,
SDK/read/query fees and16MiB payload/work are additional actual payments; none supplies
unused overlap credit. Later warm/test-hook/root-close layouts must remain fully
charged by the actual sizeof assertions, which are mandatory on every target.

The SAME16MiB work invoice simultaneously includes retained original A/WA, inert I/S,
new B records/nodes/Header/image/private-root outputs, WAL encoding/sealing/raw comparison
and native/codec scratch. Its compiled first-B bound is8,941,016B, below16MiB. The current
source-initial producer has exactly10 artifacts, whose role ceilings sum to75,133B,
and enforces a stricter87,642B complete W_B bound for current full-journal comparisons.
The existing format's general70-artifact/1,063,333B maximum remains unchanged DATA;
it does not activate general/later-message COW through this first-source issuer.

The native profile ceiling stays128 actual Files. Complete B current/commit proofs
include every bounded artifact, journal and selector, not a metadata-only subset.
A proof backing's80 slots are paid metadata capacity, not a claim that two nested
proofs open160 Files: the nested advance/final outer proof opens only its one W_B,
while the inner proof holds the complete new selected closure. Original source/control
and named-object comparison handles remain separately enumerated within the128 ceiling.
All opened helper Files drain before the consumed actual physical lease/writer End;
Ready/Saved retain bounded frames/native facts, never obsolete O/WAL handles.
Bounded per-artifact closed PathBufs are allowed in this actual funded profile; there
is no caller path, unbounded collection, recursive scan or silent path/handle-cap growth.


Large page/WAL IO runs outside admission/pins/canonical/Root under exact immutable prepaid
Work and the SAME VaultPhysicalWorkLease. All work files/results/possibly blocking cleanup
drain before that lease. Inert debt retains no graph/key/Store/session/sender. The short raw
signature cut uses only the original opaque prepared signing bytes and actual device signer;
cache raw64 BEFORE hooks/late negatives. Header sign_using reconstruction verifies exactly
that preimage using cached raw, and replay/encoding run outside global writers. Negative
one-time handoff transfers existing raw/plan/optional Header/Step only, no crypto/observer
callback, and cannot synthesize an absent signature or model.

| Actual receipt | Required physical/authority edge |
| --- | --- |
| SelectedV2TextHead | actual known source/current row2/installed writer, held O/H/source/pin proofs and complete selected roots; cold counterpart is independently captured, never a warm token |
| SavedV2RetainedAction | complete A WAL barrier, all A objects synced/named, O_A parent CAS and target-parent barrier, original final negatives |
| SavedV2UnsignedAppendIntent | complete I WAL/objects, selected O_I and parent barrier against SAME captured O_A; native Stop/lock orders before irreversible signing admission |
| Saved raw signature | complete same-I S source plus actual selected O_S barrier; no key permission is inferred from this DATA |
| SavedV2AcceptedCreate | complete B objects/index/request/publication roots, O_B CAS from SAME O_S and parent barrier, actual fresh Current/kernel/native commit admission; full proof after temporary refunds |

Owner-dead repair may persist exact already-admitted A/I/raw/pages/WAL DATA; it cannot select
O_B, sign, publish or revive Current. Final CAS needs fresh actual Current/native permission.
If commit admission wins, that finite actual effect drains under its original commit/OS lease;
Stop cannot retrospectively report pre-admission. Publication requires SavedB eligibility and
new current native/session checks. A-only storage never authorizes publication or projection.
The NEW V2 selected-prefix verifier/root anchor and SAME private ProjectionGroupView remain
part of the connected cohort; v1 OwnerProfileExpectation/ReplayMath/structural misses cannot
issue it. Cold replay is finite paid turns on one frozen capture; process death may require
new paid genesis-prefix replay until an authenticated checkpoint loader is separately built.

Required actual tests: every staging/file/pre-parent/rename/post-parent/reply cut in the table;
unknown/foreign/later/nonregular stage preservation; unlink reply ambiguity and failed parent
barrier; no dependent IO before MAIN barrier and no header key call before selected I;
SAME A/I/S after all cuts; rawSome with
HeaderNone after cancellation; two Creates/remount/token replay; active-lane drain after B;
other-group fairness/pressure;4097+ entries and height/split boundaries; unchanged SDK rows;
no publication before B; exact Stop races; original owner death/fee tails and native OS lease.
Retain an original replaced control handle after rename and prove the actual next-stage permit
refuses until its joined cleanup, while the real OS lease still fences a new mount; do not
replace this regression with only a Rust weak-count observation.
Stop-before-I requires its OWN actual Current-selected stopped-token/private-O path from the
exact SavedA/current pending closure, with no action signing or acceptance issuer. It must not
infer Stop from a token miss or erase a complete parent-bound A WAL. SavedI remains irreversible
pending; only matching B or separately reviewed resolution can clear it. This path and live
quota credit are actual cohort requirements, not positive effects provided by these DATA formats.
No tests or builds were run for this proposed document. Format/source review is still required.

The first-B immutable namespace uses `u-<64 lowercase hex Publication ID>.bin` for
Publication controls. New accepted/token/publication index, record, node, Header/body
and manifest pages use `p-`; H_B uses `h-`. These are exact fixed-role spellings,
with no alias or caller filename. A parsed filename/publication record grants no
selection, Root advance, retirement, publication or next-request authority.
