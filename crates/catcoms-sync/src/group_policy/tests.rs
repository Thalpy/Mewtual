use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;

fn owner() -> Node {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    Node::new(
        Hub::new().join(PeerId::from_u64(1)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(1),
        Box::new(ManualClock::new(1_000)),
    )
}

fn restore(bytes: &[u8]) -> Result<Node, SyncError> {
    Node::restore(
        bytes,
        Hub::new().join(PeerId::from_u64(99)),
        ChaCha20Rng::seed_from_u64(99),
        Box::new(ManualClock::new(1_000)),
    )
}

fn join(owner: &mut Node, nonce: u8) -> Node {
    let device = MlsDevice::generate().unwrap();
    let invite = owner.mint_invite([nonce; 16], u64::MAX, vec![]).unwrap();
    let kp = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let request = encode_join_req(&invite, &serialize_key_package(&kp).unwrap());
    let response = owner
        .serve_join(PeerId::from_u64(nonce as u64), &request)
        .unwrap();
    assert_eq!(response[0], JOIN_READY);
    let (welcome, signature, transfer) = decode_join_resp(&response[1..]).unwrap();
    let (group, routing) = finish_join(&device, &invite, &welcome, &signature, &transfer).unwrap();
    Node::new_joined(
        Hub::new().join(PeerId::from_u64(nonce as u64)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(nonce as u64),
        Box::new(ManualClock::new(1_000)),
        routing,
    )
}

#[test]
fn group_policy_is_bound_through_actual_admission_and_sealed_snapshot() {
    let mut alice = owner();
    assert_eq!(alice.group_mode(), GroupMode::LegacyUnverified);
    assert!(!alice.policy_allows_member_mesh());
    alice
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    assert!(!alice.group_policy_publish_ready);
    assert_eq!(alice.group_mode(), GroupMode::LegacyUnverified);
    assert!(matches!(
        alice.mint_invite([1; 16], u64::MAX, vec![]),
        Err(SyncError::Policy(PolicyError::PendingPersistence))
    ));
    let saved = alice.snapshot().unwrap();
    assert_eq!(restore(&saved).unwrap().group_mode(), GroupMode::PeerToPeer);
    alice.publish_group_policy().unwrap();
    let mut bob = join(&mut alice, 2);
    assert_eq!(bob.group_mode(), GroupMode::PeerToPeer);
    assert_eq!(bob.group_policy_digest(), alice.group_policy_digest());
    let snap = bob.snapshot().unwrap();
    let restored = restore(&snap).unwrap();
    assert_eq!(restored.group_policy_digest(), bob.group_policy_digest());
    assert!(restored.policy_allows_member_mesh());
    assert!(matches!(
        bob.initialize_group_policy(GroupMode::PeerToPeer),
        Err(SyncError::Policy(PolicyError::Unauthorized))
    ));
}

#[test]
fn group_policy_legacy_snapshot_and_invite_are_explicitly_unresolved() {
    let mut alice = owner();
    let mut bob = join(&mut alice, 2);
    assert_eq!(bob.group_mode(), GroupMode::LegacyUnverified);
    assert!(bob
        .mint_invite([9; 16], u64::MAX, vec![])
        .unwrap()
        .policy
        .is_none());
    let snapshot = bob.snapshot().unwrap();
    let extension = encode_pin(None).len() + 4;
    let old = &snapshot[..snapshot.len() - extension];
    assert_eq!(
        restore(old).unwrap().group_mode(),
        GroupMode::LegacyUnverified
    );
    for bytes in 1..extension {
        assert!(
            restore(&snapshot[..old.len() + bytes]).is_err(),
            "a partial policy extension is not a legacy snapshot"
        );
    }
    let mut unknown = encode_pin(None);
    unknown[0] = 99;
    assert!(decode_pin(&unknown).is_err());
    assert!(alice.initialize_group_policy(GroupMode::Dedicated).is_err());
    assert!(alice
        .initialize_group_policy(GroupMode::LegacyUnverified)
        .is_err());
}

#[test]
fn group_policy_migration_rejects_old_invites_before_any_membership_mutation() {
    let mut alice = owner();
    let invite = alice.mint_invite([4; 16], u64::MAX, vec![]).unwrap();
    alice
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    let joining = MlsDevice::generate().unwrap();
    let kp = joining
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let before = alice.epoch();
    assert!(alice
        .serve_join(
            PeerId::from_u64(4),
            &encode_join_req(&invite, &serialize_key_package(&kp).unwrap())
        )
        .is_none());
    assert_eq!(alice.epoch(), before);
    assert!(!alice.ledger.is_consumed(&invite.invite_nonce));
}

#[test]
fn group_policy_admission_rejects_a_different_policy_even_with_valid_inviter_signature() {
    let mut alice = owner();
    alice
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    alice.publish_group_policy().unwrap();
    let mut invite = alice.mint_invite([5; 16], u64::MAX, vec![]).unwrap();
    let joining = MlsDevice::generate().unwrap();
    let kp = joining
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let (welcome, transfer, signature) = alice
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1_000)
        .unwrap();
    let other = GroupPolicy::issue(&alice.group, &alice.device, GroupMode::Dedicated).unwrap();
    invite.bind_policy(&alice.device, other).unwrap();
    assert!(invite.verify_self());
    assert!(finish_join(&joining, &invite, &welcome, &signature, &transfer).is_err());
}

/// Malicious clients can sign policy bytes without going through GroupPolicy::issue. Verify
/// current governance on the receiving path rather than trusting the honest issuing API.
fn member_signed_policy(pin: &GroupPolicy, member: &MlsDevice) -> GroupPolicy {
    let bytes = pin.encode();
    let mut outer = Decoder::new(&bytes);
    let mut signed = Decoder::new(outer.get_bytes().unwrap());
    let domain = signed.get_str().unwrap();
    let body = signed.get_bytes().unwrap();
    let epoch = signed.get_u64().unwrap();
    let mut e = Encoder::new();
    e.put_str(domain).unwrap();
    e.put_bytes(body).unwrap();
    e.put_u64(epoch);
    e.put_bytes(&member.public_key_bytes()).unwrap();
    let payload = e.finish();
    let signature = member.sign(&payload).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&payload).unwrap();
    e.put_bytes(&signature).unwrap();
    GroupPolicy::decode(&e.finish()).unwrap()
}

#[test]
fn group_policy_control_checks_live_owner_and_cannot_change_a_pin() {
    let mut alice = owner();
    let mut bob = join(&mut alice, 2);
    alice
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    let policy = alice.group_policy().unwrap().clone();
    let forged = member_signed_policy(&policy, &bob.device);
    assert!(forged.verify_self());
    bob.on_group_policy(&forged.encode());
    assert_eq!(bob.group_mode(), GroupMode::LegacyUnverified);
    let mut control = vec![CTRL_GROUP_POLICY];
    control.extend_from_slice(&policy.encode());
    bob.on_control(alice.local_peer(), &control);
    assert_eq!(bob.group_mode(), GroupMode::PeerToPeer);
    assert_eq!(bob.group_policy_revision(), 1);
    bob.on_control(alice.local_peer(), &control);
    assert_eq!(
        bob.group_policy_revision(),
        1,
        "replay cannot cause endless persistence"
    );
    let dedicated = GroupPolicy::issue(&alice.group, &alice.device, GroupMode::Dedicated).unwrap();
    bob.on_group_policy(&dedicated.encode());
    assert_eq!(bob.group_policy_digest(), Some(policy.digest()));
    assert!(alice
        .outbox
        .iter()
        .all(|(_, frame)| frame.first() != Some(&CTRL_GROUP_POLICY)));
    alice.publish_group_policy().unwrap();
    for _ in 0..20 {
        alice.republish_group_policy_if_ready();
    }
    assert_eq!(
        alice
            .outbox
            .iter()
            .filter(|(_, frame)| frame.first() == Some(&CTRL_GROUP_POLICY))
            .count(),
        1
    );
}

#[test]
fn group_policy_owner_transfer_preserves_pin_but_unprovable_admission_fails_before_commit() {
    let mut alice = owner();
    alice
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    alice.publish_group_policy().unwrap();
    let mut bob = join(&mut alice, 2);
    let pin = bob.group_policy_digest();
    let alice_id = alice.device_id();
    bob.with_observed_mls_transition(|node| node.group.remove_member(&node.device, &alice_id))
        .unwrap();
    assert!(bob.is_designated_committer());
    assert_eq!(bob.group_policy_digest(), pin);
    let mut restored = restore(&bob.snapshot().unwrap()).unwrap();
    assert_eq!(restored.group_policy_digest(), pin);
    restored.publish_group_policy().unwrap();
    assert!(matches!(
        restored.mint_invite([7; 16], u64::MAX, vec![]),
        Err(SyncError::Policy(
            PolicyError::AdmissionAuthorityUnavailable
        ))
    ));
    // A queued preexisting admin request also rechecks this boundary at actual admission.
    let mut invite = restored
        .group
        .mint_invite(&restored.device, [8; 16], u64::MAX, vec![])
        .unwrap();
    invite
        .bind_policy(
            &restored.device,
            restored.admission_policy().unwrap().unwrap(),
        )
        .unwrap();
    let joining = MlsDevice::generate().unwrap();
    let kp = joining
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let before = restored.epoch();
    assert!(restored
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1_000)
        .is_none());
    assert_eq!(restored.epoch(), before);
    assert!(!restored.ledger.is_consumed(&invite.invite_nonce));
}
