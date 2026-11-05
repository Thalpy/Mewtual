//! Exact retained CLIENT request and immutable private pre-join provider. This is in-process
//! custody only; there is deliberately NO raw snapshot decoder -> Prepared/fresh constructor.
//! The later durable journal needs its own original mounted-source consumer. Legacy join APIs
//! retain their bytes/behavior. SDK typed heap/work admission remains an explicit separate gate.
use super::*;
use catcoms_mls::persist::{restore_join_device_provider_bounded, snapshot_join_device_bounded};

const MAX_INVITE_BYTES: usize = 64 * 1024;
const MAX_KEY_PACKAGE_BYTES: usize = 32 * 1024;

/// Resource metadata, not proof of payment, Fresh membership or a saved result. The actual App
/// client derives these limits from its retained original creation fee and keeps that owner.
#[derive(Clone, Copy)]
pub struct DirectJoinLimits {
    provider: usize,
    backing: usize,
    exchange: usize,
}
impl DirectJoinLimits {
    pub fn new(provider: usize, backing: usize, exchange: usize) -> Result<Self, SyncError> {
        if provider == 0 || backing == 0 || exchange == 0 || exchange > MAX_CONTROL_RESPONSE {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            provider,
            backing,
            exchange,
        })
    }
}

/// Owns the SAME invite, selected KP reference, exact original request and secret private master.
/// Only synchronous fresh preparation constructs it. Error/cancellation never consumes the
/// master; every authenticated reply is processed on a new private provider instead. No Clone,
/// serde, plaintext getter or generic recovered-source constructor exists.
pub struct PreparedDirectJoin {
    master: Zeroizing<Vec<u8>>,
    request: Bytes,
    invite: InviteToken,
    signer: [u8; 32],
    expected: [u8; 32],
    limits: DirectJoinLimits,
    completed: bool,
}
impl fmt::Debug for PreparedDirectJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedDirectJoin").finish_non_exhaustive()
    }
}

/// Actual authenticated candidate, produced once from the retained source, never decoded.
/// Its routing owns the fresh completion from the real selected-KP Welcome. This is still a
/// PRIVATE graph, not a saved/Current/native publication capability.
pub struct CompletedDirectJoin {
    device: MlsDevice,
    group: ServerGroup,
    routing: RoutingState,
}
impl CompletedDirectJoin {
    /// Negative consistency check on this actual nondecoded completion, never a constructor
    /// from labels or a proof of Current/save. Used before moving the original owned transport.
    #[doc(hidden)]
    pub fn validate_source(&self, group: &[u8], signer: &[u8; 32]) -> Result<[u8; 32], SyncError> {
        let source = self
            .routing
            .fresh_chat_admission
            .as_ref()
            .ok_or(SyncError::JoinRejected)?;
        if self.group.group_id() != group
            || self.device.public_key_bytes().as_slice() != signer.as_slice()
            || source.group() != group
            || source.signer() != signer
        {
            return Err(SyncError::JoinRejected);
        }
        Ok(source.attempt_id())
    }

    #[doc(hidden)]
    pub fn into_parts(self) -> (MlsDevice, ServerGroup, RoutingState) {
        (self.device, self.group, self.routing)
    }
}

impl PreparedDirectJoin {
    /// Mint exactly ONCE before network work, snapshot that actual private material, then drop
    /// the original mutable device. Provider raw capacities/output are preflighted. The input
    /// Invite is re-materialized from bounded canonical frames so spare caller Vec/String
    /// capacity cannot remain attached to an asynchronous request. SDK work itself is not metered.
    pub fn prepare(
        device: MlsDevice,
        invite: InviteToken,
        limits: DirectJoinLimits,
    ) -> Result<Self, SyncError> {
        preflight_invite(&invite)?;
        let encoded_invite = invite.encode();
        if encoded_invite.len() > MAX_INVITE_BYTES || encoded_invite.len() > limits.exchange {
            return Err(SyncError::Malformed);
        }
        let invite = InviteToken::decode(&encoded_invite).map_err(|_| SyncError::Malformed)?;
        if !invite.verify_self() {
            return Err(SyncError::JoinRejected);
        }
        let signer = device
            .public_key_bytes()
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        // Refuse an already oversized backing/map before the SDK adds another package. The
        // temporary encoding is released before the real master is produced. KeyPackage SDK
        // construction still needs the separately documented typed-work/heap gate.
        drop(snapshot_join_device_bounded(
            &device,
            limits.provider,
            limits.backing,
        )?);
        let package = device.key_package_for_invite(&invite.group_id, invite.invite_nonce)?;
        let key_package =
            catcoms_mls::device::serialize_key_package_bounded(&package, MAX_KEY_PACKAGE_BYTES)?;
        let expected = device.key_package_reference(&package)?;
        let total = 9usize
            .checked_add(encoded_invite.len())
            .and_then(|length| length.checked_add(key_package.len()))
            .filter(|length| *length <= limits.exchange)
            .ok_or(SyncError::Malformed)?;
        let mut request = Vec::with_capacity(total);
        request.push(KIND_JOIN);
        request.extend_from_slice(&(encoded_invite.len() as u32).to_be_bytes());
        request.extend_from_slice(&encoded_invite);
        request.extend_from_slice(&(key_package.len() as u32).to_be_bytes());
        request.extend_from_slice(&key_package);
        let master = snapshot_join_device_bounded(&device, limits.provider, limits.backing)?;
        // No mutable original provider survives preparation. Every later SDK entry uses a
        // separately restored candidate; processing cannot consume the master's private keys.
        drop(device);
        Ok(Self {
            master,
            request: Bytes::from(request),
            invite,
            signer,
            expected,
            limits,
            completed: false,
        })
    }

    pub fn group(&self) -> &[u8] {
        &self.invite.group_id
    }
    pub fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    pub fn inviter_device_id(&self) -> DeviceId {
        self.invite.inviter_device_id
    }

    /// Retry the original bytes. Caller MUST bound the whole direct/pushed exchange with its
    /// runtime timeout and retain the same object after an error or dropped future. Only a
    /// successful actual candidate handoff closes this object; a second success is refused.
    pub async fn finish_tracking<T: MeshTransport>(
        &mut self,
        transport: &T,
        connection_handoff: &mut PreOwnerConnectionHandoff,
        inviter: PeerId,
    ) -> Result<CompletedDirectJoin, SyncError> {
        if self.completed {
            return Err(SyncError::JoinRejected);
        }
        let response = transport
            .request(inviter, ProtocolId(RR_PROTOCOL), self.request.clone())
            .await?;
        let response = own_response(response, self.limits.exchange)?;
        let completed = match response.split_first() {
            Some((&JOIN_READY, body)) => self.finish_authenticated_body(body)?,
            Some((&JOIN_PENDING, pending)) if pending.is_empty() => {
                let mut dispatched = 0;
                loop {
                    if dispatched == 16 {
                        yield_pending_turn().await;
                        dispatched = 0;
                    }
                    let event = transport.next_event().await;
                    dispatched += 1;
                    match event {
                        Some(TransportEvent::Request {
                            from,
                            data,
                            responder,
                            ..
                        }) if from == inviter && data.first() == Some(&KIND_WELCOME) => {
                            responder.respond(Bytes::new());
                            let data = own_response(data, self.limits.exchange)?;
                            match data[1..].split_first() {
                                Some((&JOIN_READY, body)) => {
                                    break self.finish_authenticated_body(body)?
                                }
                                _ => return Err(SyncError::JoinRejected),
                            }
                        }
                        Some(event) if connection_handoff.observe(&event) => {}
                        Some(TransportEvent::Request { responder, .. }) => {
                            responder.respond(Bytes::new())
                        }
                        Some(_) => {}
                        None => return Err(SyncError::JoinRejected),
                    }
                }
            }
            _ => return Err(SyncError::JoinRejected),
        };
        self.completed = true;
        Ok(completed)
    }

    fn finish_authenticated_body(&self, body: &[u8]) -> Result<CompletedDirectJoin, SyncError> {
        // Borrowed COMPLETE framing and pinned signature precede provider allocation/SDK load.
        // An inviter-signed but invalid Welcome/routing/policy may consume ONLY the candidate.
        if body.len() > self.limits.exchange {
            return Err(SyncError::Malformed);
        }
        let mut decoder = Decoder::new(body);
        let welcome = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
        let signature = decoder
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let sealed_routing = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
        decoder.finish().map_err(|_| SyncError::Malformed)?;
        {
            let transcript = join_transcript(
                &self.invite.group_id,
                &self.invite.invite_nonce,
                welcome,
                sealed_routing,
            );
            if !self.invite.verify_inviter_signature(&transcript, signature) {
                return Err(SyncError::JoinRejected);
            }
        }
        let candidate = restore_join_device_provider_bounded(
            &self.master,
            self.limits.provider,
            self.limits.backing,
        )?;
        if candidate.public_key_bytes().as_slice() != self.signer.as_slice() {
            return Err(SyncError::JoinRejected);
        }
        // Keep the existing enforcement: actual SDK-selected KP, exact group, decryptable
        // routing, immutable policy and finally real fresh-completion attachment.
        let (group, routing) = finish_join(
            &candidate,
            &self.invite,
            welcome,
            signature,
            sealed_routing,
            &self.expected,
        )?;
        // Welcome processing writes NEW JSON/group rows into the candidate. The original
        // prejoin map preflight does not bound that retained result. Refuse before transferring
        // any candidate outside this serial stage if its actual raw backing/payload/output no
        // longer fit the SAME original invoice. SDK transient expansion already happened: this
        // is a retained-state postflight, NOT preallocation/whole-SDK heap admission.
        drop(snapshot_join_device_bounded(
            &candidate,
            self.limits.provider,
            self.limits.backing,
        )?);
        Ok(CompletedDirectJoin {
            device: candidate,
            group,
            routing,
        })
    }
}

/// A ready event flood must return control to its caller/outer timeout. Yield once only AFTER
/// actual work; the next poll resumes without another self-wake until sixteen more events.
async fn yield_pending_turn() {
    let mut yielded = false;
    std::future::poll_fn(|context| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            context.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await
}

/// Detach admitted bytes from an unknown transport slice backing BEFORE SDK work. The lower
/// adapter must account for its own response/input lifetime independently; length alone cannot
/// describe that backing. This caller-owned copy requests only its admitted exact length and
/// fits the original exchange buffers. Refusal allocates no client response copy.
fn own_response(bytes: Bytes, limit: usize) -> Result<Vec<u8>, SyncError> {
    if bytes.len() > limit {
        return Err(SyncError::Malformed);
    }
    let mut owned = Vec::with_capacity(bytes.len());
    owned.extend_from_slice(&bytes);
    Ok(owned)
}

fn preflight_invite(invite: &InviteToken) -> Result<(), SyncError> {
    if invite.group_id.is_empty()
        || invite.group_id.len() > 256
        || invite.inviter_public_key.len() != 32
        || invite.bootstrap.len() > 64
        || invite.rendezvous.len() > 64
        || invite
            .policy
            .as_ref()
            .is_some_and(|policy| policy.group_id().len() > 256)
    {
        return Err(SyncError::Malformed);
    }
    // Fixed fields/framing plus maximum existing signed-policy envelope. Every variable address
    // length is charged BEFORE unsigned/complete invite encoding can allocate its copies.
    let mut size = 1024usize + catcoms_mls::policy::MAX_GROUP_POLICY_BYTES;
    for address in invite.bootstrap.iter().chain(invite.rendezvous.iter()) {
        size = size
            .checked_add(4)
            .and_then(|size| size.checked_add(address.len()))
            .filter(|size| *size <= MAX_INVITE_BYTES)
            .ok_or(SyncError::Malformed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
