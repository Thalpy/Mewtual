//! Explicit, nonvisual recovery controls. These borrow the same actor/vault custody as Save;
//! a recovery record is historical content, never permission to install a checkpoint.

use super::*;
use catcoms_replication::studio::StudioRecovery;
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

pub enum StudioControlResponse {
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
                    pending_intents: store.load_epoch_intents(server, &logical)?.pending().len(),
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
