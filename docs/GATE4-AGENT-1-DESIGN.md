# Gate 4 Agent 1: local Save and automatic handoff runtime

Status: **design proposal awaiting adversarial review. No production code is written.**
Design base: `a052f78b62a549702686a8741932f1d2f8c98773` on `Thalpy/Mewtual` / `Create-suite-2`
(verified equal to `origin/Create-suite-2` at the time of writing; PR #26 is open with no
submitted reviews). Scope is [Agent 1 of the four handoffs](GATE4-AGENT-HANDOFFS.md).
Progress and evidence are tracked in [GATE4-AGENT-1-STATUS](GATE4-AGENT-1-STATUS.md).

**Dependency that is not satisfied.** The [core signing split](GATE4-HANDOFF-SIGNING-REVIEW.md)
at `e65bfd8` is still awaiting the user's adversarial review; `gh pr view 26` returns an empty
review list, and HANDOVER records that no CI run for `e65bfd8` had been returned. This design
consumes `handoff_authority`, `prepare_handoff_detached`, `sign_next` and `finish` as its stage
boundaries. If that review changes those signatures or their order, sections 5.2 and 6 change
with them; section 14 records the contingency. Nothing here presumes that PASS.

Accepted work this design must not weaken: the Closing-overlay foundation (`b1b0ec9`,
OVERLAY-TEST-001 closed), the handoff design (HANDOFF-001 closed), the bounded core/store
handoff implementation (`62f06d4`, HANDOFF-002 closed), the detached-inspection proposal
(`0b28f06`) and its read-only implementation (INSPECTION-TEST-001 closed), and the earlier
combined scheduling block (`6b71d96`). The SUC, TAIL, NATIVE, OVERLAY, HANDOFF and INSPECTION
closures stand. Section 3 reports regressions that are already present in the accepted code;
it does not reopen any closure, and each one is stated as an observation with its exact call path.

## 1. Outcome and boundary

A real actor must be able to:

1. durably accept an explicit local overlay operation on an eligible Closing document,
2. retain and read it across restart,
3. automatically transfer the complete branch when the verified eligible successor arrives,
4. all of the above while unrelated actor work, authoritative discovery/receive and another
   server keep progressing, and while a large preparation is paused.

Out of scope here: manual inspect/export/copy/disposition and stale-base handling (Agent 2),
preview-based local work (Agent 2), repeated-owner tenure authority (Agent 2), signed fault
repair (Agent 3), integration and full-gate acceptance (Agent 4).

Native `studio_overlay_save` is **designed and implemented behind the actor/store seam but not
registered**. Its `#[tauri::command]`, command registration and security/capability rows land in
a separate identifiable commit owned by Agent 4, gated on section 12's prerequisites.

## 2. What was audited

Read in full: `crates/catcoms-replication/src/studio/overlay.rs`,
`studio/overlay/handoff.rs`, `studio/overlay/handoff/preparation.rs`,
`studio/epoch/handoff.rs`, the settlement entry `prepare_settlement`;
`crates/catcoms-app/src/store/epoch_studio.rs` (budget, `checked_studio_source`,
`save_studio_source{,_reusing,_checked}`, `load_studio_epoch`),
`store/epoch_studio/{handoff,overlay,preparation,source}.rs`,
`store/epoch_intents.rs` (+ `overlay.rs`, `inspection.rs`),
`store/epoch_recovery/inventory.rs` (scan step), `store/epoch_owner.rs` (`close_for`);
`crates/catcoms-app/src/studio.rs`, `studio/{overlay,inspection,control,dispatch,replay}.rs`,
`studio/receiver.rs`, `studio/receiver/{catchup.rs,catchup/preview.rs,replay.rs}`,
`crates/catcoms-app/src/actor.rs` Studio arm and job loop, `registry_catchup::preparation_pool`;
`apps/desktop/src-tauri/src/studio.rs` and `studio/{inspection,requests}.rs`.
Measurements quoted are the existing ones in [P1-PERFORMANCE](P1-PERFORMANCE.md); **this design
pass executed no Cargo command and produced no new measurement.**

## 3. Audit findings that shape the design

These are properties of the code at the design base. R1 and R2 are the reason a naive
"move the existing call into `spawn_blocking`" cannot satisfy the Agent 1 outcome.

### R1 (blocking, in scope): decoding a retained branch performs a full typed reconstruction

`StudioOverlay::decode_vault` ends with `out.read(ledger)?`
([overlay.rs:419](../crates/catcoms-replication/src/studio/overlay.rs#L419)), and `read`
replays every accepted operation through `local_policy`, `prepare_local_write`, an Automerge
clone/commit per entry, `validate`, a full projection read and `recovery::preflight`
([overlay.rs:276-328](../crates/catcoms-replication/src/studio/overlay.rs#L276-L328)).
`EpochIntentState::decode` therefore reconstructs the whole branch whenever an Active or
Prepared overlay is present. This explains why the profile's `decode_ms` (13,171 ms Flipnote /
9,844 ms Index at 256 operations, debug) is essentially equal to `draft_ms`.

Every one of these custody-path callers pays that cost today, once per call:

| Call path | Reached from |
|---|---|
| `check_studio_intent_link` -> `load_epoch_intents` | `load_studio_epoch`, `checked_studio_source`, `capture_studio_source`, `studio_source_bytes_match` |
| `read_epoch_intent_record` -> `EpochIntentState::decode` | `checked_epoch_replay_state`, `write_prepared_intents`, `check_studio_handoff_write`, `check_studio_handoff_publication`, retirement |
| Five-family inventory scan, `EpochRecordKind::Intents` arm | every `studio_storage_budget` acquisition; the Intents family is **not** covered by `inventory_cache` (`cacheable` is `Registry \| Studio` only, [inventory.rs:570](../crates/catcoms-app/src/store/epoch_recovery/inventory.rs#L570)) |

Consequences at the design base, with a 256-operation retained branch:

- an **ordinary** `studio_read`/`studio_list` of that document reconstructs the branch under
  actor/vault custody (via `with_studio_source` -> `studio_source_bytes_match` or
  `load_studio_epoch`), even though the reader never asks for the draft;
- the accepted synchronous `handoff_studio_overlay_with_io` reconstructs it at least four times
  (`checked_epoch_replay_state` twice, the pre-write `read_epoch_intent_record`, and each
  `persist_handoff_intents`), plus once per inventory scan;
- `studio_source_is_warm` is cheap, but the byte match that follows it is not.

### R2 (blocking, in scope): local acceptance reconstructs the whole branch under custody

`StudioOverlay::append` validates by building `staged` and calling `staged.read(ledger)`
([overlay.rs:244](../crates/catcoms-replication/src/studio/overlay.rs#L244)). Saving the Nth
operation therefore replays N operations inside `write_studio_overlay_intent`, under the actor
lease and the sole store. Local Save cost grows linearly with the retained branch and reaches
the `draft_ms` column at the operation ceiling.

### R3 (in scope): the basis is re-derived on every non-retry Save

`save_studio_closing_overlay_with_io` calls `prepare_closing_overlay` on a freshly checked
source for every accepted append, which runs `prepare_settlement` ->
`checkpoint_for_close` ([settlement.rs:64-95](../crates/catcoms-replication/src/studio/epoch/settlement.rs#L64)).
This is required by the accepted rule "recheck the same source and authority immediately before
the first durable acceptance" and is bounded by the Closing source, not by the branch. It stays,
but it must be measured at the maximal accepted seed/source shape, not only the 1.5 KB fixture.

### R4 (in scope): replay does not exclude overlay-annotated intent ids

`studio_replay_evidence` builds `own` from `load_epoch_intents(..).pending()` filtered by author
only ([replay.rs:94-99](../crates/catcoms-app/src/studio/replay.rs#L94-L99)); it does not filter
`state.is_overlay(id)`. The accepted handoff design requires that "Active/prepared overlay ids
must be excluded explicitly before future worker integration". Today those ids reach `choose`,
which returns `NoEvidence` because no recovery snapshot holds them, so nothing is replayed by
accident at the design base; the explicit exclusion is still required before the runtime makes
overlay ids common, and it needs its own isolated mutation (M10).

### R5 (in scope): the durable commit re-reads and re-encodes more than its fences require

`handoff_studio_overlay_with_io` proves "the intent record did not change" by decoding it again
and comparing `actual.encode(&scope)` to `original`
([handoff.rs:236-239](../crates/catcoms-app/src/store/epoch_studio/handoff.rs#L236-L239)).
Comparing the authenticated plaintext digest is both cheaper and strictly stronger (it also
catches a change that re-encodes identically). Section 8 replaces the comparison, not the fence.

### R6 (reported, not this agent's scope): the Prepared source-replacement fence is synchronous

`resolve_studio_handoff_with_io` is called from the rotation, adoption and shared-write fences.
It restores the destination source and computes `evidence` under custody. Section 9 keeps that
synchronous fence exactly as accepted (correctness before latency) and adds a runtime that
resolves Prepared proactively so the fences normally find nothing to do. The residual cost on
the fence paths is stated as a limit in section 13, not hidden.

## 4. Design principles

1. **One algorithm, two drivers.** The existing synchronous entry points keep working and become
   the inline composition of the same stage functions the runtime schedules, exactly as the core
   split kept `prepare_handoff` as a compatibility adapter over the detached stages. There must
   be no second Save or handoff algorithm to drift from the reviewed one.
2. **Custody is spent on evidence, not on computation.** Under the lease the runtime reads,
   authenticates, hashes, checks live authority, accounts and writes. It decodes a branch,
   reconstructs a graph, restores a private candidate, applies typed policy or assembles a
   manifest only on a detached worker.
3. **Every detached result is a proposal.** It becomes durable only after the next custody visit
   reauthenticates the exact wrapper bytes it was derived from and rechecks live authority.
   A worker never asserts that a prior write must have happened.
4. **The permit is the unit of admission.** One shared `preparation_pool()` permit is reserved
   before the first read and owned until the job's last owner drops it. No new pool, no
   uncharged cache, no worker-owned device or MLS secret.
5. **Refusal retains work.** Every hold, cancellation, stale stamp, capacity refusal and
   authority change leaves the complete branch and the ordinary ledger untouched.

## 5. Concrete APIs

New leaf modules owned by Agent 1:

```
crates/catcoms-replication/src/studio/overlay/structural.rs   (decoder split, section 5.1)
crates/catcoms-app/src/store/epoch_studio/overlay_capture.rs  (capture + currency stamp)
crates/catcoms-app/src/store/epoch_studio/overlay_commit.rs   (staged commit seams)
crates/catcoms-app/src/studio/overlay/runtime.rs              (job state machine)
crates/catcoms-app/src/studio/overlay/save.rs                 (native two-visit local Save)
apps/desktop/src-tauri/src/studio/overlay.rs                  (unregistered native surface)
```

### 5.1 Core: structural decode, separate from reconstruction (change C-1)

R1 is fixed by giving the decoder two named entry points instead of one. The full entry point is
unchanged in behaviour and remains the default:

```rust
// crates/catcoms-replication/src/studio/overlay.rs
impl StudioOverlay {
    /// Existing behaviour: structural checks, then full ordered reconstruction.
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
    /// Same bounds, entry/ledger membership, sequence, author and envelope checks, and the
    /// same canonical re-encode equality. It does NOT replay the branch. Vault-sealed local
    /// bytes only; no network- or renderer-supplied bytes may reach it.
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
}
impl StudioOverlayState {
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError>;
}
```

`decode_vault_structural` keeps, in this order: the `MAX_EXTENSION` bound, the version tag, the
nested seed and metadata bounds computed before allocation, the complete target derivation and
its receipt/ledger scope equality, `checked_entries` (ledger membership, no duplicate id,
`sequence == index + 1`, `next_sequence == len + 1`, author equality, exact envelope hash,
timestamp bound, non-empty, `<= MAX_STUDIO_OVERLAY_OPS`), the Prepared and Completed field
decoding with their own bounds and target equality, and
`encode_vault(ledger)? == bytes`. It drops only `out.read(ledger)?`.

What this gives up, stated plainly: a record that is authenticated, canonical and structurally
consistent but whose branch is **not typed-replayable** is now accepted by the custody-path
readers and rejected later, on the detached reconstruction. Such a record cannot be produced by
a peer, a renderer or the network: the `.intents` file is AEAD-sealed with the local database key
and scope-bound, and it is written only by `write_prepared_intents` after `append`, which already
required a successful `read`. The failure it could mask is a local bug or storage damage, and the
consequence is that the branch is held and surfaced through the manual lifecycle rather than
making every ordinary read of that document fail. Reconstruction still happens on every path that
actually needs the projection.

Call sites moved to structural decoding (each one is enumerated so the reviewer can check that
nothing needing a projection was moved):

| Call site | Why structural is sufficient |
|---|---|
| `EpochStorageScan::step_inner`, Intents arm | needs the record footprint and, for reference scans, `handoff_metadata().target()`, `overlay().base_blob_cids()` and `pending()` operation CIDs. `base_blob_cids` uses `base.graph()` (the seed), not the replay; it stays. |
| `check_studio_intent_link` | needs only `handoff_metadata().check_target(target)` |
| `checked_epoch_replay_state` | needs the ledger, accounting and `overlay()` identity; never a projection |
| `write_prepared_intents` old-record read | needs the previous physical size only (see C-2) |
| `check_studio_handoff_write` / `check_studio_handoff_publication` | need `is_prepared`, `has_completed`, `check_target`, `evidence`, `pending`/`is_overlay` |
| `epoch_intents/retirement.rs` accounting reads | need the ledger and overlay id membership |
| The runtime probe H0 / S0 | deliberately weaker than the core's eligibility rule |

Call sites that keep full `decode_vault`, all of them detached or already detached:
`EpochIntentState::local_draft` (inspection rebuild, detached), the runtime's detached plan
stage (H2/S2), and Agent 2's export/copy preparation.

`StudioOverlayState::decode_vault` remains the only entry point used when the caller will return
a projection to a user, so no display path loses validation.

### 5.2 Core: no other change required

`StudioHandoffAuthority`, `prepare_handoff_detached`, `StudioHandoffSigning::{sign_next, finish}`
and `StudioHandoffCandidate::into_parts` are consumed exactly as implemented at `e65bfd8`.
The runtime supplies what the signing note says it must supply and the core deliberately does
not: shared permit ownership, numeric server, actor/sync incarnation, mount, session/request,
full source and intent wrapper stamps, actual reference inventory and the durable commit.

One new store-side derivation is needed because the renderer may not supply a close record:

```rust
// crates/catcoms-app/src/store/epoch_studio/overlay.rs
impl ServerStore {
    /// Derive the Closing basis from actual durable state only. Requires a Closing source,
    /// its current receipt head, and the matching signed close from the saved owner journal
    /// (`EpochOwnerReceiptState::close_for`). No caller supplies a close, seed, receipt,
    /// author or tenure. Returns None when the document is not Closing.
    pub(crate) fn studio_closing_basis(
        &mut self, server: u64, group: &ServerGroup, target: StudioTarget,
        device: &MlsDevice, tenure: Option<u64>, budget: &mut EpochStudioBudget,
    ) -> Result<Option<StudioClosingOverlayBasis>, AppError>;
}
```

`prepare_studio_closing_overlay` keeps its current signature for existing tests and becomes a
thin wrapper that accepts an explicit close; the runtime never uses that wrapper.

### 5.3 Store: capture and currency (one seam for all overlay work)

```rust
// crates/catcoms-app/src/store/epoch_studio/overlay_capture.rs

/// Public context and record identity. No plaintext, no key, no store handle, no Server.
pub(crate) struct StudioOverlayStamp {
    mount: Arc<()>,
    server: u64,
    document: LogicalDocument,
    target: StudioTarget,          // complete target, including the Flipnote channel
    actor: DeviceId,
    actor_key: Vec<u8>,            // local device signature public key
    owner: DeviceId,
    mls: u64,
    tenure: u64,                   // independently observed; None is refused before capture
    incarnation: RegistrySyncInstance,
    /// Absence is explicit and distinct from an unreadable record.
    intent: Option<(blake3::Hash, u64)>,
    source: Option<(blake3::Hash, u64)>,
}

/// Authenticated zeroizing plaintext plus the stamp. Carries no device or MLS secret,
/// no editable installed epoch, no source writer and no budget.
pub(crate) struct StudioOverlayCapture {
    stamp: StudioOverlayStamp,
    group: Vec<u8>,                       // group id only
    intent: Option<Zeroizing<Vec<u8>>>,
    source: Option<Zeroizing<Vec<u8>>>,
    request: StudioOverlayRequest,
}

pub(crate) enum StudioOverlayRequest {
    /// One bounded, already typed-decoded local operation and its original timestamp.
    Save { basis: StudioClosingOverlayBasis, intent: LocalIntent, ts: u64 },
    /// Automatic transfer of the retained branch named by this saved basis fingerprint.
    Handoff { basis: [u8; 32] },
    /// Resolve an interrupted Prepared record.
    Resolve,
}

impl ServerStore {
    /// The caller already owns one shared preparation permit and live membership custody.
    /// Reads are bounded (`MAX_RECORD_BYTES` for intents, `MAX_RETAINED_BYTES` for the source),
    /// use the ordinary parent-directory and regular-file restrictions, and do NOT decode.
    pub(crate) fn capture_studio_overlay(
        &self, context: &StudioOverlayContext, request: StudioOverlayRequest,
    ) -> Result<StudioOverlayCapture, AppError>;

    /// Reacquired custody. Compares mount pointer, numeric server, complete target, document,
    /// actor, actor key, owner, MLS epoch, observed tenure, and BOTH full wrapper digests with
    /// their physical sizes. Captured absence must remain absence. It does not decode.
    pub(crate) fn studio_overlay_is_current(
        &self, context: &StudioOverlayContext, stamp: &StudioOverlayStamp,
    ) -> Result<bool, AppError>;
}
```

`StudioOverlayContext` is the live-side twin of `studio::inspection::Context`: it is built inside
`sync.with_registry_context` and carries group id, device id, device public key, designated
owner, MLS epoch, observed tenure, numeric server, mount and the registry sync instance. It is
built once per custody visit and is never carried across a detach.

Deliberate difference from `capture_studio_source`: this capture does **not** call
`check_studio_intent_link` (which would decode) and does **not** evict `self.studio_source`.
The link requirement is re-derived from the captured source bytes on the detached worker and
enforced again at the commit visit through the unchanged `check_studio_handoff_write`.

### 5.4 Store: the detached plan

```rust
// crates/catcoms-app/src/store/epoch_studio/overlay_capture.rs

pub(crate) struct StudioOverlayPlan {
    stamp: StudioOverlayStamp,
    state: EpochIntentState,          // fully decoded and reconstructed here
    outcome: StudioOverlayPlanned,
}

pub(crate) enum StudioOverlayPlanned {
    /// `state` already contains the appended overlay; `draft` is its full projection.
    SaveAccepted { draft: StudioLocalDraft },
    /// Exact saved acceptance: no new envelope, no second draft, flush-only commit.
    SaveExactRetry { draft: StudioLocalDraft },
    /// The saved branch was already handed off; return the stored outcome, flush only.
    AlreadyHandedOff { outcome: StudioHandoffOutcome },
    /// Private restored successor for `prepare_handoff_detached`, plus its snapshot digest.
    HandoffSource { source: StudioEpoch, before: [u8; 32] },
    /// Resolution decision computed off custody; `source` is the restored destination.
    Resolved { source: StudioEpoch, evidence: StudioHandoffEvidence },
    /// Nothing to do, or a retained hold. Never a partial effect.
    Hold(StudioOverlayHold),
}

/// Every variant keeps the complete branch and the ordinary ledger.
pub(crate) enum StudioOverlayHold {
    NoOverlay, NotClosing, WrongAuthor, WrongTarget,
    BasisChanged, PreparedPending, SuccessorNotPristine, SuccessorMissing,
    OrdinaryIntentCollision, TenureUnknown, Structural(String),
}

impl StudioOverlayCapture {
    /// Runs only on a blocking worker. Owns authenticated plaintext, public context and the
    /// permit. Full `decode_vault`, ordered reconstruction, typed admission and private
    /// candidate restoration happen here.
    pub(crate) fn plan(self) -> Result<StudioOverlayPlan, AppError>;
}
```

`plan` restores the private successor with the existing key-free
`StudioEpoch::prepare_vault_source(snapshot, group_id, target, actor, owner)`, the same
constructor `StudioSourceCapture::rebuild` already uses. It never constructs a `ServerGroup`,
never reads a device key and produces no signature.

### 5.5 App: the job state machine

```rust
// crates/catcoms-app/src/studio/overlay/runtime.rs

pub(in crate::studio) struct OverlayRuntime {
    job: Option<OverlayJob>,
    detached: bool,                 // a worker currently owns the job's stages
    next_attempt_at: BTreeMap<StudioTarget, u64>,
    backoff: BTreeMap<StudioTarget, u64>,
    selection: usize,
    notices: SettlementNotices,
}

struct OverlayJob {
    context: OverlayJobContext,     // mount, server, incarnation, target, request kind
    permit: OwnedSemaphorePermit,   // the SAME permit from capture to release
    stage: OverlayStage,
}

enum OverlayStage {
    Captured(Box<StudioOverlayCapture>),          // ready to detach (H2/S2/R2)
    Planned(Box<StudioOverlayPlan>),              // ready for a custody visit (H3/S3/R3)
    Authorized(Box<AuthorizedHandoff>),           // ready to detach (H4)
    Signing(Box<StudioHandoffSigning>, Box<StudioOverlayPlan>),   // custody slices (H5)
    Assembling,                                   // detached finish is running (H6)
    Assembled(Box<StudioHandoffCommit>),          // ready for the commit visit (H7)
}
```

New background job and result variants, mirroring `StudioBackgroundJob::Prepare` exactly
(the permit is moved into `spawn_blocking`, so a cancelled waiter cannot refund it):

```rust
pub(crate) enum StudioBackgroundJob<T: MeshTransport> {
    // ... existing variants ...
    OverlayPlan(Box<StudioOverlayCapture>, OwnedSemaphorePermit, OverlayJobContext),
    OverlayPrepare(Box<AuthorizedHandoff>, OwnedSemaphorePermit, OverlayJobContext),
    OverlayAssemble(Box<StudioHandoffSigning>, Box<StudioOverlayPlan>,
                    OwnedSemaphorePermit, OverlayJobContext),
}
pub(crate) enum StudioBackgroundResult {
    // ... existing variants ...
    OverlayPlanned(OverlayJobContext, Result<(Box<StudioOverlayPlan>, OwnedSemaphorePermit), AppError>),
    OverlayPrepared(OverlayJobContext, Result<(Box<StudioHandoffSigning>, Box<StudioOverlayPlan>, OwnedSemaphorePermit), AppError>),
    OverlayAssembled(OverlayJobContext, Result<(Box<StudioHandoffCommit>, OwnedSemaphorePermit), AppError>),
    OverlayCancelled(OverlayJobContext),
}
```

`StudioReceiver::detach` gains overlay arms and sets `self.overlay.detached = true`;
`StudioReceiver::complete` clears it and parks the result, exactly as `Prepared` does.
`OverlayCancelled` drops the job, retains the branch and applies backoff; it never recreates
the permit, which the still-running blocking closure owns until it finishes.

### 5.6 App: control requests and responses

```rust
pub enum StudioControlAction {
    // ... existing ...
    /// First visit of the native local Save. Returns a detached preparation handle.
    PrepareOverlaySave { nonce: [u8; 16], body: Vec<u8>, ts: u64 },
    /// Second visit. The renderer never supplies a plan, basis, seed, receipt or projection.
    FinishOverlaySave(Box<StudioPreparedOverlaySave>),
}
pub enum StudioControlResponse {
    // ... existing ...
    OverlaySavePreparation(StudioOverlaySavePreparation),
    OverlaySaved(StudioOverlaySaveResult),
}
/// Distinct local-only, shared-pending and completed results. Never a settlement claim.
pub enum StudioOverlaySaveResult {
    LocalDraft { target: StudioTarget, draft: StudioLocalDraft, already_saved: bool },
    HandedOff { target: StudioTarget, outcome: StudioHandoffOutcome },
}
```

`StudioOverlaySavePreparation` and `StudioPreparedOverlaySave` mirror
`StudioInspectionPreparation` / `StudioPreparedInspection`, including the retained-permit
`Arc<Retained>` and the `StudioInspectionDelivery`-style final-delivery guard, so the accepted
inspection resource and final-delivery regressions apply unchanged to Save.

### 5.7 Native surface (designed, not registered)

```ts
// NOT callable until section 12's prerequisites pass. Documented here, not in FLIPNOTE-UI-HOOKS.
studio_overlay_save({ server, channel, object?, nonce, body })
type OverlaySave =
  | { v: 1; kind: "local-draft"; channel: string; object: string | null;
      basis: string; accepted: number; alreadySaved: boolean;
      transferState: "active"; shared: false; receipted: false; content: StudioContent }
  | { v: 1; kind: "handed-off"; channel: string; object: string | null;
      basis: string; accepted: number; epoch: string; epochId: string;
      shared: true; receipted: false };
```

`studio_overlay_read`'s existing result gains one value, `transferState: "completed"`, for a
record that holds only a completed acknowledgement; `kind` stays `"absent"` when no draft is
retained, so the accepted contract's meaning of `absent` does not change. `shared: true` means
the operations are in the local signed source and remain pending in the ordinary ledger; it is
never a delivery or settlement claim, and `receipted` is always `false` on this command.

Both commands use the existing `InvokeContext`: one UI session generation, one actor instance,
one native operation slot, one `ViewRequest` per `(state, server, target)` and one
`RequestCancellation` spanning **both** custody visits, as `studio_overlay_read` already does.

## 6. The pipeline

Costs marked "detached" never run under the lease. "Custody" stages hold the actor, the sole
`ServerStore`, the numeric-server ordering guard, the UI commit guard and the servers map.

### 6.1 Automatic handoff (Flow H), driven by the actor's background turn

| # | Stage | Custody | Work | Permit |
|---|---|---|---|---|
| H0 | Probe | yes | structural decode; cheap successor identity check | no |
| H1 | Capture | yes | reserve permit; two bounded authenticated reads; stamp | acquires |
| H2 | Plan | detached | full decode, reconstruction, private successor restore | held by worker |
| H3 | Authorize | yes | `studio_overlay_is_current`; `handoff_authority(device, group, tenure)` | held |
| H4 | Prepare | detached | `prepare_handoff_detached` (typed admission, preflight, framing bound) | held by worker |
| H5 | Sign slice (repeated) | yes | bounded `sign_next` turns with fresh live authority | held |
| H6 | Assemble | detached | `finish()`; encode the Prepared and Completed candidate records | held by worker |
| H7 | Commit | yes | inventory, rechecks, Prepared -> Source -> Completed | held |
| H8 | Notify | yes | settlement notice, `StudioUpdated`, watch rebinding | released |

H3 exists because `StudioHandoffAuthority` binds the branch's basis receipt, which is only
available after the decode, and `receipt.verify_current_owner(group, tenure)` requires the live
group. Capturing the authority at H1 would require decoding under custody, which is R1.

### 6.2 Local Save (Flow S), driven by the native two-visit command

| # | Stage | Custody | Work | Permit |
|---|---|---|---|---|
| S0 | Admit | yes | channel known, membership, request grammar, typed decode, PIX validation and pre-hold (existing Save rules) | no |
| S1 | Capture | yes | reserve permit; `studio_closing_basis`; bounded reads; stamp | acquires |
| S2 | Plan | detached | full decode; exact-retry and completed-retry checks; `append` (reconstruction) | held by worker |
| S3 | Commit | yes | re-mint the basis and require the same fingerprint; stamp equality; one accounted intent write | released after delivery |

S3 re-mints the basis from the actual current source under the same custody as the write, so the
accepted rule "recheck the same source and authority immediately before the first durable
acceptance" is preserved across the detach. A changed Closing source, a changed fingerprint, a
changed wrapper digest, a changed owner/tenure/MLS epoch or a replaced request discards the
planned state; no durable effect and no second envelope is produced.

### 6.3 Interrupted Prepared resolution (Flow R)

R0 probe (`is_prepared()`, structural), R1 capture, R2 detached restore + `evidence`, R3 commit
using the existing `resolve_studio_handoff_with_io` decision table, with the restored unit
reused only when `studio_overlay_is_current` proves the source bytes are unchanged. The
synchronous fence at the rotation, adoption, shared-write and publication entry points is
**unchanged**; the runtime only tries to get there first.

### 6.4 Custody-visit sources

A custody visit for the runtime is any `StudioReceiver::run` pass: the native receive driver
(`drive_receiver` -> `receive_once`, paced at one second while `studio_pending` is true) and any
explicit Studio document or control request. The runtime's `pending()` contribution keeps the
watch signal true while a job has work, so the driver keeps calling back at its existing cadence.
No change to `drive_receiver`'s pacing is proposed; section 13 records what that implies.

## 7. Scheduling, fairness and pacing

- **One overlay job per actor.** The runtime holds at most one `OverlayJob`, so it occupies
  exactly one of the four shared preparation slots at any time. There is no overlay-only pool.
- **Placement in the background turn.** `background_step` currently alternates replay and
  catch-up. The overlay runtime is added as a third participant with two different rules:
  - heavy stages (H1 capture, H2/H4/H6 detach, H7 commit) run only when
    `catchup.replay_ready()` holds, i.e. no page, checkpoint, discovery, registry pass,
    preparation or ready result is outstanding, mirroring `replay_step`'s gate;
  - signing slices (H5) need neither a new permit nor the retained source and may run on any
    background turn, but yield immediately if `server.sync.has_epoch_service_interest()`, any
    watch has inbound, or a background result is parked.
- **Bounded slice.** `MAX_SIGNING_TURNS_PER_VISIT = 32` and `SIGNING_SLICE_BUDGET_MS = 250`,
  whichever is reached first, measured on the runtime clock. Both constants are provisional and
  must be re-chosen from the measurement in section 13.
- **Coalescing.** One target at a time; targets are selected round-robin from the watch rail, and
  a target already holding a job is skipped. Repeated UI reads or peer traffic cannot start a
  second attempt or reset a backoff.
- **Pacing.** A hold or refusal sets `next_attempt_at = now + 30_000`, doubling to a 300,000 ms
  ceiling, reset to the floor on durable progress (a committed Save, a completed handoff or a
  resolved Prepared). `explicit_retry` may lower the floor to the base delay on an explicit
  successful access, as catch-up already does, but may not clear the backoff counter.
- **Other-server and authoritative progress.** The runtime never holds the lease across a detach
  and never awaits the network. `serve`, `advance_checkpoint`, `persist_registry_page` and
  `rotate_owner` keep their existing precedence in `CatchupRuntime::run`.

## 8. Authority, identity and currency

The matrix below is the checklist the review preamble asks for. "visit" means every custody
visit that the stage runs in, not once per job.

| Property | H0 | H1 | H3 | H5 (per visit) | H5 (per turn) | H7 | S1 | S3 |
|---|---|---|---|---|---|---|---|---|
| Channel is a known local channel | yes | yes | yes | yes | - | yes | yes | yes |
| Current membership and device signature key | yes | yes | yes | yes | yes (`check_live`) | yes | yes | yes |
| Numeric server id | yes | yes | yes | yes | - | yes | yes | yes |
| Mount pointer identity (`Arc::ptr_eq`) | yes | yes | yes | yes | - | yes | yes | yes |
| Registry sync instance (actor/sync incarnation) | - | yes | yes | yes | - | yes | yes | yes |
| Complete target (kind, channel, object) | yes | yes | yes | yes | yes | yes | yes | yes |
| Group/type/logical scope of the document | yes | yes | yes | yes | yes | yes | yes | yes |
| MLS epoch equals the captured epoch | - | yes | yes | yes | yes (`check_live`) | yes | yes | yes |
| Designated owner equals the captured owner | - | yes | yes | yes | - | yes | yes | yes |
| Independently observed owner tenure | yes (Some) | yes | yes | yes | yes (`check_live`) | yes | yes | yes |
| Receipt verifies against live owner/tenure | - | - | yes | - | yes (`check_live`) | yes | - | - |
| Intent wrapper full digest + physical size | - | capture | yes | yes | - | yes | capture | yes |
| Source wrapper full digest + physical size | - | capture | yes | yes | - | yes | capture | yes |
| Captured absence is still absence | - | capture | yes | yes | - | yes | capture | yes |
| Source-to-intent link requirement | - | - | yes | - | - | yes (`check_studio_handoff_write`) | - | yes |
| Closing basis re-minted and fingerprint equal | - | - | - | - | - | - | mint | yes |
| Native request/session generation and instance | - | - | - | - | - | - | yes | yes |
| Native final-delivery guard after conversion | - | - | - | - | - | - | - | yes |

Why the wrapper digests are rechecked per visit and not per signing turn: within a single custody
visit the runtime holds `OwnedMutexGuard<Option<ServerStore>>`, the sole store, so no other task
in this process can write those records. Between visits the guard is released, so the first turn
of every visit reauthenticates both wrappers before any `sign_next`. Live authority is rechecked
per turn because `sign_next` itself does it from the live `MlsDevice`/`ServerGroup`. If a
reviewer rejects this reasoning, the fallback is a per-turn wrapper recheck at the cost stated in
section 13; the code structure supports either by moving one call site.

Explicitly not treated as authority anywhere: a core snapshot hash, an inspection result, a
visible projection, an unchanged logical key, a basis fingerprint alone, a decoded Prepared flag,
a receipt hint, a seed marker, a Registry pointer, or a worker's claim that a write succeeded.

## 9. Durable commit

The commit visit is the accepted `handoff_studio_overlay_with_io` body with the candidate
supplied from H6 instead of built inline. In order:

1. `enter_studio_budget` against a five-family inventory minted in this visit.
2. `studio_overlay_is_current(context, stamp)` for both records and all live context (section 8).
   Any mismatch: drop the candidate, retain the branch, apply backoff, return a hold.
3. Complete-target comparison and `completed_branch` short-circuit, before source lookup,
   acknowledgement or sync reservation (HANDOFF-001, unchanged).
4. `check_handoff_references` with HANDOFF-002's complete traversal (unchanged).
5. Preflight all three replacement peaks and the intent accounting (unchanged).
6. Re-read the actual intent record and require the **authenticated plaintext digest** to equal
   the captured digest (change C-2, replacing the decode-then-re-encode comparison of R5).
7. Write Prepared (barrier 1), retaining the complete branch and ledger.
8. Mint `CheckedHandoffWrite` from the actual re-read bytes and call
   `save_studio_source_checked` with it (barrier 2). One atomic complete replacement.
9. `resolve_studio_handoff_with_io`: authenticate and flush the actual source, require every
   manifest entry as an exact current signed envelope with the same complete signed-operation
   digest, then replace Prepared with the compact completed acknowledgement (barrier 3).
10. Return the outcome. Publication becomes eligible only now.

Preserved without change: no durable signed prefix (the private candidate never touches disk
before barrier 2, and `finish()` is all-or-nothing), no per-entry retirement, the original full
pending ledger, the full signed digests, the retry floor and its rollover rule, the source
replacement fence, the publication fence, the source-required metadata link and the reference
inventory dependency. `save_studio_source_checked` and `check_studio_handoff_write` are not
modified except that their intent reads become structural (C-1) and digest-based (C-2).

**Change C-2 in detail.** `write_prepared_intents` and `persist_handoff_intents` currently obtain
the previous record through `read_epoch_intent_record`, which decodes. They gain a
bytes-and-size read (`read_scoped_intent_plain`, already used by the inspection capture) for the
`old` accounting value, and the "unchanged" fence becomes a digest comparison. The accounting
inputs (`old`, `next`, record id, generation) are identical.

## 10. Crash and interruption

Nothing new is durable, so the accepted reopen table
([GATE4-OVERLAY-HANDOFF-REVIEW](GATE4-OVERLAY-HANDOFF-REVIEW.md), "Write order and interrupted
handoff") governs restart unchanged. What the runtime adds:

| Interruption point | Result |
|---|---|
| Any detached stage (process exit, panic, cancellation) | The private candidate is lost; no durable byte changed; the branch and ledger are intact; the permit is released by the actual worker; the next probe starts a fresh attempt after backoff. |
| Between H5 slices | Same as above. Signed operations exist only inside `StudioHandoffSigning`. |
| Before barrier 1 | No handoff happened. Active, original source. |
| Between barriers 1 and 2 | Prepared with the exact recorded source-before and no branch ids in the destination: durably return to Active with the full draft (the accepted implementation's chosen route). |
| Between barriers 2 and 3 | Prepared with all exact envelopes and signed-operation digests: flush and complete without reapplying. |
| Any partial or conflicting evidence | Retain the full branch, report a hold, guess nothing. |
| S3 interrupted | The intent write is a single accounted atomic replacement with the existing exact-retry flush; an exact retry of the same request re-establishes durability without a second envelope. |

The publication fence must hold across all of the above, including after restart, before any
generic page, current-tail, seed or ordinary retry send can expose a batch between barriers 2
and 3. That path is unchanged and is covered by the retained existing regression and mutation.

## 11. Publication, invalidation and events

- The handoff emits **no packets**. It returns `StudioSavedTransaction::empty()`, so
  `publish_studio_save` sees nothing and the at-most-two-packet initial Save window is untouched.
  Transferred operations reach peers through the existing authenticated current-tail and page
  service once Completed releases the publication hold.
- After a durable local Save and after Completed, the runtime emits a settlement notice and,
  for Completed, an `AppEvent::StudioUpdated` so the local view, channel list and Registry
  pointer maintenance refresh. A local Save emits `RefreshRequired` plus the new
  `LocalDraftRetained`, never `StudioUpdated`, because no shared source changed.
- Two new `StudioSettlementState` variants are proposed:
  `LocalDraftRetained` (a local-only branch exists for this target) and
  `LocalDraftHandedOff` (the branch is in the local signed source and pending in the ordinary
  ledger). Neither is a delivery or settlement claim. Agent 2 owns the stale/manual and
  storage-refusal variants; the enum is a shared contract owned by Agent 4.
- **Generation-aware invalidation** (requirement 7). `ReplayRuntime::lifecycle` keys its context
  on `(mount, server, mls)` and caches `completed` bindings; `CatchupRuntime` caches
  `binding`/`lifecycle` similarly. Saving a draft on an already watched epoch changes the intent
  record but not the source, so an already-watched binding would not notice new pending work.
  The replay context key gains the store's `intent_generation` `Arc<()>`, and any runtime intent
  write clears the affected target from `replay.completed`. `studio_storage_budget` already
  rejects a stale `intent_generation`, so the generation is an existing, authoritative signal.
- **Replay exclusion** (R4). `studio_replay_evidence` filters `own` with
  `!state.is_overlay(id)` using the structural state, so accepted overlay operations are never
  chosen by the ordinary replay worker, never converted into an ordinary Apply and never moved to
  recovery as manual items. Mutation M10 isolates this.

## 12. Interface with Agent 2, and native Save exposure

### What Agent 1 provides to Agent 2

| Seam | Contract |
|---|---|
| `capture_studio_overlay` / `studio_overlay_is_current` / `StudioOverlayStamp` | The single capture and currency mechanism for all overlay work. Export, copy and disposition must use it rather than duplicating capture or delivery machinery. |
| `StudioOverlayCapture::plan` and `StudioOverlayPlanned` | Full decode and reconstruction happen here, on a worker. Agent 2's export uses the same detached stage rather than decoding under custody. |
| `OverlayRuntime` job model and the `StudioBackgroundJob::Overlay*` variants | One shared `preparation_pool()` permit per job, owned from capture to release; one overlay job per actor; no second pool; cancellation never refunds a live owner's slot. |
| `ServerStore::commit_studio_overlay_state(..., stamp, next_state, ...)` | One accounted intent write behind stamp equality, used by Save, by disposition and by copy bookkeeping. It never retires an entry or prunes a source by itself. |
| `ServerStore::studio_closing_basis` | Basis derivation from the actual Closing source, its receipt head and the saved signed close. No caller-supplied authority. |
| `studio_overlay_runtime_hold(target) -> Option<StudioOverlayHold>` | True while an overlay job for that target is between H1 and H7, and while the record is Prepared. Agent 2's disposition, copy and export must refuse while held, must not cancel the job, and must return a retryable refusal. |
| Structural/full decode split (C-1) and the call-site table | Agent 2 must use the full decoder for anything that returns a projection, and only detached. |
| Native two-visit pattern | `InvokeContext` reuse rules, the retained-permit result and the final-delivery guard, as already demonstrated by `studio_overlay_read`. |

### What Agent 1 requires from Agent 2 before native Save is registered

P1. A reviewed manual lifecycle: bounded inspect, export, copy-into-current and explicit
disposition for branches that cannot auto-handoff, lossless across restart and refusal.
P2. Every `StudioOverlayHold` variant in section 5.4 mapped to a user-visible, actionable state.
The runtime's holds are deliberately conservative and will occur in normal use; a hold with no
manual path is an unreachable branch for the user.
P3. Truthful native results and events plus UI-hooks rows for local-only, awaiting receipt,
stale/manual action, recovery and storage refusal.
P4. A live-tenure contract for `observed_owner_tenure_start()`: `None` is fail-closed (the
runtime already refuses capture), and a returning owner in a new tenure must observe a value
that differs from its earlier tenure. The runtime binds `tenure` as an opaque `u64` and relies on
"equal value implies the same continuous tenure". If that cannot be guaranteed, the runtime needs
a different binding and section 8 changes.
P5. An explicit statement in `GATE4-AGENT-2-STATUS.md` that P1 to P4 are implemented and have
passed their adversarial review, so Agent 4 can land the registration commit.

Until then: no `#[tauri::command]`, no entry in `lib.rs`'s handler list, no security or
capability row, and no `FLIPNOTE-UI-HOOKS` "Available now" entry for Save. The runtime is
exercised through real spawned actors and the store, which is the evidence level this design
claims.

## 13. Limits, costs and measurements that must be produced

Nothing in this section is measured yet. The numbers quoted are the existing debug-profile
observations in [P1-PERFORMANCE](P1-PERFORMANCE.md), which are one observation per case on a
small-seed title-edit fixture at the operation ceiling. They are not release latency, worst-case
bounds, heap qualification or actor-fairness proof.

Required measurements before the implementation review, each for Index and Flipnote:

1. Custody time per stage of Flow H at 1, 32 and 256 operations: H0, H1, H3, one H5 slice, H7,
   separating the inventory acquisition from the rest of H7.
2. The same at **maximal accepted shapes**, not only the existing 256 small title edits:
   the 5 MiB + 1024-byte complete plaintext intent record filled by 256 maximal-body operations,
   a 2 MiB seed, the 64 KiB combined metadata ceiling, the maximal accepted projection widths
   already used by the inspection tests (64 Index objects with retained alternatives; 999
   Flipnote frames with all 1024 conflict fields), and a large roster.
3. Retained input and output cost within one permit: captured intent plaintext plus captured
   source plaintext plus the decoded state plus the restored private successor plus the signed
   candidate plus the encoded Prepared and Completed records coexist. Report the sum of the
   accounted bounds. **This is not a measured process heap ceiling and must not be reported as
   one.**
4. Flow S custody time per stage at 1, 32 and 255 already-accepted operations, including the
   `studio_closing_basis` re-mint at S3 against a maximal Closing source and seed (R3).
5. The effect of C-1: ordinary `studio_read` custody time on a document with a 256-operation
   retained branch, before and after.
6. Wall-clock time to complete a 256-operation handoff end to end at the chosen slice bounds,
   with the count of custody visits used, so the slice constants can be re-chosen.

Known limits to state in the review request:

- **L1.** Total handoff latency is a function of the slice bound and the native receive driver's
  one-second cadence, not of a single blocking call. At the provisional bounds and the existing
  debug per-turn observation, a 256-operation handoff needs tens of custody visits and tens of
  seconds of wall clock. The branch is retained throughout and nothing is published until
  Completed. Actor responsiveness, not handoff latency, is the acceptance criterion.
- **L2.** The core requires one MLS epoch across every signature (`check_live` compares
  `group.epoch()`). An MLS commit during signing invalidates the candidate and the job restarts
  from H1 after backoff. In a group that commits frequently, the largest branches may retry
  repeatedly. No accepted work is lost, and the retry is paced. A design that survives an MLS
  transition mid-batch would be a core change and is not proposed here.
- **L3.** The synchronous Prepared fence on the rotation, adoption, shared-write and publication
  paths still restores the destination source and computes `evidence` under custody (R6). The
  runtime resolves Prepared proactively so those paths normally find nothing, but the worst case
  remains. Measure it at 256 operations.
- **L4.** C-1 moves typed-replayability validation of a retained branch from every decode to the
  detached reconstruction. Section 5.1 states exactly what that gives up and why the record
  cannot be attacker-supplied.
- **L5.** The Intents family is still uncached in the five-family scan. With C-1 each scan entry
  becomes a structural decode instead of a reconstruction, which is the fix that matters; an
  `inventory_cache` extension to the Intents family is a separate optimisation (O-1) and is not
  proposed in this design, because HANDOFF-002's reference-scan path deliberately bypasses that
  cache and the interaction needs its own review.

Optional optimisations, explicitly **not** part of this proposal and each requiring its own
review if pursued: O-1 (Intents inventory cache), O-2 (`checked_studio_source` reusing the warm
retained source under its existing full-wrapper authentication), O-3 (precomputing the Prepared
and Completed encodings on the H6 worker).

## 14. Contingency if the core signing review changes the split

If the review of `e65bfd8` requires a different authority capture, a different number of
operations per call, or a different assembly boundary:

- H3 and H4 are the only stages bound to `handoff_authority` and `prepare_handoff_detached`;
  a changed capture moves work between them without changing sections 8, 9 or 10.
- H5's slice bound is independent of how many operations a call signs; if the unit changes,
  only `MAX_SIGNING_TURNS_PER_VISIT` and the measurement in section 13.1 change.
- If the split is rejected outright and the core returns to a single batch call, Flow H's H2 to
  H6 collapse into one detached stage and the design still holds, with L1 improving and the
  mid-batch authority recheck weakening; that would need its own review.
- Sections 5.1, 5.3, 5.4, 6.2, 7, 11 and 12 do not depend on the core split at all.

## 15. Test and mutation plan

Levels are named as the common instructions require. "Store" means real sealed records and real
accounted writers. "Actor" means a real spawned actor with authenticated transport, real
membership and the production native conversion. No fixture-injected ready value, synthetic
preview or directly inserted private state substantiates a claim below.

### 15.1 Normal regressions

| # | Level | Case | Independent observation |
|---|---|---|---|
| N1 | actor | Index and Flipnote local Save on an eligible Closing document, reopen, read | Draft projection, accepted count, basis, original envelope, nonce, author and timestamp survive restart; the canonical source, gate, recovery, owner and Registry bytes are unchanged; `studio_overlay_read` returns the same draft. |
| N2 | actor | Exact local Save retry after the source rotated away from the Closing basis | Returns `alreadySaved`, creates no second envelope, does not demand a fresh Closing basis, leaves the record byte-identical apart from the accounted flush. |
| N3 | store | Ordinary failed Apply leaves a pending intent, then the same nonce/body is offered as a local Save | Refused with the ordinary-collision hold; the entry stays unannotated and `NoEvidence`. |
| N4 | actor | Eligible successor installed by a real receipt; runtime performs the automatic handoff | One durable signed source replacement containing all 256 operations in original order with original timestamps; the full ledger is still pending; the local base is released only after barrier 3; a peer then catches up through ordinary paging. |
| N5 | actor | Peer catch-up after N4 | The peer's projection equals the local projection, including conflicts and deletions; the initial-Save publication window never carried the batch. |
| N6 | store | Interrupt each of the three durable writes and each sync, then reopen | The accepted reopen table is reproduced; no false success, no lost branch, no duplicate effect. |
| N7 | store | Change the intent wrapper after capture, same physical size and same displayed draft | Commit refuses at the digest comparison; durable bytes unchanged; a fresh attempt succeeds. |
| N8 | store | Change the source wrapper after capture, same physical size | Same, at the source digest comparison. |
| N9 | actor | Change authority between stages: device key, membership removal, MLS commit, owner change, tenure change, channel removal, remount, numeric server change, actor replacement | Each produces a hold at its own check with the branch retained; each fixture passes all earlier checks first. |
| N10 | actor | Change the native session generation, the view request and the actor instance between the two Save visits, and after conversion | The second visit and the final delivery guard reject; no partial value is returned; the draft that was already durable stays durable. |
| N11 | actor | Pause a real H2 or H4 worker; meanwhile another document's Save, an authoritative checkpoint operation and another server's progress complete | All three complete while the paused job retains its slot; the paused job then finishes normally. |
| N12 | actor | Pause an H5 signing sequence between slices; drive an inbound page and a Registry pass | Authoritative work proceeds; the signing job resumes and completes; no packet was published. |
| N13 | actor | Four real jobs and results fill the shared pool; a fifth capture is attempted | Retryable capacity refusal, no overlay-only pool, and the slot is released exactly once on completion. |
| N14 | store | Cancel a paused overlay worker | The slot is not refunded while the blocking closure runs; the job is dropped and the branch retained. |
| N15 | store | Prepared record found at startup; run the resolution flow for each evidence outcome | Absent returns durably to Active with the full draft; Complete flushes and completes without reapplying; Hold retains everything. |
| N16 | store | Generic page, current-tail, seed service and ordinary retry sending while Prepared | All refuse; after barrier 3 they serve; the accepted publication-fence regression is retained. |
| N17 | store | PIX references through source, intent and recovery changes across the whole flow | Base-only, pending, superseded and removed-frame CIDs remain enumerable and protected; HANDOFF-002's dependency check still refuses a scan with missing metadata. |
| N18 | store | Maximal accepted shapes from section 13.2 | Accepted at the ceiling; one byte over each ceiling refuses with no partial output and unchanged retained data. |
| N19 | store | C-1 equivalence: structural and full decode of the same record | Identical `StudioOverlayState` fields, identical `encode_vault` bytes, identical `handoff_metadata` answers; the structural decoder rejects a missing entry, a duplicate id, a wrong sequence, a wrong author, a changed envelope and trailing data. |
| N20 | store | C-1 regression guard: a record whose branch is not typed-replayable | Custody-path readers succeed structurally, the detached reconstruction refuses, and the branch is reported as a hold rather than making ordinary reads fail. |
| N21 | actor | Overlay ids are excluded from ordinary replay | With an Active branch, a replay pass chooses nothing for those ids, moves none of them to recovery and applies none of them. |
| N22 | actor | Save a draft on an already watched epoch | The watch, replay context and completed bindings notice the new pending work through the intent generation; a second Save is not silently skipped. |
| N23 | store | Earlier Gate 4 regressions on the same documents | Solo repeated rotations and restarts, Registry pointer and tail paging, Create after Index rotation, replay and manual recovery, and persisted eviction deadlines are unchanged. |

### 15.2 Isolated mutations

Each entry names the unique guard removed, the single test that must execute and fail at its
intended assertion, and the restored regression that must then pass. Compilation failure, a zero
match filter, an unrelated panic or a broad `is_err()` does not count. Anchors must be unique.

| # | Guard removed | Test that must fail | Intended assertion |
|---|---|---|---|
| M1 | Intent wrapper digest comparison in `studio_overlay_is_current` (keep the size comparison) | N7 | "commit used a stale plan" |
| M2 | Source wrapper digest comparison in `studio_overlay_is_current` | N8 | "commit replaced a changed source" |
| M3 | Permit ownership: refund the permit when the waiter is cancelled | N14 | "cancelled worker refunded a live slot" |
| M4 | Per-visit reauthentication before the first `sign_next` of a slice | N7 variant that changes bytes between slices | "signing continued on changed records" |
| M5 | The slice bound (`MAX_SIGNING_TURNS_PER_VISIT` and the time budget) | N11/N12 | "authoritative checkpoint did not complete during signing" |
| M6 | The H0/H3 pristine-successor requirement | N4 negative variant with an extra operation in the successor | "handoff entered a non-pristine successor" |
| M7 | Ordering of barrier 1 and barrier 2 in the commit | N6 | "source was replaced before Prepared was durable" |
| M8 | `checked_entries` inside `decode_vault_structural` | N19 | "structural decode accepted an inconsistent branch" |
| M9 | The overlay-id exclusion in `studio_replay_evidence` | N21 | "ordinary replay applied an accepted overlay operation" |
| M10 | The exact-retry precedence over fresh-basis eligibility in Flow S | N2 | "saved retry demanded an absent Closing basis" |
| M11 | The ordinary-intent collision guard in the plan stage | N3 | "ordinary failed Apply became accepted local work" |
| M12 | The S3 basis re-mint and fingerprint equality | N9 (changed Closing source between visits) | "Save used a stale Closing basis" |
| M13 | The final native delivery recheck for Save | N10 | "expired delivery returned a converted value" |
| M14 | The all-or-nothing requirement in the assemble stage (persist after k signatures) | N6 | "a durable signed prefix escaped" |

The existing `studio-handoff`, `studio-overlay`, `studio-inspection` and `studio-native`
mutations are retained unchanged; M1 to M14 are additions.

### 15.3 Harness and workflow

- New script `.github/scripts/check-studio-overlay-runtime-mutations.py`, following
  `check-studio-handoff-mutations.py` exactly: unique anchors, one executed failing test per
  mutation, byte-exact restoration, passing restored regression, optional name selection, and
  per-mutation logs under `logs/gate4-overlay-runtime-*.log`.
- Requested workflow patch for Agent 4: a `runtime` job in `.github/workflows/studio-handoff.yml`
  running the focused store and actor suites and then the new mutation script, publishing
  `logs/gate4-overlay-runtime-*.log` as an artifact. The integration scenarios must be added to a
  required workflow so they cannot stay opt-in.
- Local execution stays serial on this machine: `-j 1`, the existing per-package test debug
  override, no concurrent Cargo work, and no blanket cleanup.

## 16. Dependencies and integration changes for Agent 4

| File | Change | Note |
|---|---|---|
| `crates/catcoms-app/src/studio/dispatch.rs`, `control.rs` | New `StudioControlAction` and `StudioControlResponse` variants (5.6) | Central enum edit |
| `crates/catcoms-app/src/studio/receiver/catchup.rs` | New `StudioBackgroundJob` / `StudioBackgroundResult` variants (5.5) | Central enum edit |
| `crates/catcoms-app/src/studio/receiver.rs` | `detach`, `complete`, `pending`, `background_step` gain overlay arms | Shared with Agents 2 and 3 |
| `crates/catcoms-app/src/studio/settlement.rs` | Two new `StudioSettlementState` variants (11) | Shared with Agent 2 |
| `crates/catcoms-app/src/studio/replay.rs`, `receiver/replay.rs` | Overlay-id exclusion (R4) and the intent-generation context key (11) | Shared with Agent 2 |
| `crates/catcoms-app/src/store/epoch_recovery/inventory.rs` | Structural intent decode in the scan; the reference path keeps every existing check | HANDOFF-002 adjacent; must be reviewed explicitly |
| `crates/catcoms-app/src/store/epoch_intents.rs` | Structural read entry points; digest-based unchanged fence (C-2) | Shared with Agent 3 |
| `crates/catcoms-app/src/store/epoch_studio.rs`, `epoch_studio/handoff.rs` | Commit accepts a detached candidate; reads become structural/digest-based | Shared with Agent 3's source writer coordination |
| `crates/catcoms-replication/src/studio/overlay.rs` and `overlay/handoff.rs` | `decode_vault_structural` (C-1) | Core change; needs its own verdict |
| `apps/desktop/src-tauri/src/lib.rs`, `studio.rs` | Native command, registration, security and capability rows | **Deferred** to a separate commit gated on section 12 |
| `docs/INTERFACES.md`, `docs/BACKEND-IMPLEMENTATION.md`, `docs/FLIPNOTE-UI-HOOKS.md`, `docs/GATE4-ACCEPTANCE.md` | Rows for the new seams; UI hooks records the Save command as unavailable until its prerequisites pass | Agent 4 owns these files |
| `.github/workflows/studio-handoff.yml`, `.github/scripts/` | New runtime job and mutation script (15.3) | Agent 4 owns workflows |

Agent 3 coordination: the runtime is the only new source writer and the only new preparation
consumer for overlays. Signed fault repair must resolve an interrupted Prepared overlay on every
common source path through the existing fence rather than introducing a competing writer or pool,
and must respect `studio_overlay_runtime_hold`.

## 17. Open questions for the reviewer

1. Is the per-visit (rather than per-turn) wrapper reauthentication in section 8 acceptable given
   that the store guard is exclusive within a visit, or must every `sign_next` be preceded by a
   full re-read and hash of both records?
2. Is C-1 the right shape for R1, or should the reconstruction stay in the decoder and the fix be
   confined to making the inventory scan and the link check use a different path?
3. Is an extra custody visit (H3) to mint the authority from the decoded receipt preferable to a
   new core constructor that derives the authority from the successor source's opening receipt
   under live custody? The latter removes a visit but adds an authority boundary.
4. Are the provisional slice bounds the right control, or should the runtime instead bound work
   by a measured byte or operation cost derived from the maximal-shape measurement?
5. Does L2's "restart the whole handoff on an MLS commit" need a mitigation before Gate 4, or is
   paced retry with a fully retained branch sufficient?

## 18. Adversarial design review request

Fill `[FULL_HEAD_SHA]` with the commit that adds this document before sending; the review
preamble forbids sending placeholders.

```text
Review type: design.
Base: a052f78b62a549702686a8741932f1d2f8c98773. Head: [FULL_HEAD_SHA].
Compare: https://github.com/Thalpy/Mewtual/compare/a052f78b62a549702686a8741932f1d2f8c98773...[FULL_HEAD_SHA]
Scope/evidence: docs/GATE4-AGENT-1-DESIGN.md and docs/GATE4-AGENT-1-STATUS.md at the head.
Design only: no production code, no test and no new measurement exists for this checkpoint.
Dependencies: the core signing split e65bfd8 is UNREVIEWED (PR #26 has no submitted review and
HANDOVER records no returned CI run for it); manual lifecycle: not started, native Save stays
unregistered and undocumented in FLIPNOTE-UI-HOOKS.

You are the independent adversarial reviewer for the Gate 4 scope below. Read
docs/GATE4-AGENT-HANDOFFS.md, docs/GATE4-AGENT-1-STATUS.md, docs/BACKEND-IMPLEMENTATION.md,
docs/design-creative-suite.md, docs/design-epoch-close.md, docs/INTERFACES.md and
docs/FLIPNOTE-UI-HOOKS.md, plus GATE4-OVERLAY-RUNTIME-REVIEW, GATE4-HANDOFF-SIGNING-REVIEW,
GATE4-OVERLAY-HANDOFF-REVIEW, GATE4-OVERLAY-HANDOFF-IMPLEMENTATION-REVIEW,
GATE4-CLOSING-OVERLAY-REVIEW and GATE4-INSPECTION-IMPLEMENTATION-REVIEW. Inspect the actual code
the proposal cites; documentation and implementer claims are evidence pointers, not proof.
State the resolved baseline/head, the actual reviewed scope, accepted dependencies, and whether
you executed anything or only inspected source.

Challenge the complete capture -> detached plan -> authorize -> detached preparation -> finite
signing slices -> detached assembly -> durable commit -> native delivery path, not only the
sign_next call. Sections 3 and 5.1 claim that decoding a retained branch performs a full typed
reconstruction (StudioOverlay::decode_vault ends with out.read(ledger)), that this runs on
ordinary reads, on every intent read, on the five-family inventory scan (where the Intents family
is uncached) and about four times in the accepted synchronous commit, and that StudioOverlay
::append replays the whole branch under custody. Verify those call paths in the code. If they
hold, judge whether the proposed structural/full decoder split is the right correction, whether
every moved call site is genuinely projection-free, and whether section 5.1 honestly states what
validation is deferred and why the record cannot be attacker-supplied.

Verify that the original four-slot shared reservation is owned through capture, queued work,
worker, ready result, every signing slice, commit and delivery, and that cancellation never frees
a live owner's slot; that no overlay-only pool, uncharged cache or worker-owned device/MLS secret
is introduced. Check section 8's matrix against the necessary visits: numeric server,
group/type/logical/channel, device/membership/key, MLS epoch, observed tenure, actor/sync
incarnation, mount, full source and intent wrapper digest with physical size, and the original
native request/session. Challenge specifically the per-visit rather than per-turn wrapper
recheck, and the extra H3 visit that mints the authority from the decoded receipt.

Try same-size different authenticated wrappers, the same display with extra history, a stale
session after successful conversion, a mid-signing membership or MLS change, and an interrupted
Prepared record. Confirm first local acceptance still derives from the actual Closing source, its
matching saved signed close from the owner journal and observed tenure, that the renderer
supplies no close, seed, receipt, author or tenure, that an exact saved retry does not require a
now-absent Closing basis or create a second envelope, and that an ordinary failed Apply cannot
become accepted local work across the new detach.

Require whole Prepared -> Source -> Completed with full signed evidence, the retained pending
ledger, the retry floor and rollover, required-metadata links, HANDOFF-002's complete reference
inventory, the common source-write fence and the publication hold. Judge whether replacing the
decode-then-re-encode unchanged fence with an authenticated plaintext digest comparison is
strictly stronger. Confirm no signed prefix can escape and that Completed publication uses
ordinary paging with the two-packet initial Save limit untouched.

Assess the scheduling and fairness claims in section 7 and the limits in section 13: one overlay
job per actor, heavy stages gated on catch-up idleness, bounded signing slices, coalescing and
backoff, and the honest statement that total handoff latency depends on the slice bound and the
one-second native receive cadence. Challenge L2 (an MLS commit during signing restarts the whole
handoff) and say whether paced retry with a fully retained branch is sufficient for Gate 4.

Answer the five questions in section 17 explicitly. Return PASS for this bounded design, or
numbered findings with severity, file/line, trigger, impact, evidence and required correction.
Separate design findings from the audit observations in section 3. A PASS accepts the design
only: no implementation, no measurement and no native Save exposure is claimed, Agent 2's manual
lifecycle remains a prerequisite for registration, and full Gate 4 acceptance stays with Agent 4.
```
