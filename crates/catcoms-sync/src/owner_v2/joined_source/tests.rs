//! Real MLS invite/companion joins through the actual `finish_join`/`finish_device_join` and
//! `new_joined` paths. These pin the join-FACTS custody only: exact capture, exact handoff,
//! one-time take by the original incarnation, and no facts on any other node kind. Nothing
//! here exercises pin provenance, replay, selection or signing.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;

fn inviter(peer: u64) -> Node {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    ChannelSync::new(
        Hub::new().join(PeerId::from_u64(peer)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(1_000)),
    )
}

/// One genuine invite join: the inviter's real Add/Welcome and the joiner's real finish.
fn invite_join(
    inviter: &mut Node,
    joiner: &MlsDevice,
    nonce: u8,
) -> (ServerGroup, RoutingState, [u8; 32]) {
    let invite = inviter
        .mint_invite([nonce; 16], 10_000, Vec::new())
        .unwrap();
    let package = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = joiner.key_package_reference(&package).unwrap();
    let (welcome, routing, signature) = inviter
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    let (group, transferred) =
        finish_join(joiner, &invite, &welcome, &signature, &routing, &expected).unwrap();
    (group, transferred, expected)
}

fn joined_node(peer: u64, group: ServerGroup, device: MlsDevice, routing: RoutingState) -> Node {
    ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(peer)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(1_000)),
        routing,
    )
}

#[test]
fn same_epoch_and_authenticator_cannot_transfer_join_facts_to_another_own_leaf() {
    let mut owner = inviter(0xc141);
    let joiner = MlsDevice::generate().unwrap();
    let (group, routing, _) = invite_join(&mut owner, &joiner, 0xcf);
    assert_eq!(owner.group.group_id(), group.group_id());
    assert_eq!(owner.group.epoch(), group.epoch());
    assert_eq!(
        owner.group.epoch_authenticator_id(),
        group.epoch_authenticator_id()
    );
    assert!(group.has_local_signer(&joiner.public_key_bytes()));
    assert!(!owner.group.has_local_signer(&joiner.public_key_bytes()));
    let facts = routing.v2_join_facts.as_ref().unwrap();
    assert!(!facts.matches_initial_state(&owner.group, &joiner));
    // Supply the actual inviter's post-Add local group with the genuine joiner device and
    // routing. All public tuple labels match; only the local own-leaf binding distinguishes it.
    let mut wrong = joined_node(0xc142, owner.group, joiner, routing);
    assert!(wrong.fresh_chat_admission().is_none());
    let instance = wrong.registry_instance();
    assert!(wrong.take_v2_joined_join_facts(&instance).is_err());
    assert!(
        !wrong.v2_fence.is_present(),
        "no open fence may survive the rejected handoff"
    );
}

#[test]
fn invite_join_captures_exact_facts_that_move_once_to_the_original_incarnation() {
    let mut inviter = inviter(0xc101);
    let joiner = MlsDevice::generate().unwrap();
    let (group, routing, expected) = invite_join(&mut inviter, &joiner, 0xc1);
    let facts = routing.v2_join_facts.as_ref().unwrap();
    assert_eq!(facts.group_data(), inviter.group_id().as_slice());
    assert_eq!(
        facts.signer_data().as_slice(),
        joiner.public_key_bytes().as_slice()
    );
    // The SAME encoding as an Add observation's KP reference and credential digest.
    assert_eq!(facts.key_package_data(), &expected);
    assert_eq!(
        facts.credential_data(),
        &group.own_credential_digest(&joiner).unwrap()
    );
    assert_eq!(facts.join_epoch_data(), group.epoch());
    assert_eq!(
        facts.admission_state_data(),
        &group.epoch_authenticator_id()
    );
    assert_eq!(format!("{facts:?}"), "V2JoinFacts { .. }");

    let mut node = joined_node(0xc102, group, joiner, routing);
    // Another live incarnation cannot take them, and its refusal moves nothing.
    let other = inviter.registry_instance();
    assert_eq!(
        node.take_v2_joined_join_facts(&other).unwrap_err(),
        V2MemberSignRefusal::Source
    );
    let instance = node.registry_instance();
    let (taken, fence) = node.take_v2_joined_join_facts(&instance).unwrap();
    assert_eq!(taken.key_package_data(), &expected);
    // The fence came with them, live and settled at the post-Welcome state.
    assert!(!fence.is_closed());
    let observed = fence.observe_data().unwrap();
    assert_eq!(observed.settled_epoch, taken.join_epoch_data());
    assert_eq!(observed.join_epoch, taken.join_epoch_data());
    assert_eq!(observed.selected_epoch, None);
    assert_eq!(
        node.take_v2_joined_join_facts(&instance).unwrap_err(),
        V2MemberSignRefusal::Source,
        "join facts move exactly once"
    );
    // The legacy fresh completion is independent and still waits for its own v1 consumer.
    assert!(node.fresh_chat_admission().is_some());
    assert!(node.require_no_v2_joined_source().is_err());
}

#[test]
fn founder_restored_and_repaired_nodes_have_no_join_facts() {
    let mut founder = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(0xc111)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xc111),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let instance = founder.registry_instance();
    assert!(founder.take_v2_joined_join_facts(&instance).is_err());
    assert!(founder.require_no_v2_joined_source().is_ok());

    // A restored copy of a genuinely joined node never has them: they are not serialized.
    let mut inviter = inviter(0xc112);
    let joiner = MlsDevice::generate().unwrap();
    let (group, routing, _) = invite_join(&mut inviter, &joiner, 0xc2);
    let mut joined = joined_node(0xc113, group, joiner, routing);
    let snapshot = joined.snapshot().unwrap();
    let mut restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(0xc114)),
        ChaCha20Rng::seed_from_u64(0xc114),
        Box::new(ManualClock::new(2_000)),
    )
    .unwrap();
    let instance = restored.registry_instance();
    assert!(restored.take_v2_joined_join_facts(&instance).is_err());

    // A public RoutingState re-paired with another real joined group/device keeps no facts.
    let first = MlsDevice::generate().unwrap();
    let second = MlsDevice::generate().unwrap();
    let (_first_group, first_routing, _) = invite_join(&mut inviter, &first, 0xc3);
    let (second_group, _second_routing, _) = invite_join(&mut inviter, &second, 0xc4);
    assert!(!first_routing
        .v2_join_facts
        .as_ref()
        .unwrap()
        .matches_initial_state(&second_group, &second));
    let mut repaired = joined_node(0xc115, second_group, second, first_routing);
    let instance = repaired.registry_instance();
    assert!(repaired.take_v2_joined_join_facts(&instance).is_err());
}

#[test]
fn companion_device_join_captures_no_join_facts() {
    let mut origin = inviter(0xc121);
    let companion = MlsDevice::generate().unwrap();
    let mut cert = DeviceCertificate {
        origin_id: origin.device.device_id(),
        origin_public_key: origin.device.public_key_bytes().try_into().unwrap(),
        new_device_id: companion.device_id(),
        group_id: origin.group_id(),
        device_name: "companion".into(),
        issued_ts_ms: 1_000,
        signature: [0; 64],
    };
    cert.signature = origin
        .device
        .sign(&DeviceCertificate::signing_payload(
            &cert.origin_id,
            &cert.origin_public_key,
            &cert.new_device_id,
            &cert.group_id,
            &cert.device_name,
            cert.issued_ts_ms,
        ))
        .unwrap();
    let package = companion
        .key_package_for_invite(&cert.group_id, device_bind_nonce(&cert))
        .unwrap();
    let expected = companion.key_package_reference(&package).unwrap();
    let (welcome, routing, signature) = origin
        .admit_device_now(&cert, &serialize_key_package(&package).unwrap())
        .unwrap();
    let owner: [u8; 32] = origin.device.public_key_bytes().try_into().unwrap();
    let (group, transferred) = finish_device_join(
        &companion, &cert, &owner, &welcome, &signature, &routing, &expected,
    )
    .unwrap();
    // The legacy companion completion is unchanged; only V2 join facts are absent.
    assert!(transferred.fresh_chat_admission.is_some());
    assert!(transferred.v2_join_facts.is_none());
    let mut node = joined_node(0xc122, group, companion, transferred);
    let instance = node.registry_instance();
    assert!(node.take_v2_joined_join_facts(&instance).is_err());
}
