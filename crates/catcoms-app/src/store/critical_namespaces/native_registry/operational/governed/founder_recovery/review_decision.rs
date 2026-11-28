//! Prepare a private review signature under the actual recovered
//! founder. Covering Raw here grants no selected decision or delivery.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::persistence::governance_layout::joined::review_decision::{
    DecisionControl, PreparedIndexedReviewSource, PreparedReviewBase, ReviewSigningSpec,
    SelectedReviewDecision,
};
use crate::store::persistence::v2_member_outbox::MemberSavedOriginal;
use crate::store::persistence::v2_shared_history::SharedHistoryScope;
use crate::store::{NativeTextInitialReviewRequest, ServerStore, V2MemberSaveRefusal};
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewDecisionBody, ExactReviewOriginal, ReviewDecisionBody, ReviewSpec,
        ReviewStreamEntry, ReviewStreamPosition,
    },
    ActionId, ChatLedger, FounderGenesisAnchorData, ProjectionView, V2FrozenAdmission,
    V2GeneralPrefixVerifier, V2RemovedCreateEvidence, V2RemovedCreateRecord,
};
use catcoms_rt::{NativeChatKeyEnds, NativeChatPreparedKeyCut, NativeChatSourceObservation};
use catcoms_sync::owner_v2::V2FounderReviewSignOutcome;

pub(super) use crate::store::persistence::v2_member_transport::original_review_anchor;

/// Borrowed exact decision evidence and the single protected warm key cut.
/// This lends no independent membership, row, or durable selection.
pub(in crate::store) struct FounderReviewSign<'a> {
    pub(in crate::store) decision: (
        ReviewStreamPosition,
        &'a ExactReviewDecisionBody,
        &'a V2RemovedCreateEvidence,
    ),
    pub(in crate::store) preimage: &'a [u8],
    pub(in crate::store) key: (
        &'a NativeChatSourceObservation,
        &'a mut NativeChatPreparedKeyCut,
        &'a mut NativeChatKeyEnds,
    ),
}

pub(in crate::store) type WarmFounderReviewKeys<'a> = (
    &'a NativeFounderKeys,
    &'a mut NativeChatPreparedKeyCut,
    &'a mut NativeChatKeyEnds,
);

impl NativeTextColdFounderSource {
    #[cfg(test)]
    pub(in crate::store) fn review_parent_storage_for_test(
        &self,
        channel: u128,
        parent: catcoms_replication::chat::ActionId,
    ) -> Result<(bool, bool), AppError> {
        let scope = self.history_scope(channel)?;
        self.work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                let outbox_is_installed = store.installed_member_outbox_mount()?.is_some();
                let parent_is_archived = store
                    .read_selected_shared_history_ancestor(scope, parent)?
                    .is_some();
                Ok((outbox_is_installed, parent_is_archived))
            })
    }

    #[cfg(test)]
    pub(in crate::store) fn require_review_anchor_for_test(
        &mut self,
        anchor: (OwnerHeaderId, u64),
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.restore_first_add(caller)?;
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        self.work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                governed.with_checked_review_parts(store, &self.source, |_, _, _, require| {
                    require(Some(anchor))
                })
            })
    }

    pub(in crate::store) fn finish_founder_review(
        &mut self,
        (request, select): (NativeTextInitialReviewRequest, bool),
        (native, cut, ends): (
            &NativeChatSourceObservation,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
        caller: &RequestCancellation,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<ReviewStreamEntry>, AppError> {
        if request.intent == [0; 16] {
            return Err(refused());
        }
        self.restore_first_add(caller)?;
        let scope = self.history_scope(request.channel)?;
        let (_, _ancestry_fee) = self.work.recovered_founder_resources(review::VERIFY_FEE)?;
        let (work, selection, source) = (&self.work, &self.selection, &self.source);
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let sdk = self.sdk.as_mut().ok_or_else(refused)?;
        let pin = original_review_anchor(source);
        let current = || {
            if caller.is_cancelled() {
                return Err(refused());
            }
            work.require_recovered_founder_source(selection, source)
        };
        work.with_recovered_founder_store(selection, source, |store| {
            governed.with_checked_review_parts(store, source, |verifier, group, frozen, require| {
                run_founder_review_transaction(
                    (store, scope, &pin),
                    (verifier, frozen),
                    (request, select),
                    |anchor| {
                        current()?;
                        require(anchor)
                    },
                    (
                        |ledger| {
                            ledger
                                .begin_projection_view_in(group)
                                .map_err(|_| refused())
                        },
                        |id, rng| {
                            history::saved_dependency(
                                store,
                                history::outbox_context(selection, source),
                                source,
                                scope,
                                id,
                                rng,
                            )
                        },
                    ),
                    |(position, decision, evidence), preimage| {
                        sdk.sign_recovered_review_native(
                            verifier,
                            source.data(),
                            (position, decision, evidence),
                            preimage,
                            (native, cut, ends),
                        )
                        .map_err(AppError::from)
                    },
                    rng,
                )
            })
        })
    }
}

/// Execute the same exact Intent, Attempt, Raw and case selection protocol
/// for either original founder family. The host supplies selected source,
/// local ancestry, and the one actual protected key callback.
pub(in crate::store) fn run_founder_review_transaction(
    (store, scope, pin): (&ServerStore, SharedHistoryScope, &FounderGenesisAnchorData),
    (verifier, frozen): (&V2GeneralPrefixVerifier, &[V2FrozenAdmission]),
    (request, select): (NativeTextInitialReviewRequest, bool),
    mut require: impl FnMut(Option<(OwnerHeaderId, u64)>) -> Result<(), AppError>,
    (begin_view, mut saved_dependency): (
        impl FnOnce(&mut ChatLedger) -> Result<ProjectionView, AppError>,
        impl FnMut(
            ActionId,
            &mut dyn CryptoRngCore,
        ) -> Result<Option<MemberSavedOriginal>, V2MemberSaveRefusal>,
    ),
    sign: impl FnOnce(
        (
            ReviewStreamPosition,
            &ExactReviewDecisionBody,
            &V2RemovedCreateEvidence,
        ),
        &[u8],
    ) -> Result<V2FounderReviewSignOutcome, AppError>,
    rng: &mut dyn CryptoRngCore,
) -> Result<Option<ReviewStreamEntry>, AppError> {
    if request.intent == [0; 16] {
        return Err(refused());
    }
    let mut check = |position: Option<ReviewStreamPosition>| {
        require(position.map(|position| (position.membership_header, position.membership_epoch)))
    };
    check(None)?;
    if select {
        store.finish_selected_review_append(pin, |position| check(Some(position)))?;
        if let Some(saved) =
            store.finish_selected_initial_review(pin, &request, |position| check(Some(position)))?
        {
            return Ok(Some(saved));
        }
    }
    let mut base = if select {
        store.prepare_review_base(pin, &request, &mut check)?
    } else {
        PreparedReviewBase::First
    };
    if let PreparedReviewBase::Saved(saved) = base {
        return Ok(Some(saved));
    }
    if matches!(base, PreparedReviewBase::SupersededBeforeAdmission) {
        // None has exactly this private issuer: an authenticated
        // nonce miss and superseded same-case predecessor, with
        // every signing/selection control absent. The original
        // native row/caller is rechecked by the outer operation
        // before converting this into the typed UI disposition.
        return Ok(None);
    }
    let mut original = store.prepare_held_review_original(scope, request.case, || check(None))?;
    let entry = original.with_original_data(|bytes, require_held| {
        let held = V2RemovedCreateRecord::decode(bytes).map_err(|_| refused())?;
        let target = held.target_data();
        // This vault's ID5 author is the original founder only.
        // A removed admission with another full signer cannot
        // have an independently selected original in that lane.
        let author_data = target.spec();
        if author_data.context.admission == pin.admission || author_data.subject == pin.founder {
            return Err(refused());
        }
        let author = review::frozen_author(frozen, author_data.context.admission)?;
        let raw_original = held.original_bytes_data();
        let ancestry = review::review_ancestry(
            (store, scope),
            (verifier, frozen),
            (author, raw_original),
            begin_view,
            |id| saved_dependency(id, rng),
            &mut || check(None),
        )?;
        let evidence = ancestry.evidence(verifier, author, raw_original)?;
        let decision = ExactReviewDecisionBody::new(
            ReviewDecisionBody::new(ReviewSpec {
                target: target.clone(),
                intent: request.intent,
                decision: request.decision,
                previous: request.previous,
            })
            .map_err(|_| refused())?,
            ExactReviewOriginal {
                digest: request.case.original,
                remove_header: request.case.removal.0,
                remove_epoch: request.case.removal.1,
            },
        )
        .map_err(|_| refused())?;
        decision
            .require_evidence_data(&evidence)
            .map_err(|_| refused())?;
        let prior_case = match &base {
            PreparedReviewBase::Next(prior) => prior.case_data(),
            _ => None,
        };
        decision
            .verify_new_transition_from(prior_case)
            .map_err(|_| refused())?;
        let replay = verifier.replay_data();
        let membership = (replay.header().id(), replay.mls_epoch_data());
        let position = match &base {
            PreparedReviewBase::Next(prior) => prior.next_position(membership)?,
            _ => ReviewStreamPosition {
                sequence: 1,
                previous: None,
                membership_header: membership.0,
                membership_epoch: membership.1,
            },
        };
        store.with_absent_ordinary_review_target(scope, request.case, || {
            let work = |mount: &SelectedReviewDecision,
                        control: &mut DecisionControl,
                        prior: Option<&mut PreparedIndexedReviewSource>| {
                let spec = ReviewSigningSpec {
                    pin,
                    position,
                    decision: &decision,
                    held: bytes,
                    request_digest: *decision.id().as_bytes(),
                };
                let mut recheck = |position| {
                    check(position)?;
                    require_held()
                };
                let entry = control.cover_review_raw(
                    (store, mount),
                    spec,
                    &mut recheck,
                    |preimage| sign((position, &decision, &evidence), preimage),
                    rng,
                )?;
                if let Some(prior) = prior {
                    control.select_later(
                        (store, mount),
                        (spec, &entry),
                        prior,
                        |position| recheck(Some(position)),
                        rng,
                    )?;
                } else if select {
                    control.select_first(
                        (store, mount),
                        (spec, &entry),
                        |position| recheck(Some(position)),
                        rng,
                    )?;
                }
                Ok(entry)
            };
            match &mut base {
                PreparedReviewBase::Next(prior) => store
                    .with_indexed_review_decision(prior, |mount, control, prior| {
                        work(mount, control, Some(prior))
                    }),
                _ => {
                    store.with_review_decision_control(|mount, control| work(mount, control, None))
                }
            }
        })
    })?;
    drop(original);
    drop(base);
    if select {
        store.finish_selected_review_append(pin, |position| check(Some(position)))?;
        if let Some(saved) =
            store.finish_selected_initial_review(pin, &request, |position| check(Some(position)))?
        {
            if saved != entry {
                return Err(refused());
            }
        }
    }
    Ok(Some(entry))
}
