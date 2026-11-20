//! Automatic SAME-original warm A recovery. Declared with the real Physical issuers.
//! Classification never supplies mutation authority or chooses scratch after another
//! role fails. Completed finite outcomes install before any fresh Source/Scope borrow.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    PreparedV2TextRetainRecoveryCapture, PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextRetainRecovery, V2TextRetainWarmRecoveryClassData,
};

pub(super) struct RetainWarmRecovery {
    read: Option<AdmittedV2TextRetainRecoveryRead>,
    repair: Option<AdmittedV2TextRetainRepair>,
    scratch: Option<AdmittedV2TextRetainScratch>,
    // LAST: every active marker/File/lease drains before the actual shared plan/payment.
    prepared: PreparedV2TextRetainRecovery,
}

impl RetainWarmRecovery {
    /// Never leave an admitted effect behind because Scope was stopped or its reply was
    /// lost. The cached actual result moves home even when its worker reports a late Err.
    pub(super) fn drain_cached(
        slot: &mut Option<Self>,
        transaction: &mut PreparedV2TextRetainTransaction,
    ) -> Result<(), AppError> {
        let Some(recovery) = slot.as_mut() else {
            return Ok(());
        };
        let result = if let Some(work) = recovery.read.take() {
            recovery.prepared.run_read(work)
        } else if let Some(work) = recovery.repair.take() {
            recovery.prepared.run_repair(work)
        } else if let Some(work) = recovery.scratch.take() {
            recovery.prepared.run_scratch(work)
        } else {
            Ok(())
        };
        let complete = recovery.prepared.has_completed_repair_data()
            || recovery.prepared.has_completed_scratch_data();
        if complete && recovery.prepared.install_into(transaction)? {
            drop(slot.take());
        }
        result
    }

    pub(super) fn before_stage(
        slot: &mut Option<Self>,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        transaction: &mut PreparedV2TextRetainTransaction,
    ) -> Result<(), AppError> {
        Self::drain_cached(slot, transaction)?;
        if transaction.has_ready_data() {
            return Ok(());
        }
        if slot.is_none() {
            *slot = Some(Self {
                read: None,
                repair: None,
                scratch: None,
                prepared: PreparedV2TextRetainRecovery::prepare(
                    store,
                    owner.source(),
                    transaction,
                )?,
            });
        }
        let recovery = slot.as_mut().ok_or_else(refused)?;
        if recovery.prepared.class_data().is_none() {
            // A failed capture may already hold Files and the conservative writer gate.
            // Keep its outer slot local, then drop it OUTSIDE all writers on every result;
            // a subsequent lock/close must not inherit unadmitted dormant native custody.
            let mut capture = Some(PreparedV2TextRetainRecoveryCapture::prepare(
                store,
                owner.source(),
                &recovery.prepared,
            )?);
            let resources = recovery.prepared.resources().clone();
            let admission = store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_retain_recovery_read_into(
                    lane,
                    predecessor,
                    &mut recovery.prepared,
                    &mut capture,
                    &mut recovery.read,
                )
            });
            drop(capture.take());
            Self::drain_cached(slot, transaction)?;
            admission?;
        }
        let recovery = slot.as_mut().ok_or_else(refused)?;
        // None means not yet classified; it is never interpreted as a clean workspace.
        match recovery.prepared.class_data().ok_or_else(refused)? {
            V2TextRetainWarmRecoveryClassData::CleanUnstarted
            | V2TextRetainWarmRecoveryClassData::CompleteStage => {
                drop(slot.take());
                Ok(())
            }
            V2TextRetainWarmRecoveryClassData::CompleteMain => {
                let ready = recovery.prepared.ready_repair()?;
                let mut physical = Some(PreparedV2TextRetainRepairLease::prepare(
                    store,
                    owner.source(),
                    ready,
                )?);
                let resources = ready.resources().clone();
                let admission = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_retain_repair_into(
                        lane,
                        predecessor,
                        ready,
                        &mut physical,
                        &mut recovery.repair,
                    )
                });
                drop(physical.take());
                Self::drain_cached(slot, transaction)?;
                admission
            }
            V2TextRetainWarmRecoveryClassData::FirstWalScratch => {
                let ready = recovery.prepared.ready_scratch()?;
                let mut physical = Some(PreparedV2TextRetainScratchLease::prepare(
                    store,
                    owner.source(),
                    ready,
                )?);
                let resources = ready.resources().clone();
                let admission = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_retain_scratch_into(
                        lane,
                        predecessor,
                        ready,
                        &mut physical,
                        &mut recovery.scratch,
                    )
                });
                drop(physical.take());
                Self::drain_cached(slot, transaction)?;
                admission
            }
        }
    }
}

#[cfg(all(test, windows))]
impl<T, R> PreparedV2TextHost<T, R> {
    /// Place an actual finite-worker failure AFTER the same completed pre-Create phase.
    /// No transaction/permit is manufactured; the ordinary driver must still obtain each
    /// real admission and recover the genuine native output left by that worker.
    pub(in crate::store) fn set_next_retain_fault_for_test(
        &mut self,
        cut: crate::store::persistence::v2_text::V2TextRetainTestCut,
    ) -> Result<(), AppError> {
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        if pending.predecessor.is_none() || !pending.producer_data()?.has_complete_data() {
            return Err(refused());
        }
        if pending.retained.is_none() {
            pending.retained = Some(PendingRetain::new());
        }
        pending
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .next_work_fault = Some(cut);
        Ok(())
    }
}
