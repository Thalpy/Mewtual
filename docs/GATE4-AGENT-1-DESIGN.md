# Gate 4 Agent 1: local Save and automatic handoff runtime

Status: **user PASS for this bounded runtime design, 2026-09-15. No production code is written.**
Reviewed `1bcb1bca204d721b848b17c0835faf931ae930e3...25ce89bb705dfc228c7b32874788ebd062e6fcf4`,
revision 4. AG1-TEST-001 is closed at the design boundary; AG1-001 to AG1-005 remain closed at
their previously stated boundaries. No further design change is required and no new finding arose.
The reviewer inspected source and both designs without executing Cargo, tests, mutations or
measurements.

**This PASS accepts the design only.** Code and executed tests must still establish the specified
behaviour, including the exact M5a and M5b failures with byte-exact restoration and passing
restored regressions. It grants no implementation acceptance, no measurement, no native Save
exposure, no acceptance of Agent 3's repair design and no part of full Gate 4.

Design base: `a052f78b62a549702686a8741932f1d2f8c98773`. Accepted head:
`25ce89bb705dfc228c7b32874788ebd062e6fcf4`. Scope is
[Agent 1 of the four handoffs](GATE4-AGENT-HANDOFFS.md); progress is in
[GATE4-AGENT-1-STATUS](GATE4-AGENT-1-STATUS.md).

**Unmet dependency, unchanged.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at
`e65bfd8` is still unreviewed. Section 16 carries the contingency. Agent 2's reviewed manual
lifecycle remains a prerequisite for native Save registration (12.3).

Accepted work this design must not weaken: the Closing-overlay foundation (`b1b0ec9`), the handoff
design (HANDOFF-001), the bounded core/store handoff implementation (`62f06d4`, HANDOFF-002), the
detached-inspection proposal (`0b28f06`) and its read-only implementation (INSPECTION-TEST-001),
and the combined scheduling block (`6b71d96`). No closure is reopened.

## 0. Finding ledger

| Finding | State after the revision-3 review | Where |
|---|---|---|
| AG1-001, acknowledgement must not require media admission | **Closed at the design boundary.** S0 is split into common request validation, classification, and new-authoring-only media admission; S1a is terminal and sync-only. | 6.2, 6.3, 14 N30, M16 |
| AG1-002, cross-visit inventory consistency | **Closed at the design boundary**, subject to I-4's stated implementation audit. | 9.2, R10, 14 N17, M20, M21 |
| AG1-003, protection transfer at persistence | **Closed at the design boundary.** Ordinary protection is established before potentially durable I/O and before transient ownership is released. | 8.3, R7, 14 N12, M14 |
| AG1-004, Prepared as a permanent export prohibition | **Closed** at revision 2. | 12.1 |
| AG1-005, abandoned native handle | **Closed at the design boundary.** Admission depends on actual owners, not an actor-side strong reference awaiting cleanup. | 5.5, 7.1, 14 N14, M3 |
| AG1-TEST-001, masked mutations | **Closed at the design boundary** by the revision-4 review. The positive-signing precondition, the deterministic clock seam, the staged authoritative work and the per-limit preconditions remove the priority-deferral masking path. | 7.3, 14.1 N31, 14.2 M5a/M5b |

Implementation consequences adopted from the revision-3 answers: I-4's guard becomes a **type-level
prerequisite** rather than a convention (9.2); the parked-body escalation must **classify before
invoking** a validator and carries the job's original ownership (9.2); the transient owner is kept
until the write attempt returns, with the rationale recorded (8.3).

## 1. Outcome and boundary

A real actor must durably accept an explicit local overlay operation on an eligible Closing
document, retain and read it across restart, and automatically transfer the complete branch when
the verified eligible successor arrives, while unrelated actor work, authoritative discovery and
receive, and another server keep progressing.

Out of scope: manual inspect, export, copy and disposition, stale-base handling, preview-based
local work and repeated-owner tenure (Agent 2); signed fault repair (Agent 3); integration and
full-gate acceptance (Agent 4).

Native Save is designed behind the actor and store seam but **not registered**. Its
`#[tauri::command]`, registration and security rows land in a separate identifiable commit owned by
Agent 4, gated on section 12.3.

## 2. What was audited

Revisions 1 and 2 audit lists, plus for this revision:
`store/epoch_recovery.rs` `update_epoch_recovery_accounted_with_writer`,
`store/epoch_owner.rs` `update_epoch_owner_state_with_writer`,
`store/epoch_recovery/cleanup.rs` `step_with_io`,
`store/epoch_studio.rs` `enter_studio_budget_scope` and `studio_storage_budget`,
`store/epoch_intents.rs` `write_prepared_intents`,
`store/epoch_recovery/inventory.rs` cache eligibility,
`store/creative_references.rs` `Protection::{unknown, install}` and `ProtectedBlobs::delete`,
`crates/catcoms-replication/src/studio/overlay.rs` `checked_entries` and `encode_vault`,
and `apps/desktop/src-tauri/src/studio/inspection.rs` cancellation between visits.

Facts established by that audit and used below:

- The only rotations of an inventory-relevant token in the store are
  `epoch_intents.rs:479`, `:510`, `epoch_intents/retirement.rs:199`, `:233` (all
  `intent_generation`), `epoch_recovery/cleanup.rs:94` (`studio_generation`) and `:155`
  (`intent_generation`, coverage dependent), and `epoch_studio.rs:168` and `:203`
  (`studio_generation`, on budget **mint** and on budget **entry**).
- `update_epoch_recovery_accounted_with_writer` and `update_epoch_owner_state_with_writer`
  reserve, write and commit real record replacements and rotate **neither** token.
- `enter_studio_budget_scope` rotates `studio_generation` on every entry and hands the new token
  to the active owner, so it is a single-live-budget-owner token, not a file-mutation token.
- `inventory_cache` is consulted only for `Registry` and `Studio`, and only when a scan is not
  collecting references. `Recovery`, `OwnerReceipts` and `Intents` are all uncached.
- `write_prepared_intents` consumes a supplied `old: Option<u64>` and performs no old-record read;
  it does rotate `intent_generation` on both its branches.
- `StudioOverlay::encode_vault` itself calls `checked_entries`.

**No Cargo command was executed and no new measurement exists.** Quoted numbers are the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations.

## 3. Audit observations, corrected

### R1: decoding a retained branch performs a full ordered reconstruction

`StudioOverlay::decode_vault` ends with `out.read(ledger)?`
([overlay.rs:419](../crates/catcoms-replication/src/studio/overlay.rs#L419)), replaying every
accepted operation. `EpochIntentState::decode` reaches it whenever the record carries an Active or
Prepared branch. This is why the profile's `decode_ms` is essentially `draft_ms`.

**Correction carried from revision 2.** The ordinary-read claim is conditional:
`load_studio_epoch`, `checked_studio_source`, `capture_studio_source` and
`studio_source_bytes_match` reach the intent decoder through `check_studio_intent_link` only when
the source wrapper carries the required-metadata link byte, which a source write made while
handoff metadata exists sets. A local Save writes only the intent record. A Completed-only or
floor-only record has no branch to replay. The expensive combination is a linked source plus a
retained Active or Prepared branch.

**Correction in revision 3.** Revision 2 said the Intents family is "the one family
`inventory_cache` does not cover". That is wrong: the cache admits only `Registry` and `Studio`,
and only when the scan is not collecting references, so `Recovery`, `OwnerReceipts` and `Intents`
are all uncached. The point that survives is narrower and still decisive: the Intents arm is the
one whose uncached cost scales with a retained overlay branch, so a single large branch taxes
every five-family scan anywhere in the vault.

What is unconditional: `checked_epoch_replay_state` on every Studio write transaction for that
document, and the Intents arm of every five-family scan. The fixed "about four reconstructions"
count from revision 1 remains withdrawn; the number is path dependent.

### R2: local acceptance reconstructs the whole branch under custody

`StudioOverlay::append` validates by building `staged` and calling `staged.read(ledger)`
([overlay.rs:244](../crates/catcoms-replication/src/studio/overlay.rs#L244)).

### R3: the basis is re-derived on every non-retry Save

`prepare_settlement` checks the actual receipt head, its document, closed epoch and close-record
hash, verifies the receipt against the live group and observed tenure, builds the typed checkpoint
and binds the source version. `EpochOwnerReceiptState::close_for` supplies the exact saved
receipt-to-close binding and is **historical evidence, not fresh owner authority**; the
observed-tenure verification inside `prepare_settlement` remains necessary. Cost is bounded by the
Closing source and seed, and is measured today only on a 1.5 KB fixture.

### R4: replay does not exclude overlay-annotated intent ids

`studio_replay_evidence` filters `own` by author only. The explicit exclusion is **selection
hardening**, not evidence that the ordinary Apply guard permits a bypass; `choose` returns
`NoEvidence` today and that must be preserved.

### R5: the commit's unchanged fence repeats a decode

The "strictly stronger" claim remains **withdrawn**. The existing decoder already requires
canonical re-encoding equality, so the old comparison already covered the complete encoded
contents. C-2's justification is cost only.

### R6: the Prepared fence resolves synchronously

`resolve_studio_handoff_with_io` restores the destination source before classifying evidence on the
rotation, adoption, shared-write and publication fences. Kept as a backstop; the runtime's own path
no longer restores.

### R7: a transient pixel hold does not survive a complete scan

`creative_pinned_cids` calls `Protection::unknown()`, dropping `pins`, then installs the set
derived from durable state; `ProtectedBlobs::delete` consults only the installed set. A hold added
before the scan does not survive it.

**Extended in revision 3 (AG1-003 residual).** The converse also holds and is the newly identified
gap: after a scan has installed a known set that omits a CID, **adding a durable record naming that
CID does not repair the installed set**. `Protection::install` replaces the known set and deletion
never rereads durable metadata, and `write_prepared_intents` performs accounting and persistence,
not reference installation. The existing overlay acceptance writer compensates by calling
`hold_creative` and `hold_creative_operation` before its write; those calls must survive the stage
extraction. Section 8.3 specifies where.

### R8: the inventory scan holds an exclusive store borrow for its whole traversal

`EpochStorageScan<'_>` borrows `&mut ServerStore`; its one-body-per-step rail bounds memory and
per-step work, not custody.

### R9: the two reference mechanisms are distinct

`check_handoff_references` checks candidate plus pending coverage of the branch's base CIDs, in
memory, at the commit. HANDOFF-002's authenticated inventory separately collects required
source-to-intent metadata targets and refuses to install deletion protection when that dependency
is unsatisfied. Both survive and are tested separately.

### R10 (new, AG1-002): no existing token is a vault mutation generation

`intent_generation` covers intent writes and retirement. `studio_generation` rotates on Studio
budget **mint and entry**, which makes it both too broad for a parked cursor (any unrelated Studio
budget entry would invalidate it) and too narrow for correctness (the accounted recovery and owner
writers replace real records and rotate nothing). Removing the scanner's exclusive borrow therefore
requires a new invalidation contract, not a reuse of these two. C-3 supplies it.

## 4. Design principles

1. **One algorithm, two drivers.** The existing synchronous entry points become the inline
   composition of the same stage functions the runtime schedules.
2. **Custody is spent on evidence, not on computation.** Under the lease the runtime reads,
   authenticates, hashes, structurally decodes, checks live authority, accounts and writes. Full
   decode, reconstruction, private candidate restoration, typed admission and manifest assembly run
   on a detached worker. Where a verified value already exists in memory, the commit proves it
   against the actual persisted bytes rather than recomputing it.
3. **Every detached result is a proposal.** It becomes durable only after a custody visit
   reauthenticates the exact bytes it was derived from and rechecks live authority.
4. **Admission is explicit and drop-safe.** Ownership is proved by a live `Arc`, never by a
   bookkeeping flag that some path must remember to clear.
5. **Acknowledgement is not authoring.** Recognising an accepted request authenticates target,
   author, envelope and original basis, and requires nothing that only a new acceptance needs:
   no fresh basis, no tenure, no source lookup, and no media possession.
6. **Protection is transferred, never merely released.** A job-owned reference hold may be dropped
   only once ordinary conservative protection covers the same references.
7. **Refusal retains work.**

## 5. Concrete APIs

New leaf modules owned by Agent 1:

```
crates/catcoms-replication/src/studio/overlay/structural.rs   (C-1 decoder split)
crates/catcoms-app/src/store/epoch_studio/overlay_capture.rs  (capture, stamp, plan)
crates/catcoms-app/src/store/epoch_studio/overlay_commit.rs   (staged commit seams)
crates/catcoms-app/src/studio/overlay/runtime.rs              (job state machine, admission)
crates/catcoms-app/src/studio/overlay/save.rs                 (native multi-visit local Save)
apps/desktop/src-tauri/src/studio/overlay.rs                  (unregistered native surface)
```

### 5.1 C-1: structural decode, separate from reconstruction

```rust
impl StudioOverlay {
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
}
impl StudioOverlayState { /* the same pair */ }
```

`decode_vault_structural` keeps, in order: the `MAX_EXTENSION` bound, the version tag, the nested
seed and metadata bounds computed before allocation, the complete target derivation with its
receipt and ledger scope equality, `checked_entries`, the Prepared and Completed field decoding
with their bounds and target equality, and `encode_vault(ledger)? == bytes`. It drops only
`out.read(ledger)?`.

The moved call sites are **branch-replay-free**, not free of all graph work: the reference path
deliberately retains `base_blob_cids`, which calls `base.graph()` and performs typed seed work.
The source-write and completion paths still need their separate signed-source evidence.

The validation boundary change rests on a writer obligation, not on sealing. AEAD authentication
under the local database key proves origin and integrity; it is **not** proof of typed admission,
and not every `write_prepared_intents` call follows a fresh `append`.

> **I-1.** First acceptance of an overlay entry fully validates the branch through
> `StudioOverlay::append`, which reconstructs it. Every subsequent writer preserves the
> already-checked identity and evidence rather than re-deriving them: ordinary intent writes do not
> alter the overlay extension, handoff transitions move only between the Active, Prepared and
> Completed forms derived from an already-validated state, and retirement does not remove an entry
> an annotation still requires.

Structural decode mints no authority. Full reconstruction stays mandatory before display, append,
handoff preparation or export. A record that is authenticated, canonical and structurally
consistent but not typed-replayable is accepted by metadata readers and rejected on the detached
reconstruction; the branch is then held and surfaced through the manual lifecycle. That is a real
change to the validation boundary, not equivalent validation.

Call sites moved to structural decoding: the inventory scan's Intents arm (reference scans keep
`base_blob_cids` and its typed seed work), `check_studio_intent_link`, `checked_epoch_replay_state`,
the callers that read the previous record's size, `check_studio_handoff_write`,
`check_studio_handoff_publication`, `epoch_intents/retirement.rs`, the runtime's admission,
acknowledgement classification and Flow H authority capture, and
`studio_replay_evidence`'s overlay exclusion. Call sites keeping full `decode_vault`, all detached:
`EpochIntentState::local_draft`, the runtime's detached plan stage, and Agent 2's export and copy
preparation.

### 5.2 C-2, C-3, C-4

- **C-2.** The **callers** that read the previous intent record only to obtain its physical size
  (`persist_handoff_intents`, `write_studio_overlay_intent`, `save_studio_closing_overlay_with_io`)
  use a bytes-and-size read (`read_scoped_intent_plain`) instead of a decode, and the "unchanged"
  fence compares the complete authenticated plaintext digest and physical size.
  `write_prepared_intents` itself already consumes a supplied `old` and is unchanged apart from
  receiving that value from the cheaper read. Justification is cost, not strength (R5).
- **C-3.** A vault inventory generation plus a resumable cursor, section 9.2.
- **C-4.** Bounded job-owned transient reference holds **and their transfer rule**, section 8.

### 5.3 Store: capture, stamp and plan

```rust
pub(crate) struct StudioOverlayStamp {
    mount: Arc<()>, server: u64, document: LogicalDocument, target: StudioTarget,
    actor: DeviceId, actor_key: Vec<u8>, owner: DeviceId, mls: u64,
    incarnation: RegistrySyncInstance,
    /// Captured for information. A KNOWN tenure is required only by the authoring stages.
    tenure: Option<u64>,
    intent: Option<(blake3::Hash, u64)>,   // absence is explicit
    source: Option<(blake3::Hash, u64)>,
}

pub(crate) struct StudioOverlayCapture {
    stamp: StudioOverlayStamp, group: Vec<u8>,
    intent: Option<Zeroizing<Vec<u8>>>, source: Option<Zeroizing<Vec<u8>>>,
    work: StudioOverlayWork,
}

/// Only the stages that create new durable authoring work carry authority-bearing or
/// media-bearing inputs. Acknowledgement never constructs one of these (AG1-001).
pub(crate) enum StudioOverlayWork {
    /// Classification already proved this is not an acknowledgement, the caller already
    /// re-derived and matched the Closing basis, and media admission already succeeded.
    SaveAppend { basis: StudioClosingOverlayBasis, intent: LocalIntent, ts: u64,
                 pixels: Option<CreativeHold> },
    Handoff { authority: StudioHandoffAuthority, basis: [u8; 32] },
    /// Membership only. No new-edit tenure, no basis, no media.
    Resolve,
}

pub(crate) struct StudioOverlayPlan { stamp: StudioOverlayStamp, outcome: StudioOverlayPlanned }
pub(crate) enum StudioOverlayPlanned {
    SaveAppended { state: Box<EpochIntentState>, accepted: usize },
    HandoffPrepared { signing: Box<StudioHandoffSigning>, facts: Box<HandoffFacts> },
    Resolved { source: Box<StudioEpoch>, evidence: StudioHandoffEvidence,
               next: Box<EpochIntentState> },
    Hold(StudioOverlayHold),
}
pub(crate) enum StudioOverlayHold {
    NoOverlay, NotClosing, WrongAuthor, WrongTarget, BasisChanged, StaleRequest,
    PreparedPending, SuccessorNotPristine, SuccessorMissing, OrdinaryIntentCollision,
    TenureUnknown, PixelMissing, ReferenceCapacity, InventoryUnstable, Structural(String),
}

impl ServerStore {
    /// Caller owns the per-actor admission token and one shared preparation permit and has live
    /// membership custody. Bounded reads with the ordinary parent-directory and regular-file
    /// restrictions. Does not decode and does not evict `self.studio_source`.
    pub(crate) fn capture_studio_overlay(
        &self, context: &StudioOverlayContext, work: StudioOverlayWork,
    ) -> Result<StudioOverlayCapture, AppError>;

    /// Compares mount pointer, numeric server, complete target, document, actor, actor key,
    /// owner, MLS epoch, and BOTH full wrapper digests with physical sizes. Captured absence must
    /// remain absence. Tenure equality is required only for `SaveAppend` and `Handoff`.
    pub(crate) fn studio_overlay_is_current(
        &self, context: &StudioOverlayContext, stamp: &StudioOverlayStamp, require_tenure: bool,
    ) -> Result<bool, AppError>;

    /// Cheap structural read for admission, acknowledgement classification and authority capture.
    /// Reads and authenticates one bounded record; performs no reconstruction.
    pub(crate) fn studio_overlay_structural(
        &self, server: u64, document: &LogicalDocument,
    ) -> Result<Option<EpochIntentState>, AppError>;
}

impl StudioOverlayCapture {
    /// Blocking worker only. Owns authenticated plaintext, public context, the permit, the
    /// admission token and any transient pixel hold.
    pub(crate) fn plan(self) -> Result<StudioOverlayPlan, AppError>;
}
```

### 5.4 Store: commit seams

```rust
impl ServerStore {
    /// One accounted intent write behind stamp equality. Used by Flow S, by acknowledgement and by
    /// Agent 2's disposition and copy bookkeeping. Retires nothing and prunes nothing.
    pub(crate) fn commit_studio_overlay_state(
        &mut self, context: &StudioOverlayContext, stamp: &StudioOverlayStamp,
        next: EpochIntentState, unchanged: bool, budget: &mut EpochStudioBudget,
        rng: &mut impl CryptoRngCore,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError>;

    /// Durable state only: a Closing source, its receipt head, and the matching signed close from
    /// the saved owner journal. `prepare_settlement` supplies the live owner and tenure check.
    pub(crate) fn studio_closing_basis(
        &mut self, server: u64, group: &ServerGroup, target: StudioTarget, device: &MlsDevice,
        tenure: Option<u64>, budget: &mut EpochStudioBudget,
    ) -> Result<Option<StudioClosingOverlayBasis>, AppError>;
}

/// Privately constructible ONLY by the post-write comparison in 9.1. Binds the mount, numeric
/// server, complete target, the record's authenticated plaintext digest and its physical size.
pub(crate) struct VerifiedPersistedSource { /* no public constructor */ }
```

### 5.5 App: drop-safe admission and the job state machine

AG1-005's residual was that `live: Option<Arc<()>>` needed an explicit transition that a dropped
native handle never performs. Revision 3 removes the transition entirely: the bookkeeping holds
only `Weak` handles, so release is whatever happens when the last `Arc` drops, wherever it lives.

```rust
pub(in crate::studio) struct OverlayAdmission {
    /// At most one entry, because `admit` pushes only when reaping left this empty.
    owners: Vec<std::sync::Weak<()>>,
}
impl OverlayAdmission {
    fn can_admit(&mut self) -> bool {
        self.owners.retain(|w| w.strong_count() != 0);
        self.owners.is_empty()
    }
    /// Minted once per job and cloned into every stage, worker, result and native handle.
    fn admit(&mut self) -> Option<Arc<()>> {
        if !self.can_admit() { return None; }
        let token = Arc::new(());
        self.owners.push(Arc::downgrade(&token));
        Some(token)
    }
}
```

> **I-2.** At most one overlay job per actor is live, where live means at least one `Arc` clone of
> its admission token still exists: in the tracked job, in a running detached worker whose waiter
> was cancelled, in a retained result, or in a native preparation handle. A new job for any target,
> and a native Save preparation, are refused until every clone has dropped. No path must remember
> to release, and no release depends on an awaited native call.

Per-target backoff is pacing and is not part of I-2.

```rust
pub(in crate::studio) struct OverlayRuntime {
    admission: OverlayAdmission,
    job: Option<OverlayJob>,
    next_attempt_at: BTreeMap<StudioTarget, u64>,
    backoff: BTreeMap<StudioTarget, u64>,
    /// Negative memo keyed on the store's intent generation.
    no_overlay: BTreeMap<StudioTarget, std::sync::Weak<()>>,
    /// Parked inventory cursor for the commit visit sequence (9.2).
    inventory: Option<EpochStorageCursor>,
    inventory_restarts: usize,
    selection: usize,
    notices: SettlementNotices,
}
struct OverlayJob {
    context: OverlayJobContext,
    ownership: OverlayOwnership,
    stage: OverlayStage,
}
/// Everything a worker, a result or a native handle must outlive its waiter to own. Dropping this
/// releases the admission, the shared permit and any job-owned reference hold.
pub(crate) struct OverlayOwnership {
    admission: Arc<()>, permit: OwnedSemaphorePermit, pixels: Option<CreativeHold>,
}
enum OverlayStage {
    Captured(Box<StudioOverlayCapture>),
    Signing(Box<StudioHandoffSigning>, Box<HandoffFacts>),
    Assembling,
    Assembled(Box<StudioHandoffCommit>),
    Planned(Box<StudioOverlayPlan>),
}
pub(crate) enum StudioBackgroundJob<T: MeshTransport> {
    // ... existing ...
    OverlayPlan(Box<StudioOverlayCapture>, OverlayOwnership, OverlayJobContext),
    OverlayAssemble(Box<StudioHandoffSigning>, Box<HandoffFacts>, OverlayOwnership, OverlayJobContext),
}
pub(crate) enum StudioBackgroundResult {
    // ... existing ...
    OverlayPlanned(OverlayJobContext, Result<(Box<StudioOverlayPlan>, OverlayOwnership), AppError>),
    OverlayAssembled(OverlayJobContext, Result<(Box<StudioHandoffCommit>, OverlayOwnership), AppError>),
    /// The waiter was cancelled. The runtime drops its tracked job; the still-running closure
    /// holds the last Arc, so admission stays unavailable until it finishes. No token is passed.
    OverlayCancelled(OverlayJobContext),
}
```

`StudioOverlaySavePreparation` and `StudioPreparedOverlaySave` own an `OverlayOwnership` exactly as
`StudioInspectionPreparation` owns its permit, so native dropping either handle, cancelling between
visits, or never issuing the second visit releases admission with no actor round trip.

### 5.6 App: control requests and responses

```rust
pub enum StudioControlAction {
    // ... existing ...
    /// Derive and return the current Closing basis fingerprint. Durable state only.
    BeginOverlaySave,
    /// First Save visit: common validation and acknowledgement classification. Media admission
    /// happens only if this turns out to be new authoring (AG1-001).
    PrepareOverlaySave { basis: [u8; 32], nonce: [u8; 16], body: Vec<u8> },
    FinishOverlaySave(Box<StudioPreparedOverlaySave>),
}
pub enum StudioControlResponse {
    // ... existing ...
    OverlaySaveBasis { target: StudioTarget, basis: [u8; 32], accepted: usize },
    OverlayAcknowledged(StudioOverlayAcknowledgement),
    OverlaySavePreparation(StudioOverlaySavePreparation),
    OverlaySaved { target: StudioTarget, basis: [u8; 32], accepted: usize },
}
pub enum StudioOverlayAcknowledgement {
    LocalDraft { target: StudioTarget, basis: [u8; 32], accepted: usize },
    /// Records a prior LOCAL transfer outcome only.
    Handoff { target: StudioTarget, outcome: StudioHandoffOutcome },
}
```

No Save result carries a projection; the caller refreshes with `studio_overlay_read`, which already
owns the detached reconstruction and the 32 MiB conversion bound.

### 5.7 Native surface (designed, not registered)

```ts
studio_overlay_begin({ server, channel, object? })
  -> { v: 1; kind: "eligible"; basis: string; accepted: number }
   | { v: 1; kind: "ineligible"; reason: string }

studio_overlay_save({ server, channel, object?, basis, nonce, body })
  -> { v: 1; kind: "local-draft"; channel; object; basis; accepted: number; alreadySaved: boolean }
   | { v: 1; kind: "acknowledged-handoff"; channel; object; basis; epoch: string; epochId: string;
       accepted: number }
```

- `basis` is required and is the original authoring identifier, from `studio_overlay_begin` for a
  new branch or from `studio_overlay_read` for an existing one. The caller keeps
  `(basis, nonce, body)` byte-stable across retries. It is an identifier, not authority.
- No timestamp field; the actor supplies it and it is outside envelope identity.
- An unmatched request whose `basis` is not the currently eligible one returns **stale**, never new
  work. `check_basis_floor` is the independent second fence after a rewind.
- `acknowledged-handoff` records that this exact request was previously transferred into the named
  local destination epoch. It is not delivery, receipt or settlement, and after legitimate
  retirement it does not assert that those operations are still in the current signed source or the
  pending ledger. **It also does not assert that the referenced pixels are still held** (AG1-001).
- `studio_overlay_read` gains `transferState: "completed"`; `kind: "absent"` keeps its meaning.

Both commands use the existing `InvokeContext`: one UI session generation, one actor instance, one
native operation slot, one `ViewRequest` per `(state, server, target)` and one
`RequestCancellation` spanning every custody visit.

## 6. The pipeline

### 6.1 Automatic handoff (Flow H)

| # | Stage | Custody | Work | Ownership |
|---|---|---|---|---|
| H1 | Admit, classify, authorize, capture | yes | admission + permit; bounded intent read; structural decode; eligibility probe; `handoff_authority(device, group, tenure)`; bounded source read; stamp | acquires |
| H2 | Plan | detached | full `decode_vault`; private successor restore via `prepare_vault_source`; `prepare_handoff_detached` | worker |
| H3 | Sign slice, repeated | yes | per-visit wrapper reauthentication, then bounded `sign_next` turns | held |
| H4 | Assemble | detached | `finish()`; encode the Prepared and Completed candidate records | worker |
| H5 | Commit | yes | inventory, rechecks, Prepared, Source, verified flush, Completed | held |
| H6 | Notify | yes | settlement notice, `StudioUpdated`, watch rebinding | releases |

The authority is minted at H1 from the structurally decoded branch receipt under live custody,
through the existing narrow interface, and `receipt.verify_current_owner` runs against the live
group and observed tenure. H2 independently performs the full decode from the same captured
plaintext, and `prepare_handoff_detached` rejects unless target, author and receipt match that
fully decoded state.

**Freshness, stated precisely.** Earlier capture can become stale while H2 runs; this placement is
not strictly fresher in every respect. What it guarantees is that a stale authority cannot
authorise a signature or a commit: every signing visit reauthenticates both wrappers and rechecks
live device, key, membership, MLS epoch, receipt owner and observed tenure, and H5 repeats all of
it. The reviewer accepted this placement on exactly those subsequent checks.

### 6.2 Local Save (Flow S), with media admission behind classification

AG1-001's residual was that S0 validated and held pixels before knowing whether the request was an
acknowledgement. S0 is split into three parts, and only the third does media work.

| # | Stage | Custody | Work | Ownership |
|---|---|---|---|---|
| S0 | Common request validation | yes | channel known; current membership; complete target; bounded canonical grammar; typed decode; envelope construction. **No blob read, promotion, hold or possession check.** Then admission + permit, before any body read | acquires |
| S1 | Classify | yes | bounded intent read; structural decode; `completed_retry`, then `exact_retry`, then the ordinary-collision check, all against target, author, exact envelope and the request's original basis | held |
| S1a | Acknowledge, terminal | yes | flush-only accounted write; return `OverlayAcknowledged`. **No basis minting, no tenure, no source lookup, no PIX read, promotion, hold or possession check** | releases |
| S1b | Authorize and admit media | yes | PIX validation, promotion and the transient hold (8.1); `studio_closing_basis`; require `fresh.fingerprint() == request.basis`, else **stale**; bounded source read; stamp | held |
| S2 | Plan | detached | full `decode_vault`; `append` (reconstruction) producing the next state | worker |
| S3 | Commit | yes | stamp equality; basis re-mint and fingerprint equality; PIX possession revalidation; **protection transfer (8.3)**; one accounted intent write; release the transient owner | releases |

`completed_retry`, `exact_retry` and the ordinary-collision check need only entry ids, envelopes,
`basis()`, author, target and the ledger, all of which the structural decode yields, so S1 is cheap.
Nothing in S0, S1 or S1a requires a known observed tenure, an Open or Closing source, the owner
journal, or possession of any referenced blob. A frame operation that was accepted and handed off is
therefore still acknowledgeable after its pixels have been legitimately reclaimed, which is exactly
what the `acknowledged-handoff` wording already promises.

S3 re-mints the basis under the same custody as the write, preserving the accepted rule "recheck the
same source and authority immediately before the first durable acceptance".

### 6.3 Acknowledgement after rollover

1. `completed_retry` finds no entry for the id and `exact_retry` is false: not an acknowledgement.
2. S1b derives the eligible basis and compares it with the request basis; they differ, so **stale**.
   No media admission, no detached work, no new acceptance.
3. If a rewind ever reproduced an equal fingerprint, `check_basis_floor` against the persisted
   `minimum_new_basis_closed_epoch` rejects it in the detached plan, before any write.

No nonce tombstone store, nothing unbounded retained.

### 6.4 Interrupted Prepared resolution (Flow R)

R1 capture (membership only), R2 detached restore plus `evidence` and the next state, R3 commit
under stamp equality. The synchronous resolver at the rotation, adoption, shared-write and
publication fences is unchanged as a backstop.

### 6.5 Custody-visit sources

Any `StudioReceiver::run` pass: the native receive driver (paced at one second while
`studio_pending` is true) and any explicit Studio document or control request. No change to
`drive_receiver`'s pacing is proposed; its contribution is reported separately from maximum
continuous custody (section 13).

## 7. Admission, scheduling and fairness

### 7.1 Drop-safe admission

Section 5.5 replaces revision 2's `live`/`draining` pair with weak-handle bookkeeping. The
consequences that matter:

- A background waiter cancelled mid-detach leaves the blocking closure holding the last `Arc`;
  admission stays unavailable until that closure finishes, with no `drain` call required.
- A native `PrepareOverlaySave` handle that native cancels, drops, or never follows with
  `FinishOverlaySave` releases admission when the handle drops, with no awaited actor call and no
  second visit.
- A retained result or delivery guard that still owns a clone keeps admission unavailable, so
  normal completion cannot clear it prematurely.
- `owners` holds at most one entry, because `admit` pushes only when reaping left it empty.

### 7.2 Reservation precedes every body read

Admission and the shared permit are acquired before the first bounded read in both flows, and are
released immediately when the probe or classification finds no work to schedule. A target with no
overlay is memoised in `no_overlay` against the store's `intent_generation`, so a quiescent vault is
not re-probed each turn.

### 7.3 Scheduling

- **Placement.** Heavy stages run only when `catchup.replay_ready()` holds, mirroring
  `replay_step`'s gate. Signing slices need no new permit and no retained source and may run on any
  background turn, but yield immediately if `server.sync.has_epoch_service_interest()`, any watch
  has inbound, or a background result is parked.
- **Bounded slice.** `MAX_SIGNING_TURNS_PER_VISIT = 32` and `SIGNING_SLICE_BUDGET_MS = 250`,
  whichever comes first. These are an experiment configuration, not a responsiveness guarantee: the
  deadline is checked between signatures and can overrun by one whole operation including its
  authority checks, so they must be recalibrated against the measured largest admitted individual
  operation and roster shape. The slice bound constrains H3 only.
- **Two distinct events.** The priority yield and the slice bound are different outcomes and must
  stay separable in observation as well as in code. The priority yield returns having signed
  **zero** operations; the slice bound returns having signed **at least one and fewer than all**.
  A visit that returns with work remaining proves nothing on its own, because it may have deferred
  before signing. The runtime therefore records, per visit, the remaining count at slice entry and
  at slice exit; `remaining_before - remaining_after` is the number of production `sign_next` calls
  that visit made, since the core decrements exactly one per call. N31 and M5 rest on that
  distinction, and it is the AG1-TEST-001 correction.
- **Injected clock.** The slice budget is measured on `server.runtime_clock().monotonic_ms()`, the
  same injected `catcoms_rt::Clock` the rest of the receiver uses, never `SystemClock`. This gives
  N31 a deterministic seam instead of hoping cheap and expensive operations fall on opposite sides
  of a wall-clock threshold.
- **Slice exclusivity.** A signing slice runs entirely inside the blocking worker that owns the
  moved `Server` and the vault lease: no `await`, no reentrant store operation, no callback that can
  reach the store. This is the condition attached to per-visit wrapper reauthentication (N9), and
  it also constrains how N31 may stage authoritative work: the work is queued through the sync and
  transport side after the slice has been selected, never by reaching into the store from inside it.
- **Coalescing and pacing.** One target at a time, round-robin from the watch rail; a hold sets
  `now + 30_000` doubling to 300,000 ms, reset on durable progress; `explicit_retry` may lower the
  delay but not clear the counter.
- **Honest scope.** Idle-only scheduling preserves authoritative priority and does not promise
  starvation-free overlay completion under sustained catch-up (L7).

## 8. Reference protection across detachment (C-4)

### 8.1 Transient holds

```rust
pub(super) struct Protection {
    generation: Arc<()>,
    pins: Option<CreativeReferences>,
    /// Job-owned holds. NOT cleared by `unknown()`, NOT subtractable by a complete scan, and
    /// consulted by every deletion. Dead owners are reaped on each hold and each delete.
    transient: Vec<(std::sync::Weak<()>, Vec<u8>, BTreeSet<Cid>)>,
}
/// Dropping this releases the hold. Moved through every detached stage and result.
pub(crate) struct CreativeHold { owner: Arc<()>, protection: SharedProtection }

impl ServerStore {
    /// Bounds are checked BEFORE any entry is installed, so exhaustion never leaves a partial
    /// hold. Live owners are counted through cancellation. Transient CIDs are accounted together
    /// with durable ones against MAX_CREATIVE_REFERENCES, without hiding duplicate entries.
    pub(crate) fn hold_creative_transient(
        &self, group: &[u8], cids: BTreeSet<[u8; 32]>,
    ) -> Result<CreativeHold, AppError>;
}
```

`ProtectedBlobs::delete` returns retained when the CID is in `pins` or in any live `transient` entry
for that group, keeping the mutex guard through unlink. `unknown()` and `install()` leave
`transient` untouched; a complete scan may subtract only from `pins`.

### 8.2 Exhaustion refuses admission without disturbing protection

Per the reviewer's answer to revision-2 question 3: exhausting the owner or CID allowance **refuses
new transient admission** (`ReferenceCapacity`) and must not set the whole store unknown, because a
caller must not be able to disable unrelated reclamation. This applies to new admission only. It
does **not** prohibit fail-closed unknown protection after uncertain durable I/O, or when already
retained references cannot be represented: once bytes may have been accepted, preserving an
incomplete known set would be unsafe. Section 8.3 respects that distinction.

### 8.3 The protection transfer (AG1-003, closed at the design boundary)

R7's extension is that a durable write does not repair an installed known set that omits the new
CID. The transient owner may therefore be dropped only after ordinary conservative protection covers
the same references.

> **I-3.** A `CreativeHold` may be released only after, in the same custody visit and before the
> intent write, the ordinary conservative holds for the same references have been installed:
> `hold_creative(server_id, overlay.base_blob_cids())` and
> `hold_creative_operation(document, &intent.operation)`, exactly as the existing
> `write_studio_overlay_intent` does today.

Why that is sufficient, from the actual mechanism:

- `hold_creative` rotates `Protection.generation` first, so any scan already in progress fails its
  `install` generation check and cannot replace the known set with one that omits the new CID.
- With `pins == Some`, the CIDs are added to the known set, so deletion is refused by the ordinary
  mechanism from that moment.
- If `pins.add` exceeds the rail, or `pins` is already `None`, protection becomes or stays unknown
  and every deletion is refused. That is the fail-closed case section 8.2 explicitly preserves.

S3 order: stamp equality, basis re-mint, PIX possession revalidation, **I-3's ordinary holds**, the
accounted intent write, then release the transient owner **after the write attempt returns,
whatever its outcome**. Uncertain persistence is covered because I-3 runs before the write: a rename
that may have succeeded, a failure after a temporary sibling, or a dropped worker all leave the new
references protected by the ordinary mechanism rather than by an obsolete known set.

Flow H needs no equivalent: it introduces no new CIDs, a retained branch's base and pending CIDs are
already enumerated by the inventory's Intents arm, and `check_handoff_references` independently
refuses a commit that would release a base reference (R9).

**Why the owner is kept until the write returns rather than released earlier.** Once I-3's ordinary
holds are installed, immediate release would already be safe within the same uninterrupted custody
interval: the known set contains the CID, or protection is fail-closed unknown, and the protection
generation has invalidated older installations. Keeping the owner through the attempt is chosen
anyway because it gives one ownership boundary across success, error and unwinding for little extra
complexity, and because the durable guarantee must come from I-3's pre-I/O transfer rather than from
assuming a successful return. The premature-release optimisation is deliberately not part of this
checkpoint.

## 9. Durable commit with bounded custody

### 9.1 No graph restore on the commit path

- **Before the Source write.** H2 returns `HandoffFacts { source_snapshot_digest,
  source_physical_bytes, storage_protocol_bytes, before_snapshot }`; H5 accepts them only behind
  exact authenticated wrapper stamps. A stamp mismatch discards the candidate and never falls back
  to a restore inside the barrier.
- **After the Source write.** `save_studio_source_checked` already returns a `SourceVersion` whose
  digest is the complete authenticated plaintext hash of exactly what was written. H5 re-reads the
  persisted record and requires that digest and physical size to match, which authenticates what
  actually landed rather than trusting a worker. Only that comparison may construct a
  `VerifiedPersistedSource`, which binds the mount, numeric server, complete target and version and
  has no public constructor. Because the persisted bytes are proved identical to the retained
  candidate, `evidence`, `blob_cids` and `complete` are computed from it with no second restore.
- **On mismatch.** Retain the durable Prepared hold and resolve from actual bytes through Flow R or
  the synchronous fence. Never complete from the candidate.
- **One algorithm.** `resolve_studio_handoff_with_io` takes `Option<VerifiedPersistedSource>`:
  `None` restores from disk (restart and fence path, unchanged), `Some` supplies the verified
  candidate. The decision table, barriers, accounting generations, write fences and publication hold
  are identical in both.

### 9.2 C-3: a vault inventory generation and a resumable cursor

R10 shows that neither existing token can serve. C-3 therefore introduces one.

```rust
// ServerStore
inventory_generation: Arc<()>,
```

> **I-4.** `inventory_generation` is rotated before **any** operation that can change, create,
> replace, rename, unlink or leave a temporary sibling of a file in the five inventoried families,
> and before the operation's first possible I/O rather than after its success. Rotation is
> monotonic and never restores a previous token, so an over-rotation costs a rescan and an
> under-rotation is the only unsafe direction.

Enforcement is a choke point **backed by** an audited writer list; the revision-3 answer is that
these are complementary, not alternatives, and that a helper callers are merely encouraged to invoke
would recreate a forgettable convention. The guard is therefore a **type-level prerequisite**, not a
call the writer must remember: the five-family mutation primitives take it, so a write, rename,
sync-repair or unlink on those paths cannot be expressed without one.

```rust
/// Obtainable only from `ServerStore::epoch_mutation_guard()`, which rotates
/// `inventory_generation` first. Rotation is not undone when this drops.
pub(in crate::store) struct EpochMutation<'a> { /* private */ }
impl EpochMutation<'_> {
    pub(in crate::store) fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), AppError>;
    pub(in crate::store) fn sync(&self, path: &Path, bytes: u64) -> Result<(), AppError>;
    pub(in crate::store) fn remove(&self, path: &Path) -> Result<(), AppError>;
}
```

The bare `atomic_write`, `sync_*` and unlink helpers stop being reachable for five-family paths, so
a bypass is a compile error rather than a missing convention. A failed or panicking write still
leaves the token rotated, because rotation happens in `epoch_mutation_guard` before the guard is
handed out and is never restored.

The audited writer list then proves coverage, because a correct central guard is insufficient if one
writer reaches disk another way. Known participants that rotate nothing today and must:
`update_epoch_recovery_accounted_with_writer`, the eviction settle path,
`update_epoch_owner_state_with_writer`, every accounted `epoch_registry` writer, the Studio source
writer and sealing, rotation and adoption writers, receive, `epoch_recovery/cleanup.rs`'s unlink
steps, the injected-failure writer seams used by tests, and any raw or tooling adapter. N17 and M20
keep per-family evidence for exactly this reason.

Agent 2's manual-lifecycle archive writers join this list on the same terms:
`write_studio_draft_archive_with_io` and `release_studio_draft_archive_with_io`. They write and
unlink `EpochRecordKind::DraftArchive` records, which live in the Studio family's directory and are
therefore inventoried like every other five-family file. The enum discriminant lands ahead of I-4
as an isolated seam commit with no guard and no writer, so the audit obligation attaches to the
writers when Agent 2 builds them, not to the discriminant; recording them here rather than only in
that commit's message is what keeps the audited list the single place coverage is proved.

A sync-repair is included even though it changes no bytes: the existing code already treats an
unchanged-file flush attempt as invalidating a captured inventory, and over-rotation is the safe
direction under I-4. Conversely, a budget mint or entry alone must **not** rotate, which is what
keeps I-4 separate from budget ownership; and an operation that may have changed files must rotate
even when it returns an error.

`studio_generation` is **not** used, for two independent reasons established in R10: it rotates on
every Studio budget entry, which would make a parked cursor die on unrelated Studio activity (the
self-invalidation hazard), and it is not rotated by the accounted recovery and owner writers, which
would make it unsound. `intent_generation` remains what it is and is unaffected.

```rust
pub struct EpochStorageCursor { /* directory position, accumulated inventory, progress,
   byte and record rails, coverage, mount identity, poisoning, the captured
   inventory_generation, and at most one parked authenticated record body */ }
impl ServerStore {
    pub fn begin_epoch_storage_scan(&mut self, coverage: EpochInventoryCoverage,
                                    references: bool) -> Result<EpochStorageCursor, AppError>;
    /// Checks invalidation FIRST, before resuming any expensive work, then runs bounded steps
    /// until `steps` or `budget_ms` is reached.
    pub fn step_epoch_storage_scan(&mut self, cursor: &mut EpochStorageCursor,
                                   steps: usize, budget_ms: u64)
        -> Result<EpochStorageScanProgress, AppError>;
    pub fn finish_epoch_storage_scan(&mut self, cursor: EpochStorageCursor)
        -> Result<EpochStorageInventory, AppError>;
}
```

Preserved unchanged: mount identity, coverage, scan poisoning, cardinality and byte rails,
one-body-per-step, complete target matching, the reference-scan cache bypass, and the separate
`Protection.generation` rule that a reference scan's `install` must satisfy. Invalidation is checked
both before resuming expensive work and before issuing the final inventory.

**Step granularity is not a custody bound.** A single cold `Registry` or `Studio` record's typed
validation can dominate a step, so a step cap alone cannot promise responsiveness. The cursor
therefore takes a time budget as well, and when one record's validation would exceed it the cursor
parks that record's already-read authenticated plaintext and yields; the next detached stage runs
the pure validation (`inventory_record`, `inventory_references`, `validate_vault_snapshot` are
functions over plaintext, needing no `Server`, device key or MLS secret) and the following visit
installs the result.

Three consequences, from the revision-3 answer, that the implementation must honour:

1. **Classify before invoking, never measure afterwards.** `budget_ms` is not a preemption
   mechanism: checking elapsed time after a long synchronous validator has returned does not enforce
   the boundary. The cursor decides to detach **before** calling a validator whose cost it cannot
   conservatively bound within the remaining budget, using the record's family, its authenticated
   physical size and whether the scan is collecting references, against a conservative threshold
   derived from measurement 13.7. Where cost cannot be conservatively classified, the default is to
   detach.
2. **The parked body carries the job's original ownership.** It and its validation worker and result
   hold the same `OverlayOwnership`: no second overlay pool, no capacity released when only the
   waiter is cancelled. The returned validation is bound to the original cursor identity, mount,
   record id and `inventory_generation`, and all four are rechecked before it is consumed.
3. **Parking bypasses no bound.** A genuine per-family or aggregate size-limit violation still
   refuses; scan poisoning, the cardinality and byte rails and the reference-cache exclusion are
   unchanged. At most one parked body exists at a time, which is the existing one-body-per-step
   rail, and its bytes are accounted in section 13.4.

This accepts a scheduling boundary, not a measured custody-time or heap ceiling; N18 and the
maximal-record measurement in 13.7 remain required.

**Restart and quiescence.** The runtime restarts an invalidated scan at most
`MAX_INVENTORY_RESTARTS = 3` times per commit attempt, then returns `InventoryUnstable` and applies
backoff. It does **not** fall back to a single-visit unbounded scan. A commit completes when no
`inventory_generation` rotation occurs for the duration of one scan. Because the token rotates only
on durable five-family mutation, and specifically **not** on budget mint or entry, on reads, or on
the runtime's own bookkeeping, a vault with no writes in progress satisfies that condition; the
design does not self-invalidate.

Changing the scanner from an exclusive borrow to an owned cursor is a **semantic consistency
change**, not a mechanical signature change: it is the introduction of I-4 that makes cross-visit
resumption sound. It is shared with Agent 3 and with every Studio write path, and section 15 asks
for a coordinated verdict on it.

### 9.3 Commit order

1. Finish an inventory begun and stepped under C-3 in this visit sequence; `enter_studio_budget`.
2. `studio_overlay_is_current` for both records and all live context, tenure required.
3. Complete-target comparison and `completed_branch` short-circuit, before source lookup,
   acknowledgement or sync reservation (HANDOFF-001).
4. `check_handoff_references`: candidate plus pending coverage of the branch's base CIDs (R9).
5. Preflight all three replacement peaks and the intent accounting.
6. Re-read the actual intent record and compare its complete authenticated plaintext digest and
   physical size with the captured values (C-2).
7. Barrier 1: write Prepared, retaining the complete branch and ledger.
8. Barrier 2: `save_studio_source_checked` with the `CheckedHandoffWrite` capability minted from the
   actual re-read bytes.
9. Verify the persisted source per 9.1 and construct `VerifiedPersistedSource`; barrier 3 through
   `resolve_studio_handoff_with_io`.
10. Return the outcome. Publication becomes eligible only now.

Preserved: no durable signed prefix, no per-entry retirement, the original full pending ledger, full
signed digests, the retry floor and rollover, the source replacement fence, the publication fence,
the source-required metadata link, and HANDOFF-002's inventory dependency.

## 10. Crash and interruption

| Interruption | Result |
|---|---|
| Any detached stage, including cancellation | Private candidate lost; no durable byte changed; branch, ledger, admission and pixel hold intact until the actual worker drops them; fresh attempt after backoff. |
| Between H3 slices | Same. Signatures exist only inside `StudioHandoffSigning`. |
| Before barrier 1 | No handoff happened. Active, original source. |
| Between barriers 1 and 2 | Prepared with the exact recorded source-before and no branch ids: durably return to Active with the full draft. |
| Between barriers 2 and 3 | Prepared with all exact envelopes and signed-operation digests: flush and complete without reapplying. |
| Step 9 digest mismatch | Do not complete; retain Prepared and resolve from actual bytes. |
| Partial or conflicting evidence | Retain the full branch, report a hold, guess nothing. Export remains available (12.1). |
| S3 interrupted after I-3's holds | One accounted atomic replacement with the existing exact-retry flush; the new references are protected by the ordinary mechanism regardless of the outcome; an exact retry is recognised at S1 with no fresh basis and no media work. |

## 11. Publication, invalidation and events

- The handoff emits no packets; `StudioSavedTransaction::empty()` keeps the two-packet initial Save
  window untouched, and transferred operations reach peers through ordinary current-tail and page
  service once Completed releases the publication hold.
- A local Save emits `RefreshRequired` and `LocalDraftRetained`; Completed emits `StudioUpdated` and
  `LocalDraftHandedOff`. Neither new variant is a delivery or settlement claim.
- The replay context key gains the store's `intent_generation`, and any runtime intent write clears
  the affected target from `replay.completed`, so a draft saved on an already watched epoch is
  noticed. The `no_overlay` memo uses the same handle.
- `studio_replay_evidence` filters `own` with `!state.is_overlay(id)` using the structural state.
  `NoEvidence` for a failed ordinary Save is unchanged.

## 12. Interface with Agent 2, and native Save exposure

### 12.1 Two holds (AG1-004, closed)

```rust
/// A live overlay job owns this target now. Transient, retryable, the only writer fence.
pub(crate) fn studio_overlay_live_hold(&self, target: StudioTarget) -> Option<StudioOverlayHold>;
/// The durable record is Prepared. Blocks destructive disposition, source replacement and
/// publication through the EXISTING fences. Does NOT block read-only export or inspection.
pub(crate) fn studio_overlay_transfer_hold(&self, target: StudioTarget) -> bool;
```

| Operation | Live hold | Transfer hold |
|---|---|---|
| Read-only inspection | refuse, retryable | permitted, unchanged |
| Read-only export of a typed-readable branch | refuse, retryable | permitted, without clearing Prepared, cancelling an owner, retiring anything or asserting completion |
| Copy into another eligible destination | refuse, retryable | permitted, subject to 12.2 |
| Destructive disposition | refuse, retryable | refuse |

### 12.2 Copy-while-Prepared: requirements handed to Agent 2

"A different current Open destination" is a starting condition, not the contract. Agent 2's copy
design must additionally bind the authenticated source branch, current membership and authorisation,
and the destination's **actual complete storage and document identity**: a different channel label
naming the same Flipnote object is not an independent destination, since the object id is the
logical key. It must recheck destination currency at the write, use ordinary authorised typed edits
with appropriate new authoring identity, capacity and reference protection, and must not clear
Prepared, retire the original envelopes, or count as evidence that the original handoff completed.
These are requirements on Agent 2's design, not a reason to restore the blanket prohibition.

### 12.3 What Agent 1 requires before registration

P1. A reviewed manual lifecycle: bounded inspect, export, copy-into-current and explicit
disposition, lossless across restart and refusal.
P2. Every `StudioOverlayHold` variant mapped to a user-visible, actionable state, including
`StaleRequest`, `PixelMissing`, `ReferenceCapacity`, `InventoryUnstable` and a durably unresolvable
Prepared branch.
P3. Truthful native results, events and UI-hooks rows for local-only, awaiting receipt, stale or
manual action, recovery and storage refusal.
P4. A live-tenure contract for `observed_owner_tenure_start()`: `None` is fail-closed for authoring
stages, and a returning owner in a new tenure must observe a differing value.
P5. An explicit statement in `GATE4-AGENT-2-STATUS.md` that P1 to P4 are implemented and reviewed.

## 13. Limits, costs and measurements

Nothing here is measured yet. Required, for Index and Flipnote:

1. Custody per stage of Flow H at 1, 32 and 256 operations: H1, one H3 slice, H5, with C-3's
   inventory separated from the rest of H5.
2. The same at maximal accepted shapes: the 5 MiB plus 1024-byte intent record filled by 256
   maximal-body operations, a 2 MiB seed, the 64 KiB combined metadata ceiling, the maximal accepted
   projection widths used by the inspection tests, and a large roster.
3. The largest admitted individual operation and roster shape, reported as worst single-signature
   time including its authority checks, since the deadline is checked between signatures.
4. Retained input and output within one permit: captured intent plaintext, captured source plaintext
   (dropped after H2 except its digest, size and derived facts), decoded state, restored private
   successor, signed candidate, encoded Prepared and Completed records, **and C-3's at most one
   parked record body**. Report the sum of the accounted bounds. This is not a measured heap ceiling.
5. Flow S custody per stage at 1, 32 and 255 accepted operations, including S1's structural
   classification, the S1b and S3 basis derivations against a maximal Closing source and seed, and
   S3's I-3 holds.
6. The effect of C-1 on `checked_epoch_replay_state` and on a five-family inventory of a vault
   containing several large retained branches.
7. C-3: maximum continuous custody per scan slice, **the largest single-record step** for each
   family at its accepted encoded ceiling with and without reference collection, how often the
   detached-validation escalation is needed, visits per full scan, and the restart rate under
   concurrent writes. The largest-step figures are what calibrate 9.2's conservative
   classify-before-invoking threshold; until they exist the threshold must default to detaching.
8. Wall-clock for a 256-operation handoff with the visit count, **reported separately from maximum
   continuous custody**.

Limits:

- **L1.** Total handoff latency follows from the slice bound and the receive cadence, not a single
  blocking call. Actor responsiveness is the acceptance criterion.
- **L2.** One MLS epoch spans every signature; an MLS commit during signing restarts the job after
  backoff. Accepted under an explicit eventual-stability condition, with N10 required.
- **L3.** The synchronous fence still restores the destination source when it finds an unresolved
  Prepared record with no verified candidate. Measure the worst case at 256 operations.
- **L4.** C-1 moves typed-replayability validation to the detached reconstruction under I-1.
- **L5.** `Recovery`, `OwnerReceipts` and `Intents` are all uncached in the scan; C-1 makes the
  Intents arm structural, which is the term that scales with a retained branch. An `inventory_cache`
  extension is not proposed.
- **L6.** C-3 gives bounded custody per visit and fail-closed spanning scans under I-4, with
  progress conditional on no five-family mutation for the duration of one scan. Under sustained
  writes a commit is held and retried. I-4's coverage is an audit obligation, and an under-rotating
  writer is the only unsafe direction.
- **L7.** Idle-only scheduling does not promise starvation-free overlay completion under sustained
  catch-up.

## 14. Test and mutation plan

### 14.1 Normal regressions

| # | Level | Case | Independent observation |
|---|---|---|---|
| N1 | actor | Index and Flipnote local Save, reopen, read | Projection, count, basis, envelope, nonce, author and timestamp survive restart; canonical source, gate, recovery, owner and Registry bytes unchanged. |
| N2 | actor | Absent-basis retry: accept a Save, lose the response, then make the source Open, Faulted and unavailable, and tenure Unknown; retry the exact request each time | Acknowledged at S1a with `alreadySaved`; no basis minted, no tenure required, no source lookup; record byte-identical apart from the accounted flush; no second envelope. |
| N3 | actor | Rollover: real handoff, legitimate retirement, a later handoff replacing the acknowledgement and advancing the floor, then a delayed retry of the first request while a newer basis is eligible | Returns stale; no new acceptance; no media admission; then force the rewind case and show `check_basis_floor` rejects independently. |
| N30 | actor | **AG1-001 media-free acknowledgement.** A frame operation referencing CID X is accepted and handed off; rotation, retirement and recovery retention legitimately remove every reference to X and cleanup removes its bytes; retry the original request | Acknowledged with **no blob read, promotion, hold or possession check**, no second envelope, and byte-preserving sync-only persistence. Then submit a genuinely new frame operation referencing the missing X and require refusal before acceptance with unchanged durable bytes. |
| N4 | store | Ordinary failed Apply leaves a pending intent; the same nonce and body offered as a local Save | Refused with `OrdinaryIntentCollision` at S1, before media admission; the entry stays unannotated and `NoEvidence`. |
| N5 | actor | Eligible successor installed by a real receipt; automatic handoff | One durable signed source replacement with all 256 operations in original order and timestamps; full ledger still pending; base released only after barrier 3. |
| N6 | actor | Peer catch-up after N5 | Peer projection equals local including conflicts and deletions; the initial-Save window never carried the batch. |
| N7 | store | Interrupt each durable write and sync, plus the step 9 digest mismatch | The accepted reopen table is reproduced; no false success, no lost branch, no duplicate effect. |
| N8 | store | Same-size authenticated replacement of the intent record, then the source record, after capture | Refused at the digest comparison before any signing turn; a fresh attempt succeeds. |
| N9 | store | Slice exclusivity | Structural assertion that a signing slice has no `await`, no reentrant store call and no store-reaching callback. |
| N10 | actor | Mid-signing MLS change | No durable signed prefix, no retirement, bounded retry; whole-branch completion after the context stabilises. |
| N31 | actor | **AG1-TEST-001 signing slice.** Two fixtures over a real H3 sequence, each isolating one bound, with the injected clock supplying deterministic elapsed time and **no paused detached worker**. See the preconditions and assertions below the table. | The observed visit signs at least one and fewer than all remaining operations; authoritative work completes between real signing slices; the branch then completes. |
| N11 | actor | Change device key, membership, owner, tenure, channel, mount, numeric server, actor instance between stages | Each produces its own hold; each fixture passes all earlier checks first. |
| N12 | store | **AG1-003 full lifetime.** (a) Pause S2, run a complete reference scan that installs a set omitting X, attempt protected deletion: bytes retained by the transient hold. (b) Resume S3 to success, drop **every** transient and result owner, then attempt protected deletion **before any further scan**: bytes still retained, because I-3 installed ordinary protection. (c) Repeat with an uncertain write (failure after rename, and a failure leaving a temporary sibling): bytes retained. (d) Remove the bytes externally before S3: `PixelMissing` refusal before the intent barrier with unchanged durable bytes. (e) Healthy unreferenced CID still deletes. |
| N13 | store | Transient-hold rails | Exhaustion refuses with `ReferenceCapacity` **before installing any partial hold**, does not mark the store unknown, and leaves unrelated reclamation working; live owners are counted through cancellation; duplicates are accounted, not hidden. Separately: fail-closed unknown is still reachable after uncertain durable I/O. |
| N14 | actor | **AG1-005 admission.** (a) Cancel a paused background job for target A and immediately attempt target B and a native Save. (b) Obtain a native `PrepareOverlaySave` handle and drop it without a second visit, before reconstruction starts. (c) The same while the worker is paused. | Admission stays unavailable while any real owner exists, then recovers with **no second native visit**; the shared slot is released exactly once. |
| N15 | actor | Four real jobs and results fill the shared pool | Retryable capacity refusal; no overlay-only pool. |
| N16 | actor | Pause a real H2 or H4 worker; another document's Save, an authoritative checkpoint and another server's progress complete | All three complete while the paused job retains ownership. |
| N17 | store | **C-3 spanning scans, per family, with the actual writers.** Park a cursor, then run: an accounted recovery write, an owner-journal write, a Registry write, a Studio source write, an intent write, and a cleanup unlink. Also a failed write leaving a temporary sibling, and a same-size authenticated replacement. | Every case rotates `inventory_generation` and the resumed cursor refuses at its next step and at finish; a quiescent vault completes; a Studio **budget mint or entry alone** does not invalidate; after `MAX_INVENTORY_RESTARTS` the commit holds with the branch retained. |
| N18 | store | C-3 custody bound | Maximum continuous custody per visit is bounded; a single cold record whose validation exceeds the budget is parked and validated detached, with at most one parked body. |
| N19 | store | R9 reference mechanisms, separately | A commit that would release a base reference refuses in memory; a missing required-metadata record leaves reclamation disabled after restart. |
| N20 | store | Unresolved Prepared with no live worker | Read-only export and inspection succeed; metadata, pending envelopes, Prepared state and the publication hold unchanged; destructive disposition refuses; copy into a genuinely distinct Open destination succeeds. |
| N21 | store | Prepared resolution for each evidence outcome from captured bytes | Absent returns to Active with the full draft; Complete flushes and completes without reapplying; Hold retains everything. |
| N22 | store | Page, current-tail, seed service and ordinary retry sending while Prepared | All refuse; after barrier 3 they serve. |
| N23 | store | C-1 equivalence | Identical state fields, `encode_vault` bytes and `handoff_metadata` answers; the structural decoder rejects a missing entry, a duplicate id, a wrong sequence, a wrong author, a changed envelope and trailing data. |
| N24 | store | C-1 boundary | Metadata readers succeed structurally on a non-replayable branch; the detached reconstruction refuses; ordinary metadata paths do not fail. |
| N25 | actor | Overlay ids absent from replay evidence | `studio_replay_evidence(..).own` contains no annotated id. |
| N26 | actor | Save on an already watched epoch | The intent generation invalidates the replay memo, the `no_overlay` memo and the completed bindings. |
| N27 | actor | Native session, view request and instance changed between visits and after conversion | Rejected; no partial value; anything already durable stays durable. |
| N28 | store | Maximal accepted shapes | Accepted at each ceiling; one byte over refuses with no partial output. |
| N29 | store | Earlier Gate 4 regressions | Unchanged. |

#### N31 in full (the AG1-TEST-001 correction)

Revision 3's N31 required only `remaining() > 0` after the observed visit, which a visit that
deferred on the priority gate without signing anything also satisfies. Both the unchanged and the
mutated implementation could then pass. The correction is a **positive signing precondition** plus
fixture staging that prevents the priority check from standing in for the slice bound.

Every N31 fixture asserts, for the visit that is supposed to exercise the bound:

```rust
let before = runtime.signing_remaining_for_test().expect("signing job");
// ... exactly one production background turn ...
let after = runtime.signing_remaining_for_test().expect("signing job");
assert!(after < before, "the observed visit did not sign anything");
assert!(after > 0, "the observed visit was not bounded");
assert_eq!(before - after, EXPECTED_SIGNATURES_THIS_VISIT);
```

`signing_remaining_for_test` is a `#[cfg(test)]` accessor over the production
`StudioHandoffSigning::remaining()`, in the style of the existing `replay_state_for_test`.
`remaining()` delegates to the private pending queue's length, and `PreparedOverlayChanges::sign_next`
appends the signature and removes exactly one pending operation only after that signature succeeds;
no queue item is removed before it does.

**Precisely, `before - after` counts successfully produced signatures**, not an unsuccessful
invocation and not the terminal `Ok(false)` on an empty queue. That is the intended reading and it
does not weaken N31: the observed visit is a successful, partially completed one, and it must
produce exactly the expected number of signatures. The test reads actual work progress rather than
incrementing an independent test counter.

Staging, common to both fixtures:

1. At slice entry there is **no** epoch-service interest, **no** watch inbound and **no** parked
   background result, so the priority gate cannot be the reason the visit stops. The fixture asserts
   this precondition before the turn.
2. Authoritative work is queued **after** the slice has been selected, through the sync and
   transport side (a peer page request and a checkpoint interest), never by reaching into the store
   from inside the slice, which slice exclusivity forbids.
3. The next background turn must service that authoritative work, observed as actual progress (the
   page or checkpoint advances), **before** the following slice signs again. That is the
   "authoritative progress between real signing slices" observation.
4. The branch then completes, with every original envelope and the full projection.

| Fixture | Independent precondition | Expected stop reason |
|---|---|---|
| Count limit | Remaining operations **exceed** `MAX_SIGNING_TURNS_PER_VISIT`; the injected clock advances a fixed small amount per operation so the elapsed budget **cannot** be reached within the cap | `before - after == MAX_SIGNING_TURNS_PER_VISIT` |
| Time limit | Remaining operations are **fewer** than `MAX_SIGNING_TURNS_PER_VISIT`, so the cap cannot be the reason; the injected clock crosses `SIGNING_SLICE_BUDGET_MS` after a chosen `k` operations while operations still remain | `before - after == k` |

The injected `catcoms_rt::Clock` is the deterministic seam; neither fixture depends on cheap and
expensive operations landing on opposite sides of a wall-clock threshold.

### 14.2 Isolated mutations

| # | Guard removed | Test | Assertion, at the protected boundary |
|---|---|---|---|
| M1 | Intent digest comparison in `studio_overlay_is_current` (size retained). **Redundant by design** with step 6. | N8 | "a stale plan reached a signing turn": the signing-turn counter is non-zero although the commit still refuses. |
| M2 | Source digest comparison in `studio_overlay_is_current`. **Redundant by design.** | N8 | Same boundary. |
| M3 | Weak-handle reaping in `OverlayAdmission::can_admit` (treat a dead owner as live, or a live owner as dead) | N14 | Two live admission tokens observed, or admission refused after every owner dropped. |
| M4 | Per-visit reauthentication before the first `sign_next` of a slice | N8 variant changing bytes between slices | "signing continued across visits on changed records". |
| M5a | `MAX_SIGNING_TURNS_PER_VISIT` only | **N31 count fixture** | "a single visit consumed every remaining signature": `after == 0`, so the `after > 0` assertion fails. The priority gate cannot mask it because the fixture asserts no authoritative work was pending at slice entry. The injected clock must stay below `SIGNING_SLICE_BUDGET_MS` **through the whole mutant batch**, not merely through the first cap-length prefix, or the time bound would stop the mutant and the count bound would not be isolated. This is a stronger staging condition than the normal run needs, and it is the M5a fixture's responsibility. |
| M5b | `SIGNING_SLICE_BUDGET_MS` only | **N31 time fixture** | Same assertion, with the count cap unable to be the reason because the fixture has fewer operations than the cap. |
| M6 | The H1 pristine-successor probe. **Redundant by design** with `check_overlay_successor`. | N5 negative variant | "**H2 started** for a non-pristine successor": a detached plan job was scheduled. Permit consumption alone is not the observation, because reservation now precedes the probe. |
| M7 | Barrier 1 before barrier 2 | N7 | "the source was replaced before Prepared was durable". |
| M8 | **`entry.sequence != index as u64 + 1` inside the shared `checked_entries`**, so the decoder and `encode_vault` use the same weakened predicate | N23 | "a wrong-sequence branch decoded and re-encoded successfully". A separate, labelled-redundant mutation removes only the structural decoder's earlier `checked_entries` call and asserts early refusal, since `encode_vault` would otherwise catch it. |
| M9 | The `!is_overlay(id)` filter in `studio_replay_evidence` | N25 | "an annotated id appeared in actual replay evidence". |
| M10 | S1 acknowledgement classification running before basis minting | N2 | "an accepted retry demanded a fresh Closing basis". |
| M11 | The S1b request-basis equality comparison | N3 | "a delayed request acquired the current basis". |
| M12 | The S3 basis re-mint; fixture removes the matching signed close from the **owner journal** between visits, with wrapper stamps unchanged and the substituted journal independently passing all earlier validation and current accounting inputs | N11 variant | "Save committed after its Closing basis became underivable". |
| M13 | `Protection::transient` consultation in `ProtectedBlobs::delete` | N12(a) | "a live job-owned CID was deleted". |
| M14 | **I-3's ordinary holds in S3** (release the transient owner without transferring) | N12(b) | "newly accepted pixels were deleted after a successful Save and before the next scan". |
| M15 | The S3 pixel possession revalidation | N12(d) | "a new acceptance named absent pixels". |
| M16 | Media admission placed before classification (restore revision 2's S0) | N30 | "a completed retry was refused because its pixels were reclaimed". |
| M17 | The step 9 persisted-source digest comparison | N7 | "completion proceeded without authenticating what landed". |
| M18 | The all-or-nothing requirement in the assemble stage | N7 | "a durable signed prefix escaped". |
| M19 | The transfer-hold versus live-hold split | N20 | "export of an unresolved Prepared branch was refused with no live worker". |
| M20 | `inventory_generation` rotation in **one** writer at a time: recovery, owner, Registry, Studio source, intent, cleanup unlink | N17, the matching family case | "a spanning scan finished across a real write to that family". |
| M21 | The `step_epoch_storage_scan` invalidation check (leaving only the `finish` check) | N17 | "the cursor resumed expensive work after invalidation". |
| M22 | The final native delivery recheck for Save | N27 | "an expired delivery returned a converted value". |

Existing `studio-handoff`, `studio-overlay`, `studio-inspection` and `studio-native` mutations are
retained. Unique anchors, one executed failing test, the intended assertion, byte-exact restoration
and a passing restored regression are required for every entry.

### 14.3 Harness and workflow

New `.github/scripts/check-studio-overlay-runtime-mutations.py`, following
`check-studio-handoff-mutations.py`, with logs under `logs/gate4-overlay-runtime-*.log`. Requested
workflow patch for Agent 4: a `runtime` job in `.github/workflows/studio-handoff.yml` running the
focused store and actor suites and the mutation script, publishing the logs, added to a required
workflow. Local execution stays serial: `-j 1`, the existing per-package test debug override, no
concurrent Cargo work, no blanket cleanup.

## 15. Dependencies and integration changes for Agent 4

| File | Change | Note |
|---|---|---|
| `crates/catcoms-replication/src/studio/overlay.rs`, `overlay/handoff.rs` | C-1 `decode_vault_structural` | Core change; own line in the verdict |
| `crates/catcoms-app/src/store.rs` and every five-family writer | **I-4 `inventory_generation` and `epoch_mutation_guard`**, including `epoch_recovery.rs`, `epoch_owner.rs`, `epoch_registry/*`, `epoch_studio*`, `epoch_intents*`, `epoch_recovery/cleanup.rs` | The largest and highest-risk item; an under-rotating writer is the only unsafe direction. Needs a coordinated verdict with Agent 3 |
| `crates/catcoms-app/src/store/epoch_recovery/inventory.rs` | C-1 structural Intents arm (reference path keeps `base_blob_cids`); **C-3 owned cursor with time budget and parked-body escalation** | HANDOFF-002 adjacent; shared with Agent 3 |
| `crates/catcoms-app/src/store/creative_references.rs` | **C-4 transient holds**, `delete` consulting both tables, `unknown()`/`install()` leaving transient untouched, bounds checked before installing | HANDOFF-002 adjacent; shared with every blob writer |
| `crates/catcoms-app/src/store/epoch_intents.rs` and its callers | Structural read entry points; C-2 digest fence in the **callers**; `read_scoped_intent_plain` visibility | Shared with Agent 3 |
| `crates/catcoms-app/src/store/epoch_studio.rs`, `epoch_studio/handoff.rs` | Commit accepts a detached candidate and `Option<VerifiedPersistedSource>` | Shared with Agent 3 |
| All `scan_epoch_storage_with_studio` / `scan_studio_receive_inventory` call sites | `begin` + bounded stepping + `finish` | `studio.rs`, `control.rs`, `receiver/catchup.rs`, `receiver/replay.rs`, `creative_references.rs`, Agent 3's repair paths |
| `crates/catcoms-app/src/studio/dispatch.rs`, `control.rs` | New action and response variants (5.6) | Central enum edit |
| `crates/catcoms-app/src/studio/receiver/catchup.rs` | New job and result variants, `OverlayOwnership` (5.5) | Central enum edit |
| `crates/catcoms-app/src/studio/receiver.rs` | `detach`, `complete`, `pending`, `background_step`, `control` gain overlay arms and the admission record | Shared with Agents 2 and 3 |
| `crates/catcoms-app/src/studio/settlement.rs` | Two new `StudioSettlementState` variants | Shared with Agent 2 |
| `crates/catcoms-app/src/studio/replay.rs`, `receiver/replay.rs` | Overlay-id exclusion and the intent-generation context key | Shared with Agent 2 |
| `apps/desktop/src-tauri/src/lib.rs`, `studio.rs` | Native commands, registration, security and capability rows | **Deferred**, gated on 12.3 |
| `docs/INTERFACES.md`, `BACKEND-IMPLEMENTATION.md`, `FLIPNOTE-UI-HOOKS.md`, `GATE4-ACCEPTANCE.md` | Rows for the new seams | Agent 4 owns |
| `.github/workflows/studio-handoff.yml`, `.github/scripts/` | New runtime job and mutation script | Agent 4 owns |

Agent 3 coordination: the runtime is the only new source writer and preparation consumer for
overlays. Repair must resolve an interrupted Prepared overlay through the existing fence, respect
both holds, adopt C-3's cursor at its scan call sites, and take `epoch_mutation_guard` in any writer
it adds.

**Agent 3 has accepted I-4.** Their design revision 1, committed at `7efc9c2` after this design's
revision 3, records in its section 13.1 that the owner record write, the recovery stage and the
successor write are all five-family durable mutations that must rotate the token at the same audited
choke point if I-4 lands, and that their design is unaffected if it does not. They also ask that
`save_studio_source_checked`'s `handoff` parameter shape be preserved so a parallel `repair`
parameter can be added without a third mechanism: **this design preserves it**, since 9.3 step 8
passes the existing `CheckedHandoffWrite` capability unchanged and the
`Option<VerifiedPersistedSource>` input is added to the resolver, not to the writer. Their design
also confirms it introduces no competing source writer and no second preparation pool. The
coordinated verdict the reviewer asked for therefore has both sides on record; the exhaustive
choke-point audit remains an implementation-review obligation.

One reading to foreclose: Agent 3's "unaffected if I-4 does not land" clause is about a different
integration choice, not an opt-out. **Once the resumable scanner is deployed, a participating repair
writer cannot decline the generation discipline**, because a spanning cursor's soundness depends on
every five-family mutation rotating the token. If I-4 is not adopted, C-3 is not adopted either and
the scanner keeps its exclusive borrow; the two stand or fall together.

## 16. Contingency if the core signing review changes the split

H1's authority capture and H2's `prepare_handoff_detached` are the only stages bound to the split.
H3's slice bound is independent of how many operations a call signs. If the split is rejected and
the core returns to one batch call, H2 to H4 collapse into a single detached stage; L1 improves and
the mid-slice authority recheck weakens, which needs its own review. Sections 5.1, 5.3, 5.4, 6.2,
6.3, 7, 8, 9.2, 11 and 12 do not depend on the split.

## 17. Reviewer answers adopted, and what remains open

From revision 1, unchanged: per-visit wrapper reauthentication conditional on slice exclusivity;
the structural and full decode split with full validation as the default; the existing narrow
authority interface with no new constructor; both a count limit and a time budget; restart on MLS
change under an explicit eventual-stability condition.

From revision 2:

1. **C-3 preferred over the synchronous residual, but only with a complete mutation-generation
   discipline.** Adopted as I-4 with an audited choke point, before-I/O rotation, all five families
   and temporary siblings, and explicit non-use of `studio_generation`. Step count is distinguished
   from a custody-time bound, with a measured largest step and a detached-validation escalation.
   Quiescence is defined in terms of `inventory_generation` rotations only, so the design does not
   self-invalidate. The synchronous fallback is not offered as a substitute for correcting the
   cursor; if C-3 is rejected, its maximum continuous custody needs separate measurement and an
   explicitly accepted limit.
2. **H1 authority placement accepted.** Section 6.1 no longer claims strictly improved freshness and
   states what the subsequent checks actually guarantee.
3. **Transient-hold exhaustion refuses new admission** without disturbing existing protection, with
   bounds checked before any partial hold, live owners counted through cancellation, and durable
   plus transient accounting that does not hide duplicates. Fail-closed unknown remains reachable
   after uncertain durable I/O (8.2), and I-3 depends on it.
4. **Copy-while-Prepared requirements** handed to Agent 2 in 12.2, including that a different
   channel label for the same Flipnote object is not an independent destination.

From revision 3, all three answers adopted with their attached consequences:

5. **Choke point backed by an audited writer list, not one or the other.** 9.2 makes the guard a
   type-level prerequisite, so a five-family write, rename, sync-repair or unlink cannot be
   expressed without it and a bypass is a compile error rather than a forgotten convention. The
   writer list still proves coverage, and N17 with M20 keeps per-family evidence. I-4 stays separate
   from budget ownership in both directions: a budget mint or entry alone must not invalidate, and
   an operation that may have changed files must invalidate even when it returns an error.
6. **Park and validate detached, within the accepted encoded bounds**, rather than aborting a scan
   because a valid record is expensive. The three consequences are written into 9.2: classify before
   invoking, because `budget_ms` is not a preemption mechanism and a post-hoc elapsed check enforces
   nothing, defaulting to detachment when cost cannot be conservatively classified; the parked body
   and its worker and result keep the job's original ownership and are rebound to cursor, mount,
   record and generation before consumption; and no bound, poisoning rule or reference-cache
   exclusion is bypassed. This accepts a scheduling boundary, not a measured custody or heap ceiling.
7. **Keep the transient owner until the write attempt returns**, as specified. 8.3 records why:
   earlier release would already be safe inside one uninterrupted custody interval, but retaining it
   gives a single ownership boundary across success, error and unwinding, and the durable guarantee
   must come from I-3's pre-I/O transfer rather than from assuming a successful return. The
   premature-release optimisation is deliberately out of scope for this checkpoint.

Nothing remains open. Every design question raised across revisions 1 to 4 is answered and adopted,
and the revision-4 review returned PASS with no new finding.

## 18. Review record and the next checkpoint

### 18.1 Design review, closed

| Revision | Base | Head | Verdict |
|---|---|---|---|
| 1 | `a052f78b62a549702686a8741932f1d2f8c98773` | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | REQUEST CHANGES: AG1-001 to AG1-005, AG1-TEST-001 |
| 2 | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | REQUEST CHANGES: AG1-004 closed |
| 3 | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | `1bcb1bca204d721b848b17c0835faf931ae930e3` | REQUEST CHANGES: AG1-001, AG1-002, AG1-003, AG1-005 closed |
| 4 | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | **PASS**: AG1-TEST-001 closed; bounded runtime design accepted |

All four reviews were source and design inspection; no Cargo command, test, mutation or measurement
was executed in any of them, and none is claimed by this document.

### 18.2 What the PASS does not cover

Implementation of C-1, C-2, C-3, C-4 and I-4; the runtime itself; every measurement in section 13;
every regression in 14.1 and every mutation in 14.2, including the exact M5a and M5b executed
failures with byte-exact restoration and passing restored regressions; the core signing split at
`e65bfd8`; Agent 2's manual lifecycle and therefore native Save registration; Agent 3's repair
design; and full Gate 4 acceptance, which remains with Agent 4.

### 18.3 Implementation review request (review preamble 1)

Send this at the first bounded implementation checkpoint, not before. The bracketed fields must be
real before sending; the preamble forbids placeholders. Section 15's sequencing note asks that I-4
with its writer audit, then C-3, then C-1 and C-4, then the runtime each get their own line in the
verdict rather than arriving as one commit.

```text
Review type: bounded implementation.
Base: [FULL_BASE_SHA]. Head: [FULL_HEAD_SHA]. Compare: [IMMUTABLE_COMPARE_URL].
Scope/evidence: docs/GATE4-AGENT-1-STATUS.md at the head, and the design accepted at
25ce89bb705dfc228c7b32874788ebd062e6fcf4 (PASS, design boundary only).
Dependencies: [CORE_SIGNING_VERDICT]; native Save exposure: [ACTUAL_STATE, expected: unregistered,
pending Agent 2's reviewed manual lifecycle].
Suites and runs: [COMMANDS, RESULTS, RUN_AND_JOB_URLS, ACTUAL_CHECKOUT_SHAS].
Mutations: [PER_MUTATION EXECUTED FAILURE, INTENDED ASSERTION, RESTORED PASS].
Per-item verdict requested for: I-4 and its writer audit; C-3; C-1; C-4; the runtime.

Challenge the complete capture, detached plan, authorize, finite signing slices, detached assembly,
durable commit and native delivery path as implemented, not as designed. Identify every expensive
decode, graph restore, inventory traversal and final conversion that still holds Server, vault or
native custody, and check the measurements in design section 13 against what the code actually does
at maximal accepted shapes.

Verify the accepted invariants hold in code: I-1 (first acceptance fully validates; later writers
preserve the checked identity), I-2 (one live per-actor admission proved by a live Arc, including a
dropped native handle and a cancelled worker), I-3 (ordinary protection installed before possible
I/O and before the transient owner is released), I-4 (rotation before any five-family mutation,
kept on failure and unwinding, with the audited writer list proving coverage and a budget mint or
entry alone not rotating). Confirm the original shared permit is owned from capture to release and
that cancellation never frees a live owner's slot.

Require whole Prepared, Source, Completed with full signed evidence, the retained pending ledger,
the retry floor and rollover, HANDOFF-001's target comparison, HANDOFF-002's inventory dependency
and, separately, check_handoff_references; the common source-write fence and the publication hold.
No signed prefix may escape, and Completed publication must use ordinary paging with the two-packet
initial Save limit untouched.

For the mutation evidence, verify the unique guard removed, the executed test count, the exact
intended assertion, byte-exact restoration and the restored pass. M5a and M5b must each fail at the
after > 0 assertion with the other bound unable to be the stop reason, and M1, M2 and M6 are
intentionally redundant guards asserting early refusal and resource consumption rather than
corrupted durable state. A compilation failure, a zero-match filter or an unrelated refusal is not
detection.

Return a verdict for this runtime boundary only. Native Save must remain unregistered until Agent
2's manual lifecycle passes its own review, and full Gate 4 acceptance remains with Agent 4.
```

