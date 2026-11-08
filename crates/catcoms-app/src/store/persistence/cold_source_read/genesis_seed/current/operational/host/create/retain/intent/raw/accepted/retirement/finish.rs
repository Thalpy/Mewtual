//! Finish the first request without keeping its transaction graph as history.
//!
//! A fresh WC proof retires the unique lane. A distinct fresh finish guard then
//! moves the SAME advanced verifier into a flat resident and the whole remaining
//! B into outside disposal. Lost replies are recovered from those actual owning
//! outputs before touching any obsolete positive B/SDK/query path.
use super::*;
use crate::store::persistence::PreparedV2TextResidentProjection;

impl PendingAcceptedCreate {
    fn retry_completed_resident_into<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &mut Option<V2TextCreateLane>,
        resident: &mut Option<PreparedV2TextResidentProjection>,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        if self.transaction.is_none() {
            // After transfer the old B is inert. No fresh positive finish guard
            // can be issued from it, even if the original callback lost its reply.
            return PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
                &self.transaction,
                resident.as_ref().ok_or_else(refused)?,
                &self.disposal,
            );
        }
        let transaction = self.transaction.as_mut().ok_or_else(refused)?;
        transaction.prepare_completed_resident_data()?;
        let basis = transaction.accepted_final_basis_data()?.clone();
        let saved = Arc::clone(transaction.saved_retirement_completion()?);
        let resources = transaction.resources().clone();
        if self.retirement.retired.is_none() {
            let current_owner = owner.borrow_current()?;
            store.with_v2_text_current(&current_owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.retire_completed_text_lane_into(
                    transaction,
                    &basis,
                    lane,
                    &mut self.retirement.drained,
                    &mut self.retirement.retired,
                )
            })?;
        }
        // The previous guard is gone. This distinct admission rechecks actual WC,
        // removed-lane identity and the complete same-model source association.
        let current_owner = owner.borrow_current()?;
        let result = store.with_v2_text_current(&current_owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            let finish = guard.admit_completed_finish(
                self.retirement.retired.as_ref().ok_or_else(refused)?,
                &saved,
                self.transaction.as_ref().ok_or_else(refused)?,
            )?;
            PreparedV2TextAcceptedCreate::finish_resident_into(
                &finish,
                &mut self.transaction,
                resident,
                &mut self.disposal,
            )
        });
        // This outside SavedWC alias itself retains the old B metadata. End it
        // before the caller destroys the whole old request graph and checks fees.
        drop(saved);
        result
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn resident_without_old_request_for_test(&self) -> bool {
        self.resident.is_some()
            && self.request_preparation.is_none()
            && self.lookup.is_none()
            && self.selected.is_none()
            && self.prepared.is_none()
            && self.recovery.is_none()
            && self.capture.is_none()
            && self.retired.is_none()
    }

    /// Verify the coupled original outputs before destroying their old owners.
    /// No Source upgrade, new Current, SDK operation or physical IO is needed to
    /// recover a transfer that completed before a cancelled/lost callback reply.
    fn drain_finished_request_outside_writers(&mut self) -> Result<bool, AppError> {
        let Some(resident) = self.resident.as_ref() else {
            return Ok(false);
        };
        if let Some(request) = self.request_preparation.as_ref() {
            let accepted = request
                .retained
                .as_ref()
                .and_then(|pending| pending.intent.as_ref())
                .and_then(|pending| pending.raw.as_ref())
                .and_then(|pending| pending.accepted.as_ref())
                .ok_or_else(refused)?;
            PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
                &accepted.transaction,
                resident,
                &accepted.disposal,
            )?;
        }
        // Finish any original admitted finite cleanup before classifying the old
        // promotion state. Never silently discard an unexpected live recovery debt.
        self.drain_admitted_promotion_work()?;
        self.drain_cached_retirement();
        if self.capture.is_some()
            || self.prepared.is_some()
            || self.recovery.is_some()
            || self.lane_preparation.is_some()
            || self.lane.is_some()
            || self.admitted_stage.is_some()
            || self.admitted_commit.is_some()
            || self.closing.is_some()
            || self.drained.is_some()
            || self.retired.is_some()
        {
            return Err(refused());
        }
        if let Some(lookup) = self.lookup.as_mut() {
            lookup.drain_admitted()?;
        }
        // RetiredLane, predecessor/basis, old query/read observations, the SDK and
        // WHOLE old B drain outside every writer. The resident owns none of them.
        drop(self.request_preparation.take());
        drop(self.lookup.take());
        drop(self.selected.take());
        drop(self.observer_preparation.take());
        Ok(true)
    }

    /// Complete the first original request and retain its SAME advanced verifier.
    /// False means real external readers still need to drain. Success is local
    /// completion; native publication and general next-request COW remain separate.
    pub(in crate::store) fn retry_finish_accepted_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<bool, AppError> {
        // This path MUST precede the old positive first-request dispatch. A
        // transferred old B has no model/candidate and cannot admit another guard.
        if self.drain_finished_request_outside_writers()? {
            let resident = self.resident.as_ref().ok_or_else(refused)?;
            resident.require_completed_request_data(&request)?;
            resident.validate_negative(layout.checked_store()?)?;
            return Ok(true);
        }
        let retired = self
            .request_preparation
            .as_ref()
            .and_then(|pending| pending.retained.as_ref())
            .and_then(|pending| pending.intent.as_ref())
            .and_then(|pending| pending.raw.as_ref())
            .and_then(|pending| pending.accepted.as_ref())
            .is_some_and(|pending| pending.retirement.retired.is_some());
        if !retired
            && !self.retry_retire_accepted_create_files(
                layout,
                V2CreateRequest {
                    token: request.token,
                    channel: request.channel,
                    text: request.text,
                },
                rng,
            )?
        {
            return Ok(false);
        }
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        let accepted = pending
            .retained
            .as_mut()
            .and_then(|pending| pending.intent.as_mut())
            .and_then(|pending| pending.raw.as_mut())
            .and_then(|pending| pending.accepted.as_mut())
            .ok_or_else(refused)?;
        accepted
            .transaction
            .as_ref()
            .ok_or_else(refused)?
            .require_request_data(&request)?;
        let result = accepted.retry_completed_resident_into(
            layout.checked_store()?,
            &self.owner,
            &mut pending.lane,
            &mut self.resident,
        );
        // A post-transfer error must still release all actual old owners. Preserve
        // the resident output before reporting the original failure to the caller.
        let completed = self.drain_finished_request_outside_writers()?;
        result?;
        if !completed {
            return Err(refused());
        }
        let resident = self.resident.as_ref().ok_or_else(refused)?;
        resident.require_completed_request_data(&request)?;
        resident.validate_negative(layout.checked_store()?)?;
        Ok(true)
    }
}
