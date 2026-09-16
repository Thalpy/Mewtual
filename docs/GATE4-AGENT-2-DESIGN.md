# Gate 4 Agent 2: overlay lifecycle, provisional local work and repeated tenure

Status: **revision 2, design proposal, awaiting re-review. No production code is written, no test
has been added and no Cargo command was executed.**

Revision 1 (`a901f6b0f64df2b4ea9cc0221b64ac98276f582d`) received CHANGES REQUIRED on all three
boundaries, with nine findings. This revision answers every one of them. Section 0 maps each
finding to its correction; verify each against the code, not the prose.

Design base for this revision: `a901f6b0f64df2b4ea9cc0221b64ac98276f582d`. Original design base:
`1bcb1bca204d721b848b17c0835faf931ae930e3`. Scope is
[Agent 2 of the four handoffs](GATE4-AGENT-HANDOFFS.md); progress is in
[GATE4-AGENT-2-STATUS](GATE4-AGENT-2-STATUS.md). The applicable review scope is
[review preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

Three separable review boundaries, unchanged from revision 1:

1. the manual lifecycle (inspect, export, copy, disposition), stale bases and repeated tenure;
2. the **separately reviewed** extension for durable local work based only on an
   `AwaitingTenureReceipt` preview (section 8);
3. the **correction to locally observed owner tenure** (section 9.3), which changes an
   authority-bearing observation in `catcoms-sync` and now also touches `catcoms-mls`.

Accepted work this design must not weaken: the Closing-overlay foundation (`b1b0ec9`,
OVERLAY-TEST-001 closed), the handoff design (HANDOFF-001) and its bounded core/store
implementation (`62f06d4`, HANDOFF-002 closed), the detached-inspection proposal (`0b28f06`) and
its read-only implementation (`c47ae0b`, INSPECTION-TEST-001 closed), the combined scheduling
block (`6b71d96`), and the accepted provisional read-only preview contract (`a89bde6`/`134394e`,
NATIVE-TEST-001 and TAIL-TEST-001 closed). No closure is reopened.

**Unmet dependencies.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at `e65bfd8` is
unreviewed. Agent 1's runtime design is unaccepted; this design consumes its seams by name only and
section 14 records what breaks if they change. Agent 3's design is separate and is not consumed
here except through the tenure seam of section 9.4.

## 0. Disposition of the revision-1 findings

| Finding | Disposition in revision 2 | Where |
|---|---|---|
| **1 (High)** `Copied` disposal has no valid terminal representation: `copy` is required by the mode, forbidden without `active`, and cleared by the transition | **Corrected by removing the cause.** Copy bookkeeping is deleted from the durable record entirely. Disposal no longer depends on copy. The terminal `disposed` manifest is self-contained and independently valid with `active` absent, and positive encode/decode/reopen cases exist for both modes. | 5.1, 6.4, 6.5, 17.1 N11-N13, N23 |
| **2 (High)** the copy proof can account for the wrong source work; projection-level copying is not envelope-level preservation | **Corrected twice.** (a) Copy is no longer a disposal precondition, so no destructive action rests on it. (b) `source_entry` is deleted from the request: `restore::plan` now *derives* the source operation ids it consumes, and the design states plainly that copy is projection-level and lossy for superseded operations, conflicts and original provenance. Preservation is supplied by the archive, not by copy. | 6.3 C-P, 6.5, 17.1 N6-N10 |
| **3 (High)** repeated disposal erases the only defence against an old Save retry | **Corrected by a durable branch-generation namespace.** A monotonic `branch_generation` is bound into the branch identifier every request must carry, so an old request matches nothing and returns **Stale**, never a new acceptance. One retained manifest still gives a terminal acknowledgement for the most recent disposal; forgetting older ones degrades to refusal, never to acceptance. | 5.1, 6.6, 17.1 N17, N17b, M10b |
| **4 (Medium)** the unchanged inspection capture cannot supply the destination inputs copy needs | **Corrected.** A composite capture taken in the same custody visit under the same single permit: the existing source capture plus the destination's authenticated Studio source and recovery record plaintexts with their digest and physical size, rechecked at preview completion and at application. The "only the rebuild function changes" claim is withdrawn. | 5.2, 6.3 C1-C4, 17.1 N8b |
| **5 (Medium)** a structurally valid, non-replayable branch has no lossless export path | **Corrected, and it simplified the archive.** Export and the archive are built from the **structural** record plus the ledger envelopes and do not require typed reconstruction. A reconstruction failure is labelled, not fatal, and `Preserved` disposal therefore works for a non-replayable branch. | 5.3, 6.2, 6.4, 7 S3, 17.1 N22 |
| **6 (Medium)** the preview mint has no way to obtain its required seed-only bytes | **Corrected.** `UnconfirmedStudioSeed` retains the exact verified checkpoint bytes it parsed, exposed only through the existing scoped callback, with explicit memory accounting. The mint does not trust that retention: the detached stage **re-parses** those bytes against the candidate receipt before the branch is built. | 8.1, 8.2, 8.3, 17.1 N25b |
| **7 (High)** L5's safety argument overlooks Unknown-tenure readers | **Corrected; the false argument is withdrawn.** `complete_checkpoint_head_scoped`'s `is_some_and` means an Unknown-tenure reader accepts a proof's claimed tenure, so disagreement is not self-correcting. Agreement is now established structurally: `Position` gains the committer's leaf identity so every witness recognises the same authenticated membership discontinuity, plus a membership rule for the one residual case. The inference is rephrased in terms of current continuous membership. | 9.3, 17.1 N-T6, N-T6b, M22b |
| **8 (Medium)** the native storage-refusal rule is false after a partially completed copy | **Corrected.** C5 is deleted with the copy bookkeeping, so the partial-copy state no longer exists. The three-state write outcome is specified for the write sequence that does exist: archive durable with disposal pending. | 6.4, 11, 12 |
| **9 (Medium)** explicit discard confirmation is required but absent from the schema | **Corrected.** An exact required confirmation token in both the Rust request and the native argument list, with a typed constructor that only that literal produces. | 5.1, 5.5, 6.4 D5 |
| Reviewer's N19 precision point: destination operations do not retain the branch's base-only, superseded and removed references | **Accepted and corrected.** N19 is rewritten. After `Preserved` disposal the **archive's** reference collection retains them; after `Discarded` they become reclaimable, which is what the user confirmed. | 6.4, 10 R3, 17.1 N19 |
| Reviewer's answers to questions 16.1-16.6 | Adopted, including 16.1's ruling that a newly authored projection edit is not a lossless substitute, and 16.2's preference for a v3 terminal record with the archive's durability established first. | 6.4, 16 |

## 1. Outcome and boundary

Every retained local branch has a bounded, authorized way to be inspected, exported, copied into
current authorized work, or explicitly disposed of, and none of those is available by accident.
Repeated owner changes and newcomers preserve the difference between unconfirmed history, local
work and verified authority, and a fail-closed Unknown tenure has an implemented, legitimate way to
become Known on which every participant agrees.

In scope: the four manual operations and their durable records; the draft archive; stale, rewound
and nonpristine bases; preview-based local work; A -> B -> A succession and rejoining; the
live-tenure contract consumed by Agents 1 and 3; truthful native results, events and UI-hooks rows.

Out of scope: actor-scheduled Save and automatic handoff runtime, preparation permits, the
inventory cursor and transient reference holds (Agent 1); signed fault repair (Agent 3);
integration, shared-document edits and full-gate acceptance (Agent 4). **Nothing here registers
`studio_overlay_save`**; section 13 states exactly which of Agent 1's registration prerequisites
this design satisfies, and the status note carries the authoritative answer.

There is **no import path**. Neither the export nor the archive can re-enter a vault as authority.

## 2. What was audited

Revision 1's audit table stands. It is not repeated in full. Facts added or corrected for this
revision, each read at the pinned source:

- **A1.** `ServerStore::read_studio_record(&scope)` and `read_epoch_studio_plain` return
  `AuthenticatedEpochFileBytes { plain, physical_bytes }` for a Studio source record under
  `MAX_SEALED_BYTES`, with the parent-directory and regular-file restrictions already applied
  ([epoch_studio.rs:648-700](../crates/catcoms-app/src/store/epoch_studio.rs#L648)). The recovery
  family has the equivalent reader. **Finding 4's composite capture needs no new reader**, only
  wider visibility and a second stamp.
- **A2.** `UnconfirmedStudioSeed::parse` ends by requiring
  `projection.checkpoint(receipt.close_record_hash)?.bytes() == bytes`
  ([provisional.rs:41-44](../crates/catcoms-replication/src/studio/provisional.rs#L41)), so the
  canonical seed is byte-identical to what was fetched **at parse time**. It does **not** retain
  those bytes, and the tail subsequently advances `projection`, so the identity cannot be
  recomputed later. `ProvisionalStudioSeedPreparation` owns `raw` and drops it in `prepare()`.
  This confirms finding 6 and fixes its shape: retain the bytes, and re-prove the binding rather
  than trusting the retention.
- **A3.** `openmls::group::Member` exposes `index`, `credential`, `encryption_key` and
  `signature_key`. `ServerGroup` currently surfaces only `index` and `signature_key`
  ([group.rs:138-171](../crates/catcoms-mls/src/group.rs#L138)). A `DeviceId` is derived from the
  signature key, so a changed signature key is already an owner change; the **credential** is the
  field that distinguishes a re-add from an ordinary self-update, because
  [group.rs:247-274](../crates/catcoms-mls/src/group.rs#L247) binds a joiner's KeyPackage
  credential to `(this group, invite_nonce)` while an update rotates keys and not the credential.
  This is the discriminator section 9.3 uses.
- **A4.** `complete_checkpoint_head_scoped` rejects a proof/observation tenure mismatch only under
  `observed_owner_tenure_start().is_some_and(|t| t != proof.tenure_start_group_epoch)`
  ([detached.rs:172-180](../crates/catcoms-sync/src/receipt_head/detached.rs#L172)). With local
  observation `None` a valid fresh proof mints a `HeadSelection` carrying the proof's claimed
  tenure. The reviewer is right: this is deliberate accepted reader behaviour and it means a wrong
  value is **not** universally refused. Revision 1's safety argument is withdrawn.
- **A5.** `StudioRecoveryItem`'s `value` field already names the source operation id of the value
  the planner selects (`frame_value`/`index_value` match `source.op_id`), and the deletion and
  creation arms take `tombstones[id].first()` and `creations.first()`
  ([restore.rs:72-83, 198-202, 264-267](../crates/catcoms-app/src/studio/restore.rs#L72)). The
  planner therefore **can** report the exact source operation ids it consumed, which is finding 2's
  correction (a).

## 3. Audit observations

Revision 1's O1 (the foundation's hold is a removal filter, not a lifecycle), O3 (`StudioRecovery`
is the wrong container), O4 (`restore::plan` is the copy engine), O5 (a preview's durable content is
content-addressed) and O8 (classification needs no reconstruction) stand unchanged.

**O2 is corrected.** Revision 1 concluded from the 64 KiB metadata ceiling that no archive was
possible and therefore that a verified copy had to serve as preservation. The reviewer's 16.1
rejects the conclusion, and it was wrong for a second reason the reviewer did not need to raise:
the constraint rules out putting bodies **in the extension**, not out of an archive as such. A
bounded archive can live as a second record **inside the existing Intents family**, under the same
directory, sealing, budget, generation, vault ceiling and inventory arm. That is not a sixth
inventoried family; it is one more record kind in a family the scan already walks. Section 6.5
specifies its schema, provenance, accounting, reference collection, ordering and quotas, which is
what 16.1 requires of any archive representation.

**O9 (new).** Copy and preservation are different jobs and revision 1 conflated them. `restore::plan`
is a projection planner: it recovers the *selected value* of an element, so a branch entry that was
superseded within the branch, a conflict, and the original authorship of an accepted envelope have
no representation in what it produces. Copy is therefore a genuinely useful way to carry work
forward and a genuinely invalid proof that work was preserved. Revision 2 keeps the first role and
deletes the second.

**O10 (new).** The rollover defence that `completed` provides comes from advancing
`minimum_new_basis_closed_epoch`, which disposal deliberately does not do (finding 3). The cheap
replacement is not a history of acknowledgements but a **namespace**: if every request carries a
branch identifier that includes a monotonic generation, an old request cannot collide with a new
branch at all, and the worst outcome of forgetting an old manifest is a refusal.

## 4. Design principles

1. **Reuse the accepted read machinery.** Export, archiving and copy planning use the existing
   inspection capture, permit, stamp, currency contract and delivery fence. Where copy needs more
   than that capture holds, the capture is **extended explicitly** (5.2), not claimed to be
   unchanged.
2. **Read-only operations change no durable byte.**
3. **Removal is a transition with durable evidence.** Work leaves the ledger only through one
   explicit, authorized, exactly retryable transaction that has either established lossless
   preservation first or recorded that the user waived it.
4. **Preservation is lossless or it is not preservation.** A newly authored projection edit carries
   work forward; it does not preserve envelopes, ordering, conflicts or provenance.
5. **Copy authors new work** and is never a precondition for destroying anything.
6. **Provenance is carried, never inferred.**
7. **Expiry governs the preview, not the work.**
8. **Tenure is observed or Unknown**, and where an inference is made, every participant must be able
   to make the same one from the same authenticated evidence.
9. **Identity is generational.** An old request must be distinguishable from new work by
   construction, not by retained history.
10. **Refusal retains work**, and every refusal names an actionable state.

## 5. Concrete APIs

New leaf modules owned by Agent 2:

```
crates/catcoms-replication/src/studio/overlay/disposal.rs      (v3 arm, terminal manifest)
crates/catcoms-app/src/store/epoch_intents/disposal.rs         (the disposal transaction)
crates/catcoms-app/src/store/epoch_intents/archive.rs          (the draft archive record)
crates/catcoms-app/src/studio/lifecycle.rs                     (structural classification)
crates/catcoms-app/src/studio/overlay/copy.rs                  (composite capture, copy driver)
apps/desktop/src-tauri/src/studio/overlay.rs                   (native surface)
```

### 5.1 Core: provenance, generation, and the v3 terminal record

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StudioOverlayProvenance { Closing, Unconfirmed }

impl StudioOverlayState {
    pub fn provenance(&self) -> Option<StudioOverlayProvenance>;
    /// Monotonic per logical document. Incremented exactly once when a branch is first
    /// accepted where none existed. Never reset, never reused, never decremented.
    pub fn branch_generation(&self) -> u64;
    /// The identity every request must carry: H("catcoms/studio-overlay-branch/v1",
    /// basis fingerprint, branch_generation). An identifier, never authority.
    pub fn branch_id(&self) -> Option<[u8; 32]>;
    pub fn disposed(&self) -> Option<&StudioOverlayDisposal>;

    /// Classification for an incoming Save request, evaluated before any basis mint, tenure
    /// read, source lookup or media admission. Exactly one arm can match.
    pub fn classify_request(&self, target: StudioTarget, branch: [u8; 32], intent: &LocalIntent)
        -> Result<StudioOverlayRequestClass, ReplError>;

    /// The one transition that drops an active branch without transferring it. The caller has
    /// already proved authorization, archive durability (for `Preserved`) and explicit user
    /// confirmation (for `Discarded`); this rebuilds and validates state only.
    pub fn dispose(&self, ledger: &IntentLedger, decision: StudioDisposalDecision,
                   sequence: u64, at: u64) -> Result<(Self, BTreeSet<[u8; 32]>), ReplError>;
}

pub enum StudioOverlayRequestClass {
    /// `branch` names the live branch; ordinary exact-retry or append applies.
    Active,
    /// `branch` names the retained `completed` manifest with a matching envelope.
    Transferred(StudioHandoffOutcome),
    /// `branch` names the retained `disposed` manifest with a matching envelope.
    Disposed(StudioOverlayDisposal),
    /// `branch` names nothing this record knows. NEVER a new acceptance. A genuinely new
    /// branch carries the identifier minted now by `studio_overlay_begin` (6.6).
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioDisposalMode {
    /// A durable draft archive for this exact branch content existed before the removal.
    Preserved { archive: [u8; 32] },   // the archive record's content digest
    /// The user explicitly destroyed the bodies after observing the branch.
    Discarded,
}

/// Terminal and SELF-CONTAINED. Valid with `active`, `prepared` and `copy` all absent, because
/// no field of it refers to them (finding 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioOverlayDisposal {
    pub target: StudioTarget, pub author: DeviceId,
    pub provenance: StudioOverlayProvenance,
    pub basis: [u8; 32], pub branch: [u8; 32], pub content: [u8; 32],
    pub generation: u64, pub mode: StudioDisposalMode,
    pub accepted: usize, pub sequence: u64, pub at: u64,
    /* private: Vec<Entry> (id, envelope, sequence, ts), <= MAX_STUDIO_OVERLAY_OPS */
}

/// Only this literal constructs it; no bool and no defaulted field can stand in (finding 9).
pub struct StudioDiscardConfirmation(());
impl StudioDiscardConfirmation {
    pub const TOKEN: &'static str = "destroy-local-draft";
    pub fn parse(value: &str) -> Option<Self>;
}
```

`StudioCopyProgress` and the `copy` record arm of revision 1 **do not exist**. Copy writes nothing
to the branch's record (findings 1, 2, 8).

**Encoding rules**, following the accepted v1/v2 discipline:

1. A state expressible as v1 still encodes as v1 byte-for-byte (the existing `legacy` rule).
2. A state expressible as v2, meaning `provenance == Closing`, `generation == 1`, no `disposed`,
   still encodes as **v2 byte-for-byte**. Existing records are never rewritten, so Agent 1's C-2
   digest fence and the accepted canonical re-encode equality are unaffected for every current
   vault. A v1 or v2 record decodes with `generation = 1`.
3. Tag `3` is emitted only when `provenance == Unconfirmed`, `generation != 1`, or
   `disposed.is_some()`. Its layout is the v2 layout followed by: `generation`, a provenance byte
   (0 `Closing`, 1 `Unconfirmed` with `provider`, `observed_mls_epoch`, `observed_at_ms`), and an
   optional `disposed` block.
4. A reader accepts exactly one complete v1, v2 or v3 form. Unknown version bytes, trailing bytes,
   duplicate ids, noncanonical order and a wrong `sequence` reject, and the existing canonical
   re-encode equality check runs unchanged. Old readers already fail closed on tag 3.
5. For `Unconfirmed` the nested v1 basis blob is untouched: `source_id` and `source_version` must
   be canonically zero and the provenance fields live in the v3 outer record.

**Validation additions to `StudioOverlayState::validate`**, inside the existing method so the
decoder and `encode_vault` share one predicate:

- no operation id occurs in both `completed.entries` and `disposed.entries`, and none of either
  occurs in `active`;
- `disposed.accepted == disposed.entries.len()`, `validate_manifest(disposed.entries)` passes, and
  a ledger entry still present for a disposed id must match its author and envelope (the tolerance
  `completed` already has);
- `disposed.generation <= branch_generation`, and `branch_generation >= 1`;
- **no rule of `disposed` refers to `active`, `prepared` or any live field**, so a terminal record
  with `active == None` is valid, re-encodes canonically and reopens (finding 1);
- an `active` branch may coexist with a `disposed` manifest, in which case
  `active.generation > disposed.generation` is implied by `branch_id` construction and asserted;
- `provenance == Unconfirmed` forbids `prepared` and a nonzero nested `source_id`/`source_version`;
- the combined metadata ceiling is charged as today. With `copy` gone, two 256-entry manifests plus
  headers occupy approximately 43 KiB of the 64 KiB budget; the encoder refuses over it and the
  branch stays retained (limit L3).

**The provenance guard.** `prepare_handoff`, `prepare_handoff_detached` and `prepared_manifest`
refuse unless `provenance == Closing`, before any authority work. This single core fence makes an
unconfirmed branch structurally incapable of becoming signed history.

### 5.2 Store: the composite capture (finding 4)

Revision 1's claim that only the rebuild function changes is withdrawn. The source capture holds
one record's plaintext and stamp and nothing about a destination, so copy needs a second bounded
capture taken in the **same custody visit under the same single preparation permit**:

```rust
pub(crate) struct StudioOverlayCopyCapture {
    /// The accepted capture, unchanged: one bounded authenticated intent record plus
    /// `StudioInspectionStamp` (mount, server, document, target, author, digest, physical size).
    source: StudioInspectionCapture,
    destination: StudioDestinationCapture,
}

/// Authenticated bytes and their currency stamp. No projection is materialized under custody;
/// both projections are built on the detached worker, preserving "custody is spent on evidence".
pub(crate) struct StudioDestinationCapture {
    stamp: StudioDestinationStamp,
    source: Zeroizing<Vec<u8>>,            // read_studio_record, <= MAX_SEALED_BYTES (A1)
    recovery: Option<Zeroizing<Vec<u8>>>,  // <= MAX_RECOVERY_SLOTS_BYTES + 1024
}
pub(crate) struct StudioDestinationStamp {
    mount: Arc<()>, server: u64, document: LogicalDocument, target: StudioTarget,
    /// (blake3 of authenticated plaintext, physical bytes) for each record, the same currency
    /// contract `studio_inspection_is_current` already uses.
    source: (blake3::Hash, u64),
    recovery: Option<(blake3::Hash, u64)>,
}

impl ServerStore {
    /// Caller holds live membership custody and one shared preparation permit. Two bounded
    /// authenticated reads. Decodes nothing.
    pub(crate) fn capture_studio_overlay_copy(
        &self, server: u64, group: &[u8], source: StudioTarget, destination: StudioTarget,
        author: DeviceId,
    ) -> Result<StudioOverlayCopyCapture, AppError>;

    /// Re-reads BOTH destination records and compares digest and physical size, in addition to
    /// the existing `studio_inspection_is_current` check on the source record.
    pub(crate) fn studio_destination_is_current(
        &self, server: u64, group: &[u8], destination: StudioTarget,
        stamp: &StudioDestinationStamp,
    ) -> Result<bool, AppError>;
}
```

Detached work decodes both destination records, builds the destination projection and its at most
three recovery projections, reconstructs the draft, and runs `restore::plan`. The destination's
`doc_id` and `EpochPhase::Open` are asserted on the decoded destination record, and again at C3 and
C4 against freshly read bytes.

### 5.3 Store: export, the raw-evidence fallback and the archive payload

Export and the archive share one serializer and **do not require typed reconstruction**
(finding 5): they are built from the structurally decoded extension plus the ledger envelopes.

```rust
pub(crate) enum StudioInspectionPurpose {
    /// Existing behaviour: full reconstruction and a `StudioLocalDraft` projection.
    Draft,
    /// Structural decode plus ledger envelopes. Typed reconstruction is ATTEMPTED and its
    /// outcome labelled, never required.
    Archive,
    CopyPlan(Box<StudioOverlayCopyChoice>),
}
impl StudioInspectionCapture {
    pub(crate) fn rebuild_for(self, purpose: StudioInspectionPurpose)
        -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError>;
}
pub(crate) struct StudioInspectedDraft {
    pub(crate) target: StudioTarget,
    pub(crate) prepared: bool,
    pub(crate) draft: Option<StudioLocalDraft>,
    pub(crate) archive: Option<StudioDraftArchivePayload>,
    pub(crate) plan: Option<StudioOverlayCopyPlan>,
    pub(crate) branch: Option<[u8; 32]>,
    pub(crate) content: Option<[u8; 32]>,
    pub(crate) provenance: Option<StudioOverlayProvenance>,
    /// `Ok(())` when typed reconstruction succeeded, `Err(reason)` when it did not. An Err
    /// value still yields a complete archive and a complete export (finding 5).
    pub(crate) replayable: Result<(), String>,
}
```

**`catcoms-studio-draft-v1` payload**, the single format used by export and by the archive record:

```
u8   version = 1
u8   provenance (0 Closing, 1 Unconfirmed)
u8   replayable (0 no, 1 yes)
bytes document.server_id, u16 doc_type tag, bytes document.logical_key
u8   target kind, bytes channel, [bytes object]
bytes author(32), bytes basis(32), bytes branch(32), bytes content(32)
u64  branch_generation
bytes receipt (canonical Receipt::encode)
bytes seed
[provenance == 1: bytes provider(32), u64 observed_mls_epoch, u64 observed_at_ms]
u32  count
     per entry, in saved sequence: bytes id(32), u64 sequence, u64 ts,
                bytes author(32), bytes operation (DomainOp::encode)
```

Bounded by construction: every field comes from the already bounded record, so the payload is at
most `MAX_RECORD_BYTES` plus framing. It carries no signature and no key material, and there is no
decoder in the vault direction: it cannot produce a `StudioClosingOverlayBasis`, a `StudioOverlay`,
a `VerifiedReceipt` or any store record. `replayable == 0` is the raw-evidence case finding 5
requires: exact seed, ordered complete envelopes, timestamps, scope and provenance, with the
reconstruction failure labelled rather than concealed.

### 5.4 Store: the draft archive record and the disposal transaction

```rust
impl ServerStore {
    /// One archive record per (server, logical document), written to a sibling path in the SAME
    /// Intents directory under a distinct sealing domain. Accounted in `EpochIntentBudget`
    /// against `MAX_VAULT_INTENT_BYTES` and the archive ceiling of 6.5. Takes Agent 1's
    /// `epoch_mutation_guard` like every other five-family writer.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn write_studio_draft_archive_with_io(
        &mut self, server: u64, document: &LogicalDocument, payload: &StudioDraftArchivePayload,
        ts: u64, rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget, intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioDraftArchiveRecord, AppError>;

    pub(crate) fn read_studio_draft_archive(
        &self, server: u64, document: &LogicalDocument,
    ) -> Result<Option<StudioDraftArchiveRecord>, AppError>;

    /// Explicit, separately confirmed, and itself truthfully destructive.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn release_studio_draft_archive_with_io(/* .. */)
        -> Result<(), AppError>;

    /// ONE accounted atomic replacement of the intent record: the terminal manifest is written
    /// in the SAME sealed plaintext that removes the named annotated entries. Retires no
    /// ordinary intent, prunes no source, deletes no blob, writes no recovery record, and for
    /// `Preserved` performs no archive write of its own: the archive is already durable.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn dispose_studio_overlay_with_io(
        &mut self, server: u64, document: &LogicalDocument, target: StudioTarget,
        group: &ServerGroup, device: &MlsDevice, request: &StudioOverlayDisposalRequest,
        ts: u64, rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget, intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioOverlayDisposal, AppError>;
}

pub struct StudioOverlayDisposalRequest {
    /// Branch identity including its generation (finding 3).
    pub branch: [u8; 32],
    /// `branch_hash(active, ledger)` from the inspection the user actually saw.
    pub content: [u8; 32],
    pub accepted: usize,
    pub mode: StudioDisposalRequestMode,
}
pub enum StudioDisposalRequestMode {
    Preserve,
    /// Finding 9: the confirmation is a required field with a typed constructor.
    Discard(StudioDiscardConfirmation),
}
```

The lifecycle classification is unchanged from revision 1 apart from carrying `generation`,
`content` and the disposal mode:

```rust
pub enum StudioOverlayLifecycle {
    Absent,
    Draft { provenance: StudioOverlayProvenance, transfer: StudioOverlayTransfer,
            eligibility: StudioOverlayEligibility, basis: [u8; 32], branch: [u8; 32],
            content: [u8; 32], generation: u64, accepted: usize, archived: bool },
    Transferred(StudioHandoffOutcome),
    Disposed(StudioOverlayDisposal),
}
```

### 5.5 App and native surface

```rust
pub enum StudioControlAction {
    // ... existing ...
    OverlayLifecycle,
    ExportOverlay,
    FinishOverlayExport(Box<StudioPreparedInspection>),
    /// Produces the archive payload and writes the durable archive record.
    ArchiveOverlay,
    FinishOverlayArchive(Box<StudioPreparedInspection>),
    ReleaseOverlayArchive(Box<StudioArchiveReleaseRequest>),
    PrepareOverlayCopy(Box<StudioOverlayCopyChoice>),
    FinishOverlayCopyPreview(Box<StudioPreparedCopy>),
    ApplyOverlayCopy(Box<StudioOverlayCopyApply>),
    DisposeOverlay(Box<StudioOverlayDisposalRequest>),
}

pub struct StudioOverlayCopyChoice {
    pub destination: StudioTarget, pub item: StudioRecoveryItem, pub mode: StudioRecoveryMode,
}
/// `source_entry` of revision 1 is DELETED (finding 2). The plan reports what it consumed.
pub struct StudioOverlayCopyApply {
    pub destination: StudioTarget, pub item: StudioRecoveryItem, pub mode: StudioRecoveryMode,
    pub branch: [u8; 32], pub content: [u8; 32],
    pub epoch_id: u128, pub expected_projection: [u8; 32],
    pub nonce: [u8; 16], pub body: Vec<u8>,
}
```

Native commands registered by this design, none of which enables Save:

```ts
studio_overlay_read({ server, channel, object? })                  // extended, section 11
studio_overlay_lifecycle({ server, channel, object? })
studio_overlay_export({ server, channel, object? })
studio_overlay_archive({ server, channel, object? })
studio_overlay_archive_read({ server, channel, object? })
studio_overlay_archive_release({ server, channel, object?, archive, confirm })
studio_overlay_copy_preview({ server, channel, object?, destination, choice, mode })
studio_overlay_copy_apply({ server, channel, object?, destination, edit })
studio_overlay_dispose({ server, channel, object?, branch, content, accepted, mode, confirm? })
```

`confirm` is required exactly when `mode === "discard"` and must be the literal
`"destroy-local-draft"`; any other value, and its absence, refuse before any durable work
(finding 9). `studio_overlay_archive_release` takes the literal `"release-local-archive"`.

All use the existing `InvokeContext`: one UI session generation, one actor instance, one native
operation slot, one `ViewRequest` per `(state, server, target)` and one `RequestCancellation`
spanning every custody visit, exactly as `studio_overlay_read` does today. The two-visit commands
keep the existing 5-second `StudioInspectionDelivery` fence.

## 6. The manual operations

### 6.1 Inspect: unchanged

`studio_overlay_read` keeps its accepted contract; section 11 adds fields.

### 6.2 Export

| # | Stage | Custody | Work |
|---|---|---|---|
| X1 | Begin | yes | `begin_studio_inspection`, unchanged: channel known, current membership, `(group, device, owner, mls)`, one shared permit, one bounded authenticated read, stamp. |
| X2 | Rebuild | detached | `rebuild_for(Archive)`: **structural** decode, target and author checks, the section 5.3 payload from the structural entries and the ledger envelopes. Typed reconstruction is attempted and its outcome recorded in `replayable`; failure does not abort (finding 5). |
| X3 | Finish | yes | `finish_studio_inspection`'s existing rechecks, including the store stamp digest and physical size. |
| X4 | Deliver | native | Existing delivery guard, `bounded_view`, base64. |

- **E1.** Export writes no durable byte and authorizes nothing. Permitted while a transfer hold
  exists; refused, retryably, while a live hold exists.
- **E2.** Current membership is required through the unchanged `inspection_context`. A removed
  member acquires no offline export right.
- **E3.** Export is not `.pixa`, not settlement and not permission to delete the original. Disposal
  has its own request, fences and confirmation.
- **E4.** The payload names referenced PIX CIDs; it contains no pixel bytes and promotes, holds or
  fetches nothing.

### 6.3 Copy into current

Two-phase, mirroring the accepted recovery `Preview` -> `Apply` shape. Copy carries work forward and
is **never** a precondition for disposal (findings 1, 2).

**Destination scope**, unchanged from revision 1 and endorsed by the reviewer's 16.6:

- **Same-document copy** (primary): the branch's own logical document, now Open.
- **Cross-document copy** (the Prepared case): a different logical document of the same `doc_type`
  in the same channel, in practice Flipnote to Flipnote. Index is same-document only.
- **C-0.** The destination's identity is its complete `LogicalDocument`. A different channel label
  naming the same Flipnote object is the same destination and is refused while a transfer hold
  exists.

**Planner changes.**

```rust
pub(crate) fn plan(
    current: &StudioProjection, historical: &StudioProjection,
    history: &[&StudioProjection], item: StudioRecoveryItem,
    mode: StudioRecoveryMode, restorer: DeviceId, scope: PlanScope,
) -> Result<StudioRecoveryPlan, AppError>;
pub(crate) enum PlanScope { SameDocument, CrossDocument }

pub struct StudioRecoveryPlan {
    pub disposition: StudioRecoveryDisposition,
    pub body: Option<Vec<u8>>,
    pub original_author: Option<DeviceId>,
    /// NEW (finding 2): the operation ids in `historical` that this proposal consumes,
    /// derived by the planner from the item it actually resolved. Bounded and small.
    pub source_ops: Vec<[u8; 32]>,
}
```

`control::preview` passes `SameDocument` and maps its `&[StudioRecovery]` to projections; recovery
behaviour is otherwise unchanged. `CrossDocument` keeps every capacity, conflict, tombstone and
over-cap check and drops only the logical-key equality.

| # | Stage | Custody | Work |
|---|---|---|---|
| C1 | Begin | yes | `capture_studio_overlay_copy` (5.2): the accepted source capture plus the destination's two authenticated records and their stamps, one permit, one visit. |
| C2 | Plan | detached | Decode both destination records, build the destination and recovery projections, reconstruct the draft, run `restore::plan`. |
| C3 | Preview | yes | The existing source-stamp recheck plus `studio_destination_is_current`, destination channel known, destination still Open with the same `doc_id`, `expected_projection = recovery_fingerprint()`. Returns one bounded proposed body or an explicit hold. Saves nothing. |
| C4 | Apply | yes | Exact-retry shortcut first (`contains_exact_operation` on the destination), then re-plan from durable state and require `epoch_id`, `expected_projection`, `disposition == Ready` and a byte-identical `body`. Then the ordinary `StudioRequest::Apply` publication path. |

There is **no C5**: copy writes nothing to the branch's record (findings 1, 2, 8).

- **C1'.** Refused, retryably, while a live hold exists on either document, and while a transfer
  hold exists on the destination. Permitted while a transfer hold exists on the source, and it does
  not clear `Prepared`, retire the original envelopes, or count as evidence that the original
  handoff completed.
- **C2'.** A bulk copy is the user issuing C3/C4 per item. There is no batch command and no batch
  atomicity. Each item consumes ordinary admission, typed policy, capacity preflight, reference
  protection and content budget.
- **C-P (finding 2, stated plainly).** Copy is **projection-level**. It recovers the selected value
  of an element as a new operation authored by the copier. It does **not** preserve a branch entry
  that was superseded within the branch, a conflict alternative, the original authorship of an
  accepted envelope, or the accepted ordering. `source_ops` reports exactly which source operations
  the proposal resolved and nothing more. No count of copied items ever establishes that the branch
  was preserved; only the archive of 6.5 does that. The native result and the hooks row say so.

### 6.4 Disposition

One transaction, two truthful modes, a self-contained terminal manifest.

| # | Precondition, checked under one exclusive store visit before any write |
|---|---|
| D1 | Current membership, complete target and channel scope, and the requester is the branch's own author. |
| D2 | No live hold and **no transfer hold**: a Prepared branch refuses outright. |
| D3 | `request.branch` equals the durable `branch_id()` (basis fingerprint and generation), `request.content` equals `branch_hash(active, ledger)`, and `request.accepted` equals the entry count. A UI that has not re-inspected since the branch changed refuses with `BranchChanged`. |
| D4 | For `Preserve`: a durable archive record exists for this document, it decodes, its `content`, `branch` and `generation` equal the branch's, and its entry list equals the branch's entries by id, envelope, sequence and timestamp. This is full-envelope matching against **retained lossless evidence**, not against a re-authored projection edit (finding 2, 16.1). |
| D5 | For `Discard`: `StudioDiscardConfirmation` is present and exact, in addition to D3. No archive is required. Nothing else substitutes for it (finding 9). |
| D6 | Intent and storage preflight for the complete replacement peak, including at the vault cap. |

**The write.** `EpochIntentState` is rebuilt as the ledger with exactly the branch's annotated ids
removed and the extension with `active` cleared and `disposed` set, then encoded, preflighted,
sealed and atomically persisted in **one** replacement through the existing writer and sync barrier.
Only durable completion returns an acknowledgement. For `Preserve` the archive is already durable
before this transaction begins (6.5), so the ordering is archive -> flush -> single atomic intent
replacement, which is 16.2's requirement.

**What is not touched.** No ordinary intent is removed; the removal set is exactly
`disposed.entries`. The receipt-retirement path keeps its existing overlay filter and remains
incapable of removing an annotated id. No source, recovery, owner-journal, Registry or blob write
accompanies the transaction, and **disposal performs no unlink**.

**Reference consequence, corrected (reviewer's N19 point).** Disposal stops the branch from
contributing to the conservative reference set. For `Preserved`, the archive record's own reference
collection (6.5) continues to protect the branch's base-only, superseded, removed and pending CIDs,
so nothing the archive names becomes reclaimable. For `Discarded`, any CID with no other holder
becomes reclaimable after a complete scan, which is exactly what the user confirmed. Revision 1's
claim that the destination's copied operations retain the branch's references was wrong and is
withdrawn: a destination operation retains only the CIDs it names.

**Honest labelling.** `Preserved` satisfies recovery-before-removal: the complete envelopes, order,
timestamps, seed, scope and provenance are retained losslessly and their pixels stay protected.
`Discarded` is an explicit **waiver** of preservation, not a satisfaction of it, and every result
and hooks row says so.

### 6.5 The draft archive record

Placement, answering 16.1's requirement that any archive representation have its own bounded schema,
provenance and coordinated inventory and writer design.

- **Family.** One additional record kind **inside the existing Intents family**: a sibling path in
  the same directory as the intent record, `epoch_draft_archive_path(&scope)`, under a distinct
  sealing domain `b"catcoms/epoch-draft-archive-store/v1"`. This is not a sixth inventoried family;
  it is one more record in a family the scan already walks (O2).
- **Schema.** Plaintext is the scope bytes, an archive header (version, sequence, `at`, `branch`,
  `content`, `generation`, `provenance`, `replayable`) and the section 5.3 payload. Bounded by
  `MAX_ARCHIVE_RECORD_BYTES = MAX_RECORD_BYTES`; sealed by the same framing.
- **Cardinality and quotas.** At most **one archive per logical document**. A second preserving
  disposal on the same document requires the user to release the existing archive first, through
  the separate confirmed `studio_overlay_archive_release`. Vault-wide archives are capped at
  **16 MiB inside** the existing `MAX_VAULT_INTENT_BYTES` of 64 MiB, not additional to it. Refusals
  are `ArchiveCapacity` and retain all work.
- **Accounting.** Charged in `EpochIntentBudget.records`, `record_slots` and `bytes`, with the full
  replacement peak, temporary siblings and both generations invalidated on failed I/O, exactly as
  the intent record is.
- **Inventory.** The Intents arm recognises the archive record kind and charges its bytes and slot.
  The **reference** arm decodes its bounded canonical payload and collects the seed projection's
  CIDs and every operation's CIDs, the same two sets `base_blob_cids()` and
  `hold_creative_operation` produce for a live branch. A corrupt or unsupported archive fails closed
  for reclamation, as every other record does. This is the coordinated inventory work 16.1 demands
  and it must be agreed with Agent 1 (C-1, C-3, I-4) and Agent 3.
- **Ordering.** The archive is written and flushed as its own accounted replacement, taking Agent
  1's `epoch_mutation_guard`, **before** the disposal transaction. A crash between them leaves the
  archive durable and the branch intact; the exact retry re-verifies D4 and proceeds. A crash during
  the archive write removes nothing.
- **Authority.** The archive is never `StudioRecovery`, never replay evidence, never a source,
  never importable, and never occupies a recovery slot or an eviction deadline. It is readable and
  exportable, and it is destroyed only by the explicit release action.

### 6.6 Request identity and rollover (finding 3)

`minimum_new_basis_closed_epoch` is still not advanced by disposal, because a fresh Save on a
still-eligible basis after a disposal is a legitimate new decision. The rollover defence is instead
a namespace:

- `branch_generation` is monotonic per logical document, starts at 1, and is incremented exactly
  once when a branch is first accepted where none exists. It is never reset or reused, and
  exhaustion refuses new branches rather than wrapping.
- `branch_id = H("catcoms/studio-overlay-branch/v1", basis fingerprint, branch_generation)`.
- `studio_overlay_begin` returns the branch id that a new acceptance would create or extend: the
  current id when an active branch exists on that basis, otherwise the id for
  `branch_generation + 1`. `studio_overlay_read` returns the current branch's id.
- Every Save, copy and disposal request carries that `branch`. `classify_request` (5.1) matches it
  against the active branch, the `completed` manifest and the `disposed` manifest, in that order,
  and returns **Stale** otherwise. **Stale is a refusal; it is never a new acceptance.**

The reviewer's trigger now resolves safely: accept and dispose G1 (generation 1), accept G2
(generation 2), dispose G2 replacing the manifest, then deliver a delayed exact retry of a G1
request. Its `branch` names generation 1, which matches neither the retained generation-2 manifest
nor any live branch, so it returns Stale. G1's work is not resurrected and no unbounded history is
kept. Retaining one manifest preserves the terminal acknowledgement for the most recent disposal;
forgetting older ones degrades to refusal, which is the safe direction the reviewer required.

## 7. Stale, rewound and nonpristine bases

Unchanged from revision 1 except for S3. `studio_overlay_lifecycle` classifies from durable state
alone: `SourceMissing`, `SourceNotClosing`, `SourceReplaced`, `SourceRewound`, `CloseMissing`,
`ReceiptChanged`, `SuccessorNotPristine`, `SuccessorMissing`, `TenureUnknown`, `Fault`,
`NotCurrentAuthor`, and `Unconfirmed(..)`. Automatic transfer is refused for every one of them; the
manual path remains available; the branch, its envelopes, its order and its protected references are
retained across restart and any refusal. `check_basis_floor` remains the independent second fence
after a rewind. Agent 1's `Hold` outcomes map onto these reasons, satisfying its prerequisite P2.

- **S3, corrected (finding 5).** A branch that is authenticated, canonical and structurally
  consistent but fails typed reconstruction is classified `Manual(NotReplayable)`. Export, archiving
  and **`Preserved` disposal all remain available**, because none of them requires reconstruction
  (5.3, 6.5). Only the typed projection view and copy planning refuse, and the native result labels
  the reconstruction failure explicitly. Revision 1's position, that discard was the only remaining
  resolution, is withdrawn.

## 8. Durable local work on an awaiting-tenure preview: separate design review

### 8.1 Provenance and the seed problem (finding 6)

The reviewer confirmed the direction and identified that the current preview callback cannot supply
the original checkpoint bytes: `prepare()` consumes `raw`, `PreparedProvisionalStudioSeed` retains a
parsed `UnconfirmedStudioSeed`, and the tail subsequently advances its projection, so
`projection.checkpoint(..)` no longer reproduces the seed (A2).

Correction, in three parts:

1. **Retain the exact bytes.** `UnconfirmedStudioSeed` gains a private
   `seed_bytes: Zeroizing<Vec<u8>>` set in `parse` to the argument it has just proved equal to
   `projection.checkpoint(receipt.close_record_hash)?.bytes()`. It is immutable and unaffected by
   the tail.
2. **Expose it only through the existing scoped callback.**
   `ProvisionalStudioSeedUse` gains `seed_bytes: &'a [u8]`, so every current-scope check the
   accepted contract already performs (mount, numeric server, channel, copied watch, attempt
   generation, current membership, proven provider identity, unexpired hint) gates access to it.
   No public accessor and no `Clone` is added.
3. **Do not trust the retention.** The mint copies the bytes under custody, and the **detached**
   plan stage re-runs `UnconfirmedStudioSeed::parse(target, &receipt, &captured_bytes)` before the
   branch is built. That re-proves the receipt binding, the canonical compact encoding and the
   seed-to-projection identity from first principles. A mismatch refuses with no durable change.
   The same re-parse runs on every restart reconstruction of the branch.

**Memory and capacity accounting**, which the reviewer required to be explicit: a ready preview now
retains its parsed graph **and** its original seed bytes, at most 2 MiB each. That retention is
inside the existing retained-seed slot, not additional to it, and the design's own rails (8.3) count
it. With the accepted three preview-eligible slots the worst case adds up to 6 MiB of retained bytes
across the runtime, which must be measured (15.6) and reported honestly, not assumed.

```rust
/// Minted ONLY inside a live `with_provisional_studio_seed` callback. No public constructor and
/// no path from a caller-supplied receipt, epoch id or projection.
pub struct StudioUnconfirmedOverlayBasis(/* private */);
impl StudioUnconfirmedOverlayBasis { pub fn fingerprint(&self) -> [u8; 32]; }
```

It binds `target`, the local device as `author`, the candidate `Receipt` bytes, the exact seed
bytes, the provider `DeviceId`, the current MLS epoch and the receiver-clock observation time;
`source_id` and `source_version` are canonically zero. Additional mint conditions: `tail_complete()`
must be true; the target's logical document must have **no installed source**; the requester and the
provider must both be current members and the provider the proven endpoint identity of the hint's
peer. `fingerprint()` covers the provenance discriminant, so a `Closing` and an `Unconfirmed` basis
over the same receipt and seed cannot be interchanged.

### 8.2 What is persisted, and why the tail is not

The persisted base is the seed checkpoint only, at
`doc_id = epoch_id(doc_type, logical_key, closed_epoch + 1, close_record_hash)`. The signed tail is
not persisted: it is bounded at 20,000 operations and 4 MiB, which cannot coexist with a 2 MiB seed
in a 5 MiB record, and persisting other members' signed operations would require re-verifying
foreign signatures out of the vault on every restart. The reviewer's 16.4 accepts this.

The consequence is stated plainly and tested: **typed admission runs against the seed-only base**, so
an operation valid only against the tail is refused at acceptance with an explicit reason, before
any durable change, even though the live preview displays the merged content. Section 11's native
result distinguishes the merged preview view from the persisted draft base.

### 8.3 Local storage and quotas

Simultaneous, not additive, and all checked before acknowledgement. The reviewer's 16.5 endorses the
layered shape and correctly notes the values are unvalidated.

| Rail | Value |
|---|---|
| Unconfirmed branches per logical document | 1 |
| Unconfirmed branches per server | 3 |
| Accepted operations per unconfirmed branch | 64 |
| Seed bytes | 2 MiB (existing checkpoint limit) |
| Extension metadata | 64 KiB (unchanged) |
| Record total | `MAX_RECORD_BYTES` (unchanged) |
| Vault-wide unconfirmed persisted bytes | 8 MiB, **inside** `MAX_VAULT_INTENT_BYTES` |
| Retained original seed bytes per ready preview | 2 MiB, inside the existing retained-seed slot |

Per-document, per-server and vault-wide limits all apply together with the record and metadata
limits; a per-channel limit alone would allow aggregate growth as channels accumulate. The
per-server count and vault-wide byte total come from the inventory's Intents arm, which requires
Agent 1's structural decode to expose the provenance discriminant and the charged bytes (section
14). Finding 6's retained seed bytes are counted in the **memory** accounting as well as the
persisted-byte accounting. Refusals are `StorageRefused { reason }` and retain all existing work.

### 8.4 Expiry versus retained work

Unchanged from revision 1. Preview expiry, capacity eviction, replacement, unwatch and rewatch,
lock, mount or server replacement, membership change and restart remove the live preview and never
the durably accepted branch; a retained branch never revives a preview, extends a hint lifetime,
re-enters the ready cache or produces an `AwaitingTenureReceipt` result. A refused acceptance leaves
editor work unsaved and visible and reports no durable success.

### 8.5 What an unconfirmed branch can never do

Installed source; epoch gate; `VerifiedCheckpoint`; owner tenure; receipt issuance, verification or
publication; signing; Registry pointer publication; settlement; receipt-covered retirement;
`StudioRecovery` evidence; replay evidence; ordinary Apply; automatic handoff or any `Prepared`
state, which `validate` forbids outright. What it can do: the same four manual operations with the
same fences, plus the derived classification below.

### 8.6 Reconciliation, derived rather than persisted

Computed on read from durable state; nothing is written, so there is no reconciliation crash window.
`AwaitingSource` while no installed source exists. `BaseConfirmed` when the installed source's
`doc_id` equals the branch's base `doc_id` **and** its opening checkpoint's seed change hash equals
the branch's, in which case copy into that source becomes available when it is Open.
`BaseSuperseded` otherwise, with copy still offered against the actual current projection under an
honest label. `BaseConfirmed` is a statement that two hashes agree, never a promotion of preview
attribution, tenure or signing authority.

### 8.7 Acceptance path

Distinct control actions so neither path can be reached with the other's request:
`BeginUnconfirmedOverlaySave`, `PrepareUnconfirmedOverlaySave { branch, nonce, body }`,
`FinishUnconfirmedOverlaySave`. Staging, stamps, admission, permit ownership, PIX admission
placement, retry classification and commit ordering are Agent 1's Flow S unchanged, with three
substitutions: `studio_closing_basis` becomes the 8.1 mint; the S3 re-mint re-enters
`with_provisional_studio_seed` and requires the same fingerprint; and 8.3's rails are charged
alongside the ordinary ones. The detached stage re-parses the captured seed bytes (8.1 part 3). If
Agent 1's Flow S is not implemented, this path is not implemented either; it is not a second writer.

## 9. Repeated-owner tenure

### 9.1 What already works

`ReceiptBook` and `OwnerReceiptJournal` are tenure-keyed, and a continuously present member already
observes A -> B -> A correctly. The integration work is to prove this end to end through real
actors, real membership changes and real restart, and to prove the refusals.

### 9.2 The required cases

T1 A -> B -> A with a continuously present observer; T2 restart between each transition; T3 a member
joining between owner changes; T4 hidden higher old-tenure history; T5 a device that becomes owner
by its own join. Details as in revision 1; T5 is section 9.3.

### 9.3 The correction, and how L5 is closed (finding 7)

**Problem.** `new_joined` calls `OwnerTenure::unknown` unconditionally, so a device joining into a
recycled low leaf becomes the designated committer with `None` and can never issue a receipt, while
every witness knows the answer.

**Part 1: the self-join inference, rephrased.** Revision 1 justified it as "was not a member at any
earlier epoch", which the reviewer correctly says is false for a returning device. The accurate
statement is about **current continuous membership**:

> This device's current continuous membership in this group began at this epoch. A tenure is an
> uninterrupted run as designated committer, so this device's *current* tenure cannot have begun
> before its current membership did. If it is the committer now, its current tenure started here.

```rust
// crates/catcoms-sync/src/owner_tenure.rs
impl OwnerTenure {
    /// Deliberately NOT available to `unknown`, which also serves legacy snapshots where the
    /// device may have been committer for an unknown number of prior epochs.
    pub(super) fn joined(group: &ServerGroup, device: &MlsDevice) -> Self {
        let mut state = Self::unknown(group);
        if group.designated_committer() == Some(device.device_id()) {
            state.start = Some(state.position.epoch);
        }
        state
    }
}
```

`new_joined` calls `joined(&this.group, &this.device)`. `unknown`, `new`, the restore path,
`applied`'s existing arms, `start` and `decode`'s `start > epoch` rejection are untouched.

**Part 2: why the old safety argument is withdrawn.** Revision 1 claimed a wrong value produces
receipts nobody accepts. A4 shows that is false: `complete_checkpoint_head_scoped` refuses a
proof/observation mismatch only when local observation is `Some`, so an Unknown-tenure newcomer
accepts a fresh proof's claimed tenure and can hold a selection a continuously observing witness
refuses. Disagreement is therefore not self-correcting, and "until a later witnessed transition" is
not a progress guarantee. Agreement must be established structurally.

**Part 3: extending the observation rule so every participant sees the same discontinuity.** A
witness currently compares `Position { owner: Option<DeviceId>, epoch }`, which cannot distinguish a
same-commit remove-and-re-add of the committer from an ordinary same-owner commit. `Position` gains
the committer's leaf identity:

```rust
// catcoms-mls
impl ServerGroup {
    /// Leaf index and a digest over the designated committer's leaf identity:
    /// blake3(index, signature_key, credential bytes). The HPKE `encryption_key` is
    /// DELIBERATELY EXCLUDED so that an ordinary self-update, which rotates keys but keeps the
    /// credential, does not look like a discontinuity.
    pub fn designated_committer_leaf(&self) -> Option<(u32, [u8; 32])>;
}

// catcoms-sync
struct Position { owner: Option<DeviceId>, leaf: Option<(u32, [u8; 32])>, epoch: u64 }
```

`applied` gains one explicit arm: across a contiguous step, when `before.owner == after.owner` and
both are `Some` but `before.leaf != after.leaf`, the result is `Some(after.epoch)`, a new tenure.
Every other arm is unchanged, so same-owner commits and self-updates still preserve knowledge.

The credential is the correct discriminator because `group.rs:247-274` binds a joiner's KeyPackage
credential to `(this group, invite_nonce)`, so a genuine rejoin always presents a different
credential, while an update does not change it (A3). Under this rule the witness computes
`Some(after.epoch)` and the rejoining device computes `Some(join epoch)`, which is the same value.
L5's disagreement is closed.

**Part 4: the residual of the residual, and its membership rule.** A re-add that reuses the same
signature key **and** the same credential at the same leaf index would still be invisible. Two
conditions exclude it, both testable: the invite path binds a fresh per-join nonce into the
credential and the invite ledger refuses replay, so a legitimate rejoin cannot reuse a credential;
and the commit builder refuses to remove and re-add the designated committer in one commit. This is
the reviewer's "enforce a membership rule excluding this transition", applied only to the case part
3 cannot see.

**Snapshot format.** `OwnerTenure::encode`/`decode` gain a versioned tail carrying the leaf digest;
the 57-byte cap becomes 97. A v1 snapshot decodes by taking `owner` and `epoch` from the snapshot
and the leaf identity from the live group, which is safe because `Position::of(group)` reads the
live group anyway and the existing equality check still refuses a stale position.

**What this does not fix.** A legacy snapshot whose owner has no saved tenure bytes stays Unknown,
correctly. A device that becomes committer across an epoch gap it did not observe stays Unknown.
Section 9.5 is not implemented.

### 9.4 The live-tenure seam for Agents 1 and 3

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOwnerTenure { Known(u64), Unknown }
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub fn observed_owner_tenure(&self) -> StudioOwnerTenure;
    pub(crate) fn require_observed_owner_tenure(&self) -> Result<u64, AppError>;
}
```

- **V1.** `Unknown` is fail-closed for minting a Closing overlay basis, first local acceptance,
  handoff preparation, every signing turn, the commit, receipt issuance, rotation, Registry pointer
  publication, and Agent 3's live v2 repair issuance and application.
- **V2.** A returning owner in a new tenure observes a strictly different value from its earlier
  tenure, now including the 9.3 part 3 case.
- **V3.** The value is read at every custody visit that needs it and never cached across one.
- **V4.** A verified-reading tenure from a fresh owner proof travels only inside the existing
  private `HeadSelection` / `RegistrySeedFetch` / `CheckpointSeedSelectionUse` types. It is never
  written into `owner_tenure`, never returned by `observed_owner_tenure`, and never usable as this
  device's own authoring tenure. A4 is why this separation matters.
- **V5.** Agent 3 takes `require_observed_owner_tenure()` for issuance and holds, never substitutes,
  on `Unknown`.

### 9.5 Not proposed for implementation

The witnessed-transition attestation protocol is recorded as the considered and rejected
alternative for legacy-snapshot and unobserved-gap owners. It is a new authority protocol whose
guarantee is a quorum-of-witnesses property rather than a cryptographic proof, and the reviewer's
16.3 agrees it is not a substitute for the 9.3 integration.

## 10. References, admission and budgets

- **R1.** Retained branches keep their conservative protection unchanged: the inventory's reference
  path enumerates `base_blob_cids()` plus every pending operation's CIDs.
- **R2.** Copy adds ordinary references through the destination's own operations, under
  `hold_creative_operation` and the ordinary admission and possession checks.
- **R3, corrected.** Disposal performs no unlink. For `Preserved`, the archive record's reference
  collection (6.5) keeps every CID the branch named protected. For `Discarded`, a CID with no other
  holder becomes reclaimable after a complete scan. A destination copy retains only the CIDs it
  names and is not a preservation mechanism (reviewer's N19 point).
- **R4.** The disposal and archive writes obey the same budget discipline as every other intent
  write: full replacement peak preflight, both generations invalidated on failed I/O, refusal at the
  vault cap without spending deletion credit, and an exact sync-only retry needing no replacement
  headroom.
- **R5.** Unconfirmed branches charge the ordinary per-server content budget in addition to 8.3's
  rails, consume no settlement or protocol reserve and create no unscanned cache.

## 11. Native results, events and the proposed UI-hooks update

```ts
type OverlayInspection =
  | { v: 1; kind: "absent"; channel: Decimal; object: Hex32 | null }
  | { v: 1; kind: "local-draft"; channel: Decimal; object: Hex32 | null;
      basis: Hex64; branch: Hex64; content: Hex64; generation: Decimal; accepted: number;
      transferState: "active" | "prepared";
      provenance: "closing" | "unconfirmed";
      eligibility: "transferable" | "manual";
      manualReason: OverlayManualReason | null;
      unconfirmedState: "awaitingSource" | "baseConfirmed" | "baseSuperseded" | null;
      replayable: boolean; archived: boolean;
      readOnly: true; content_: StudioContent | null }
  | { v: 1; kind: "disposed"; channel: Decimal; object: Hex32 | null;
      basis: Hex64; branch: Hex64; generation: Decimal; accepted: number;
      mode: "preserved" | "discarded"; archive: Hex64 | null };

type OverlayManualReason =
  | "sourceMissing" | "sourceNotClosing" | "sourceReplaced" | "sourceRewound"
  | "successorNotPristine" | "successorMissing" | "receiptChanged" | "closeMissing"
  | "tenureUnknown" | "fault" | "notCurrentAuthor" | "unconfirmed" | "notReplayable";
```

`replayable: false` carries a null typed projection with every other field present, which is
finding 5's user-visible shape.

**Write outcomes, corrected (finding 8).** Every command that writes reports exactly one of three
states, and they are never conflated:

| Outcome | Meaning | Correct client action |
|---|---|---|
| `refused` | The refusal happened before any durable write. Nothing changed. | Fix the named condition; a new attempt is safe. |
| `uncertain` | A write may have landed. | Retry the **exact** request; do not mint a new nonce or a new confirmation. |
| `partial` | A prior durable step of a multi-step sequence succeeded and a later one did not. Today the only such sequence is `archive durable, disposal pending`. | Retry the exact disposal; the archive is not rewritten. |

Revision 1's blanket rule that a storage refusal keeps the work unsaved applies only to `refused`.

Proposed rows for `FLIPNOTE-UI-HOOKS.md`, which Agent 4 applies:

| UI action | Native command | Result |
|---|---|---|
| Read a retained local draft | `studio_overlay_read` | Extended `OverlayInspection`; read-only, never an ordinary view |
| Show the lifecycle row | `studio_overlay_lifecycle` | Cheap structural state and reason; no content |
| Back up a draft to a file | `studio_overlay_export` | Bounded `{format:"catcoms-studio-draft-v1", basis, branch, accepted, provenance, replayable, bytes, bytesB64}`; changes nothing and authorizes no deletion |
| Keep a lossless in-app archive | `studio_overlay_archive` | Durable archive record; required before a preserving disposal |
| Read or release the archive | `studio_overlay_archive_read`, `studio_overlay_archive_release` | Release is separately confirmed and destroys the archive |
| Preview copying one item | `studio_overlay_copy_preview` | One proposed domain edit or an explicit hold; saves nothing; reports the source operations it resolved |
| Apply that exact copy | `studio_overlay_copy_apply` | Ordinary provisional content Save into the destination |
| Dispose of a draft | `studio_overlay_dispose` | `mode:"preserve"` requires a matching durable archive; `mode:"discard"` requires the exact confirmation token and destroys the bodies |

Truthfulness rules asserted by tests:

- `local-draft` is local only, with or without an archive or any number of copied items.
- **A copy count is never a preservation claim** (C-P). Only `archived: true` plus a matching
  `content` establishes that the branch is losslessly retained.
- `mode:"discarded"` states plainly that the bodies are gone; it is a waiver, not a preservation.
- `provenance:"unconfirmed"` is unconfirmed history; `unconfirmedState:"baseConfirmed"` means two
  hashes agree, not that the provider was ever owner. The merged preview view and the persisted
  draft base are labelled separately (8.2).
- `manualReason:"tenureUnknown"` means this device cannot presently prove the current owner's
  tenure, not that anything is wrong with the work.

Events reuse the existing bounded `SettlementNotices` rail and the `settlement-changed` channel:
`LocalDraftManual` and `LocalDraftDisposed`. Neither is a delivery, settlement or finality claim.
Agent 1's `LocalDraftRetained` and `LocalDraftHandedOff` are separate.

## 12. Crash, interruption and recovery ordering

| Interruption | Result |
|---|---|
| Any detached rebuild, including cancellation | No durable byte changed; branch, ledger, protection and permit intact until the worker drops them; the source and destination stamp rechecks refuse a stale result. |
| During the archive write, before rename | Nothing archived, nothing removed; the exact retry re-archives. |
| After the archive rename, before its flush | The exact retry reloads the authenticated archive, verifies D4 and flushes; no second archive and no second sequence. |
| Archive durable, disposal not yet attempted or failed | `partial`. The branch is intact and the exact disposal retry proceeds without rewriting the archive. |
| During the disposal write, before rename | Nothing removed, nothing recorded; the request is retryable verbatim. |
| After the disposal rename, before its flush | The exact retry reloads the record, sees `disposed`, and performs the sync-only flush `retire_included_with_io` already implements for `removed == 0`. |
| Copy interrupted at any point | The ordinary Save retry contract applies unchanged; there is no bookkeeping write to be inconsistent with (findings 1, 8). |
| Disposal requested while `Prepared` | Refused by D2. |
| Restart with a retained unconfirmed branch and no preview | Reconstructs by re-parsing its own persisted seed bytes against its receipt (8.1 part 3); `AwaitingSource` until an installed source exists. |
| Restart mid-copy with the destination rotated | The destination stamp, `epoch_id` and fingerprint refuse; re-preview against the new Open epoch. |

## 13. Prerequisites this design supplies to Agent 1

| Agent 1 requirement | Supplied by | State |
|---|---|---|
| P1 reviewed manual lifecycle | 6.1-6.6, 12 | Designed, unimplemented, unreviewed |
| P2 every `StudioOverlayHold` variant mapped to an actionable state | 7, 11 | Designed |
| P3 truthful native results, events and hooks rows | 11, including the corrected three-state write outcome | Designed |
| P4 live-tenure contract | 9.4 V1-V5, with 9.3's progress path | Designed |
| P5 explicit statement that P1-P4 are implemented and reviewed | Status note | **No** |

Agent 1 must not register `studio_overlay_save` on the strength of this document.

## 14. Dependencies and integration changes for Agent 4

| File | Change | Note |
|---|---|---|
| `catcoms-replication/src/studio/overlay.rs` | provenance on `BasisData` and its fingerprint | Core; own verdict line |
| `.../studio/overlay/handoff.rs`, new `overlay/disposal.rs` | v3 encoding, `branch_generation`, `branch_id`, `classify_request`, the terminal `disposed` arm, extended `validate`, and the provenance guard on `prepare_handoff*` | Core; shared with Agent 1's C-1 |
| `.../studio/provisional.rs` | retained `seed_bytes` and the scoped accessor (finding 6) | Core; boundary (b) |
| `catcoms-sync/src/registry_seed/provisional/seed.rs` | `ProvisionalStudioSeedUse.seed_bytes` | Sync; boundary (b) |
| `catcoms-mls/src/group.rs` | `designated_committer_leaf()`; the commit builder's remove-and-re-add refusal | **Authority-bearing; boundary (c)** |
| `catcoms-sync/src/owner_tenure.rs`, `lib.rs` | `Position.leaf`, the `applied` discontinuity arm, `joined`, the versioned snapshot tail, the `new_joined` call site | **Authority-bearing; boundary (c)** |
| `catcoms-app/src/store/epoch_intents.rs` | `StudioOverlayLifecycle`; the Intents-arm archive record kind, provenance and unconfirmed byte counters | Shared with Agent 1 (C-1, C-3, I-4) and Agent 3 |
| `.../store/epoch_intents/retirement.rs` | Unchanged; the overlay filter stays | Shared with Agent 3 |
| new `.../store/epoch_intents/{disposal,archive}.rs` | The disposal transaction and the archive record | Agent 2 leaves |
| `.../store/epoch_intents/inspection.rs` | `StudioInspectionPurpose`, `rebuild_for`, the composite copy capture and destination stamp | Shared with Agent 1 |
| `.../store/epoch_studio.rs` | wider visibility for `read_studio_record` | Shared with Agent 3 |
| `.../store/epoch_recovery/inventory.rs` | archive record kind and its reference collection | **Shared with Agent 1 and Agent 3; highest-risk item after I-4** |
| `catcoms-app/src/studio/restore.rs` | `history: &[&StudioProjection]`, `PlanScope`, `source_ops` | Agent 2 |
| `catcoms-app/src/studio/{control,dispatch}.rs` | New actions and responses | **Central enum edit** |
| `catcoms-app/src/studio/{inspection,settlement}.rs` | `rebuild_for` plumbing; two new settlement variants | Shared with Agent 1 |
| new `catcoms-app/src/studio/{lifecycle.rs, overlay/copy.rs}` | Classification and copy driver | Agent 2 leaves |
| `apps/desktop/src-tauri/*` | Nine commands, registration, security and capability rows | Agent 4 registers |
| `docs/*` shared | Section 11's rows | **Agent 4 owns; not edited here** |
| `.github/workflows/studio-overlay.yml`, `.github/scripts/` | A `lifecycle` job and a mutation script | Agent 4 owns |

**Handed to Agent 1:** structural decode must expose the provenance discriminant and charged bytes;
the Save classification must call `classify_request` rather than a bare `completed_retry`; Flow S's
basis mint is parameterized by provenance; the archive writer takes `epoch_mutation_guard` and the
inventory cursor must cover the archive record kind. **Handed to Agent 3:** the 9.4 seam with V1 and
V5; repair must resolve an interrupted Prepared overlay through the existing fence and must never
remove an annotated id outside the 6.4 transaction; an `Unconfirmed` branch is not repairable
history.

## 15. Limits, costs and measurements

Nothing here is measured. Required, for Index and Flipnote: (1) export and archive at 1, 64 and 256
operations and at the maximal record shape, separating the two custody visits from the detached
stage; (2) copy preview at the same shapes, separating destination decode, projection build and
`restore::plan`; (3) the disposal transaction at 256 entries with both manifests present, plus the
sync-only exact retry at the vault cap; (4) the archive record's effect on a five-family inventory
and on its reference arm; (5) `studio_overlay_lifecycle` on a vault with several large retained
branches; (6) the retained original seed bytes of finding 6 across three ready previews, as actual
retained memory rather than an assumed bound.

- **L1.** Copy planning and the typed projection view require reconstruction; export, archiving and
  `Preserved` disposal do not (corrected by finding 5).
- **L2.** Copy is per item, with no batch command and no batch atomicity.
- **L3.** Two full 256-entry manifests plus headers occupy roughly 43 KiB of the 64 KiB metadata
  ceiling; a document that has completed a 256-entry transfer and then accumulates a second
  256-entry branch can refuse disposal with `MetadataFull`, retaining the branch. Must be measured.
- **L4.** `Discarded` destroys the operation bodies; only the bounded manifest survives.
- **L5, superseded.** Revision 1's tenure residual is closed by 9.3 parts 3 and 4. What remains is
  narrower: the discriminator depends on credentials binding a fresh per-join nonce and on the
  commit builder refusing a same-commit remove-and-re-add of the committer. Both are testable
  obligations, not assumptions, and N-T6b asserts them.
- **L6.** A legacy-snapshot owner and an unobserved-gap owner remain Unknown and cannot rotate.
- **L7.** The persisted unconfirmed base is the seed checkpoint, not the previewed tail.
- **L8.** There is no import path for an export or an archive.
- **L9 (new).** One archive per logical document. A second preserving disposal requires an explicit
  release first, and the vault-wide archive ceiling of 16 MiB can refuse an archive on a vault that
  is otherwise within its intent budget.
- **L10 (new).** A ready preview now retains up to 2 MiB of original seed bytes in addition to its
  parsed graph; the worst case across three slots is 6 MiB of retained memory, unmeasured.

## 16. Open questions for the re-review

Revision 1's six questions were answered and those answers are adopted. Remaining:

1. **Archive placement.** Is a second record kind inside the existing Intents family, with its own
   sealing domain, budget participation, inventory arm and reference collection, the right
   representation, or should the archive be its own inventoried family despite the collision with
   Agent 1's I-4 and Agent 3's writers?
2. **Archive cardinality.** Is one archive per logical document plus an explicit release the right
   bound, or should a small bounded set with an eviction rule exist? A set reintroduces the
   eviction-versus-preservation tension the recovery rail already has.
3. **Generational identity.** Does `branch_id` including `branch_generation` fully close finding 3,
   and is returning `Stale` (rather than a terminal acknowledgement) acceptable for a retry of a
   disposal older than the one retained manifest?
4. **The leaf discriminator.** Is `blake3(index, signature_key, credential)`, excluding the HPKE
   `encryption_key` so self-updates preserve knowledge, the right field set? Is the commit-builder
   membership rule of 9.3 part 4 necessary, or does the invite ledger's fresh-nonce credential
   binding already exclude the case on its own?
5. **Preview seed retention.** Is retaining the exact verified seed bytes plus a detached re-parse
   preferable to a bounded reconstruction API, given L10's memory cost?

## 17. Test and mutation plan

### 17.1 Normal regressions

Revision 1's N1-N5, N20-N21, N23-N28, N30-N31 and N-T1 to N-T5, N-T7 to N-T8 are retained with the
schema changes of section 5. Added, changed or corrected:

| # | Level | Case | Independent observation |
|---|---|---|---|
| N6 | actor | Same-document copy after rotation, per item | Each item routes through the ordinary Save path, is authored by the copier with a fresh nonce, and appears in the destination projection; **no byte of the branch's record changes at any point** (findings 1, 2, 8). |
| N7 | actor | Copy exact retry after a lost response | `already_saved` true, no second destination operation; still no branch-record write. |
| N8 | actor | Stale `expected_projection` or `epoch_id` | Refused; nothing saved; re-preview succeeds. |
| N8b | store | **Finding 4 destination currency.** Change the destination's source record, then its recovery record, between C1 and C3, and again between C3 and C4, each with an authenticated same-size replacement | Each refuses at `studio_destination_is_current` with its own digest or size comparison; each fixture passes the source stamp check first. |
| N9 | store | Copy admission failures: `FLIPNOTE_MAX_FRAMES`, `FLIPNOTE_FRAME_BYTES`, `MAX_INDEX_OBJECTS`, over-cap, tombstoned target, missing PIX | Each yields its specific disposition or refusal; no partial destination write; branch and references intact. |
| N9b | store | **Finding 2 derived source ids.** A branch with one accepted `InsertFrame` referencing CID X and a base title. Request a Title copy | `source_ops` names the title's source operation and **not** the insertion; no request field can name a different entry, because `source_entry` does not exist; the native result reports the derived ids. Then assert that no copy count or `source_ops` value permits a `Preserve` disposal. |
| N10 | store | Cross-document copy while `Prepared` | Permitted into a genuinely distinct Flipnote in the same channel; refused when the destination is the branch's own document reached through another channel label; `prepared` never cleared. |
| N11 | store | **Finding 1 positive terminal case, `Preserved`.** Archive, then dispose, then reopen the vault | The archive is durable and decodes; exactly the annotated ids are gone; every ordinary intent remains; the record re-encodes canonically with `active == None`, `prepared == None` and `disposed` present; a second reopen is byte-identical. |
| N12 | store | `Preserve` without a matching archive, and with an archive whose `content`, `branch`, `generation` or entry list differs | Each refuses at D4 with the branch fully retained. |
| N13 | store | **Finding 1 positive terminal case, `Discarded`.** Dispose with the exact confirmation, then reopen | Manifest present with `mode:"discarded"`, entries gone, ordinary entries intact, canonical re-encode and reopen both succeed. |
| N14 | store | Wrong `branch`, wrong `content`, wrong `accepted`, wrong author, wrong channel, a missing confirmation, and a confirmation with any other literal | Each refuses at its own check with the branch intact; each fixture passes every earlier check first. |
| N15 | store | Disposal under a transfer hold and under a live hold | Refused; read-only export and archiving still succeed under the transfer hold. |
| N16 | store | Interrupt the archive write and the disposal write at each barrier | Reproduces section 12's table, including the `partial` state and the post-rename sync-only retry with no second manifest and no second sequence. |
| N17 | store | Delayed Save retry for the most recently disposed branch | `classify_request` returns `Disposed` before any basis mint, tenure read, source lookup or media work; terminal acknowledgement; no new branch. |
| N17b | store | **Finding 3 rollover.** Dispose G1 on basis B, accept G2 on B, dispose G2, restart, then deliver delayed exact retries of both G1 and G2 requests | G2's retry is acknowledged from the retained manifest; **G1's retry returns Stale and creates no branch, no entry and no second envelope**; `branch_generation` is monotonic across both disposals and across restart. |
| N18 | store | Retirement naming both ordinary and disposed ids | Ordinary ids retire; disposed ids are already absent; no annotated id of a live branch is ever removed by that path. |
| N19 | store | **Corrected reference lifecycle.** Base-only, superseded, removed-frame and pending CIDs, through: branch retained, archived, `Preserved` disposal, and `Discarded` disposal | Retained and archived: all four survive cleanup and reopen, the archived case proved by removing the branch and keeping only the archive. After `Preserved` disposal: still all four, held by the archive's reference collection. After `Discarded` disposal: a CID with no other holder becomes reclaimable and a CID still named by another holder does not. |
| N19b | store | Archive record accounting and corruption | The archive charges its bytes and slot; the vault-wide 16 MiB ceiling and the one-per-document rule each refuse before any write; a corrupt archive fails closed for reclamation rather than releasing its references. |
| N22 | store | **Finding 5 non-replayable branch** | Classification is `Manual(NotReplayable)`; export and archiving succeed and contain the exact seed, ordered envelopes, timestamps, scope and provenance with `replayable == 0`; `Preserved` disposal succeeds; only the typed projection and copy planning refuse; metadata readers do not fail. |
| N25b | actor | **Finding 6 seed extraction.** Accept unconfirmed work from a preview whose tail is non-empty | The captured bytes equal the originally fetched seed, not a checkpoint of the tail-advanced projection; the detached re-parse against the candidate receipt succeeds; a mutated captured byte fails the re-parse with no durable change; after restart the branch reconstructs from its persisted bytes with the same projection. |
| N-T6 | actor | Remove then rejoin in separate commits, the rejoining device becoming owner | The rejoining owner and every witness observe the same start; its receipts verify. |
| N-T6b | actor | **Finding 7 same-commit case.** Remove and re-add the designated committer in one commit, with four observers: the rejoining owner, a known-tenure witness, an **Unknown-tenure newcomer** that requests a fresh owner proof, and a restarted copy of the witness | With 9.3 part 3 the witness and the rejoining owner agree, so the newcomer's proof-derived selection agrees too, and the restarted witness agrees after `decode`. Separately assert that the commit builder refuses to construct such a commit (part 4) and that an ordinary committer self-update does **not** reset the observed start. |

### 17.2 Isolated mutations

Revision 1's M2-M8 (renumbered where the guard moved), M11-M14, M16-M21 and M23-M26 are retained.
Changed, added or corrected:

| # | Guard removed | Test | Assertion |
|---|---|---|---|
| M1 | The `validate` rule that no id occurs in both `completed.entries` and `disposed.entries` | N23 | "an id was both transferred and disposed": the record decoded and re-encoded successfully. |
| M1b | The `disposed`-is-self-contained property: reintroduce a `disposed` field that refers to a live field | N11 | "a terminal record failed to reopen": the post-disposal reopen fails, which is finding 1's defect made executable. |
| M3 | D3's `content` equality | N14 | "a stale request disposed a branch it had not seen". |
| M3b | D3's `branch` equality (generation ignored) | N17b | "an old generation's request matched the current branch". |
| M4 | D4's archive entry-list comparison, then separately its `content` comparison | N12 | "a branch was disposed against a non-matching archive": two mutations. |
| M5 | D5's confirmation requirement, then separately the exact-literal check | N14 | "a discard proceeded without explicit confirmation"; "any string confirmed a discard". |
| M9 | **Corrected.** Split the single sealed replacement into two writes, manifest first then ledger removal, leaving the intermediate state reachable | N16 | The fixture must reach the interruption **between the two writes** and observe "entries removed with no durable manifest" or "a manifest with the entries still present". A refusal caused by an invalid replacement does **not** count, so the mutant must produce two individually valid records. |
| M10b | `classify_request` returning `Stale` for an unknown branch id (return a new-acceptance class instead) | N17b | "a disposed branch was resurrected by a delayed retry". |
| M15 | C4's `contains_exact_operation` retry shortcut | N7 | "a copy retry created a second destination operation". |
| M22 | 9.3's `designated_committer == device` condition in `joined` | N-T2 | "a joiner that is not the committer invented the current owner's tenure". |
| M22b | The `before.leaf != after.leaf` arm in `applied` | N-T6b | "a witness preserved a stale tenure across a real membership discontinuity": the witness and the rejoining owner disagree, and the Unknown-tenure newcomer accepts the value the witness refuses. |
| M22c | The exclusion of `encryption_key` from the leaf digest | N-T6b's self-update case | "an ordinary committer self-update reset the observed tenure". |
| M26 | **Corrected fixture.** The `Unconfirmed`-forbids-`prepared` rule in `validate` | N23 | The fixture must be a record that passes **every other** structural guard, including provenance encoding, zero source identifiers, entry ordering and canonical re-encode, so the failure isolates this prohibition. |
| M27 | 8.1 part 3's detached re-parse of the captured seed bytes | N25b | "a mutated captured seed became a durable base". |

Reconciliation mutations M21 must use a fixture where document identity and seed-hash matching
differ independently, so removing either half of the predicate fails on the intended comparison and
not on an earlier validation. Every entry requires a unique anchor, one executed failing test, the
intended assertion, byte-exact restoration and a passing restored regression. No mutation result
exists yet.

### 17.3 Harness and workflow

A new `.github/scripts/check-studio-overlay-lifecycle-mutations.py` following
`check-studio-overlay-mutations.py`, with logs under `logs/gate4-overlay-lifecycle-*.log`, and a
requested `lifecycle` job in `.github/workflows/studio-overlay.yml` added to a required workflow.
Local execution stays serial: `-j 1`, the existing per-package test debug override, no concurrent
Cargo work, no blanket cleanup.

## 18. Re-review request

Fill `[FULL_HEAD_SHA]` with the commit that adds this revision before sending. Do not send a
placeholder.

```text
Review type: design re-review after CHANGES REQUIRED on all three boundaries.
Base: a901f6b0f64df2b4ea9cc0221b64ac98276f582d. Head: [FULL_HEAD_SHA].
Compare: https://github.com/Thalpy/Mewtual/compare/a901f6b0f64df2b4ea9cc0221b64ac98276f582d...[FULL_HEAD_SHA]
Scope/evidence: docs/GATE4-AGENT-2-DESIGN.md revision 2 and docs/GATE4-AGENT-2-STATUS.md.
Design only: no production code, no test and no measurement exists. No Cargo command was run.
Dependencies unchanged: e65bfd8 is still unreviewed; Agent 1's runtime design is unaccepted and is
consumed by name only; Agent 3's design is not consumed except through the tenure seam; native Save
stays unregistered and out of FLIPNOTE-UI-HOOKS; GATE4-AGENT-2-STATUS still states that Agent 1's
P5 is false.

Please return three separable verdicts again: (a) manual lifecycle, stale bases and repeated-tenure
integration; (b) the preview-local-work extension; (c) the locally observed tenure correction,
which now also touches catcoms-mls.

Section 0 maps each of the nine findings to its correction. Verify each against the code, not the
prose. The four structural changes to attack first:

Finding 1 and 2 are corrected by DELETING copy bookkeeping from the durable record. There is no
`copy` arm, no C5 stage and no `source_entry` request field. Disposal's preserving mode now rests on
a durable draft archive record, and the terminal `disposed` manifest is self-contained: confirm from
the proposed `validate` that no rule of `disposed` refers to `active`, `prepared` or any live field,
so a record with `active == None` re-encodes canonically and reopens. Attack N11, N13 and M1b, which
is the finding-1 defect made executable. Then attack the honesty of C-P in 6.3: copy is stated to be
projection-level and lossy for superseded operations, conflicts, ordering and original provenance,
and no copy count may permit a preserving disposal (N9b).

Finding 3 is corrected by a branch-generation namespace rather than acknowledgement history.
`branch_id = H(basis fingerprint, branch_generation)` and `classify_request` returns Stale for any
id the record does not know. Run the reviewer's own trigger as N17b: dispose G1, accept G2 on the
same basis, dispose G2, restart, then retry both. Require G1's retry to return Stale with no branch,
no entry and no second envelope, and G2's to return the terminal acknowledgement. Judge whether
degrading older acknowledgements to refusal is acceptable (question 16.3) and attack M3b and M10b.

Finding 5's correction changes the archive's cost model: export and the archive are built from the
STRUCTURAL record plus ledger envelopes, so typed reconstruction is attempted and labelled, never
required. That is what makes a preserving disposal available for a non-replayable branch. Confirm
this does not weaken the accepted typed inspection view, and attack N22.

Finding 6 is corrected by retaining the exact verified seed bytes in UnconfirmedStudioSeed, exposing
them only through the accepted scoped callback, and NOT trusting that retention: the detached stage
re-runs UnconfirmedStudioSeed::parse against the candidate receipt before the branch is built, and
again on every restart reconstruction. Verify from provisional.rs that parse's existing
`projection.checkpoint(..).bytes() == bytes` equality is what makes the retained value provable, that
the tail's advance is why recomputation is impossible, and that the memory cost is accounted in 8.3
and L10 rather than assumed. Attack N25b and M27.

Finding 7 is the one whose correction you should attack hardest, because revision 1's argument was
wrong. The false claim that a wrong value produces receipts nobody accepts is withdrawn, on the basis
of `complete_checkpoint_head_scoped`'s `is_some_and`: an Unknown-tenure reader accepts a proof's
claimed tenure. Agreement is now structural. Position gains the committer's leaf identity,
blake3(index, signature_key, credential), and `applied` gains one arm treating a same-owner
contiguous step with a changed leaf identity as a new tenure at after.epoch, which is exactly what
the rejoining device's self-join inference computes. The HPKE encryption_key is deliberately EXCLUDED
so an ordinary committer self-update preserves knowledge; verify that choice against
openmls::group::Member and against group.rs:247-274, which binds a joiner's credential to
(group, invite_nonce). Judge the residual of the residual in 9.3 part 4, a re-add reusing both the
same signature key and the same credential at the same leaf, and whether the commit-builder
membership rule is necessary or the invite ledger's fresh nonce already excludes it (question 16.4).
Attack N-T6b, which now includes a known-tenure witness, an Unknown-tenure newcomer requesting a
fresh proof, a restarted witness and an ordinary self-update, plus M22b and M22c. Also confirm the
inference is now phrased in terms of current continuous membership and is still confined to
ChannelSync::new_joined, never OwnerTenure::unknown and never the restore path, and that the
versioned snapshot tail keeps decode's existing position equality and start > epoch rejections.

Findings 4, 8 and 9 are smaller. For 4, confirm the composite capture takes both destination records
under the SAME single preparation permit in the SAME visit, materializes no projection under custody,
and rechecks both digests and physical sizes at preview completion and at application (N8b). For 8,
confirm the three-state write outcome, refused / uncertain / partial, and that `partial` names the
only multi-step sequence that exists, archive durable with disposal pending. For 9, confirm the
confirmation token is a required field with a typed constructor in both the Rust request and the
native argument list, and that N14 and M5 can actually exercise its absence and a wrong literal.

Also judge the new surface this revision adds, which did not exist at revision 1: the draft archive
record as a second record kind inside the existing Intents family, with its own sealing domain,
budget participation, one-per-document cardinality, 16 MiB vault ceiling inside the existing 64 MiB,
Intents-arm accounting and, critically, its REFERENCE collection. Section 10's R3 now claims that a
preserving disposal keeps every CID the branch named protected through the archive, and that a
destination copy retains only the CIDs it names; revision 1's contrary claim is withdrawn. Verify
R3 against creative_references.rs and the cleanup path, and attack N19 and N19b. Question 16.1 asks
whether this placement is right or whether the archive should be its own inventoried family despite
colliding with Agent 1's I-4 and Agent 3's writers.

Answer the five questions in section 16. Confirm the corrected test plan: M9 must reach the
interruption between two individually valid records rather than a different refusal, M26's fixture
must pass every other structural guard, and the reconciliation mutations must separate document
identity from seed-hash matching. Return PASS for each of (a), (b) and (c) separately, or numbered
findings with severity, file/line, trigger, impact, evidence and required correction, stating which
boundary each belongs to and which revision-1 findings remain open. A PASS accepts design only: no
implementation, no measurement and no native Save exposure is claimed, signed repair and combined
runtime integration are separate, and full Gate 4 acceptance remains with Agent 4.
```
