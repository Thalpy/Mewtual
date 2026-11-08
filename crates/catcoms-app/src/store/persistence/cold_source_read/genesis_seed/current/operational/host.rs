//! Owning first-promotion driver. Native posting and ordinary transactions are separate.
//!
//! Every admitted finite effect is dispatched before trying to borrow live Current again.
//! Late Scope errors cannot discard an already admitted write, release its lease early or
//! cause another candidate to be generated. All callbacks borrow caller-owned cache slots.
use super::negative::{PreparedV2TextSourceObserver, V2TextSourceNegativeObserver};
use super::owner::V2TextOwnedCurrent;
use super::PreparedV2TextRead;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextPromotionCommit, AdmittedV2TextPromotionStage, PreparedV2TextPhysicalLease,
    V2CurrentLayoutStore,
};
use crate::store::critical_namespaces::refused;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextLane, RetiredV2TextPromotionLane, V2TextLane, V2TextPromotionLaneDrain,
    V2TextPromotionLaneDrained,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextPromotion, PreparedV2TextPromotionCapture, SavedV2TextPromotion,
    V2TextQuotaPolicy,
};
use crate::AppError;
use std::sync::Arc;

mod recovery;
use recovery::PromotionRecovery;
mod lookup;
use lookup::PendingTokenLookup;
mod create;
mod projection;
pub(in crate::store) use create::{
    OriginalV2TextResidentObservation, OriginalV2UnsignedPlanObservation,
    PreparedV2NextUnsignedAppend, PreparedV2TextResidentProjection,
    PreparedV2UnsignedOriginalCustody, V2AdvancedAppendBasis, V2TextResidentParentData,
};
use create::{PendingCompletedCreate, PendingCreate};

/// Trusted local immutable-history limits, separate from memory reservations and fixed
/// control-file capacity. This is no promise that the filesystem has that space free.
#[derive(Clone, Copy, Debug)]
pub struct V2TextHostPolicy {
    quota: V2TextQuotaPolicy,
}

impl V2TextHostPolicy {
    /// Configure before constructing an owning host. Exact durable usage and outstanding
    /// reservations must survive restart; constructing this value cannot reset them.
    pub fn new(immutable_byte_limit: u128, immutable_object_limit: u128) -> Result<Self, AppError> {
        Ok(Self {
            quota: V2TextQuotaPolicy::new(immutable_byte_limit, immutable_object_limit)?,
        })
    }
}

/// Actual original owner and all same-candidate first-promotion debt. No raw SDK/node or
/// adopting constructor is exposed. This still cannot post or issue a projection token.
pub struct PreparedV2TextHost<T, R> {
    projection: Option<Arc<projection::V2TextProjectionDomain>>,
    resident: Option<PreparedV2TextResidentProjection>,
    capture: Option<PreparedV2TextPromotionCapture>,
    prepared: Option<PreparedV2TextPromotion>,
    observer_preparation: Option<PreparedV2TextSourceObserver>,
    observer: Option<V2TextSourceNegativeObserver>,
    lane_preparation: Option<PreparedV2TextLane>,
    lane: Option<V2TextLane>,
    admitted_stage: Option<AdmittedV2TextPromotionStage>,
    admitted_commit: Option<AdmittedV2TextPromotionCommit>,
    recovery: Option<PromotionRecovery>,
    lookup: Option<PendingTokenLookup>,
    request_preparation: Option<PendingCreate>,
    next_request_preparation: Option<Box<PendingCompletedCreate>>,
    closing: Option<V2TextPromotionLaneDrain>,
    drained: Option<V2TextPromotionLaneDrained>,
    retired: Option<RetiredV2TextPromotionLane>,
    selected: Option<Arc<SavedV2TextPromotion>>,
    policy: V2TextQuotaPolicy,
    #[cfg(all(test, windows))]
    next_work_fault: Option<crate::store::persistence::v2_text::V2TextPromotionTestCut>,
    // Whole original wrapper/payment remains intact. Drop below cancels before ANY debt
    // fields drain; destruction is not an acknowledgment that a running worker has ended.
    owner: V2TextOwnedCurrent<T, R>,
}

impl<T, R> PreparedV2TextHost<T, R> {
    /// Moving a read-only owner creates no allocation or permission. The trusted policy
    /// is fixed for this host; all actual captures and work acquire their own fees first.
    pub fn from_read(read: PreparedV2TextRead<T, R>, policy: V2TextHostPolicy) -> Self {
        Self {
            projection: None,
            resident: None,
            capture: None,
            prepared: None,
            observer_preparation: None,
            observer: None,
            lane_preparation: None,
            lane: None,
            admitted_stage: None,
            admitted_commit: None,
            recovery: None,
            lookup: None,
            request_preparation: None,
            next_request_preparation: None,
            closing: None,
            drained: None,
            retired: None,
            selected: None,
            policy: policy.quota,
            #[cfg(all(test, windows))]
            next_work_fault: None,
            owner: read.owner,
        }
    }

    /// Finish the original family's existing cleanup before first operational capture.
    /// Once preparation starts, use its phase dispatcher; never consult the old decoder.
    pub fn retry_original_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.resident.is_some()
            || self.next_request_preparation.is_some()
            || self.capture.is_some()
            || self.prepared.is_some()
            || self.selected.is_some()
        {
            return Err(refused());
        }
        self.owner.retry_seed_settlement(layout, rng)
    }

    /// Complete ONLY the first empty operational selector. It does not accept a message,
    /// change MLS state, publish a packet or prove complete later-head crash recovery.
    /// The same actual work, frames, ownership and quota survive each fallible return.
    pub fn retry_empty_promotion(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        self.drain_admitted_promotion_work()?;
        // An outcome removed from the registry was cached before the late postcheck.
        // Its payload and refunds must drain OUTSIDE all host writers, even on retry.
        self.drain_cached_retirement();

        // Once the same verifier has moved into its completed resident, dropping
        // the old O1 receipt must never make genesis preparation available again.
        if self.resident.is_some() || self.next_request_preparation.is_some() {
            return Err(refused());
        }
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        if let Some(selected) = self.selected.as_ref() {
            let resources = selected.resources().clone();
            return store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                selected.validate_selected_under_current_guard(&guard)
            });
        }
        if self.capture.is_none() && self.prepared.is_none() {
            self.capture = Some(PreparedV2TextPromotionCapture::prepare(
                store,
                owner.source(),
            )?);
        }
        if self.prepared.is_none() {
            let capture = self.capture.as_mut().ok_or_else(refused)?;
            let resources = capture.resources().clone();
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                capture.capture_genesis_parent(&guard)
            })?;
            capture.take_promotion_into(self.policy, &mut self.prepared)?;
        }
        let prepared = self.prepared.as_mut().ok_or_else(refused)?;
        prepared.prepare_candidates(store, owner.source(), rng)?;
        #[cfg(all(test, windows))]
        if let Some(cut) = self.next_work_fault.take() {
            prepared.set_work_fault_for_test(cut);
        }

        if self.observer_preparation.is_none() {
            self.observer_preparation = Some(PreparedV2TextSourceObserver::prepare(store, &owner)?);
        }
        if self.observer.is_none() {
            let preparation = self.observer_preparation.as_ref().ok_or_else(refused)?;
            let slot = &mut self.observer;
            store.with_v2_text_current(&owner, |admission| {
                *slot = Some(preparation.capture(admission)?);
                Ok(())
            })?;
        }
        if self.lane_preparation.is_none() {
            self.lane_preparation = Some(PreparedV2TextLane::prepare(
                store,
                &owner,
                self.observer.as_ref().ok_or_else(refused)?,
            )?);
        }
        if self.lane.is_none() {
            let preparation = self.lane_preparation.as_mut().ok_or_else(refused)?;
            let slot = &mut self.lane;
            store.with_v2_text_current(&owner, |admission| {
                let mut namespace = store.lock_v2_text_owner_namespace(admission)?;
                namespace.claim_text_lane_into(preparation, slot)
            })?;
            preparation.drain_unused_outside_writers();
        }
        let lane = self.lane.as_ref().ok_or_else(refused)?;
        let resources = prepared.resources().clone();
        if prepared.ready_selection().is_err() {
            PromotionRecovery::before_stage(&mut self.recovery, store, &owner, lane, prepared)?;
        }
        if prepared.ready_selection().is_err() {
            let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
                store,
                owner.source(),
                &resources,
            )?);
            let slot = &mut self.admitted_stage;
            let admission = store.with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_stage_into(lane, prepared, &mut lease, slot)
            });
            // Cache Ready before returning a late admission error. The actual marker,
            // all temporary Files and the physical lease drain outside every writer.
            if let Some(admitted) = self.admitted_stage.take() {
                prepared.run_stage(admitted)?;
            }
            admission?;
        }
        if prepared.saved_promotion().is_err() {
            let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
                store,
                owner.source(),
                &resources,
            )?);
            let slot = &mut self.admitted_commit;
            let admission = store.with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_commit_into(
                    lane,
                    prepared.ready_selection()?,
                    &mut lease,
                    slot,
                )
            });
            if let Some(admitted) = self.admitted_commit.take() {
                prepared.run_commit(admitted)?;
            }
            admission?;
        }
        let saved = prepared.saved_promotion()?;
        if self.closing.is_none() && self.drained.is_none() {
            store.with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.close_text_promotion_lane_into(lane, saved, &mut self.closing)
            })?;
        }
        // No Scope, pins, canonical or Root guard is retained through this actual join.
        Self::join_closed_promotion(&mut self.closing, &mut self.drained)?;
        let drained = self.drained.as_ref().ok_or_else(refused)?;
        let result = store.with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.retire_text_promotion_lane_into(&mut self.lane, saved, drained, &mut self.retired)
        });
        self.drain_cached_retirement();
        result
    }

    /// Stop new original-owner use and finish this host's already admitted finite work.
    /// This does not retire or discard uncertain durable state, close a future message
    /// projection reader, or supply a receipt that later-head recovery has completed.
    pub fn cancel_and_drain_admitted_promotion(&mut self) -> Result<(), AppError> {
        self.owner.cancel_original();
        if let Some(domain) = &self.projection {
            domain.cancel();
        }
        if let Some(request) = self.request_preparation.as_mut() {
            request.drain_retained()?;
        }
        if let Some(next) = self.next_request_preparation.as_mut() {
            next.drain_originals()?;
        }
        if let Some(lookup) = self.lookup.as_mut() {
            lookup.drain_admitted()?;
        }
        self.drain_admitted_promotion_work()?;
        self.drain_cached_retirement();
        if self
            .projection
            .as_ref()
            .is_some_and(|domain| !domain.try_closed())
        {
            return Err(AppError::Invalid("Chat is still closing.".into()));
        }
        Ok(())
    }

    /// Authenticate and retain this token's SAME selected-root lookup. This supplies no
    /// new-Create, accepted-message or publication result: a complete workspace proof and
    /// actual A/I/S/B transaction are separate, still-required callers.
    pub(in crate::store) fn retry_token_lookup(
        &mut self,
        layout: &V2CurrentLayoutStore,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        if let Some(lookup) = self.lookup.as_mut() {
            lookup.drain_admitted()?;
        }
        if self.resident.is_some()
            || self.next_request_preparation.is_some()
            || self.selected.is_none()
        {
            return Err(refused());
        }
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        if self.lookup.is_none() {
            self.lookup = Some(PendingTokenLookup::prepare(store, &owner, token)?);
        }
        self.lookup
            .as_mut()
            .ok_or_else(refused)?
            .retry(store, &owner, token)
    }

    fn drain_admitted_promotion_work(&mut self) -> Result<(), AppError> {
        // The same admitted effect must finish before any new positive Source check,
        // including a retry whose owner was revoked by a late callback or user lock.
        if self.recovery.is_some() {
            let prepared = self.prepared.as_mut().ok_or_else(refused)?;
            PromotionRecovery::drain_cached(&mut self.recovery, prepared)?;
        }
        if self.admitted_stage.is_some() {
            let prepared = self.prepared.as_mut().ok_or_else(refused)?;
            let admitted = self
                .admitted_stage
                .take()
                .expect("checked original admission");
            prepared.run_stage(admitted)?;
        }
        if self.admitted_commit.is_some() {
            let prepared = self.prepared.as_mut().ok_or_else(refused)?;
            let admitted = self
                .admitted_commit
                .take()
                .expect("checked original admission");
            prepared.run_commit(admitted)?;
        }
        // Joining is negative lifecycle work. Source revocation cannot strand this
        // already-closed control behind a new Current borrow; no writers are held here.
        Self::join_closed_promotion(&mut self.closing, &mut self.drained)
    }

    fn join_closed_promotion(
        closing: &mut Option<V2TextPromotionLaneDrain>,
        drained: &mut Option<V2TextPromotionLaneDrained>,
    ) -> Result<(), AppError> {
        if let Some(original) = closing.take() {
            match original.join() {
                Ok(done) => *drained = Some(done),
                Err((original, error)) => {
                    *closing = Some(original);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Concrete registry retirement already succeeded; cache the SAME saved outcome
    /// before dropping active work payment. This method runs only outside all writers.
    /// Historical Saved retains its own small read loan after the 16 MiB work fee drains.
    fn drain_cached_retirement(&mut self) {
        if self.retired.is_none() {
            return;
        }
        self.selected = Some(
            self.prepared
                .as_ref()
                .expect("retirement follows actual promotion")
                .saved_promotion()
                .expect("retirement validated the same actual Saved")
                .clone(),
        );
        drop(self.retired.take());
        drop(self.drained.take());
        drop(self.prepared.take());
        drop(self.recovery.take());
        drop(self.capture.take());
        drop(self.lane_preparation.take());
    }

    /// Inject only a real finite-IO cut in the next original worker. Production has no
    /// caller-selected phase, admission bypass or error-to-absence substitution.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn fail_next_promotion_work_for_test(
        &mut self,
        cut: crate::store::persistence::v2_text::V2TextPromotionTestCut,
    ) {
        self.next_work_fault = Some(cut);
    }
}

impl<T, R> Drop for PreparedV2TextHost<T, R> {
    fn drop(&mut self) {
        self.owner.cancel_original();
        if let Some(domain) = &self.projection {
            domain.cancel();
        }
    }
}

impl<T, R> std::fmt::Debug for PreparedV2TextHost<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2TextHost").finish_non_exhaustive()
    }
}
