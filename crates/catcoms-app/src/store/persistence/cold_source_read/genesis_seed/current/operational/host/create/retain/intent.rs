//! Owning unsigned-I dispatcher. Paired with the concrete Physical I issuer and
//! Factory's distinct Stage/Commit admissions before declaration. This never signs.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextIntentCommit, AdmittedV2TextIntentStage,
};
use crate::store::persistence::v2_text::PreparedV2TextUnsignedIntent;
mod raw;
mod sign;

pub(super) struct PendingUnsignedIntent {
    raw: Option<raw::PendingRawSignature>,
    stage: Option<AdmittedV2TextIntentStage>,
    commit: Option<AdmittedV2TextIntentCommit>,
    // LAST: owns whole original A, SDK/model payments and actual accepted query/reader.
    transaction: Option<PreparedV2TextUnsignedIntent>,
}
impl PendingUnsignedIntent {
    fn new() -> Self {
        Self {
            raw: None,
            stage: None,
            commit: None,
            transaction: None,
        }
    }
    pub(super) fn drain_admitted(&mut self) -> Result<(), AppError> {
        if let Some(raw) = self.raw.as_mut() {
            raw.drain_admitted()?;
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
                    guard.admit_text_intent_stage_into(
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
                guard.admit_text_intent_commit_into(
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
        // A historical SavedI is not positive signing authority. Even this DATA-only
        // caller reproves the exact selected I closure through current ownership.
        let saved = transaction.saved_unsigned_intent()?;
        store.with_v2_text_current(owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            saved.validate_selected_under_current_guard(&guard)
        })
    }
}

impl PendingRetain {
    fn retry_intent(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        request: &V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        if let Some(original) = self
            .intent
            .as_ref()
            .and_then(|pending| pending.transaction.as_ref())
        {
            original.require_request_data(request)?;
        } else {
            self.transaction
                .as_ref()
                .ok_or_else(refused)?
                .require_request_data(request)?;
        }
        if self.intent.is_none() {
            self.require_absent_accepted_data()?;
            self.intent = Some(PendingUnsignedIntent::new());
        }
        let intent = self.intent.as_mut().ok_or_else(refused)?;
        PreparedV2TextUnsignedIntent::take_into(
            store,
            owner.source(),
            &mut self.transaction,
            &mut self.unsigned_custody,
            &mut self.accepted.as_mut().ok_or_else(refused)?.query,
            &mut intent.transaction,
        )?;
        intent.retry(store, owner, lane, predecessor, request, rng)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn unsigned_intent_saved_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .and_then(|p| p.intent.as_ref())
            .and_then(|p| p.transaction.as_ref())
            .is_some_and(PreparedV2TextUnsignedIntent::has_saved_data)
    }
    /// Persist/select only the SAME unsigned intent. No signature, accepted B, rendered
    /// message or network publication follows from returning this SavedI outcome.
    pub(in crate::store) fn retry_unsigned_intent_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if let Some(pending) = self.request_preparation.as_mut() {
            pending.drain_retained()?;
        }
        let has_intent = self
            .request_preparation
            .as_ref()
            .and_then(|p| p.retained.as_ref())
            .is_some_and(|p| p.intent.is_some());
        if !has_intent {
            self.retry_prepare_unsigned_custody_data(
                layout,
                V2CreateRequest {
                    token: request.token,
                    channel: request.channel,
                    text: request.text,
                },
                rng,
            )?;
        }
        // Once I owns all three originals, never rerun the old A/query/SDK pipeline.
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        pending.retained.as_mut().ok_or_else(refused)?.retry_intent(
            store,
            &owner,
            pending.lane.as_ref().ok_or_else(refused)?,
            pending.predecessor.as_ref().ok_or_else(refused)?,
            &request,
            rng,
        )
    }
}
