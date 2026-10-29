//! A genuine old signed response must not borrow the fresh init key of a later local attempt.
use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn signed_old_welcome_with_available_old_provider_keys_refuses_a_fresh_attempt() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut node = ChannelSync::new(
        Hub::new().join(PeerId::from_u64(801)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(0x0080_1a77),
        Box::new(ManualClock::new(1_000)),
    );
    let invite = node.mint_invite([0x81; 16], 10_000, Vec::new()).unwrap();
    let joiner = MlsDevice::generate().unwrap();
    let local_group = ServerGroup::create(&joiner).unwrap();
    let old = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let old_reference = joiner.key_package_reference(&old).unwrap();
    let (welcome, routing, signature) = node
        .admit_now(&invite, &serialize_key_package(&old).unwrap(), 1_000)
        .unwrap();
    let fresh = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = joiner.key_package_reference(&fresh).unwrap();
    let snapshot = snapshot_server(&joiner, &local_group).unwrap();
    let (restored, _) = restore_server(&snapshot).unwrap();
    assert!(
        ServerGroup::join(&restored, &welcome).is_ok(),
        "old init key is genuinely available"
    );
    assert!(
        finish_join(&joiner, &invite, &welcome, &signature, &routing, &expected).is_err(),
        "a real inviter signature does not bind another local attempt"
    );
    let mut invalid_signature = signature;
    invalid_signature[0] ^= 1;
    assert!(finish_join(
        &joiner,
        &invite,
        &welcome,
        &invalid_signature,
        &routing,
        &old_reference
    )
    .is_err());
    let (joined, transferred) = finish_join(
        &joiner,
        &invite,
        &welcome,
        &signature,
        &routing,
        &old_reference,
    )
    .unwrap();
    assert_eq!(
        joined.group_id(),
        invite.group_id,
        "both refusals happened before consuming the matching init material"
    );
    let evidence = transferred.fresh_chat_admission.as_ref().unwrap();
    assert_eq!(evidence.group(), invite.group_id);
    assert_eq!(evidence.signer().as_slice(), joiner.public_key_bytes());
    assert_eq!(evidence.key_package_reference(), Some(old_reference));
    assert_eq!(
        evidence.authenticated_welcome_hash(),
        Some(*blake3::hash(&welcome).as_bytes())
    );
    assert!(
        open_routing_transfer(&joined, &joiner, &routing)
            .unwrap()
            .fresh_chat_admission
            .is_none(),
        "decoding the transfer alone cannot mint completion evidence"
    );
    let mut node = ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(802)),
        joined,
        joiner,
        ChaCha20Rng::seed_from_u64(802),
        Box::new(ManualClock::new(1_000)),
        transferred,
    );
    assert!(node.take_fresh_chat_admission().is_some());
    assert!(
        node.take_fresh_chat_admission().is_none(),
        "original completion moves exactly once"
    );
}

#[test]
fn raw_epoch_zero_restore_or_default_routing_cannot_mint_fresh_creation_or_join_evidence() {
    let device = MlsDevice::generate().unwrap();
    let mut founded = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(803)),
        device,
        ChaCha20Rng::seed_from_u64(803),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let actual = founded.take_fresh_chat_admission().unwrap();
    assert_eq!(format!("{actual:?}"), "VerifiedFreshChatAdmission { .. }");
    assert_eq!(actual.group(), founded.group_id());
    assert!(actual.key_package_reference().is_none());
    let snapshot = founded.snapshot().unwrap();
    let mut restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(804)),
        ChaCha20Rng::seed_from_u64(804),
        Box::new(ManualClock::new(2_000)),
    )
    .unwrap();
    assert_eq!(restored.epoch(), 0);
    assert!(restored.take_fresh_chat_admission().is_none());
    let mls = snapshot_server(&founded.device, &founded.group).unwrap();
    let (device, group) = restore_server(&mls).unwrap();
    let mut raw = ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(805)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(805),
        Box::new(ManualClock::new(2_000)),
        RoutingState::default(),
    );
    assert!(
        raw.take_fresh_chat_admission().is_none(),
        "raw group/default routing is not a live completion"
    );
}

#[test]
fn fresh_routing_completion_cannot_move_to_another_group_device_or_same_key_rejoin() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut node = ChannelSync::new(
        Hub::new().join(PeerId::from_u64(806)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(806),
        Box::new(ManualClock::new(1_000)),
    );
    let device = MlsDevice::generate().unwrap();
    // A genuine same-key rejoin uses fresh provider state. Capture the real identity BEFORE
    // A1 joins: duplicating afterward would also retain A1's group and OpenMLS refuses to
    // replace an already stored GroupId. Keep A1's actual group/proof for the swap assertion.
    let rejoining_device = device.duplicate().unwrap();
    assert_eq!(
        device.public_key_bytes(),
        rejoining_device.public_key_bytes()
    );
    let join = |node: &mut ChannelSync<_, _>, device: &MlsDevice, nonce: u8| {
        let invite = node.mint_invite([nonce; 16], 10_000, Vec::new()).unwrap();
        let package = device
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let expected = device.key_package_reference(&package).unwrap();
        let (welcome, routing, signature) = node
            .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
            .unwrap();
        finish_join(device, &invite, &welcome, &signature, &routing, &expected).unwrap()
    };
    let (first_group, first_routing) = join(&mut node, &device, 0x91);
    let other_device = MlsDevice::generate().unwrap();
    let other_group = ServerGroup::create(&other_device).unwrap();
    let first_proof = first_routing.fresh_chat_admission.as_ref().unwrap();
    assert!(first_proof.matches_initial_state(&first_group, &device));
    assert!(!first_proof.matches_initial_state(&first_group, &other_device));
    assert!(!first_proof.matches_initial_state(&other_group, &other_device));

    node.with_observed_mls_transition(|node| {
        node.group.remove_member(&node.device, &device.device_id())
    })
    .unwrap();
    let (second_group, second_routing) = join(&mut node, &rejoining_device, 0x92);
    assert_eq!(first_group.group_id(), second_group.group_id());
    assert_ne!(
        first_group.epoch_authenticator_id(),
        second_group.epoch_authenticator_id()
    );
    assert!(!first_proof.matches_initial_state(&second_group, &rejoining_device));
    let wrong_admission = ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(807)),
        second_group,
        rejoining_device,
        ChaCha20Rng::seed_from_u64(807),
        Box::new(ManualClock::new(1_000)),
        first_routing,
    );
    assert!(wrong_admission.fresh_chat_admission().is_none());
    let wrong_group = ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(808)),
        other_group,
        other_device,
        ChaCha20Rng::seed_from_u64(808),
        Box::new(ManualClock::new(1_000)),
        second_routing,
    );
    assert!(wrong_group.fresh_chat_admission().is_none());
}
