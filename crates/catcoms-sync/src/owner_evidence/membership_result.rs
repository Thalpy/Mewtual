//! Post-install consuming response and trusted in-process checkpoint proof handoff. No signed
//! result getter exists on the private candidate, and immutable outcomes own no runtime lease.
use super::membership_request::OwnerDirectRequest;
use super::*;
use catcoms_rt::{RequestAuthority, SharedRequestKeepalive};

struct ReplyBytes {
    operation: Arc<OwnerOperationTail>,
    _service: SharedRequestKeepalive,
}
struct CommitBytes {
    bytes: Vec<u8>,
    _service: SharedRequestKeepalive,
}
impl AsRef<[u8]> for CommitBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl AsRef<[u8]> for ReplyBytes {
    fn as_ref(&self) -> &[u8] {
        &self.operation.response
    }
}
#[derive(Debug)]
struct OriginalReplyEffect(RequestCancellation);
#[async_trait::async_trait]
impl RequestAuthority for OriginalReplyEffect {
    fn authorized(&self) -> bool {
        !self.0.is_cancelled()
    }
    async fn revoked(&self) {
        let mut original = self.0.clone();
        original.cancelled().await;
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn owner_commit_submission_owed(&self) -> bool {
        self.owner_operation.is_some() && !self.owner_commit_submitted
    }

    /// Submit retained exact commit debt under the ORIGINAL native scope and actual owned
    /// byte custody. `true` means this call got a local driver classification; false skips an
    /// already submitted runtime item. Neither clears durable debt or claims remote receipt.
    /// Admitted libp2p cache/handler memory is outside the existing one-shot queue/private-phase
    /// contract; whole active-host/cache resource admission remains an explicit activation gate.
    pub async fn publish_saved_owner_operation_checkpoint(
        &mut self,
        effect: RequestCancellation,
        service: SharedRequestKeepalive,
    ) -> Result<bool, catcoms_rt::PublishOnceError> {
        use catcoms_rt::PublishOnceError;
        if self.owner_commit_submitted {
            return Ok(false);
        }
        if effect.is_cancelled() || self.owner_evidence_unbound() || !self.local_chat_scope.live() {
            return Err(PublishOnceError::Revoked);
        }
        let operation = self
            .owner_operation
            .as_ref()
            .ok_or(PublishOnceError::Revoked)?;
        let record = self
            .commit_log
            .iter()
            .find(|record| record.commit_id() == operation.commit)
            .ok_or(PublishOnceError::Failed)?;
        if record.group_id.len() > 256
            || record.mls_commit.len() > catcoms_rt::MAX_PUBLISH_ONCE_BYTES - 1024
            || operation.topic.len() > catcoms_rt::MAX_PUBLISH_ONCE_TOPIC_BYTES
        {
            return Err(PublishOnceError::TooLarge);
        }
        let encoded = record.encode();
        let mut frame = Vec::with_capacity(1 + encoded.len());
        frame.push(CTRL_COMMIT);
        frame.extend_from_slice(&encoded);
        let data = Bytes::from_owner(CommitBytes {
            bytes: frame,
            _service: service.clone(),
        });
        self.transport
            .publish_once_authorized(
                Topic::new(Bytes::copy_from_slice(&operation.topic)),
                data,
                effect.retain_custody(service),
            )
            .await?;
        self.owner_commit_submitted = true;
        Ok(true)
    }
    /// Inert original founder realm label for the explicitly closed native checkpoint host.
    pub fn owner_checkpoint_channel(&self) -> Result<u128, SyncError> {
        self.initial_owner_evidence
            .as_ref()
            .ok_or(SyncError::ChatScopeUnavailable)?
            .founder_channel(&self.device.public_key_bytes())
    }
    /// Attempt a response from the already installed immutable outcome. The host supplies the
    /// ORIGINAL native writer/admission capture from before its tick await and real prepaid
    /// service custody. This consumes the exact Responder; actual driver admission rechecks the
    /// same effect. ReplyWait cancellation is deliberately NOT part of the final effect gate.
    pub fn respond_saved_owner_operation_checkpoint(
        &self,
        request: OwnerDirectRequest,
        effect: RequestCancellation,
        service: SharedRequestKeepalive,
    ) -> Result<(), TransportError> {
        if effect.is_cancelled() || self.owner_evidence_unbound() || !self.local_chat_scope.live() {
            return Err(TransportError::Cancelled);
        }
        let operation = self
            .owner_operation
            .as_ref()
            .ok_or(TransportError::NoResponse)?;
        let (invite, package) = request
            .original_parts()
            .map_err(|_| TransportError::NoResponse)?;
        if !operation.exact_request(operation.channel, &invite.encode(), package) {
            return Err(TransportError::NoResponse);
        }
        let bytes = Bytes::from_owner(ReplyBytes {
            operation: operation.clone(),
            _service: service.clone(),
        });
        request.responder.try_respond_with_custody(
            bytes,
            Arc::new(OriginalReplyEffect(effect)),
            service,
        )
    }

    /// Bounded immutable metadata only. Native must still hold/check actual original Current/
    /// saved-head custody before using a result or transferring its original public proof.
    pub fn owner_operation_request_id(&self) -> Option<[u8; 32]> {
        self.owner_operation
            .as_ref()
            .map(|operation| operation.binding)
    }

    /// Trusted-process CHECKPOINT proof handoff after actual source Saved/current/head. The
    /// destination's expectation was captured independently BEFORE its selected-KP handshake;
    /// never build it from these facts. This returns the existing opaque actual member witness,
    /// not reusable signed bytes, a root pin, or evidence of a remote fsync protocol.
    pub fn verify_joined_from_current_owner_checkpoint<U: MeshTransport, S: CryptoRngCore>(
        &self,
        member: &ChannelSync<U, S>,
        expected: &OwnerProfileExpectation,
        channel: u128,
    ) -> Result<VerifiedJoinedOwnerEvidence, OwnerEvidencePrepareError> {
        if self.owner_evidence_unbound()
            || !self.local_chat_scope.live()
            || self
                .owner_operation
                .as_ref()
                .is_none_or(|operation| operation.channel != channel)
        {
            return Err(OwnerEvidencePrepareError::Context);
        }
        let state = self
            .initial_owner_evidence
            .as_ref()
            .ok_or(OwnerEvidencePrepareError::Context)?;
        let frames = state.operation_proof_frames()?;
        let refs: Vec<_> = frames.iter().map(Vec::as_slice).collect();
        member.verify_joined_owner_evidence(expected, &refs, channel)
    }
}
