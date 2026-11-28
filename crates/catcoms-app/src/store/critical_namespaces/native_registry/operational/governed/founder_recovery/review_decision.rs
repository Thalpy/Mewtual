//! Prepare the first private review signature under the actual recovered
//! founder. Covering Raw here grants no selected decision or delivery.
use super::*;
use crate::store::persistence::governance_layout::joined::review_decision::ReviewSigningSpec;
use crate::store::NativeTextInitialReviewRequest;
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewDecisionBody, ExactReviewOriginal, ReviewDecisionBody, ReviewSpec,
        ReviewStreamEntry, ReviewStreamPosition,
    },
    FounderGenesisAnchorData, V2RemovedCreateRecord,
};
use catcoms_rt::{NativeChatKeyEnds, NativeChatPreparedKeyCut, NativeChatSourceObservation};

impl NativeTextColdFounderSource {
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

    pub(in crate::store) fn finish_initial_review(
        &mut self,
        (request, select): (NativeTextInitialReviewRequest, bool),
        (native, cut, ends): (
            &NativeChatSourceObservation,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
        caller: &RequestCancellation,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ReviewStreamEntry, AppError> {
        if request.intent == [0; 16] {
            return Err(refused());
        }
        self.restore_first_add(caller)?;
        let scope = self.history_scope(request.channel)?;
        let outbox = self.outbox_context()?.id();
        let (_, _ancestry_fee) = self.work.recovered_founder_resources(review::VERIFY_FEE)?;
        let (work, selection, source) = (&self.work, &self.selection, &self.source);
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let sdk = self.sdk.as_mut().ok_or_else(refused)?;
        let genesis = source.data().genesis_public_data();
        let profile = genesis.profile_data();
        let header = genesis.header_data();
        let pin = FounderGenesisAnchorData {
            group: profile.spec().group.clone(),
            founder: profile.spec().founder,
            profile: profile.id(),
            tenure: profile.tenure(),
            admission: genesis.admission_data().admission_id(),
            header: header.id(),
            state: header.spec().state,
        };
        let current = || {
            if caller.is_cancelled() {
                return Err(refused());
            }
            work.require_recovered_founder_source(selection, source)
        };
        work.with_recovered_founder_store(selection, source, |store| {
            governed.with_checked_review_parts(store, source, |verifier, group, frozen, require| {
                let mut check =
                    |position: Option<ReviewStreamPosition>| {
                        current()?;
                        require(position.map(|position| {
                            (position.membership_header, position.membership_epoch)
                        }))
                    };
                let mut original =
                    store.prepare_held_review_original(scope, request.case, || check(None))?;
                original.with_original_data(|bytes, require_held| {
                    let held = V2RemovedCreateRecord::decode(bytes).map_err(|_| refused())?;
                    let target = held.target_data();
                    // This vault's ID5 author is the original founder only.
                    // A removed admission with another full signer cannot
                    // have an independently selected original in that lane.
                    let author_data = target.spec();
                    if author_data.context.admission == pin.admission
                        || author_data.subject == pin.founder
                    {
                        return Err(refused());
                    }
                    let author = review::frozen_author(frozen, author_data.context.admission)?;
                    let raw_original = held.original_bytes_data();
                    let ancestry = review::review_ancestry(
                        (store, scope, outbox),
                        (verifier, group, frozen),
                        (author, raw_original),
                        &mut || check(None),
                    )?;
                    let evidence = ancestry.evidence(verifier, author, raw_original)?;
                    let decision = ExactReviewDecisionBody::new(
                        ReviewDecisionBody::new(ReviewSpec {
                            target: target.clone(),
                            intent: request.intent,
                            decision: request.decision,
                            previous: None,
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
                    decision
                        .verify_new_transition_from(None)
                        .map_err(|_| refused())?;
                    let replay = verifier.replay_data();
                    let position = ReviewStreamPosition {
                        sequence: 1,
                        previous: None,
                        membership_header: replay.header().id(),
                        membership_epoch: replay.mls_epoch_data(),
                    };
                    store.with_absent_ordinary_review_target(scope, request.case, || {
                        store.with_review_decision_control(|mount, control| {
                            let spec = ReviewSigningSpec {
                                pin: &pin,
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
                                |preimage| {
                                    sdk.sign_recovered_review_native(
                                        verifier,
                                        source.data(),
                                        (position, &decision, &evidence),
                                        preimage,
                                        (native, cut, ends),
                                    )
                                    .map_err(AppError::from)
                                },
                                rng,
                            )?;
                            if select {
                                control.select_first(
                                    (store, mount),
                                    (spec, &entry),
                                    |position| recheck(Some(position)),
                                    rng,
                                )?;
                            }
                            Ok(entry)
                        })
                    })
                })
            })
        })
    }
}
