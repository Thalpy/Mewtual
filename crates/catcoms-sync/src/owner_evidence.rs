//! Private owner-profile preparation from an ACTUAL unchanged founding operation.
//!
//! The returned object deliberately exposes no signed-record/export or successful-save API.
//! A forthcoming mounted-store Ready/lease adapter must cover the profile/archive/MLS state
//! before exposing certificates. This does not activate a protocol or install a trusted pin.
use super::*;
use catcoms_replication::chat::{owner_facts::*, ChatError, Rights};
mod initial_tail;
mod joined;
pub(super) use initial_tail::InitialOwnerEvidence;
pub use initial_tail::ManagedOwnerEvidenceBinding;
pub use joined::VerifiedJoinedOwnerEvidence;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OwnerEvidencePrepareError {
    #[error("owner evidence requires the original actual founder context")]
    Context,
    #[error(transparent)]
    Fact(#[from] ChatError),
}

/// Opaque private preparation. No Clone, decoder, raw signed-byte getter or caller-supplied
/// persistence boolean exists. Its labels alone confer no authoring or verification authority.
pub struct PreparedOwnerEvidence {
    profile: SignedOwnerFact,
    original_authenticator: [u8; 32],
    initial_channel: u128,
}
impl std::fmt::Debug for PreparedOwnerEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedOwnerEvidence")
            .field("profile", &self.profile.id())
            .field("initial_channel", &self.initial_channel)
            .finish_non_exhaustive()
    }
}
impl PreparedOwnerEvidence {
    /// Immutable metadata only; the root must still be acquired by the trusted host path.
    pub fn profile_id(&self) -> OwnerFactId {
        self.profile.id()
    }
    pub fn group(&self) -> &[u8] {
        &self.profile.spec().group
    }
    pub fn founder(&self) -> &[u8; 32] {
        self.profile.issuer()
    }
    pub fn initial_channel(&self) -> u128 {
        self.initial_channel
    }

    /// MLS-state equality ONLY, not a physical Actor/store/mount lease. A restored copy can
    /// match these bytes but cannot create this preparation. The native adapter must retain
    /// the original lease/capture; consuming fresh provenance for local scope initialization
    /// does not invalidate a preparation already made from that actual source.
    pub fn matches_original_mls_state<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> bool {
        self.group() == node.group.group_id()
            && self.founder().as_slice() == node.device.public_key_bytes()
            && self.original_authenticator == node.group.epoch_authenticator_id()
            && node.group.is_active()
            && node.group.has_local_signer(self.founder())
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Prepare private fixed-founder profile bytes without modifying membership, outboxes,
    /// documents or trusted configuration. Native/app persistence integration is still gated.
    /// A raw restored/new_joined group and a later roster cannot supply the required source.
    pub fn prepare_owner_evidence_checkpoint(
        &self,
        initial_channel: u128,
    ) -> Result<PreparedOwnerEvidence, OwnerEvidencePrepareError> {
        let source = self
            .fresh_chat_admission()
            .ok_or(OwnerEvidencePrepareError::Context)?;
        if !source.is_original_founder_state(&self.group, &self.device) {
            return Err(OwnerEvidencePrepareError::Context);
        }
        let group = self.group.group_id();
        let founder: [u8; 32] = self
            .device
            .public_key_bytes()
            .as_slice()
            .try_into()
            .map_err(|_| OwnerEvidencePrepareError::Context)?;
        // Public shared label only. Actual creation is established by the opaque source above;
        // this hash cannot establish it, and no vault nonce or current roster enters the ID.
        let mut h = blake3::Hasher::new();
        h.update(b"mewtual/chat/founder-creation/v1\0");
        h.update(&(group.len() as u32).to_be_bytes());
        h.update(&group);
        h.update(&founder);
        let creation = *h.finalize().as_bytes();
        let profile = SignedOwnerFact::sign_using(
            OwnerFactSpec {
                group,
                profile: None,
                body: OwnerFactBody::Profile { creation, founder },
            },
            founder,
            |bytes| self.device.sign(bytes).map_err(|_| ChatError::Signature),
        )?;
        Ok(PreparedOwnerEvidence {
            profile,
            original_authenticator: self.group.epoch_authenticator_id(),
            initial_channel,
        })
    }
}

#[cfg(test)]
mod tests;

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Stage original signed facts privately. No bytes are exported and no runtime authority is
    /// installed. The actual founder preparation and unchanged MLS/source are checked again;
    /// exact retry retains the SAME already-signed facts rather than resampling signatures.
    pub fn stage_owner_evidence_checkpoint(
        &mut self,
        prepared: &PreparedOwnerEvidence,
    ) -> Result<(), SyncError> {
        if !prepared.matches_original_mls_state(self)
            || self.fresh_chat_admission().is_none()
            || !self.docs.is_empty()
            || !self.ledger.is_empty()
            || !self.durable_chat.initialization_empty()
            || !self.local_chat_scope.initialization_snapshot_allowed()
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        if let Some(existing) = &self.initial_owner_evidence {
            return if existing.matches_preparation(prepared) {
                Ok(())
            } else {
                Err(SyncError::ChatScopeUnavailable)
            };
        }
        self.initial_owner_evidence = Some(InitialOwnerEvidence::founder(prepared, &self.device)?);
        Ok(())
    }

    /// Stage this actual exact-KP join's independently verified owner certificates. The host
    /// still must reserve original lineage/budget, sync the immutable pin + canonical snapshot,
    /// commit Current, and bind the actual Saved/head writer before any context/publication.
    pub fn stage_joined_owner_evidence_checkpoint(
        &mut self,
        verified: &VerifiedJoinedOwnerEvidence,
    ) -> Result<(), SyncError> {
        if !verified.matches_original(self)
            || !self.docs.is_empty()
            || !self.ledger.is_empty()
            || !self.durable_chat.initialization_empty()
            || !self.local_chat_scope.initialization_snapshot_allowed()
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        if let Some(existing) = &self.initial_owner_evidence {
            return if existing.matches_joined(verified) {
                Ok(())
            } else {
                Err(SyncError::ChatScopeUnavailable)
            };
        }
        self.initial_owner_evidence = Some(InitialOwnerEvidence::joined(verified));
        Ok(())
    }

    /// Trusted mounted canonical-host seam, analogous to the exact scope binding. Only the
    /// actual Saved+Current+writer result may implement this. A plain Current scope, decoded
    /// pin/tail, or current roster is insufficient. Host MUST validate complete frozen bytes
    /// and canonical source immediately before this synchronous call; Sync cannot authenticate
    /// arbitrary Rust trait implementations. No decoder invokes it.
    pub fn bind_managed_owner_evidence(
        &mut self,
        binding: &impl ManagedOwnerEvidenceBinding,
    ) -> Result<(), SyncError> {
        let state = self
            .initial_owner_evidence
            .as_mut()
            .ok_or(SyncError::ChatScopeUnavailable)?;
        if state.profile() != binding.profile_id()
            || binding.group() != self.group.group_id()
            || binding.signer().as_slice() != self.device.public_key_bytes()
            || binding.publication_authority().is_cancelled()
            || !self.local_chat_scope.live()
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        state.verify_local_chain(
            &self.group,
            &self.device,
            binding.owner_founder(),
            binding.signer(),
        )?;
        state.bind(binding.publication_authority())
    }

    /// Saved metadata only. A tail/profile label cannot construct canonical runtime authority.
    pub fn owner_evidence_profile_id(&self) -> Option<OwnerFactId> {
        self.initial_owner_evidence
            .as_ref()
            .map(InitialOwnerEvidence::profile)
    }

    pub(super) fn owner_evidence_unbound(&self) -> bool {
        self.initial_owner_evidence
            .as_ref()
            .is_some_and(|state| !state.live())
    }
}
