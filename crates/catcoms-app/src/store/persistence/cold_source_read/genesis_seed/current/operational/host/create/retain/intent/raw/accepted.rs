//! Whole owning B dispatcher and same-verifier finalization. B visibility does not
//! close roots, delete files, publish, retire a lane or finish the request lifecycle.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
};
use crate::store::persistence::v2_text::PreparedV2TextAcceptedCreate;
mod retirement;

pub(super) struct PendingAcceptedCreate {
    stage: Option<AdmittedV2TextAcceptedCreateStage>,
    commit: Option<AdmittedV2TextAcceptedCreateCommit>,
    retirement: retirement::PendingAcceptedRetirement,
    disposal: Option<Box<PreparedV2TextAcceptedCreate>>,
    // LAST: constructor acquires NEW B2MiB before this actual Box/all its payloads.
    transaction: Option<Box<PreparedV2TextAcceptedCreate>>,
}
impl PendingAcceptedCreate {
    fn new() -> Self {
        Self {
            stage: None,
            commit: None,
            retirement: retirement::PendingAcceptedRetirement::default(),
            disposal: None,
            transaction: None,
        }
    }
    pub(super) fn drain_admitted(&mut self) -> Result<(), AppError> {
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
        // Negative cleanup precedes ANY fresh Source attempt, including a late
        // post-Core cancellation. Before advance this is a proven no-work operation.
        if let Some(transaction) = self.transaction.as_mut() {
            transaction.drain_accepted_retired_math_if_present()?;
            self.retirement.drain_outside_writers(transaction)?;
        }
        Ok(())
    }
    fn retry_selection(
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
            if !transaction.has_ready_data() {
                let mut physical = Some(PreparedV2TextPhysicalLease::prepare(
                    store,
                    owner.source(),
                    &resources,
                )?);
                let admission = store.with_v2_text_current(owner, |a| {
                    let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                    guard.admit_text_accepted_create_stage_into(
                        lane,
                        predecessor,
                        transaction,
                        &mut physical,
                        &mut self.stage,
                    )
                });
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
            let admission = store.with_v2_text_current(owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.admit_text_accepted_create_commit_into(
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
        let saved = transaction.saved_accepted_create()?;
        store.with_v2_text_current(owner, |a| {
            let guard = store.lock_v2_text_current_group(a, &resources)?;
            saved.validate_selected_under_current_guard(&guard)
        })
    }
    fn retry_verifier_finalization<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &V2TextCreateLane,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        let transaction = self.transaction.as_mut().ok_or_else(refused)?;
        // Outside Arc clones avoid any immutable PreparedB loan across the mutable
        // Root cut. Neither Current guard stores a PreparedB lifetime or model loan.
        let saved = Arc::clone(transaction.saved_accepted_create()?);
        let resources = transaction.resources().clone();
        let advance_result = {
            let current_owner = owner.borrow_current()?;
            store.with_v2_text_current(&current_owner, |a| {
                let mut current = store.lock_v2_text_current_group(a, &resources)?;
                let advance = current.admit_accepted_advance(lane, &saved, transaction)?;
                transaction.advance_accepted_under_guard(&advance)
            })
        };
        // Old math is genuinely destroyed OUTSIDE every AD/canonical/Root writer.
        // The next distinct issuer proves actual advanced+retiredNone BEFORE taking
        // a NEW complete selected-B proof; the earlier advance guard cannot substitute.
        transaction.drain_accepted_retired_math_if_present()?;
        advance_result?;
        let current_owner = owner.borrow_current()?;
        store.with_v2_text_current(&current_owner, |a| {
            let mut current = store.lock_v2_text_current_group(a, &resources)?;
            let final_guard = current.admit_accepted_final(lane, &saved, transaction)?;
            transaction.confirm_accepted_final_under_guard(&final_guard)
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Select the exact immutable accepted B while retaining its original verifier,
    /// candidate, SDK, old query pins and lane. Retirement/projection remain separate.
    pub(in crate::store) fn retry_accepted_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if let Some(pending) = self.request_preparation.as_mut() {
            pending.drain_retained()?;
        }
        let has_b = self
            .request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .and_then(|p| p.intent.as_ref())
            .and_then(|p| p.raw.as_ref())
            .and_then(|p| p.accepted.as_ref())
            .is_some_and(|p| p.transaction.is_some());
        if !has_b {
            self.retry_raw_signature_create_request(
                layout,
                V2CreateRequest {
                    token: request.token,
                    channel: request.channel,
                    text: request.text,
                },
                rng,
            )?;
        }
        let store = layout.checked_store()?;
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        let raw = pending
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .intent
            .as_mut()
            .ok_or_else(refused)?
            .raw
            .as_mut()
            .ok_or_else(refused)?;
        if !has_b {
            // The same whole S produces/caches its actual SDK Header and Core
            // candidate OUTSIDE writers. Never keep a node-backed Source loan here.
            let signature = raw.transaction.as_mut().ok_or_else(refused)?;
            signature.require_request_data(&request)?;
            signature.materialize_candidate_after_saved_s(store, &self.owner)?;
        }
        if raw.accepted.is_none() {
            raw.accepted = Some(PendingAcceptedCreate::new());
        }
        let accepted = raw.accepted.as_mut().ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        PreparedV2TextAcceptedCreate::take_into(
            store,
            owner.source(),
            &mut raw.transaction,
            &mut accepted.transaction,
        )?;
        accepted.retry_selection(
            store,
            &owner,
            pending.lane.as_ref().ok_or_else(refused)?,
            pending.predecessor.as_ref().ok_or_else(refused)?,
            &request,
            rng,
        )
    }

    /// Advance the SAME verifier once, drain old math, then confirm with a distinct
    /// fresh-B guard. This does not yet provide WC, lane release or native publication.
    pub(in crate::store) fn retry_finalize_accepted_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_accepted_create_request(layout, request, rng)?;
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
            .retry_verifier_finalization(
                layout.checked_store()?,
                &self.owner,
                pending.lane.as_ref().ok_or_else(refused)?,
            )
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn accepted_create_saved_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .and_then(|p| p.intent.as_ref())
            .and_then(|p| p.raw.as_ref())
            .and_then(|p| p.accepted.as_ref())
            .and_then(|p| p.transaction.as_ref())
            .is_some_and(|p| p.has_saved_data())
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn accepted_create_final_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .and_then(|p| p.intent.as_ref())
            .and_then(|p| p.raw.as_ref())
            .and_then(|p| p.accepted.as_ref())
            .and_then(|p| p.transaction.as_ref())
            .is_some_and(|p| p.accepted_final_basis_data().is_ok())
    }
}
