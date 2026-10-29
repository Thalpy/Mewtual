# Owner evidence storage and the first retained admin case

Concrete next persistence/candidate supplement, pending independent review before implementation.
This is the minimum safety boundary needed to deliver the same actual held case through
[the privileged admin reads](CHAT-ADMIN-READ-CONTRACT.md). No production activation or completed
owner succession is claimed.

## Immutable mounted root pin

Use a separate vault-sealed `chat-owner-pins.bin`, keyed by actual MLS group bytes, not numeric
server ID. The plaintext grammar is length-prefixed domain `catcoms/chat-owner-pins/v1`, `u32`
row count, then strictly group-sorted unique rows: group bytes (1..256), full founder key (32),
ProfileCID (32), source kind `u8` (0 actual private fresh founder, 1 explicit trusted-host expected pin),
source-reference digest (32). Digest/key fields use the normal exact-length byte encoding.
Unknown kinds/trailing bytes/duplicates refuse the complete registry before installation.

The global non-evicting limits are 65,536 rows and 32 MiB encoded plaintext; reserve 384 bytes per
row plus fixed header before accepting it. Maximum row is 369 bytes, so the reservation covers all
group lengths. Missing/malformed/unknown registry on an owner-managed vault fails closed.
Physical sealed-file size is bounded before reading/decrypting using the exact existing framing
overhead; plaintext/count/field bounds are checked before structured allocation. Roots
survive `remove_server`/Leave and cannot be changed by numeric aliases. A matching group/key/Profile
tuple is an exact duplicate; a different root for the same group refuses without mutation. Original
source provenance remains retained. This is trusted local configuration, not a public proof of MLS
history or a stored membership/authority boolean.

Install only from the actual private unpublished founder source under its original physical/native/store
capture, or an explicitly trusted-host verification pin captured before join. Verification configuration
alone grants no membership, writer or publication capability. A joined Actor additionally requires its
actual fresh selected-KP/Welcome completion and matching founder Bootstrap before writable startup.
A returned Profile, today's roster, raw snapshot or renderer scalar
cannot manufacture an install source. The generic local authoring scope remains separate: its
provenance can survive unrelated epochs, while the founder preparation requires original actual
creation state. The preparation's MLS equality method is not original physical custody.

Managed scope v3 is insufficient: a scope-only v3 reader ignores this registry. Add critical vault
version **4** with its own KDF domain `mewtual-vault-chat-owner-evidence/v4`; changing the byte to
1/2/3 must not decrypt under an old reader. Preserve EVERY existing scope3 Current/Retired/Initializing
row and original nonce/revision during upgrade. Only a provably new missing preactivation owner-pin
inventory may initialize empty; never replace an existing scope inventory. Sync inventories, then activate
v4 while the exclusive vault session and passphrase are available. Do this only in the host's
unlock/setup path for the gated profile, not by retaining the passphrase or prompting mid-operation.
Scope management recognizes 3/4 and never downgrades 4; password changes preserve 4. On any activation
failure, no usable owner-managed Store/context returns. Missing mandatory registry is not recreated
as empty after activation. Production defaults remain unchanged until the full profile is reviewed.

Root pin first is an inert immutable configuration dependency. Persist/sync it before the single
covering canonical owner-state transaction; a crash may leave a dangling pin but no accepted/exportable action.
The root registry must retain the actual NotReplaced/ReplacedNotDurable outcome, conservatively
latch all possibly installed rows, and require an actual synced same-root rewrite on retry. No
epoch/lock/current-role change clears uncertainty. Use the original mounted store and its serializer;
host repair may run without a renderer. No installed-file-is-durable inference.

Known pinned groups require the exact supported owner tail on every positive restore/save/admission.
Checking a pin only when a tail happens to be present permits stripping to an old legacy snapshot.
Native restore looks up the actual decoded group before any Actor publication; missing/wrong pin,
missing/wrong owner tail, malformed archive or unsupported policy refuses positive legacy fallback.
An owner tail restored without an external trusted mounted pin is explicitly UNBOUND: no positive
owner operation, action admission or publication is authorized, including non-native/library paths.
Mounted known-pin consumers enter required mode BEFORE start even when their supplied snapshot
lacks a tail; raw snapshot fields cannot install a runtime verified root. Preserve ordinary unpinned groups'
existing behavior; do not silently convert their legacy messages into proved history.
This is API-level immutable retention, not cryptographic anti-rollback of an entire vault or all its
authenticated files. No external monotonic witness exists for an attacker/user replacing all state
with an old consistent backup. Known missing proof/custody references still fail closed; do not
promise to recover a later deletion whose entire authoritative history was rolled back away.

First founder initialization uses a wholly private unpublished Server wrapper, before ANY legacy
registration, start/subscription, self advertisement, accepted work or exposed group result. A genuine
Found token alone is insufficient after a live legacy server has already exported data without
advancing its MLS epoch. If a crash occurs after the inert pin but before the first canonical save,
that never-exposed new group fails closed as a creation failure; no old accepted history existed.
Do not revive it from a pinned legacy snapshot or infer a new Found token. Actual fresh joiners
persist an explicitly trusted verification pin before their join attempt and expose no local Actor
until their canonical joined snapshot is saved. Pre-snapshot joiner crash/private-KP recovery remains
the explicitly excluded limitation; successful member restore starts from its saved canonical state.

## One writable actor and canonical local head

The existing `ServerStore::_session: VaultSessionGuard` retains the operating-system
`.vault.session.lock` for the entire physical mount, acquired before KDF/unseal. It already excludes
independent Stores/processes on the same physical directory; retain it. The short `.vault.lock` is
different. Copied vaults/keys and whole-state malicious rollback remain outside this local guarantee.

Within that mount, reserve ONE writable registration for `(real group, full local signer)`; for
fixed-founder issuance that signer must also be the pinned founder in supported actual tenure.
Hold registration through unregistered startup/cancellation and actual task lifetime, revoke the
original writer before replacement, and do not let cached/pre-registration writers survive it.
Duplicates are refused/read-only; no second writable alias starts or publishes.

Retain one canonical encrypted owner snapshot independently of numeric IDs at
`owner-state/<key>.bin`, where key is lowercase BLAKE3 of domain
`catcoms/owner-head-key/v1\0` plus normal length-prefixed group/full signer. This namespace is private
and MUST NOT be served through the network BlobStore. It survives numeric `remove_server`/Leave.
Numeric snapshots are non-authoritative mirrors/locators for a pinned group. Positive restore must
load the actual canonical head or refuse; an arbitrary old alias snapshot cannot become the head.
Known divergent retained signed owner transitions are conflict evidence and freeze positive owner
work; do not choose a branch by first open, timestamp, generation or CID. This signed-owner
equivocation boundary is distinct from ordinary concurrent message actions, which merge
automatically and non-destructively without user repair.
The founder-admission conflict escrow is also prepaid before any later owner transition, so an
unknown second Admission/Remove at an already witnessed epoch cannot evade contradiction retention
through the ordinary new-admission capacity path. It confers no positive unknown-action authority.

The canonical plaintext wrapper is length-prefixed domain `catcoms/owner-canonical-snapshot/v1`,
group, full local signer, root ProfileCID, original scope domain/nonce (all exact 32-byte fields),
scope revision `u64`, local head generation `u64`, closed optional parent-head digest, then the
bounded full private Sync snapshot. Its digest uses `catcoms/owner-head-id/v1\0` plus the canonical
wrapper. This is local writer progress, NEVER public governance finality. The CHECKPOINT-1 private
snapshot bound is 144 MiB plus 1 KiB wrapper overhead; separately bound its 64 MiB proof archive,
64 MiB action frames, 8 MiB provider state and 8 MiB other snapshot data. No full-state hash/provider
bytes enter public admin DTOs. With the existing 24-byte nonce and 16-byte AEAD tag, the exact
maximum sealed canonical file is **150,996,008 bytes**. Before admitting a canonical head, charge
**301,992,016 persistent bytes** against the host's explicit configured canonical-state quota:
one maximum current file plus one maximum temporary replacement. This is a CHECKPOINT-1 availability
cost, not a production storage target. There is no automatic quota/default activation. Pin inventory
and other retained files have separate charges; ordinary file growth cannot spend this replacement
reservation. A head cannot be admitted when the configured quota lacks its full reservation.

After replacement, the new current file consumes the current allowance and the old temporary-file
allowance becomes available only for that head's next replacement. Required proof records and
contradiction escrow remain retained inside the current file; removing a numeric alias does not
release the head's reservation. Pre-rename failure cleans only its exact temporary file. An uncertain
rename retains its whole exact candidate and head debt; no second candidate/replacement may be
allocated or written until repair resolves it. Reserving logical bytes cannot guarantee OS free space;
actual ENOSPC remains an honest failure and cannot release publication. Physical read bounds apply
before allocation/decryption, and concurrent reader/candidate buffers need separately acquired bounded
permits. These persistent charges are not a claim about allocator/RSS: implementation review must
inventory every simultaneous plaintext, sealed and candidate buffer rather than multiplying a single
file bound silently. Tests exercise replacement at the configured quota, full ordinary record rows,
and uncertainty without accepting a second replacement.

Use ONE fixed exact staging slot per canonical head (`owner-state/<key>.replace.tmp`), created only
under the exclusive mount and original head writer. Do not use the generic PID/counter temporary
namespace for this bounded obligation. Before any new replacement after restart, inventory that
exact slot under custody; an orphan pre-rename stage never reached the export barrier and may be
removed by exact non-recursive cleanup, followed by parent sync. Never delete/replace the current
canonical file or an in-memory uncertain candidate as orphan cleanup. A missing canonical head plus
an orphan stage does not infer successful creation or fresh provenance. Cleanup/sync failure refuses
a new writer; repeated crash/reopen cannot accumulate extra staging files or exceed its one prepaid
replacement allowance. The actual implementation must preserve the normal create/write/file-sync/
rename/parent-sync ordering and typed replacement outcome at this fixed slot.

The Store alone constructs opaque canonical-load/registration/head-capture objects. Each preparation
retains its ORIGINAL physical writer identity, mount/admission observer, canonical head digest and
generation through await. Under admission writer -> logical-head writer -> original Store I/O,
recheck all original identities and CAS against current authenticated head before signing/commit.
The initialization callback already holds the original admission writer: accept its opaque borrowed
`ChatAdmissionStoragePermit` and NEVER reacquire that writer. An Initializing permit allows only
private canonical save. If save succeeds but the external inventory remains Initializing across a
crash, the Active-looking snapshot tail is still private/unbound. Only the exact returned Current
Binding, with the SAME runtime owner identity after the durable inventory barrier, can activate the
canonical RootWriter or publication; a reservation observer alone cannot promote it.
Generation increases by checked one only on a valid local transition; it never selects foreign
owner authority. Ordinary snapshots carrying the evidence archive must use this SAME canonical
write/CAS path. Generic `save_server` cannot strip a pin/tail or overwrite a newer canonical head.

Root-pin insertions share ONE global registry serializer across all numeric IDs/groups and merge
all existing rows under current mounted Store custody. Logical head writers additionally serialize
the same group/signer. If any canonical replacement is uncertain, its debt fences ALL aliases and
uncommitted queues until exact original-candidate repair; an epoch/lock/re-registration cannot clear
it. A raw stale sealed alias is not a new capture from the current logical key.

Fresh process restore does NOT turn visible authenticated canonical bytes into proof that a prior
parent sync completed. Before activating any RootWriter, positive admission/publication or saved
receipt from a canonical reload, take original admission -> head custody, validate the exact current
wrapper/pin/scope/head, and perform a durable exact-byte rewrite (or equivalent sync of that exact
file AND its `owner-state` parent). Store::open syncing the vault root alone is insufficient for this
child directory. A failed barrier leaves the loaded state private/unbound and retains uncertainty;
only an actual completed barrier releases its exact retained obligations. This also applies when the
previous process died after rename but before recording an error. Reopening cannot clear debt by
calling the bytes installed, and no unrelated new transition may substitute for this repair.

Actual export cancellation retains the FULL original admission observer plus writer cancellation
using existing `RequestCancellation::with_authority` conjunction. `authorized` checks both and
`revoked` waits for either; do not replace/strip an original authority or copy a Sender-owning lease
into export jobs. Writer removal closes its own authority; admission retirement immediately closes
the conjunction through the original weak atomic, without either writer lock or a watcher task.
Already committed signed history remains immutable; stale uncommitted branches cannot export.

Tests mount the same genuine owner snapshot as two numeric IDs, hold two different actual
Add/Remove preparations, and prove only the original current-head writer can sign/commit/export.
After replacing/removing that Actor, stale alias import must resume the known canonical head or
refuse. Include actual post-rename uncertainty across both aliases, global concurrent pin insertions,
revocation during lower submission, and independent physical mount exclusion. Do not claim this
prevents a maliciously copied finalizer key in another vault from violating the signed trust profile.

## Whole candidate and install inventory

Build on an isolated restored MLS provider/group BEFORE actual observed Add/Remove; the actual
owner signer/current authenticator/profile and native instance are captured before await and
rechecked at final custody. Do not call `admit_now`, staged membership helpers, subscription drivers
or outbox drain on live state before save. CHECKPOINT-1 initially supports the strict single-owner,
no-staged-commit, plain fresh-KP admission/removal path; unsupported companion/cascade/succession
operations refuse before mutation, not halfway through an acknowledged transition.

The covering snapshot encodes private candidate fields directly; do not temporarily replace only
live `device`/`group` and let other fields or callbacks observe a mixed graph. The typed candidate
must account for every item below. Extract shared snapshot encoding deliberately and keep the
unpinned legacy encoding unchanged. Candidate/profile serialization stays private until the save.

| State | Preparation, covering save and post-save handling |
| --- | --- |
| MLS device/provider/group | Isolated copy from actual current MLS snapshot; actual observed Add/Remove runs there. Save all provider/init/epoch state with its public facts. Install only after durable result. Never put private provider bytes in public archive/DTOs. |
| Past epoch keys | Capture keys for all open document IDs before advance, clone/evict under candidate epoch. Fail strict preparation if a required key cannot be captured; do not inherit a warn-and-continue loss path. Original message graphs remain retained. |
| Routing | Candidate label, secret window, control topic/window and desired-subscription plan. Remove rotates exactly once; signed removal publishes on the ORIGINAL pre-rotation topic. No subscribe/unsubscribe occurs during prepare/save. |
| Owner tenure/admin state | Candidate observed tenure/archive and full-device admin roster/generation, pruned against actual new group; fixed founder remains current owner. Never infer past office from current roster. |
| Invites and replies | Candidate invite-ledger consumption plus exact request/KP-bound response/cache and one-time retry receipt. Save recoverable Welcome/routing/bootstrap bytes in the prepaid publication obligation. Do not let legacy FIFO cache eviction discard them. |
| Commit/control history | Candidate signed CommitRecord and bounded commit log; required public authority facts are separately retained, not dependent on the 256-record log. Membership counters/dirty revisions change with the committed candidate. |
| Public evidence/core | Original signed Profile/Admission/RealmGrant/Basis/Acceptance/Removal facts, original typed actions, exact request receipts, quota reservations, and affected admission/case/projection state. Restore derives semantics/admission from original proofs, not saved authority flags. |
| Local scope/fresh source | Retain original opaque mounted admission binding and native capture; no fresh source from restoring the candidate. Other-member removal preserves own local lifetime. Self-retirement/unsupported succession refuses this checkpoint path. |
| Peer durable state | Candidate peer records filtered by actual new roster, exact record revision and immutable before-removal descriptor references needed by cleanup. Fresh key/route claims cannot borrow short fingerprints. |
| Operational peer state | After durable install, invalidate removed signer proofs/receipt sets and existing eviction/record-bound reachability/retry/repair/discovery state using the original verified descriptor plus current remaining roster. Keep unrelated connection/path state. Split pure cleanup from routing rotation so it does not rotate twice or lose resolution by pruning descriptors first. |
| Network/transients | Keep actual transport, Clock/RNG seams, subscriptions and unaffected live queues. Queue only saved exact publication/eviction/subscription obligations after full install and lease release. No driver/callback sees private candidate state. |
| Product/audit revisions | Advance actual archive/admin/channel revisions even with equal IDs/counts. Emit moderation/channel updates for committed changes and host repair; privileged reads expose only committed retained rows. |

Every new affected field must either enter this candidate/install plan or be demonstrably derived
after commit. Actual-diff review must trace `snapshot_epoch_keys`, routing helpers, tenure observation,
invite/admit caches, `record_commit`, `note_removal_applied`, `queue_eviction` and its record-bound
peer cleanup. An incomplete field list is not permission to keep the live field accidentally changed.

Use the SAME Actor Ready/lease, per-server persistence serializer, current mounted Store, exact
registry incarnation and original UI/native scope, plus the logical canonical-head custody above.
Native ordinary persistence already acquires its serializer before requesting its Actor snapshot;
preserve that order, then obtain admission/head custody in the defined order without holding a
head writer while waiting for admission. Save MLS + archive/actions + quota/receipt/publication state
in ONE atomic encrypted canonical snapshot referencing the already synced immutable pin. Numeric
server mirrors are optional recovery locators and never the publication barrier or head selector.
Only a real Durable result AND the original Current Binding/head registration release
certificates/Welcome/Commit or audit rows.

NotReplaced leaves original live graph intact. ReplacedNotDurable or unwind after possible replacement
latches the WHOLE candidate, blocks rollback snapshots and freezes further MLS advancement until
exact repair. MLS forks cannot be unioned as if they were message graphs. Original signed CRDT/typed
receive evidence can be merged explicitly into the covering repair without overwriting latest live
history or contradictory owner evidence. Reconcile current peer state before the repair snapshot;
then install idempotent cleanup once. Failed cleanup after a durable save retains a repairable saved
operation; it cannot report rejection, mint a replacement transition or erase signed history.

## First real case and read outcome

1. Actual founder prepares the profile from the fresh factory, installs its immutable pin, and saves
   the initial member grant/basis through the covering transaction.
2. A real second member mints a fresh KeyPackage. Actual observed Add produces its shared admission,
   grant/basis, exact Welcome and bootstrap; store before returning those bytes. The member verifies
   the original evidence using the explicitly trusted captured pin and exact finished join.
3. Sign two original typed Creates with that actual member key. Certify one exact action. Keep the
   second provisional/unproven; do not certify it merely because a replica displayed it.
4. Actual observed Remove freezes only the certified set. In opposite bubbles, deliver the unproven
   action before/after this cut: semantic validity is retained, effective admission is Held in both.
   The certified history remains accepted; no local first-seen or clock exception exists.
5. Persist the same actual held action/cut/proofs and expose its deterministic case through the two
   privileged admin reads. Exact detail opens original signed bytes/body inside moderation. A fresh
   Store/Actor restore with the mounted pin yields the same case/revision inventory.
6. Reuse this case for the next signed approve/reject/reopen plus durable abandon journal; do not
   replace it with a fake message or a separately pinned fixture-only witness.

Tests must include actual registry/candidate rename and parent-sync faults, stale alias/instance and
tail stripping, older managed3 readers, pin/role/session loss, whole-state cancellation/unwind, no
early transport bytes, row/byte/control saturation and full contradictory cuts, exact reply loss,
latest received history through repair, real second-member verification and fresh restore, repeated
crash/orphan-stage cleanup at full quota, reopen of a real rename/parent-sync failure with another
failing restore barrier (no export) followed by exact successful barrier, and the
committed retained case appearing in the existing frontend timeline. Owner reply retry alone does
not recover a crashed joiner whose original private KeyPackage state was lost.
