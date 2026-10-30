//! Private owner-profile preparation from an ACTUAL unchanged founding operation.
//!
//! The returned object deliberately exposes no signed-record/export or successful-save API.
//! A forthcoming mounted-store Ready/lease adapter must cover the profile/archive/MLS state
//! before exposing certificates. This does not activate a protocol or install a trusted pin.
use super::*;
use catcoms_replication::chat::{owner_facts::*, ChatError};

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
