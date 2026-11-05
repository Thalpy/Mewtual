//! Original private fresh-source observations. No snapshot/decoder/raw label can create one.
use super::*;
use std::sync::{Arc, Weak};
use tokio::sync::watch;

pub(super) struct FreshOriginLife {
    identity: Arc<()>,
    owner: watch::Sender<bool>,
}
impl FreshOriginLife {
    pub(super) fn new() -> Self {
        Self {
            identity: Arc::new(()),
            owner: watch::channel(false).0,
        }
    }
    pub(super) fn observe(&self) -> OriginalFounderObservation {
        OriginalFounderObservation {
            identity: Arc::downgrade(&self.identity),
            negative: RequestCancellation::new(self.owner.subscribe(), None),
        }
    }
}

/// Non-Clone receiver-only original lifetime, not current membership or a save capability.
/// Weak physical identity cannot keep the actual source owner alive or grant a replacement.
pub(crate) struct OriginalFounderObservation {
    identity: Weak<()>,
    negative: RequestCancellation,
}
impl OriginalFounderObservation {
    /// Allocation-free, negative-only comparison for the closed V2 genesis pairing phase.
    /// Its private constructor already bound the immutable restored device's full signer;
    /// here use borrowed group/own-leaf fields rather than the generic allocating comparison.
    pub(crate) fn matches_genesis_restored(
        &self,
        source: &VerifiedFreshChatAdmission,
        group: &ServerGroup,
        actual_signer: &[u8; 32],
    ) -> bool {
        !self.negative.is_cancelled()
            && self
                .identity
                .upgrade()
                .is_some_and(|original| Arc::ptr_eq(&original, &source.origin.identity))
            && matches!(source.kind, FreshKind::Found { .. })
            && source.group.as_slice() == group.group_id_ref()
            && &source.signer == actual_signer
            && source.admission_state == group.epoch_authenticator_id()
            && group.is_active()
            && group.has_local_signer(actual_signer)
    }

    pub(crate) fn matches(
        &self,
        source: &VerifiedFreshChatAdmission,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> bool {
        !self.negative.is_cancelled()
            && self
                .identity
                .upgrade()
                .is_some_and(|original| Arc::ptr_eq(&original, &source.origin.identity))
            && source.is_original_founder_state(group, device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn actual_founder_observation_does_not_keep_the_taken_original_token_owner_alive() {
        let mut node = ChannelSync::found(
            Hub::new().join(PeerId::from_u64(0x2f15)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x2f15),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        let observation = node
            .fresh_chat_admission()
            .unwrap()
            .observe_original_founder(&node.group, &node.device)
            .unwrap();
        assert!(observation.identity.upgrade().is_some());
        assert!(!observation.negative.is_cancelled());
        let source = node.take_fresh_chat_admission().unwrap();
        assert!(
            !observation.negative.is_cancelled(),
            "the actual moved owner remains alive"
        );
        drop(source);
        assert!(observation.identity.upgrade().is_none());
        assert!(
            observation.negative.is_cancelled(),
            "receiver-only observation cannot retain the owner/sender"
        );
    }
}
