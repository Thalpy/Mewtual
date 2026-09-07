# Design: epoch close, owner checkpoints and bounded recovery (P1)

Status: accepted design, revision 5; protocol-core implementation and adversarial testing have
started. The current slice defines and tests operation envelopes, closes, receipts and their
crash journals, the persisted epoch gate, durable intent metadata, and bounded recovery slots.
The checkpoint/registry slice adds deterministic raw seeds, receipt-bound checkpoint installation and
vault restore, exact projection-size preflight, and the typed registry materializer. Recovery-slot
transitions now also have a standalone vault-sealed store API with crash-safe replacement/retry,
peak-space accounting, bounded recovery-file inventory discovery and explicit staging cleanup.
Owner receipt decisions now have an accounted vault-backed prepare/completion adapter with exact
crash retries. A combined bounded inventory/cleanup covers owner journals and recovery files;
explicit intent-covering variants and vault-backed local intent preparation add the third family.
Registry epochs add the fourth: inbound edits and owner seals persist, and local edits now save
their intent before the epoch, returning publication-ready ciphertext only after both barriers.
An exact retry reseals the original signed change under current membership, not a new delta.
Registry installation now saves recovery and receipt-covered intent retirement before atomic
successor selection. One saved author-owned intent can be replayed through a checked bounded
store step; network publication, background replay, other managed-file families and production
orchestration are not wired yet. Studio
materializers, settlement orchestration, sync discovery, complete storage integration and app/UI events
remain later slices and the feature is not usable yet. Revision 4
dialled the protocol back to a bounded checkpoint-and-recovery mechanism. Revision 5 makes the
five remaining lifecycle corrections: adoption is folded into the first crash-safe receipt of
each owner tenure (section 11); a newcomer reads old-owner heads provisionally and gets
authoritative verification only from the new owner's first receipt (sections 1, 11, 12);
receipt verification atomically seals the old epoch before the recovery snapshot is computed
(section 7); the recovery transition has one staged slot inside the reserve (section 10); and
the unsettled tail is one epoch (section 8).

Implementation clarification: the first protocol tests made two values that were implicit in
the prose explicit on the receipt wire. The receipt carries the tenure-start group epoch so a
newcomer can recompute the tenure id, and it carries the inherited seed change hash so the new
owner authenticates the exact inherited checkpoint bytes, not only their close. The implementation
also caps a complete signed epoch operation at 256 KiB (inside the 64 KiB domain envelope and
4 MiB aggregate limits), uses conflict-wide rather than epoch-specific repair records so inherited
checkpoint equivocation can be repaired, and stores idempotency markers as collision-free reserved
root keys rather than racing creation of one shared metadata map. Finally, authoritative receipt
verification takes the locally observed tenure start or a fresh request-bound owner head proof;
the persisted gate is bound to the full server/logical-document scope and concrete epoch id;
receipt-book ingest and the `Open` to `Closing`/`Fault` transition occur under that gate's one
lock. Restored documents also rebind Automerge's local actor to the full device identity before
P1 permits another edit. As before, the current owner key alone cannot distinguish old and new A
tenures after an A-to-B-to-A succession.

Builds on `catcoms-replication` (`EncryptedDoc`, `SignedOp`, `SealedOp`), `catcoms-sync`
catch-up, the MLS designated committer in `catcoms-mls`, the sealed blob store, the backend
`Clock` seam, and the `catcoms-storage` padding scheme. It does not use the roles document for
any authority decision.

## 1. What this protocol guarantees

**The problem.** A replicated document retains every accepted signed operation, so a member
overwriting one field forever grows every peer's disk, memory, sync and verification work
without bound. Bounding history needs a point at which history is agreed and can be pruned, and
agreement needs an authority. The authority is the **server owner** (the MLS designated
committer), who already serialises admission and signs the roster.

**What is being protected.** Flipnote frame lists and metadata, score cells, studio indexes,
announcement doodle-wall metadata, and edits made while disconnected or while a rotation
awaits the owner. Image blobs are immutable and outside this protocol.

**Guarantees.**

1. Every peer's retained state per logical document and per server is bounded (section 12).
2. Peers holding the same receipts and the same open-epoch operations converge on the same
   materialization.
3. A joining peer reaches the current state of a logical document with the latest checkpoint
   plus at most one open epoch, never a root walk. After an owner succession, a newcomer can
   **read** an old-owner head provisionally but can **prove** its view current only once the
   new owner has issued that document's first receipt of the tenure; existing peers keep the
   state they verified earlier.
4. Nothing accepted into an open epoch is removed before an owner receipt, and then only after
   the epoch is sealed and the excluded content is persisted in a recovery snapshot.
5. An edit is **provisional until a receipt covers it**. If a checkpoint excludes it, its author
   replays it automatically from a durable local intent, and it is visible in recovery. There
   is no stronger promise.
6. Recovery keeps the previous two materialized versions of a document that lost a fork, warns
   before evicting one, and offers Restore, Copy into current version, and Export.
7. Owner equivocation is a visible read-only fault ended only by a signed repair.
8. The owner is one honest client. A malicious owner is out of scope.

**Availability.** Rotations settle only when the owner signs. Epoch 0 and the open epoch are
always editable; a freshly rotated document takes vault-sealed overlay edits until its receipt
arrives. After a succession, a document rotates again only once the new owner issues its first
receipt for it; reading and open-epoch editing continue meanwhile.

## 2. Conventions

Every hash is `H(domain, part1, part2, ...)` = SHA-256 over the concatenation of, for each part
in order, a 4-byte big-endian length followed by the part's bytes. `domain` is ASCII. Strings
are UTF-8 with no normalization. Integers are 8-byte big-endian. Identities are the 32 raw
device-id bytes. Change and record hashes are 32 raw bytes. "Canonical JSON" is the
serialization the jam patch validator already uses. Every derivation ships a golden vector.
Every persisted decision, fault and receipt is keyed by `(server id, doc type tag, logical
key, closed_epoch, tenure id)`.

## 3. Logical documents, epochs, ids

A **logical document** is `(doc type tag, logical key)`, a chain of **epoch documents**.

- Epoch 0: `first 16 bytes of H("catcoms-doc-epoch0:v1", doc type tag, logical key)`. Many
  documents never leave it (section 6 lower bound), bounded by the epoch maximum.
- A close of `closed_epoch = e` names `checkpoint_epoch = e + 1` with id
  `first 16 bytes of H("catcoms-doc-epoch:v1", doc type tag, logical key, e + 1, close record
  hash)`. A checkpoint id is authorized by a locally validated close or a verified receipt naming
  that close hash; an unauthorized id accepts only bounded seed metadata.
- A peer retains epoch `e` until the checkpoint is verified and installed, then prunes
  through `e`.

## 4. Domain operations

```
DomainOp v1 = { v:1, nonce (16 bytes), type, key, op: <type-specific canonical JSON> }
op_id = H("catcoms-domain-op:v1", logical key, verified outer author identity, nonce)
```

`op_id` is derived only, recomputed by every receiver from the verified outer `SignedOp`
author. Every collection element has a stable 32-hex element id chosen at creation; collections
project as ordered sets by element id; concurrent insertions of different ids after one
predecessor order by ascending `op_id`; insertions of the same id with different payloads
resolve to the smallest `op_id` with others shown as conflicts; a tombstone wins over any
insertion of its id; scalars project by Automerge's concurrent-put rule within an epoch and by
the checkpoint's bounded conflict data across one; the envelope is at most 64 KiB; delivery
order never changes a projection. An operation is applied as one Automerge change that also
writes the marker of section 9, into an epoch that is `Open` (section 7).

## 5. Budgets and accounting

An epoch's user-content budget is 20,000 signed operations and 4 MiB, hard maxima at ingest and
in the editor, every signed operation counted in full. A **per-device share** of 5,000
operations and 1 MiB per epoch applies to every device except the owner's. Reserved space per
epoch: 2 MiB seed, 64 KiB closes, 16 KiB receipts. One complete signed operation is capped at
256 KiB before admission; its canonical domain envelope remains capped at 64 KiB.
P1 v2 deltas are raw Automerge kind-1 changes. Before Automerge parsing, the codec scans the
raw column framing without expanding runs: at most 65,536 primitive actions (7 for registry
operations), 1,048,576 expanded column cells, 262,144 predecessor references, and 16 MiB of
expanded key strings per change. String lengths must fit their encoded column before allocation;
value lengths must fit the raw-byte budget; unsupported or compressed column specs reject.
These are parser-work limits, separate from the signed-operation count, wire bytes and typed
schema limits. A newly admitted operation's inner signing key must be a current roster member;
group sealing by a relay alone does not authorize the inner author. Already accepted history
survives removal, while receipt-authorized historical closes and seeds use their separate paths.

## 6. Close records

```
CloseRecord v1 = { v:1, server id, doc type tag, doc id, closed_epoch,
                   heads: [<change hash>...] (1..64, distinct, ascending),
                   author public key, signature }
signature over  H("catcoms-close-sig:v1", server id, doc type tag, doc id, closed_epoch,
                  canonical record bytes without the signature)
record hash     H("catcoms-close:v1", canonical record bytes without the signature)
```

Ingest: at most 4 KiB, rejected before parsing above; unknown fields reject; heads ascending;
`server id`, `doc type tag`, `doc id` must match the document; signature verifies; the author
must be a current member, except that a close whose hash equals the close hash in a verified
receipt is accepted as historically authorized. Any unreceipted close from a removed author is
rejected. Valid when its closure satisfies all of: at least 10,000 operations, or at least
2 MiB, or any non-owner device in it at its share; at most 20,000; at most 4 MiB. Seeds, closes
and receipts never count toward the lower bound.

## 7. Open epochs, candidates, sealing and settlement

Each epoch document has a state under one **per-document gate** shared by local edits, inbound
ingest and settlement: `Open`, `Closing`, `Settled`, or read-only `Fault`. Only `Open` accepts
operations.

The open epoch is retained whole while it has no receipt. Closes are candidates in quarantine,
validated against the epoch already held; missing heads trigger ordinary catch-up of that
epoch, at most 3 attempts, then parking (24 h in a slot, then a 32-byte tombstone, cap 1024,
that triggers one re-request if the heads arrive). Quarantine: 16 records per epoch, 2 per
author; one validation in flight per author, 4 per epoch per hour; negative cache 64 per
author, 1024 per epoch, 24 h; smallest record hash counts per author. At most 8 candidate
records per epoch plus any close named by a persisted owner decision or a verified receipt,
pinned outside the cap. Candidate order has no authority.

**Settlement, one at a time per peer, under the gate:**

1. **Seal.** Verifying a receipt for the epoch moves it from `Open` to `Closing` atomically
   under the gate. From this instant no local edit and no inbound operation for that epoch is
   accepted: an inbound operation is quarantined (bounded by the quarantine cap) and later
   dropped once the epoch is `Settled`, because it can no longer enter any checkpoint; its
   author's own intent, if any, replays into the checkpoint. A race between an inbound operation
   and the seal therefore ends in exactly one of two states: accepted before the seal and so
   inside the snapshot computation, or rejected after it.
2. **Snapshot.** Compute the accepted operations in the sealed epoch outside the receipted
   closure and materialize their effect as a recovery snapshot into the **staged slot**
   (section 10). If the storage preflight refuses the staged slot, hold the receipt in
   `Closing`; nothing is pruned and nothing new is accepted.
3. **Install.** Verify and apply the checkpoint's seed (section 8).
4. **Reconstruct.** Replace the sealed epoch with exactly the receipted closure plus the
   receipted close and the receipt, atomically (document, log, snapshot); drop other candidates.
5. **Prune** through the sealed epoch and mark it `Settled`.
6. **Replay** this peer's own unconfirmed intents that the closure excluded into the checkpoint.

Registry store implementation: steps 3-5 select one seed-backed successor by an atomic replacement
of the full Closing restart unit, rather than writing a temporary pruned predecessor. It flushes
that complete source before saving recovery, then durably retires only exact receipt-covered
intent envelopes before replacement. A crash before replacement leaves the source as the finality
proof; afterward the successor's opening receipt and seed are the proof. Exact installation retries
flush the actual successor without rerunning predecessor retirement or replacing newer edits.
Save retries retain their original concrete document id; deliberate replay explicitly targets the
new one. Full-quota capacity orchestration and automatic replay are not live-wired yet.

An intent is **final** when inside a receipted closure. Until then it is retained, vault-sealed,
and replayed wherever the document is next `Open`.

## 8. Seeds, checkpoints, retirement, and receipts

**Seed.** One change whose bytes are a pure function of the close: actor
`H("catcoms-seed-actor:v1", checkpoint epoch id)`, seq 1, start op 1, no deps, time 0, empty
message, operations = the typed checkpoint in canonical order. Deduplicated by Automerge change
hash; golden vector over the whole raw change. The owner writes it with the receipt; after 60 s
any member may. Only `(doc id, change hash)` metadata is retained for checkpoints whose receipt
is unverified (4096 entries per server, no bodies). A seed above 1 MiB travels unpadded because
the padding ceiling is 1 MiB; that disclosure must be added to `THREAT-MODEL.md` before
implementation.

The raw Automerge 0.10 change (chunk kind 1) is the seed wire representation. Compressed changes,
document snapshots and bundles reject before parsing. Verification checks the receipt's complete
change hash and checksum before decoding, then the actor, sequence, start op, dependencies,
timestamp, message and consumer schema. The actor derivation treats the concrete u128 document id
as its 16 big-endian bytes. A raw-byte golden vector pins the entire change. Installation returns
a separate document whose user-operation log excludes the seed; its optional vault-snapshot
extension retains the logical scope, checkpoint epoch, close hash and seed hash. Every accepted
checkpoint edit descends from that seed. Existing seedless vault snapshots retain their encoding.

**Retirement.** The checkpoint carries the canonical projection of the receipted heads plus
bounded conflict data and nothing else. Markers are never carried. Tombstones are never carried
(closed-epoch operations cannot enter a checkpoint, element ids are random, and replay consults
recovery snapshots, which carry tombstones, dropping an intent whose element was deleted and
telling its author). Conflicts are bounded to 4 values per field and 1024 entries per
checkpoint; overflow goes into that settlement's recovery snapshot. Every edit preflights the
exact encoded next checkpoint and is refused with "document full" above 2 MiB, so checkpoint
size plateaus under any number of rotations.

**Receipt.**

```
Receipt v1 = { v:1, server id, doc type tag, logical key, closed_epoch,
               close record hash, seed change hash,
               tenure start group epoch, tenure id,
               inherited_checkpoint_epoch, inherited close record hash | none,
               inherited seed change hash | none,
               owner public key, signature }
signature over  H("catcoms-receipt-sig:v1", all fields except the signature)
tenure id       H("catcoms-tenure:v1", server id, owner public key, group epoch at which this
                  device became designated committer)
```

Owner-only, at most 1 KiB, verified at ingest against the peer's own group state (the signer is
the current designated committer) and the document's current tenure id (section 11). A peer that
observed the succession also checks `tenure start group epoch` against that transition; a newcomer
uses the carried value to recompute the signed tenure id without claiming access to pre-join MLS
history. The `inherited_*` fields carry the adoption (section 11), authenticate both the inherited
close and its exact seed, and are identical on every receipt of one tenure for one document.

**Crash-safe issuance, constant state.** Per logical document the owner keeps its high-water
`(closed_epoch, receipt bytes)` and at most one in-flight decision persisted atomically and
vault-sealed before publication; after restart it republishes the in-flight decision. A
decision is irrevocable while it is the high-water and replaceable once a later epoch's
receipt is published, since every peer rejects closes at or below its own high-water. Per
logical document a peer keeps the latest verified receipt and the previous until installed.

**Editing and depth.** Epoch 0 and the `Open` epoch accept shared edits from any member. A
checkpoint epoch becomes `Open` once its receipt is verified and its seed installed; before
that, edits are an overlay of intents rendered onto the expected checkpoint. Because a
checkpoint opens only after its predecessor's receipt, **there is at most one unreceipted epoch
beyond the latest receipted checkpoint**: the open one. A close for the open epoch is a
candidate; a close for anything else is rejected. The "unsettled tail" a joiner fetches is
therefore exactly the open epoch.

**Fault and repair.** Two valid receipts under one tenure for one epoch naming different closes
put the document into a read-only **fault** state, both kept, surfaced to everyone. It ends
with a `ReceiptRepair v1 = { v:1, server id, doc type tag, logical key, tenure id, receipt hash a,
receipt hash b, selected receipt hash, repair sequence, owner public key, signature }` (at most
1 KiB, sequence strictly increasing, persisted before publication like a receipt). The record
selects the entire conflicting receipt rather than one epoch so it also repairs differing inherited
fields first observed on different receipt epochs. Applying it is held until the losing receipt's
checkpoint, if held, is persisted as a recovery snapshot through the staged slot.

Implementation note for owner issuance: `ServerStore::prepare_epoch_owner_receipt` now persists
the bounded canonical owner journal before returning. `mark_epoch_owner_receipt_published` reloads
and durably records completion; an exact completion retry cannot clear a later pending receipt.
A verified strictly newer tenure can replace an unfinished old-tenure decision under the same
write barrier, preventing a returning owner from being stranded. The scope-sealed record charges
protocol bytes and shares recovery's logical-document reserve owner. This is not yet a publisher:
the future coordinator must validate closure/seed before signing, re-save/reverify before sending,
and integrate the combined namespace/orphan inventory and cleanup with the other managed types.
Reads are historical only.

## 9. Intents and markers

An intent is a domain operation held vault-sealed until final. When authored, the same
Automerge change writes the reserved root scalar `_p1/op/<64-lowercase-hex-op_id> = 1`; edit and
marker commit atomically and the whole change is charged to the epoch. A root key avoids the
concurrent-create race of a lazily-created shared map, and every domain materializer ignores this
reserved prefix. Replay into an `Open` epoch is idempotent by the marker keys; replay into a
checkpoint consults recovery snapshots for tombstones and applies otherwise.
The registry's stable keys need a narrower rule than Studio's random element ids: its store
replay step HOLDS, never deletes, a saved intent when new authoring targets current/retained/staged
deletion evidence or would replace a higher current admitted/overflow pointer hint. It accepts
only the saved id and an explicitly captured existing Open epoch, requires the original current
local author, and never rewrites the nonce/envelope. Because origin epochs are not recorded in
the ledger, even a deliberate later re-put can conservatively need explicit recovery. After
two-snapshot eviction, absent deletion evidence is only best-effort protection.

An exact authenticated CURRENT-log operation instead reseals its original signed bytes, without
changing materialization, even if deletion/newer hints arrived afterward. This exception requires
full-envelope equality, not a marker, and keeps a Tombstone's failed post-rename flush retryable.
All recovery validation, scope/current-author checks and both vault barriers still apply. The
single-step API returns prepared ciphertext or a held reason; it neither schedules/sends replay
nor retires held intents. Live replay scheduling and explicit recovery actions remain later work.
Caps: 64 KiB per intent, 10,000 intents and 4 MiB per logical document, 64 MiB per vault,
20,000 markers per epoch (one for each operation admitted by the epoch maximum).

## 10. Recovery snapshots and the staged slot

Implemented persistence prerequisite: `ServerStore::update_epoch_recovery` reloads and saves the
complete slot record under exclusive mutable store access. It returns only after the existing
file-sync/rename primitive succeeds (plus parent-directory sync on Unix). The sealed record binds
local server id, full MLS group id, document type and logical key, and retains the last completed
eviction pair so an acknowledgement can be retried after a post-rename flush failure. The original
warning deadline survives reload and retries. Reads never evict. Encoders preflight the exact
aggregate snapshot size before allocating; reads cap file bytes before unsealing.

This API does **not** yet install/prune epochs or accept network requests. Its three slots are
logical: atomic replacement temporarily duplicates ciphertext, and the existing writer can leave
crash-orphan temporary siblings. Global admission/reserve accounting and cleanup must cover those
bytes before integration. Records currently remain after server removal, like held blobs, until
the retention lifecycle is wired. These are outstanding implementation tasks, not guarantees this
standalone slice claims to have completed.

A **recovery snapshot** is a typed materialization of content that lost a settlement, a repair
or a succession rewind:

```
RecoverySnapshot v1 = { v:1, doc type tag, logical key, epoch, base close record hash | none,
                        reason: excluded | rewound | conflict_overflow | repair,
                        projection: <typed canonical projection>,
                        tombstones: [ {element id, op_id, author} ],
                        elements: [ {element id, predecessor element id, op_id, author} ],
                        conflicts: [ {field or element id, values: [{value, op_id, author}]} ],
                        applied_ops: [ op_id ... ] }
```

The tombstone and element lists are strictly ordered by element id, conflicts by target and then
value operation id, and applied operations by operation id; duplicates reject. All three physical
slots must name the same logical document.

**Registry recovery specialization (implemented, excluded settlements only).** Registry keys are
typed logical keys, not random element ids. Its generic `tombstones`, `elements` and `conflicts`
arrays are empty; the projection payload retains complete admitted pointers, overflow and
pointer-key tombstones, plus excluded domain operations with full author identities. Inherited
seed-only pointers have no invented operation author. The outer `applied_ops` is the sorted union
of accepted source-operation ids; excluded operation ids must be a subset. The outer base close
is the source epoch's opening close, absent only at epoch zero. The selected receipt hash is
historical provenance, not current authority or permission to replay another author's operation.

Payload v1 uses the existing big-endian/length-framed codec, in this exact order: `u8 version=1`,
`bytes group_id`, `u8 bucket`, `bytes receipt_hash[32]`, admitted-pointer list, overflow-pointer
list, tombstone-key list, excluded-operation list. Lists start with `u32 count`. Pointer entries
are `u16 type_tag, bytes logical_key, u64 target_epoch`; tombstones omit target_epoch. Excluded
entries are `bytes author[32], bytes DomainOp-v1`, strictly ordered by derived operation id.
Pointer/key lists are strictly ordered by type/key, mutually disjoint; overflow requires all 2048
admitted slots occupied. The combined key count is at most 2048 seed keys plus 20,000 operations,
and the entire generic snapshot remains at most 6 MiB. Exact size preflight precedes encoding.
The payload deliberately excludes quarantine/gate bookkeeping: identical recovery content keeps
its snapshot id and warning deadline across retries despite newly quarantined packets.

`stage_registry_recovery` recomputes from checked Closing state under exclusive store access,
verifies its source accounting and (before a nonempty save) all existing typed slots, then uses the
accounted recovery adapter. No evidence needs a slot when exclusions, overflow and tombstones
are all empty; that path does not inspect or claim health of old recovery files. Failure or an
eviction-pending warning never installs/prunes the source. The
first/second snapshot may still refuse at the content ceiling until a future multi-record
settlement reservation is implemented. Restore/Copy/Export, repair/rewind-specific typed records,
checkpoint installation and intent retirement remain separate work; this adapter grants none
of those actions implicitly.

At most 6 MiB, preflighted before persistence. Physical state per logical document is **two
retained slots plus one staged slot**, and the staged slot is counted in the settlement
reserve (section 12). The transition:

1. A settlement writes its snapshot into the staged slot. If both retained slots are free or
   one is free, the staged snapshot is promoted immediately and settlement continues.
2. If both retained slots are occupied, the peer emits `RecoveryEvictionPending` carrying both
   the oldest snapshot id and staged snapshot id, shows "a previous version will be removed" with
   **Export**, and holds settlement in
   `Closing` for a grace period: until the warning has been acknowledged or 7 days have
   passed, whichever first.
3. After the grace period the oldest retained snapshot is evicted, the staged one is promoted,
   and settlement continues from step 3 of section 7.

A retry with the exact staged snapshot is idempotent and retains the original deadline. The
acknowledgement echoes both ids, so a delayed UI action cannot authorize a later eviction. A crash
at any point leaves at most three snapshots on disk with the staged one marked staged;
restart resumes the transition from the persisted step. A rewind deeper than two epochs
materializes only the two most recent epochs of the losing branch, offering Export of the rest
before dropping them. Each retained snapshot offers **Restore** (domain-level difference applied
as operations by the restoring member, additive, conflicts shown), **Copy into current version**
(replacing conflicting elements after confirmation), and **Export**. Restore re-puts a registry
pointer only for a document with a verified non-empty epoch 0 or a verified receipt.

## 11. Owner succession: adoption inside the first receipt

When the designated committer changes, nothing stops. Reading and open-epoch editing continue
under the receipts peers already hold. What stops is rotation: a receipt from the new owner is
accepted only once it also **adopts** the document, and adoption is not a separate record. The
new owner's **first receipt of its tenure for a document** carries `inherited_checkpoint_epoch`
plus the inherited close and seed change hashes, naming the exact checkpoint it chose to inherit
(the highest one it holds a verified receipt for, or epoch 0 with neither hash). That receipt is issued under the same
persist-before-publish rule as every receipt, so a crash cannot produce two adoptions: the
decision `(logical key, tenure id, inherited checkpoint)` is persisted before publication and
reused after restart, and every later receipt of the tenure repeats the same inherited fields.
A receipt whose inherited fields differ from the tenure's persisted first receipt is an
equivocation and enters the fault state like any other.

Peers verify a first-of-tenure receipt against the current committer and the tenure transition
they observed (or the fresh owner head proof in section 12), record the tenure id and
the inherited checkpoint as the document's baseline for that tenure, and treat any
old-tenure history above the inherited checkpoint as a rewind into recovery snapshots. The
initial tenure id, before any succession, is derived the same way from the founding owner and
the group epoch at which it became committer, so there is no undefined state. Old-owner
signatures are rejected because the signer is not the committer; tenures of one device are
distinct because the tenure id includes the group epoch.

**Newcomers.** A peer joining after a succession, for a document the new owner has not yet
receipted, can verify neither the old receipts (the signer is not the current committer) nor a
new one (none exists). It reads the head hinted by members provisionally, may edit the open
epoch (edits are provisional anyway), and shows "current owner has not yet confirmed this
document's history". It gains authoritative verification when the new owner's first receipt
arrives. This is the honest contract: owner absence never prevents reading, and can prevent a
newcomer from proving its view current. Once a receipt exists, the newcomer gains authoritative
verification from a fresh nonce-bound proof by the current owner selecting that exact receipt;
a replayed receipt from an earlier tenure of the same owner key remains only a hint.

## 12. Discovery, registry, catch-up, storage

**Discovery.** `ReceiptHeadRequest { doc type tag, logical key, nonce }` (at most 256 bytes,
rate-limited and already bound to the authenticated requester by the request layer), answered by
any member with `{ receipt | none, repair | none, owner proof | none }`. Answers are hints. When
the responder is the current owner and a receipt exists, `owner proof` signs the logical key,
receipt hash, receipt tenure start, full requester identity and request nonce. That one-shot proof
is not a lease and makes no promise about future receipts; it only establishes which receipt the
owner selected for this request, including across A→B→A ownership. A joiner fetches the hinted
head (seed by hash, checkpoint by catch-up, the open epoch
through its close if any) or epoch 0 if none, and starts working; edits made under a stale
view are provisional and replay per guarantee 5. Registry pointers store
`current_checkpoint_epoch`, 0 meaning unrotated; a pointer more than one epoch beyond the
peer's head for that document triggers one head request, never a walk.

**Registry.** 256 logical documents, bucket = the first byte of `H("catcoms-registry-bucket:v1",
doc type tag, logical key)`, rooted at epoch 0 of `("registry", server id, b)`. Operations
`put_pointer(key, checkpoint_epoch)` and `tombstone_pointer(key)`; reclamation only by
tombstone; stable admission of 2048 live pointers per bucket with previously admitted pointers
keeping their slots; every seed carries the complete projection; warn at 3900 epochs,
read-only at 4096 pending migration.

Implementation format: the bucket logical key is `H("catcoms-registry-key:v1", server id, b)`
with `b` framed as a u64. Target keys carry both their document type and logical-key bytes.
The exact canonical bodies are `{"epoch":0,"key":"<lowercase hex>","t":"put_pointer","type":16}`
and `{"key":"<lowercase hex>","t":"tombstone_pointer","type":16}`. The physical root uses
`bucket`, `epoch`, `key`, `kind:"registry"`, `v:1`, and flat `p/<four-hex-tag>/<hex-key>` values
for pointer epochs, `d/... = true` for tombstones and seed-only `s/... = true` for admitted slots.
Flat keys avoid concurrent creation of competing nested maps. Only seed verification can introduce
slot keys. A registry pointer number is a discovery hint: concurrent values choose the maximum,
not an authority or history winner. This is specific to registry hints, not Studio scalar values.
Target numbers span u64; the 4096 ceiling applies to the registry's own epoch. Tombstones win
throughout an open epoch and retire at rotation. Overflow remains explicit in the materializer
for settlement recovery; it is not silently included in an over-cap seed. Both local and inbound
registry changes preflight the exact encoded next seed, with a fixed-size placeholder close hash.
The maximal-key bucket fixture pins capacity and a repeated-rotation fixture pins retirement.
Registry change validation also binds every predecessor operation id to the same root property
at the change's causal heads. Immutable headers are only created when absent, preserving harmless
equal-value conflicts from concurrent bucket creation. An already-applied put/tombstone may write
only its new intent marker, but the requested value must already exist at those exact causal
heads; a value held solely on another merged branch cannot justify the no-op.

The implemented `RegistryEpoch` coordinator now keeps that typed document, gate, receipt book
and opening receipt privately owned under one exclusive mutation interface. Its bounded local
restart format stores the raw seed and signed operations, not a separate Automerge save; restore
rebuilds the DAG, validates typed changes and matches complete gate metadata and receipt phase.
Receipt admission only closes editing and retains the full source. The store now attaches it to
vault persistence/inventory for inbound edits and receipt seals, returning outcomes only after
the save/flush barrier. Local edits now join intent preparation and registry persistence, while
live publication/discovery remain unwired. The store installation transaction now saves recovery
and receipt-covered intent retirement before atomically replacing the source. The core successor
builder alone leaves the predecessor untouched and gives no authority to discard it.

**Catch-up.** Requests carry up to 64 heads and an opaque provider cursor bound by HMAC to
`(provider, requester, doc type, doc id, log generation, position, expiry 10 minutes)`; pages
are topologically ordered and dependency-complete relative to the heads plus everything
delivered under the cursor; a reconstructed log bumps the generation and answers "restart".
Catch-up serves seeds by change hash, closes, receipts and repairs by record hash, receipt
heads by logical key, and operation pages by document id.

**Storage.** Preflight admission under one accounting lock before any inbound record or local
commit. Never evictable: the open epoch, the current checkpoint and its predecessor until
installed, receipts, repairs, closes, registry epochs, intents, the staged snapshot. A
**settlement reserve** of 48 MiB covers one serialized settlement: the staged snapshot (6 MiB),
a 2 MiB seed, a reconstruction copy and temporary files, conflict overflow; a 16 MiB allowance
covers receipts, closes and protocol seed metadata. Peer-writable registry history, seed bodies
and admission metadata charge ordinary content instead, so peers cannot exhaust receipt space.
Only exact receipt growth in a registry record charges the protocol allowance. Settlement still
requires sufficient protocol and reserve headroom. If the preflight fails and nothing safely
evictable remains (retained snapshots past their warning, then receipted non-current epochs of
documents not opened in 30 days), the peer refuses the content and shows "storage limit reached".

| State | Bound |
|---|---|
| open epoch user content | 4 MiB, 20,000 operations; per non-owner device 1 MiB, 5,000; markers 10,000 |
| protocol reserve per epoch | 2 MiB seed, 64 KiB closes, 16 KiB receipts |
| checkpoint | 2 MiB encoded, preflighted on every edit; conflicts 1024 x 4 |
| close quarantine | 16 records, 2 per author, 4 KiB each; parked tombstones 1024; sealed-epoch operations share the cap |
| candidate closes | 8 per epoch plus the pinned decided or receipted close |
| negative cache | 1024 per epoch, 64 per author, 24 h |
| seed metadata for unverified checkpoints | 4096 entries per server |
| receipts per logical document | latest verified, previous until installed |
| owner state per logical document | high-water receipt, one in-flight decision, first-of-tenure decision, fault evidence of 2 receipts and the latest repair |
| retained epochs per logical document | last receipted checkpoint, predecessor until installed, the open epoch |
| recovery snapshots | 2 retained plus 1 staged per logical document, 6 MiB each |
| intents | 64 KiB each, 10,000 and 4 MiB per logical document, 64 MiB per vault |
| registry | 256 buckets, 2048 live pointers each, 4096 epochs each |
| studio storage per server | 2 GiB including a 48 MiB settlement reserve and a 16 MiB allowance |

**Ingest limits per device**: per epoch document sustained 10 operations per second, burst 50,
plus the share; per server sustained 50 per second, burst 200, at most 16 documents with
in-flight validation work; not relied on for correctness.

Implemented admission prerequisite: `EpochStorageBudget` takes a complete trusted local inventory
and splits the 2 GiB total into 1984 MiB ordinary content, 16 MiB protocol allowance, and 48 MiB
settlement reserve. Its fixed-size inventory map has a local 65,536-record rail, counting empty
crash-orphan files too. An over-rail inventory requires bounded cleanup before reconciliation,
never truncation of accounting. Single-record
replacement checks both final occupancy and the peak old inventory plus full replacement and
scratch; no planned deletion is subtracted before I/O. Settlement copies may use the reserve,
but ordinary writes may not. Held staged bytes pin that reserve to one document. A reservation
sets the budget unready before returning; dropping or forgetting it requires complete inventory
reconciliation. Successful I/O commits prevalidated counters; pre-I/O cancellation alone refunds.

The accounted recovery adapter observes the actual authenticated old record (full content/staged
split, physical ciphertext length and document owner), reserves its full replacement, saves and
then commits. It is not the multi-record settlement transaction: a first/second recovery snapshot
becomes retained immediately and can still be refused at the content cap. Supporting a settlement
that frees other history requires the later transaction to hold those new bytes in reserve until
the actual source deletion commits. Complete managed-type inventory/bootstrap, temporary cleanup, and a sole
coordinator excluding the low-level unaccounted API remain required before production wiring.

Recovery-namespace discovery is implemented as an exclusive borrowing scan, scheduled in steps of
at most 64 directory entries and one bounded authenticated body. It stops at 131,072 visited names
or 65,536 final/temporary records and never exposes partial results as complete. It reconstructs
full scope from authenticated records, validates canonical names, and retains metadata only.
Temporaries are never read/promoted/deleted: verified final destinations identify their ownership
after the whole scan, while any unresolved orphan prevents per-server composition. Known temporary
bytes count wholly as settlement scratch. This remains one inventory component, not the sole
coordinator or permission to prune; all other P1 record types still need inventory integration.

Explicit recovery-staging cleanup is now implemented as a separate borrowing job. It removes
only canonical unpublished atomic-write siblings, never final recovery files or logical staged
versions. Each successful batch (including an empty retry) runs the existing directory sync;
failed batches may have partially removed siblings but expose no completion or budget refund.
EOF transitions directly into a new inventory while retaining exclusive store access. A scan may
still find missed orphans because deletion affects directory traversal; another bounded cleanup
pass is permitted. Accounting is released only by reconciliation of complete current inventories,
not by the cleanup's observed-byte counter. Source history/intents must survive until their final
save succeeds. Startup/settlement wiring and cleanup of other managed types remain future work.

`scan_epoch_storage` and `cleanup_epoch_storage_staging` now extend that same engine to the union
of recovery and owner-journal files without relaxing aggregate rails. Coverage stays explicit and
unchanged across cleanup, scan and completed metadata; old recovery-only APIs preserve their
scope. Owner bodies use their small namespace-specific cap and schema; their saved bytes charge
protocol allowance. Orphans require an authenticated destination of the SAME namespace, never a
matching digest in another family. Combined cleanup removes only unpublished attempts, never
saved pending/high-water decisions. This union is not all P1 storage or a production budget bootstrap.

Local intent preparation is now a standalone vault adapter. It stores the existing bounded
`IntentLedger` under a full local-server/group/type/key scope, with the author derived from the
actual local MLS device and checked against current membership. New intents save before success;
no accepted intent is removed by this adapter. An exact matching retry authenticates the existing
ledger (including any newer intents), then syncs that unchanged file and its parent without a
second copy. This repairs the post-rename durability boundary even when no copy fits at the cap.
New writes charge full ordinary-content replacement peak, not the settlement reserve.

`scan_epoch_storage_with_intents` and `cleanup_epoch_storage_staging_with_intents` opt into all
these three families; earlier coverage stays unchanged. Intent temporaries charge content
and cleanup never removes final ledgers. The dedicated vault intent budget is constructed only
from completed inventory including all three families. Its 64 MiB cap conservatively counts physical final bytes,
framing, all intent temporaries (even unresolved ones), and peak replacement copies. A private
mounted-store generation invalidates older budgets and scan results before intent write/sync or
cleanup attempts; failures require fresh inventory reconciliation. Per-document 10,000-intent /
4 MiB canonical-operation limits still come from the core ledger. Global intent metadata also has
the scanner's 65,536-record rail. This does not admit other managed families' metadata: the sole
coordinator still must compose all storage types, validate domain semantics before preparation,
replay only as the original author, and enforce the ordered source/recovery/retirement/selection
barriers in section 7; registry store adapters now do so, but live orchestration remains unwired.

The explicit `scan_epoch_storage_with_registry` and `cleanup_epoch_storage_staging_with_registry`
now include the fourth implemented family, scope-bound vault `.registry-epoch` records. Inventory
reuses full raw-seed/signed-log/gate/receipt validation, but returns metadata only and requires no
present-time owner authorization. Inbound edits and seals reload under an exclusive store borrow;
no arbitrary replacement API exists. Changed bytes reserve the full replacement peak, identical
bytes use a sync-only retry, and no result escapes a failed write/flush. Seals keep the full source.
Unpublished registry attempts conservatively charge ordinary content (including receipt-write
attempts), so an orphan at the content ceiling may require explicit cleanup before reconciliation.
This is still not a sole all-family coordinator or live transport path; graph rebuild work needs
the specified ingress scheduling/rate limits when integrated.

Local `edit_registry_epoch` now performs canonical/scope/current-author and Open checks before
journaling, including a full-envelope comparison against any retained operation with the same id.
It then durably prepares the intent, reloads the epoch and either applies the edit or reseals its
exact saved signed operation; only after the epoch save/flush does ciphertext return. Failure of
the second record leaves a valid durable intent, never a published operation without recovery data.
Exact retries sync both unchanged files without replacement copies, including when restoring has
derived a new quota-exempt owner. No new persistence or wire version is required. Markers do not
retire intents, Closing/Fault refuses publication preparation, and automatic cross-epoch replay
remains part of recovery. Actual sending must recheck session, server incarnation, current
membership/MLS epoch and the retained document's Open lifecycle; this API sends nothing itself.

## 13. Application events

`AppEvent::SettlementChanged { doc type tag, logical key, state }` on every change of a
document's settlement state: `Open`, `Closing`, `AwaitingReceipt`, `Settled`,
`HeldForStorage`, `Fault`, `Repairing`, `AwaitingTenureReceipt`, `RecoveryAvailable`,
`RecoveryEvictionPending`, `StorageRefused`. The desktop bridge forwards it with the same shape
as `StatusUpdated` (tested).

## 14. Tests

- Owner crashes before and after publishing its first post-succession receipt, with different
  checkpoints discoverable in between; in both delivery orders every peer records the same
  inherited checkpoint and seed hash and the same recomputable tenure id, and any later differing
  receipt is a fault.
- A newcomer after succession reads an old-owner head provisionally, edits the open epoch, and
  gains verification when the first tenure receipt arrives; its edits replay if excluded.
- Settlement paused after computing the closure; an authenticated inbound operation races; it
  is either inside the recovery snapshot or never accepted, on every interleaving.
- Both retained snapshot slots full; a third settlement stages, emits the eviction warning,
  holds in `Closing`, and completes after acknowledgement or 7 days; crash and restart at every
  step of the transition leave at most three snapshots and resume correctly.
- Alice edits while partitioned; the owner receipts without her edit; on reconnect her edit
  replays once and is visible in recovery until confirmed.
- A joiner told "epoch 0" by a stale provider while a receipt exists edits epoch 0, then learns
  the receipt; its edits replay into the checkpoint.
- A close for any epoch other than the open one is rejected; a joiner fetches exactly the
  checkpoint plus the open epoch.
- Thousands of create, delete and overwrite rotations: checkpoint size plateaus; no marker or
  tombstone in any checkpoint.
- Ten thousand rotations: owner and peer receipt state constant-sized.
- Owner crash before, during and after publishing a receipt: the same receipt republished.
- Two receipts for one epoch: fault on every peer; restart during repair; a joiner after repair.
- Tenure A, B, A yields distinct tenure ids; an old owner's post-handoff signature is rejected;
  a hidden higher old-tenure receipt discovered later becomes a recovery snapshot.
- One-member server fills, closes, settles and continues; a single non-owner writer at its
  share can rotate.
- A receipted close by a removed author fetched under the receipt; an unreceipted one rejected.
- A maximal excluded snapshot settling at the storage cap within the reserve, staged slot
  included.
- Same logical-key bytes under two document types kept separate.
- Registry stable admission, tombstone reclamation, identical seeds across writers, pointer 0
  for unrotated documents, a far pointer triggering one head request.
- Settlement events forwarded through the bridge for every state.
- Golden vectors for every derivation, including the whole raw seed change and the maximal
  checkpoints of every consuming type.

## 15. Explicitly not provided

- Irrevocable edits under any lease or attestation.
- Server-wide succession baselines, manifests, or a separate adoption record.
- Preservation of losing branches beyond two snapshots per document.
- Automatic reconciliation of rare partitions beyond replaying the peer's own intents.
- Proof of currency for a newcomer before the current owner has receipted a document.
- Any defence against a malicious owner.
