//! Owning S dispatcher. Declared with actual S worker/admission consumers. A cached
//! signature is not accepted history, a completed Header, projection or publication.
use super::*;
mod accepted;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
};
use crate::store::persistence::v2_text::PreparedV2TextRawSignature;
use accepted::PendingAcceptedCreate;

pub(super) struct PendingRawSignature {
    stage: Option<AdmittedV2TextRawSignatureStage>,
    commit: Option<AdmittedV2TextRawSignatureCommit>,
    accepted: Option<PendingAcceptedCreate>,
    // LAST: whole I includes the SAME actual verifier, SDK and all real payments.
    transaction: Option<PreparedV2TextRawSignature>,
}
impl PendingRawSignature {
    fn new() -> Self {
        Self {
            stage: None,
            commit: None,
            accepted: None,
            transaction: None,
        }
    }
    pub(super) fn drain_admitted(&mut self) -> Result<(), AppError> {
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
        Ok(())
    }
    fn retry(
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
                let admission = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_raw_signature_stage_into(
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
            let admission = store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_raw_signature_commit_into(
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
        let saved = transaction.saved_raw_signature()?;
        store.with_v2_text_current(owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            saved.validate_selected_under_current_guard(&guard)
        })
    }
}
impl PendingUnsignedIntent {
    fn retry_raw_signature(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        request: &V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        if let Some(existing) = self.raw.as_ref().and_then(|p| p.transaction.as_ref()) {
            existing.require_request_data(request)?;
        } else {
            self.transaction
                .as_ref()
                .ok_or_else(refused)?
                .require_request_data(request)?;
        }
        if self.raw.is_none() {
            self.raw = Some(PendingRawSignature::new());
        }
        let pending = self.raw.as_mut().ok_or_else(refused)?;
        PreparedV2TextRawSignature::take_into(
            store,
            owner.source(),
            &mut self.transaction,
            &mut pending.transaction,
        )?;
        pending.retry(store, owner, lane, predecessor, request, rng)
    }
}
impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Select only durable S for the original key result. Once S owns I, every retry
    /// dispatches that original S phase before any old I validator or signer path.
    pub(in crate::store) fn retry_raw_signature_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if let Some(pending) = self.request_preparation.as_mut() {
            pending.drain_retained()?;
        }
        let has_s = self
            .request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .and_then(|p| p.intent.as_ref())
            .is_some_and(|p| p.raw.is_some());
        if !has_s {
            self.retry_sign_original_create_data(
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
        let owner = self.owner.borrow_current()?;
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        pending
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .intent
            .as_mut()
            .ok_or_else(refused)?
            .retry_raw_signature(
                store,
                &owner,
                pending.lane.as_ref().ok_or_else(refused)?,
                pending.predecessor.as_ref().ok_or_else(refused)?,
                &request,
                rng,
            )
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn raw_signature_saved_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .and_then(|p| p.intent.as_ref())
            .and_then(|p| p.raw.as_ref())
            .and_then(|p| p.transaction.as_ref())
            .is_some_and(PreparedV2TextRawSignature::has_saved_data)
    }
}
