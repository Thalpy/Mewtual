use super::*;
use catcoms_mls::{InviteToken, MlsDevice, ServerGroup};
use catcoms_net::seeded_transport_identity_data;
use catcoms_replication::chat::join_bindings::{
    GenesisAnchorBindingData, InviteBindingSpecData, JoinPayloadKindData,
};

struct Founder {
    device: MlsDevice,
    group: ServerGroup,
    public: [u8; 32],
    reservation: Vec<u8>,
    policy: Vec<u8>,
    transport: SeededTransportIdentityData,
}

impl Founder {
    fn new() -> Self {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let public = device.public_key_bytes().try_into().unwrap();
        let policy = GroupPolicy::issue(&group, &device, GroupMode::PeerToPeer)
            .unwrap()
            .encode();
        let transport = seeded_transport_identity_data(&[21; 32]).unwrap();
        let reservation = encode_reservation(&group.group_id(), &transport, 1234).unwrap();
        Self {
            device,
            group,
            public,
            reservation,
            policy,
            transport,
        }
    }

    fn binding(&self, signer: &MlsDevice, policy: &[u8], plan: &[u8]) -> SignedInviteBindingData {
        let inviter = signer.public_key_bytes().try_into().unwrap();
        SignedInviteBindingData::sign_using(
            InviteBindingSpecData {
                anchor: GenesisAnchorBindingData {
                    group: self.group.group_id(),
                    founder: self.public,
                    profile: [1; 32],
                    admission: [2; 32],
                    tenure: [3; 32],
                    header: [4; 32],
                    state: [5; 32],
                },
                inviter,
                nonce: [6; 16],
                expires_at_ms: 1000,
                policy: JoinPayloadKindData::Policy.commitment_data(policy).unwrap(),
                public_routing_plan: JoinPayloadKindData::RoutingPlan
                    .commitment_data(plan)
                    .unwrap(),
            },
            |bytes| {
                signer
                    .sign_retained_ed25519_bounded(&inviter, bytes, MAX_JOIN_BINDING_BYTES)
                    .map_err(|_| ChatError::Signature)
            },
        )
        .unwrap()
    }

    fn packet(&self, signer: &MlsDevice, policy: &[u8], plan: &[u8]) -> Vec<u8> {
        encode_packet(&self.binding(signer, policy, plan).encode(), policy, plan).unwrap()
    }
}

#[test]
fn native_invite_packet_requires_exact_complete_versioned_envelope() {
    let f = Founder::new();
    let binding = f.binding(&f.device, &f.policy, &f.reservation);
    let packet = f.packet(&f.device, &f.policy, &f.reservation);
    let opened = NativeTextInvitePacketData::decode(&packet).unwrap();
    assert_eq!(opened.binding().id_data(), binding.id_data());
    assert_eq!(opened.policy_bytes(), f.policy.as_slice());
    assert_eq!(opened.reservation_bytes(), f.reservation.as_slice());
    assert!(InviteToken::decode(&packet).is_err());
    assert!(NativeTextInvitePacketData::decode(&binding.encode()).is_err());
    for len in 0..packet.len() {
        assert!(NativeTextInvitePacketData::decode(&packet[..len]).is_err());
    }
    let mut version = packet.clone();
    version[4 + PACKET.len() + 1] ^= 1;
    assert!(NativeTextInvitePacketData::decode(&version).is_err());
    let mut trailer = packet.clone();
    trailer.push(0);
    assert!(NativeTextInvitePacketData::decode(&trailer).is_err());
    assert_eq!(
        NativeTextInvitePacketData::decode(&vec![0; MAX_NATIVE_TEXT_INVITE_PACKET_BYTES + 1]).err(),
        Some(ChatError::Bound)
    );
}

#[test]
fn native_invite_packet_rejects_signature_and_both_body_substitutions() {
    let f = Founder::new();
    let signed = f.binding(&f.device, &f.policy, &f.reservation);
    let mut changed = signed.encode();
    *changed.last_mut().unwrap() ^= 1;
    let decoded = SignedInviteBindingData::decode(&changed).unwrap();
    assert_eq!(decoded.id_data(), signed.id_data());
    let packet = encode_packet(&changed, &f.policy, &f.reservation).unwrap();
    assert_eq!(
        NativeTextInvitePacketData::decode(&packet).err(),
        Some(ChatError::Signature)
    );

    let mut other_policy = f.policy.clone();
    other_policy[0] ^= 1;
    let other_plan = encode_reservation(&f.group.group_id(), &f.transport, 1235).unwrap();
    for (policy, plan) in [
        (other_policy.as_slice(), f.reservation.as_slice()),
        (f.policy.as_slice(), other_plan.as_slice()),
    ] {
        let packet = encode_packet(&signed.encode(), policy, plan).unwrap();
        assert_eq!(
            NativeTextInvitePacketData::decode(&packet).err(),
            Some(ChatError::Scope)
        );
    }
}

#[test]
fn native_invite_packet_refuses_coherently_signed_foreign_policy_and_transport_data() {
    let f = Founder::new();
    let other = Founder::new();
    let dedicated = GroupPolicy::issue(&f.group, &f.device, GroupMode::Dedicated)
        .unwrap()
        .encode();
    for (signer, policy, plan) in [
        (&other.device, f.policy.as_slice(), f.reservation.as_slice()),
        (&f.device, other.policy.as_slice(), f.reservation.as_slice()),
        (&f.device, dedicated.as_slice(), f.reservation.as_slice()),
        (&f.device, f.policy.as_slice(), other.reservation.as_slice()),
    ] {
        let packet = f.packet(signer, policy, plan);
        assert_eq!(
            NativeTextInvitePacketData::decode(&packet).err(),
            Some(ChatError::Scope)
        );
    }

    let mut inconsistent = f.reservation.clone();
    let peer = f.transport.peer_id_data().as_bytes();
    let offset = inconsistent
        .windows(peer.len())
        .position(|bytes| bytes == peer)
        .unwrap();
    inconsistent[offset] ^= 1;
    let packet = f.packet(&f.device, &f.policy, &inconsistent);
    assert_eq!(
        NativeTextInvitePacketData::decode(&packet).err(),
        Some(ChatError::Scope)
    );

    let mut inconsistent = f.reservation.clone();
    let commitment = f.transport.commitment_data();
    let offset = inconsistent
        .windows(commitment.len())
        .position(|bytes| bytes == commitment)
        .unwrap();
    inconsistent[offset] ^= 1;
    let packet = f.packet(&f.device, &f.policy, &inconsistent);
    assert_eq!(
        NativeTextInvitePacketData::decode(&packet).err(),
        Some(ChatError::Scope)
    );

    let mut contact_plan = f.reservation.clone();
    *contact_plan.last_mut().unwrap() = 1;
    let packet = f.packet(&f.device, &f.policy, &contact_plan);
    assert_eq!(
        NativeTextInvitePacketData::decode(&packet).err(),
        Some(ChatError::Malformed)
    );
}
