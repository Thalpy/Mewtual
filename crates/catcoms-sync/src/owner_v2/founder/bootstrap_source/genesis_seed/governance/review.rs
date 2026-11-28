//! One review signature from the actual selected warm founder. App owns
//! the covered Intent and Attempt, the selected journal, and Raw custody.
use super::*;
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewDecisionBody, ReviewStreamPosition, UnsignedReviewStreamEntry,
        MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES,
    },
    FounderGenesisAnchorData, V2GeneralPrefixVerifier, V2RemovedCreateEvidence,
};
use catcoms_rt::NativeChatKeyRefusal;

catcoms_rt::redacted_debug! {
    V2FounderReviewSignOutcome;
}

/// Outcome of the original ReviewDecision key call. A local admission
/// refusal proves the signing primitive was never invoked, even if RT
/// already charged an End. Any other error leaves the Attempt ambiguous.
pub struct V2FounderReviewSignOutcome(Option<[u8; 64]>);

impl V2FounderReviewSignOutcome {
    pub(super) fn refused_before_signing() -> Self {
        Self(None)
    }

    pub(super) fn signed(raw: [u8; 64]) -> Self {
        Self(Some(raw))
    }

    /// The exact signature, or a definite Busy or Unavailable from the
    /// original native cut before the signing primitive could run.
    pub fn into_raw(self) -> Option<[u8; 64]> {
        self.0
    }
}

pub(super) fn enter_review_key(
    original: &NativeChatSourceObservation,
    cut: &mut NativeChatPreparedKeyCut,
    ends: &mut NativeChatKeyEnds,
) -> Result<bool, SyncError> {
    let purpose = NativeChatKeyPurpose::ReviewDecision;
    cut.require_never_entered_data(original, purpose)
        .map_err(|_| unavailable())?;
    match cut.enter(original, purpose, ends) {
        Ok(()) => Ok(true),
        Err(NativeChatKeyRefusal::Busy | NativeChatKeyRefusal::Unavailable) => Ok(false),
        Err(_) => Err(unavailable()),
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Sign only under the current selected founder pair and its original
    /// protected ReviewDecision cut. Return exact Raw before any late gate
    /// so the caller's admitted transaction can cover it during lock.
    pub fn sign_selected_review_native(
        &self,
        verifier: &V2GeneralPrefixVerifier,
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
    ) -> Result<V2FounderReviewSignOutcome, SyncError> {
        let original = self.native_origin.as_ref().ok_or_else(unavailable)?;
        let installed = self.installed.ok_or_else(unavailable)?;
        if self.pending.is_some()
            || self.removal.is_some()
            || !original.same_original(native)
            || (position.membership_header, position.membership_epoch)
                != (installed.header, installed.epoch)
        {
            return Err(unavailable());
        }
        // This confirms the actual MLS fingerprint and the same Core issuer
        // and founder Admission; the request position alone proves neither.
        self.observe_member(verifier, 0)?;
        verifier
            .require_current_removed_create(held)
            .map_err(|_| unavailable())?;
        decision
            .require_evidence_data(held)
            .map_err(|_| unavailable())?;
        let seed = &self.node.seed.body;
        let genesis = &seed.genesis;
        let pin = FounderGenesisAnchorData {
            group: genesis.profile.spec().group.clone(),
            founder: genesis.profile.spec().founder,
            profile: genesis.profile.id(),
            tenure: genesis.profile.tenure(),
            admission: genesis.admission.admission_id(),
            header: genesis.header.id(),
            state: genesis.header.spec().state,
        };
        let exact = UnsignedReviewStreamEntry::rederive(&pin, position, decision, covered_preimage)
            .map_err(|_| unavailable())?;
        if !enter_review_key(original, cut, ends)? {
            return Ok(V2FounderReviewSignOutcome::refused_before_signing());
        }
        seed.device
            .sign_retained_ed25519_bounded(
                &pin.founder,
                exact.signing_bytes(),
                MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES,
            )
            .map(V2FounderReviewSignOutcome::signed)
            .map_err(|_| unavailable())
    }
}
