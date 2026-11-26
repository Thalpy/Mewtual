use super::*;
use catcoms_crypto::DeviceKeypair;

fn fixture() -> (GenesisAnchorBindingData, SignedInviteBindingData) {
    let founder = DeviceKeypair::from_seed(&[3; 32]);
    let anchor = GenesisAnchorBindingData {
        group: vec![4; 16],
        founder: founder.verifying_key().to_bytes(),
        profile: [5; 32],
        admission: [6; 32],
        tenure: [7; 32],
        header: [8; 32],
        state: [9; 32],
    };
    let invite = SignedInviteBindingData::sign_using(
        catcoms_replication::chat::join_bindings::InviteBindingSpecData {
            anchor: anchor.clone(),
            inviter: anchor.founder,
            nonce: [10; 16],
            expires_at_ms: 999,
            policy: [11; 32],
            public_routing_plan: [12; 32],
        },
        |bytes| Ok(founder.sign(bytes)),
    )
    .unwrap();
    (anchor, invite)
}

fn limits() -> DirectJoinLimits {
    DirectJoinLimits::new(512 * 1024, 512 * 1024, MAX_V2_JOIN_REQUEST_PACKET_BYTES).unwrap()
}

#[test]
fn v2_join_request_retains_the_original_private_bundle_and_both_package_commitments() {
    let (anchor, invite) = fixture();
    let joiner = MlsDevice::generate().unwrap();
    let joiner_key = joiner.public_key_bytes();
    let prepared =
        PreparedV2JoinRequest::prepare(joiner, &anchor, &invite, [21; 32], 998, limits()).unwrap();
    let bytes = prepared.request_bytes_data().to_vec();
    let packet = V2JoinRequestPacketData::decode(&bytes).unwrap();
    let data = packet.founder_add_data();
    let spec = data.request.spec_data();
    assert_eq!(spec.joiner.as_slice(), joiner_key.as_slice());
    assert_eq!(spec.request_token, [21; 32]);
    assert_eq!(spec.invite, invite.id_data());
    let receiver = MlsDevice::generate().unwrap();
    let package = receiver.parse_key_package(data.key_package).unwrap();
    let identity = receiver.key_package_identity(&package).unwrap();
    assert_eq!(
        &spec.key_package_reference,
        identity.key_package_reference()
    );
    assert_eq!(&spec.credential, identity.credential_digest());
    assert_ne!(spec.key_package_reference, spec.key_package_wire);
    assert_eq!(
        spec.key_package_wire,
        JoinPayloadKindData::KeyPackageWire
            .commitment_data(data.key_package)
            .unwrap()
    );
    prepared.validate_retained_package_data().unwrap();
    prepared.validate_retained_package_data().unwrap();
    assert_eq!(prepared.request_bytes_data(), bytes.as_slice());
}

#[test]
fn v2_join_request_requires_the_accepted_anchor_nonzero_token_and_unexpired_invite() {
    let (anchor, invite) = fixture();
    let mut other = anchor.clone();
    other.header[0] ^= 1;
    for (pin, token, now) in [
        (&other, [21; 32], 998),
        (&anchor, [0; 32], 998),
        (&anchor, [21; 32], 999),
    ] {
        assert!(matches!(
            PreparedV2JoinRequest::prepare(
                MlsDevice::generate().unwrap(),
                pin,
                &invite,
                token,
                now,
                limits(),
            ),
            Err(SyncError::JoinRejected)
        ));
    }
}

#[test]
fn v2_join_request_packet_refuses_partial_mixed_or_forged_representations() {
    let (anchor, invite) = fixture();
    let prepared = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        &anchor,
        &invite,
        [21; 32],
        998,
        limits(),
    )
    .unwrap();
    let bytes = prepared.request_bytes_data();
    for length in 0..bytes.len() {
        assert!(V2JoinRequestPacketData::decode(&bytes[..length]).is_err());
    }
    let mut trailer = bytes.to_vec();
    trailer.push(0);
    assert!(V2JoinRequestPacketData::decode(&trailer).is_err());
    let mut next = bytes.to_vec();
    next[4 + TAG.len() + 1] ^= 1;
    assert!(V2JoinRequestPacketData::decode(&next).is_err());

    let packet = V2JoinRequestPacketData::decode(bytes).unwrap();
    let data = packet.founder_add_data();
    let original_invite = data.invite.encode();
    let original_request = data.request.encode();
    let mut invite_bytes = original_invite.clone();
    let mut request = original_request.clone();
    let mut key_package = data.key_package.to_vec();
    *invite_bytes.last_mut().unwrap() ^= 1;
    *request.last_mut().unwrap() ^= 1;
    *key_package.last_mut().unwrap() ^= 1;
    for (i, r, kp) in [
        (
            invite_bytes.as_slice(),
            original_request.as_slice(),
            data.key_package,
        ),
        (
            original_invite.as_slice(),
            request.as_slice(),
            data.key_package,
        ),
        (
            original_invite.as_slice(),
            original_request.as_slice(),
            key_package.as_slice(),
        ),
    ] {
        let wire = encode(i, r, kp, MAX_V2_JOIN_REQUEST_PACKET_BYTES).unwrap();
        assert!(matches!(
            V2JoinRequestPacketData::decode(&wire),
            Err(SyncError::JoinRejected)
        ));
    }
}
