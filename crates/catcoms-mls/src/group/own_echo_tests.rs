//! Messages attributed to this device's own leaf must be refused, never reported as processed.
//!
//! openmls 0.8 returned an error for them; 0.9 returns `OwnPrivateMessage`/`OwnPendingCommit` as
//! successful processing. Our wire format is ciphertext-only, so every echo arrives as a
//! PrivateMessage the author cannot decrypt. If [`ServerGroup::process_incoming`] reported those as
//! `Incoming::Other`, the sync layer's `apply_commit_in_order` would record an unmerged commit as
//! applied while the epoch stayed put.

use super::*;
use catcoms_wire::DocType;

fn group_of_two() -> (MlsDevice, ServerGroup, MlsDevice, ServerGroup) {
    let alice = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let kp = bob
        .key_package_for_invite(&group.group_id(), [1; 16])
        .unwrap();
    let out = group.add_member(&alice, kp).unwrap();
    let bob_group = ServerGroup::join(&bob, &out.welcome).unwrap();
    (alice, group, bob, bob_group)
}

#[test]
fn an_own_application_message_echoed_back_is_refused_without_state_change() {
    let (alice, mut group, bob, mut bob_group) = group_of_two();
    let epoch = group.epoch();
    let own = group
        .create_application_message(&alice, b"echo me")
        .unwrap();
    assert!(group.process_incoming(&alice, &own).is_err());
    assert_eq!(group.epoch(), epoch);
    // The real recipient still decrypts it: refusing the echo consumed nothing shared.
    match bob_group.process_incoming(&bob, &own).unwrap() {
        Incoming::Application(plain) => assert_eq!(plain, b"echo me"),
        other => panic!("expected the application message, got {other:?}"),
    }
}

#[test]
fn an_own_staged_commit_echoed_back_is_refused_and_stays_pending() {
    let (alice, mut group, bob, mut bob_group) = group_of_two();
    let epoch = group.epoch();
    let carol = MlsDevice::generate().unwrap();
    let staged = group
        .stage_add(
            &alice,
            carol
                .key_package_for_invite(&group.group_id(), [2; 16])
                .unwrap(),
        )
        .unwrap();
    // The echo of our own pending commit must not be reported as processed...
    assert!(group.process_incoming(&alice, &staged.commit).is_err());
    assert_eq!(group.epoch(), epoch, "the echo did not advance the epoch");
    // ...and the pending commit is untouched: merging it is still the way to adopt it.
    group.merge_staged_self(&alice).unwrap();
    assert_eq!(group.epoch(), epoch + 1);
    // Bob applies the same commit and lands on the same epoch secrets.
    match bob_group.process_incoming(&bob, &staged.commit).unwrap() {
        Incoming::CommitApplied { removed } => assert!(!removed),
        other => panic!("expected an applied commit, got {other:?}"),
    }
    assert_eq!(
        group.channel_secret(&alice, DocType::Channel, 1).unwrap(),
        bob_group.channel_secret(&bob, DocType::Channel, 1).unwrap()
    );
}

#[test]
fn an_own_merged_commit_echoed_back_is_refused() {
    let (alice, mut group, _bob, _bob_group) = group_of_two();
    let carol = MlsDevice::generate().unwrap();
    let out = group
        .add_member(
            &alice,
            carol
                .key_package_for_invite(&group.group_id(), [3; 16])
                .unwrap(),
        )
        .unwrap();
    let epoch = group.epoch();
    assert!(group.process_incoming(&alice, &out.commit).is_err());
    assert_eq!(group.epoch(), epoch);
}
