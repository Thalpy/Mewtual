//! Explicit, nonvisual recovery controls. These borrow the same actor/vault custody as Save;
//! a recovery record is historical content, never permission to install a checkpoint.

use super::*;
use crate::store::StudioOverlayDisposalRequest;
use catcoms_replication::studio::{
    StudioDisposalMode, StudioDraftArchive, StudioOverlayDisposal, StudioRecovery,
};
use catcoms_replication::{RecoveryReason, RecoveryTransition};

/// The renderer names a saved version, never supplies a recovery payload. Apply echoes one
/// bounded canonical body from a preview; the actor revalidates it, never trusts that echo.
#[derive(Debug)]
pub struct StudioControlRequest {
    pub target: StudioTarget,
    pub action: StudioControlAction,
}
impl StudioControlRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        if let StudioControlAction::Apply(apply) = &self.action {
            if apply.body.len() > catcoms_replication::epoch::MAX_DOMAIN_OP_BYTES {
                return Err(invalid("recovery body exceeds the operation bound"));
            }
            StudioRequest::Apply {
                target: self.target,
                epoch_id: apply.epoch_id,
                nonce: apply.nonce,
                body: apply.body.clone(),
            }
            .validate()?;
        }
        Ok(())
    }
}

pub struct StudioRecoveryApply {
    pub snapshot: [u8; 32],
    pub item: StudioRecoveryItem,
    pub mode: StudioRecoveryMode,
    pub epoch_id: u128,
    pub expected_projection: [u8; 32],
    pub nonce: [u8; 16],
    pub body: Vec<u8>,
}
impl std::fmt::Debug for StudioRecoveryApply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioRecoveryApply { .. }")
    }
}
#[derive(Debug)]
pub struct StudioRecoveryPreview {
    pub target: StudioTarget,
    pub snapshot: [u8; 32],
    pub epoch_id: u128,
    pub fingerprint: [u8; 32],
    pub plan: StudioRecoveryPlan,
}

#[derive(Debug)]
pub enum StudioControlAction {
    /// Trusted-local two-visit inspection. Renderer input never contains a prepared result.
    InspectOverlay,
    FinishOverlayInspection(Box<StudioPreparedInspection>),
    /// Classify the retained draft without rebuilding it: which generation is live, whether a
    /// terminal manifest is retained, and whether an archive exists. Reads only.
    OverlayLifecycle,
    /// Build the archive payload and write the durable archive record. Two visits, like inspection,
    /// because the rebuild is detached work and the write is not.
    ///
    /// This is the only path that creates an archive, and therefore the only way a preserving
    /// disposal can ever have something to point at.
    ArchiveOverlay,
    FinishOverlayArchive(Box<StudioPreparedInspection>),
    /// Read the preserved archive back. Never a basis, a source or an owner claim: reading evidence
    /// cannot turn it into authority.
    ReadOverlayArchive,
    /// Destroy the preserved archive. Separately confirmed, and the only thing in the system that
    /// removes one.
    ReleaseOverlayArchive(Box<StudioArchiveReleaseRequest>),
    /// Drop the live branch, preserving or discarding its bodies. The archive for a preserving
    /// disposal must already be durable: evidence first, removal second.
    DisposeOverlay(Box<StudioOverlayDisposalRequest>),
    /// Separately retryable discoverability step after restoring content; never guesses an
    /// epoch from the UI or rewinds a pointer to a newer checkpoint.
    RestorePointer,
    Preview {
        snapshot: [u8; 32],
        item: StudioRecoveryItem,
        mode: StudioRecoveryMode,
    },
    Apply(Box<StudioRecoveryApply>),
    /// Read metadata for both retained slots and the optional staged slot. Never evicts.
    List,
    Read {
        snapshot: [u8; 32],
    },
    /// Export the bounded canonical recovery envelope, not an animation or fetched PIX bytes.
    Export {
        snapshot: [u8; 32],
    },
    /// Acknowledge precisely the warning displayed to the user, including on an exact retry.
    Acknowledge {
        oldest_snapshot: [u8; 32],
        staged_snapshot: [u8; 32],
    },
}

#[derive(Debug)]
pub struct StudioRecoverySummary {
    pub id: [u8; 32],
    pub epoch: u64,
    pub reason: RecoveryReason,
    pub staged: bool,
    pub encoded_bytes: usize,
}

/// Actual local source metadata. Open never means that its current edits are receipted.
#[derive(Debug)]
pub struct StudioSettlementSource {
    pub epoch_id: u128,
    pub epoch: u64,
    pub phase: EpochPhase,
}

#[derive(Debug)]
pub struct StudioRecoveryListing {
    pub target: StudioTarget,
    pub source: Option<StudioSettlementSource>,
    /// Newest retained first, then the staged version if any; at most three entries.
    pub versions: Vec<StudioRecoverySummary>,
    pub eviction_pending: Option<RecoveryTransition>,
    /// Pending is not synonymous with excluded: ordinary provisional Saves also keep intents.
    pub pending_intents: usize,
}

/// A distinct historical view, deliberately not a StudioView with a fabricated current phase.
#[derive(Debug)]
pub struct StudioRecoveryVersion {
    pub summary: StudioRecoverySummary,
    pub projection: StudioProjection,
}

/// What a release must name, so a confirmation cannot be spent on an archive the user never saw.
#[derive(Debug)]
pub struct StudioArchiveReleaseRequest {
    /// `StudioDraftArchive::archive_id`, from the read that populated the dialog.
    pub archive: [u8; 32],
    /// The typed confirmation. Its presence is the proof; the literal cannot be defaulted.
    pub confirmation: StudioReleaseConfirmation,
}

/// Proof that a human was shown a destructive choice and took it, for the release path.
///
/// Mirrors `StudioDiscardConfirmation`'s shape and for the same reason: a `bool` that happens to be
/// true, or a field a caller forgot to set, must not be able to destroy preserved evidence.
#[derive(Debug)]
pub struct StudioReleaseConfirmation(());

impl StudioReleaseConfirmation {
    pub const TOKEN: &'static str = "release-local-archive";

    /// Exact match only. A near miss is a caller that built the string rather than echoing the user.
    pub fn parse(value: &str) -> Option<Self> {
        (value == Self::TOKEN).then_some(Self(()))
    }
}

/// How a retained draft stands right now, without rebuilding it.
#[derive(Debug)]
pub struct StudioOverlayLifecycle {
    pub target: StudioTarget,
    /// `None` when no branch is live: the vault may still hold a terminal manifest.
    pub branch: Option<[u8; 32]>,
    /// The live branch's content digest, carried so a disposal can be addressed at all. Present
    /// exactly when `branch` is: they are read together from the same branch on purpose.
    pub content: Option<[u8; 32]>,
    pub generation: u64,
    pub accepted: usize,
    /// Whether a preserved archive exists for this document, and its identity if so.
    pub archive: Option<[u8; 32]>,
    /// Set when the most recent terminal event was a disposal, with the mode it recorded.
    pub disposed: Option<StudioDisposalMode>,
    pub transferred: bool,
}

pub enum StudioControlResponse {
    OverlayPreparation(StudioInspectionPreparation),
    OverlayInspection(StudioOverlayInspection),
    OverlayLifecycle(Box<StudioOverlayLifecycle>),
    /// An archive was just written. A distinct variant from `OverlayArchive` because one of these
    /// changed the vault and the other did not, and a caller that cannot tell them apart cannot
    /// tell a user whether anything happened.
    OverlayArchived {
        archive: Box<StudioDraftArchive>,
        id: [u8; 32],
        physical_bytes: u64,
        /// What typed reconstruction found at archive time. `Err` is not a failure of the archive:
        /// the branch was already unreplayable and the archive records that it was.
        replayable: Result<(), String>,
    },
    /// The decoded archive and its identity. Reading is not authority.
    OverlayArchive {
        archive: Box<StudioDraftArchive>,
        id: [u8; 32],
        physical_bytes: u64,
    },
    /// The archive is gone. Both budgets are closed; the caller reconciles before its next write.
    OverlayArchiveReleased,
    /// The branch is gone and this is the terminal record of it.
    OverlayDisposed(Box<StudioOverlayDisposal>),
    PointerRestored {
        target: StudioTarget,
        epoch: u64,
        registry_epoch_id: u128,
    },
    Preview(StudioRecoveryPreview),
    /// Ordinary Save completed (or was exactly retried). Neither pointer restoration nor
    /// remote delivery/owner settlement is implied. Read again before choosing another item.
    Applied {
        target: StudioTarget,
        already_saved: bool,
    },
    List(StudioRecoveryListing),
    Version(StudioRecoveryVersion),
    Export {
        snapshot: [u8; 32],
        bytes: Vec<u8>,
    },
    Acknowledged(StudioRecoveryListing),
}
impl std::fmt::Debug for StudioControlResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Export bytes and even a historical title are private vault content.
        f.write_str(match self {
            Self::OverlayPreparation(_) => "OverlayPreparation { .. }",
            Self::OverlayInspection(_) => "OverlayInspection { .. }",
            Self::OverlayLifecycle(_) => "OverlayLifecycle { .. }",
            Self::OverlayArchived { .. } => "OverlayArchived { .. }",
            Self::OverlayArchive { .. } => "OverlayArchive { .. }",
            Self::OverlayArchiveReleased => "OverlayArchiveReleased",
            Self::OverlayDisposed(_) => "OverlayDisposed { .. }",
            Self::PointerRestored { .. } => "PointerRestored { .. }",
            Self::Preview(_) => "Preview { .. }",
            Self::Applied { .. } => "Applied { .. }",
            Self::List(_) => "List { .. }",
            Self::Version(_) => "Version { .. }",
            Self::Export { .. } => "Export { .. }",
            Self::Acknowledged(_) => "Acknowledged { .. }",
        })
    }
}

#[derive(Debug)]
pub struct StudioControlReady {
    pub(crate) lease: oneshot::Sender<StudioVaultLease>,
    pub(crate) result: oneshot::Receiver<Result<StudioControlResponse, String>>,
}
impl StudioControlReady {
    pub async fn execute(self, lease: StudioVaultLease) -> Result<StudioControlResponse, String> {
        self.lease
            .send(lease)
            .map_err(|_| "Studio control expired".to_string())?;
        self.result
            .await
            .unwrap_or_else(|_| Err("Studio control expired, cancelled or server stopped".into()))
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Second visit of an archive: revalidate the capture, then write the record.
    ///
    /// The currency check is `finish_studio_inspection`'s, unchanged. What it buys here is the same
    /// thing it buys a read - that the branch did not change under the detached rebuild - but the
    /// consequence is larger, because this visit writes. An archive of a branch that has moved on
    /// would be evidence for work nobody did.
    ///
    /// The durable read at the end is not decoration: it is how the caller learns the physical size
    /// that its storage budget was just charged, and it proves the record is actually there rather
    /// than reporting success from the value that was written.
    fn finish_studio_archive(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        prepared: StudioPreparedInspection,
    ) -> Result<StudioControlResponse, AppError> {
        let inspection = self.finish_studio_inspection(store, server, target, prepared)?;
        let replayable = inspection.replayable();
        self.sync.with_registry_context(|group, device, _, rng| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("Studio requires current membership"));
            }
            let logical = target.document(&group.group_id()).map_err(invalid)?;
            let archive = inspection.archive()?;
            let mut scan = store.scan_epoch_storage_with_studio()?;
            while !scan.step()?.complete {}
            let inventory = scan.finish()?;
            let mut budget = store.studio_storage_budget(server, group, &inventory)?;
            store.write_studio_draft_archive(server, &logical, archive, rng, &mut budget)?;
            let (archive, id, physical_bytes) = store
                .read_studio_draft_archive_for_app(server, &logical)?
                .ok_or_else(|| invalid("the draft archive did not survive its own write"))?;
            Ok(StudioControlResponse::OverlayArchived {
                archive: Box::new(archive),
                id,
                physical_bytes,
                replayable,
            })
        })
    }

    /// Exclusive trusted-local custody only. All slots are authenticated and typed before any
    /// action, including ack. Corruption is never an empty recovery rail or an eviction permit.
    pub(crate) fn studio_control_transaction(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        request: StudioControlRequest,
    ) -> Result<StudioControlResponse, AppError> {
        request.validate()?;
        let target = request.target;
        if !self
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        let request = match request.action {
            StudioControlAction::InspectOverlay => {
                return self
                    .begin_studio_inspection(store, server, target)
                    .map(StudioControlResponse::OverlayPreparation)
            }
            StudioControlAction::FinishOverlayInspection(prepared) => {
                return self
                    .finish_studio_inspection(store, server, target, *prepared)
                    .map(StudioControlResponse::OverlayInspection)
            }
            // Archiving takes the same two visits as inspection and the same capture; only the
            // rebuild differs, which is why the first visit is literally the inspection's.
            StudioControlAction::ArchiveOverlay => {
                return self
                    .begin_studio_inspection(store, server, target)
                    .map(StudioControlResponse::OverlayPreparation)
            }
            StudioControlAction::FinishOverlayArchive(prepared) => {
                return self.finish_studio_archive(store, server, target, *prepared);
            }
            action => StudioControlRequest { target, action },
        };
        self.sync
            .with_registry_context(|group, device, clock, rng| {
                if group.member_signature_key(&device.device_id()).as_deref()
                    != Some(device.public_key_bytes().as_slice())
                {
                    return Err(invalid("Studio requires current membership"));
                }
                let logical = target.document(&group.group_id()).map_err(invalid)?;
                let mut recovery = store.load_epoch_recovery(server, &logical)?;
                // Validate even unselected slots. A malformed historical channel must not hide
                // behind a valid selected version, nor may its blobs be freed by acknowledgement.
                for snapshot in recovery.retained().chain(recovery.staged()) {
                    StudioRecovery::from_snapshot(snapshot, &logical, target.channel())
                        .map_err(invalid)?;
                }
                match request.action {
                    StudioControlAction::InspectOverlay
                    | StudioControlAction::FinishOverlayInspection(_)
                    | StudioControlAction::ArchiveOverlay
                    | StudioControlAction::FinishOverlayArchive(_) => {
                        unreachable!("inspection and archiving route before recovery decoding")
                    }
                    // Read-only. It deliberately does NOT rebuild the branch: the whole point is to
                    // tell a caller what it is looking at cheaply enough to do before deciding
                    // whether to pay for an inspection.
                    StudioControlAction::OverlayLifecycle => {
                        let state = store.load_epoch_intents_structural(server, &logical)?;
                        let metadata = state.handoff_metadata();
                        let live = state.live_branch()?;
                        let archive = store
                            .read_studio_draft_archive_for_app(server, &logical)?
                            .map(|(_, id, _)| id);
                        return Ok(StudioControlResponse::OverlayLifecycle(Box::new(
                            StudioOverlayLifecycle {
                                target,
                                branch: live.map(|live| live.id),
                                content: live.map(|live| live.content),
                                generation: metadata.map_or(0, |m| m.branch_generation()),
                                accepted: state.overlay().map_or(0, |o| o.accepted()),
                                archive,
                                disposed: metadata.and_then(|m| m.disposed()).map(|d| d.mode),
                                transferred: metadata.is_some_and(|m| m.has_completed()),
                            },
                        )));
                    }
                    StudioControlAction::ReadOverlayArchive => {
                        let (archive, id, physical_bytes) = store
                            .read_studio_draft_archive_for_app(server, &logical)?
                            .ok_or_else(|| {
                                invalid("no preserved draft archive for this document")
                            })?;
                        return Ok(StudioControlResponse::OverlayArchive {
                            id,
                            physical_bytes,
                            archive: Box::new(archive),
                        });
                    }
                    StudioControlAction::ReleaseOverlayArchive(request) => {
                        // Destructured rather than field-accessed so the confirmation is visibly
                        // spent here. Release has no mode to match on, so the confirmation is
                        // purely a construction gate: naming it is what stops a later edit from
                        // deleting the field without a single compiler error.
                        let StudioArchiveReleaseRequest {
                            archive,
                            confirmation,
                        } = *request;
                        let StudioReleaseConfirmation(()) = confirmation;
                        let mut scan = store.scan_epoch_storage_with_studio()?;
                        while !scan.step()?.complete {}
                        let inventory = scan.finish()?;
                        let mut budget = store.studio_storage_budget(server, group, &inventory)?;
                        store.release_studio_draft_archive(
                            server,
                            &logical,
                            archive,
                            &mut budget,
                        )?;
                        return Ok(StudioControlResponse::OverlayArchiveReleased);
                    }
                    StudioControlAction::DisposeOverlay(request) => {
                        let mut scan = store.scan_epoch_storage_with_studio()?;
                        while !scan.step()?.complete {}
                        let inventory = scan.finish()?;
                        let mut budget = store.studio_storage_budget(server, group, &inventory)?;
                        let manifest = store.dispose_studio_overlay(
                            server,
                            &logical,
                            target,
                            group,
                            device,
                            *request,
                            clock.now_ms(),
                            rng,
                            &mut budget,
                        )?;
                        return Ok(StudioControlResponse::OverlayDisposed(Box::new(manifest)));
                    }
                    StudioControlAction::RestorePointer => {
                        let mut scan = store.scan_epoch_storage_with_studio()?;
                        while !scan.step()?.complete {}
                        let inventory = scan.finish()?;
                        let mut budget = store.studio_storage_budget(server, group, &inventory)?;
                        let (epoch, registry_epoch_id) = store.restore_studio_registry_pointer(
                            server,
                            group,
                            target,
                            device,
                            rng,
                            &mut budget,
                        )?;
                        return Ok(StudioControlResponse::PointerRestored {
                            target,
                            epoch,
                            registry_epoch_id,
                        });
                    }
                    StudioControlAction::Preview {
                        snapshot,
                        item,
                        mode,
                    } => {
                        return self::preview(
                            store, server, group, device, target, &recovery, snapshot, item, mode,
                        )
                        .map(StudioControlResponse::Preview);
                    }
                    StudioControlAction::Apply(_) => {
                        return Err(invalid(
                            "recovery Apply requires the ordinary publication path",
                        ))
                    }
                    StudioControlAction::Read { snapshot }
                    | StudioControlAction::Export { snapshot } => {
                        let saved = recovery
                            .retained()
                            .chain(recovery.staged())
                            .find(|s| s.id().ok() == Some(snapshot))
                            .ok_or_else(|| invalid("recovery version is no longer retained"))?;
                        let bytes = saved.encode().map_err(invalid)?;
                        if matches!(request.action, StudioControlAction::Export { .. }) {
                            return Ok(StudioControlResponse::Export { snapshot, bytes });
                        }
                        let typed =
                            StudioRecovery::from_snapshot(saved, &logical, target.channel())
                                .map_err(invalid)?;
                        return Ok(StudioControlResponse::Version(StudioRecoveryVersion {
                            summary: StudioRecoverySummary {
                                id: snapshot,
                                epoch: saved.epoch,
                                reason: saved.reason,
                                staged: recovery.staged().is_some_and(|s| s == saved),
                                encoded_bytes: bytes.len(),
                            },
                            projection: typed.projection().clone(),
                        }));
                    }
                    StudioControlAction::Acknowledge {
                        oldest_snapshot,
                        staged_snapshot,
                    } => {
                        // Same five-family accounting owner as Save/rotation; no unaccounted write
                        // and no source pruning here. The next ordinary receiver turn resumes it.
                        let mut scan = store.scan_epoch_storage_with_studio()?;
                        while !scan.step()?.complete {}
                        let inventory = scan.finish()?;
                        let mut budget = store.studio_storage_budget(server, group, &inventory)?;
                        recovery = store.with_studio_protocol_budget(
                            server,
                            group,
                            &mut budget,
                            |store, budget| {
                                store
                                    .update_epoch_recovery_accounted(
                                        server,
                                        &logical,
                                        crate::store::EpochRecoveryAction::Acknowledge {
                                            oldest_snapshot,
                                            staged_snapshot,
                                        },
                                        clock,
                                        rng,
                                        budget,
                                    )
                                    .map(|updated| updated.state)
                            },
                        )?;
                    }
                    StudioControlAction::List => {}
                }
                let source = store.with_studio_source(server, group, target, device, |state| {
                    Ok(StudioSettlementSource {
                        epoch_id: state.doc_id(),
                        epoch: state.epoch(),
                        phase: state.phase(),
                    })
                })?;
                let versions = recovery
                    .retained()
                    .chain(recovery.staged())
                    .map(|s| {
                        Ok(StudioRecoverySummary {
                            id: s.id().map_err(invalid)?,
                            epoch: s.epoch,
                            reason: s.reason,
                            staged: recovery.staged().is_some_and(|staged| staged == s),
                            encoded_bytes: s.encode().map_err(invalid)?.len(),
                        })
                    })
                    .collect::<Result<Vec<_>, AppError>>()?;
                let listing = StudioRecoveryListing {
                    target,
                    source,
                    versions,
                    eviction_pending: recovery.eviction_pending()?,
                    pending_intents: store
                        .load_epoch_intents_structural(server, &logical)?
                        .pending()
                        .len(),
                };
                Ok(
                    if matches!(request.action, StudioControlAction::Acknowledge { .. }) {
                        StudioControlResponse::Acknowledged(listing)
                    } else {
                        StudioControlResponse::List(listing)
                    },
                )
            })
    }
}

/// Recompute from saved typed content, including other recovery slots that may have changed
/// since the UI's preview. In particular, a new tombstone cannot bypass eligibility via an
/// unchanged current-projection hash.
#[allow(clippy::too_many_arguments)]
fn preview(
    store: &mut ServerStore,
    server: u64,
    group: &catcoms_mls::ServerGroup,
    device: &catcoms_mls::MlsDevice,
    target: StudioTarget,
    recovery: &crate::store::EpochRecoveryState,
    snapshot: [u8; 32],
    item: StudioRecoveryItem,
    mode: StudioRecoveryMode,
) -> Result<StudioRecoveryPreview, AppError> {
    let logical = target.document(&group.group_id()).map_err(invalid)?;
    let history = recovery
        .retained()
        .chain(recovery.staged())
        .map(|s| StudioRecovery::from_snapshot(s, &logical, target.channel()).map_err(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    let index = recovery
        .retained()
        .chain(recovery.staged())
        .position(|s| s.id().ok() == Some(snapshot))
        .ok_or_else(|| invalid("recovery version is no longer retained"))?;
    let (epoch_id, phase, projection) = store
        .with_studio_source(server, group, target, device, |s| {
            Ok((s.doc_id(), s.phase(), s.projection()?))
        })?
        .unwrap_or_else(|| {
            let state = types::StudioEpoch::new(group, target, device.device_id())
                .expect("validated Studio target");
            (
                state.doc_id(),
                state.phase(),
                state.projection().expect("empty Studio projection"),
            )
        });
    if phase != EpochPhase::Open {
        return Err(invalid("Studio recovery edits require an Open epoch"));
    }
    let mut plan = super::restore::plan(
        &projection,
        history[index].projection(),
        &history,
        item,
        mode,
        device.device_id(),
    )?;
    if plan.disposition == StudioRecoveryDisposition::Ready {
        if let StudioRecoveryItem::Object { id } = item {
            // An index entry itself proves nothing about its object. Do not republish a dangling
            // or differently scoped historical object, even when the requested Put is well formed.
            let target = StudioTarget::Flipnote {
                channel: target.channel(),
                object: id,
            };
            let exists = store
                .with_studio_source(server, group, target, device, |s| {
                    Ok(s.op_count() > 0 || s.epoch() > 0)
                })?
                .unwrap_or(false);
            if !exists {
                plan = StudioRecoveryPlan {
                    disposition: StudioRecoveryDisposition::MissingTarget,
                    body: None,
                    original_author: None,
                };
            }
        }
    }
    Ok(StudioRecoveryPreview {
        target,
        snapshot,
        epoch_id,
        fingerprint: projection.recovery_fingerprint().map_err(invalid)?,
        plan,
    })
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Exact current-log retry comes before the selected snapshot lookup: successful recovery
    /// remains retryable after that old snapshot is evicted. A pending intent alone cannot skip
    /// the fresh eligibility and stale-view checks. No state is changed by this preparation.
    pub(crate) fn prepare_studio_recovery_apply(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        apply: StudioRecoveryApply,
    ) -> Result<(StudioRequest, bool), AppError> {
        if apply.body.len() > catcoms_replication::epoch::MAX_DOMAIN_OP_BYTES {
            return Err(invalid("recovery body exceeds the operation bound"));
        }
        let request = StudioRequest::Apply {
            target,
            epoch_id: apply.epoch_id,
            nonce: apply.nonce,
            body: apply.body.clone(),
        };
        request.validate()?;
        if !self
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        let already_saved = self.sync.with_registry_context(|group, device, _, _| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("Studio requires current membership"));
            }
            let logical = target.document(&group.group_id()).map_err(invalid)?;
            let recovery = store.load_epoch_recovery(server, &logical)?;
            for s in recovery.retained().chain(recovery.staged()) {
                StudioRecovery::from_snapshot(s, &logical, target.channel()).map_err(invalid)?;
            }
            let op = domain(target, apply.nonce, apply.body.clone());
            let exact = store
                .with_studio_source(server, group, target, device, |state| {
                    if state.doc_id() != apply.epoch_id || state.phase() != EpochPhase::Open {
                        return Err(invalid("recovery preview epoch is no longer Open/current"));
                    }
                    state.contains_exact_operation(device.device_id(), &op)
                })?
                .unwrap_or(false);
            if exact {
                return Ok(true);
            }
            let plan = preview(
                store,
                server,
                group,
                device,
                target,
                &recovery,
                apply.snapshot,
                apply.item,
                apply.mode,
            )?;
            if plan.epoch_id != apply.epoch_id || plan.fingerprint != apply.expected_projection {
                return Err(invalid("recovery preview is stale; preview again"));
            }
            if plan.plan.disposition != StudioRecoveryDisposition::Ready
                || plan.plan.body.as_ref() != Some(&apply.body)
            {
                return Err(invalid(
                    "recovery choice is no longer Ready or body differs",
                ));
            }
            Ok(false)
        })?;
        Ok((request, already_saved))
    }
}
