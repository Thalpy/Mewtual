use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::join_bindings::InviteBindingSpecData;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_joined_pin_index_keeps_first_ciphertexts_across_sorted_insertions() {
    use index::JoinedPinIndex;
    let founder = DeviceKeypair::from_seed(&[71; 32]);
    let make_invite = |group: u8| {
        let signer = founder.verifying_key().to_bytes();
        SignedInviteBindingData::sign_using(
            InviteBindingSpecData {
                anchor: GenesisAnchorBindingData {
                    group: vec![group; 16],
                    founder: signer,
                    profile: [73; 32],
                    admission: [74; 32],
                    tenure: [75; 32],
                    header: [76; 32],
                    state: [77; 32],
                },
                inviter: signer,
                nonce: [group; 16],
                expires_at_ms: 800,
                policy: [78; 32],
                public_routing_plan: [79; 32],
            },
            |bytes| Ok(founder.sign(bytes)),
        )
        .unwrap()
    };
    let first_invite = make_invite(82);
    let second_invite = make_invite(81);
    let first_anchor = &first_invite.spec_data().anchor;
    let layout = CurrentLayoutData {
        vault: [83; 32],
        root: [84; 32],
    };
    let key = [85; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(87);
    let empty = JoinedPinIndex::seal_initial(&key, layout, &mut rng).unwrap();
    let empty = JoinedPinIndex::open(&key, layout, &empty).unwrap();
    assert!(empty.is_empty());
    assert!(empty.lookup(first_anchor).unwrap().is_none());
    let first_frame = empty
        .prepare_insert(&key, &first_invite, [86; 32], &mut rng)
        .unwrap();
    let first = JoinedPinIndex::open(&key, layout, &first_frame).unwrap();
    empty.require_successor(&first).unwrap();
    let original = first
        .lookup(first_anchor)
        .unwrap()
        .unwrap()
        .pending_context_data();
    assert!(first
        .prepare_insert(&key, &first_invite, [88; 32], &mut rng)
        .is_err());
    let second_frame = first
        .prepare_insert(&key, &second_invite, [89; 32], &mut rng)
        .unwrap();
    let second = JoinedPinIndex::open(&key, layout, &second_frame).unwrap();
    first.require_successor(&second).unwrap();
    assert!(empty.require_successor(&second).is_err());
    assert!(second
        .lookup(first_anchor)
        .unwrap()
        .is_some_and(|pin| pin.pending_context_data() == original));
    let mut other_anchor = first_anchor.clone();
    other_anchor.header[0] ^= 1;
    assert!(second.lookup(&other_anchor).is_err());
    let mut other_layout = layout;
    other_layout.root[0] ^= 1;
    assert!(JoinedPinIndex::open(&key, other_layout, &second_frame).is_err());
    assert!(JoinedPinIndex::open(&key, layout, &second_frame[..second_frame.len() - 1]).is_err());
}

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
    let original_index = index::JoinedPinIndex::seal_initial(&key, layout, &mut rng).unwrap();
    assert!(pending::PreparedPinnedPending::prepare(
        prepare(&another),
        &original_index,
        &invite,
        &key,
        layout,
        (pending, None),
        &mut rng,
    )
    .is_err());
    let source = prepare(&invite);
    let original_request = source.request_bytes_data().to_vec();
    let mut first = pending::PreparedPinnedPending::prepare(
        source,
        &original_index,
        &invite,
        &key,
        layout,
        (pending, None),
        &mut rng,
    )
    .unwrap();
    let first_frame = first.seal(&key, &mut rng).unwrap().to_vec();
    assert_eq!(first.seal(&key, &mut rng).unwrap(), first_frame);
    assert!(first.seal(&[21; 32], &mut rng).is_err());
    let reopened =
        pending::RecoveredPinnedPending::open(&key, layout, &first_frame, limits).unwrap();
    let (before, after) = reopened.index_frames();
    assert_eq!(before, original_index.as_slice());
    assert_ne!(before, after);
    let (private, context) = reopened.pending_data();
    assert_eq!(private.request_bytes_data(), original_request);
    assert_eq!(context.original_pending, pending);
    assert_ne!(context.pin, bound.pin);
    assert!(pending::RecoveredPinnedPending::open(
        &key,
        layout,
        &first_frame[..first_frame.len() - 1],
        limits
    )
    .is_err());
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
