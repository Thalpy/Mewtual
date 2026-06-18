//! Local, no-network tests for the MLS group core: founding, joining via a
//! Welcome, per-channel key agreement and separation, encrypted application
//! messages, and epoch rotation on removal (a removed member cannot derive the
//! new epoch's keys).

use catcoms_mls::{Incoming, MlsDevice, ServerGroup};
use catcoms_wire::DocType;

/// Found a group with `alice` and add `bob` to it; return both groups at the
/// same epoch.
fn two_member_group() -> (MlsDevice, ServerGroup, MlsDevice, ServerGroup) {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();

    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let bob_kp = bob.key_package().unwrap();
    let welcome = alice_group.add_member(&alice, bob_kp).unwrap().welcome;
    let bob_group = ServerGroup::join(&bob, &welcome).unwrap();

    (alice, alice_group, bob, bob_group)
}

#[test]
fn founder_creates_singleton_group() {
    let alice = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&alice).unwrap();
    assert_eq!(group.epoch(), 0);
    assert_eq!(group.member_count(), 1);
    assert!(group.contains_device(&alice.device_id()));
}

#[test]
fn add_and_join_converge_on_membership() {
    let (alice, alice_group, bob, bob_group) = two_member_group();

    assert_eq!(alice_group.epoch(), 1);
    assert_eq!(bob_group.epoch(), 1);
    assert_eq!(alice_group.member_count(), 2);
    assert_eq!(bob_group.member_count(), 2);
    for group in [&alice_group, &bob_group] {
        assert!(group.contains_device(&alice.device_id()));
        assert!(group.contains_device(&bob.device_id()));
    }
    // Both members are in the same group instance.
    assert_eq!(alice_group.group_id(), bob_group.group_id());
}

#[test]
fn members_agree_on_channel_keys_and_separate_channels_differ() {
    let (alice, alice_group, bob, bob_group) = two_member_group();

    let alice_ch1 = alice_group
        .channel_secret(&alice, DocType::Channel, 1)
        .unwrap();
    let bob_ch1 = bob_group.channel_secret(&bob, DocType::Channel, 1).unwrap();
    assert_eq!(
        alice_ch1, bob_ch1,
        "members must derive the same channel key"
    );

    let alice_ch2 = alice_group
        .channel_secret(&alice, DocType::Channel, 2)
        .unwrap();
    assert_ne!(alice_ch1, alice_ch2, "different channels must differ");

    let alice_wiki1 = alice_group
        .channel_secret(&alice, DocType::Wiki, 1)
        .unwrap();
    assert_ne!(alice_ch1, alice_wiki1, "different doc types must differ");

    // Content vs metadata derivation is domain-separated.
    let alice_meta1 = alice_group
        .metadata_secret(&alice, DocType::Channel, 1)
        .unwrap();
    assert_ne!(
        alice_ch1, alice_meta1,
        "content and metadata keys must differ"
    );
}

#[test]
fn application_message_roundtrips_between_members() {
    let (alice, mut alice_group, bob, mut bob_group) = two_member_group();

    let ciphertext = alice_group
        .create_application_message(&alice, b"hello bob")
        .unwrap();
    match bob_group.process_incoming(&bob, &ciphertext).unwrap() {
        Incoming::Application(pt) => assert_eq!(pt, b"hello bob"),
        other => panic!("expected an application message, got {other:?}"),
    }
}

#[test]
fn removing_a_member_rotates_the_epoch_and_locks_them_out() {
    let (alice, mut alice_group, bob, bob_group) = two_member_group();

    let key_before = alice_group
        .channel_secret(&alice, DocType::Channel, 1)
        .unwrap();
    // Bob (still at the old epoch) derived the same key while a member.
    assert_eq!(
        key_before,
        bob_group.channel_secret(&bob, DocType::Channel, 1).unwrap()
    );

    alice_group.remove_member(&alice, &bob.device_id()).unwrap();
    assert_eq!(alice_group.epoch(), 2);
    assert_eq!(alice_group.member_count(), 1);
    assert!(!alice_group.contains_device(&bob.device_id()));

    // The channel key rotated with the epoch...
    let key_after = alice_group
        .channel_secret(&alice, DocType::Channel, 1)
        .unwrap();
    assert_ne!(
        key_before, key_after,
        "epoch change must rotate channel keys"
    );

    // ...and Bob, stuck at the old epoch, cannot derive the new key.
    let bob_key = bob_group.channel_secret(&bob, DocType::Channel, 1).unwrap();
    assert_ne!(
        bob_key, key_after,
        "a removed member must not be able to derive the post-removal key"
    );
}
