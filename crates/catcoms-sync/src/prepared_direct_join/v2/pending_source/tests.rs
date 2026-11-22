use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::join_bindings::InviteBindingSpecData;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn v2_pending_join_keeps_the_original_private_package_and_pin() {
    let founder = DeviceKeypair::from_seed(&[31; 32]);
    let anchor = GenesisAnchorBindingData {
        group: vec![32; 16],
        founder: founder.verifying_key().to_bytes(),
        profile: [33; 32],
        admission: [34; 32],
        tenure: [35; 32],
        header: [36; 32],
        state: [37; 32],
    };
    let invite = SignedInviteBindingData::sign_using(
        InviteBindingSpecData {
            anchor: anchor.clone(),
            inviter: anchor.founder,
            nonce: [38; 16],
            expires_at_ms: 100,
            policy: [39; 32],
            public_routing_plan: [40; 32],
        },
        |bytes| Ok(founder.sign(bytes)),
    )
    .unwrap();
    let limits = DirectJoinLimits::new(512 * 1024, 512 * 1024, 256 * 1024).unwrap();
    let mut join = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        &anchor,
        &invite,
        [41; 32],
        90,
        limits,
    )
    .unwrap();
    let original = join.request_bytes_data().to_vec();
    let key = [42; 32];
    let expected = V2PendingJoinContextData {
        vault: [43; 32],
        root: [44; 32],
        original_pending: [45; 32],
        pin: [46; 32],
    };
    let wrong = V2PendingJoinContextData {
        pin: [47; 32],
        ..expected
    };
    let mut rng = ChaCha20Rng::seed_from_u64(191);
    let sealed = join.seal_pending_source(&key, expected, &mut rng).unwrap();
    let (nonce, cipher) = sealed.encrypted_parts();
    let restored = SealedV2PendingJoinSource::from_encrypted_parts(*nonce, cipher).unwrap();
    let cipher = cipher.to_vec();
    assert_eq!(
        join.seal_pending_source(&key, expected, &mut rng)
            .unwrap()
            .encrypted_parts()
            .1,
        cipher
    );
    assert!(join.seal_pending_source(&key, wrong, &mut rng).is_err());
    let recovered =
        RecoveredV2PendingJoinSource::open(&restored, &key, expected, &invite, limits).unwrap();
    assert_eq!(recovered.request_bytes_data(), original);
    assert!(RecoveredV2PendingJoinSource::open(&restored, &key, wrong, &invite, limits).is_err());
    let mut wrong_invite = invite.spec_data().clone();
    wrong_invite.nonce = [48; 16];
    let wrong_invite =
        SignedInviteBindingData::sign_using(wrong_invite, |bytes| Ok(founder.sign(bytes))).unwrap();
    assert!(
        RecoveredV2PendingJoinSource::open(&restored, &key, expected, &wrong_invite, limits)
            .is_err()
    );
}
