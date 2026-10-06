//! The visible exit from Fault (design 5.7, 6.1 and Flow X). The runtime never selects a winner:
//! a read reports both candidates, and a repair echoes both hashes and the chosen one, which the
//! store re-derives under custody. Issuance takes V5's authoring requirement for its distinct
//! refusals and the durable owner snapshot for a snapshot-covered tenure, and refuses if the two
//! disagree. Application by a peer needs an authoring tenure too: `Imported` and `Unknown` hold.

use super::*;
use crate::registry_head::ServerOwnerSnapshot;
use crate::store::{
    EpochRegistryState, EpochStudioBudget, EpochStudioState, StudioFaultEvidence,
    StudioRepairOutcome, StudioRepairRequest,
};
use catcoms_replication::{Receipt, ReceiptRepair, RepairDisposition};
use std::sync::Arc;

/// What a person needs to choose between two conflicting receipts, and nothing that chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StudioFaultCandidate {
    pub receipt_hash: [u8; 32],
    pub closed_epoch: u64,
    pub close_record_hash: [u8; 32],
    pub seed_change_hash: [u8; 32],
    pub inherited_epoch: Option<u64>,
    /// This source installed exactly this checkpoint. Not a tie-break.
    pub locally_installed: bool,
}

/// A signed repair as this device holds it. `held` means persisted at B1 and not yet recycled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioRepairStatus {
    pub repair_hash: [u8; 32],
    pub selected: [u8; 32],
    pub sequence: u64,
    pub held: bool,
    pub disposition: Option<RepairDisposition>,
    pub install_pending: bool,
    pub installed: bool,
}

/// Why this device may not decide now. Each is a different situation for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRepairBlocker {
    NotOwner,
    /// An unverified imported tenure. Waiting does not resolve this.
    TenureUnverified,
    /// The owner has not been observed taking office yet.
    TenureUnobserved,
    /// A decision is already persisted; it is resumed, never replaced.
    HeldRepair,
    NoFault,
    /// This device's repair job for this fault is running. Its result appears as
    /// `last_attempt` once it commits.
    Scheduled,
}

/// What asking for a repair did. The repair itself runs as a detached job (design 10.3), so a
/// request never waits for it; its outcome is read back through the fault view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRepairStart {
    /// A job for exactly this work is running, newly or from an earlier identical request.
    Scheduled,
    /// Another repair job, or a full shared preparation pool. Nothing was reserved; ask again.
    Busy,
}

/// The last repair attempt this device's runtime finished for a fault: volatile, local and
/// bounded, never durable state. Lets a person see what happened to a decision they scheduled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioRepairReport {
    Completed(StudioRepairOutcome),
    /// Bounded error text. Nothing was claimed beyond what the store committed before it.
    Failed(String),
}

/// Which document a fault view or repair is about: the target's own source, or the Registry
/// bucket that makes it discoverable. Explicit, so one decision can never be read as the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioFaultScope {
    Source,
    RegistryBucket(u8),
}

#[derive(Debug)]
pub struct StudioFaultView {
    pub target: StudioTarget,
    pub scope: StudioFaultScope,
    pub source: control::StudioSettlementSource,
    pub candidates: Option<[StudioFaultCandidate; 2]>,
    pub repair: Option<StudioRepairStatus>,
    pub may_decide: bool,
    pub blocked_by: Option<StudioRepairBlocker>,
    /// Further retained pairs, decidable one at a time after this one.
    pub waiting: usize,
    /// Operations a replacement would move into recovery. Not a claim they are lost.
    pub preserved_operations: usize,
    /// The last repair attempt this runtime finished for this scope, if it remembers one.
    pub last_attempt: Option<StudioRepairReport>,
}

impl StudioFaultView {
    /// Add what only the runtime knows: whether its job for this scope is running, and how its
    /// last one ended. A running job blocks a second decision, as a held one does.
    pub(crate) fn annotate_runtime(&mut self, running: bool, last: Option<StudioRepairReport>) {
        if running {
            self.blocked_by = Some(StudioRepairBlocker::Scheduled);
            self.may_decide = false;
        }
        self.last_attempt = last;
    }
}

fn candidate(receipt: &Receipt, opening: Option<[u8; 32]>) -> StudioFaultCandidate {
    StudioFaultCandidate {
        receipt_hash: receipt.hash(),
        closed_epoch: receipt.closed_epoch,
        close_record_hash: receipt.close_record_hash,
        seed_change_hash: receipt.seed_change_hash,
        inherited_epoch: match receipt.inherited {
            catcoms_replication::InheritedCheckpoint::EpochZero => None,
            catcoms_replication::InheritedCheckpoint::Checkpoint { epoch, .. } => Some(epoch),
        },
        locally_installed: opening == Some(receipt.hash()),
    }
}

impl StudioFaultView {
    fn new(
        target: StudioTarget,
        scope: StudioFaultScope,
        evidence: StudioFaultEvidence,
        owner: bool,
        tenure: StudioOwnerTenure,
    ) -> Self {
        let candidates = evidence
            .decidable
            .as_ref()
            .map(|pair| pair.each_ref().map(|r| candidate(r, evidence.opening)));
        let repair = match (&evidence.held, &evidence.resolved) {
            (Some(held), resolved) => Some(StudioRepairStatus {
                repair_hash: held.hash(),
                selected: held.selected_receipt_hash,
                sequence: held.repair_sequence,
                held: true,
                disposition: resolved
                    .as_ref()
                    .filter(|s| s.repair == *held)
                    .map(|s| s.disposition),
                install_pending: resolved
                    .as_ref()
                    .is_some_and(|s| s.repair == *held && s.install_pending),
                installed: resolved
                    .as_ref()
                    .is_some_and(|s| s.repair == *held && s.installed),
            }),
            (None, Some(state)) => Some(StudioRepairStatus {
                repair_hash: state.repair.hash(),
                selected: state.repair.selected_receipt_hash,
                sequence: state.repair.repair_sequence,
                held: false,
                disposition: Some(state.disposition),
                install_pending: state.install_pending,
                installed: state.installed,
            }),
            (None, None) => None,
        };
        let blocked_by = if !owner {
            Some(StudioRepairBlocker::NotOwner)
        } else if let StudioOwnerTenure::Imported(_) = tenure {
            Some(StudioRepairBlocker::TenureUnverified)
        } else if tenure == StudioOwnerTenure::Unknown {
            Some(StudioRepairBlocker::TenureUnobserved)
        } else if evidence.held.is_some() {
            Some(StudioRepairBlocker::HeldRepair)
        } else if candidates.is_none() {
            Some(StudioRepairBlocker::NoFault)
        } else {
            None
        };
        Self {
            target,
            scope,
            source: control::StudioSettlementSource {
                epoch_id: evidence.doc_id,
                epoch: evidence.epoch,
                phase: evidence.phase,
            },
            candidates,
            repair,
            may_decide: blocked_by.is_none(),
            blocked_by,
            waiting: evidence.waiting,
            preserved_operations: evidence.operations,
            last_attempt: None,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn check_owner_snapshot(
        &self,
        store: &ServerStore,
        server: u64,
        snapshot: &ServerOwnerSnapshot,
    ) -> Result<(), AppError> {
        if snapshot.server != server || !Arc::ptr_eq(&snapshot.mount, &store.registry_mount()) {
            return Err(AppError::Invalid(
                "owner snapshot belongs to another mount/server".into(),
            ));
        }
        Ok(())
    }

    /// S-4, read-only. Anyone may read; only the current owner with a known tenure may decide.
    pub(crate) fn read_studio_fault(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
    ) -> Result<StudioFaultView, AppError> {
        let tenure = self.observed_owner_tenure();
        let owner = self.sync.with_registry_context(|group, device, _, _| {
            group.designated_committer() == Some(device.device_id())
        });
        let known = match tenure {
            StudioOwnerTenure::Known(start) if owner => Some(start),
            _ => None,
        };
        let evidence = self
            .sync
            .with_registry_context(|group, device, _, _| {
                store.studio_fault_evidence(server, group, target, device, known)
            })?
            .ok_or_else(|| AppError::Invalid("no saved source for this target".into()))?;
        Ok(StudioFaultView::new(
            target,
            StudioFaultScope::Source,
            evidence,
            owner,
            tenure,
        ))
    }

    /// Owner issuance and application in one custody visit (Flow I then Flow A). The signed
    /// repair exists for anyone else only after B1 returns. An exact retry of a held decision
    /// resumes it; a different selection while one is held refuses.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_studio_fault_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        snapshot: &ServerOwnerSnapshot,
        request: StudioRepairRequest,
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochStudioState), AppError> {
        // V5 first: Known alone, with distinct refusals for Imported and Unknown, before any
        // other check can answer with a less specific reason.
        let observed = self.require_observed_owner_tenure()?;
        self.check_studio_fault_channel(target)?;
        self.check_owner_snapshot(store, server, snapshot)?;
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |group, device, rng, tenure| {
                if tenure != observed {
                    return Err(AppError::Invalid(
                        "observed and durable owner tenure disagree".into(),
                    ));
                }
                store.issue_studio_repair(
                    server,
                    group,
                    target,
                    device,
                    tenure,
                    request,
                    raw_seed,
                    clock.as_ref(),
                    rng,
                    budget,
                )
            })?
    }

    /// The owner resumes its own persisted decision: after a crash, or once the selected seed
    /// arrives. Same authority as issuance; it never signs anything new.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_studio_fault_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        snapshot: &ServerOwnerSnapshot,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochStudioState), AppError> {
        let observed = self.require_observed_owner_tenure()?;
        self.check_studio_fault_channel(target)?;
        self.check_owner_snapshot(store, server, snapshot)?;
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |group, device, rng, tenure| {
                if tenure != observed {
                    return Err(AppError::Invalid(
                        "observed and durable owner tenure disagree".into(),
                    ));
                }
                store.apply_studio_repair(
                    server,
                    group,
                    target,
                    device,
                    repair,
                    pair,
                    tenure,
                    raw_seed,
                    clock.as_ref(),
                    rng,
                    budget,
                )
            })?
    }

    /// Flow A for a peer that received a distributed repair. Applying is authoring under 6.3, so
    /// an unverified or unobserved tenure holds rather than substituting the repair's own claim.
    /// The owner must use [`Self::resume_studio_fault_repair`], which writes its owner record.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_studio_fault_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochStudioState), AppError> {
        let tenure = self.require_observed_owner_tenure()?;
        self.check_studio_fault_channel(target)?;
        self.sync
            .with_registry_context(|group, device, clock, rng| {
                if group.designated_committer() == Some(device.device_id()) {
                    return Err(AppError::Invalid(
                        "the owner resumes repairs through its durable snapshot".into(),
                    ));
                }
                store.apply_studio_repair(
                    server, group, target, device, repair, pair, tenure, raw_seed, clock, rng,
                    budget,
                )
            })
    }

    /// The seed a fetched pass verified for exactly this repair's selected receipt. Applying is
    /// authoring (6.3), so the pass must have been selected under this device's observed owner
    /// tenure: a proof pass carries the proof's own claim, which is never repair evidence. The
    /// Studio repair job calls this at S1 and carries the bytes to its S3.
    pub(crate) fn repaired_seed_bytes(
        &mut self,
        store: &ServerStore,
        server: u64,
        pass: &crate::studio_exchange::discovery::ServerCheckpointFetch,
        repair: &ReceiptRepair,
    ) -> Result<Vec<u8>, AppError> {
        let observed = self.require_observed_owner_tenure()?;
        if pass.server != server || !Arc::ptr_eq(&pass.mount, &store.registry_mount()) {
            return Err(AppError::Invalid(
                "seed pass belongs to a replaced mount".into(),
            ));
        }
        self.sync
            .with_checkpoint_seed_selection(&pass.inner, |_, _, _, selected| {
                if selected.receipt.hash() != repair.selected_receipt_hash {
                    return Err(AppError::Invalid(
                        "seed pass is not for this repair's selection".into(),
                    ));
                }
                if selected.tenure != observed {
                    return Err(AppError::Invalid(
                        "seed pass was not selected under the observed owner tenure".into(),
                    ));
                }
                selected
                    .checkpoint
                    .map(|seed| seed.bytes().to_vec())
                    .ok_or_else(|| AppError::Invalid("no verified seed for this repair".into()))
            })?
    }

    /// Install a fetched selected seed for a repair this bucket owes, through the repair
    /// transaction itself so the outcome is typed (Installed, RecoveryPending, StorageRefused)
    /// and an owner's record is recycled in the same step. The owner goes through its durable
    /// snapshot exactly as a resume does; a peer through Flow A, which refuses the owner. The
    /// Studio equivalent is the repair job's S3, `execute_replace`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn install_repaired_registry_seed(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        pass: &crate::studio_exchange::discovery::ServerCheckpointFetch,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        owner_snapshot: Option<&ServerOwnerSnapshot>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochRegistryState), AppError> {
        let catcoms_sync::checkpoint_exchange::CheckpointTarget::Registry(bucket) =
            pass.inner.target()
        else {
            return Err(AppError::Invalid(
                "a bucket repair needs a Registry seed".into(),
            ));
        };
        let seed = self.repaired_seed_bytes(store, server, pass, repair)?;
        match owner_snapshot {
            Some(snapshot) => self.resume_registry_bucket_repair(
                store,
                server,
                bucket,
                snapshot,
                repair,
                pair,
                Some(&seed),
                budget,
            ),
            None => self.apply_registry_bucket_repair(
                store,
                server,
                bucket,
                repair,
                pair,
                Some(&seed),
                budget,
            ),
        }
    }

    /// The Registry bucket a Studio target's pointer lives in, the scope of its discoverability.
    pub(crate) fn studio_registry_bucket(&self, target: StudioTarget) -> Result<u8, AppError> {
        let logical = target
            .document(&self.group_id())
            .map_err(|e| AppError::Invalid(e.to_string()))?;
        Ok(
            catcoms_replication::registry::PointerKey::new(logical.doc_type, logical.logical_key)
                .map_err(|e| AppError::Invalid(e.to_string()))?
                .bucket(),
        )
    }

    /// S-4 for the target's Registry bucket. A faulted bucket blocks discovery of a healthy Index
    /// or Flipnote, so it is decidable on its own, under an explicit scope.
    pub(crate) fn read_registry_fault(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
    ) -> Result<StudioFaultView, AppError> {
        self.check_studio_fault_channel(target)?;
        let bucket = self.studio_registry_bucket(target)?;
        let tenure = self.observed_owner_tenure();
        let owner = self.sync.with_registry_context(|group, device, _, _| {
            group.designated_committer() == Some(device.device_id())
        });
        let known = match tenure {
            StudioOwnerTenure::Known(start) if owner => Some(start),
            _ => None,
        };
        let evidence = self
            .sync
            .with_registry_context(|group, device, _, _| {
                store.registry_fault_evidence(server, group, bucket, device, known)
            })?
            .ok_or_else(|| AppError::Invalid("no saved Registry bucket for this target".into()))?;
        Ok(StudioFaultView::new(
            target,
            StudioFaultScope::RegistryBucket(bucket),
            evidence,
            owner,
            tenure,
        ))
    }

    /// Owner issuance and application for the target's Registry bucket, under the same V5 and
    /// durable-snapshot authority as a Studio source repair.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_registry_fault_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        snapshot: &ServerOwnerSnapshot,
        request: StudioRepairRequest,
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochRegistryState), AppError> {
        let observed = self.require_observed_owner_tenure()?;
        self.check_studio_fault_channel(target)?;
        self.check_owner_snapshot(store, server, snapshot)?;
        let bucket = self.studio_registry_bucket(target)?;
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |group, device, rng, tenure| {
                if tenure != observed {
                    return Err(AppError::Invalid(
                        "observed and durable owner tenure disagree".into(),
                    ));
                }
                store.with_studio_protocol_budget(server, group, budget, |store, storage| {
                    store.issue_registry_repair(
                        server,
                        group,
                        bucket,
                        device,
                        tenure,
                        request,
                        raw_seed,
                        clock.as_ref(),
                        rng,
                        storage,
                    )
                })
            })?
    }

    /// The owner resumes its own persisted Registry decision. It never signs anything new.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_registry_fault_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        snapshot: &ServerOwnerSnapshot,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochRegistryState), AppError> {
        // V5 first, so an unobserved tenure is refused as such before any channel check.
        self.require_observed_owner_tenure()?;
        self.check_studio_fault_channel(target)?;
        let bucket = self.studio_registry_bucket(target)?;
        self.resume_registry_bucket_repair(
            store, server, bucket, snapshot, repair, pair, raw_seed, budget,
        )
    }

    /// Bucket-keyed owner resume, for a fetched bucket seed that knows only its bucket.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resume_registry_bucket_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        bucket: u8,
        snapshot: &ServerOwnerSnapshot,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochRegistryState), AppError> {
        let observed = self.require_observed_owner_tenure()?;
        self.check_owner_snapshot(store, server, snapshot)?;
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |group, device, rng, tenure| {
                if tenure != observed {
                    return Err(AppError::Invalid(
                        "observed and durable owner tenure disagree".into(),
                    ));
                }
                store.with_studio_protocol_budget(server, group, budget, |store, storage| {
                    store.apply_registry_repair(
                        server,
                        group,
                        bucket,
                        device,
                        repair,
                        pair,
                        tenure,
                        raw_seed,
                        clock.as_ref(),
                        rng,
                        storage,
                    )
                })
            })?
    }

    /// Flow A for a peer applying a distributed Registry repair; authoring tenure required.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_registry_fault_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochRegistryState), AppError> {
        self.check_studio_fault_channel(target)?;
        let bucket = self.studio_registry_bucket(target)?;
        self.apply_registry_bucket_repair(store, server, bucket, repair, pair, raw_seed, budget)
    }

    /// Bucket-keyed Flow A, for a Registry discovery completion that knows only its bucket.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_registry_bucket_repair(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        bucket: u8,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        raw_seed: Option<&[u8]>,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochRegistryState), AppError> {
        let tenure = self.require_observed_owner_tenure()?;
        self.sync
            .with_registry_context(|group, device, clock, rng| {
                if group.designated_committer() == Some(device.device_id()) {
                    return Err(AppError::Invalid(
                        "the owner resumes repairs through its durable snapshot".into(),
                    ));
                }
                store.with_studio_protocol_budget(server, group, budget, |store, storage| {
                    store.apply_registry_repair(
                        server, group, bucket, device, repair, pair, tenure, raw_seed, clock, rng,
                        storage,
                    )
                })
            })
    }

    fn check_studio_fault_channel(&self, target: StudioTarget) -> Result<(), AppError> {
        if !self
            .channels()
            .iter()
            .any(|channel| channel.id == u128::from_be_bytes(target.channel()))
        {
            return Err(AppError::Invalid("unknown Studio channel".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
