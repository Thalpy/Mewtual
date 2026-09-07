//! Trusted local registry storage integration. This does not register managed documents in the
//! legacy doc map, run their gate, or provide durable storage. The app adapter owns those checks.

use super::*;
use catcoms_replication::epoch::MAX_SIGNED_EPOCH_OP_BYTES;
use catcoms_replication::registry::RegistryOp;
use catcoms_replication::DomainOp;
use catcoms_rt::PublishSubmission;

/// Opaque process-local sync incarnation, not a membership or UI-unlock capability. A pass binds
/// this when constructed so restoring the same group/device cannot authorize old queued work.
#[derive(Clone)]
pub struct RegistrySyncInstance(Arc<()>);

impl RegistrySyncInstance {
    pub(super) fn new() -> Self {
        Self(Arc::new(()))
    }
}

impl fmt::Debug for RegistrySyncInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrySyncInstance { .. }")
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Capture this exact live sync instance. A restored node always receives a fresh token.
    pub fn registry_instance(&self) -> RegistrySyncInstance {
        self.registry_instance.clone()
    }

    /// Pointer identity, deliberately not Arc's value equality (every unit value is equal).
    pub fn matches_registry_instance(&self, instance: &RegistrySyncInstance) -> bool {
        Arc::ptr_eq(&self.registry_instance.0, &instance.0)
    }

    /// Synchronous trusted-local storage adapter seam. There is no await/reentrancy or mutable
    /// membership handle. Callers must still enforce the durable document gate and inventories;
    /// borrowing this context is not by itself permission to edit or publish. Keep the exclusive
    /// sync/store borrows through subsequent publication to prevent known-state interleaving.
    pub fn with_registry_context<O>(
        &mut self,
        work: impl FnOnce(&ServerGroup, &MlsDevice, &dyn Clock, &mut R) -> O,
    ) -> O {
        work(
            &self.group,
            &self.device,
            self.clock.as_ref(),
            &mut self.rng,
        )
    }

    /// Submit one bounded, authentic OWN registry operation under current MLS/routing state.
    /// This low-level seam proves neither Open nor durability: the trusted app must obtain the
    /// packet from its checked saved replay while holding the store exclusively through await.
    /// No generic doc is opened and no retry is queued. A transport error/duplicate is not a
    /// rollback or delivery proof; libp2p may retain normal cache/handler effects after an attempt.
    pub async fn publish_local_registry_once(
        &mut self,
        expected_doc_id: u128,
        sealed: SealedOp,
    ) -> Result<PublishSubmission, SyncError> {
        if sealed.doc_type != DocType::DocRegistry
            || sealed.doc_id != expected_doc_id
            || sealed.epoch != self.group.epoch()
            || sealed.blob.ciphertext.len() > MAX_SIGNED_EPOCH_OP_BYTES + 4 + 16
        {
            return Err(SyncError::Malformed);
        }
        let own_key = self.device.public_key_bytes();
        if self
            .group
            .member_signature_key(&self.device.device_id())
            .as_deref()
            != Some(own_key.as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let key = self
            .group
            .channel_secret(&self.device, DocType::DocRegistry, expected_doc_id)?;
        let signed = sealed.open(&key)?;
        if signed.doc_type != DocType::DocRegistry
            || signed.doc_id != expected_doc_id
            || signed.author_device != self.device.device_id()
            || signed.author_pubkey != own_key
            || !signed.verify()
        {
            return Err(SyncError::Unauthorized);
        }
        let domain = DomainOp::decode(signed.domain_op.as_deref().ok_or(SyncError::Malformed)?)?;
        let operation = RegistryOp::decode(&domain.body)?;
        if operation.domain_op(&self.group.group_id(), domain.nonce)? != domain {
            return Err(SyncError::Malformed);
        }
        let topic = self
            .channel_topic_for(DocType::DocRegistry, expected_doc_id, self.routing_label)
            .ok_or(SyncError::NoSuchDoc)?;
        // No await separates final checks from starting the one-shot future. The exclusive
        // borrow keeps known membership/routing fixed until its result or cancellation.
        Ok(self
            .transport
            .publish_once(topic, Bytes::from(sealed.encode()))
            .await?)
    }
}

#[cfg(test)]
mod tests;
