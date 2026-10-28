//! Explicit local Studio transactions. The actor supplies live membership while its caller lends
//! the sole mounted vault and lifecycle guards. Successful actor saves make a bounded one-shot
//! publication attempt; saved state is never reported as delivered or settled.

use crate::{AppError, Server, ServerStore};
use catcoms_replication::studio::{
    FlipnoteHeader, FlipnoteOp, IndexOp, ScoreHeader, ScoreOp, StudioExpiry, StudioKind,
    StudioProjection, StudioTarget,
};
use catcoms_replication::{epoch_zero_id, DomainOp};
pub use catcoms_replication::{studio as types, EpochPhase, RecoveryReason, RecoveryTransition};
use catcoms_rt::{CryptoRngCore, MeshTransport};
use tokio::sync::{oneshot, OwnedMutexGuard};

mod tenure;
pub use tenure::StudioOwnerTenure;
mod copy;
pub use copy::{
    StudioCopyPreparation, StudioOverlayCopyApply, StudioOverlayCopyPreview,
    StudioOverlayCopyRefusal, StudioPreparedCopy,
};
mod inspection;
mod overlay;
pub use inspection::{
    StudioDelivered, StudioInspectionDelivery, StudioInspectionPreparation,
    StudioOverlayInspection, StudioPreparedInspection,
};
pub use overlay::StudioOverlaySaveTicket;
mod preview;
pub use preview::{StudioPreview, StudioPreviewDelivery, StudioRead};
mod publication;
pub(crate) use publication::StudioSavedTransaction;
mod control;
mod fault;
pub use catcoms_replication::{RepairDisposition, RepairHold};
pub use fault::{
    StudioFaultCandidate, StudioFaultScope, StudioFaultView, StudioRepairBlocker,
    StudioRepairReport, StudioRepairStart, StudioRepairStatus,
};
mod replay;
/// Crate-visible because copy's detached planner lives in the store and calls `plan` directly.
pub(crate) mod restore;
mod settlement;
mod sync_state;
pub use control::{
    StudioArchiveReleaseRequest, StudioControlAction, StudioControlReady, StudioControlRequest,
    StudioControlResponse, StudioHandoffAttempt, StudioHandoffAttemptReason,
    StudioHandoffAttemptState, StudioOverlayArchived, StudioOverlayExport, StudioOverlayLifecycle,
    StudioRecoveryApply, StudioRecoveryListing, StudioRecoveryPreview, StudioRecoverySummary,
    StudioRecoveryVersion, StudioReleaseConfirmation, StudioSettlementSource,
    StudioUnconfirmedOverlaySaveRequest, StudioUnconfirmedSaveOutcome, CONTROL_REPLY_DROPPED,
};
pub use restore::{
    StudioRecoveryDisposition, StudioRecoveryItem, StudioRecoveryMode, StudioRecoveryPlan,
    StudioScoreHeaderField,
};
use settlement::SettlementNotices;
pub use settlement::StudioSettlementState;
pub use sync_state::{StudioSyncObservation, StudioSyncPauseCause};

/// A linked id/Index hint is not evidence of an existing Score in this group and channel. The
/// header probe classifies absent/wrong-family sources as missing; the full typed load then
/// authenticates the actual source. Called under custody at recovery/Copy preview and Apply,
/// and immediately before ordinary Save's intent barrier. No network request is made here.
fn sound_link_exists(
    store: &mut ServerStore,
    server: u64,
    group: &catcoms_mls::ServerGroup,
    device: &catcoms_mls::MlsDevice,
    target: StudioTarget,
    body: &[u8],
) -> Result<bool, AppError> {
    if !matches!(target, StudioTarget::Flipnote { .. }) {
        return Ok(true);
    }
    let FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some(object))) =
        FlipnoteOp::decode(body).map_err(invalid)?
    else {
        return Ok(true);
    };
    let score = StudioTarget::Score {
        channel: target.channel(),
        object,
    };
    if !store.studio_object_holds_work(server, group, score)? {
        return Ok(false);
    }
    Ok(store
        .with_studio_source(server, group, score, device, |state| {
            Ok((state.op_count() > 0 || state.epoch() > 0)
                && matches!(state.projection()?, StudioProjection::Score(_)))
        })?
        .unwrap_or(false))
}
mod dispatch;
pub(crate) use dispatch::{StudioDispatch, StudioReply, StudioResponse};
mod receiver;
pub(crate) use receiver::StudioBackgroundResult;
pub(crate) use receiver::StudioReceiver;
#[cfg(test)]
pub(crate) use receiver::{PreviewHarness, StudioOverlaySaveVisit};

/// Bounded before entering the actor queue. Bodies are the existing canonical Studio JSON,
/// not renderer-authored Automerge changes. Keep ids/nonces/bodies stable across retries.
pub enum StudioRequest {
    Read {
        target: StudioTarget,
    },
    Apply {
        target: StudioTarget,
        epoch_id: u128,
        nonce: [u8; 16],
        body: Vec<u8>,
    },
    /// Several Score operations in ONE transaction: one durable server snapshot, one storage
    /// scan, one publication window and one update event instead of one of each per edit, which
    /// is what made the composer save "step by step". The operations are applied in order, each
    /// as its own signed operation with its own nonce, so an exact retry of the whole batch is as
    /// safe as retrying each edit (accepted items reseal, they are never applied twice). The
    /// batch is NOT atomic: an item refused after earlier items were written leaves those items
    /// durable, and the error names the item so the caller can retry exactly or drop it. Score
    /// only: flipnote and Index bodies need per-request admission (blob custody, link evidence).
    ApplyBatch {
        target: StudioTarget,
        epoch_id: u128,
        operations: Vec<StudioBatchOperation>,
    },
    /// Object header first, index second. This is retryable, not a cross-document transaction:
    /// failure can leave an unlisted object. The caller must retry these EXACT fields.
    Create {
        channel: [u8; 16],
        object: [u8; 16],
        nonce: [u8; 16],
        title: String,
        ts: u64,
    },
    /// The same durable object-then-index workflow with an explicit score family. Keeping the
    /// original Create variant preserves existing typed Rust callers as well as native defaults.
    CreateScore {
        channel: [u8; 16],
        object: [u8; 16],
        nonce: [u8; 16],
        title: String,
        ts: u64,
    },
}
impl std::fmt::Debug for StudioRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioRequest { .. }")
    }
}
/// One member of [`StudioRequest::ApplyBatch`]: the same nonce/body pair a single Apply carries.
pub struct StudioBatchOperation {
    pub nonce: [u8; 16],
    pub body: Vec<u8>,
}
impl std::fmt::Debug for StudioBatchOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioBatchOperation { .. }")
    }
}
/// Bounds of one batched Apply, checked before the actor queue. The renderer mirrors both in
/// `studio-native.ts`; an autosave that gathers more edits sends several batches.
pub const STUDIO_BATCH_MAX_OPERATIONS: usize = 64;
pub const STUDIO_BATCH_MAX_BYTES: usize = 256 * 1024;
impl StudioRequest {
    pub fn target(&self) -> StudioTarget {
        match self {
            Self::Read { target }
            | Self::Apply { target, .. }
            | Self::ApplyBatch { target, .. } => *target,
            Self::Create {
                channel, object, ..
            } => StudioTarget::Flipnote {
                channel: *channel,
                object: *object,
            },
            Self::CreateScore {
                channel, object, ..
            } => StudioTarget::Score {
                channel: *channel,
                object: *object,
            },
        }
    }
    /// Static bounds/grammar only; identity, causal state and persistence are checked in actor.
    pub fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::Read { .. } => Ok(()),
            Self::Apply {
                target,
                nonce,
                body,
                ..
            } => {
                match target {
                    StudioTarget::Index { .. } => {
                        IndexOp::decode(body).map_err(invalid)?;
                    }
                    StudioTarget::Flipnote { .. } => {
                        // All sound families now have causal admission, bounded state, native
                        // evidence and durability. Static parsing never grants write authority;
                        // the ordinary current-member/epoch gate still runs below.
                        FlipnoteOp::decode(body).map_err(invalid)?;
                    }
                    StudioTarget::Score { .. } => {
                        ScoreOp::decode(body).map_err(invalid)?;
                    }
                }
                domain(*target, *nonce, body.clone())
                    .encode()
                    .map(|_| ())
                    .map_err(invalid)
            }
            Self::ApplyBatch {
                target, operations, ..
            } => {
                if !matches!(target, StudioTarget::Score { .. }) {
                    return Err(invalid("batched Apply is limited to Score targets"));
                }
                if operations.is_empty() || operations.len() > STUDIO_BATCH_MAX_OPERATIONS {
                    return Err(invalid(format!(
                        "batched Apply holds 1 to {STUDIO_BATCH_MAX_OPERATIONS} operations"
                    )));
                }
                let mut nonces = std::collections::BTreeSet::new();
                let mut total = 0usize;
                for operation in operations {
                    // Two items with one nonce would be one operation id: the second could only
                    // ever reseal the first, silently dropping an edit the author made.
                    if !nonces.insert(operation.nonce) {
                        return Err(invalid("batched Apply repeats a nonce"));
                    }
                    total = total.saturating_add(operation.body.len());
                    if total > STUDIO_BATCH_MAX_BYTES {
                        return Err(invalid("batched Apply exceeds its byte bound"));
                    }
                    ScoreOp::decode(&operation.body).map_err(invalid)?;
                    domain(*target, operation.nonce, operation.body.clone())
                        .encode()
                        .map(|_| ())
                        .map_err(invalid)?;
                }
                Ok(())
            }
            Self::Create { title, ts, .. } | Self::CreateScore { title, ts, .. } => {
                if title.len() > 64 * 1024 {
                    return Err(invalid("Studio title exceeds the operation bound"));
                }
                if *ts > 9_007_199_254_740_991 {
                    return Err(invalid("invalid creation timestamp"));
                }
                let header = initial_header(self.target(), title)?;
                domain(self.target(), [0; 16], header)
                    .encode()
                    .map_err(invalid)?;
                // Both complete envelopes must fit BEFORE the object write. The actual creator
                // is supplied later from the live actor; every identity has the same hex width.
                let index = IndexOp::PutObject {
                    object: [0; 16],
                    kind: creation_kind(self.target())?,
                    title: title.clone(),
                    created_by: crate::DeviceId::from_bytes([0; 32]),
                    ts: *ts,
                    expiry: StudioExpiry::Unrecorded,
                }
                .encode()
                .map_err(invalid)?;
                domain(
                    StudioTarget::Index {
                        channel: self.target().channel(),
                    },
                    [0; 16],
                    index,
                )
                .encode()
                .map(|_| ())
                .map_err(invalid)
            }
        }
    }
    pub(crate) fn changes_state(&self) -> bool {
        !matches!(self, Self::Read { .. })
    }
}
fn creation_kind(target: StudioTarget) -> Result<StudioKind, AppError> {
    match target {
        StudioTarget::Flipnote { .. } => Ok(StudioKind::Flipnote),
        StudioTarget::Score { .. } => Ok(StudioKind::Score),
        StudioTarget::Index { .. } => Err(invalid("an index is not an object creation target")),
    }
}
/// Select the EXPECTED object family from a validated Index/restore operation. This is routing,
/// not evidence that the object exists: every caller must independently check its authenticated
/// sealed wrapper and typed source under this exact family/channel. Never fall back to art.
pub(crate) fn object_target(channel: [u8; 16], object: [u8; 16], kind: StudioKind) -> StudioTarget {
    match kind {
        StudioKind::Flipnote => StudioTarget::Flipnote { channel, object },
        StudioKind::Score => StudioTarget::Score { channel, object },
    }
}
fn initial_header(target: StudioTarget, title: &str) -> Result<Vec<u8>, AppError> {
    match creation_kind(target)? {
        StudioKind::Flipnote => FlipnoteOp::SetHeader(FlipnoteHeader::Title(title.into()))
            .encode()
            .map_err(invalid),
        StudioKind::Score => ScoreOp::SetHeader(ScoreHeader::Title(title.into()))
            .encode()
            .map_err(invalid),
    }
}
fn domain(target: StudioTarget, nonce: [u8; 16], body: Vec<u8>) -> DomainOp {
    let (doc_type, key) = match target {
        StudioTarget::Index { channel } => (catcoms_wire::DocType::StudioIndex, channel),
        StudioTarget::Flipnote { object, .. } | StudioTarget::Score { object, .. } => {
            (catcoms_wire::DocType::StudioObject, object)
        }
    };
    DomainOp {
        nonce,
        doc_type,
        logical_key: key.to_vec(),
        body,
    }
}

/// A saved/read projection, not evidence of delivery or owner settlement. An absent object is
/// None; an absent Index reads as an empty epoch-zero view without creating a file.
#[derive(Debug)]
pub struct StudioView {
    pub epoch_id: u128,
    pub epoch: u64,
    pub phase: EpochPhase,
    pub projection: StudioProjection,
}

/// Caller-created, trusted-local custody only. `ordering` MUST retain the native numeric-server,
/// UI commit and registry-incarnation guards through the synchronous transaction AND bounded
/// initial publication, until lease destruction. No locks
/// may be awaited after the actor is Ready: use fail-fast acquisition and retry on contention.
pub struct StudioVaultLease {
    pub(crate) store: OwnedMutexGuard<Option<ServerStore>>,
    pub(crate) server: u64,
    _ordering: Box<dyn Send>,
    pub(crate) preview_reads: bool,
    cancellation: Option<catcoms_rt::RequestCancellation>,
}
impl std::fmt::Debug for StudioVaultLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioVaultLease { .. }")
    }
}
impl StudioVaultLease {
    pub fn new(
        store: OwnedMutexGuard<Option<ServerStore>>,
        server: u64,
        ordering: impl Send + 'static,
    ) -> Self {
        Self {
            store,
            server,
            _ordering: Box::new(ordering),
            preview_reads: false,
            cancellation: None,
        }
    }

    /// Transfer native cancellation AND its operation-slot keepalive to the actual worker/send
    /// lifetime. Dropping the invoke must not refund capacity while its finite save still runs.
    pub fn with_cancellation(mut self, cancellation: catcoms_rt::RequestCancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancellation.as_ref().is_some_and(|c| c.is_cancelled())
    }
    pub(crate) fn background_cancellation(&self) -> Option<catcoms_rt::RequestCancellation> {
        self.cancellation.clone()
    }
}

/// Two-phase rendezvous: the actor is already in its exclusive Studio arm, so acquiring the
/// vault now cannot block its event producer BEFORE it reaches this arm. Dropping closes it.
#[derive(Debug)]
pub struct StudioReady {
    pub(crate) lease: oneshot::Sender<StudioVaultLease>,
    pub(crate) result: oneshot::Receiver<Result<Option<StudioRead>, String>>,
}
impl StudioReady {
    /// Compatibility API for ordinary document consumers. Unconfirmed previews are omitted.
    pub async fn execute(self, lease: StudioVaultLease) -> Result<Option<StudioView>, String> {
        Ok(self
            .execute_result(lease)
            .await?
            .and_then(|read| match read {
                StudioRead::Document(view) => Some(view),
                StudioRead::AwaitingTenureReceipt(_) => None,
            }))
    }
    pub async fn execute_read(
        self,
        mut lease: StudioVaultLease,
    ) -> Result<Option<StudioRead>, String> {
        lease.preview_reads = true;
        self.execute_result(lease).await
    }
    async fn execute_result(self, lease: StudioVaultLease) -> Result<Option<StudioRead>, String> {
        self.lease
            .send(lease)
            .map_err(|_| "Studio transaction expired".to_string())?;
        self.result.await.unwrap_or_else(|_| {
            Err("Studio transaction expired, cancelled or server stopped".into())
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Local-only test helper. Production actor saves retain the exact durable packets instead
    /// of reconstructing/re-saving the same operations immediately for their initial send.
    #[cfg(test)]
    pub(crate) fn studio_transaction(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        request: StudioRequest,
    ) -> Result<Option<StudioView>, AppError> {
        self.studio_transaction_with_publication(store, server, request)
            .map(|saved| saved.view)
    }

    /// Synchronous under actor+vault+lifecycle custody. Full scans are bounded, but accepted-size
    /// latency remains unqualified; this explicit operation is not automatic network serving.
    pub(crate) fn studio_transaction_with_publication(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        request: StudioRequest,
    ) -> Result<StudioSavedTransaction, AppError> {
        let mut packets = Vec::new();
        request.validate()?;
        let target = request.target();
        if !self
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        // A new object starts at epoch zero, but its existing channel Index may have rotated.
        // Capture that checked source under the SAME exclusive transaction as both writes;
        // never derive a current Index epoch from a UI fixture or a Registry pointer hint.
        let mut create_index_epoch = None;
        let mut accepted_sound_retry = false;
        self.sync.with_registry_context(|group, device, _, _| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("Studio requires current membership"));
            }
            if let StudioRequest::Apply {
                target: StudioTarget::Index { .. },
                nonce,
                body,
                ..
            } = &request
            {
                let logical = target.document(&group.group_id()).map_err(invalid)?;
                IndexOp::decode_domain(
                    &logical,
                    &domain(target, *nonce, body.clone()),
                    &device.device_id(),
                )
                .map_err(invalid)?;
            }
            if let StudioRequest::Apply {
                target: StudioTarget::Flipnote { .. },
                epoch_id,
                nonce,
                body,
            } = &request
            {
                if matches!(
                    FlipnoteOp::decode(body).map_err(invalid)?,
                    FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some(_)))
                        | FlipnoteOp::SetExport { .. }
                ) {
                    // An acknowledgement of an exact accepted edit is not new link authority and
                    // remains retryable after the dependency later disappears. Fresh edits must
                    // independently authenticate the actual linked source before any new intent.
                    let op = domain(target, *nonce, body.clone());
                    accepted_sound_retry = store
                        .with_studio_source(server, group, target, device, |state| {
                            Ok(state.doc_id() == *epoch_id
                                && state.contains_exact_operation(device.device_id(), &op)?)
                        })?
                        .unwrap_or(false);
                    if !accepted_sound_retry
                        && !sound_link_exists(store, server, group, device, target, body)?
                    {
                        return Err(invalid(
                            "linked Score is not installed in this group and channel",
                        ));
                    }
                }
            }
            if let StudioRequest::Create {
                channel,
                object,
                nonce,
                title,
                ts,
            }
            | StudioRequest::CreateScore {
                channel,
                object,
                nonce,
                title,
                ts,
            } = &request
            {
                let initial = domain(target, *nonce, initial_header(target, title)?);
                if let Some(held) = store.load_studio_epoch(server, group, target, device)? {
                    if !held.contains_exact_operation(device.device_id(), &initial)? {
                        return Err(invalid("object already exists; use Apply to edit it"));
                    }
                }
                let index = StudioTarget::Index { channel: *channel };
                if let Some(held) = store.load_studio_epoch(server, group, index, device)? {
                    create_index_epoch = Some(held.doc_id());
                    let StudioProjection::Index(projection) = held.projection()? else {
                        return Err(invalid("wrong index type"));
                    };
                    if projection.objects.contains_key(object)
                        || projection.overflow.contains_key(object)
                        || projection.deleted_objects.contains_key(object)
                        || projection.tombstones.contains_key(object)
                    {
                        let put = domain(
                            index,
                            *nonce,
                            IndexOp::PutObject {
                                object: *object,
                                kind: creation_kind(target)?,
                                title: title.clone(),
                                created_by: device.device_id(),
                                ts: *ts,
                                expiry: StudioExpiry::Unrecorded,
                            }
                            .encode()
                            .map_err(invalid)?,
                        );
                        if !held.contains_exact_operation(device.device_id(), &put)? {
                            return Err(invalid("object id already occurs in the index"));
                        }
                    }
                }
            }
            Ok(())
        })?;
        // Replacing refresh MUST precede transient pre-holds. The later accounting scan stays
        // reference-neutral, or it could erase a PIX hold before its intent reaches disk.
        if !store.creative_references_known() {
            // Failed discovery keeps deletion disabled. It must not prevent reading an otherwise
            // healthy document; mutation accounting independently refuses unsafe disk state.
            let _ = store.creative_pinned_cids();
        }
        if let StudioRequest::Apply {
            target,
            nonce,
            body,
            ..
        } = &request
        {
            let logical = target.document(&self.group_id()).map_err(invalid)?;
            store.hold_creative_operation(&logical, &domain(*target, *nonce, body.clone()));
        }
        if let StudioRequest::ApplyBatch {
            target, operations, ..
        } = &request
        {
            let logical = target.document(&self.group_id()).map_err(invalid)?;
            for operation in operations {
                store.hold_creative_operation(
                    &logical,
                    &domain(*target, operation.nonce, operation.body.clone()),
                );
            }
        }
        // New local pixel references require exact already-held bytes in this mounted vault.
        // No network fetch or silent reference to a placeholder hash. The pre-hold above and
        // durable write hooks protect these bytes from same-mount cache reclamation; this
        // validation alone does not protect against filesystem damage or external deletion.
        if let StudioRequest::Apply {
            target: StudioTarget::Flipnote { .. },
            body,
            ..
        } = &request
        {
            match FlipnoteOp::decode(body).map_err(invalid)? {
                FlipnoteOp::InsertFrame { cid, bytes, .. }
                | FlipnoteOp::ReplaceFrame { cid, bytes, .. } => {
                    let mut blobs = store.blob_store(&hex::encode(self.group_id()))?;
                    let cid = crate::Cid::from_bytes(cid);
                    let pixels = blobs.get_bounded(&cid, bytes as usize)?.ok_or_else(|| {
                        invalid("publish the frame PIX before saving its reference")
                    })?;
                    if pixels.len() as u64 != bytes {
                        return Err(invalid("frame byte declaration differs from PIX"));
                    }
                    crate::creative::validate_pix(&pixels)?;
                    if pixels[4] != 191 || pixels[5] != 143 {
                        return Err(invalid("Flipnote frames must be 192x144"));
                    }
                    blobs.put_staged(&pixels)?;
                    if !blobs.promote_staged_bounded(&cid, pixels.len())? {
                        return Err(invalid("frame promotion failed"));
                    }
                }
                FlipnoteOp::SetExport { .. } if accepted_sound_retry => {
                    // Recovery/Copy's receiver still executes the ordinary Save transaction
                    // after its own exact-accepted shortcut. This is acknowledgement, not new
                    // media admission: lost bytes or evicted source history cannot strand it.
                    // The full current signed own log above, not a pending intent, proves it.
                }
                FlipnoteOp::SetExport { cid, bytes, .. } => {
                    // A renderer's declaration or prior publication ACK is not evidence that
                    // this mounted vault still holds a valid package. Validate exact held bytes
                    // before writing a durable intent/reference; the earlier conservative
                    // transient hold protects candidates from reclamation, not publication.
                    // Never fetch at a Save boundary.
                    let mut blobs = store.blob_store(&hex::encode(self.group_id()))?;
                    let cid = crate::Cid::from_bytes(cid);
                    let package = blobs.get_bounded(&cid, bytes as usize)?.ok_or_else(|| {
                        invalid("publish the PIXA before saving its export reference")
                    })?;
                    if package.len() as u64 != bytes {
                        return Err(invalid("export byte declaration differs from PIXA"));
                    }
                    crate::creative::validate_pixa(&package)?;
                    blobs.put_staged(&package)?;
                    if !blobs.promote_staged_bounded(&cid, package.len())? {
                        return Err(invalid("export promotion failed"));
                    }
                }
                _ => {}
            }
        }
        // MLS/device/tenure needed to reopen must be durable before a new Studio source names
        // them. Native holds the same numeric-id and incarnation guards as ordinary snapshots.
        if request.changes_state() {
            let snapshot = self.snapshot()?;
            self.sync
                .with_registry_context(|_, _, _, rng| store.save_server(server, &snapshot, rng))?;
        }
        let view = self
            .sync
            .with_registry_context(|group, device, clock, rng| {
                if group.member_signature_key(&device.device_id()).as_deref()
                    != Some(device.public_key_bytes().as_slice())
                {
                    return Err(invalid("Studio requires current membership"));
                }
                let read =
                    |store: &mut ServerStore, target| -> Result<Option<StudioView>, AppError> {
                        if let Some(view) =
                            store.with_studio_source(server, group, target, device, |state| {
                                Ok(StudioView {
                                    epoch_id: state.doc_id(),
                                    epoch: state.epoch(),
                                    phase: state.phase(),
                                    projection: state.projection()?,
                                })
                            })?
                        {
                            return Ok(Some(view));
                        }
                        if matches!(target, StudioTarget::Index { .. }) {
                            let empty = catcoms_replication::studio::StudioEpoch::new(
                                group,
                                target,
                                device.device_id(),
                            )
                            .map_err(invalid)?;
                            return Ok(Some(StudioView {
                                epoch_id: empty.doc_id(),
                                epoch: 0,
                                phase: EpochPhase::Open,
                                projection: empty.projection().map_err(invalid)?,
                            }));
                        }
                        Ok(None)
                    };
                if matches!(request, StudioRequest::Read { .. }) {
                    return read(store, target);
                }
                let mut scan = store.scan_epoch_storage_with_studio()?;
                while !scan.step()?.complete {}
                let inventory = scan.finish()?;
                let mut budget = store.studio_storage_budget(server, group, &inventory)?;
                // A single request seals at most two packets (Create: object then Index); a
                // batch seals one per operation, bounded by its validated item count.
                let packet_cap = match &request {
                    StudioRequest::ApplyBatch { operations, .. } => operations.len(),
                    _ => 2,
                };
                let mut apply =
                    |target: StudioTarget, epoch_id, nonce, body| -> Result<(), AppError> {
                        let logical = target.document(&group.group_id()).map_err(invalid)?;
                        let op = DomainOp {
                            nonce,
                            doc_type: logical.doc_type,
                            logical_key: logical.logical_key,
                            body,
                        };
                        // Only actual saved output can enter the private batch. No second mutation
                        // or reconstructed body is needed for the initial publication. The full
                        // signed log and intent remain the durable retry authority, not this packet.
                        if packets.len() >= packet_cap {
                            return Err(invalid("Studio save packet cap"));
                        }
                        let (sealed, _) = store.edit_studio_epoch(
                            server,
                            group,
                            target,
                            epoch_id,
                            device,
                            op,
                            clock.now_ms(),
                            rng,
                            &mut budget,
                        )?;
                        packets.push(publication::StudioSavedPacket {
                            target,
                            epoch_id,
                            sealed,
                        });
                        Ok(())
                    };
                match request {
                    StudioRequest::Apply {
                        target,
                        epoch_id,
                        nonce,
                        body,
                    } => apply(target, epoch_id, nonce, body)?,
                    StudioRequest::ApplyBatch {
                        target,
                        epoch_id,
                        operations,
                    } => {
                        // In order, each under the same epoch check and causal validation a
                        // lone Apply gets. The items before a refused one are already durable
                        // (their intents are journalled and their edits saved), so the error
                        // says which item stopped the batch: an exact retry reseals the saved
                        // items and meets the same refusal, which is the caller's cue to drop it.
                        let count = operations.len();
                        for (index, operation) in operations.into_iter().enumerate() {
                            apply(target, epoch_id, operation.nonce, operation.body).map_err(
                                |error| {
                                    invalid(format!(
                                        "batched Apply item {} of {count} was refused ({error}); \
                                         the items before it are saved, so retry the same batch \
                                         exactly or discard that item",
                                        index + 1
                                    ))
                                },
                            )?;
                        }
                    }
                    StudioRequest::Create {
                        channel,
                        object,
                        nonce,
                        title,
                        ts,
                    }
                    | StudioRequest::CreateScore {
                        channel,
                        object,
                        nonce,
                        title,
                        ts,
                    } => {
                        let object_target = target;
                        apply(
                            object_target,
                            epoch_zero_id(catcoms_wire::DocType::StudioObject, &object),
                            nonce,
                            initial_header(object_target, &title)?,
                        )?;
                        apply(
                            StudioTarget::Index { channel },
                            create_index_epoch.unwrap_or_else(|| {
                                epoch_zero_id(catcoms_wire::DocType::StudioIndex, &channel)
                            }),
                            nonce,
                            IndexOp::PutObject {
                                object,
                                kind: creation_kind(object_target)?,
                                title,
                                created_by: device.device_id(),
                                ts,
                                expiry: StudioExpiry::Unrecorded,
                            }
                            .encode()
                            .map_err(invalid)?,
                        )?;
                    }
                    StudioRequest::Read { .. } => unreachable!(),
                }
                read(store, target)
            })?;
        // A partial Create or failed final read returns Err above: none of its temporary packets
        // escape for automatic publication. Already saved local content/intents remain retryable.
        // A batch's packets all name one (target, epoch); the watch it establishes is one watch.
        let mut observed: Vec<_> = packets.iter().map(|p| (p.target, p.epoch_id)).collect();
        observed.dedup();
        if observed.is_empty() {
            let logical = target.document(&self.group_id()).map_err(invalid)?;
            observed.push((
                target,
                view.as_ref()
                    .map(|v| v.epoch_id)
                    .unwrap_or_else(|| epoch_zero_id(logical.doc_type, &logical.logical_key)),
            ));
        }
        Ok(StudioSavedTransaction {
            preview: None,
            view,
            packets,
            observed,
        })
    }
}
fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("Studio: {error}"))
}

#[cfg(test)]
mod tests;
