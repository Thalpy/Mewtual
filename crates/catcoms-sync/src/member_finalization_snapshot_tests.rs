fn restore_pending_snapshot(bytes: &[u8]) -> Result<Node, SyncError> {
    Node::restore(
        bytes,
        Hub::new().join(PeerId::from_u64(1)),
        ChaCha20Rng::seed_from_u64(91),
        Box::new(ManualClock::new(2_000)),
    )
}

fn snapshot_without_pending_tail(node: &mut Node) -> Vec<u8> {
    let tail = node.pending_finalization_snapshot().unwrap();
    let mut snapshot = node.snapshot().unwrap().to_vec();
    snapshot.truncate(snapshot.len() - 4 - tail.len());
    snapshot
}

fn snapshot_with_pending_tail(prefix: &[u8], tail: &[u8]) -> Vec<u8> {
    let mut frame = Encoder::new();
    frame.put_bytes(tail).unwrap();
    let mut snapshot = prefix.to_vec();
    snapshot.extend_from_slice(&frame.finish());
    snapshot
}

fn pending_test_tail(version: u8, entries: &[(DeviceId, PeerId)]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(version);
    e.put_u32(entries.len() as u32);
    for (device, peer) in entries {
        e.put_bytes(device.as_bytes()).unwrap();
        e.put_bytes(peer.as_bytes()).unwrap();
    }
    e.finish()
}

#[tokio::test]
async fn pending_admission_correlation_survives_restore_without_proof_or_routes() {
    let (mut alice, bob) = pair(true);
    let peer = bob.local_peer();
    assert!(!alice.peer_records.contains_key(&bob.device_id()));
    assert_eq!(alice.member_finalization_candidates(), vec![peer]);
    let snapshot = alice.snapshot().unwrap();
    let mut restored = restore_pending_snapshot(&snapshot).unwrap();
    assert_eq!(restored.member_finalization_candidates(), vec![peer]);
    assert!(restored.finalized_member_peers().is_empty());
    assert!(!restored.peer_is_connected(peer));
    assert_eq!(restored.dial_local_reconnect_routes().await, 0);
    assert!(!restored.finalize_member_connection(peer).await.unwrap());

    let legacy = snapshot_without_pending_tail(&mut alice);
    let mut restored = restore_pending_snapshot(&legacy).unwrap();
    assert!(restored.member_finalization_candidates().is_empty());
    assert_eq!(restored.group_mode(), GroupMode::PeerToPeer);
}

#[test]
fn pending_snapshot_tail_refuses_unknown_truncated_oversized_or_ambiguous_records() {
    let (mut alice, bob) = pair(true);
    let prefix = snapshot_without_pending_tail(&mut alice);
    let pair = (bob.device_id(), bob.local_peer());
    let valid = pending_test_tail(PENDING_SNAPSHOT_VERSION, &[pair]);
    for tail in [
        pending_test_tail(PENDING_SNAPSHOT_VERSION + 1, &[pair]),
        valid[..valid.len() - 1].to_vec(),
        pending_test_tail(PENDING_SNAPSHOT_VERSION, &[pair, pair]),
        pending_test_tail(
            PENDING_SNAPSHOT_VERSION,
            &[
                (DeviceId::from_bytes([0; 32]), pair.1),
                (DeviceId::from_bytes([1; 32]), pair.1),
            ],
        ),
        vec![0; MAX_PENDING_SNAPSHOT_BYTES + 1],
    ] {
        assert!(restore_pending_snapshot(&snapshot_with_pending_tail(&prefix, &tail)).is_err());
    }
    let mut count = Encoder::new();
    count.put_u8(PENDING_SNAPSHOT_VERSION);
    count.put_u32(MAX_PEER_RECORDS as u32 + 1);
    assert!(
        restore_pending_snapshot(&snapshot_with_pending_tail(&prefix, &count.finish())).is_err()
    );
    let mut malformed_frame = prefix;
    malformed_frame.extend_from_slice(&100u32.to_be_bytes());
    assert!(restore_pending_snapshot(&malformed_frame).is_err());
}

#[tokio::test]
async fn restored_pending_correlations_prune_removed_conflicting_and_legacy_members() {
    let (mut alice, bob) = pair(true);
    let pending = alice.pending_finalization_snapshot().unwrap();
    alice.ingest_peer_record(bob.self_record().unwrap().clone());
    let prefix = snapshot_without_pending_tail(&mut alice);
    let restored =
        restore_pending_snapshot(&snapshot_with_pending_tail(&prefix, &pending)).unwrap();
    assert!(
        restored.member_finalization_pending.is_empty(),
        "a signed descriptor supersedes correlation"
    );
    assert!(restored.finalized_member_peers().is_empty());
    alice.request_remove(&bob.device_id()).await.unwrap();
    let prefix = snapshot_without_pending_tail(&mut alice);
    let mut restored =
        restore_pending_snapshot(&snapshot_with_pending_tail(&prefix, &pending)).unwrap();
    assert!(restored.member_finalization_candidates().is_empty());
    assert!(restored.member_finalization_pending.is_empty());

    let (mut legacy, member) = pair(false);
    let pending = pending_test_tail(
        PENDING_SNAPSHOT_VERSION,
        &[(member.device_id(), member.local_peer())],
    );
    let prefix = snapshot_without_pending_tail(&mut legacy);
    let mut restored =
        restore_pending_snapshot(&snapshot_with_pending_tail(&prefix, &pending)).unwrap();
    assert_eq!(restored.group_mode(), GroupMode::LegacyUnverified);
    assert!(restored.member_finalization_candidates().is_empty());
}
