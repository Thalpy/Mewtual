use catcoms_mls::{GroupMode, GroupPolicy, InviteToken, MlsDevice, PolicyError, ServerGroup};

#[test]
fn group_policy_requires_the_actual_mls_owner_and_binds_the_group() {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&alice).unwrap();
    assert!(matches!(
        GroupPolicy::issue(&group, &bob, GroupMode::PeerToPeer),
        Err(PolicyError::Unauthorized)
    ));
    assert!(GroupPolicy::issue(&group, &alice, GroupMode::LegacyUnverified).is_err());
    let policy = GroupPolicy::issue(&group, &alice, GroupMode::PeerToPeer).unwrap();
    policy.verify_current_owner(&group).unwrap();
    assert!(policy
        .verify_pin(&ServerGroup::create(&bob).unwrap())
        .is_err());
    assert_eq!(GroupPolicy::decode(&policy.encode()).unwrap(), policy);
    let mut corrupt = policy.encode();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(GroupPolicy::decode(&corrupt).is_err());
    assert!(GroupPolicy::decode(&vec![0; 4_097]).is_err());
}

#[test]
fn group_policy_pin_survives_owner_transfer_but_first_adoption_requires_current_authority() {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&alice).unwrap();
    let pin = GroupPolicy::issue(&group, &alice, GroupMode::PeerToPeer).unwrap();
    let welcome = group
        .add_member(&alice, bob.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut bob_group = ServerGroup::join(&bob, &welcome).unwrap();
    // Low-level MLS transition models a legitimate designated-committer succession. The policy
    // check must neither erase the sealed pin nor treat Alice's old signature as current proof.
    bob_group.remove_member(&bob, &alice.device_id()).unwrap();
    assert_eq!(bob_group.designated_committer(), Some(bob.device_id()));
    pin.verify_pin(&bob_group).unwrap();
    assert!(matches!(
        pin.verify_current_owner(&bob_group),
        Err(PolicyError::Unauthorized)
    ));
    let endorsed = pin.endorse(&bob_group, &bob).unwrap();
    assert_eq!(endorsed.digest(), pin.digest());
    assert_ne!(endorsed.issuer(), pin.issuer());
    endorsed.verify_current_owner(&bob_group).unwrap();
}

#[test]
fn group_policy_v3_invite_binds_exact_body_and_v2_remains_unchanged() {
    let alice = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&alice).unwrap();
    let mut token = group.mint_invite(&alice, [3; 16], 50_000, vec![]).unwrap();
    let legacy = token.encode();
    let decoded = InviteToken::decode(&legacy).unwrap();
    assert!(decoded.policy.is_none());
    assert!(decoded.verify_self());
    assert_eq!(decoded.encode(), legacy);

    let policy = GroupPolicy::issue(&group, &alice, GroupMode::PeerToPeer).unwrap();
    token.bind_policy(&alice, policy.clone()).unwrap();
    let decoded = InviteToken::decode(&token.encode()).unwrap();
    assert!(decoded.verify_self());
    assert_eq!(decoded.policy.as_ref().unwrap().digest(), policy.digest());
    let mut stripped = decoded.clone();
    stripped.policy = None;
    assert!(
        !stripped.verify_self(),
        "v3 signature cannot be downgraded to v2"
    );
    let dedicated = GroupPolicy::issue(&group, &alice, GroupMode::Dedicated).unwrap();
    let mut replaced = decoded;
    replaced.policy = Some(dedicated);
    assert!(
        !replaced.verify_self(),
        "even another valid owner policy is not this invite's policy"
    );
}
