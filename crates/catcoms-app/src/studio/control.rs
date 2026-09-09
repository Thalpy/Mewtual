//! Explicit, nonvisual recovery controls. These borrow the same actor/vault custody as Save;
//! a recovery record is historical content, never permission to install a checkpoint.

use super::*;
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::{RecoveryReason, RecoveryTransition};

/// Fixed-size requests: the renderer names a saved version, never supplies a recovery payload.
#[derive(Debug)]
pub struct StudioControlRequest {
    pub target: StudioTarget,
    pub action: StudioControlAction,
}

#[derive(Debug)]
pub enum StudioControlAction {
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
    List(StudioRecoveryListing),
    Version(StudioRecoveryVersion),
    Export { snapshot: [u8; 32], bytes: Vec<u8> },
    Acknowledged(StudioRecoveryListing),
}
impl std::fmt::Debug for StudioControlResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Export bytes and even a historical title are private vault content.
        f.write_str(match self {
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
