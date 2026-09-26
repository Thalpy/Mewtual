//! A connected member exchange establishing both descriptor/endpoint bindings after admission.
//! Private addresses never cross this boundary. The host retains only its own successful
//! outbound Noise dial evidence after this exchange proves the endpoint is a current member.
use super::*;

pub(super) const KIND_MEMBER_FINALIZE: u8 = 26;
const MAX_FRAME: usize = 4_096;
const RESPONSE_DOMAIN: &str = "catcoms/member-finalization/response/v1";
const DEADLINE_MS: u64 = 2_000;

fn body(
    policy: [u8; 32],
    requester: PeerId,
    provider: PeerId,
    descriptor: &PeerDescriptor,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(1);
    e.put_bytes(&policy).expect("digest fits");
    e.put_bytes(requester.as_bytes()).expect("peer fits");
    e.put_bytes(provider.as_bytes()).expect("peer fits");
    e.put_bytes(&descriptor.encode()).expect("descriptor fits");
    e.finish()
}

fn descriptor(
    bytes: &[u8],
    policy: [u8; 32],
    requester: PeerId,
    provider: PeerId,
) -> Option<PeerDescriptor> {
    if bytes.len() > MAX_FRAME {
        return None;
    }
    let mut d = Decoder::new(bytes);
    if d.get_u8().ok()? != 1
        || d.get_bytes().ok()? != policy
        || d.get_bytes().ok()? != requester.as_bytes()
        || d.get_bytes().ok()? != provider.as_bytes()
    {
        return None;
    }
    let record = PeerDescriptor::decode(d.get_bytes().ok()?).ok()?;
    d.finish().ok()?;
    Some(record)
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn prune_member_finalization_candidates(&mut self) {
        let local = self.transport.local_peer();
        self.member_finalization_pending.retain(|device, peer| {
            self.group.contains_device(device)
                && *peer != local
                && !self.peer_records.contains_key(device)
                && !self
                    .peer_records
                    .values()
                    .any(|record| record.peer_id == *peer.as_bytes())
        });
    }

    /// Remember the endpoint of an actually accepted MLS admission. This is retry metadata,
    /// never endpoint proof or permission to dial. A descriptor supersedes it, and replaying a
    /// cached Welcome from another endpoint cannot replace the original pending correlation.
    pub fn note_member_finalization_candidate(&mut self, peer: PeerId, device: DeviceId) {
        self.prune_member_finalization_candidates();
        if !self.policy_allows_member_mesh()
            || !self.group.contains_device(&device)
            || device == self.device.device_id()
            || peer == self.transport.local_peer()
            || self.peer_records.contains_key(&device)
            || self
                .peer_records
                .values()
                .any(|record| record.peer_id == *peer.as_bytes())
            || self
                .member_finalization_pending
                .iter()
                .any(|(other, candidate)| *other != device && *candidate == peer)
            || self.member_finalization_pending.len() >= MAX_PEER_RECORDS
        {
            return;
        }
        self.member_finalization_pending
            .entry(device)
            .or_insert(peer);
    }

    /// Current member claims and admitted endpoints eligible for connected-only finalization.
    /// Unknown Noise peers (including infrastructure) are deliberately absent.
    pub fn member_finalization_candidates(&mut self) -> Vec<PeerId> {
        self.prune_member_finalization_candidates();
        if !self.policy_allows_member_mesh() {
            return Vec::new();
        }
        let mut peers: Vec<_> = self
            .peer_records
            .iter()
            .filter(|(device, _)| {
                **device != self.device.device_id() && self.group.contains_device(device)
            })
            .map(|(_, record)| PeerId::new(record.peer_id))
            .chain(self.member_finalization_pending.values().copied())
            .collect();
        peers.sort();
        peers.dedup();
        peers
    }

    /// Current dual-key bindings suitable for retaining LOCAL outbound listener observations.
    /// Sealed hints remain candidates on restart and are roster/descriptor checked at each dial.
    pub fn finalized_member_peers(&self) -> Vec<PeerId> {
        if !self.policy_allows_member_mesh() {
            return Vec::new();
        }
        self.member_peers
            .iter()
            .filter(|proof| {
                proof.bound
                    && self.group.contains_device(&proof.device)
                    && self
                        .peer_records
                        .get(&proof.device)
                        .is_some_and(|record| record.peer_id == *proof.peer.as_bytes())
                    && self.peer_uniquely_claimed_by_current_member(proof.peer)
            })
            .map(|proof| proof.peer)
            .collect()
    }

    fn accept_finalization_record(
        &mut self,
        peer: PeerId,
        pubkey: &[u8],
        record: PeerDescriptor,
    ) -> bool {
        let device = DeviceId::from_public_key_bytes(pubkey);
        if record.device_pubkey != pubkey
            || record.peer_id != *peer.as_bytes()
            || !self.group.contains_device(&device)
            || !record.verify_self()
        {
            return false;
        }
        self.ingest_peer_record(record);
        if self
            .peer_records
            .get(&device)
            .is_none_or(|record| record.peer_id != *peer.as_bytes())
            || !self.peer_uniquely_claimed_by_current_member(peer)
        {
            return false;
        }
        self.promote_member_peer_bound(peer, device, true);
        self.touch_member_routes();
        true
    }

    /// Confirm continuing P2P membership over an existing connection, without any implicit dial.
    /// The policy, epoch, both transport endpoints and request digest are signed. Unsupported or
    /// temporarily unavailable peers leave the host's standing finalization obligation pending.
    pub async fn finalize_member_connection(&mut self, peer: PeerId) -> Result<bool, SyncError> {
        if !self.policy_allows_member_mesh() || !self.peer_is_connected(peer) {
            return Ok(false);
        }
        let Some(record) = self.self_record() else {
            return Ok(false);
        };
        let policy = self.group_policy_digest().ok_or(SyncError::Malformed)?;
        let requester = self.transport.local_peer();
        let inner = body(policy, requester, peer, record);
        let request_digest = *blake3::hash(&inner).as_bytes();
        let (request, auth) = self.build_authed_request(KIND_MEMBER_FINALIZE, &inner)?;
        let response = match futures::future::select(
            Box::pin(self.transport.request_connected(
                peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from(request),
            )),
            self.clock
                .sleep(std::time::Duration::from_millis(DEADLINE_MS)),
        )
        .await
        {
            futures::future::Either::Left((response, _)) => response?,
            futures::future::Either::Right(((), _)) => {
                return Err(TransportError::Unreachable(peer).into())
            }
        };
        if response.is_empty() {
            return Ok(false);
        }
        if response.len() > MAX_FRAME {
            return Err(SyncError::Malformed);
        }
        let (pubkey, signature, answer) = decode_signed_commit_resp(&response)?;
        if answer.len() < 32 || answer[..32] != request_digest {
            return Err(SyncError::Malformed);
        }
        let transcript = signed_resp_transcript(
            RESPONSE_DOMAIN,
            &self.group.group_id(),
            &self.device.public_key_bytes(),
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &answer,
        );
        if !verify_with_public_bytes(&pubkey, &transcript, &signature) {
            return Err(SyncError::Malformed);
        }
        let record =
            descriptor(&answer[32..], policy, requester, peer).ok_or(SyncError::Malformed)?;
        Ok(self.accept_finalization_record(peer, &pubkey, record))
    }

    pub(super) fn serve_member_finalization(
        &mut self,
        from: PeerId,
        data: &[u8],
    ) -> Option<Vec<u8>> {
        if !self.policy_allows_member_mesh() || data.len() > MAX_FRAME {
            return None;
        }
        let (inner, pubkey, auth) = self.authenticate_request(KIND_MEMBER_FINALIZE, data, from)?;
        if auth.epoch != self.group.epoch() {
            return None;
        }
        let policy = self.group_policy_digest()?;
        let provider = self.transport.local_peer();
        let record = descriptor(&inner, policy, from, provider)?;
        let own = self.self_record()?.clone();
        let device = DeviceId::from_public_key_bytes(&pubkey);
        let now = self.clock.monotonic_ms();
        if self
            .member_finalization_served_at
            .get(&device)
            .is_some_and(|last| now.saturating_sub(*last) < MIN_PEX_INTERVAL_MS)
        {
            return None;
        }
        if !self.member_finalization_served_at.contains_key(&device)
            && self.member_finalization_served_at.len() >= MAX_PEER_RECORDS
        {
            self.member_finalization_served_at
                .retain(|member, _| self.group.contains_device(member));
            if self.member_finalization_served_at.len() >= MAX_PEER_RECORDS {
                return None;
            }
        }
        if !self.accept_finalization_record(from, &pubkey, record) {
            return None;
        }
        self.member_finalization_served_at.insert(device, now);
        let mut answer = blake3::hash(&inner).as_bytes().to_vec();
        answer.extend_from_slice(&body(policy, from, provider, &own));
        let transcript = signed_resp_transcript(
            RESPONSE_DOMAIN,
            &self.group.group_id(),
            &pubkey,
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &answer,
        );
        let signature = self.device.sign(&transcript).ok()?;
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &answer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock, MemNetwork};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    type Node = ChannelSync<MemNetwork, ChaCha20Rng>;

    fn pair(p2p: bool) -> (Node, Node) {
        let hub = Hub::new();
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let mut alice = Node::new(
            hub.join(PeerId::from_u64(1)),
            group,
            device,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );
        if p2p {
            alice
                .initialize_group_policy(GroupMode::PeerToPeer)
                .unwrap();
            alice.snapshot().unwrap();
            alice.publish_group_policy().unwrap();
        }
        let device = MlsDevice::generate().unwrap();
        let invite = alice.mint_invite([7; 16], 60_000, vec![]).unwrap();
        let package = device
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let response = alice
            .serve_join(
                PeerId::from_u64(2),
                &encode_join_req(&invite, &serialize_key_package(&package).unwrap()),
            )
            .unwrap();
        let (welcome, signature, transfer) = decode_join_resp(&response[1..]).unwrap();
        let (group, routing) =
            finish_join(&device, &invite, &welcome, &signature, &transfer).unwrap();
        let mut bob = Node::new_joined(
            hub.join(PeerId::from_u64(2)),
            group,
            device,
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            routing,
        );
        alice.publish_self_record(Vec::new(), 65_536).unwrap();
        bob.publish_self_record(Vec::new(), 65_536).unwrap();
        (alice, bob)
    }

    #[tokio::test]
    async fn finalization_binds_both_members_and_retains_descriptors_across_restore() {
        let (mut alice, mut bob) = pair(true);
        let peer = alice.local_peer();
        let (done, ()) = tokio::join!(bob.finalize_member_connection(peer), async {
            loop {
                if let Some(TransportEvent::Request {
                    from,
                    data,
                    responder,
                    ..
                }) = alice.transport.next_event().await
                {
                    responder.respond(Bytes::from(alice.handle_request(from, &data)));
                    break;
                }
            }
        });
        assert!(done.unwrap());
        assert_eq!(alice.finalized_member_peers(), vec![bob.local_peer()]);
        assert_eq!(bob.finalized_member_peers(), vec![alice.local_peer()]);
        for mut node in [alice, bob] {
            let peer = node.local_peer();
            let restored = Node::restore(
                &node.snapshot().unwrap(),
                Hub::new().join(peer),
                ChaCha20Rng::seed_from_u64(3),
                Box::new(ManualClock::new(2_000)),
            )
            .unwrap();
            assert_eq!(restored.group_mode(), GroupMode::PeerToPeer);
            assert!(restored.member_routes()[0].peer_id.is_some());
            assert!(
                restored.finalized_member_peers().is_empty(),
                "a stored descriptor is not a fresh endpoint proof"
            );
        }
    }

    #[test]
    fn finalization_refuses_legacy_policy_cross_endpoint_and_policy_substitution() {
        let (mut alice, mut bob) = pair(true);
        let policy = alice.group_policy_digest().unwrap();
        let own = bob.self_record().unwrap().clone();
        for (digest, target, source) in [
            ([8; 32], alice.local_peer(), bob.local_peer()),
            (policy, PeerId::from_u64(99), bob.local_peer()),
            (policy, alice.local_peer(), PeerId::from_u64(99)),
        ] {
            let inner = body(digest, bob.local_peer(), target, &own);
            let (request, _) = bob
                .build_authed_request(KIND_MEMBER_FINALIZE, &inner)
                .unwrap();
            assert!(alice
                .serve_member_finalization(source, &request[1..])
                .is_none());
            assert!(alice.finalized_member_peers().is_empty());
        }
        let (mut legacy, _) = pair(false);
        let inner = body(policy, bob.local_peer(), legacy.local_peer(), &own);
        let (request, _) = bob
            .build_authed_request(KIND_MEMBER_FINALIZE, &inner)
            .unwrap();
        assert!(legacy
            .serve_member_finalization(bob.local_peer(), &request[1..])
            .is_none());
    }

    #[tokio::test]
    async fn locally_removed_member_keeps_mode_but_loses_reconnect_and_serving_authority() {
        let (mut alice, mut bob) = pair(true);
        let alice_peer = alice.local_peer();
        let alice_id = alice.device.device_id();
        let bob_id = bob.device.device_id();
        bob.ingest_peer_record(alice.self_record().unwrap().clone());
        bob.promote_member_peer_bound(alice_peer, alice_id, true);
        assert!(bob.policy_allows_member_mesh());
        assert_eq!(bob.finalized_member_peers(), vec![alice_peer]);
        alice.request_remove(&bob_id).await.unwrap();
        let removed = alice.commit_log.back().unwrap();
        let mut control = vec![CTRL_COMMIT];
        control.extend_from_slice(&removed.encode());
        bob.on_control(alice_peer, &control);
        assert!(
            !bob.group.is_active(),
            "actual signed local removal reached MLS"
        );
        assert_eq!(
            bob.group_mode(),
            GroupMode::PeerToPeer,
            "mode is historical policy, not current permission"
        );
        assert!(!bob.policy_allows_member_mesh());
        assert!(!bob.policy_allows_service());
        assert!(bob.finalized_member_peers().is_empty());
        assert!(!bob.finalize_member_connection(alice_peer).await.unwrap());
        assert_eq!(bob.dial_local_reconnect_routes().await, 0);
        assert!(bob.serve_member_finalization(alice_peer, &[]).is_none());
    }

    #[tokio::test]
    async fn admission_candidates_do_not_become_proof_and_retire_on_descriptor_or_removal() {
        let (mut alice, bob) = pair(true);
        let peer = bob.local_peer();
        assert_eq!(alice.member_finalization_candidates(), vec![peer]);
        assert!(alice.finalized_member_peers().is_empty());
        let outsider = PeerId::from_u64(99);
        alice.note_member_finalization_candidate(outsider, DeviceId::from_bytes([99; 32]));
        assert_eq!(alice.member_finalization_candidates(), vec![peer]);
        let mut descriptor = bob.self_record().unwrap().clone();
        descriptor.peer_id = *outsider.as_bytes();
        descriptor.signature = bob
            .device
            .sign(&peer_record_signing_payload(
                &descriptor.device_pubkey,
                &descriptor.peer_id,
                &descriptor.addresses,
                descriptor.seq,
            ))
            .unwrap();
        assert!(alice.ingest_peer_record(descriptor));
        assert_eq!(alice.member_finalization_candidates(), vec![outsider]);
        assert!(
            alice.member_finalization_pending.is_empty(),
            "contradictory signed descriptor retires the admission candidate"
        );
        alice.request_remove(&bob.device.device_id()).await.unwrap();
        assert!(alice.member_finalization_candidates().is_empty());
        alice.note_member_finalization_candidate(peer, bob.device.device_id());
        assert!(alice.member_finalization_pending.is_empty());
    }

    #[test]
    fn full_descriptor_shape_fits_both_authenticated_finalization_frames() {
        let (alice, mut bob) = pair(true);
        let mut largest = bob.self_record().unwrap().clone();
        // This intentionally overestimates real valid public multiaddrs: test the full codec
        // limit, rather than assuming typical addresses stay short. Signing keys are Ed25519.
        assert_eq!(largest.device_pubkey.len(), 32);
        largest.addresses = vec!["x".repeat(MAX_PEX_ADDR_LEN); MAX_PEX_ADDRESSES];
        assert_eq!(largest.encode().len(), 2_232);
        let inner = body(
            bob.group_policy_digest().unwrap(),
            bob.local_peer(),
            alice.local_peer(),
            &largest,
        );
        let (request, _) = bob
            .build_authed_request(KIND_MEMBER_FINALIZE, &inner)
            .unwrap();
        let mut answer = blake3::hash(&inner).as_bytes().to_vec();
        answer.extend_from_slice(&inner);
        let response = encode_signed_commit_resp(&bob.device.public_key_bytes(), &[0; 64], &answer);
        assert_eq!(request.len(), 2_490);
        assert_eq!(response.len(), 2_485);
        assert!(request.len() <= MAX_FRAME, "max request: {}", request.len());
        assert!(
            response.len() <= MAX_FRAME,
            "max response: {}",
            response.len()
        );
    }
}
