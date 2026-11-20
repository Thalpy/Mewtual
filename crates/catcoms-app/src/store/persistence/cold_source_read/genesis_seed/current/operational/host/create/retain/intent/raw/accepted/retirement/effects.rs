//! Owning dispatch of the two distinct physical retirement runs.
//!
//! A consumed deletion permit may finish after cancellation, but cannot authorize
//! completion. WC always needs a new Current admission after deletion's real Files,
//! OS lease and writer End have drained. Keep admitted work before inspecting any
//! live source so a lost reply never loses a durable WD or repeats a fresh plan.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
};

#[derive(Default)]
pub(super) struct PendingRetirementEffects {
    deletion: Option<AdmittedV2TextAcceptedRetirementDeletion>,
    completion: Option<AdmittedV2TextAcceptedRetirementCompletion>,
}

impl PendingRetirementEffects {
    /// Dispatch only already-consumed finite work. No Source/Current, new lease,
    /// nonce or completion role is obtained by this cancellation-safe cleanup.
    pub(super) fn drain_admitted(
        &mut self,
        transaction: &mut PreparedV2TextAcceptedCreate,
    ) -> Result<(), AppError> {
        if let Some(deletion) = self.deletion.take() {
            transaction.run_retirement_deletion(deletion)?;
        }
        if let Some(completion) = self.completion.take() {
            transaction.run_retirement_completion(completion)?;
        }
        Ok(())
    }

    fn retry<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        transaction: &mut PreparedV2TextAcceptedCreate,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &V2TextCreateLane,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        self.drain_admitted(transaction)?;
        let basis = transaction.accepted_final_basis_data()?.clone();
        let resources = transaction.resources().clone();
        if !transaction.has_ready_retirement_completion_data()
            && !transaction.has_saved_retirement_completion_data()
        {
            let current_owner = owner.borrow_current()?;
            transaction.prepare_retirement_data(store, current_owner.source())?;
            let mut physical = Some(PreparedV2TextPhysicalLease::prepare(
                store,
                current_owner.source(),
                &resources,
            )?);
            let admission = store.with_v2_text_current(&current_owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_retirement_deletion_into(
                    lane,
                    transaction,
                    &basis,
                    drained,
                    &mut physical,
                    &mut self.deletion,
                )
            });
            // Unadmitted lease destruction and admitted finite work both run after
            // every writer releases, including when admission lost its final reply.
            drop(physical.take());
            self.drain_admitted(transaction)?;
            admission?;
        }
        if !transaction.has_saved_retirement_completion_data() {
            // The previous drain returned only after the real deletion marker's
            // physical lease and End dropped. It still grants no WC permission.
            let current_owner = owner.borrow_current()?;
            let ready = transaction.ready_retirement_completion()?;
            let mut physical = Some(PreparedV2TextPhysicalLease::prepare(
                store,
                current_owner.source(),
                &resources,
            )?);
            let admission = store.with_v2_text_current(&current_owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_retirement_completion_into(
                    lane,
                    ready,
                    &basis,
                    drained,
                    &mut physical,
                    &mut self.completion,
                )
            });
            drop(physical.take());
            self.drain_admitted(transaction)?;
            admission?;
        }
        // Cached actual WC is not current authorization. A fresh selected WC/new-B
        // proof is required on each successful owning retry; final lane retirement
        // and resident transfer still need their own distinct issuer.
        let current_owner = owner.borrow_current()?;
        store.with_v2_text_current(&current_owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            transaction.validate_retirement_completion_under_current_guard(&guard, drained)
        })
    }
}

impl PendingAcceptedCreate {
    fn retry_retirement_effects<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &V2TextCreateLane,
        request: &V2CreateRequest<'_>,
    ) -> Result<bool, AppError> {
        self.drain_admitted()?;
        let transaction = self.transaction.as_mut().ok_or_else(refused)?;
        transaction.require_request_data(request)?;
        if !self.retirement.retry(transaction, store, owner, lane)? {
            return Ok(false);
        }
        self.retirement.effects.retry(
            transaction,
            store,
            owner,
            lane,
            self.retirement.drained.as_ref().ok_or_else(refused)?,
        )?;
        Ok(true)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Finish the original private-file retirement after first-B finalization.
    /// Pending external readers return false. Success includes actual saved WC and
    /// its new Current proof, but does not release the lane or publish a message.
    pub(in crate::store) fn retry_retire_accepted_create_files(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<bool, AppError> {
        if !self.retry_close_accepted_create_roots(
            layout,
            V2CreateRequest {
                token: request.token,
                channel: request.channel,
                text: request.text,
            },
            rng,
        )? {
            return Ok(false);
        }
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        pending
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .intent
            .as_mut()
            .ok_or_else(refused)?
            .raw
            .as_mut()
            .ok_or_else(refused)?
            .accepted
            .as_mut()
            .ok_or_else(refused)?
            .retry_retirement_effects(
                layout.checked_store()?,
                &self.owner,
                pending.lane.as_ref().ok_or_else(refused)?,
                &request,
            )
    }
}
