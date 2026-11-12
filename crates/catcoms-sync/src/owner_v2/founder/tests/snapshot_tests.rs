//! Actual one-founder SDK/private-source tests. No physical host fee, Current, covering save,
//! archive namespace or positive restore is supplied by these private DATA fixtures.
use super::*;

fn limits() -> FounderSnapshotLimits {
    FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap()
}
fn source(seed: u64) -> (Node, Reservation, FrozenFounder) {
    let mut node = found(seed);
    let reservation = reserve(&mut node);
    let frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    (node, reservation, frozen)
}
fn entries() -> usize {
    initialization_snapshot::encoding_entries_on_this_thread()
}
fn denied(node: &Node, frozen: &FrozenFounder) {
    assert!(frozen.borrow_for_covering_save(node).is_err());
    assert!(frozen
        .borrow_private_snapshot_for_covering_save(node)
        .is_err());
}

#[test]
fn typed_s0_limits_refuse_zero_overflow_and_closed_ceiling_before_encoding() {
    let before = entries();
    for (snapshot, mls, backing) in [
        (0, 0, 0),
        (65_536, 1, 1),
        (usize::MAX, usize::MAX, usize::MAX),
        (9 * 1024 * 1024, 8 * 1024 * 1024, 1),
        (9 * 1024 * 1024, 9 * 1024 * 1024, 1),
        (1024 * 1024, 1, 9 * 1024 * 1024),
    ] {
        assert!(FounderSnapshotLimits::new(snapshot, mls, backing).is_err());
    }
    let l = limits();
    assert_eq!(l.max_snapshot_bytes(), 1024 * 1024);
    assert_eq!(l.max_mls_bytes(), 512 * 1024);
    assert_eq!(l.max_provider_backing_bytes(), 512 * 1024);
    assert_eq!(l.max_provider_payload_bytes(), 1024 * 1024);
    assert_eq!(entries(), before);
}

#[test]
fn actual_s0_is_cached_once_with_exact_bytes_pointer_digest_and_real_sdk_restore() {
    let (mut node, _reservation, mut frozen) = source(0x3f01);
    assert!(frozen
        .borrow_private_snapshot_for_covering_save(&node)
        .is_err());
    let before = entries();
    let view = frozen.cache_private_snapshot(&mut node, limits()).unwrap();
    let bytes = Zeroizing::new(view.snapshot.to_vec());
    let digest = view.digest;
    let pointer = view.snapshot.as_ptr();
    assert_eq!(entries(), before + 1);
    for _ in 0..3 {
        let view = frozen.cache_private_snapshot(&mut node, limits()).unwrap();
        assert_eq!(view.snapshot, bytes.as_slice());
        assert_eq!(view.snapshot.as_ptr(), pointer);
        assert_eq!(view.digest, digest);
        assert_eq!(view.limits, limits());
        assert!(frozen.borrow_for_covering_save(&node).is_ok());
    }
    assert_eq!(
        entries(),
        before + 1,
        "bounded metadata checks must not create another full S0"
    );
    let restored = ChannelSync::restore(
        &bytes,
        Hub::new().join(PeerId::from_u64(0x3f02)),
        ChaCha20Rng::seed_from_u64(0x3f02),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert_eq!(restored.group_id(), node.group_id());
    assert_eq!(restored.my_public_key(), node.my_public_key());
    assert_eq!(
        restored.group.epoch_authenticator_id(),
        node.group.epoch_authenticator_id()
    );
    assert_eq!(
        restored
            .group
            .channel_secret(&restored.device, DocType::Channel, 9)
            .unwrap(),
        node.group
            .channel_secret(&node.device, DocType::Channel, 9)
            .unwrap()
    );
    assert!(restored.fresh_chat_admission().is_none());
    assert!(restored.durable_send_context().is_err());
    denied(&restored, &frozen);
    assert!(frozen
        .cache_private_snapshot(
            &mut node,
            FounderSnapshotLimits::new(2 * 1024 * 1024, 1024 * 1024, 1024 * 1024).unwrap()
        )
        .is_err());
    assert_eq!(
        frozen
            .borrow_private_snapshot_for_covering_save(&node)
            .unwrap()
            .snapshot
            .as_ptr(),
        pointer
    );
    assert_eq!(entries(), before + 1);
}

#[test]
fn extra_actual_key_package_refuses_cached_source_without_epoch_change_or_resnapshot() {
    let (mut node, _reservation, mut frozen) = source(0x3f03);
    let digest = frozen
        .cache_private_snapshot(&mut node, limits())
        .unwrap()
        .digest;
    let before = entries();
    let auth = node.group.epoch_authenticator_id();
    node.device
        .key_package_for_invite(&node.group_id(), [3; 16])
        .unwrap();
    assert_eq!(node.group.epoch_authenticator_id(), auth);
    assert!(
        frozen.original.validate_original(&node).is_ok(),
        "old Fresh/epoch checks alone miss this mutation"
    );
    denied(&node, &frozen);
    assert!(frozen.cache_private_snapshot(&mut node, limits()).is_err());
    assert_eq!(entries(), before);
    assert!(frozen.snapshot.is_some());
    assert_ne!(digest.as_bytes(), &[0; 32]);
}

#[test]
fn every_allowed_variable_metadata_family_is_compared_against_exact_original_s0() {
    for field in 0..5 {
        let (mut node, _reservation, mut frozen) = source(0x3f10 + field);
        frozen.cache_private_snapshot(&mut node, limits()).unwrap();
        let before = entries();
        match field {
            0 => node.set_rendezvous_nodes(vec![("/ip4/127.0.0.1/tcp/9".into(), vec![1; 32])]),
            // Internal adversarial replacements isolate the source comparator even though the
            // future opaque host will expose none of these mutable fields to its caller.
            1 => node.file_wrap_key = Zeroizing::new([2; 32]),
            2 => node.routing_label = node.routing_label.wrapping_add(1),
            3 => {
                let secret = node.routing_secrets.values_mut().next().unwrap();
                secret[0] ^= 1;
            }
            4 => {
                node.group_policy = Some(
                    GroupPolicy::issue(&node.group, &node.device, GroupMode::PeerToPeer).unwrap(),
                )
            }
            _ => unreachable!(),
        }
        denied(&node, &frozen);
        assert!(frozen.cache_private_snapshot(&mut node, limits()).is_err());
        assert!(frozen.snapshot.is_some());
        assert_eq!(entries(), before);
    }
    // This is exact CURRENT equality, not a history-of-all-transient-mutations counter.
    let (mut node, _reservation, mut frozen) = source(0x3f20);
    let original = frozen
        .cache_private_snapshot(&mut node, limits())
        .unwrap()
        .digest;
    node.set_rendezvous_nodes(vec![("temporary".into(), vec![1])]);
    denied(&node, &frozen);
    node.set_rendezvous_nodes(vec![]);
    assert_eq!(
        frozen
            .borrow_private_snapshot_for_covering_save(&node)
            .unwrap()
            .digest,
        original
    );
}

#[test]
fn original_negative_revocation_after_encoding_prevents_cache_admission() {
    let (mut node, reservation, mut frozen) = source(0x3f21);
    let profile = frozen.profile.clone();
    let before = entries();
    let original = reservation.owner.clone();
    super::super::snapshot::set_after_encoding(Box::new(move || {
        original.send_replace(true);
    }));
    assert!(frozen.cache_private_snapshot(&mut node, limits()).is_err());
    assert_eq!(entries(), before + 1);
    assert!(frozen.snapshot.is_none());
    assert_eq!(frozen.profile, profile);
    denied(&node, &frozen);
}

#[test]
fn tiny_provider_and_nonempty_finalization_refuse_before_inner_encoding_and_preserve_work() {
    let (mut node, _reservation, mut frozen) = source(0x3f22);
    let before = entries();
    let tiny = FounderSnapshotLimits::new(65_537, 1, 1).unwrap();
    assert!(frozen.cache_private_snapshot(&mut node, tiny).is_err());
    assert_eq!(entries(), before);
    assert!(frozen.snapshot.is_none());
    assert!(frozen.borrow_for_covering_save(&node).is_ok());
    node.member_finalization_pending
        .insert(node.device.device_id(), PeerId::from_u64(999));
    assert!(frozen.cache_private_snapshot(&mut node, limits()).is_err());
    assert_eq!(entries(), before);
    assert_eq!(
        node.member_finalization_pending.len(),
        1,
        "never prune to fit capture"
    );
    assert!(frozen.snapshot.is_none());
}

#[test]
fn cache_cannot_survive_taken_fresh_scope_retirement_or_real_mls_add() {
    let (mut node, _reservation, mut frozen) = source(0x3f23);
    frozen.cache_private_snapshot(&mut node, limits()).unwrap();
    drop(node.take_fresh_chat_admission().unwrap());
    denied(&node, &frozen);
    let (mut node, _reservation, mut frozen) = source(0x3f24);
    frozen.cache_private_snapshot(&mut node, limits()).unwrap();
    node.local_chat_scope.retire();
    denied(&node, &frozen);
    let (mut node, _reservation, mut frozen) = source(0x3f25);
    frozen.cache_private_snapshot(&mut node, limits()).unwrap();
    let device = MlsDevice::generate().unwrap();
    let package = device
        .key_package_for_invite(&node.group_id(), [25; 16])
        .unwrap();
    node.with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert_eq!(node.group.member_count(), 2);
    denied(&node, &frozen);
}

#[test]
fn cache_keeps_no_reservation_sender_and_public_representation_changes_also_refuse() {
    let (mut node, reservation, mut frozen) = source(0x3f26);
    frozen.cache_private_snapshot(&mut node, limits()).unwrap();
    drop(reservation);
    denied(&node, &frozen);
    let (mut node, _reservation, mut frozen) = source(0x3f27);
    frozen.cache_private_snapshot(&mut node, limits()).unwrap();
    *frozen.header.last_mut().unwrap() ^= 1;
    denied(&node, &frozen);
    *frozen.header.last_mut().unwrap() ^= 1;
    assert!(frozen
        .borrow_private_snapshot_for_covering_save(&node)
        .is_ok());
}
