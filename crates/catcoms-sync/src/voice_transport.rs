//! V2 transport admission. Normal request dispatch routes kind28 here only when the local
//! `VoiceRouting` switch enables service (Disabled by default); `voice_schedule` drives the
//! endpoint/source/helper work it admits. Encrypted SDP/ICE still has no native phase factory or browser adapter, so only public
//! controls and protected confirmation reach native owners. Legacy kind8 and its authentication
//! clock/framing remain unchanged.
use super::*;
use voice_v2::helper::BoundRoute;
use voice_v2::{Disposition, Envelope, VoiceError, MAX_ENVELOPE};
mod forwarding;
mod reply;
pub(super) use forwarding::RouteOwners;
pub use forwarding::{
    CompletedVoiceForward, CompletedVoiceSource, PendingVoiceForward, PendingVoiceSource,
    VoiceCompletionFailure,
};
use reply::{Receipt, VoiceTransportReply};

const KIND: u8 = KIND_VOICE_TRANSPORT;
const OUTER_OVERHEAD: usize = 145;
const AUTH_AGE_MS: u64 = 20_000;
const RESPONSE_DOMAIN: &str = "mewtual/voice/transport-response/v2";

/// Strict borrowed framing: reject size/field width/trailing bytes before copying or crypto.
struct Outer<'a> {
    inner: &'a [u8],
    key: &'a [u8; 32],
    auth: RequestAuth,
    signature: &'a [u8; 64],
}
impl<'a> Outer<'a> {
    fn parse(frame: &'a [u8]) -> Result<Self, VoiceError> {
        if frame.len() > MAX_ENVELOPE + OUTER_OVERHEAD {
            return Err(VoiceError::Bounds);
        }
        let mut d = Decoder::new(frame);
        if d.get_u8()? != KIND {
            return Err(VoiceError::Malformed);
        }
        let inner = d.get_bytes()?;
        if inner.is_empty() || inner.len() > MAX_ENVELOPE {
            return Err(VoiceError::Bounds);
        }
        let key = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let ts = d.get_u64()?;
        let nonce = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let epoch = d.get_u64()?;
        let signature = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        d.finish()?;
        Ok(Self {
            inner,
            key,
            auth: RequestAuth { ts, nonce, epoch },
            signature,
        })
    }
}

/// Typed native custody observation, never decryption, room liveness or media evidence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceTransportStatus {
    /// The exact final endpoint admitted this packet under its native owner.
    EndpointAdmitted = 1,
    /// This helper retained the packet; the final recipient remains unconfirmed.
    HelperQueued = 2,
    /// The authenticated current peer refused this exact request.
    Refused = 3,
}
use VoiceTransportStatus as Status;
impl Status {
    fn parse(value: u8) -> Result<Self, VoiceError> {
        match value {
            1 => Ok(Self::EndpointAdmitted),
            2 => Ok(Self::HelperQueued),
            3 => Ok(Self::Refused),
            _ => Err(VoiceError::Malformed),
        }
    }
}
fn answer_body(status: Status, object: [u8; 32], inner: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(2);
    e.put_u8(status as u8);
    e.put_bytes(&object).expect("fixed");
    e.put_bytes(blake3::hash(inner).as_bytes()).expect("fixed");
    e.put_u32(inner.len() as u32);
    e.finish()
}
pub(super) fn response_transcript(
    group: &[u8],
    requester: &[u8],
    auth: RequestAuth,
    from: PeerId,
    responder: PeerId,
    body: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&signed_resp_transcript(
        RESPONSE_DOMAIN,
        group,
        requester,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        body,
    ))
    .expect("bounded");
    e.put_bytes(from.as_bytes()).expect("fixed");
    e.put_bytes(responder.as_bytes()).expect("fixed");
    e.finish()
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Verify only the signed typed response to this exact native request. A future detached
    /// completion must ALSO recheck its affine flight, route generation and local owner; this
    /// method cannot authorize application delivery or discharge a whole-room obligation.
    pub fn verify_voice_transport_response(
        &mut self,
        from: PeerId,
        request: &[u8],
        object_hash: [u8; 32],
        response: &[u8],
    ) -> Result<VoiceTransportStatus, VoiceError> {
        self.verify_voice_response(from, request, object_hash, response, None, None)
    }
    fn verify_voice_response(
        &mut self,
        from: PeerId,
        request: &[u8],
        object_hash: [u8; 32],
        response: &[u8],
        retained: Option<&Envelope>,
        progress: Option<&Arc<voice_v2::negotiation::Exchange>>,
    ) -> Result<VoiceTransportStatus, VoiceError> {
        if response.len() > 512 {
            return Err(VoiceError::Bounds);
        }
        self.reconcile_voice_actor();
        let request = Outer::parse(request)?;
        if request.key.as_slice() != self.device.public_key_bytes()
            || request.auth.epoch != self.group.epoch()
            || self.voice_v2_clock().now_ms().abs_diff(request.auth.ts) > AUTH_AGE_MS
        {
            return Err(VoiceError::Unauthorized);
        }
        let responder = self
            .voice_bound_route(from)
            .ok_or(VoiceError::Unauthorized)?
            .device;
        let mut d = Decoder::new(response);
        let key: &[u8; 32] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let signature: &[u8; 64] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let body = d.get_bytes()?;
        d.finish()?;
        if DeviceId::from_public_key_bytes(key) != responder
            || self.group.member_signature_key(&responder).as_deref() != Some(key.as_slice())
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut d = Decoder::new(body);
        if d.get_u8()? != 2 {
            return Err(VoiceError::Malformed);
        }
        let status = Status::parse(d.get_u8()?)?;
        let packet: [u8; 32] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let frame_hash: [u8; 32] = d
            .get_bytes()?
            .try_into()
            .map_err(|_| VoiceError::Malformed)?;
        let frame_bytes = d.get_u32()? as usize;
        d.finish()?;
        if frame_hash != *blake3::hash(request.inner).as_bytes()
            || frame_bytes != request.inner.len()
            || (packet != object_hash && (status != Status::Refused || packet != [0; 32]))
        {
            return Err(VoiceError::RetryMismatch);
        }
        // Detached helper work already owns the immutable verified envelope and full request
        // buffers. It reserves only bounded ACK crypto here. Public callers must debit the
        // original envelope's decode/encoding footprint instead; they cannot supply this owner.
        let verification_bytes = if retained.is_some() {
            response
        } else {
            request.inner
        };
        let ticket = self
            .voice_transport_actor()?
            // Verification also decodes the original envelope. Charge its maximum buffer
            // footprint, rather than only the small ACK, before any signature or allocation.
            .admit_transport_verification_in_exchange(
                responder,
                verification_bytes,
                progress,
                retained.is_some(),
            )?;
        let transcript = response_transcript(
            &self.group.group_id(),
            request.key,
            request.auth,
            self.transport.local_peer(),
            from,
            body,
        );
        if !verify_with_public_bytes(key, &transcript, signature) {
            return Err(VoiceError::Signature);
        }
        drop(transcript);
        let decoded;
        let envelope = if let Some(envelope) = retained {
            envelope
        } else {
            decoded = Envelope::decode(request.inner)?;
            &decoded
        };
        if envelope.object().hash() != object_hash {
            return Err(VoiceError::RetryMismatch);
        }
        // A helper can attest only to its own custody. Re-run the original request's role
        // checks as the responder, including the exact targeted member and current policy.
        // Otherwise a correctly signed helper ACK could falsely settle final delivery.
        let context = voice_v2::Context {
            local: responder,
            ..self.voice_v2_context(envelope.object().scope().room)?
        };
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let disposition = envelope.authorize(self.device.device_id(), &context, |id| {
            members.contains(&id)
        })?;
        if !matches!(
            (disposition, status),
            (_, Status::Refused)
                | (Disposition::Deliver, Status::EndpointAdmitted)
                | (Disposition::Forward(_), Status::HelperQueued)
        ) {
            return Err(VoiceError::Unauthorized);
        }
        ticket.check(self.voice_transport_actor()?, verification_bytes)?;
        Ok(status)
    }

    /// Unique current proven BOUND device/Noise-peer relations, intersected with the actual live
    /// connection snapshot. Released PEX source proofs and self-signed descriptors alone cannot
    /// enter this set. No routes are dialed or promoted by this lookup.
    pub fn voice_bound_routes(&self) -> Vec<BoundRoute> {
        self.voice_routes(None)
    }
    fn voice_bound_route(&self, peer: PeerId) -> Option<BoundRoute> {
        // Most hostile frames have no bound proof at all. Refuse before enumerating the roster
        // or descriptor table; even eligible requests need only this one endpoint's relation.
        if !self
            .member_peers
            .iter()
            .any(|proof| proof.bound && proof.peer == peer)
        {
            return None;
        }
        self.voice_routes(Some(peer)).into_iter().next()
    }
    fn voice_routes(&self, only: Option<PeerId>) -> Vec<BoundRoute> {
        let connections = self.transport.connection_snapshot();
        if connections.len() > catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT {
            return Vec::new();
        }
        let live: HashSet<_> = connections.into_iter().map(|row| row.peer).collect();
        // Enumerate/hash MLS members exactly once. Calling contains_device inside the proof ×
        // descriptor scan reconstructs the entire roster on each iteration before rate debit.
        let index = VoiceRouteIndex::new(
            self.group.member_device_ids(),
            self.peer_records
                .iter()
                .map(|(device, record)| (*device, PeerId::new(record.peer_id))),
        );
        let mut routes = Vec::new();
        let mut devices = HashSet::new();
        let mut peers = HashSet::new();
        for proof in &self.member_peers {
            if proof.bound
                && only.is_none_or(|peer| peer == proof.peer)
                && proof.device != self.device.device_id()
                && live.contains(&proof.peer)
                && index.members.contains(&proof.device)
                && self
                    .peer_records
                    .get(&proof.device)
                    .is_some_and(|record| record.peer_id == *proof.peer.as_bytes())
                && index.unique.get(&proof.peer) == Some(&Some(proof.device))
                && devices.insert(proof.device)
                && peers.insert(proof.peer)
            {
                routes.push(BoundRoute {
                    device: proof.device,
                    peer: proof.peer,
                });
            }
        }
        routes.sort_by_key(|route| route.device);
        routes
    }

    /// Crate transport-integration seam, not a webview signing/receive command and not yet a live
    /// wire handler. Unknown/unbound Noise endpoints refuse before verification. Endpoint ingress
    /// remains Refused until an exact native Watcher/LocalJoin queue is installed; helper retention
    /// is typed admission only and never establishes endpoint delivery, liveness or media.
    #[cfg(test)]
    pub(crate) fn receive_voice_transport(
        &mut self,
        from: PeerId,
        frame: &[u8],
    ) -> Option<VoiceTransportReply> {
        self.prepare_voice_transport_reply(from, frame, None)
    }

    /// Trusted dispatch seam, called by normal dispatch (`serve_voice_transport`). Bind the
    /// prepared ACK to this exact request's responder; no caller can detach response bytes or
    /// substitute a later responder. A refused preparation drops the responder unanswered, which
    /// the requester sees as unavailable. Driver ingress and packet gates remain distinct.
    pub fn handle_voice_transport_request(
        &mut self,
        from: PeerId,
        frame: &[u8],
        responder: catcoms_rt::Responder,
    ) {
        let original = responder.cancellation_authority();
        if let Some(reply) = self.prepare_voice_transport_reply(from, frame, Some(original)) {
            let _ = reply.respond(responder);
        }
    }

    fn prepare_voice_transport_reply(
        &mut self,
        from: PeerId,
        frame: &[u8],
        original_request: Option<Arc<dyn catcoms_rt::RequestAuthority>>,
    ) -> Option<VoiceTransportReply> {
        if original_request
            .as_ref()
            .is_some_and(|original| !original.authorized())
        {
            return None;
        }
        let outer = Outer::parse(frame).ok()?;
        let route = self.voice_bound_route(from)?;
        let requester = route.device;
        self.reconcile_voice_actor();
        if DeviceId::from_public_key_bytes(outer.key) != requester
            || self.group.member_signature_key(&requester).as_deref() != Some(outer.key.as_slice())
            || outer.auth.epoch != self.group.epoch()
            || self.voice_v2_clock().now_ms().abs_diff(outer.auth.ts) > AUTH_AGE_MS
        {
            return None;
        }
        // This affine debit replaces helper.authorize_route's usual requester charge. Its actual
        // credit stays owned through outer verification, strict inner signature decode and routing.
        let ticket = self
            .voice_transport_actor()
            .ok()?
            .admit_transport_verification(requester, outer.inner)
            .ok()?;
        let ticket = match original_request {
            Some(original) => ticket.bind_request(original).ok()?,
            None => ticket, // Unit intake oracle, never the production request handler.
        };
        // Helper admission consumes the affine credential; this independent actual-work owner
        // retains its one credit through the response signature without cloning admission.
        let verification_work = ticket.actual_work();
        let verification_authority = ticket.reply_authority();
        // Capacity and exact route ownership precede persistent packet/origin admission.
        // The route observes normal epoch changes too, which EffectAuthority alone does not.
        let reply_route = self
            .capture_voice_reply_route(route, verification_work.clone())
            .ok()?;
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            KIND,
            outer.inner,
            outer.key,
            outer.auth.ts,
            &outer.auth.nonce,
            outer.auth.epoch,
            Some(&from),
        );
        if !verify_with_public_bytes(outer.key, &transcript, outer.signature) {
            return None;
        }
        // Do not retain a full outer transcript alongside decoded ciphertext and canonical
        // encodings. The ticket accounts the remaining serialized-buffer peak explicitly.
        drop(transcript);
        ticket
            .check(self.voice_transport_actor().ok()?, outer.inner)
            .ok()?;
        let mut receipt = None;
        let (status, object_hash) = match Envelope::decode(outer.inner) {
            Err(_) => (Status::Refused, [0; 32]),
            Ok(envelope) => {
                let hash = envelope.object().hash();
                let status = (|| -> Result<Status, VoiceError> {
                    let context = self.voice_v2_context(envelope.object().scope().room)?;
                    let room = self.voice_room_authority(context.scope.room)?;
                    let routes = self.voice_bound_routes();
                    let disposition = self
                        .voice_transport_actor()?
                        .authorize_verified_route(&ticket, &envelope, &context)?;
                    match disposition {
                        Disposition::Forward(_) => {
                            let actor = self.voice_transport_actor()?;
                            let admitted = actor.helper()?.admit_verified(
                                actor, ticket, envelope, &context, &room, &routes,
                            )?;
                            if !admitted.authorized() {
                                return Err(VoiceError::Unauthorized);
                            }
                            receipt = Some(Receipt::Helper(admitted));
                            Ok(Status::HelperQueued)
                        }
                        Disposition::Deliver => {
                            let admitted =
                                self.admit_voice_endpoint(ticket, envelope, &context, room)?;
                            if admitted.object_hash() != hash || !admitted.authorized() {
                                return Err(VoiceError::Unauthorized);
                            }
                            receipt = Some(Receipt::Endpoint(admitted));
                            Ok(Status::EndpointAdmitted)
                        }
                    }
                })()
                .unwrap_or(Status::Refused);
                (status, hash)
            }
        };
        // A retained helper admission is not a final-recipient receipt. Even an authenticated
        // HelperQueued response leaves the source's final edge pending for alternate routes.
        self.voice_transport_actor().ok()?.helper().ok()?;
        let body = answer_body(status, object_hash, outer.inner);
        let transcript = response_transcript(
            &self.group.group_id(),
            outer.key,
            outer.auth,
            from,
            self.transport.local_peer(),
            &body,
        );
        let signature = self.device.sign(&transcript).ok()?;
        self.voice_transport_actor().ok()?.helper().ok()?;
        VoiceTransportReply::new(
            encode_signed_commit_resp(&self.device.public_key_bytes(), &signature, &body),
            verification_authority,
            reply_route,
            receipt,
            verification_work,
        )
    }
}

struct VoiceRouteIndex {
    members: HashSet<DeviceId>,
    unique: HashMap<PeerId, Option<DeviceId>>,
}
impl VoiceRouteIndex {
    fn new(
        members: impl IntoIterator<Item = DeviceId>,
        records: impl IntoIterator<Item = (DeviceId, PeerId)>,
    ) -> Self {
        let members: HashSet<_> = members.into_iter().collect();
        let mut unique = HashMap::new();
        for (device, peer) in records {
            if members.contains(&device) {
                unique
                    .entry(peer)
                    .and_modify(|owner| *owner = None)
                    .or_insert(Some(device));
            }
        }
        Self { members, unique }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_transport_route_index_visits_each_member_and_record_once_at_the_bound() {
        use std::cell::Cell;
        let member_visits = Cell::new(0);
        let record_visits = Cell::new(0);
        let devices: Vec<_> = (0..512u64)
            .map(|n| DeviceId::from_bytes(*PeerId::from_u64(n).as_bytes()))
            .collect();
        let index = VoiceRouteIndex::new(
            devices
                .iter()
                .copied()
                .inspect(|_| member_visits.set(member_visits.get() + 1)),
            devices
                .iter()
                .enumerate()
                .map(|(n, id)| (*id, PeerId::from_u64(n as u64)))
                .inspect(|_| record_visits.set(record_visits.get() + 1)),
        );
        assert_eq!((member_visits.get(), record_visits.get()), (512, 512));
        for (n, device) in devices.iter().enumerate() {
            assert_eq!(index.unique[&PeerId::from_u64(n as u64)], Some(*device));
        }
        let ambiguous = VoiceRouteIndex::new(
            devices.clone(),
            [
                (devices[1], PeerId::from_u64(3)),
                (devices[2], PeerId::from_u64(3)),
            ],
        );
        assert_eq!(ambiguous.unique[&PeerId::from_u64(3)], None);
        let outsider = DeviceId::from_bytes([255; 32]);
        let valid = VoiceRouteIndex::new(
            devices.clone(),
            [
                (devices[1], PeerId::from_u64(3)),
                (outsider, PeerId::from_u64(3)),
            ],
        );
        assert_eq!(valid.unique[&PeerId::from_u64(3)], Some(devices[1]));
    }
    fn framed(inner: &[u8], key: &[u8]) -> Vec<u8> {
        let mut result = vec![KIND];
        result.extend_from_slice(&encode_authed_request(
            inner, key, 1000, &[1; 16], 7, &[2; 64],
        ));
        result
    }
    #[test]
    fn voice_transport_borrows_closed_fixed_width_outer_frame_before_crypto() {
        let frame = framed(b"immutable native packet", &[9; 32]);
        let parsed = Outer::parse(&frame).unwrap();
        assert_eq!(parsed.inner, b"immutable native packet");
        assert!(std::ptr::eq(parsed.inner.as_ptr(), frame[5..].as_ptr()));
        assert!(Outer::parse(&framed(b"packet", &[9; 31])).is_err());
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(Outer::parse(&trailing).is_err());
        let mut changed_kind = frame.clone();
        changed_kind[0] = 8;
        assert!(Outer::parse(&changed_kind).is_err());
        for size in [0, MAX_ENVELOPE + 1] {
            assert!(Outer::parse(&framed(&vec![0; size], &[9; 32])).is_err());
        }
        assert!(Outer::parse(&framed(&vec![0; MAX_ENVELOPE], &[9; 32])).is_ok());
    }
    #[test]
    fn voice_transport_ack_domain_binds_both_noise_endpoints_and_exact_frame() {
        let body = answer_body(Status::HelperQueued, [4; 32], b"exact frame");
        let auth = RequestAuth {
            ts: 1000,
            nonce: [1; 16],
            epoch: 7,
        };
        let base = response_transcript(
            b"group",
            &[9; 32],
            auth,
            PeerId::from_u64(1),
            PeerId::from_u64(2),
            &body,
        );
        assert_ne!(
            base,
            response_transcript(
                b"group",
                &[9; 32],
                auth,
                PeerId::from_u64(3),
                PeerId::from_u64(2),
                &body
            )
        );
        assert_ne!(
            base,
            response_transcript(
                b"group",
                &[9; 32],
                auth,
                PeerId::from_u64(1),
                PeerId::from_u64(3),
                &body
            )
        );
        assert_ne!(
            body,
            answer_body(Status::EndpointAdmitted, [4; 32], b"exact frame")
        );
        assert_ne!(
            body,
            answer_body(Status::HelperQueued, [4; 32], b"other frame")
        );
    }
}
