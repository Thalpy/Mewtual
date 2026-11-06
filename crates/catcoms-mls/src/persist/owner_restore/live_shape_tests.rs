//! Real OpenMLS state-generation requirements for a future live-current restore policy.
//!
//! These tests exercise the unchanged legacy wrappers/decoder. They do NOT establish that the
//! separate closed loader accepts this graph, that SDK work is prepaid, or that a restored node
//! has Fresh/Current/native authority. No emitted private JSON is altered or decoded into an AST.
//! Identity randomness stays inside the existing MLS constructors; assertions depend on the
//! fixed operation sequence, never generated identity bytes, wall-clock values or timings.

use crate::{restore_server, snapshot_server, Incoming, MlsDevice, ServerGroup};
use catcoms_wire::DocType;
use openmls_traits::OpenMlsProvider;

const CHANNEL: u128 = 0x431;

struct Endpoint {
    // Explicit group-before-device destruction keeps SDK graph cleanup ahead of its provider.
    group: ServerGroup,
    device: MlsDevice,
}

fn pair() -> (Endpoint, Endpoint) {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let package = bob
        .key_package_for_invite(&alice_group.group_id(), [1; 16])
        .unwrap();
    let added = alice_group.add_member(&alice, package).unwrap();
    let bob_group = ServerGroup::join(&bob, &added.welcome).unwrap();
    (
        Endpoint {
            group: alice_group,
            device: alice,
        },
        Endpoint {
            group: bob_group,
            device: bob,
        },
    )
}

fn send(endpoint: &mut Endpoint, value: u32) -> Vec<u8> {
    endpoint
        .group
        .create_application_message(&endpoint.device, &value.to_be_bytes())
        .unwrap()
}

fn receive(endpoint: &mut Endpoint, message: &[u8], expected: u32) {
    let Incoming::Application(plaintext) = endpoint
        .group
        .process_incoming(&endpoint.device, message)
        .unwrap()
    else {
        panic!("actual application input was not delivered as application data");
    };
    assert!(plaintext.as_slice() == expected.to_be_bytes());
}

fn reject(endpoint: &mut Endpoint, message: &[u8]) {
    assert!(endpoint
        .group
        .process_incoming(&endpoint.device, message)
        .is_err());
}

fn exporter(endpoint: &Endpoint) -> [u8; 32] {
    endpoint
        .group
        .channel_secret(&endpoint.device, DocType::Channel, CHANNEL)
        .unwrap()
}

/// Compare the actual full signer at every native index, rather than only a roster count.
fn roster(endpoint: &Endpoint) -> Vec<(u32, [u8; 32])> {
    let mut entries: Vec<_> = endpoint
        .group
        .member_device_ids()
        .into_iter()
        .map(|id| {
            (
                endpoint.group.member_leaf_index(&id).unwrap(),
                endpoint
                    .group
                    .member_signature_key(&id)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            )
        })
        .collect();
    entries.sort_unstable();
    entries
}

/// Drop the real old graph/provider before reconstructing the actual serialized state. This
/// is an SDK persistence check, not a filesystem durability or original-host revival claim.
fn reopen(endpoint: Endpoint) -> Endpoint {
    let expected_signer = endpoint.device.public_key_bytes();
    let expected_epoch = endpoint.group.epoch();
    let expected_roster = roster(&endpoint);
    let expected_exporter = exporter(&endpoint);
    let snapshot = snapshot_server(&endpoint.device, &endpoint.group).unwrap();
    drop(endpoint);
    let (device, group) = restore_server(&snapshot).unwrap();
    let restored = Endpoint { group, device };
    assert!(restored.device.public_key_bytes() == expected_signer);
    assert_eq!(restored.group.epoch(), expected_epoch);
    assert!(roster(&restored) == expected_roster);
    assert!(exporter(&restored) == expected_exporter);
    restored
}

/// Bounded genuine-row observation only. A borrowed byte needle avoids a test-only JSON AST,
/// copies of private rows and accidentally logging ratchet material through assert_eq output.
fn message_row_has(endpoint: &Endpoint, needle: &[u8]) -> bool {
    let map = endpoint.device.provider().storage().values.read().unwrap();
    let mut rows = map
        .iter()
        .filter(|(key, _)| key.starts_with(b"MessageSecrets"));
    let row = rows.next().expect("one actual message-secrets row").1;
    assert!(rows.next().is_none());
    row.windows(needle.len()).any(|part| part == needle)
}

fn message_row_digest(endpoint: &Endpoint) -> [u8; 32] {
    let map = endpoint.device.provider().storage().values.read().unwrap();
    let mut rows = map
        .iter()
        .filter(|(key, _)| key.starts_with(b"MessageSecrets"));
    let row = rows.next().expect("one actual message-secrets row").1;
    assert!(rows.next().is_none());
    *blake3::hash(row).as_bytes()
}

#[test]
fn exporter_text_keys_do_not_initialize_application_ratchets() {
    let (alice, bob) = pair();
    let before = message_row_digest(&alice);
    for _ in 0..8 {
        assert!(exporter(&alice) == exporter(&bob));
    }
    assert!(message_row_digest(&alice) == before);
    assert!(!message_row_has(&alice, b"EncryptionRatchet"));
    assert!(!message_row_has(&alice, b"DecryptionRatchet"));
    let restored = reopen(alice);
    assert!(exporter(&restored) == exporter(&bob));
}

#[test]
fn actual_first_application_send_survives_reopen_without_reusing_generation() {
    let (mut alice, mut bob) = pair();
    let before = message_row_digest(&alice);
    let first = send(&mut alice, 1);
    receive(&mut bob, &first, 1);
    assert!(message_row_has(&alice, b"EncryptionRatchet"));
    assert!(message_row_digest(&alice) != before);
    let mut alice = reopen(alice);
    let second = send(&mut alice, 2);
    // A rewind/reused generation is refused by Bob's already-consumed receive ratchet.
    receive(&mut bob, &second, 2);
    reject(&mut bob, &first);
    let mut bob = reopen(bob);
    let third = send(&mut alice, 3);
    receive(&mut bob, &third, 3);
}

#[test]
fn actual_skipped_generation_queue_and_consumed_holes_survive_reopen() {
    let (mut alice, mut bob) = pair();
    let messages: Vec<_> = (0..8).map(|value| send(&mut alice, value)).collect();
    receive(&mut bob, &messages[6], 6);
    assert!(message_row_has(&bob, b"DecryptionRatchet"));
    assert!(message_row_has(&bob, b"past_secrets"));
    let mut bob = reopen(bob);
    receive(&mut bob, &messages[4], 4);
    reject(&mut bob, &messages[4]);
    let mut bob = reopen(bob);
    receive(&mut bob, &messages[3], 3);
    receive(&mut bob, &messages[7], 7);
    reject(&mut bob, &messages[0]);
}

#[test]
fn invalid_authentication_does_not_consume_the_real_original_generation() {
    let (mut alice, mut bob) = pair();
    let messages: Vec<_> = (0..3).map(|value| send(&mut alice, value)).collect();
    let mut corrupted = messages[2].clone();
    *corrupted.last_mut().unwrap() ^= 1;
    reject(&mut bob, &corrupted);
    // prepare_decryption may initialize lazy nodes before authentication. It must not consume
    // the original generation; byte-identical provider state is intentionally NOT asserted.
    receive(&mut bob, &messages[2], 2);
    let mut bob = reopen(bob);
    receive(&mut bob, &messages[1], 1);
    reject(&mut bob, &messages[2]);
}

#[test]
fn exact_forward_distance_boundary_and_cold_past_window_use_real_packets() {
    let (mut alice, mut bob) = pair();
    // Generation1000 requires1000 skipped derivations PLUS the target derivation. A five-entry
    // retained queue does not justify charging only five derivations or five backing slots.
    let messages: Vec<_> = (0..=1001).map(|value| send(&mut alice, value)).collect();
    reject(&mut bob, &messages[1001]);
    receive(&mut bob, &messages[1000], 1000);
    let mut bob = reopen(bob);
    receive(&mut bob, &messages[996], 996);
    reject(&mut bob, &messages[995]);
    reject(&mut bob, &messages[1000]);
    let next = send(&mut alice, 1002);
    receive(&mut bob, &next, 1002);
}

#[test]
fn actual_interior_remove_reuse_and_later_epoch_roundtrip_keep_native_identity() {
    let (mut alice, mut bob) = pair();
    let carol_device = MlsDevice::generate().unwrap();
    let package = carol_device
        .key_package_for_invite(&alice.group.group_id(), [2; 16])
        .unwrap();
    let added = alice.group.add_member(&alice.device, package).unwrap();
    assert!(matches!(
        bob.group
            .process_incoming(&bob.device, &added.commit)
            .unwrap(),
        Incoming::CommitApplied { removed: false }
    ));
    let mut carol = Endpoint {
        group: ServerGroup::join(&carol_device, &added.welcome).unwrap(),
        device: carol_device,
    };
    let removed_id = bob.device.device_id();
    let removed_index = alice.group.member_leaf_index(&removed_id).unwrap();
    let carol_index = alice
        .group
        .member_leaf_index(&carol.device.device_id())
        .unwrap();
    assert!(
        removed_index < carol_index,
        "remove an actual interior leaf"
    );
    let removed = alice
        .group
        .stage_remove(&alice.device, &removed_id)
        .unwrap();
    assert!(matches!(
        carol
            .group
            .process_incoming(&carol.device, &removed.commit)
            .unwrap(),
        Incoming::CommitApplied { removed: true }
    ));
    alice.group.merge_staged_self(&alice.device).unwrap();
    assert_eq!(alice.group.epoch(), 3);
    assert!(roster(&alice) == roster(&carol));
    // Capture the real blank-hole source before another Add fills it.
    let mut carol = reopen(carol);
    assert!(carol.group.member_leaf_index(&removed_id).is_none());

    let dave_device = MlsDevice::generate().unwrap();
    let package = dave_device
        .key_package_for_invite(&alice.group.group_id(), [3; 16])
        .unwrap();
    let added = alice.group.add_member(&alice.device, package).unwrap();
    assert!(matches!(
        carol
            .group
            .process_incoming(&carol.device, &added.commit)
            .unwrap(),
        Incoming::CommitApplied { removed: false }
    ));
    let dave = Endpoint {
        group: ServerGroup::join(&dave_device, &added.welcome).unwrap(),
        device: dave_device,
    };
    assert_eq!(
        alice.group.member_leaf_index(&dave.device.device_id()),
        Some(removed_index),
        "the actual new subject must occupy the actual removed native slot"
    );
    let mut alice = reopen(alice);
    let carol = reopen(carol);
    let mut dave = reopen(dave);
    assert_eq!(alice.group.epoch(), 4);
    assert_eq!(carol.group.epoch(), 4);
    assert_eq!(dave.group.epoch(), 4);
    assert!(roster(&alice) == roster(&carol));
    assert!(roster(&alice) == roster(&dave));
    assert!(exporter(&alice) == exporter(&carol));
    assert!(exporter(&alice) == exporter(&dave));
    let message = send(&mut dave, 40);
    receive(&mut alice, &message, 40);
}

#[test]
fn fifty_real_members_later_epoch_and_nonzero_sender_survive_cold_roundtrip() {
    // This genuine topology-generation case belongs in the primary's final slow validation.
    // Only the last joiner needs a live witness:49 owner Adds + one final Welcome still produce
    // a real50-member tree, without replaying every old commit on49 unused endpoints.
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let group_id = group.group_id();
    let mut last = None;
    for index in 1u128..50 {
        let device = MlsDevice::generate().unwrap();
        let package = device
            .key_package_for_invite(&group_id, index.to_be_bytes())
            .unwrap();
        let added = group.add_member(&owner, package).unwrap();
        if index == 49 {
            last = Some(Endpoint {
                group: ServerGroup::join(&device, &added.welcome).unwrap(),
                device,
            });
        }
    }
    let owner = Endpoint {
        group,
        device: owner,
    };
    let last = last.unwrap();
    assert_eq!(owner.group.member_count(), 50);
    assert_eq!(owner.group.epoch(), 49);
    assert_eq!(last.group.epoch(), 49);
    assert!(
        owner
            .group
            .member_leaf_index(&last.device.device_id())
            .unwrap()
            > 1
    );
    let mut owner = reopen(owner);
    let mut last = reopen(last);
    assert!(roster(&owner) == roster(&last));
    assert!(exporter(&owner) == exporter(&last));
    let outbound = send(&mut owner, 50);
    receive(&mut last, &outbound, 50);
    let reply = send(&mut last, 51);
    receive(&mut owner, &reply, 51);
    let mut owner = reopen(owner);
    let mut last = reopen(last);
    let after_reopen = send(&mut owner, 52);
    receive(&mut last, &after_reopen, 52);
}
