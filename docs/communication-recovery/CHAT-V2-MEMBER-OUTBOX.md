# Chat V2 ordinary-member durable outbox

Storage integration checkpoint, 11 November. The storage engine in
`crates/catcoms-app/src/store/persistence/v2_member_outbox/` passes all 31 focused
tests: 27 storage cases, including both pressure corrections, and four genuine Core
plan-to-signed-original constructor cases. The production signed-input constructor
now exists and is consumed by the founder-as-member host. **Production activation
is incomplete**: the real member-source save admission, parent-directory provisioning
and transport wiring remain open. Only cfg(test) fixtures can currently construct
the parent directory and save admission. The module's explicit dead-code allowance
must be removed when the real wiring is complete. HANDOVER records the broader gates.

This implements the "Independently durable outbox" section of
[CHAT-V2-ORDINARY-MEMBER-PATH.md](CHAT-V2-ORDINARY-MEMBER-PATH.md). It is independent of
the founder A/I/S/B/WC certification journal (`v2_text`), its closed inventory and the
legacy Automerge `durable_chat` outbox. Founder certification gates nothing here.

## What it guarantees

- A saved member action survives crashes: once `SavedLocal` is reported, the exact signed
  original bytes are durably selected and can be re-read and republished without the founder.
- Exact request-token lookup before authoring: an identical retry recovers the original
  result (even one re-signed with a new nonce); a changed text, kind, target, parents or
  channel is a conflict. A second save of an already stored ActionId under another token is
  refused, whatever its signature bytes.
- Crash recovery at the tested write, barrier and selector boundaries, finishing admitted work
  exactly once and leaving no unreferenced objects behind.
- Publication debt is paged and ordered; a checkpoint advances only over a contiguous
  prefix of exactly submitted (or Held) items. The original bytes never change across
  transport epochs, and the outbox stores no epoch.
- Receipts, entries, original chunks and dispositions are kept after debt clears.
- Storage is bounded by persistent counters; pressure refuses new work and evicts nothing.

What it does not claim: authorization of any kind, exactly-once signing across a crash
before the signed original reaches the outbox, peer receipt or durability on other devices,
or protection against a malicious local process rewriting the private directory (the threat
model's cooperative-directory assumption).

## Namespace and context

One namespace `mo-<64 hex context id>` per member context, under the parent directory of
the declared critical namespace5 (below; never inside a closed founder/group inventory).

### Declared namespace5 (member outbox layout)

Critical catalog namespace5/version1 declares the member-outbox layout. Its descriptor is
the tag `catcoms/private-chat-v2-member-outbox-layout/v1\0`, u16 version1, then three
length-framed b32 fields: vault, root and BLAKE3 of
`catcoms/private-chat-v2-member-outbox-current-descriptor/v1\0 || u64 BE length || exact ID3
descriptor`. Its fixed objects:

```text
<vault>/chat-critical-namespaces.member-outbox.seed   transient two-catalog declaration seed
<vault>/chat-owner-v2/member-outbox-layout.bin        sealed marker: the exact descriptor
<vault>/chat-owner-v2/member-outbox-v1/               parent of every mo-<context> namespace
```

- Namespace5 is independent of governance4 and either may be declared first. Every
  declaration (4 or 5) inserts exactly one entry at its sorted position over the whole
  original catalog (`catalog_declaration`), keeping every other entry byte for byte, with
  checked generation + 1 and the whole-plaintext parent digest. For a catalog whose IDs
  are all below the new one this equals the earlier append-only bytes.
- Only an opener whose `V2CurrentLayoutPolicy` carries `with_member_outbox(..)` reads ID5
  or its artifacts. Every other opener refuses them before any cleaner.
- Binaries that predate ID5 are fenced in two ways:
  - during the declaration, by an owned empty legacy catalog stage. It precedes every
    content-bearing member output, until the declaration commits or its abandonment starts;
  - afterwards, by their closed grammar.

  CI checks this against the real predecessor commit `ce402364`. Every JOIN transition
  preserves ID5 exactly.
- The declaration follows governance4's protocol (empty stage fence, seed, marker, parent
  directory, candidate stage, rename, seed removal), preceded by a zero-byte claim of the
  seed name. Every fence therefore has a claim beside it.
  - **Complete authenticated seed:** cold recovery finishes the declaration.
  - **First seed that never authenticated:** the declaration is abandoned. The cold
    inspector unlinks the zero-byte fence, then the seed. Each unlink is followed by a
    directory barrier and a proof that the name is gone. The warm retry then declares
    afresh. This happens only when there is no marker, no parent and no ID5 in the
    authenticated current catalog.
  - **Anything else without a complete seed** is preserved and refused: orphan markers or
    directories, a lost marker, an authenticated but invalid seed, and any stage that is not
    a valid ordinary successor.

  [Recovery design](CHAT-V2-NAMESPACE5-RECOVERY.md).
- Declared from the Current3 opener after ID3, or from the governance opener after ID4,
  serially after that opener's own declaration debt. Declaring 5 never creates or implies 4.
- `V2MemberOutboxPolicy` supplies an independent pool (sized by
  `V2MemberOutboxPolicy::required_capacity`), the persistent quotas and the per-Store
  context bound. The declaration's transient fee stays on the catalog/JOIN pool.

The context ID is a domain-separated hash of vault, layout root, trusted pin, Profile,
Profile tenure, original group nonce, full device key and group. Transport epoch and the
per-entry Admission are excluded. Consequently a removal and rejoin with the same Profile,
tenure, group nonce and device reopens the SAME namespace; each entry records the Admission
it was signed under, and that is what keeps A1's work from being adopted by A2 (below).
`group_nonce` means the authenticated selected `SignedOwnerProfile.spec().creation`.
It never means a live local Scope nonce, which changes during cold activation.

```text
head.bin               selected local Head
head.stage.v1          selector-CAS stage (only between WAL MAIN and the CAS)
outbox.wal             WAL MAIN: the committed transition being finished
outbox.wal.stage.v1    pre-commit WAL stage
a-<id>.bin             original action chunk pages (general cold-action codec)
e-<id>.bin             entries, one per saved request
n-<id>.bin             trie nodes
```

No caller-supplied or scanned name is ever used, with one bounded exception: when
`head.bin` is missing, at most three names are read to prove the namespace holds nothing
but an interrupted creation.

## Formats

- **Control files** (`head.*`, `outbox.wal*`): deterministic public preamble
  `u32 len || magic || u16 version || context || binding`, then `u32 sealed length`, then
  `nonce || ciphertext` under a member-outbox-specific key derived from the vault db key.
  The plaintext begins with the same preamble, which authenticates it. The head's binding is
  its parent head ID (zero at generation 0); the WAL's binding is the head it starts from.
- **Immutable objects** are private chat pages (shared page crypto). Because that page key is
  shared with other private pages, each plaintext starts with its own role tag and the
  context ID; action chunks wrap the unchanged general cold-action chunk codec (128 KiB /
  64-parent grammar, at most three chunks) with context, ordinal and index.
- **Head**: generation, parent head ID, trie root, next ordinal, publication front, and the
  persistent counters `live_bytes` and `live_objects` over every reachable object.
- **Entry**: context, ordinal, token, request digest, original Admission ID, ActionId,
  original-byte digest, original length and chunk page IDs.
- **Trie**: copy-on-write hash trie, fanout 16, at most 32 rows per leaf, depth at most 16.
  Row kinds: token -> receipt; ordinal -> entry (the paged debt); ActionId -> ordinal and
  original digest; ordinal -> disposition. Positions hash the full key with the context;
  every load re-checks depth, nibble prefix, row positions, strict order and context, and
  full keys are always compared.
- **WAL**: kind, exact candidate head file, every output frame with role and ID, every
  retiree with its byte size, and every settled debt item.

All frames are sealed once under the `catcoms-rt` RNG seam and reused byte for byte by every
retry and by recovery; nothing is ever resealed for an uncertain transaction.

## Transactions

Append (one saved entry) and Checkpoint (advance the publication front) share one protocol:

1. Physical precondition: `head.bin` equals the owner's selected head byte for byte and no
   WAL, WAL stage or head stage exists. A stale owner or unfinished cleanup stops here.
2. Write the WAL stage, sync, sync the directory.
3. Rename it onto `outbox.wal`, sync the directory. **Commit point.**
4. Create or prefix-complete every immutable output; one directory barrier.
5. Write the head stage, sync, re-check the parent is still selected, rename onto
   `head.bin`, sync the directory. `SavedLocal` is cached here, before cleanup can fail.
6. Delete each retiree, directory barrier, delete the WAL, directory barrier.

Step 1 requires the WAL to be absent, so the previous transaction's cleanup is durable
before another can begin, and a failed cleanup leaves the WAL for recovery to finish.
Nothing in the protocol calls a signer, SDK, projection or network, or waits for payment.

Preparation does all fallible authoring first: lookup, conflict and duplicate refusal,
sealing, trie planning (refusing depth or collision exhaustion as `IndexExhausted`, never
dropping a row), and the quota check, all before admission.

### Admission edge

`PreparedMemberAppend::admission_facts()` names the context, selected parent head, ordinal,
token, request digest, ActionId, original digest and original Admission. The one production
issuer of `MemberOutboxSaveAdmission` is the operational host's member-save final edge
(below): it issues the member lane's own save-source end for exactly these facts, re-checks
the Store's member-outbox mount, observes the native session authority last, and only then
calls `at_member_save_final_edge(facts, lease)`. The constructor is visible to
`persistence` because the host is a sibling subtree; the facts alone are labels. The
cfg(test) constructor grants durability only.

`admit` refuses, returning every input unchanged, unless the admission names exactly the
prepared facts and the owner still selects the prepared parent. The admitted worker then
owns the exclusive owner, the presealed bytes, their payment and the lease; `run` cannot be
cancelled and releases nothing until all IO and cleanup have finished. Recovery of an
already-admitted WAL is not a fresh signing or publication admission.

### What a caller may do with each save outcome

Refusal values are plain enums and prove nothing by themselves, so this is a caller
contract that the integration code must follow, not something the outbox can enforce:

| Outcome | Was anything stored? | Caller action |
|---|---|---|
| `Saved(receipt)` | yes, this original | done; keep the receipt, never sign this token again |
| `AlreadySaved(receipt)` | yes, possibly an EARLIER original | treat as saved with that receipt; if its ActionId differs from the in-hand original, drop the in-hand copy and never publish it |
| `TokenConflict`, `Foreign`, `IndexExhausted` | no | permanent for this request; it may be abandoned |
| `DuplicateAction` | already retained under ANOTHER token; it may already have been submitted or be Held | reconcile its existing receipt/debt; report the duplicate-token invariant breach, never present this as a fresh unsaved action |
| `Busy`, `Stale`, `NeedsRecovery`, `Incomplete`, `Payment`, or a `Failed` read error from `prepare_append` | no | keep the original; retry later (recover first when asked) |
| `Pressure(..)` | no | keep the original and retry after debt drains; dropping it is a product choice the UI must report as "not saved" |
| `run()` → `Uncertain` | unknown | `recover`, then exact `lookup`: `Hit` is saved, `Absent` may be re-saved with the SAME returned original or abandoned; never re-sign |

`lookup` refuses with `NeedsRecovery` until a pending WAL is resolved, so an `Absent` answer
always comes from a recovered, selected head.

## Production save path

The approved boundary is [CHAT-V2-MEMBER-SAVE-HANDOFF](CHAT-V2-MEMBER-SAVE-HANDOFF.md). Root
implements it; the native producer that calls it is F's and does not exist yet.

### Store mount and custody cells

`ServerStore::member_outbox_mount()` installs, once per physical mount, a
`MemberOutboxMount` from the declared namespace5 layout: the mount's registry, the production
parent (`MemberOutboxParentDirectory::from_mount`, held by the mount's original directory
identity after re-proving the whole mount) and a table of per-context cells bounded by
`V2MemberOutboxPolicy::max_contexts`, each paid from the independent member pool. The engine
key is the vault database key, as the Formats section specifies.

A cell lives with the Store, not the host. It holds the idle exclusive owner and at most one
request's custody DATA:

| Custody | Meaning | Next phase-1 call |
|---|---|---|
| `Prepared` | presealed, not admitted (exact original inside) | `Ready`; adopts it into a replacement lane if needed |
| `Running` | a job owns the owner; the exact original stays in the cell | `Busy` |
| `Unresolved { lost }` | uncertain run, unwound job (`lost`), or mismatched de-admission | recover the owner, look up with the original's OWN request: Hit settles it, Absent re-preseals the SAME original; never signs |
| `Saved` | SavedLocal cached by the job before it released anything | tells the lane, reports the receipt |

A cell never holds a save-source end, physical lease or native commit guard; those move with
the job. Phase 1 keeps custody in the cell across every lane and IO call (an original leaves
it only for pure DATA sealing) and recovers a poisoned cell, since every state write is a
single assignment; the final edge refuses a poisoned cell. Every phase-1 shortcut compares
the caller's request digest, so a same-token retry with changed DATA is `Conflict` whether
the request is presealed, cached Saved, Unresolved or durable. Another token's retained
original is reported as `Retained { token }` so a composer can resume it.

Primary M4 decision: retain bounded exact signed-original DATA across a UI lock;
do not discard the only unsaved copy. This does not retain save/publication authority:
a later invocation needs fresh native generation/row and actual source checks.
No UI-lock plaintext-erasure guarantee is made, and memory-only custody is never
reported as SavedLocal.

Fresh authoring rechecks the native negative after namespace recovery/lookup IO;
that negative is not F's actual close/key credential. Before fallible append payment,
the Store cell owns the known original as Unresolved. A retryable preseal refusal
cannot leave it solely in a replaceable host. Once its cell is locked, that call's
bounded context/canonical-parent scratch is funded from a named, checked part of
the existing slot allowance; it drops before unlock. Engine lookup/recovery fees
remain separate and may drain without refunding the scratch's payer early.
Save-source issuance/close stays serialized, while end retirement is atomic and
does not acquire the issuance mutex under native custody.

### Phases

- `PreparedV2TextHost::prepare_member_save` (phase 1, no native UI commit custody): funds
  the lookup, derives the stable context from historical DATA (mounted layout vault and root,
  `V2GroupPinId::of` the exact original pin ciphertext, Profile ID and initial tenure, the
  Profile's `creation` nonce, the full device key, the group), settles any earlier custody
  without signing, runs the exact lookup BEFORE any authoring (Hit is reconciled with the
  lane and reported Saved; Conflict refuses; read failures are errors), then signs through
  the member lane, takes the original and preseals it into the cell. The `prepare_append`
  refusal table above is applied; the original is restored to the lane on every retryable
  refusal.
- `PreparedV2TextHost::admit_member_save` (phase 2, under the native commit guard): no IO,
  no payment wait, no allocation, fail-fast locks only. Requires `Prepared` custody for the
  token and an admissible idle owner, then the lane's save-source end, then the cached mount
  negatives, then the native session's cancellation authority (the linearization point).
  Every refusal returns the native custody whole and keeps the presealed DATA.
- `MemberSaveJob::run` (native-owned `spawn_blocking`): commits, deposits `Saved` or
  `Unresolved` and the owner into the cell, then drops the source end and the native custody
  LAST. Dropping the job unrun de-admits with no IO (the exact presealed append returns to
  the cell; the next attempt needs a fresh final edge). An unwind leaves the original in the
  cell as `Unresolved { lost: true }`.

### Save-source lifecycle

The member lane owns one serialized lifecycle shared by end issuance and close: a closed flag
and monotone issued/completed counters under one mutex (active = issued - completed, at most
one). The lane's `cancel` closes it; its `try_closed` requires the member domain's group close
AND every issued end dropped, so a host close stays pending while an admitted job does IO and
can finish after cancellation. Issuance refuses after close, while the member domain's source
negative is not live, at the bound, or when poisoned. The lifecycle body holds the member
domain's paid negative and its own fee, never a Host, Store or SDK backlink.

### Lane hooks

`canonical_member_parents` is shared with the driver; `admit_save_source` checks the bound
context, the token Lent with exactly the ActionId and the request's historical Admission (the
Plan's, or an adopted original's); `reconcile_saved` retires only the matching token's request
(Lent: reports whether the lent original is the stored one; Pending/Complete: only when the
stored spec is this exact request); `adopt_retained_original` lets a replacement lane hold a
retained original as Lent without a Draft or Plan, so it is saved without signing again.

Residual: an original moved into `prepare_append` would be lost if that pure DATA sealing
panicked; the next call then finds the lane Lent with no custody and retires the token after
an exact Absent lookup instead of signing again.

## Recovery

| Found | Action |
|---|---|
| `head.bin` missing, namespace empty | Initialise generation 0. |
| `head.bin` missing, only a complete authenticated generation-0 head stage | Promote it. |
| `head.bin` missing, only a torn generation-0 head stage | Remove it, initialise. |
| `head.bin` missing, anything else | Preserve, refuse `Incomplete`. |
| No WAL, no stages | Clean. |
| Head stage without WAL | Preserve, refuse. |
| WAL stage only, strict prefix of the exact preamble bound to the selected head, or that preamble with a torn length or body | Owned torn pre-commit scratch: remove it. Nothing it carried was ever reported. |
| WAL stage only, complete, authenticates, parent is the selected head | Promote to MAIN, then roll forward. |
| WAL stage only, anything else (wrong parent, failed MAC, trailing bytes) | Preserve, refuse. |
| WAL MAIN whose parent is the selected head | Re-derive the whole transition, then finish steps 4 to 6. |
| WAL MAIN whose candidate is the selected head | Finish cleanup only, proving each retiree unreachable. |
| WAL MAIN otherwise, or WAL MAIN plus a WAL stage | Preserve, refuse. |

Re-deriving a transition means re-planning it from the selected parent and checking the WAL
against that plan: entry decode and context; chunk assembly with signature, group, device,
length and request digest; every new node plaintext byte for byte; the exact retiree list;
and the candidate head's counters. For a checkpoint, each settled item must be the
contiguous prefix at the parent front, the exact entry its ordinal row names, with a
disposition consistent with its own Admission.

A torn immutable output or head stage is completed only when its entire existing prefix
equals the authenticated expected bytes; mismatched bytes are never truncated or repaired.
An IO error is always a failure, never absence; every authentication or decode failure of
stored state is the preserved `Incomplete` refusal.

## Exclusion and cleanup

- One owner per context per `MemberOutboxRegistry`. The claim is released only when the
  owner is dropped, never on cancellation; an admitted worker owns the owner, so the claim
  covers all of its IO and cleanup.
- Every writing operation (open/recovery, append, checkpoint) holds a
  `VaultPhysicalWorkLease` for its whole IO span, so a replacement Store cannot mount while
  that work drains. An idle owner holds no lease. The lease is OS exclusion, not authority.
- Even if two owners existed (two registries, as in the stale-owner test), step 1 and the
  CAS re-check refuse a commit over a head the owner did not select.
- Readers borrow the owner and transactions move it, so no reader can hold an old root
  while a transaction retires its nodes. A replaced node is deleted only after the CAS; in
  cold cleanup each still-present retiree is first proven unreachable from the new root at
  its own encoded position.

## Publication

`debt_batch(current_admission)` returns up to eight unsent originals in ordinal order with
their exact bytes. An item signed under the presented current Admission is `Publishable`;
one signed under any other Admission is `Held` and can never be marked submitted. The
publisher reseals each Publishable original under the current envelope (persisting mutated
MLS sender state before export, which is the transport integration's responsibility), then
calls `record_submitted`. A checkpoint settles only the contiguous prefix of Held or
submitted items, so a failed earlier item is never skipped, and records a durable
`Submitted` or `Held` disposition row for each. A batch is bound to its context, its front
and its exact entries: a stale batch (front moved) or a foreign one (other context) is
refused. A lost checkpoint reply may cause the same signed original to be submitted again;
local submission is not peer receipt, durability confirmation or certification.

After removal and rejoin under A2, A1's receipts stay readable (attributed to A1), A1's
unsent entries are settled as `Held` (kept in the audit for the separately authorised
moderation path), and A2 cannot take over A1's tokens.

## Pressure and payments

`MemberOutboxQuota` bounds unsent debt, live object bytes and live objects. The byte and
object checks include the crash overlap: old and new nodes plus the WAL and head stage
coexist until cleanup. The selected head excludes replaced nodes before unlink,
but a clean owner and new admissions remain unavailable until the listed deletions
and barriers actually finish. Exact token receipts and
conflicts are resolved before pending-count pressure, including when the queue is full.
Pressure refuses with `Pressure(Pending | Bytes | Objects)` before admission;
nothing is evicted.

Append admission additionally commits enough logical quota to checkpoint all pending
debt. With selected live bytes L and P pending entries it requires
`L + P * MAX_NODE_OUTPUT_BYTES + WAL_FILE_CAP + HEAD_FILE_CAP` within the byte
quota, and the corresponding live-object count plus
`P * MAX_NODE_OUTPUTS + 2` within the object quota. Arithmetic exhaustion refuses.
Every checkpoint settles at least one entry and adds at most those node caps, so
the commitment cannot grow under an unchanged quota. The actual old/new operation
peak check still applies. This conservative commitment can reduce the attainable
pending count; it is not disk preallocation or protection against external ENOSPC
or finite trie-geometry exhaustion.

Lowering a quota preserves receipts and debt but may refuse checkpoints as well
as new appends. A future user-facing quota setting must reject a reduction below
existing commitments if it promises to retain automatic publication progress.

Every operation takes a prepaid `CatalogReadReservation` from the owner's
`CriticalCatalogReadPool` and validates it before allocating: owner metadata 16 KiB
(retained for the owner's life), open/recovery 6 MiB, append 6 MiB, lookup 2 MiB, debt batch
5 MiB, checkpoint 4 MiB. These are logical invoices derived from the module's hard caps,
not measured allocator/RSS figures. Values that retain buffers (prepared work, lookup hits,
debt batches) keep their payment until destroyed; the physical lease is separate.

## Tests

`cargo test -p catcoms-app --lib v2_member_outbox` covers: the private-page overhead pin;
head context binding and stage classification; save/lookup/conflict (text, parents,
channel); lost reply and identical re-signed retry; duplicate ActionId; foreign signer;
admission fact binding; crashes at all nine append boundaries and six checkpoint boundaries
with a full file-set and counter audit; a crash during roll-forward; torn WAL stages at
seven lengths with re-save of the returned original; corrupt, stale and foreign stages, WAL
and head preserved byte for byte; torn chunk and head stage completion and mismatch
preservation; exact and mismatched pre-existing outputs; interrupted creation and a
populated namespace with a lost head; A1/A2 rejoin and tenure isolation; cancellation after
admission with registry and vault-lease exclusion; Busy and stale-owner commits; pending,
byte and object pressure; payment pool and size checks; index exhaustion; splits with a
reachability audit; and identical original bytes across simulated epochs, prefix-only
settlement, stale and foreign batches, and audit after debt clears.

## Remaining integration gaps

The primary's approved A/B/F implementation boundary is recorded in
[CHAT-V2-MEMBER-SAVE-HANDOFF](CHAT-V2-MEMBER-SAVE-HANDOFF.md). Root now implements the save
path above; all 18 focused Windows save cases now pass in the independent CI lane,
alongside 57 storage/catalog executions on both Linux and Windows. Strict/full/native
gates remain incomplete. [Current evidence](../HANDOVER.md).

- The native producer (F): the checked factory that builds `MemberSaveNativeAdmission`
  from the actual owned UI commit guard and row custody, the per-server-incarnation entry,
  lock order, `spawn_blocking` dispatch and the native barrier tests. Root fixtures use a
  test custody box and establish no native authorization.
- Request persistence above the host: lookup recovers a save only from the SAME token,
  channel, body and exact canonical parent list. Parents are the request's explicit list
  (never recomputed from tips), so the request composer must persist the exact parent list it
  sent; a retry after restart with different parents is a conflict by design, not a hit.
- True host replacement on one Store (a second whole original owner) is not exercised; the
  regressions replace the member lane under the same owner. The cell is memory and the
  engine's restart-equivalent cases resolve lookup from the durable outbox; they do not
  establish whole-Host reopen. The actual cold-genesis factory rejects a promoted
  operational head. A distinct cold-operational factory and genuine native bootstrap
  are still required. [Native prerequisites](CHAT-V2-NATIVE-MEMBER-SAVE-NEXT.md).
- Nonfounder sends and the joined-member source (C), including enforcing the original
  Admission's actual current lifetime at `admit_save_source`.
- Member saves on a governance4 vault: the governance wrapper yields no Store or host, so
  only the namespace5 declaration and mount work there today.
- Pool sizing and the independent pool credits; these invoices are not measured fits.
- Transport: resealing under the current envelope, persisting sender state before export,
  and calling `record_submitted` only after transport accepts an envelope. Checkpoints are
  not yet routed through the cell (a checkpoint must take the idle owner the same way).
- Removing the module's dead-code allowance once transport consumes the publication half.
- A durable per-attempt epoch record is not kept; add one only if diagnostics need it.
- Two-device or native acceptance of the outbox is not covered by these storage tests.
