//! Owning, nonblocking old-root retirement after actual accepted finalization.
//!
//! Closing a root forbids new readers; it does not prove the old readers ended.
//! Keep every real close/drain owner across a lost reply, release our own logical
//! readers outside writers, and poll the actual remaining Ends on later turns.
//! This boundary never deletes a file, releases the lane or publishes a message.
use super::*;
mod effects;
mod finish;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextRetirementRootsClose, RemovedV2TextRetirementRootGenerations,
    RetiredV2TextCreateLane, V2TextRetirementRootsClosed, V2TextRetirementRootsDrained,
};

/// Inline in the original paid PendingCreate phase. The separate B invoice funds
/// the concrete prepared/closed/drained payloads and their two bounded arrays;
/// no additional heap allocation or uncharged root snapshot is made by this wrapper.
#[derive(Default)]
pub(super) struct PendingAcceptedRetirement {
    effects: effects::PendingRetirementEffects,
    retired: Option<RetiredV2TextCreateLane>,
    prepared: Option<PreparedV2TextRetirementRootsClose>,
    closed: Option<V2TextRetirementRootsClosed>,
    drained: Option<V2TextRetirementRootsDrained>,
    removed: Option<RemovedV2TextRetirementRootGenerations>,
    // Orchestration DATA only, cached while the actual outside Removed owner exists.
    // Every later WD/WC issuer must still validate the opaque Drained owner itself.
    detached: bool,
    #[cfg(all(test, windows))]
    after_close: Option<Box<dyn FnOnce() + Send>>,
}

impl PendingAcceptedRetirement {
    /// Negative cleanup is legal after ScopeStop. Never reborrow live Current or
    /// revalidate the now-closing original queries to release their actual Ends.
    pub(super) fn drain_outside_writers(
        &mut self,
        transaction: &mut PreparedV2TextAcceptedCreate,
    ) -> Result<(), AppError> {
        self.effects.drain_admitted(transaction)?;
        if self.closed.is_some() || self.drained.is_some() {
            transaction.drain_original_query_readers_outside_writers()?;
        }
        if self.closed.is_some() {
            V2TextRetirementRootsClosed::poll_drained_into(&mut self.closed, &mut self.drained)?;
        }
        if self.removed.is_some() {
            // Cache the actual completed registry move before dropping its owning
            // generations. A late Current error must not forget that completed cut.
            self.detached = true;
            drop(self.removed.take());
        }
        Ok(())
    }

    fn retry<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        transaction: &mut PreparedV2TextAcceptedCreate,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &V2TextCreateLane,
    ) -> Result<bool, AppError> {
        self.drain_outside_writers(transaction)?;
        if self.detached {
            // Historical completion only. No new effect is admitted by returning it.
            return Ok(true);
        }
        let basis = transaction.accepted_final_basis_data()?.clone();
        let resources = transaction.resources().clone();
        if self.closed.is_none() && self.drained.is_none() {
            let current_owner = owner.borrow_current()?;
            if self.prepared.is_none() {
                self.prepared = Some(PreparedV2TextRetirementRootsClose::prepare(
                    store,
                    current_owner.source(),
                    transaction,
                    &basis,
                    lane,
                )?);
            }
            #[cfg(all(test, windows))]
            if let Some(run) = self.after_close.take() {
                self.prepared
                    .as_mut()
                    .expect("same actual prepared root-close owner")
                    .set_after_close_for_test(run);
            }
            let result = store.with_v2_text_current(&current_owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.close_text_retirement_roots_into(
                    lane,
                    transaction,
                    &basis,
                    &mut self.prepared,
                    &mut self.closed,
                )
            });
            // Close may have succeeded before its reply failed. Real owner slots,
            // rather than Result::Ok, determine which negative cleanup is now safe.
            self.drain_outside_writers(transaction)?;
            result?;
        }
        let Some(drained) = self.drained.as_mut() else {
            // Another real reader still owns an End. Ordinary retry is quiet and
            // nonblocking; elapsed time and empty registry slots cannot force it.
            return Ok(false);
        };
        let current_owner = owner.borrow_current()?;
        let result = store.with_v2_text_current(&current_owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.detach_text_retirement_roots_into(
                lane,
                transaction,
                &basis,
                drained,
                &mut self.removed,
            )
        });
        self.drain_outside_writers(transaction)?;
        result?;
        Ok(self.detached)
    }
}

impl PendingAcceptedCreate {
    fn retry_retirement_roots<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &V2TextCreateLane,
        request: &V2CreateRequest<'_>,
    ) -> Result<bool, AppError> {
        self.drain_admitted()?;
        let transaction = self.transaction.as_mut().ok_or_else(refused)?;
        // Releasing old readers is safe even if the caller mistakenly supplies a
        // different request; request equality still gates the requested operation.
        self.retirement.drain_outside_writers(transaction)?;
        transaction.require_request_data(request)?;
        self.retirement.retry(transaction, store, owner, lane)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Close all four original roots and detach their metadata once actual readers
    /// drain. False is ordinary pending work. True records this cleanup boundary
    /// only: WD/WC, lane retirement, resident reuse and native publication are separate.
    pub(in crate::store) fn retry_close_accepted_create_roots(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<bool, AppError> {
        let finalized = self
            .request_preparation
            .as_ref()
            .and_then(|pending| pending.retained.as_ref())
            .and_then(|pending| pending.intent.as_ref())
            .and_then(|pending| pending.raw.as_ref())
            .and_then(|pending| pending.accepted.as_ref())
            .and_then(|pending| pending.transaction.as_ref())
            .is_some_and(|transaction| transaction.accepted_final_basis_data().is_ok());
        if !finalized {
            self.retry_finalize_accepted_create_request(
                layout,
                V2CreateRequest {
                    token: request.token,
                    channel: request.channel,
                    text: request.text,
                },
                rng,
            )?;
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
            .retry_retirement_roots(
                layout.checked_store()?,
                &self.owner,
                pending.lane.as_ref().ok_or_else(refused)?,
                &request,
            )
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn accepted_roots_detached_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|pending| pending.retained.as_ref())
            .and_then(|pending| pending.intent.as_ref())
            .and_then(|pending| pending.raw.as_ref())
            .and_then(|pending| pending.accepted.as_ref())
            .is_some_and(|pending| pending.retirement.detached)
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_accepted_roots_close_for_test(
        &mut self,
        run: Box<dyn FnOnce() + Send>,
    ) {
        self.request_preparation
            .as_mut()
            .and_then(|pending| pending.retained.as_mut())
            .and_then(|pending| pending.intent.as_mut())
            .and_then(|pending| pending.raw.as_mut())
            .and_then(|pending| pending.accepted.as_mut())
            .expect("genuine accepted owner before configuring its next close")
            .retirement
            .after_close = Some(run);
    }
}
