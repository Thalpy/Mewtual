use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_mls::MlsDevice;
use catcoms_replication::chat::join_bindings::{
    GenesisAnchorBindingData, InviteBindingSpecData, SignedInviteBindingData,
};
use catcoms_sync::{DirectJoinLimits, PreparedV2JoinRequest};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [71; 32];

fn request(token: u8) -> PreparedV2JoinRequest {
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
        InviteBindingSpecData {
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
    PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        &anchor,
        &invite,
        [token; 32],
        998,
        DirectJoinLimits::new(512 * 1024, 512 * 1024, 64 * 1024).unwrap(),
    )
    .unwrap()
}

#[test]
fn governed_add_fence_authenticates_exact_request_and_parent() {
    let context = TextContextIdData::from_bytes([13; 32]);
    let parent = TextHeadIdData::from_bytes([14; 32]);
    let original = request(15);
    let wire = original.request_bytes_data();
    let packet = V2JoinRequestPacketData::decode(wire).unwrap();
    let data = GovernedAddFenceData::for_request(context, parent, &packet);
    assert_eq!(data.request, blake3::derive_key(REQUEST_DOMAIN, wire));
    assert_eq!(data.invite, packet.founder_add_data().invite.id_data());
    let raw = data
        .seal_data(&KEY, &mut ChaCha20Rng::seed_from_u64(72))
        .unwrap();
    let opened = GovernedAddFenceData::open_data(&KEY, &raw).unwrap();
    assert!(opened == data);
    assert!(
        opened
            != GovernedAddFenceData::for_request(
                TextContextIdData::from_bytes([16; 32]),
                parent,
                &packet
            )
    );
    assert!(
        opened
            != GovernedAddFenceData::for_request(
                context,
                TextHeadIdData::from_bytes([16; 32]),
                &packet
            )
    );

    let second = request(16);
    let packet = V2JoinRequestPacketData::decode(second.request_bytes_data()).unwrap();
    let changed = GovernedAddFenceData::for_request(context, parent, &packet);
    assert_eq!(changed.invite, data.invite);
    assert!(opened != changed);
}

#[test]
fn governed_add_request_record_preserves_only_the_original_packet() {
    let original = request(37);
    let packet = V2JoinRequestPacketData::decode(original.request_bytes_data()).unwrap();
    let data = GovernedAddFenceData::for_request(
        TextContextIdData::from_bytes([41; 32]),
        TextHeadIdData::from_bytes([42; 32]),
        &packet,
    );
    let mut rng = ChaCha20Rng::seed_from_u64(160);
    let fence = data.seal_data(&KEY, &mut rng).unwrap();
    let raw = super::request::seal_data(&KEY, &fence, &packet, &mut rng).unwrap();
    assert_eq!(
        super::request::open_data(&KEY, &fence, &raw)
            .unwrap()
            .as_slice(),
        original.request_bytes_data()
    );
    let resealed = data.seal_data(&KEY, &mut rng).unwrap();
    assert!(super::request::open_data(&KEY, &resealed, &raw).is_err());
    assert!(super::request::open_data(&KEY, &fence, &raw[..raw.len() - 1]).is_err());
    assert!(super::request::open_record(&KEY, &fence, &raw)
        .unwrap()
        .fetch_verifier
        .is_none());
    let verifier = [43; 32];
    let later =
        super::request::seal_with_fetch_verifier(&KEY, &fence, &packet, Some(verifier), &mut rng)
            .unwrap();
    let reopened = super::request::open_record(&KEY, &fence, &later).unwrap();
    assert_eq!(reopened.packet.as_slice(), original.request_bytes_data());
    assert_eq!(reopened.fetch_verifier, Some(verifier));
    assert!(super::request::open_record(&KEY, &resealed, &later).is_err());
    assert!(super::request::seal_with_fetch_verifier(
        &KEY,
        &fence,
        &packet,
        Some([0; 32]),
        &mut rng,
    )
    .is_err());
    let replacement = request(38);
    let replacement = V2JoinRequestPacketData::decode(replacement.request_bytes_data()).unwrap();
    assert!(super::request::seal_data(&KEY, &fence, &replacement, &mut rng).is_err());
}

#[test]
fn governed_add_fence_preserves_old_wal_authentication_and_refusal() {
    let kind = TextControlKindData::Wal;
    let mut rng = ChaCha20Rng::seed_from_u64(73);
    let data = GovernedAddFenceData {
        context: TextContextIdData::from_bytes([17; 32]),
        parent: TextHeadIdData::from_bytes([18; 32]),
        invite: [19; 32],
        request: [20; 32],
    };
    let raw = data.seal_data(&KEY, &mut rng).unwrap();
    if let Some(path) = std::env::var_os("MEWTUAL_GOVERNED_ADD_FENCE_FIXTURE") {
        std::fs::write(path, raw.as_slice()).unwrap();
    }
    assert!(matches!(
        inspect_seed_stage_mac_data(&KEY, &raw).unwrap(),
        TextControlMacInspectionData::Authenticated
    ));
    assert!(matches!(
        inspect_known_control_mac_data(&KEY, &raw).unwrap(),
        TextControlMacInspectionData::Authenticated
    ));
    assert!(open_control_data(&KEY, kind.id_data(&raw), &raw).is_err());
    let mut plain = unseal(
        &GovernedAddFenceData::wal_key(&KEY),
        &unframe(&raw).unwrap(),
    )
    .unwrap();
    assert_eq!(plain.len(), PLAIN_BYTES);
    plain[4 + WAL_TAG.len() + 1] = 1;
    let v1 = PreparedTextControlFrameData::prepare_data(kind, &KEY, &plain, &mut rng).unwrap();
    assert_eq!(
        open_control_data(&KEY, v1.id_data(), v1.frame_data())
            .unwrap()
            .as_slice(),
        plain.as_slice()
    );
    assert!(GovernedAddFenceData::open_data(&KEY, v1.frame_data()).is_err());

    plain[4 + WAL_TAG.len() + 1] = FENCE_VERSION as u8;
    plain[4 + WAL_TAG.len() + 2 + 4] ^= 1;
    let other_role = frame(&seal(&GovernedAddFenceData::wal_key(&KEY), &plain, &mut rng).unwrap());
    assert!(GovernedAddFenceData::open_data(&KEY, &other_role).is_err());
    assert!(GovernedAddFenceData::open_data(&[0; 32], &raw).is_err());
    for length in 0..raw.len() {
        assert!(GovernedAddFenceData::open_data(&KEY, &raw[..length]).is_err());
    }
    let mut corrupt = raw.to_vec();
    corrupt[FRAME_BYTES - 1] ^= 1;
    assert!(GovernedAddFenceData::open_data(&KEY, &corrupt).is_err());
    let mut trailer = raw.to_vec();
    trailer.push(0);
    assert!(GovernedAddFenceData::open_data(&KEY, &trailer).is_err());
}
