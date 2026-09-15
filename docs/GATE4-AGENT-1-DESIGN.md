# Gate 4 Agent 1: local Save and automatic handoff runtime

Status: **revision 2, design proposal, awaiting re-review. No production code is written.**
Revision 1 (`ac12822f04337b3e388618f81ce4a4b29d1e9b87`) received REQUEST CHANGES with four P2
design findings and two P3 specification gaps. This revision answers all six, corrects the
audit claims the reviewer qualified or rejected, and adopts the reviewer's answers to the
revision-1 questions as binding constraints.

Design base: `a052f78b62a549702686a8741932f1d2f8c98773`. Revision 1 head:
`ac12822f04337b3e388618f81ce4a4b29d1e9b87`. Scope is
[Agent 1 of the four handoffs](GATE4-AGENT-HANDOFFS.md); progress and evidence are in
[GATE4-AGENT-1-STATUS](GATE4-AGENT-1-STATUS.md).

**Unmet dependency, unchanged.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md) at
`e65bfd8` is still unreviewed; the reviewer inspected its interfaces to assess this proposal and
explicitly did not grant it a PASS. Section 16 carries the contingency.

Accepted work this design must not weaken: the Closing-overlay foundation (`b1b0ec9`,
OVERLAY-TEST-001 closed), the handoff design (HANDOFF-001 closed), the bounded core/store handoff
implementation (`62f06d4`, HANDOFF-002 closed), the detached-inspection proposal (`0b28f06`) and
its read-only implementation (INSPECTION-TEST-001 closed), and the combined scheduling block
(`6b71d96`). No closure is reopened here.

## 0. Disposition of the revision-1 findings

| Finding | Disposition | Where |
|---|---|---|
| AG1-001 P2, Flow S breaks the retry contract | Corrected. Acknowledgement classification now precedes basis minting and is cheap enough to run under custody; the native request carries its original basis; rollover returns stale rather than acquiring today's basis; the handed-off wording is corrected. | 6.2, 6.3, 5.6, 5.7, 14 N2/N3 |
| AG1-002 P2, H7 still reconstructs under custody | Corrected. No graph restore remains on the commit path: the persisted source is proved by digest equality against the retained candidate, and the resolver accepts that verified reuse. Inventory gets an explicit bounded strategy (C-3) with a stated eventual-progress condition. | 9, C-3, 13 L3/L6 |
| AG1-003 P2, the PIX pre-hold does not survive detachment | Corrected. A bounded job-owned transient hold that complete scans may not subtract, plus a commit-time possession revalidation that refuses before the intent barrier. | 8, C-4, 14 N12/N13 |
| AG1-004 P2, Prepared becomes a permanent export prohibition | Corrected. The single hold is split into a transient live-operation hold and a durable transfer hold; read-only export of a typed-readable Prepared branch is explicitly permitted. | 12.1, 14 N20 |
| AG1-005 P3, admission invariant does not follow | Corrected. An explicit per-actor admission token owned through cancelled workers and retained results, in which native Save participates; the permit is reserved before the first body read and H0 is folded into H1. | 7, 5.5, I-2 |
| AG1-TEST-001 P3, masked mutations | Corrected. Each mutation now observes at the boundary it protects; intentionally redundant guards are labelled and tested for early refusal and resource behaviour. | 14.2 |
| Audit corrections (R1 scope, read count, C1 "projection-free", C1 trust wording, R5 "strictly stronger", R3/R4 nuance, two reference mechanisms) | Accepted and rewritten. | 3 |
| Reviewer answers to revision-1 questions 1 to 5 | Adopted as binding constraints with their stated conditions. | 17 |

## 1. Outcome and boundary

A real actor must durably accept an explicit local overlay operation on an eligible Closing
document, retain and read it across restart, and automatically transfer the complete branch when
the verified eligible successor arrives, while unrelated actor work, authoritative discovery and
receive, and another server keep progressing.

Out of scope: manual inspect/export/copy/disposition and stale-base handling, preview-based local
work, repeated-owner tenure authority (all Agent 2); signed fault repair (Agent 3); integration
and full-gate acceptance (Agent 4).

Native Save is designed and implemented behind the actor/store seam but **not registered**. Its
`#[tauri::command]`, registration and security rows land in a separate identifiable commit owned
by Agent 4, gated on section 12's prerequisites.

## 2. What was audited

Revision 1's audit list, plus, for this revision:
`crates/catcoms-app/src/store/creative_references.rs` (`Protection::{unknown, install}`,
`hold_creative`, `hold_creative_operation`, `creative_pinned_cids`, `ProtectedBlobs::delete`),
`store/epoch_recovery/inventory.rs` (`collect_creative_references`, `finish_creative_references`,
`EpochStorageScan` borrow and step rails), `store/epoch_studio.rs`
(`save_studio_source_checked`'s returned `SourceVersion`), and
`crates/catcoms-replication/src/studio/overlay.rs` (`BasisData::fingerprint`,
`StudioOverlay::{exact_retry, contains}`, `StudioOverlayState::completed_retry`).
**No Cargo command was executed and no new measurement exists.** Quoted numbers are the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations.

## 3. Audit observations, corrected

These are properties of the accepted code. They are reported as required, not as reopened
closures. Revision 1 overstated two of them; the corrections are marked.

### R1: decoding a retained branch performs a full ordered reconstruction

`StudioOverlay::decode_vault` ends with `out.read(ledger)?`
([overlay.rs:419](../crates/catcoms-replication/src/studio/overlay.rs#L419)), which replays every
accepted operation through `local_policy`, `prepare_local_write`, an Automerge clone and commit
per entry, `validate`, a projection read and `recovery::preflight`. `EpochIntentState::decode`
reaches it whenever the record carries an Active or Prepared branch. This is why the profile's
`decode_ms` is essentially `draft_ms`.

**Correction to revision 1.** The claim that *ordinary* Studio reads always pay this was too
broad. `load_studio_epoch`, `checked_studio_source`, `capture_studio_source` and
`studio_source_bytes_match` reach the intent decoder through `check_studio_intent_link` only when
the **source wrapper carries the required-metadata link byte**, which `save_studio_source_checked`
sets on a source write made while handoff metadata exists. A local Save writes only the intent
record, so a freshly accepted branch on a source that has not been rewritten since does not make
ordinary reads decode it. A Completed-only or floor-only record has no Active branch to replay and
is cheap. The expensive combination is *linked source plus retained Active or Prepared branch*,
which is reachable (a source rewrite while the branch exists, for example receipt or Fault
recording, rotation, adoption or a completed handoff that leaves a new Active branch) but is not
the default state after a first Save.

What is unconditional, and is the reason the runtime cannot ignore this:

| Path | Effect |
|---|---|
| `checked_epoch_replay_state` | every Studio write transaction on that document reconstructs the branch |
| Five-family inventory scan, `EpochRecordKind::Intents` arm | **every** `studio_storage_budget` mint anywhere in the vault reconstructs **every** retained branch in the vault; the Intents family is the one family `inventory_cache` does not cover (`cacheable` is `Registry \| Studio`, [inventory.rs:570](../crates/catcoms-app/src/store/epoch_recovery/inventory.rs#L570)) |
| `write_prepared_intents` old-size read, `check_studio_handoff_write`, `check_studio_handoff_publication`, retirement | each decodes again |

**Correction to revision 1.** "About four" reconstructions in the synchronous commit was a fixed
count that the code does not guarantee. The number is path dependent (preparation reads, the
old-size lookup, the unchanged check, persistence, the source fence and resolution) and can be
higher. No timing claim follows from the count.

### R2: local acceptance reconstructs the whole branch under custody

`StudioOverlay::append` validates by building `staged` and calling `staged.read(ledger)`
([overlay.rs:244](../crates/catcoms-replication/src/studio/overlay.rs#L244)), so saving the Nth
operation replays N operations inside `write_studio_overlay_intent`, under the lease.

### R3: the basis is re-derived on every non-retry Save

`save_studio_closing_overlay_with_io` calls `prepare_closing_overlay` on a freshly checked source
for every accepted append. That is more than a phase read: `prepare_settlement` checks the actual
receipt head, its document, closed epoch and close-record hash, verifies the receipt against the
live group and observed tenure, builds the typed checkpoint from the named closure and binds the
source version. Deriving the close through `EpochOwnerReceiptState::close_for` preserves the exact
saved receipt-to-close binding; **that journal is historical evidence, not fresh owner authority**,
and the observed-tenure verification inside `prepare_settlement` remains necessary. The cost is
bounded by the Closing source and seed, not by the branch, and is currently measured only on a
1.5 KB fixture.

### R4: replay does not exclude overlay-annotated intent ids

`studio_replay_evidence` builds `own` by author only
([replay.rs:94-99](../crates/catcoms-app/src/studio/replay.rs#L94-L99)). The accepted handoff
design requires explicit exclusion of Active and Prepared overlay ids before worker integration.
**This is selection hardening, not evidence that the current ordinary Apply guard permits an
overlay bypass**: today those ids reach `choose`, which returns `NoEvidence` because no recovery
snapshot holds them. `NoEvidence` for a failed ordinary Save must be preserved unchanged.

### R5: the commit's unchanged fence repeats a decode

`handoff_studio_overlay_with_io` proves "the intent record did not change" by decoding it again
and comparing `actual.encode(&scope)` to `original`
([handoff.rs:236-239](../crates/catcoms-app/src/store/epoch_studio/handoff.rs#L236-L239)).

**Correction to revision 1. The claim that a plaintext-digest comparison is "strictly stronger" is
withdrawn.** The ledger decoder already requires canonical re-encoding equality and both overlay
versions preserve and check their canonical encodings, so for accepted canonical records the
existing comparison already compares the complete encoded contents, not a projection. The correct
justification for change C-2 is narrower: comparing the complete authenticated plaintext digest
and physical size avoids repeating the reconstruction at the currency check and preserves
full-byte currency more directly and more cheaply, under the usual collision assumption, given
that the captured contents already passed the required validation. It is not demonstrably
stricter, and no normalization counterexample is claimed.

### R6: the Prepared fence resolves synchronously

`resolve_studio_handoff_with_io` is reached from the rotation, adoption, shared-write and
publication fences and restores the destination source before classifying evidence. Section 9
keeps that backstop and removes the restore from the runtime's own path.

### R7 (new, from AG1-003): a transient pixel hold does not survive a complete scan

`creative_pinned_cids` calls `Protection::unknown()`, which sets a new generation and drops
`pins`, then installs the set derived from durable state. A pre-hold added by `hold_creative`
before that scan is therefore discarded, and `ProtectedBlobs::delete` consults only the installed
set. A hold added *during* the scan invalidates the install through the generation check, but a
hold added *before* it does not survive. Today the window between the pre-hold and the durable
intent write is inside one custody visit, so no other operation can run a scan in between.
Detaching Flow S opens that window. Section 8 closes it.

### R8 (new, from AG1-002): the inventory scan holds an exclusive store borrow for its whole traversal

`EpochStorageScan<'_>` borrows `&mut ServerStore`. Its one-body-per-step rail bounds memory and
per-step work, not custody: a loop over `step()` under one lease is a single uninterrupted
critical section whose length scales with the vault's records and cold bytes. Reference scans
deliberately bypass the pure validation cache, so a reference-collecting scan is the expensive
case. This is pre-existing and is paid by every Studio write today; it becomes the dominant
remaining term once C-1 removes the branch replay from the Intents arm.

### R9 (new): the two reference mechanisms are distinct and both must survive

`check_handoff_references` checks that the candidate source's CIDs plus the pending operations'
CIDs cover the overlay's base CIDs, in memory, at the commit. HANDOFF-002's complete authenticated
inventory separately collects required source-to-intent metadata targets and refuses to install
deletion protection when that dependency is unsatisfied. They are different traversals with
different jobs; revision 1 blurred them. Both must survive this refactor and both are tested
separately (N17, N19).

## 4. Design principles

1. **One algorithm, two drivers.** The existing synchronous entry points become the inline
   composition of the same stage functions the runtime schedules, exactly as the core split kept
   `prepare_handoff` as a compatibility adapter. There must be no second Save, handoff or
   resolution algorithm to drift from the reviewed one.
2. **Custody is spent on evidence, not on computation.** Under the lease the runtime reads,
   authenticates, hashes, structurally decodes, checks live authority, accounts and writes. Full
   decode, branch reconstruction, private candidate restoration, typed admission and manifest
   assembly run only on a detached worker. Where a verified value already exists in memory, the
   commit proves it against the actual persisted bytes rather than recomputing it.
3. **Every detached result is a proposal.** It becomes durable only after a custody visit
   reauthenticates the exact wrapper bytes it was derived from and rechecks live authority. No
   worker assertion substitutes for authenticating what actually landed.
4. **Admission is explicit.** One per-actor overlay admission token and one shared
   `preparation_pool()` permit are reserved before the first body read and owned until the last
   actual owner, including a cancelled worker, drops them.
5. **Acknowledgement is not authoring.** Recognising an accepted request is a separate contract
   from creating one: it authenticates target, author, envelope and original request context and
   never demands, or silently supplies, a fresh Closing basis.
6. **Refusal retains work.** Every hold, cancellation, stale stamp, capacity refusal and authority
   change leaves the complete branch and the ordinary ledger untouched.

## 5. Concrete APIs

New leaf modules owned by Agent 1:

```
crates/catcoms-replication/src/studio/overlay/structural.rs   (decoder split, C-1)
crates/catcoms-app/src/store/epoch_studio/overlay_capture.rs  (capture, stamp, plan)
crates/catcoms-app/src/store/epoch_studio/overlay_commit.rs   (staged commit seams)
crates/catcoms-app/src/studio/overlay/runtime.rs              (job state machine, admission)
crates/catcoms-app/src/studio/overlay/save.rs                 (native two-visit local Save)
apps/desktop/src-tauri/src/studio/overlay.rs                  (unregistered native surface)
```

### 5.1 C-1: structural decode, separate from reconstruction

```rust
impl StudioOverlay {
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
}
impl StudioOverlayState {
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
}
```

`decode_vault_structural` keeps, in order: the `MAX_EXTENSION` bound, the version tag, the nested
seed and metadata bounds computed before allocation, the complete target derivation and its
receipt and ledger scope equality, `checked_entries` (ledger membership, no duplicate id,
`sequence == index + 1`, `next_sequence == len + 1`, author equality, exact envelope hash,
timestamp bound, non-empty, at most `MAX_STUDIO_OVERLAY_OPS`), the Prepared and Completed field
decoding with their own bounds and target equality, and `encode_vault(ledger)? == bytes`. It drops
only `out.read(ledger)?`.

**Correction to revision 1: "projection-free" was too broad.** The moved call sites are
*branch-replay-free*, not free of all graph work. In particular the reference path deliberately
retains `StudioOverlay::base_blob_cids`, which calls `base.graph()` and performs typed seed work
([overlay.rs:330-332](../crates/catcoms-replication/src/studio/overlay.rs#L330-L332)); C-1 does
not remove that, and must not be advertised as doing so. The source-write and completion paths
likewise still need their separate signed-source evidence.

**Correction to revision 1: the trust argument.** AEAD authentication under the local database key
proves origin and integrity; it is **not** proof of typed admission, and not every
`write_prepared_intents` call follows a fresh `append` (ordinary edits, handoff transitions and
retirement also rewrite these records). The invariant this design relies on is therefore stated
directly, and is a requirement on the writers, not an inference from sealing:

> **I-1.** First acceptance of an overlay entry fully validates the branch through
> `StudioOverlay::append`, which reconstructs it. Every subsequent writer of that record must
> preserve the already-checked identity and evidence rather than re-deriving them: ordinary intent
> writes must not alter the overlay extension, handoff transitions may only move between the
> Active, Prepared and Completed forms derived from an already-validated state, and retirement may
> not remove an entry an annotation still requires.

Structural decode mints no authority. Full reconstruction stays mandatory before display, append,
handoff preparation or export. A record that is authenticated, canonical and structurally
consistent but not typed-replayable is accepted by metadata readers and rejected on the detached
reconstruction; the branch is then held and surfaced through the manual lifecycle rather than
making the document's metadata paths fail. That is a real, reviewable change to the validation
boundary, not equivalent validation.

Call sites moved to structural decoding, each justified by the data it needs:

| Call site | Needs |
|---|---|
| Inventory scan, Intents arm | record footprint; for reference scans `handoff_metadata().target()`, `overlay().base_blob_cids()` (retains its typed seed work) and `pending()` operation CIDs |
| `check_studio_intent_link` | `handoff_metadata().check_target(target)` |
| `checked_epoch_replay_state` | ledger, accounting, `overlay()` identity |
| `write_prepared_intents` old-record read | previous physical size only (C-2) |
| `check_studio_handoff_write`, `check_studio_handoff_publication` | `is_prepared`, `has_completed`, `check_target`, `evidence`, `pending`, `is_overlay` |
| `epoch_intents/retirement.rs` | ledger and overlay id membership |
| Runtime admission (7.2), acknowledgement classification (6.3), Flow H authority capture (6.1) | entry ids, envelopes, `basis()`, author, target, receipt bytes, Prepared and Completed presence |
| `studio_replay_evidence` overlay exclusion | `is_overlay(id)` |

Call sites keeping full `decode_vault`, all detached: `EpochIntentState::local_draft` (inspection),
the runtime's detached plan stage, Agent 2's export and copy preparation.

### 5.2 C-2, C-3, C-4: the other changes

- **C-2.** `write_prepared_intents` and `persist_handoff_intents` obtain the previous record's size
  through a bytes-and-size read (`read_scoped_intent_plain`, already used by the inspection
  capture) instead of a decode, and the "unchanged" fence compares the complete authenticated
  plaintext digest and physical size. Accounting inputs are identical. Justification is R5 as
  corrected: cost, not strength.
- **C-3 (new, required by AG1-002).** A resumable five-family inventory cursor, section 9.2.
- **C-4 (new, required by AG1-003).** Bounded job-owned transient reference holds, section 8.

### 5.3 Store: capture, stamp and plan

```rust
// store/epoch_studio/overlay_capture.rs

/// Public context and record identity. No plaintext, key, store handle or Server.
pub(crate) struct StudioOverlayStamp {
    mount: Arc<()>, server: u64, document: LogicalDocument, target: StudioTarget,
    actor: DeviceId, actor_key: Vec<u8>, owner: DeviceId, mls: u64,
    incarnation: RegistrySyncInstance,
    /// Captured for information. Requiring a KNOWN tenure belongs to the authoring stages only
    /// (AG1-001): acknowledgement and Prepared resolution must not demand new-edit tenure.
    tenure: Option<u64>,
    /// Absence is explicit and distinct from an unreadable record.
    intent: Option<(blake3::Hash, u64)>,
    source: Option<(blake3::Hash, u64)>,
}

/// Authenticated zeroizing plaintext plus the stamp. No device or MLS secret, no editable
/// installed epoch, no source writer, no budget.
pub(crate) struct StudioOverlayCapture {
    stamp: StudioOverlayStamp,
    group: Vec<u8>,
    intent: Option<Zeroizing<Vec<u8>>>,
    source: Option<Zeroizing<Vec<u8>>>,
    work: StudioOverlayWork,
}

/// Only the stages that create new durable authoring work carry authority-bearing inputs.
pub(crate) enum StudioOverlayWork {
    /// Classification already proved this is NOT an acknowledgement, and the caller already
    /// re-derived and matched the Closing basis under the same custody visit.
    SaveAppend { basis: StudioClosingOverlayBasis, intent: LocalIntent, ts: u64,
                 pixels: Option<CreativeHold> },
    /// Live authority captured under custody from the structurally decoded branch receipt.
    Handoff { authority: StudioHandoffAuthority, basis: [u8; 32] },
    /// Membership only. No new-edit tenure, no basis.
    Resolve,
}

pub(crate) struct StudioOverlayPlan { stamp: StudioOverlayStamp, outcome: StudioOverlayPlanned }

pub(crate) enum StudioOverlayPlanned {
    /// `state` contains the appended overlay and its encoded bytes; `accepted` is its new count.
    SaveAppended { state: Box<EpochIntentState>, accepted: usize },
    /// Private restored successor already consumed into a signing batch.
    HandoffPrepared { signing: Box<StudioHandoffSigning> },
    /// Restart or fence path only: the destination restored from captured bytes.
    Resolved { source: Box<StudioEpoch>, evidence: StudioHandoffEvidence,
               next: Box<EpochIntentState> },
    Hold(StudioOverlayHold),
}

/// Every variant keeps the complete branch and the ordinary ledger.
pub(crate) enum StudioOverlayHold {
    NoOverlay, NotClosing, WrongAuthor, WrongTarget, BasisChanged, StaleRequest,
    PreparedPending, SuccessorNotPristine, SuccessorMissing, OrdinaryIntentCollision,
    TenureUnknown, PixelMissing, Structural(String),
}

impl ServerStore {
    /// Caller already owns the per-actor admission token and one shared preparation permit,
    /// and has live membership custody. Bounded reads with the ordinary parent-directory and
    /// regular-file restrictions. Does NOT decode and does NOT evict `self.studio_source`.
    pub(crate) fn capture_studio_overlay(
        &self, context: &StudioOverlayContext, work: StudioOverlayWork,
    ) -> Result<StudioOverlayCapture, AppError>;

    /// Compares mount pointer, numeric server, complete target, document, actor, actor key,
    /// owner, MLS epoch, and BOTH full wrapper digests with physical sizes. Captured absence
    /// must remain absence. Tenure equality is required only for `SaveAppend` and `Handoff`.
    pub(crate) fn studio_overlay_is_current(
        &self, context: &StudioOverlayContext, stamp: &StudioOverlayStamp,
        require_tenure: bool,
    ) -> Result<bool, AppError>;

    /// Cheap structural read for admission, acknowledgement classification and authority
    /// capture. Reads and authenticates one bounded record; performs no reconstruction.
    pub(crate) fn studio_overlay_structural(
        &self, server: u64, document: &LogicalDocument,
    ) -> Result<Option<EpochIntentState>, AppError>;
}

impl StudioOverlayCapture {
    /// Blocking worker only. Owns authenticated plaintext, public context, the permit, the
    /// admission token and any transient pixel hold. Full `decode_vault`, reconstruction,
    /// typed admission, private candidate restoration and `prepare_handoff_detached` run here.
    pub(crate) fn plan(self) -> Result<StudioOverlayPlan, AppError>;
}
```

`StudioOverlayContext` is the live-side twin of `studio::inspection::Context`: group id, device id
and public key, designated owner, MLS epoch, observed tenure, numeric server, mount and registry
sync instance, built inside `sync.with_registry_context` once per custody visit and never carried
across a detach.

### 5.4 Store: commit seams

```rust
// store/epoch_studio/overlay_commit.rs
impl ServerStore {
    /// One accounted intent write behind stamp equality. Used by Flow S, by acknowledgement
    /// and by Agent 2's disposition and copy bookkeeping. Retires nothing and prunes nothing.
    pub(crate) fn commit_studio_overlay_state(
        &mut self, context: &StudioOverlayContext, stamp: &StudioOverlayStamp,
        next: EpochIntentState, unchanged: bool, budget: &mut EpochStudioBudget,
        rng: &mut impl CryptoRngCore,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError>;

    /// Derive the Closing basis from durable state only: a Closing source, its receipt head,
    /// and the matching signed close from the saved owner journal (`close_for`, historical
    /// evidence). `prepare_settlement` supplies the live owner and observed-tenure check.
    pub(crate) fn studio_closing_basis(
        &mut self, server: u64, group: &ServerGroup, target: StudioTarget,
        device: &MlsDevice, tenure: Option<u64>, budget: &mut EpochStudioBudget,
    ) -> Result<Option<StudioClosingOverlayBasis>, AppError>;
}
```

### 5.5 App: admission and the job state machine

```rust
// crates/catcoms-app/src/studio/overlay/runtime.rs

/// AG1-005: one LIVE reservation per actor, not one tracked job. The token is held by the job,
/// by every detached worker and by every retained result. A cancelled waiter parks a Weak here;
/// admission is refused until that Weak's strong count reaches zero, so a still-running
/// cancelled closure cannot be joined by a second job for another target.
pub(in crate::studio) struct OverlayAdmission {
    live: Option<Arc<()>>,
    draining: Vec<std::sync::Weak<()>>,
}
impl OverlayAdmission {
    fn can_admit(&mut self) -> bool {
        self.draining.retain(|w| w.strong_count() != 0);
        self.live.is_none() && self.draining.is_empty()
    }
    fn admit(&mut self) -> Arc<()>;          // minted once, cloned into every stage
    fn release(&mut self, token: Arc<()>);   // ordinary completion
    fn drain(&mut self, token: std::sync::Weak<()>); // cancellation; not a release
}

pub(in crate::studio) struct OverlayRuntime {
    admission: OverlayAdmission,
    job: Option<OverlayJob>,
    next_attempt_at: BTreeMap<StudioTarget, u64>,
    backoff: BTreeMap<StudioTarget, u64>,
    /// Negative admission memo keyed by the store's intent generation: a target with no overlay
    /// is not re-probed until any intent write invalidates the generation.
    no_overlay: BTreeMap<StudioTarget, std::sync::Weak<()>>,
    selection: usize,
    notices: SettlementNotices,
}

struct OverlayJob {
    context: OverlayJobContext,   // mount, server, incarnation, target, request kind
    admission: Arc<()>,
    permit: OwnedSemaphorePermit,
    pixels: Option<CreativeHold>, // Flow S only; see section 8
    stage: OverlayStage,
}

enum OverlayStage {
    Captured(Box<StudioOverlayCapture>),                 // ready to detach
    Signing(Box<StudioHandoffSigning>, Box<HandoffFacts>), // custody slices
    Assembling,                                          // detached finish is running
    Assembled(Box<StudioHandoffCommit>),                 // ready for the commit visit
    Planned(Box<StudioOverlayPlan>),                     // ready for the commit visit
}
```

Background job and result variants mirror `StudioBackgroundJob::Prepare` exactly: the permit,
admission token and any pixel hold are moved into `spawn_blocking`, so a cancelled waiter cannot
refund any of them.

```rust
pub(crate) enum StudioBackgroundJob<T: MeshTransport> {
    // ... existing ...
    OverlayPlan(Box<StudioOverlayCapture>, OverlayOwnership, OverlayJobContext),
    OverlayAssemble(Box<StudioHandoffSigning>, Box<HandoffFacts>, OverlayOwnership, OverlayJobContext),
}
/// The three things a worker must outlive its waiter to own.
pub(crate) struct OverlayOwnership {
    permit: OwnedSemaphorePermit, admission: Arc<()>, pixels: Option<CreativeHold>,
}
pub(crate) enum StudioBackgroundResult {
    // ... existing ...
    OverlayPlanned(OverlayJobContext, Result<(Box<StudioOverlayPlan>, OverlayOwnership), AppError>),
    OverlayAssembled(OverlayJobContext, Result<(Box<StudioHandoffCommit>, OverlayOwnership), AppError>),
    OverlayCancelled(OverlayJobContext, std::sync::Weak<()>),
}
```

`OverlayCancelled` carries the admission `Weak` so the runtime calls `drain`, not `release`.
Native Save participates in the same record: `StudioReceiver::control` acquires the admission and
the permit for `PrepareOverlaySave`, and both travel inside `StudioPreparedOverlaySave` to the
second visit, so a native Save and a background handoff cannot both be live.

### 5.6 App: control requests and responses

```rust
pub enum StudioControlAction {
    // ... existing ...
    /// Derive and return the current Closing basis fingerprint. Durable state only; mints
    /// nothing and grants nothing. Refusable.
    BeginOverlaySave,
    /// First Save visit. `basis` is the original fingerprint the caller must keep stable
    /// across retries (AG1-001). Acknowledgement is classified and committed in this visit.
    PrepareOverlaySave { basis: [u8; 32], nonce: [u8; 16], body: Vec<u8> },
    /// Second Save visit. The renderer supplies no plan, basis object, seed, receipt or
    /// projection; only the opaque prepared handle.
    FinishOverlaySave(Box<StudioPreparedOverlaySave>),
}
pub enum StudioControlResponse {
    // ... existing ...
    OverlaySaveBasis { target: StudioTarget, basis: [u8; 32], accepted: usize },
    /// The request was recognised as already accepted; nothing new was authored.
    OverlayAcknowledged(StudioOverlayAcknowledgement),
    /// A new acceptance needs detached work; this handle owns the admission and permit.
    OverlaySavePreparation(StudioOverlaySavePreparation),
    OverlaySaved { target: StudioTarget, basis: [u8; 32], accepted: usize },
}
pub enum StudioOverlayAcknowledgement {
    /// Exact retry of a retained accepted entry.
    LocalDraft { target: StudioTarget, basis: [u8; 32], accepted: usize },
    /// Completed-handoff acknowledgement. Records a prior LOCAL transfer outcome only.
    Handoff { target: StudioTarget, outcome: StudioHandoffOutcome },
}
```

`StudioOverlaySavePreparation` and `StudioPreparedOverlaySave` mirror
`StudioInspectionPreparation` / `StudioPreparedInspection`, including the retained-ownership
`Arc<Retained>` and the delivery guard, so the accepted inspection resource and final-delivery
regressions apply unchanged.

None of the Save results carries a projection. The caller refreshes with the existing
`studio_overlay_read`, which already owns the detached reconstruction and the 32 MiB conversion
bound. This keeps the write path free of a second large conversion and keeps the write result
honest about what it proves.

### 5.7 Native surface (designed, not registered)

```ts
// NOT callable until section 12's prerequisites pass. Not in FLIPNOTE-UI-HOOKS until then.
studio_overlay_begin({ server, channel, object? })
  -> { v: 1; kind: "eligible"; basis: string; accepted: number }
   | { v: 1; kind: "ineligible"; reason: string }

studio_overlay_save({ server, channel, object?, basis, nonce, body })
  -> { v: 1; kind: "local-draft"; channel; object; basis; accepted: number; alreadySaved: boolean }
   | { v: 1; kind: "acknowledged-handoff"; channel; object; basis; epoch: string; epochId: string;
       accepted: number }
```

AG1-001 corrections embedded in this contract:

- `basis` is **required** and is the original authoring identifier. It comes from
  `studio_overlay_begin` for a new branch, or from `studio_overlay_read`'s existing `basis` field
  for a branch that already exists. The caller keeps `(basis, nonce, body)` byte-stable across
  retries. It is an identifier, not authority: a new acceptance still derives and verifies
  eligibility from actual durable state, and the request's basis must equal the freshly derived
  fingerprint.
- No `ts` field. The actor supplies the timestamp from its own clock, as the accepted store API
  already does; `exact_retry` compares the envelope, which does not include `ts`, so a retry with a
  fresh actor timestamp is still an exact retry.
- A request that matches no retained entry and whose `basis` is not the currently eligible one
  returns a specific **stale** error. It never acquires today's basis. After acknowledgement
  rollover this is the path a delayed retry takes; the persisted
  `minimum_new_basis_closed_epoch` floor is the second, independent fence if a rewind ever
  reproduces an identical basis.
- `acknowledged-handoff` **records that this exact request was previously transferred into the
  named local destination epoch**. It is not delivery, receipt or settlement. After legitimate
  retirement it does **not** assert that those operations are still present in the current signed
  source or in the pending ledger, and the command performs no source lookup to make such a claim.
  The revision-1 `shared: true` / `receipted: false` booleans are removed because they could not be
  honoured; publication and receipt state come only from ordinary reads and settlement events.
- `studio_overlay_read` gains `transferState: "completed"` for a record holding only a completed
  acknowledgement. `kind: "absent"` keeps its accepted meaning.

Both commands use the existing `InvokeContext`: one UI session generation, one actor instance, one
native operation slot, one `ViewRequest` per `(state, server, target)` and one
`RequestCancellation` spanning both custody visits.

## 6. The pipeline

C-1 makes acknowledgement classification and authority capture cheap enough to run under custody.
That removes revision 1's separate detached-decode-then-authorize round trip from both flows and,
more importantly, puts the acknowledgement decision *before* any basis minting, which is the
AG1-001 correction.

### 6.1 Automatic handoff (Flow H)

| # | Stage | Custody | Work | Owns admission, permit |
|---|---|---|---|---|
| H1 | Admit, classify, authorize, capture | yes | admission + permit; bounded intent read; **structural** decode; eligibility probe; `handoff_authority(device, group, tenure)`; bounded source read; stamp | acquires |
| H2 | Plan | detached | full `decode_vault`; private successor restore via `prepare_vault_source`; `prepare_handoff_detached` (typed admission, preflight, framing bound) | worker |
| H3 | Sign slice, repeated | yes | per-visit wrapper reauthentication, then bounded `sign_next` turns | held |
| H4 | Assemble | detached | `finish()`; encode the Prepared and Completed candidate records | worker |
| H5 | Commit | yes | inventory, rechecks, Prepared, Source, verified flush, Completed | held |
| H6 | Notify | yes | settlement notice, `StudioUpdated`, watch rebinding | releases |

The authority is minted at H1 from the **structurally decoded** branch receipt, under live custody,
through the existing narrow `handoff_authority` interface, and `receipt.verify_current_owner` runs
against the live group and observed tenure. H2 independently performs the **full** decode from the
same captured plaintext, and `prepare_handoff_detached` rejects unless
`self.target == authority.target`, `active.receipt() == &authority.receipt` and
`active.author() == authority.actor` hold against that fully decoded state. A structural-versus-full
divergence therefore cannot survive into a signature.

**Deviation from the reviewer's answer to revision-1 question 3, flagged for rejection.** The
reviewer preferred a separate live visit *after* detached decoding has identified the branch
receipt, and rejected a new opening-receipt authority constructor. This revision keeps the existing
narrow interface and adds no constructor, but it moves the capture into H1 because C-1 makes
identification cheap. The security properties are the same or better: the authority is still minted
under live custody from the branch's own receipt, and the stale window between identification and
capture is removed rather than widened. If the reviewer still wants the separate visit, reinstating
it is a local change that costs one round trip and no other section moves.

### 6.2 Local Save (Flow S)

| # | Stage | Custody | Work | Owns admission, permit |
|---|---|---|---|---|
| S0 | Admit | yes | channel known, membership, request grammar and typed decode, PIX validation and **transient hold** (section 8) | admission + permit |
| S1 | Classify | yes | bounded intent read; **structural** decode; `completed_retry` then `exact_retry` against target, author, envelope and the request's original basis | held |
| S1a | Acknowledge, terminal | yes | flush-only accounted write; return `OverlayAcknowledged`; **no basis minting, no tenure requirement, no source lookup** | releases |
| S1b | Authorize, capture | yes | `studio_closing_basis`; require `fresh.fingerprint() == request.basis`, else **stale**; bounded source read; stamp | held |
| S2 | Plan | detached | full `decode_vault`; ordinary-collision check; `append` (reconstruction) producing the next state | worker |
| S3 | Commit | yes | re-mint the basis and require the same fingerprint; stamp equality; **PIX possession revalidation**; one accounted intent write | releases |

`completed_retry` and `exact_retry` need only entry ids, envelopes, `basis()`, author and target,
all of which the structural decode yields, so S1 is cheap. Nothing in S1 or S1a requires a known
observed tenure, an Open or Closing source, or the owner journal. That is the AG1-001 correction:
a saved acceptance is recognised and acknowledged even when the source has become Open, Faulted or
unavailable, or when tenure has become Unknown.

S3 re-mints the basis under the same custody as the write, preserving the accepted rule "recheck
the same source and authority immediately before the first durable acceptance". A changed Closing
source, a changed fingerprint, a missing or replaced signed close in the owner journal, a changed
owner or tenure, a changed wrapper digest, or a replaced native request discards the planned state:
no durable effect and no second envelope.

### 6.3 Acknowledgement after rollover

The AG1-001 rollover scenario resolves as follows, with three independent fences:

1. Request A arrives after its acknowledgement was replaced and its ordinary entry retired.
   `completed_retry` finds no entry for A's id and `exact_retry` is false: **not an
   acknowledgement**.
2. S1b derives the currently eligible basis and compares it with A's request basis. They differ,
   so S1b returns **stale**. A never acquires today's basis, and no detached work is scheduled.
3. If a rewind ever reproduced a fingerprint equal to A's basis, `StudioOverlayState::append`'s
   `check_basis_floor` against the persisted `minimum_new_basis_closed_epoch` rejects it in the
   detached plan, before any write.

No nonce tombstone store is introduced, and nothing unbounded is retained.

### 6.4 Interrupted Prepared resolution (Flow R)

R1 capture (membership only, no new-edit tenure), R2 detached restore from captured bytes plus
`evidence` and the next state, R3 commit under stamp equality. The synchronous resolver at the
rotation, adoption, shared-write and publication fences is unchanged as a correctness backstop;
the runtime only tries to reach the record first. Section 9.3 gives the shared decision path.

### 6.5 Custody-visit sources

Any `StudioReceiver::run` pass: the native receive driver (`drive_receiver` to `receive_once`,
paced at one second while `studio_pending` is true) and any explicit Studio document or control
request. The runtime's `pending()` contribution keeps the watch signal true while a job has work.
No change to `drive_receiver`'s pacing is proposed; section 13 states what that implies, reported
separately from maximum continuous custody as the reviewer requires.

## 7. Admission, scheduling and fairness

### 7.1 The real admission invariant (AG1-005)

Revision 1 inferred "one occupied slot per actor" from `Option<OverlayJob>`, which does not follow
because `OverlayCancelled` clears the tracked job while the blocking closure still owns its permit.
The invariant is now explicit and enforced by `OverlayAdmission`:

> **I-2.** At most one overlay job per actor is *live*, where live means the admission token is
> still held by the job, by a running detached worker (including one whose waiter was cancelled),
> or by a retained result. A new job for any target, and a native Save preparation, are refused
> until every drained token's strong count has reached zero.

Per-target backoff is a pacing mechanism and is explicitly **not** part of this guarantee. The
shared `preparation_pool()` permit is still globally owned in the accepted way; I-2 is about the
actor's own admission record, which is what the fairness claims in 7.3 rest on.

### 7.2 Reservation precedes every body read

Revision 1's H0 structurally decoded a record with no reservation. Structural decoding still opens,
reads, unseals and allocates a bounded record and its retained seed, so it is not a metadata-only
probe. H0 is therefore folded into H1: the admission token and the permit are acquired **before**
the first bounded read, carried forward, and released immediately when the probe finds no work. A
target with no overlay is memoised in `no_overlay` against the store's `intent_generation` weak
handle, so a quiescent vault is not re-probed each turn and an intent write anywhere invalidates
the memo.

### 7.3 Scheduling

- **Placement.** Heavy stages (H1, H2, H4, H5, S1b, S2, S3, R1 to R3) run only when
  `catchup.replay_ready()` holds, mirroring `replay_step`'s gate. Signing slices (H3) need no new
  permit and no retained source and may run on any background turn, but yield immediately if
  `server.sync.has_epoch_service_interest()`, any watch has inbound, or a background result is
  parked.
- **Bounded slice.** `MAX_SIGNING_TURNS_PER_VISIT = 32` and `SIGNING_SLICE_BUDGET_MS = 250`,
  whichever is reached first. Per the reviewer's answer to question 4 these are an experiment
  configuration, not a responsiveness guarantee: the deadline is checked between signatures and can
  overrun by one whole operation including its authority checks, so the constants must be
  recalibrated against the measured largest admitted individual operation and roster shape
  (section 13). The slice bound constrains H3 only; it does not constrain H1, S1b or S3 basis
  construction, or H5's inventory and verification.
- **Slice exclusivity.** A signing slice runs entirely inside the blocking worker that owns the
  moved `Server` and the vault lease. It contains no `await`, no reentrant store operation and no
  callback that can reach the store. This is the condition the reviewer attached to accepting
  per-visit wrapper reauthentication, and it is a testable structural property (N9).
- **Coalescing and pacing.** One target at a time, round-robin from the watch rail; a hold sets
  `now + 30_000` doubling to a 300,000 ms ceiling, reset on durable progress. `explicit_retry` may
  lower the delay to the floor but may not clear the counter. Repeated UI reads or peer traffic
  cannot start a second attempt.
- **Honest scope.** Idle-only scheduling preserves authoritative priority and does **not** promise
  starvation-free overlay completion under sustained catch-up. Stated as L7.

## 8. Reference protection across detachment (AG1-003, C-4)

R7 is the gap: `Protection::unknown()` drops a pre-hold and the completed scan reinstalls from
durable state only, so a new frame CID that exists solely for an in-flight Save can be deleted
while S2 is detached, after which every S3 stamp check can still pass.

**C-4: bounded job-owned transient holds.**

```rust
// store/creative_references.rs
pub(super) struct Protection {
    generation: Arc<()>,
    pins: Option<CreativeReferences>,
    /// Job-owned holds. NOT cleared by `unknown()`, NOT subtractable by a complete scan, and
    /// consulted by every deletion. Dead owners are reaped on each hold and each delete.
    transient: Vec<(std::sync::Weak<()>, Vec<u8>, BTreeSet<Cid>)>,
}
/// Dropping this releases the hold. It is moved through every detached stage and result, so a
/// cancelled waiter cannot release it before the actual worker finishes.
pub(crate) struct CreativeHold { owner: Arc<()>, protection: SharedProtection }

impl ServerStore {
    /// Bounded: at most MAX_TRANSIENT_HOLD_OWNERS live owners, and transient CIDs count against
    /// the existing MAX_CREATIVE_REFERENCES rail. Exhaustion refuses the Save; it never
    /// silently drops a hold and never marks protection unknown.
    pub(crate) fn hold_creative_transient(
        &self, group: &[u8], cids: BTreeSet<[u8; 32]>,
    ) -> Result<CreativeHold, AppError>;
}
```

`ProtectedBlobs::delete` returns "retained" when the CID is in `pins` **or** in any live
`transient` entry for that group, keeping the mutex guard through unlink as it already does.
`unknown()` and `install()` leave `transient` untouched; a complete scan may still subtract only
from `pins`. The `MAX_CREATIVE_REFERENCES` rail keeps a pathological caller from disabling
reclamation, and exhaustion is a refusal, not an unknown-protection state.

**Ownership.** Flow S mints one `CreativeHold` at S0 for the new operation's CIDs, moves it into
`OverlayOwnership`, and drops it only after S3's intent write crosses its durability barrier, at
which point the durable record names the CID and any completed scan will include it. A cancelled
waiter does not release it; the blocking closure does.

**Second gate.** Before the S3 intent barrier the runtime re-verifies possession with the existing
bounded checks used by ordinary Save (`get_bounded`, `validate_pix`, the 192x144 check, and
`promote_staged_bounded`). Missing bytes refuse with `PixelMissing` **before** the barrier. This is
not the primary mechanism, because re-adding a pin after the bytes have gone is insufficient; it
covers external deletion and storage damage.

**Scope.** Flow H introduces no new CIDs. A retained Active branch's base CIDs and its pending
operations' CIDs are already enumerated by the inventory's Intents arm, and `check_handoff_references`
independently refuses a commit that would release a base reference (R9). Only Flow S's not-yet-durable
CID needed a new mechanism.

## 9. Durable commit with bounded custody (AG1-002)

### 9.1 No graph restore remains on the commit path

Revision 1 kept `checked_studio_source` at H5 and the restore inside `resolve_studio_handoff_with_io`
after the Source write. Both are removed from the runtime's path:

- **Before the Source write.** H2 already restored the installed pristine successor to derive the
  private candidate. It returns `HandoffFacts { source_snapshot_digest, source_physical_bytes,
  storage_protocol_bytes, before_snapshot }` alongside the signing batch. H5 accepts those values
  behind exact authenticated wrapper stamps (`studio_overlay_is_current`), which is precisely the
  reviewer's requirement to "reuse already validated source and candidate data only behind exact
  authenticated wrapper stamps". A stamp mismatch discards the candidate; it never falls back to a
  restore inside the barrier.
- **After the Source write.** `save_studio_source_checked` already returns
  `EpochStudioState { unit, source: Some(SourceVersion { digest, bytes, .. }) }`, where `digest` is
  the complete authenticated plaintext hash of exactly what was written. H5 re-reads the persisted
  record and requires that digest and physical size to match. That authenticates **what actually
  landed**; it is not a worker's claim that a write must have completed. Because the persisted
  bytes are proved identical to the retained candidate, `evidence`, `blob_cids` and `complete` are
  computed from the in-memory candidate with **no second restore**.
- **One algorithm.** `resolve_studio_handoff_with_io` gains an
  `Option<VerifiedPersistedSource>` input rather than a parallel implementation: `None` restores
  from disk (the restart and fence path, unchanged), `Some` supplies a candidate whose digest has
  just been proved equal to the persisted record. The decision table, the barriers, the accounting
  generations, the common write fences and the publication hold are identical in both.

### 9.2 C-3: a bounded inventory strategy

R8 is the remaining term. `EpochStorageScan<'_>` borrows the store for its whole traversal, so a
loop over `step()` is one uninterrupted critical section.

**C-3.** Replace the borrowing scanner with an owned, resumable cursor:

```rust
pub struct EpochStorageCursor { /* directory position, accumulated inventory, progress,
                                  byte and record rails, and the generations captured at start */ }
impl ServerStore {
    pub fn begin_epoch_storage_scan(&mut self, coverage: EpochInventoryCoverage,
                                    references: bool) -> Result<EpochStorageCursor, AppError>;
    /// Runs at most `steps` bounded steps, then returns. The cursor may be parked between
    /// custody visits.
    pub fn step_epoch_storage_scan(&mut self, cursor: &mut EpochStorageCursor, steps: usize)
        -> Result<EpochStorageScanProgress, AppError>;
    pub fn finish_epoch_storage_scan(&mut self, cursor: EpochStorageCursor)
        -> Result<EpochStorageInventory, AppError>;
}
```

Correctness across visits rests on the existing generation discipline, not on new invariants: the
cursor captures `intent_generation` and `studio_generation` at `begin`, and
`finish_epoch_storage_scan` refuses unless both still pointer-match, exactly as
`studio_storage_budget` already refuses a stale inventory. Any intervening write therefore
invalidates a spanning scan, which is fail-closed. The runtime restarts the scan at most
`MAX_INVENTORY_RESTARTS = 3` times per commit attempt; beyond that it returns a hold and applies
backoff. It does **not** fall back to a single-visit unbounded scan, because that would reintroduce
exactly the custody window this finding is about.

`while !scan.step()?.complete {}` call sites elsewhere become `begin` plus a bounded loop plus
`finish` with no behaviour change, so this is a mechanical refactor of a shared seam rather than a
semantic change. It is shared with Agent 3 and every Studio write path, so it is listed for Agent 4
in section 15 and needs its own line in the verdict.

**Eventual progress condition, stated honestly.** A commit completes when the vault is quiescent
for the duration of one scan. Under sustained local writes the commit is held and retried with
backoff; the branch is retained throughout and nothing is published. This is the same class of
condition the reviewer accepted for L2 and for idle-only scheduling.

If C-3's review fails, the fallback is the measured residual: a single-visit scan bounded by the
existing record and byte rails, reported as maximum continuous custody in section 13. The design
does not prefer that fallback, because it contradicts principle 2.

### 9.3 Commit order

Unchanged from the accepted transaction, with the candidate supplied by H4 and the verified reuse
of 9.1:

1. `enter_studio_budget` against an inventory finished in this visit under C-3.
2. `studio_overlay_is_current` for both records and all live context, tenure required.
3. Complete-target comparison and `completed_branch` short-circuit, before source lookup,
   acknowledgement or sync reservation (HANDOFF-001).
4. `check_handoff_references`: candidate plus pending coverage of the branch's base CIDs (R9).
5. Preflight all three replacement peaks and the intent accounting.
6. Re-read the actual intent record and compare its complete authenticated plaintext digest and
   physical size with the captured values (C-2).
7. Write Prepared, barrier 1, retaining the complete branch and ledger.
8. Mint `CheckedHandoffWrite` from the actual re-read bytes; `save_studio_source_checked`,
   barrier 2, one atomic complete replacement.
9. Re-read the persisted source; require the returned `SourceVersion` digest and size to match;
   then `resolve_studio_handoff_with_io` with the verified candidate: authenticate, flush, require
   every manifest entry as an exact current signed envelope with the same complete
   signed-operation digest, replace Prepared with the compact completed acknowledgement, barrier 3.
10. Return the outcome. Publication becomes eligible only now.

Preserved without change: no durable signed prefix, no per-entry retirement, the original full
pending ledger, full signed digests, the retry floor and rollover, the source replacement fence,
the publication fence, the source-required metadata link, and HANDOFF-002's inventory dependency.

## 10. Crash and interruption

Nothing new is durable before barrier 1, so the accepted reopen table governs restart unchanged.

| Interruption | Result |
|---|---|
| Any detached stage, including cancellation | Private candidate lost; no durable byte changed; branch, ledger and pixel hold intact until the actual worker drops them; admission drains; fresh attempt after backoff. |
| Between H3 slices | Same. Signatures exist only inside `StudioHandoffSigning`. |
| Before barrier 1 | No handoff happened. Active, original source. |
| Between barriers 1 and 2 | Prepared with the exact recorded source-before and no branch ids: durably return to Active with the full draft. |
| Between barriers 2 and 3 | Prepared with all exact envelopes and signed-operation digests: flush and complete without reapplying. Flow R restores from captured bytes on a worker, then commits under stamp equality. |
| Step 9 digest mismatch | Do not complete. Leave the durable Prepared hold and let Flow R or the synchronous fence resolve from actual bytes. |
| Partial or conflicting evidence | Retain the full branch, report a hold, guess nothing. Export remains available, section 12. |
| S3 interrupted | One accounted atomic replacement with the existing exact-retry flush; an exact retry re-establishes durability with no second envelope, now recognised at S1 without a fresh basis. |

## 11. Publication, invalidation and events

- The handoff emits no packets; `StudioSavedTransaction::empty()` keeps the two-packet initial Save
  window untouched, and transferred operations reach peers through ordinary current-tail and page
  service once Completed releases the publication hold.
- A local Save emits `RefreshRequired` and `LocalDraftRetained`, never `StudioUpdated`, because no
  shared source changed. Completed emits `StudioUpdated` and `LocalDraftHandedOff`. Neither new
  `StudioSettlementState` variant is a delivery or settlement claim.
- **Generation-aware invalidation.** The replay context key gains the store's `intent_generation`
  `Arc<()>`, and any runtime intent write clears the affected target from `replay.completed`, so a
  draft saved on an already watched epoch is noticed. `studio_storage_budget` already treats that
  generation as authoritative. The runtime's `no_overlay` memo uses the same handle.
- **Replay exclusion (R4).** `studio_replay_evidence` filters `own` with `!state.is_overlay(id)`
  using the structural state, so annotated ids never enter replay selection. `NoEvidence` for a
  failed ordinary Save is unchanged.

## 12. Interface with Agent 2, and native Save exposure

### 12.1 AG1-004: two holds, not one

Revision 1's single `studio_overlay_runtime_hold` made a durably unresolvable Prepared record a
permanent prohibition on export and copy, with no live worker to wait for. It is split:

```rust
/// A live overlay job owns this target right now. Transient, retryable, and the only hold that
/// blocks another writer. False as soon as the job's admission token drains.
pub(crate) fn studio_overlay_live_hold(&self, target: StudioTarget) -> Option<StudioOverlayHold>;
/// The durable record is Prepared. Blocks destructive disposition, source replacement and
/// publication through the EXISTING fences. It does NOT block read-only export or inspection.
pub(crate) fn studio_overlay_transfer_hold(&self, target: StudioTarget) -> bool;
```

Rules agreed with Agent 2:

| Operation | Live hold | Transfer hold (Prepared) |
|---|---|---|
| Read-only inspection | refuse, retryable | **permitted**, unchanged from the accepted inspection path |
| Read-only export of a typed-readable branch | refuse, retryable | **permitted**: separately owned, stamp-checked, without clearing Prepared, cancelling another owner, retiring anything or asserting shared completion |
| Copy into another eligible destination | refuse, retryable | permitted, because a copy is a new authorized typed edit into a different current Open source and does not touch the Prepared destination; it must still respect admission, recovery and reference-inventory limits and must not remove or retire the branch |
| Destructive disposition | refuse, retryable | refuse: completion is uncertain, and disposition requires Agent 2's reviewed transition |

The accepted inspection path already demonstrates that a Prepared branch can be read without
resolving it, so permitting export costs no new capability. N20 requires that an unresolved
Prepared branch with no live worker can be exported while its original metadata, pending envelopes
and publication hold remain unchanged.

### 12.2 What Agent 1 provides

`capture_studio_overlay`, `studio_overlay_is_current`, `studio_overlay_structural`,
`StudioOverlayCapture::plan`, `commit_studio_overlay_state`, `studio_closing_basis`,
`hold_creative_transient` and `CreativeHold`, the `OverlayAdmission` and `OverlayOwnership` model
with the `StudioBackgroundJob::Overlay*` variants, the two holds above, the C-1 decode split with
its call-site table, C-3's cursor, and the native multi-visit pattern.

### 12.3 What Agent 1 requires before registration

P1. A reviewed manual lifecycle: bounded inspect, export, copy-into-current and explicit
disposition, lossless across restart and refusal.
P2. Every `StudioOverlayHold` variant mapped to a user-visible, actionable state, including
`StaleRequest`, `PixelMissing` and a durably unresolvable Prepared branch.
P3. Truthful native results, events and UI-hooks rows for local-only, awaiting receipt,
stale or manual action, recovery and storage refusal.
P4. A live-tenure contract for `observed_owner_tenure_start()`: `None` is fail-closed for authoring
stages, and a returning owner in a new tenure must observe a value differing from its earlier
tenure. The runtime binds `tenure` as an opaque `u64` and relies on "equal value implies the same
continuous tenure".
P5. An explicit statement in `GATE4-AGENT-2-STATUS.md` that P1 to P4 are implemented and reviewed.

Until then: no `#[tauri::command]`, no handler-list entry, no security or capability row, and no
FLIPNOTE-UI-HOOKS "Available now" entry for Save.

## 13. Limits, costs and measurements

Nothing here is measured yet. Required measurements, for Index and Flipnote:

1. Custody time per stage of Flow H at 1, 32 and 256 operations: H1, one H3 slice, H5, separating
   the C-3 inventory from the rest of H5.
2. The same at **maximal accepted shapes**: the 5 MiB plus 1024-byte complete plaintext intent
   record filled by 256 maximal-body operations, a 2 MiB seed, the 64 KiB combined metadata
   ceiling, the maximal accepted projection widths used by the inspection tests (64 Index objects
   with retained alternatives; 999 Flipnote frames with all 1024 conflict fields), and a large
   roster.
3. **The largest admitted individual operation and the relevant roster shape**, as the reviewer
   requires, so the slice constants bound real overrun rather than a title edit. Report the
   worst single-signature time including its repeated authority checks, since the deadline is
   checked between signatures.
4. Retained input and output cost within one permit: captured intent plaintext, captured source
   plaintext (dropped after H2 except its digest, size and derived facts), decoded state, restored
   private successor, signed candidate, and the encoded Prepared and Completed records. Report the
   sum of the accounted bounds. **This is not a measured process heap ceiling.**
5. Flow S custody per stage at 1, 32 and 255 accepted operations, including S1's structural
   classification and the S1b and S3 basis derivations against a maximal Closing source and seed.
6. The effect of C-1 on `checked_epoch_replay_state` and on a five-family inventory of a vault
   containing several large retained branches.
7. C-3: maximum continuous custody per bounded scan slice, the number of visits a full scan needs,
   and the observed restart rate under concurrent writes.
8. Wall-clock time for a 256-operation handoff with the count of custody visits used, **reported
   separately from maximum continuous custody**, because the one-second receive cadence is a
   distinct term.

Limits:

- **L1.** Total handoff latency is a function of the slice bound and the receive cadence, not of a
  single blocking call. Actor responsiveness, not handoff latency, is the acceptance criterion.
- **L2.** The core requires one MLS epoch across every signature. An MLS commit during signing
  invalidates the candidate and the job restarts from H1 after backoff. Accepted by the reviewer
  for this bounded design **under an explicit eventual-stability condition**: paced retries, a
  fully retained branch, no weakening of the MLS check, no signature reuse across contexts, and a
  usable manual escape (which depends on the AG1-004 correction). It does not guarantee automatic
  completion under perpetual membership churn. N10 is the required mid-signing test.
- **L3.** The synchronous fence at the rotation, adoption, shared-write and publication entry
  points still restores the destination source when it finds an unresolved Prepared record with no
  verified candidate available. The runtime resolves proactively so those paths normally find
  nothing. Measure the worst case at 256 operations.
- **L4.** C-1 moves typed-replayability validation from every decode to the detached
  reconstruction, under invariant I-1. Section 5.1 states what that gives up.
- **L5.** The Intents family remains uncached in the scan; with C-1 each entry is a structural
  decode. An `inventory_cache` extension is a separate optimisation and is not proposed, because
  HANDOFF-002's reference path deliberately bypasses that cache.
- **L6 (new).** C-3 gives bounded custody per visit and fail-closed spanning scans, with progress
  conditional on quiescence for the duration of one scan. Under sustained local writes a commit is
  held and retried.
- **L7 (new).** Idle-only scheduling preserves authoritative priority and does not promise
  starvation-free overlay completion under sustained catch-up.

Optional optimisations, not proposed and each needing its own review: Intents inventory cache;
`checked_studio_source` reusing the warm retained source behind its existing authentication.

## 14. Test and mutation plan

### 14.1 Normal regressions

| # | Level | Case | Independent observation |
|---|---|---|---|
| N1 | actor | Index and Flipnote local Save, reopen, read | Draft projection, accepted count, basis, original envelope, nonce, author and timestamp survive restart; canonical source, gate, recovery, owner and Registry bytes unchanged. |
| N2 | actor | **AG1-001 absent-basis retry.** Accept a Save, lose the response, then make the source Open, then Faulted, then unavailable, then make tenure Unknown; retry the exact request each time | Each retry is acknowledged at S1a with `alreadySaved`; no basis is minted; no tenure is required; no source lookup occurs; the record is byte-identical apart from the accounted flush; no second envelope exists. |
| N3 | actor | **AG1-001 rollover.** Real handoff, legitimate retirement of its ordinary entries, a later handoff that replaces the acknowledgement and advances the floor, then a delayed retry of the first request while a newer Closing basis is eligible | Returns stale; no new acceptance; the overwritten-title operation does not reappear; the floor is not consulted as a substitute for the basis comparison; then force the rewind case and show `check_basis_floor` rejects independently. |
| N4 | store | Ordinary failed Apply leaves a pending intent; the same nonce and body are offered as a local Save | Refused with `OrdinaryIntentCollision`; the entry stays unannotated and `NoEvidence`. |
| N5 | actor | Eligible successor installed by a real receipt; automatic handoff | One durable signed source replacement with all 256 operations in original order and timestamps; the full ledger still pending; the base released only after barrier 3. |
| N6 | actor | Peer catch-up after N5 | Peer projection equals local, including conflicts and deletions; the initial-Save window never carried the batch. |
| N7 | store | Interrupt each of the three durable writes and each sync, plus the step 9 digest mismatch | The accepted reopen table is reproduced; no false success, no lost branch, no duplicate effect. |
| N8 | store | Same-size authenticated replacement of the intent record, then of the source record, after capture | Refused at the digest comparison **before any signing turn**; durable bytes unchanged; a fresh attempt succeeds. |
| N9 | store | **Slice exclusivity.** Structural assertion that a signing slice contains no `await`, no reentrant store call and no store-reaching callback | The condition the reviewer attached to per-visit reauthentication holds. |
| N10 | actor | **L2 mid-signing MLS change.** Add a member between slices | No durable signed prefix, no retirement, bounded retry; after the context stabilises the whole branch completes. |
| N11 | actor | Change device key, membership, owner, tenure, channel, mount, numeric server, actor instance between stages | Each produces its own hold with the branch retained; each fixture passes all earlier checks first. |
| N12 | store | **AG1-003 PIX lifetime.** Pause S2; run a complete reference scan and a protected deletion attempt against the new CID; resume S3 | The bytes are retained because the transient hold is live; then repeat with the bytes removed externally and require `PixelMissing` refusal before the intent barrier with unchanged durable bytes. |
| N13 | store | Transient-hold rails | Exhausting `MAX_TRANSIENT_HOLD_OWNERS` or `MAX_CREATIVE_REFERENCES` refuses the Save and never marks protection unknown; dead owners are reaped; an unrelated unreferenced CID still deletes. |
| N14 | actor | **AG1-005 admission.** Cancel a paused job for target A, then immediately attempt a job for target B and a native Save, while A's worker is still held | Both are refused until A's admission token drains; then one is admitted; the shared slot is released exactly once. |
| N15 | actor | Four real jobs and results fill the shared pool | Retryable capacity refusal; no overlay-only pool. |
| N16 | actor | Pause a real H2 or H4 worker; another document's Save, an authoritative checkpoint and another server's progress complete | All three complete while the paused job retains its ownership; the paused job then finishes. |
| N17 | store | **C-3 inventory.** A vault with many cold records; run a commit | Maximum continuous custody per visit is bounded; a concurrent write invalidates a spanning scan and it restarts; after `MAX_INVENTORY_RESTARTS` the commit holds with the branch retained; a quiescent vault completes. |
| N18 | store | **R9 reference mechanisms.** `check_handoff_references` and HANDOFF-002's inventory dependency, separately | A commit that would release a base reference refuses in memory; a missing required-metadata record leaves reclamation disabled after restart. Both retained. |
| N19 | store | PIX through source, intent and recovery transitions across the whole flow | Base-only, pending, superseded and removed-frame CIDs remain enumerable and protected. |
| N20 | store | **AG1-004 unresolved Prepared.** A typed-readable branch whose Prepared evidence is partial and cannot resolve, with no live worker | Read-only export and inspection succeed; original metadata, pending envelopes, Prepared state and the publication hold are unchanged; destructive disposition still refuses; a copy into a different eligible Open destination succeeds. |
| N21 | store | Prepared resolution for each evidence outcome, from captured bytes | Absent returns durably to Active with the full draft; Complete flushes and completes without reapplying; Hold retains everything. |
| N22 | store | Generic page, current-tail, seed service and ordinary retry sending while Prepared | All refuse; after barrier 3 they serve. |
| N23 | store | **C-1 equivalence.** Structural and full decode of the same record | Identical state fields, identical `encode_vault` bytes, identical `handoff_metadata` answers; the structural decoder rejects a missing entry, a duplicate id, a wrong sequence, a wrong author, a changed envelope and trailing data. |
| N24 | store | **C-1 boundary.** A record whose branch is not typed-replayable | Metadata readers succeed structurally; the detached reconstruction refuses; the branch is a hold; ordinary metadata paths do not fail. |
| N25 | actor | Overlay ids absent from replay evidence | `studio_replay_evidence(..).own` contains no annotated id; nothing is applied and nothing is moved to recovery. |
| N26 | actor | Save on an already watched epoch | The intent generation invalidates the replay memo, the `no_overlay` memo and the completed bindings; a second Save is not skipped. |
| N27 | actor | Native session generation, view request and actor instance changed between Save visits and after conversion | Rejected; no partial value; anything already durable stays durable. |
| N28 | store | Maximal accepted shapes | Accepted at each ceiling; one byte over refuses with no partial output and unchanged retained data. |
| N29 | store | Earlier Gate 4 regressions | Solo repeated rotations and restarts, Registry pointer and tail paging, Create after Index rotation, replay and manual recovery, persisted eviction deadlines unchanged. |

### 14.2 Isolated mutations (AG1-TEST-001)

Each mutation names the unique guard, the single test that must execute and fail, and **an
observation at the boundary that guard actually protects**, with earlier conditions independently
valid. Intentionally redundant guards are labelled and tested for early-refusal and resource
behaviour rather than for corrupted durable state.

| # | Guard removed | Test | Intended assertion, at the protected boundary |
|---|---|---|---|
| M1 | Intent digest comparison in `studio_overlay_is_current` (size retained). **Redundant by design** with step 6. | N8 | "a stale plan reached a signing turn": signing-turn counter is non-zero and a detached preparation was consumed, although the final commit still refuses. |
| M2 | Source digest comparison in `studio_overlay_is_current`. **Redundant by design.** | N8 | Same boundary: the job proceeded past H1 on changed source bytes. |
| M3 | `OverlayAdmission::drain` (release on cancellation instead) | N14 | "a second job was admitted while the first worker was still live": two live admission tokens observed. |
| M4 | Per-visit reauthentication before the first `sign_next` of a slice | N8 variant changing bytes between slices | "signing continued across visits on changed records". |
| M5 | The slice count and time budget | N16 | "an authoritative checkpoint did not complete during signing". |
| M6 | The H1 pristine-successor probe. **Redundant by design** with `check_overlay_successor`. | N5 negative variant | "a non-pristine successor consumed a permit and a detached preparation": resource counters show the job was admitted and H2 ran. |
| M7 | Barrier 1 before barrier 2 | N7 | "the source was replaced before Prepared was durable". |
| M8 | `checked_entries` inside `decode_vault_structural` | N23 | "structural decode accepted an inconsistent branch". |
| M9 | The `!is_overlay(id)` filter in `studio_replay_evidence` | N25 | "an annotated id appeared in actual replay evidence": asserted on the evidence set, not on application. |
| M10 | The S1 acknowledgement classification running **before** basis minting | N2 | "an accepted retry demanded a fresh Closing basis": refusal observed with the retained entry present. |
| M11 | The S1b request-basis equality comparison | N3 | "a delayed request acquired the current basis": a new acceptance appears for a request whose original basis differs. |
| M12 | The S3 basis re-mint, fixture removing the matching signed close from the **owner journal** between visits (wrapper stamps unchanged) | N11 variant | "Save committed after its Closing basis became underivable". |
| M13 | `Protection::transient` consultation in `ProtectedBlobs::delete` | N12 | "a live job-owned CID was deleted": the bytes are gone while the job is paused. |
| M14 | The S3 pixel possession revalidation | N12 second half | "a new acceptance named absent pixels". |
| M15 | The step 9 persisted-source digest comparison | N7 | "completion proceeded without authenticating what landed". |
| M16 | The all-or-nothing requirement in the assemble stage | N7 | "a durable signed prefix escaped". |
| M17 | The transfer-hold versus live-hold split (export blocked while Prepared) | N20 | "export of an unresolved Prepared branch was refused with no live worker". |
| M18 | `finish_epoch_storage_scan`'s generation check | N17 | "a spanning scan finished across an intervening write". |
| M19 | The final native delivery recheck for Save | N27 | "an expired delivery returned a converted value". |

Existing `studio-handoff`, `studio-overlay`, `studio-inspection` and `studio-native` mutations are
retained unchanged. Unique anchors, one executed failing test, the intended assertion, byte-exact
restoration and a passing restored regression are required for every entry; an unrelated refusal or
a compilation failure is not detection.

### 14.3 Harness and workflow

New `.github/scripts/check-studio-overlay-runtime-mutations.py`, following
`check-studio-handoff-mutations.py` exactly, with per-mutation logs under
`logs/gate4-overlay-runtime-*.log`. Requested workflow patch for Agent 4: a `runtime` job in
`.github/workflows/studio-handoff.yml` running the focused store and actor suites and then the
mutation script, publishing the logs as an artifact, added to a required workflow so the
integration scenarios cannot stay opt-in. Local execution stays serial: `-j 1`, the existing
per-package test debug override, no concurrent Cargo work, no blanket cleanup.

## 15. Dependencies and integration changes for Agent 4

| File | Change | Note |
|---|---|---|
| `crates/catcoms-replication/src/studio/overlay.rs`, `overlay/handoff.rs` | C-1 `decode_vault_structural` | Core change; needs its own line in the verdict |
| `crates/catcoms-app/src/store/epoch_recovery/inventory.rs` | C-1 structural decode in the Intents arm (reference path keeps every existing check, including `base_blob_cids`); **C-3 resumable cursor** | HANDOFF-002 adjacent and shared with Agent 3; the largest integration item |
| `crates/catcoms-app/src/store/creative_references.rs` | **C-4 transient holds**, `ProtectedBlobs::delete` consulting both tables, `unknown()` and `install()` leaving transient untouched | HANDOFF-002 adjacent; shared with every blob writer |
| `crates/catcoms-app/src/store/epoch_intents.rs` | Structural read entry points; C-2 digest-based unchanged fence; `read_scoped_intent_plain` visibility | Shared with Agent 3 |
| `crates/catcoms-app/src/store/epoch_studio.rs`, `epoch_studio/handoff.rs` | Commit accepts a detached candidate and a `VerifiedPersistedSource`; structural and digest-based reads | Shared with Agent 3's source-writer coordination |
| All `scan_epoch_storage_with_studio` / `scan_studio_receive_inventory` call sites | Mechanical `begin` + bounded loop + `finish` under C-3 | `studio.rs`, `control.rs`, `receiver/catchup.rs`, `receiver/replay.rs`, `creative_references.rs`, Agent 3's repair paths |
| `crates/catcoms-app/src/studio/dispatch.rs`, `control.rs` | New `StudioControlAction` and `StudioControlResponse` variants (5.6) | Central enum edit |
| `crates/catcoms-app/src/studio/receiver/catchup.rs` | New `StudioBackgroundJob` / `StudioBackgroundResult` variants and `OverlayOwnership` (5.5) | Central enum edit |
| `crates/catcoms-app/src/studio/receiver.rs` | `detach`, `complete`, `pending`, `background_step`, `control` gain overlay arms and the admission record | Shared with Agents 2 and 3 |
| `crates/catcoms-app/src/studio/settlement.rs` | Two new `StudioSettlementState` variants (11) | Shared with Agent 2 |
| `crates/catcoms-app/src/studio/replay.rs`, `receiver/replay.rs` | Overlay-id exclusion (R4) and the intent-generation context key (11) | Shared with Agent 2 |
| `apps/desktop/src-tauri/src/lib.rs`, `studio.rs` | Native commands, registration, security and capability rows | **Deferred** to a separate commit gated on 12.3 |
| `docs/INTERFACES.md`, `BACKEND-IMPLEMENTATION.md`, `FLIPNOTE-UI-HOOKS.md`, `GATE4-ACCEPTANCE.md` | Rows for the new seams; UI hooks records Save as unavailable until its prerequisites pass | Agent 4 owns these files |
| `.github/workflows/studio-handoff.yml`, `.github/scripts/` | New runtime job and mutation script (14.3) | Agent 4 owns workflows |

Agent 3 coordination: the runtime is the only new source writer and the only new preparation
consumer for overlays. Signed fault repair must resolve an interrupted Prepared overlay through the
existing fence rather than introducing a competing writer or pool, must respect
`studio_overlay_live_hold` and `studio_overlay_transfer_hold`, and must adopt C-3's cursor at its
own scan call sites.

## 16. Contingency if the core signing review changes the split

- H1's authority capture and H2's `prepare_handoff_detached` are the only stages bound to the
  split; a changed capture moves work between them without changing sections 8, 9, 10 or 12.
- H3's slice bound is independent of how many operations a call signs; only the constants and
  measurement 13.3 change.
- If the split is rejected and the core returns to one batch call, H2 to H4 collapse into a single
  detached stage; L1 improves and the mid-slice authority recheck weakens, which would need its own
  review.
- Sections 5.1, 5.3, 5.4, 6.2, 6.3, 7, 8, 9.2, 11 and 12 do not depend on the split at all.

## 17. Reviewer answers adopted, and what remains open

Adopted as binding, with the reviewer's stated conditions:

1. **Per-visit wrapper reauthentication**, conditional on slice exclusivity; both complete wrappers
   and their physical sizes are compared before the first signature of each visit, every visit
   starts fresh, and the core's per-signature live checks are retained. N9 tests the condition.
2. **The structural and full decode split**, with full validation as the default, per-consumer
   audit, retained typed seed reference enumeration, and structural success as neither a display
   result nor a write capability. Invariant I-1 in section 5.1 carries the writer obligation.
3. **The existing narrow authority interface**, with no new constructor. Section 6.1 records one
   deviation: the capture moves into H1 because C-1 makes identification cheap, removing a round
   trip rather than widening a window. Flagged for rejection.
4. **Both a count limit and an elapsed-time budget**, calibrated against admitted byte and work
   bounds, with the one-operation overrun acknowledged, the largest admitted operation and roster
   measured, and the receive cadence reported separately from maximum continuous custody.
5. **Restart on MLS change**, under the explicit eventual-stability condition, with N10 required
   before Gate 4 acceptance and the manual escape provided by Agent 2 including the AG1-004 fix.

Open for this re-review:

- Is C-3 the right correction for AG1-002's inventory term, given that it changes a seam shared
  with Agent 3 and every Studio write path, or should the fallback measured residual be accepted
  instead?
- Is the section 6.1 deviation from answer 3 acceptable?
- Is the transient-hold rail in section 8 (owners and CIDs counted against
  `MAX_CREATIVE_REFERENCES`, exhaustion refusing the Save) the right failure mode, or should
  exhaustion instead mark protection unknown?
- Does the copy-while-Prepared rule in 12.1 need a stronger fence than "a different current Open
  destination", given Agent 2 owns copy semantics?

## 18. Re-review request

Fill `[FULL_HEAD_SHA]` with the commit that adds this revision before sending.

```text
Review type: design re-review after REQUEST CHANGES.
Base: ac12822f04337b3e388618f81ce4a4b29d1e9b87. Head: [FULL_HEAD_SHA].
Compare: https://github.com/Thalpy/Mewtual/compare/ac12822f04337b3e388618f81ce4a4b29d1e9b87...[FULL_HEAD_SHA]
Scope/evidence: docs/GATE4-AGENT-1-DESIGN.md revision 2 and docs/GATE4-AGENT-1-STATUS.md.
Design only: no production code, no test and no new measurement exists.
Dependencies unchanged: e65bfd8 is still unreviewed; Agent 2's manual lifecycle remains a
registration prerequisite; native Save stays unregistered and out of FLIPNOTE-UI-HOOKS.

This revision answers AG1-001 to AG1-005 and AG1-TEST-001. Section 0 maps each finding to its
correction. Verify each against the code, not against the prose.

AG1-001: acknowledgement classification now runs under custody on the STRUCTURAL decode, before
any basis minting, because completed_retry and exact_retry need only entry ids, envelopes,
basis(), author and target. Confirm that is true of the actual code. Check that S1a requires no
tenure, no Open or Closing source and no owner journal, so an accepted retry is acknowledged when
the source has become Open, Faulted or unavailable or tenure is Unknown. Check the native request
now carries its original basis, that ts is actor-supplied and absent from the envelope, that an
unmatched request whose basis differs returns stale rather than acquiring today's basis, and that
check_basis_floor remains an independent second fence after a rewind. Confirm the
acknowledged-handoff wording no longer promises current source or pending-ledger presence, and
that no source lookup was reintroduced to make wording true. Attack N2 and N3.

AG1-002: no graph restore should remain on the runtime commit path. H2 returns the installed
successor's facts and they are accepted only behind exact authenticated wrapper stamps; after the
Source write, the persisted record is re-read and its complete authenticated plaintext digest and
size must equal the SourceVersion the writer returned, after which evidence, blob_cids and
complete are computed from the retained candidate with no second restore;
resolve_studio_handoff_with_io takes an Option<VerifiedPersistedSource> so there is one algorithm,
not two. Challenge whether digest equality against what was written is genuinely authenticating
what landed rather than a worker assertion. Then challenge C-3: a resumable inventory cursor whose
finish refuses unless both generations still pointer-match, a bounded restart count, and a hold
rather than a fallback to an unbounded single-visit scan. Judge whether that is the right bounded
strategy for a seam shared with Agent 3 and every Studio write path, or whether the measured
residual fallback is preferable. Check L6's eventual-progress condition is stated honestly.

AG1-003: Protection::unknown drops pins and a completed scan reinstalls from durable state, so the
correction is a transient hold table that unknown() and install() do not touch and that a complete
scan may not subtract, owned by an Arc moved through every detached stage so a cancelled waiter
cannot release it, plus an S3 possession revalidation that refuses before the intent barrier.
Verify the rails (owner count and MAX_CREATIVE_REFERENCES) fail by refusing the Save rather than
by disabling reclamation, that ProtectedBlobs::delete consults both tables under one guard through
unlink, and that Flow H genuinely needs no equivalent because base and pending CIDs are already
durable. Attack N12 and N13.

AG1-004: the single hold is split into a transient live-operation hold and a durable transfer
hold. Read-only export and inspection of a typed-readable Prepared branch are permitted without
clearing Prepared, cancelling an owner, retiring anything or asserting completion; destructive
disposition still refuses; copy into a different current Open destination is permitted. Judge
whether the copy rule needs a stronger fence given Agent 2 owns copy semantics. Attack N20.

AG1-005: the invariant is now one LIVE admission per actor, enforced by a token held by the job,
by every detached worker including a cancelled one, and by every retained result, with drained
Weak handles blocking admission until their strong count reaches zero. Native Save participates
through StudioReceiver::control. The permit and token are reserved before the first bounded read
and H0 is folded into H1, with a no_overlay memo keyed on the store's intent generation. Check
that this is a real guarantee and not a restatement, and that the fairness claims rest on it.

AG1-TEST-001: every mutation now observes at the boundary its guard protects, with earlier
conditions independently valid. M1, M2 and M6 are labelled intentionally redundant and assert
early refusal and resource consumption rather than corrupted durable state; M9 asserts the
annotated id is absent from actual replay evidence rather than that it was applied; M11 and M12
replace the revision-1 M12, with M12's fixture removing the matching signed close from the owner
journal so the wrapper stamps cannot mask it. Verify none of the nineteen can pass for an
unrelated reason.

Audit corrections to confirm: R1's ordinary-read claim is now conditional on the source wrapper's
link byte and a retained Active or Prepared branch, with the unconditional cost located in
checked_epoch_replay_state and the uncached Intents arm of the five-family scan; the fixed read
count is withdrawn; C-1 is described as branch-replay-free rather than projection-free, with
base_blob_cids and its typed seed work retained; the C-1 trust argument is replaced by invariant
I-1 on the writers, with AEAD authentication explicitly not treated as proof of typed admission;
R5's "strictly stronger" claim is withdrawn and replaced by a cost justification; R3 records that
close_for is historical evidence and that observed-tenure verification remains necessary; R4 is
labelled selection hardening; R9 separates check_handoff_references from HANDOFF-002's inventory
dependency. Confirm each correction is accurate and complete.

Answer the four questions in section 17. Return PASS for this bounded design, or numbered findings
with severity, file/line, trigger, impact, evidence and required correction, and say explicitly
which revision-1 findings remain open. A PASS accepts the design only: no implementation, no
measurement and no native Save exposure is claimed, Agent 2's reviewed lifecycle remains a
registration prerequisite, the signing split still needs its own implementation review, and full
Gate 4 acceptance remains with Agent 4.
```
