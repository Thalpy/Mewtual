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
fn members_agree_on_routing_secrets_which_are_domain_separated() {
    let (alice, alice_group, bob, bob_group) = two_member_group();

    // Both members at the same epoch derive the same routing secret (the source of
    // ns_secret_L) and the same routing-transfer wrap key.
    let a_ns = alice_group.routing_metadata_secret(&alice).unwrap();
    let b_ns = bob_group.routing_metadata_secret(&bob).unwrap();
    assert_eq!(
        a_ns, b_ns,
        "members must agree on the routing metadata secret"
    );

    let a_xfer = alice_group.routing_transfer_key(&alice).unwrap();
    let b_xfer = bob_group.routing_transfer_key(&bob).unwrap();
    assert_eq!(
        a_xfer, b_xfer,
        "members must agree on the routing-transfer key"
    );

    // The transfer key is domain-separated from the routing secret (so it is never
    // equal to any ns_secret_L) and from per-document content/metadata keys.
    assert_ne!(a_ns, a_xfer, "routing secret and transfer key must differ");
    let a_meta = alice_group
        .metadata_secret(&alice, DocType::Channel, 1)
        .unwrap();
    assert_ne!(
        a_ns, a_meta,
        "routing secret is domain-separated from doc metadata"
    );
    assert_ne!(a_xfer, a_meta);
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
fn staged_add_can_be_aborted_with_keys_intact() {
    // The fork-loser primitive: stage an Add, then abort it. The group must be
    // exactly as before; same epoch, same membership, same channel key, still
    // usable; so a committer that loses a fork loses nothing.
    let alice = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let key_before = alice_group
        .channel_secret(&alice, DocType::Channel, 1)
        .unwrap();
    let base_before = alice_group.epoch_authenticator_id();

    let bob = MlsDevice::generate().unwrap();
    let staged = alice_group
        .stage_add(&alice, bob.key_package().unwrap())
        .unwrap();
    assert_eq!(staged.commit_epoch, 0);
    assert_eq!(staged.base_authenticator, base_before);

    alice_group.abort_staged(&alice).unwrap();
    assert_eq!(alice_group.epoch(), 0, "abort must not advance the epoch");
    assert_eq!(
        alice_group.member_count(),
        1,
        "abort must not add the member"
    );
    assert_eq!(
        alice_group
            .channel_secret(&alice, DocType::Channel, 1)
            .unwrap(),
        key_before,
        "abort must preserve epoch key material"
    );
    // The group is still operational: a fresh stage+merge works afterward.
    let staged2 = alice_group
        .stage_add(&alice, bob.key_package().unwrap())
        .unwrap();
    alice_group.merge_staged_self(&alice).unwrap();
    assert_eq!(alice_group.epoch(), 1);
    assert!(alice_group.contains_device(&bob.device_id()));
    assert!(staged2.welcome.is_some());
}

#[test]
fn staged_then_merged_add_matches_direct_add() {
    // Stage→merge advances the epoch and adds the member, and the joiner can join
    // from the staged Welcome; identical outcome to the atomic add_member path.
    let alice = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();

    let staged = alice_group
        .stage_add(&alice, bob.key_package().unwrap())
        .unwrap();
    alice_group.merge_staged_self(&alice).unwrap();
    let bob_group = ServerGroup::join(&bob, &staged.welcome.unwrap()).unwrap();

    assert_eq!(alice_group.epoch(), 1);
    assert_eq!(bob_group.epoch(), 1);
    assert_eq!(alice_group.group_id(), bob_group.group_id());
    assert!(alice_group.contains_device(&bob.device_id()));
}

#[test]
fn members_at_the_same_epoch_agree_on_the_base_authenticator() {
    // The fork-vs-lag binding: every member derives the SAME epoch fingerprint at
    // the same epoch, and it changes when the epoch advances.
    let (alice, mut alice_group, bob, bob_group) = two_member_group();
    let a = alice_group.epoch_authenticator_id();
    let b = bob_group.epoch_authenticator_id();
    assert_eq!(a, b, "members must agree on the epoch fingerprint");

    let carol = MlsDevice::generate().unwrap();
    alice_group
        .add_member(&alice, carol.key_package().unwrap())
        .unwrap();
    assert_ne!(
        alice_group.epoch_authenticator_id(),
        a,
        "the fingerprint must change when the epoch advances"
    );
    let _ = bob; // (bob_group kept at the old epoch intentionally)
}

#[test]
fn every_member_rejects_an_add_with_a_cross_group_credential() {
    // Membership integrity must not rest on trusting the committer: an honest
    // member independently rejects an Add whose credential is bound to a *different*
    // group, even though the (malicious) committer produced a structurally valid
    // MLS commit. Defends against a compromised committer injecting an outsider.
    let alice = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let welcome = alice_group
        .add_member(&alice, bob.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut bob_group = ServerGroup::join(&bob, &welcome).unwrap();
    assert_eq!(bob_group.epoch(), 1);

    // Alice (malicious) adds Dave with a KeyPackage credential bound to ANOTHER
    // group. openmls accepts it (the binding is an application-level invariant), and
    // Alice merges it locally.
    let other_group_id = vec![7u8; 32];
    let dave = MlsDevice::generate().unwrap();
    let dave_kp = dave
        .key_package_for_invite(&other_group_id, [9u8; 16])
        .unwrap();
    let outcome = alice_group.add_member(&alice, dave_kp).unwrap();

    // Bob, applying the same commit, rejects it and does not advance.
    let result = bob_group.process_incoming(&bob, &outcome.commit);
    assert!(
        result.is_err(),
        "a cross-group-bound Add must be rejected on apply by every member"
    );
    assert_eq!(
        bob_group.epoch(),
        1,
        "Bob did not advance on the rejected commit"
    );
    assert!(!bob_group.contains_device(&dave.device_id()));
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
