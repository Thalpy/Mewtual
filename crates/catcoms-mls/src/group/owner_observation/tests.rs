use super::*;
use crate::{restore_server, snapshot_server};

#[test]
fn local_owner_observation_is_actual_add_fresh_kp_and_remove_bytes() {
    let founder = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&founder).unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&group.group_id(), [8; 16])
        .unwrap();
    let expected = member.key_package_reference(&package).unwrap();
    let credential = package
        .leaf_node()
        .credential()
        .tls_serialize_detached()
        .unwrap();
    let (outcome, added) = group.add_member_observed(&founder, package).unwrap();
    assert_eq!(added.kind(), LocalMembershipChangeKind::Add);
    assert_eq!(added.subject().as_slice(), member.public_key_bytes());
    assert_eq!(added.key_package_reference(), Some(expected));
    assert_eq!(
        added.credential_digest(),
        Some(credential_digest(&credential))
    );
    assert_eq!(added.before_epoch(), 0);
    assert_eq!(added.after_epoch(), 1);
    assert_eq!(added.commit(), outcome.commit);
    assert!(added.matches_current(&group, &founder));
    let joined = ServerGroup::join_for_key_package(&member, &outcome.welcome, &expected).unwrap();
    assert_eq!(
        joined.own_credential_digest(&member).unwrap(),
        added.credential_digest().unwrap(),
        "actual selected member credential matches the owner's exact Add observation"
    );
    assert!(joined.own_credential_digest(&founder).is_err());
    let saved = snapshot_server(&member, &joined).unwrap();
    let (restored_member, restored_group) = restore_server(&saved).unwrap();
    assert_eq!(
        restored_group
            .own_credential_digest(&restored_member)
            .unwrap(),
        added.credential_digest().unwrap(),
        "local credential comparison survives restore without minting a fresh Add proof"
    );
    assert_eq!(joined.group_id(), group.group_id());
    assert_eq!(
        joined.epoch_authenticator_id(),
        group.epoch_authenticator_id()
    );
    let removed = group
        .remove_member_observed(&founder, &member.device_id())
        .unwrap();
    assert_eq!(removed.kind(), LocalMembershipChangeKind::Remove);
    assert_eq!(removed.subject(), added.subject());
    assert_eq!(removed.before_epoch(), 1);
    assert_eq!(removed.after_epoch(), 2);
    assert!(!removed.commit().is_empty());
    assert_eq!(
        removed.commit_digest(),
        *blake3::hash(removed.commit()).as_bytes()
    );
    assert_eq!(removed.key_package_reference(), None);
    assert_eq!(removed.credential_digest(), None);
    assert!(removed.matches_current(&group, &founder));
    assert!(!added.matches_current(&group, &founder));
    assert!(group
        .remove_member_observed(&founder, &member.device_id())
        .is_err());
    assert!(
        removed.matches_current(&group, &founder),
        "failed work creates no replacement token"
    );
}

#[test]
fn local_owner_observation_refuses_same_epoch_same_member_different_actual_fork() {
    let founder = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&founder).unwrap();
    let before = snapshot_server(&founder, &group).unwrap();
    let (fork_founder, mut fork) = restore_server(&before).unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&group.group_id(), [9; 16])
        .unwrap();
    let (_, observed) = group
        .add_member_observed(&founder, package.clone())
        .unwrap();
    fork.add_member(&fork_founder, package).unwrap();
    assert_eq!(group.group_id(), fork.group_id());
    assert_eq!(group.epoch(), fork.epoch());
    assert!(fork.contains_device(&member.device_id()));
    assert_ne!(
        group.epoch_authenticator_id(),
        fork.epoch_authenticator_id()
    );
    assert!(
        !observed.matches_current(&fork, &fork_founder),
        "group/key/epoch equality cannot substitute a different actual transition"
    );
}

#[test]
fn local_owner_observation_same_key_and_same_credential_rejoin_binds_new_kp_and_commit() {
    let founder = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&founder).unwrap();
    let member = MlsDevice::generate().unwrap();
    let package1 = member
        .key_package_for_invite(&group.group_id(), [10; 16])
        .unwrap();
    let (_, a1) = group.add_member_observed(&founder, package1).unwrap();
    group
        .remove_member_observed(&founder, &member.device_id())
        .unwrap();
    let rejoined = member.duplicate().unwrap();
    let package2 = rejoined
        .key_package_for_invite(&group.group_id(), [10; 16])
        .unwrap();
    let expected2 = rejoined.key_package_reference(&package2).unwrap();
    let (outcome2, a2) = group.add_member_observed(&founder, package2).unwrap();
    assert_eq!(a1.subject(), a2.subject());
    assert_eq!(a1.credential_digest(), a2.credential_digest());
    assert_ne!(a1.key_package_reference(), a2.key_package_reference());
    assert_ne!(a1.commit_digest(), a2.commit_digest());
    assert!(!a1.matches_current(&group, &founder));
    assert!(a2.matches_current(&group, &founder));
    let joined2 =
        ServerGroup::join_for_key_package(&rejoined, &outcome2.welcome, &expected2).unwrap();
    assert_eq!(
        joined2.epoch_authenticator_id(),
        group.epoch_authenticator_id()
    );
}

#[test]
fn local_owner_observation_remains_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<LocalMembershipChange>();
}

#[test]
fn local_owner_observation_wrong_actual_local_member_refuses_before_mutation() {
    let founder = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&founder).unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&group.group_id(), [12; 16])
        .unwrap();
    let expected = member.key_package_reference(&package).unwrap();
    let (outcome, added) = group.add_member_observed(&founder, package).unwrap();
    let _joined = ServerGroup::join_for_key_package(&member, &outcome.welcome, &expected).unwrap();
    assert!(group.contains_device(&member.device_id()));
    assert!(!group.has_local_device(&member.device_id()));
    assert!(!group.has_local_signer(&member.public_key_bytes()));
    let newcomer = MlsDevice::generate().unwrap();
    let package = newcomer
        .key_package_for_invite(&group.group_id(), [13; 16])
        .unwrap();
    let epoch = group.epoch();
    let state = group.epoch_authenticator_id();
    assert!(matches!(
        group.add_member_observed(&member, package.clone()),
        Err(MlsError::Internal(
            "owner observation needs active local device"
        ))
    ));
    assert!(matches!(
        group.remove_member_observed(&member, &member.device_id()),
        Err(MlsError::Internal(
            "owner observation needs active local device"
        ))
    ));
    assert_eq!(group.epoch(), epoch);
    assert_eq!(group.epoch_authenticator_id(), state);
    assert!(added.matches_current(&group, &founder));
    assert!(matches!(
        group.remove_member_observed(&founder, &founder.device_id()),
        Err(MlsError::Internal(
            "observed local self-retirement is unsupported"
        ))
    ));
    assert_eq!(group.epoch_authenticator_id(), state);
    group.add_member_observed(&founder, package).unwrap();
    assert_eq!(group.epoch(), epoch + 1);
}

#[test]
fn own_credential_restored_byte_boundary_refuses_before_tls_serialization() {
    use openmls_traits::OpenMlsProvider;
    use std::cell::Cell;
    for length in [
        MAX_MEMBER_CATALOG_CREDENTIAL_BYTES,
        MAX_MEMBER_CATALOG_CREDENTIAL_BYTES + 1,
    ] {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        // Alter the actual stored own leaf then use the normal loader. This tests a restored
        // unsupported shape, not a claim anyone can forge MLS input or the Store's seal.
        {
            let mut values = owner.provider().storage().values.write().unwrap();
            let (_, bytes) = values
                .iter_mut()
                .find(|(key, _)| key.starts_with(b"Tree"))
                .expect("one actual group tree");
            let mut tree: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            *tree
                .pointer_mut(
                    "/tree/leaf_nodes/0/node/payload/credential/serialized_credential_content/vec",
                )
                .expect("actual persisted own credential") = serde_json::json!(vec![7u8; length]);
            *bytes = serde_json::to_vec(&tree).unwrap();
        }
        let snapshot = snapshot_server(&owner, &group).unwrap();
        let (device, restored) = restore_server(&snapshot).unwrap();
        let encodings = Cell::new(0);
        let result = restored.own_credential_digest_with(&device, || {
            encodings.set(encodings.get() + 1);
        });
        if length == MAX_MEMBER_CATALOG_CREDENTIAL_BYTES {
            assert!(result.is_ok());
            assert_eq!(encodings.get(), 1);
        } else {
            assert!(result.is_err());
            assert_eq!(
                encodings.get(),
                0,
                "unsupported own body must not be serialized"
            );
        }
    }
}
