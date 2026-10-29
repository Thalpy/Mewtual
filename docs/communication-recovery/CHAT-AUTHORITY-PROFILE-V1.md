# Owner-certified chat evidence: first executable checkpoint

Proposed signed-format, reserve and producer supplement to the
[reviewed admission contract](CHAT-ADMISSION-ADMIN-AUDIT.md). Independent review is required before
these wire/persistence changes. No production activation, owner succession, legacy conversion or
review UI is implied by this first checkpoint. Its signed resource profile is explicitly
`owner-evidence-CHECKPOINT-1`, not an activation-ready long-lived 20/50-member chat profile.

Source checkpoint: strict record codecs/IDs and actual local MLS Add/Remove observations have a
clear read-only actual-diff review. On the primary's isolated combined `d1fd039b`, the affected
seven-crate all-feature test metadata check passes without warnings; MLS `group::*` passes 39/39
in 2.44s, including the five new observation regressions; owner-fact tests pass 7/7 in 27.01s.
The new private actual-founder preparation/origin slice has a clear static review and three authored
tests that are still **uncompiled/unrun**. The primary exclusively schedules Rust execution.
Scoped formatting/diff checks pass. Retained archive, mounted trusted-pin adapter, covering
transaction and real second-member/fresh-restore vertical below remain absent; primitive results do
not establish them or production readiness.

The checkpoint must exercise actual founder creation, a fresh-KeyPackage join, a signed typed action,
an owner-certified acceptance and an actual removal cut. A second real member and a fresh restore
verify the original public records under the same explicit owner trust profile. The next checkpoint
must use this same retained case for admission-aware effects and durable review/timeline integration.

## Signed grammar and scope

Use `catcoms-wire` fixed-width big-endian integers and length-prefixed bytes. There are no maps,
optional unknown fields, compression, floats, presentation clocks or caller-supplied verified flags.
Every decoder checks the complete encoded bound before parsing; vector count and remaining bytes
are checked before allocation; fixed digests/keys have exact lengths; lists are strictly sorted and
unique; unknown versions/kinds and trailing bytes refuse. Bad input never partially replaces state.
List counts are `u16`; fixed digest entries use ordinary length-prefixed 32-byte fields except the
explicit packed Removal array. Closed variant discriminants are `u8`, with only the documented values.

Each canonical semantic record begins with the length-prefixed domain
`mewtual/chat/owner-fact/v1\0`, kind `u8`, group bytes (1..256), existing typed action profile `u16`, profile CID and full
32-byte issuer key, followed by the closed kind-specific fields below. The initial Profile omits
its self-referential profile CID. A signed envelope contains the semantic bytes and exact 64-byte
Ed25519 signature. Its separate domain is `mewtual/chat/owner-fact-envelope/v1\0`.

`FactId = BLAKE3("mewtual/chat/owner-fact-id/v1\0" || semantic bytes)`. Signature representation is
excluded. Verify every representation before deduplication; retaining a canonical valid signature
representative cannot change authority. Existing ActionCID, signing bytes and snapshot grammar are
unchanged. Group admission and channel permission are distinct: an Admission is group-global;
RealmGrant, Basis and Acceptance add the exact channel `u128` to their payload. Existing
`GrantId`/`BasisId` receive domain-separated hashes of canonical RealmGrant/Basis semantic bytes,
never a vault scope or arbitrary roster label. All channel records must match the original action's
deterministic Realm. A valid group admission alone grants no all-channel action permission.
The exact ID prefixes are `mewtual/chat/owner-grant-id/v1\0` and
`mewtual/chat/owner-basis-id/v1\0`. Transition commit digests hash the exact serialized MLS Commit
bytes with BLAKE3; credential digests use prefix `mewtual/chat/mls-credential/v1\0` plus its exact
serialized credential bytes. These fields do not expose private provider or init-key material.
The founder creation reference is
`BLAKE3("mewtual/chat/founder-creation/v1\0" || group-length:u32 || group || full-founder-key)`.
That public shared label proves no creation by itself. The private preparation caller requires the
actual Found source in its original authenticated state, then signs with the actual MlsDevice.

| Kind | Exact payload after the common header |
| --- | --- |
| Profile = 1 | Fixed trust-rule value `owner-non-equivocating-founder/v1`, fresh creation group/transition reference, full founder finalizer key, supported-succession value `none`, and the exact signed resource-profile identifier. Group/issuer must agree. |
| Admission = 2 | Subject full key, actual Add commit digest, before/after epoch, exact MLS KeyPackage reference, credential digest and Profile predecessor reference. Its FactId is the shared group admission identity. A founder admission uses a distinct closed fresh-creation variant, never an all-zero fake KeyPackage/Add. |
| RealmGrant = 3 | Channel, exact group Admission FactId/full subject key and closed Rights bits. GrantId hashes this statement. Later channels can receive separate grants for the same supported active admission without consuming/replaying an Add observation. |
| Basis = 4 | Channel, exact Admission FactId, resulting GrantId, subject full key and strictly sorted covered BasisIds (at most 65). BasisId hashes this signed semantic statement. Referenced coverage needs independently verified matching records, not just an asserted list. |
| Acceptance = 5 | Channel, exact ActionCID, Admission FactId, subject full key, GrantId, BasisId and exact target Create or closed absent variant. This certifies logical acceptance of this one action, not physical creation time. |
| Removal = 6 | Exact group Admission FactId/full subject key, actual Remove commit digest and before/after epoch, plus a complete accepted ActionCID set across its channel grants. Encode `count:u16` and ONE packed length-prefixed byte field of exactly `32*count` bytes, strictly sorted/unique; never one length prefix per CID. |
| JoinBootstrap = 7 | Fresh attempt KeyPackage reference, full joiner key, authenticated Welcome digest and expected founder key, in addition to the common Profile CID. It binds profile transfer to the real completed attempt, never a raw restored group. |

Profile, Admission, RealmGrant, Basis, Acceptance and Bootstrap envelopes are at most 4 KiB each.
Removal is at most 140 KiB with at most 4,096 exact accepted ActionCIDs across channels. Its packed
array costs `4 + 32*4096 = 131,076` bytes, plus its count, header, exact fields and envelope; tests
must prove the full maximum-group-ID envelope fits, including all length prefixes and signature.
With the stated grammar, the common header is 366 bytes, Removal payload 131,202 bytes and complete
envelope **131,680 bytes**, below 143,360 bytes (140 KiB). Pin these exact maximum calculations in
tests; changing a domain/field/encoding requires changing the charge calculation together.
This deliberately starts with an explicit
bounded cut set, avoiding an unreviewed Merkle/consistency proof format. Exceeding the supported set
refuses new acceptance before issuance; it never truncates a cut or evicts accepted history. A future
larger/commitment-based profile needs an explicit new signed resource profile and compatibility review.

The owner attests to actual transition facts. A receiver can verify these signatures, commitments,
scope and consistency with its own authenticated facts, but cannot independently replay private MLS
state from them. The signed profile explicitly trusts the pinned finalizer not to equivocate. Neither
an MLS epoch label nor a signed certificate proves physical time or Byzantine finality.

## Root acquisition and real producer

No downloaded self-signed Profile, current roster lookup or restored status can install its own root.
Local founding requires an opaque observation produced only by the successful real
`ServerGroup::create` operation. The observer binds the exact group/full founder key and fresh
creation; `load`, `join`, epoch zero inference and `ChannelSync::new` around arbitrary state cannot
fabricate it. The app's vault-local authoring scope remains a separate lifetime guard.

A joining member needs its opaque successful fresh join completion, the expected profile/founder
pin captured before the attempt from authenticated profile-bearing invite context, and the matching
signed JoinBootstrap. Check all group/key/KP/Welcome bindings. The prerequisite `d11fffc3` validates
the *actually selected* Welcome KeyPackage; mere presence of a fresh entry is insufficient. The
scope implementation owns that nonserialized completion carrier in `RoutingState` and the pre-await
mount/lineage capture. This producer must not reinterpret either as a portable historical grant.

The first staging API is explicitly a **trusted-pin host API**: its expected
`(group, ProfileCID, full founder key)` comes from an actual founder-created observation through an
explicit trusted application/test handoff, captured before await. It cannot come from the returned
Profile/Bootstrap or current roster. Existing `InviteToken::verify_self` proves its inviter's key
and signed invite content; a non-founder inviter does not authenticate this new founder/profile
tuple. Automatic profile transfer needs a separately gated profile-bearing invite/response
adapter; old invite, routing and Welcome signature domains stay unchanged. Missing pin/bootstrap
support leaves the new profile inactive. No generic current-owner fallback is allowed.

Successful actual Add and Remove operations produce narrow nonserialized observations inside the
MLS wrapper: group, exact full subject, KeyPackage reference/credential digest for Add, actual commit
digest and before/after epoch. Private fields have no Deserialize/restore constructor. Failed work
cannot leave an old observation available as a new transition. Observation is consumed only against
the exact current group/transition and an already pinned Profile. Staged/fork-resolved or companion
paths lacking the required observation stay unsupported rather than inventing evidence.

The sync producer rechecks local full signer/current owner and supported tenure/profile, reserves the
complete archive/control obligation, performs the actual transition and signs its matching public
fact. It cannot sign an arbitrary Admission from renderer-provided group/key/KP fields. A same-key
A1 removal/A2 rejoin uses a new actual Add/KeyPackage/Admission/Grant commitment. The original A1
action never acquires A2 authority. Unsupported succession blocks new issuance, retaining old proofs.

The verifier constructs ordinary historical `VerifiedAuthorityBasis` only from the exact verified
Admission/RealmGrant/Basis chain. Admission/projection results additionally bind the entire original ActionCID,
realm, full signer, admission, grant/basis and target. They cannot release another action, accept a
different body sharing a nonce, or turn ordinary reusable authority into late-action approval.
Approval for a later cross-author Delete remains a distinct exact moderation requirement.

## Archive and reservations

Add one bounded public proof archive alongside the typed action evidence, with no serialized authority
bits. It retains original signed fact envelopes and exact action bindings. The archive is plaintext
internally and must be inside the existing encrypted server save. It is not an MLS provider export.
Checkpoint global-per-server grammar limits are 64 retained group-admission slots, 32,768 evidence/control rows and
64 MiB encoded archive bytes, counting all channels/profiles together. Channels cannot multiply the
quota. Resource values are signed profile constants, not peer-selected unbounded decoder arguments.

The 64 slots count **retained lifetime group incarnations**, including removed/rejoined members;
they are not 64 current members and are not multiplied per channel. Track active and retained counts
separately. Reaching this checkpoint lifetime limit refuses new typed admission, preserving prior
history; it is a known staging availability cliff, not the production membership policy.

Reserve bytes **and rows/work slots**, not bytes alone:

| Obligation | Maximum retained artifacts and bytes | Reserved rows | Reserved control work |
| --- | --- | --- | --- |
| One group admission | Admission 4 KiB; initial RealmGrant 4 KiB; initial Basis 4 KiB; future Removal 140 KiB; fixed transition/retry-terminal slot 4 KiB; publication-index slot 4 KiB; first genuine contradiction escrow 140 KiB: **300 KiB** total | **7**, one per listed artifact | **3**, retirement, covering reconciliation/publication and contradiction processing |
| One outstanding acceptance operation | One exact Acceptance at most 4 KiB, plus one fixed retry/negative-terminal slot at most 4 KiB: **8 KiB** total | **2** | **2**, covering reconciliation and publication/negative termination |
| Future review operation (not enabled by this checkpoint grammar) | Original signed review proposal at most 8 KiB separately charged; reserve two finality certificates at most 4 KiB each, one conflicting raw proposal at most 8 KiB, and one fixed terminal slot at most 4 KiB: **20 KiB** additional reservation | **4 additional**, plus the separately charged original proposal | **3**, covering reconciliation, publication/negative termination and first finality contradiction |
| Profile, additional RealmGrant/Basis/Bootstrap or signed review proposal | Its own measured bounded envelope/index bytes, separately charged before signing; original action frames also retain their existing ledger charges | **1 per record**, plus the applicable reserved operation rows above | Ordinary bounded work unless explicitly using a previously reserved control obligation |

The 8 KiB allowance is exactly two artifacts, not an unlimited collection of future decisions.
A later finality/negative artifact not representable in the prepaid slots must acquire a new
reservation before any signing/promise. Updating a terminal slot never grows it past 4 KiB, and
already signed decisions/certificates are not overwritten to free space. The initial admission's
terminal/index rows hold exact references, not a Welcome, Commit or message body.

Ordinary facts/recommendations cannot borrow this partition. Check consumed plus outstanding row,
byte and work reservations under the same serializer before accepting work. There are at most 32
outstanding owner operations and **288 reserved control-work slots** (64*3 + 32*3); ordinary queues
cannot consume them. Active obligations share these slots, not an additional unbounded repair map.
Remove/finality must still fit when ordinary rows are full but reserved bytes remain. The global
64 MiB cap includes consumed and still-reserved bytes. A reserved byte is not simultaneously free
ordinary credit. Per-entry/index metadata also has fixed row limits; these are logical charges, not
claims about allocator RSS or Tauri serialization. Materializing a required record moves its bytes
from outstanding reservation to consumed archive charge, never double charges it or makes it free.
After an operation's certificate, fixed retry/negative-terminal slot and publication obligations are
durably represented/retired, release only unused reservation. The retained signed record, protective
receipt slot and required proof remain charged. After a removal is durable, unused future-retirement
reserve for that admission can be released; its consumed cut/proofs and historical row remain.
Its unused contradiction escrow remains reserved: later genuine finalizer equivocation must still
be detectable at full ordinary capacity. This permanent checkpoint cost is part of its disclosed
scale limit and must shrink through the production fixed-size cut/compact proof design below.
An ambiguous save cannot release either side's obligation until exact repair resolves custody.
Reconstruct all three accounts from retained records and outstanding exact obligations on restore;
reject inconsistent accounting rather than accepting serialized credit/authority counters.

Original MLS/Welcome/Commit and sealed publication bytes are separately retained by the covering
server transaction, not hidden in a 4 KiB index charge or called public evidence. The checkpoint
reserves a separate **1 MiB publication-byte allowance per outstanding owner operation**, measures
the complete exact publication bundle against it before commit, and refuses oversize candidates.
Keep consumed bytes charged until that exact obligation safely retires; a lost Welcome reply keeps
its recoverable bytes. This is additional to the 64 MiB public archive and existing action/provider
storage, not a claim that the whole server snapshot fits 64 MiB. The actual encoded candidate must
pass its independent store/frame/storage checks. No old direct-admit-cache eviction may discard a
new profile's still-required lost-reply bytes.

The contradiction escrow retains one additional complete owner-signed conflicting fact, including
a maximum-sized second Removal, alongside the original retained fact. A 4 KiB digest-only slot is
not a substitute for the signed 140 KiB semantic body. Validate both signatures/scope/contradiction
under the bounded reserved control turn, retain the pair atomically, and derive breached-profile
state from their original bytes. Neither order may preserve a first cut as the authority winner.
Once that affected admission's first-pair escrow is consumed, refuse further unsupported conflict
evidence with explicit incomplete inventory while keeping the scope fail closed; do not evict either
proof or re-enable positive effects. Any future review/finality format must likewise prepay its full
raw-decision/certificate pair as tabulated before dispatch. No review record/selection kind exists
in the first checkpoint codec, so its button remains disabled.

Owner-side retained response bytes support exact live same-attempt retry; they do not recover a
crashed joiner that lost its original private init key/KeyPackage provider state. A reminted package
against a spent one-use invite is a different attempt. Joiner-attempt private durable recovery is
outside this checkpoint; do not confuse successful member snapshot restore with that guarantee.

Admission/acceptance refuses before changing membership or signing a positive certificate if the
future control obligation does not fit. Exact duplicate proof delivery consumes no second slot.
Required known cut/retirement/finality work consumes its reserved slot even at ordinary saturation;
no eviction of accepted history or prior cuts. Drain control verification separately with bounded
work. Unsupported authenticated transitions retain their bounded reference in the control reserve
and freeze affected positive projection rather than continuing old provisional effects.

There is no unknown-action acceptance/approval cache. Import original action evidence first, or return
a typed missing-evidence result and request that bounded dependency. Invalid signatures/scope and
foreign witnesses refuse without changing a good record or reserving an attacker-selected ID.
This positive missing-action rule cannot short-circuit an already verifiable contradiction between
pinned-owner statements about a known admission/frozen cut. Verify and retain that bounded negative
evidence in its prepaid contradiction escrow even if the referenced action body is missing; it
confers no positive action authority and creates no free unknown-action cache.
Conflicting genuinely signed finalizer facts stay bounded retained conflict evidence. No timestamp,
CID or first arrival picks their authority. Under a cut, exact membership in its signed accepted set
is necessary; a later Acceptance outside that frozen set cannot manufacture historical acceptance.
An already verified Acceptance contradicted by its alleged cut is a breached-profile conflict, not
local permission to choose whichever arrived first.

Archive snapshots use separate strict domain `mewtual/chat/owner-archive/v1\0`, a trusted pin reference,
closed resource profile, sorted signed records and exact original action/evidence references. Check
the 64 MiB frame/count limits before allocation and validate the complete snapshot before installing
it. Restore obtains the trusted pin from the app-owned mounted record/provenance, verifies original
signatures/relationships and reconstructs reservations and admission; snapshot fields cannot mint
a pin or verified witness. A sticky server snapshot tail rejects old readers before publication.

### Required scalable successor before activation

The checkpoint's 64 retained incarnations and 4,096 accepted actions per admission are lifetime
cliffs. Even with unused reservation released, a fixed 64 MiB archive eventually fills; retaining
8 KiB forever per action would fill after only roughly 6-7 thousand actions with a 50-member group.
Neither those bounds nor a test that fits them completes the long-lived chat goal. Do not activate
this signed checkpoint profile as production policy or silently raise its constants in place.

The production successor must review a fixed-size cut over a paged, append-only accepted-action
Merkle log per admission: signed checkpoint `(admission, tree size, root)` plus exact inclusion and
append-only consistency proofs, with distinct leaf/node hash domains. Leaves bind the exact action
and admission/grant/basis. Proposed leaf-index/tree-size bounds support at least 2^32 retained leaves;
inclusion paths stay at most 32 sibling digests, with an independently checked consistency-proof
bound (proposed 33). Cut size is independent of the number of old posts.
Log position proves inclusion/order under the finalizer trust rule; it never chooses a branch when
two signed checkpoints conflict. Missing inclusion/consistency evidence stays pending.

Batch one signed checkpoint across many actions and charge each retained public proof page once,
with counted references from cases/actions. Charge actual consumed pages/receipts and only outstanding
future obligations, rather than a permanent 8 KiB slab for every old post. Membership retirement still
has its separately reserved fixed control footprint. Keep a configured persistent archive quota
distinct from fixed network frame, hot-cache, per-turn verification and page limits; use bounded
incremental disk/page work rather than loading lifetime history into a network turn. A full configured
retention store refuses new promises without discarding required proofs or preventing a reserved cut.

This successor needs concrete tree/proof codecs, append/consistency rules, durable accounting and
compatibility review before implementation/activation. Preserve original checkpoint signed records
and their exact provenance during migration; a new root cannot retroactively claim a previously
unaccepted action was accepted. Required tests include more than 4,096 posts from one admission,
many retire/rejoin cycles and long-running 20/50-member bubbles with bounded hot memory/turn work.

## Covering transaction and publication

Existing `serve_join`/`admit_now`/`admit_device_now` can queue a Commit/Welcome before an app save.
The new profile must not inherit that order. A profile-aware Actor transaction holds the candidate
MLS transition, control facts, accepted-set change, exact retry binding/result, archive reservations
and publication obligation under one Ready/lease covering save. It cannot return a Welcome,
certificate or final acceptance to any transport/native caller while merely prepared.

Preparation isolates transport publication, inbound graph mutation and ordinary snapshots. On the
same Actor's final native lease, recheck mounted store, persistence serializer, exact server instance,
session and original owner/admission context. Save candidate MLS plus proof archive and publication
obligation atomically. Only `Durable` releases its exact bytes; local success is not remote receipt.
Publication and reply loss retry saved bytes, not a new Add, grant or acceptance signature.

Pre-replacement failure restores the original live state. Successful rename followed by failed parent
sync latches the whole candidate and blocks rollback-only snapshots/publication; host repair rewrites
that same candidate under the original serializer/store/incarnation even locked or without a renderer.
Unwind/cancellation cannot release an unsaved Commit or detach a pre-registration publisher. Restore
checks uncertainty, retired exact fences, pin and archive before Actor spawn/publication. The real
store's typed NotReplaced/ReplacedNotDurable outcomes remain authoritative. This must be an actual
store transaction, not a test boolean or caller's after-the-fact declaration that a save succeeded.

## Focused implementation and tests

Expected files: narrow MLS observation module; `replication::chat` signed record/archive modules;
sync owner-fact producer and snapshot tail; app Ready/lease adapter and actor hooks; beside-module
tests using existing Clock/RNG and in-memory mesh/store fault seams. Avoid RoutingState scope hunks
owned by the fresh-KP work. No active native command/UI or released wire adapter is added in the
first checkpoint; those remain mandatory follow-on integration gates, not a completed user feature.

Required first checkpoint tests use a real founder, real freshly minted KeyPackage/Welcome join,
actual signed action and actual removal. Verify exact original evidence on a second member and
fresh restore. Also cover foreign roots/group/channel/full key/KP/Welcome substitution; A1/A2 same-key
rejoin in both orders; invalid-first and real signature variants; malformed counts/tails/unknown kinds;
action/cut opposite order; acceptance outside a frozen cut; independent grant/basis dependencies;
saturated ordinary budget with successful required removal; full ordinary quota then two genuinely
signed contradictory cuts in both orders and restore; actual pre-rename/post-rename sync
faults, unwind and lost replies; no publication before durable save; restart with missing proof;
and unapproved foreign Delete versus real prior-effect tombstone custody. Later effect/review tests
must reuse the same real records/case rather than replace this path with fixture-only witnesses.
Pin row-full/byte-space-left removal, all 4,096 real cut IDs with a maximum-length group, and
consumed/retained/unused-reservation reconstruction after restore; short successful cuts do not
exercise the declared mandatory reserve.

Concrete review questions before runtime are root-pin transfer custody, exact record/domain grammar,
reserved-byte arithmetic, actual Add/Remove observation coverage and candidate publication isolation.
Production remains off until those and the later native/UI/compatibility/authority gates pass.
