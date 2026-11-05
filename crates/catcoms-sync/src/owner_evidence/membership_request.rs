//! One bounded original direct JOIN request, retained without invoking legacy admission. The
//! canonical App drain must reserve its own candidate/input copies BEFORE calling original_parts.
use super::*;
use catcoms_rt::{RequestAuthority, Responder};

/// Opaque original driver request. No Clone/decoder/native scalar can construct its responder;
/// dropping it honestly closes that wait without changing any accepted durable result.
pub struct OwnerDirectRequest {
    pub(super) from: PeerId,
    pub(super) data: Bytes,
    pub(super) responder: Responder,
}
impl std::fmt::Debug for OwnerDirectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerDirectRequest").finish_non_exhaustive()
    }
}
impl OwnerDirectRequest {
    /// Data decode only. The real Store drain pays bounded invite/KP copies FIRST; fresh
    /// invitation, policy, KP signature/lifetime and original writer checks happen separately.
    pub fn original_parts(&self) -> Result<(InviteToken, &[u8]), SyncError> {
        let Some((&KIND_JOIN, body)) = self.data.split_first() else {
            return Err(SyncError::Malformed);
        };
        let mut decoder = Decoder::new(body);
        let invite = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
        let package = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
        decoder.finish().map_err(|_| SyncError::Malformed)?;
        if invite.is_empty()
            || invite.len() > operation_tail::MAX_INVITE
            || package.is_empty()
            || package.len() > operation_tail::MAX_PACKAGE
        {
            return Err(SyncError::Malformed);
        }
        let decoded = InviteToken::decode(invite).map_err(|_| SyncError::Malformed)?;
        if decoded.encode().as_slice() != invite {
            return Err(SyncError::Malformed);
        }
        Ok((decoded, package))
    }
    pub fn is_current(&self) -> bool {
        self.responder.is_current()
    }
    pub fn peer(&self) -> PeerId {
        self.from
    }
    /// Request wait cancellation is useful before private work, but must NOT become permanent
    /// authority on the historical outcome or the final response gate (the driver consumes it).
    pub fn cancellation_authority(&self) -> Arc<dyn RequestAuthority> {
        self.responder.cancellation_authority()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn queue_owner_direct_request(
        &mut self,
        from: PeerId,
        data: Bytes,
        responder: Responder,
    ) {
        if self.owner_direct_request.is_some()
            || self.owner_evidence_unbound()
            || data.len() > MAX_CONTROL_REQUEST
        {
            responder.respond(Bytes::new());
            return;
        }
        self.owner_direct_request = Some(OwnerDirectRequest {
            from,
            data,
            responder,
        });
    }
    /// Native canonical host only: taking a request is neither admission nor a successful reply.
    /// No legacy serve_join/admit_now ran. A bounded drain must retain this original until its
    /// actual covering save/install or honest failure, never reconstruct another responder.
    pub fn take_owner_direct_request(&mut self) -> Option<OwnerDirectRequest> {
        self.owner_direct_request.take()
    }
}
