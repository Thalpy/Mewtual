use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::join_bindings::InviteBindingSpecData;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_joined_pin_keeps_the_original_anchor_ciphertext_and_pending_provenance() {
    let founder = DeviceKeypair::from_seed(&[7; 32]);
    let anchor = GenesisAnchorBindingData {
        group: vec![3; 16],
        founder: founder.verifying_key().to_bytes(),
        profile: [4; 32],
        admission: [5; 32],
        tenure: [6; 32],
        header: [7; 32],
        state: [8; 32],
    };
    let invite = SignedInviteBindingData::sign_using(
        InviteBindingSpecData {
            anchor: anchor.clone(),
            inviter: anchor.founder,
            nonce: [9; 16],
            expires_at_ms: 800,
            policy: [10; 32],
            public_routing_plan: [11; 32],
        },
        |bytes| Ok(founder.sign(bytes)),
    )
    .unwrap();
    let layout = CurrentLayoutData {
        vault: [12; 32],
        root: [13; 32],
    };
    let pending = [14; 32];
    let key = [15; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(169);
    let raw = seal_initial(&key, layout, pending, &anchor, &invite, &mut rng).unwrap();
    let id = V2GroupPinId::of(&raw);
    let pin = open_original(&key, layout, id, &anchor, &raw).unwrap();
    assert_eq!(pin.anchor_data(), &anchor);
    assert_eq!(pin.accepted_invite.encode(), invite.encode());
    let bound = pin.pending_context_data();
    assert_eq!(bound.pin, *id.as_bytes());
    assert_eq!(bound.original_pending, pending);
    assert_eq!(bound.root, layout.root);
    let limits = catcoms_sync::DirectJoinLimits::new(512 * 1024, 512 * 1024, 256 * 1024).unwrap();
    let prepare = |invite: &SignedInviteBindingData| {
        catcoms_sync::PreparedV2JoinRequest::prepare(
            catcoms_mls::MlsDevice::generate().unwrap(),
            &anchor,
            invite,
            [20; 32],
            700,
            limits,
        )
        .unwrap()
    };
    let mut exact = prepare(&invite);
    let sealed = exact.seal_pending_source(&key, bound, &mut rng).unwrap();
    let recovered = pin.open_pending_source(&key, sealed, limits).unwrap();
    assert_eq!(recovered.request_bytes_data(), exact.request_bytes_data());
    let mut another = invite.spec_data().clone();
    another.nonce = [19; 16];
    let another =
        SignedInviteBindingData::sign_using(another, |bytes| Ok(founder.sign(bytes))).unwrap();
    let mut wrong = prepare(&another);
    let wrong = wrong.seal_pending_source(&key, bound, &mut rng).unwrap();
    assert!(pin.open_pending_source(&key, wrong, limits).is_err());
    assert!(open_original(
        &key,
        CurrentLayoutData {
            root: [16; 32],
            ..layout
        },
        id,
        &anchor,
        &raw
    )
    .is_err());
    assert!(open_original(
        &key,
        CurrentLayoutData {
            vault: [17; 32],
            ..layout
        },
        id,
        &anchor,
        &raw
    )
    .is_err());
    assert!(open_original(&key, layout, id, &anchor, &raw[..raw.len() - 1]).is_err());
    let resealed = seal_initial(&key, layout, pending, &anchor, &invite, &mut rng).unwrap();
    assert!(open_original(&key, layout, id, &anchor, &resealed).is_err());
    assert!(seal_initial(&key, layout, [0; 32], &anchor, &invite, &mut rng).is_err());
    let other = GenesisAnchorBindingData {
        state: [18; 32],
        ..anchor
    };
    assert!(seal_initial(&key, layout, pending, &other, &invite, &mut rng).is_err());
    assert!(open_original(&key, layout, id, &other, &raw).is_err());
}
