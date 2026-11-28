//! The original recovered founder's narrow review signature. App owns the
//! covered intent, selected case and stream, and the authenticated Raw write.
use super::*;
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewDecisionBody, ReviewStreamPosition, UnsignedReviewStreamEntry,
        MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES,
    },
    FounderGenesisAnchorData, V2GeneralPrefixVerifier, V2RemovedCreateEvidence,
};
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatPreparedKeyCut, NativeChatSourceObservation,
};

impl PreparedV2FounderRecoveredAdd {
    /// App must cover the original exact Intent and Attempt before entry.
    /// Only a successful cut can use this retained founder key. Return the
    /// exact Raw immediately so App can cover it even if lock has started.
    /// The retained Raw grants no selected decision or native disclosure.
    pub fn sign_recovered_review_native(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        original: &OpenedBootstrapSourceData,
        (position, decision, held): (
            ReviewStreamPosition,
            &ExactReviewDecisionBody,
            &V2RemovedCreateEvidence,
        ),
        covered_preimage: &[u8],
        (native, cut, ends): (
            &NativeChatSourceObservation,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
    ) -> Result<[u8; 64], SyncError> {
        if self.source != original.source_id_data()
            || !self.group.is_designated_committer(&self.device)
            || self
                .native_origin
                .as_ref()
                .is_some_and(|bound| !bound.same_original(native))
            || position.membership_header != self.expected.header
            || position.membership_epoch != self.expected.epoch
        {
            return Err(unavailable());
        }
        self.bound_member(verifier)?;
        verifier
            .require_current_removed_create(held)
            .map_err(|_| unavailable())?;
        decision
            .require_evidence_data(held)
            .map_err(|_| unavailable())?;
        let genesis = original.genesis_public_data();
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
        let exact = UnsignedReviewStreamEntry::rederive(&pin, position, decision, covered_preimage)
            .map_err(|_| unavailable())?;
        cut.require_never_entered_data(native, NativeChatKeyPurpose::ReviewDecision)
            .map_err(|_| unavailable())?;
        // RT can retain an End before its final cancellation check. That
        // End alone never authorizes signing or creates a recoverable Raw.
        cut.enter(native, NativeChatKeyPurpose::ReviewDecision, ends)
            .map_err(|_| unavailable())?;
        if self.native_origin.is_none() {
            self.native_origin = Some(native.clone());
        }
        self.device
            .sign_retained_ed25519_bounded(
                &self.signer,
                exact.signing_bytes(),
                MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES,
            )
            .map_err(|_| unavailable())
    }
}
