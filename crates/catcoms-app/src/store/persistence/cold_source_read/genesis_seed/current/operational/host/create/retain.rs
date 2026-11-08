//! Owning A dispatcher. Declared only together with the real Physical A cell and Factory
//! Stage/Commit issuers. Retaining the packet is not Append acceptance or publication.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRetainCommit, AdmittedV2TextRetainStage, PreparedV2TextPhysicalLease,
};
use crate::store::persistence::v2_text::{PreparedV2TextRetainTransaction, V2TextQuotaPolicy};
mod accepted;
mod intent;
mod recovery;
use recovery::RetainWarmRecovery;

pub(super) struct PendingRetain {
    intent: Option<intent::PendingUnsignedIntent>,
    pub(super) unsigned: Option<super::append::FirstUnsignedAppendData>,
    pub(super) unsigned_custody: Option<PreparedV2UnsignedOriginalCustody>,
    accepted: Option<accepted::PendingAcceptedLookup>,
    transaction: Option<PreparedV2TextRetainTransaction>,
    stage: Option<AdmittedV2TextRetainStage>,
    commit: Option<AdmittedV2TextRetainCommit>,
    recovery: Option<RetainWarmRecovery>,
    #[cfg(all(test, windows))]
    next_work_fault: Option<crate::store::persistence::v2_text::V2TextRetainTestCut>,
}

impl PendingRetain {
    pub(super) fn unsigned_custody_inputs(
        &mut self,
    ) -> Result<
        (
            &PreparedV2TextRetainTransaction,
            &mut Option<super::append::FirstUnsignedAppendData>,
            &mut Option<PreparedV2UnsignedOriginalCustody>,
        ),
        AppError,
    > {
        Ok((
            self.transaction.as_ref().ok_or_else(refused)?,
            &mut self.unsigned,
            &mut self.unsigned_custody,
        ))
    }
    pub(super) fn transaction_data(&self) -> Result<&PreparedV2TextRetainTransaction, AppError> {
        self.transaction.as_ref().ok_or_else(refused)
    }
    pub(super) fn unsigned_inputs(
        &mut self,
    ) -> Result<
        (
            &PreparedV2TextRetainTransaction,
            &mut Option<super::append::FirstUnsignedAppendData>,
        ),
        AppError,
    > {
        Ok((
            self.transaction.as_ref().ok_or_else(refused)?,
            &mut self.unsigned,
        ))
    }
    pub(super) fn new() -> Self {
        Self {
            intent: None,
            unsigned: None,
            unsigned_custody: None,
            accepted: None,
            transaction: None,
            stage: None,
            commit: None,
            recovery: None,
            #[cfg(all(test, windows))]
            next_work_fault: None,
        }
    }

    /// Already-admitted finite effects dispatch before any new Source/Scope borrow.
    /// A worker caches Ready/Saved before reporting a lost return. A retry must inspect
    /// those exact cached outcomes instead of repeating packet generation or quota grants.
    pub(super) fn drain_admitted(&mut self) -> Result<(), AppError> {
        if let Some(intent) = self.intent.as_mut() {
            intent.drain_admitted()?;
        }
        if let Some(accepted) = self.accepted.as_mut() {
            accepted.drain_admitted()?;
        }
        if let Some(stage) = self.stage.take() {
            self.transaction
                .as_mut()
                .ok_or_else(refused)?
                .run_stage(stage)?;
        }
        if let Some(commit) = self.commit.take() {
            self.transaction
                .as_mut()
                .ok_or_else(refused)?
                .run_commit(commit)?;
        }
        if let Some(transaction) = self.transaction.as_mut() {
            RetainWarmRecovery::drain_cached(&mut self.recovery, transaction)?;
        }
        Ok(())
    }

    pub(super) fn take_originals(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        policy: V2TextQuotaPolicy,
        predecessor: &BoundV2TextCreatePredecessor,
        request: &V2CreateRequest<'_>,
        custody: &mut Option<PreparedV2RetainOriginalCustody>,
        lookup: &mut PendingTokenLookup,
    ) -> Result<(), AppError> {
        if self.transaction.is_some() {
            if custody.is_some() {
                return Err(refused());
            }
            return self
                .transaction
                .as_ref()
                .ok_or_else(refused)?
                .require_request_data(request);
        }
        custody
            .as_ref()
            .ok_or_else(refused)?
            .producer()
            .require_request_data(request)?;
        let (selected, query) = lookup.retain_inputs()?;
        PreparedV2TextRetainTransaction::take_into(
            store,
            owner.source(),
            selected,
            predecessor,
            policy,
            custody,
            query,
            &mut self.transaction,
        )
    }

    pub(super) fn retry(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        request: &V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        let transaction = self.transaction.as_mut().ok_or_else(refused)?;
        transaction.require_request_data(request)?;
        let resources = transaction.resources().clone();
        if !transaction.has_saved_data() {
            transaction.prepare_candidates(store, owner.source(), rng)?;
            #[cfg(all(test, windows))]
            if let Some(cut) = self.next_work_fault.take() {
                transaction.set_work_fault_for_test(cut);
            }
            if !transaction.has_ready_data() {
                RetainWarmRecovery::before_stage(
                    &mut self.recovery,
                    store,
                    owner,
                    lane,
                    predecessor,
                    transaction,
                )?;
            }
            if !transaction.has_ready_data() {
                let mut physical = Some(PreparedV2TextPhysicalLease::prepare(
                    store,
                    owner.source(),
                    &resources,
                )?);
                let admission = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_retain_stage_into(
                        lane,
                        predecessor,
                        transaction,
                        &mut physical,
                        &mut self.stage,
                    )
                });
                // Destruction/refunds and actual Files/OS cleanup happen OUTSIDE host
                // writers even when the callback's final Scope postcheck rejects use.
                drop(physical.take());
                if let Some(stage) = self.stage.take() {
                    transaction.run_stage(stage)?;
                }
                admission?;
            }
            let ready = transaction.ready_selection()?;
            let mut physical = Some(PreparedV2TextPhysicalLease::prepare(
                store,
                owner.source(),
                &resources,
            )?);
            let admission = store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_retain_commit_into(
                    lane,
                    predecessor,
                    ready,
                    &mut physical,
                    &mut self.commit,
                )
            });
            drop(physical.take());
            if let Some(commit) = self.commit.take() {
                transaction.run_commit(commit)?;
            }
            admission?;
        }
        let saved = transaction.saved_retained_action()?;
        store.with_v2_text_current(owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            saved.validate_selected_under_current_guard(&mut guard)
        })
    }
}

impl PendingCreate {
    pub(in super::super) fn drain_retained(&mut self) -> Result<(), AppError> {
        self.drain_originals()?;
        if let Some(retained) = self.retained.as_mut() {
            retained.drain_admitted()?;
        }
        Ok(())
    }

    fn retry_retained(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lookup: &mut PendingTokenLookup,
        policy: V2TextQuotaPolicy,
        request: &V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.drain_retained()?;
        let predecessor = self.predecessor.as_ref().ok_or_else(refused)?;
        let lane = self.lane.as_ref().ok_or_else(refused)?;
        if self.retained.is_none() {
            // The SDK+packet owning handoff must already be complete. Missing originals
            // cannot be replaced with a decoded WAL or a new signer invocation here.
            let custody = self.custody.as_ref().ok_or_else(refused)?;
            custody.producer().require_request_data(request)?;
            self.retained = Some(PendingRetain::new());
        }
        let retained = self.retained.as_mut().ok_or_else(refused)?;
        retained.take_originals(
            store,
            owner,
            policy,
            predecessor,
            request,
            &mut self.custody,
            lookup,
        )?;
        retained.retry(store, owner, lane, predecessor, request, rng)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Cover only the original retained A. Returning Ok does not accept, render or publish
    /// the message: durable I, raw signature S, accepted B and verifier advance follow.
    pub(in crate::store) fn retry_retain_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if let Some(pending) = self.request_preparation.as_mut() {
            pending.drain_retained()?;
        }
        if self
            .request_preparation
            .as_ref()
            .is_none_or(|pending| pending.retained.is_none())
        {
            self.retry_prepare_create_request(
                layout,
                V2CreateRequest {
                    token: request.token,
                    channel: request.channel,
                    text: request.text,
                },
                rng,
            )?;
        }
        // Once the A cell owns Query, do not recreate/revalidate the old lookup against
        // an advanced selector. Its complete logical reader remains in that SAME cell.
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        self.request_preparation
            .as_mut()
            .ok_or_else(refused)?
            .retry_retained(
                store,
                &owner,
                self.lookup.as_mut().ok_or_else(refused)?,
                self.policy,
                &request,
                rng,
            )
    }
}
