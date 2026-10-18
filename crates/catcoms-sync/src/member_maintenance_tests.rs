// Included beside the existing real MLS/member-finalization fixture.
async fn detached_reply(alice: &mut Node, bob: &mut Node, kind: MemberMaintenanceKind) -> CompletedMemberMaintenance {
    detached_reply_with(alice, bob, kind, |_, _, response| response).await
}

async fn detached_reply_with(
    alice: &mut Node,
    bob: &mut Node,
    kind: MemberMaintenanceKind,
    mut reply: impl FnMut(&Node, &[u8], Vec<u8>) -> Vec<u8>,
) -> CompletedMemberMaintenance {
    let pending = bob.prepare_member_maintenance(alice.local_peer(), kind).unwrap().unwrap();
    let (_publisher, receiver) = tokio::sync::watch::channel(false);
    let (completed, ()) = tokio::join!(pending.execute(RequestCancellation::new(receiver, None)), async {
        loop {
            if let Some(TransportEvent::Request { from, data, responder, .. }) = alice.transport.next_event().await {
                let response = alice.handle_request(from, &data);
                responder.respond(Bytes::from(reply(alice, &data, response)));
                break;
            }
        }
    });
    completed
}

fn add_maintenance_member(alice: &mut Node, nonce: u8) -> DeviceId {
    // A real owner-authorized MLS Add, with delivery to the old member intentionally absent.
    let device = MlsDevice::generate().unwrap();
    let invite = alice.mint_invite([nonce; 16], 60_000, vec![]).unwrap();
    let package = device.key_package_for_invite(&invite.group_id, invite.invite_nonce).unwrap();
    assert_eq!(alice.serve_join(PeerId::from_u64(nonce as u64),
        &encode_join_req(&invite, &serialize_key_package(&package).unwrap())).unwrap()[0], 1);
    device.device_id()
}

#[tokio::test]
async fn detached_member_maintenance_recovers_quiet_and_equal_count_membership_changes() {
    let (mut alice, mut bob) = pair(true);
    let carol = add_maintenance_member(&mut alice, 30);
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Commits).await;
    assert!(bob.complete_member_maintenance(completed).unwrap());
    assert!(bob.group.contains_device(&carol));
    let count = bob.group.member_device_ids().len();
    let old_epoch = bob.group.epoch();
    alice.request_remove(&carol).await.unwrap();
    let dan = add_maintenance_member(&mut alice, 31);
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Commits).await;
    assert!(bob.complete_member_maintenance(completed).unwrap());
    assert_eq!(bob.group.member_device_ids().len(), count);
    assert!(bob.group.epoch() > old_epoch);
    assert!(!bob.group.contains_device(&carol));
    assert!(bob.group.contains_device(&dan));
    assert_eq!(bob.group.epoch(), alice.group.epoch());
    assert!(bob.member_peers.iter().all(|proof| !proof.bound),
        "released kind2 cannot upgrade a signer to endpoint-bound evidence");
    // The next stage must be signed at the recovered epoch, and ordinary additive finalization
    // remains the separate stronger exchange. A successful commit may revoke its old gate.
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Finalize).await;
    assert!(bob.complete_member_maintenance(completed).unwrap());
}

#[tokio::test]
async fn detached_member_maintenance_signed_commit_count_refuses_before_promotion_or_apply() {
    let (mut alice, mut bob) = pair(true);
    add_maintenance_member(&mut alice, 30);
    let epoch = bob.group.epoch();
    let completed = detached_reply_with(&mut alice, &mut bob, MemberMaintenanceKind::Commits,
        |alice, request, _| {
            let (_, pubkey, ts, nonce, epoch, _) = decode_authed_request(&request[1..]).unwrap();
            let mut e = Encoder::new();
            e.put_u32(MAX_COMMIT_BUNDLE_RECORDS + 1);
            let bundle = e.finish();
            let transcript = catchup_resp_transcript(&alice.group.group_id(), &pubkey, ts, &nonce, epoch, &bundle);
            encode_signed_commit_resp(&alice.device.public_key_bytes(), &alice.device.sign(&transcript).unwrap(), &bundle)
        }).await;
    assert!(matches!(bob.complete_member_maintenance(completed),
        Err(MemberMaintenanceError::Failed(SyncError::Malformed))));
    assert_eq!(bob.group.epoch(), epoch);
    assert!(bob.member_peers.is_empty());
    assert!(bob.pending_commits.is_empty());
    assert_eq!(bob.stats.commits_applied, 0);
    assert_eq!(bob.stats.commits_buffered, 0);
}

#[tokio::test]
async fn detached_member_maintenance_rejects_oversized_and_wrong_request_commit_replies() {
    let (mut alice, mut bob) = pair(true);
    add_maintenance_member(&mut alice, 30);
    let epoch = bob.group.epoch();
    let oversized = detached_reply_with(&mut alice, &mut bob, MemberMaintenanceKind::Commits,
        |_, _, _| vec![0; MAX_CATCHUP_RESPONSE + 1]).await;
    assert!(matches!(bob.complete_member_maintenance(oversized), Err(MemberMaintenanceError::Failed(_))));
    assert_eq!(bob.group.epoch(), epoch);
    assert!(bob.member_peers.is_empty());
    let mut captured = Vec::new();
    let _lost_reply = detached_reply_with(&mut alice, &mut bob, MemberMaintenanceKind::Commits,
        |_, _, response| { captured = response.clone(); response }).await;
    let replay = detached_reply_with(&mut alice, &mut bob, MemberMaintenanceKind::Commits,
        |_, _, _| captured.clone()).await;
    assert!(!bob.complete_member_maintenance(replay).unwrap());
    assert_eq!(bob.group.epoch(), epoch);
    assert!(bob.member_peers.is_empty());
    let current = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Commits).await;
    assert!(bob.complete_member_maintenance(current).unwrap());
    assert_eq!(bob.group.epoch(), alice.group.epoch());
}

#[tokio::test]
async fn detached_member_maintenance_authenticates_current_finalization() {
    let (mut alice, mut bob) = pair(true);
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Finalize).await;
    assert!(bob.complete_member_maintenance(completed).unwrap());
    assert_eq!(bob.finalized_member_peers(), vec![alice.local_peer()]);
}

#[tokio::test]
async fn detached_member_maintenance_driver_reconnect_rejects_old_work_without_root_events() {
    for completed_first in [false, true] {
        let (mut alice, mut bob) = pair(true);
        let clock = Arc::new(ManualClock::new(1000));
        alice.clock = clock.clone(); bob.clock = clock.clone();
        let pending = if completed_first { None } else { Some(bob.prepare_member_maintenance(alice.local_peer(), MemberMaintenanceKind::Finalize).unwrap().unwrap()) };
        let completed = if completed_first { Some(detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Finalize).await) } else { None };
        alice.transport = Arc::new(alice.transport.rejoin());
        let completed = if let Some(pending) = pending {
            let (_publisher, signal) = tokio::sync::watch::channel(false);
            pending.execute(RequestCancellation::new(signal, None)).await
        } else { completed.unwrap() };
        assert!(matches!(bob.complete_member_maintenance(completed), Err(MemberMaintenanceError::Superseded)));
        clock.advance_ms(MIN_PEX_INTERVAL_MS);
        let current = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Finalize).await;
        // The old ordered edge cannot revoke a freshly captured driver connection owner.
        bob.note_peer_disconnected(alice.local_peer());
        assert!(bob.complete_member_maintenance(current).unwrap());
    }
}

#[tokio::test]
async fn detached_member_maintenance_actual_signed_removal_rejects_a_held_reply() {
    for kind in [MemberMaintenanceKind::Finalize, MemberMaintenanceKind::Commits] {
    let (mut alice, mut bob) = pair(true);
    let completed = detached_reply(&mut alice, &mut bob, kind).await;
    alice.request_remove(&bob.device.device_id()).await.unwrap();
    let mut control = vec![CTRL_COMMIT];
    control.extend_from_slice(&alice.commit_log.back().unwrap().encode());
    bob.on_control(alice.local_peer(), &control);
    assert!(!bob.group.is_active());
    assert!(matches!(bob.complete_member_maintenance(completed), Err(MemberMaintenanceError::Superseded)));
    assert!(bob.finalized_member_peers().is_empty());
    }
}

#[tokio::test]
async fn detached_member_maintenance_expired_answer_is_a_deadline_failure() {
    for kind in [MemberMaintenanceKind::Finalize, MemberMaintenanceKind::Commits] {
    let (mut alice, mut bob) = pair(true);
    let clock = ManualClock::new(1_000);
    bob.clock = Arc::new(clock.clone());
    let completed = detached_reply(&mut alice, &mut bob, kind).await;
    clock.advance_ms(if kind == MemberMaintenanceKind::Finalize { 2_000 } else { 3_000 });
    assert!(matches!(bob.complete_member_maintenance(completed), Err(MemberMaintenanceError::Failed(SyncError::Transport(TransportError::Timeout(_))))));
    assert!(bob.finalized_member_peers().is_empty());
    }
}

#[tokio::test]
async fn detached_member_maintenance_old_root_commit_reply_cannot_apply_to_restored_owner() {
    let (mut alice, mut bob) = pair(true);
    add_maintenance_member(&mut alice, 30);
    let epoch = bob.group.epoch();
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Commits).await;
    let mut restored = Node::restore(&bob.snapshot().unwrap(), Hub::new().join(bob.local_peer()),
        ChaCha20Rng::seed_from_u64(71), Box::new(ManualClock::new(1_000))).unwrap();
    assert!(matches!(restored.complete_member_maintenance(completed), Err(MemberMaintenanceError::Superseded)));
    assert_eq!(restored.group.epoch(), epoch);
    assert!(restored.member_peers.is_empty());
}

#[test]
fn detached_member_maintenance_duplicate_descriptor_does_not_dirty_snapshot_revision() {
    let (alice, mut bob) = pair(false);
    let record = alice.self_record().unwrap().clone();
    let before = bob.peer_record_revision();
    assert!(bob.ingest_peer_record(record.clone()));
    let stored = bob.peer_record_revision();
    assert_ne!(stored, before);
    assert!(!bob.ingest_peer_record(record));
    assert_eq!(bob.peer_record_revision(), stored);
    bob.note_peer_disconnected(alice.local_peer());
    bob.note_peer_connected(alice.local_peer());
    assert_eq!(bob.peer_record_revision(), stored, "liveness is not durable descriptor mutation");
}

#[tokio::test]
async fn detached_member_maintenance_rejects_disconnect_reconnect_and_descriptor_replacement() {
    for replacement in [false, true] {
        let (mut alice, mut bob) = pair(true);
        bob.ingest_peer_record(alice.self_record().unwrap().clone());
        let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Finalize).await;
        if replacement {
            alice.publish_self_record(Vec::new(), 65_537).unwrap();
            bob.ingest_peer_record(alice.self_record().unwrap().clone());
        } else {
            // Model the driver's actual aggregate close before its ordered Root event.
            alice.transport = Arc::new(alice.transport.rejoin());
            bob.note_peer_disconnected(alice.local_peer());
            bob.note_peer_connected(alice.local_peer());
        }
        assert!(matches!(bob.complete_member_maintenance(completed), Err(MemberMaintenanceError::Superseded)));
        assert!(bob.finalized_member_peers().is_empty());
    }
}

#[tokio::test]
async fn detached_member_maintenance_policy_activation_and_root_replacement_are_irreversible() {
    let (mut alice, mut bob) = pair(true);
    bob.group_policy_active = false;
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Pex).await;
    bob.group_policy_active = true;
    bob.reconcile_member_maintenance();
    bob.group_policy_active = false;
    bob.reconcile_member_maintenance();
    assert!(matches!(bob.complete_member_maintenance(completed), Err(MemberMaintenanceError::Superseded)));
    bob.group_policy_active = true;
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Finalize).await;
    let mut restored = Node::restore(&bob.snapshot().unwrap(), Hub::new().join(bob.local_peer()), ChaCha20Rng::seed_from_u64(71), Box::new(ManualClock::new(1_000))).unwrap();
    assert!(matches!(restored.complete_member_maintenance(completed), Err(MemberMaintenanceError::Superseded)));
    assert!(restored.finalized_member_peers().is_empty());
}

#[tokio::test]
async fn detached_member_maintenance_preserves_legacy_pex_without_bound_proof() {
    let (mut alice, mut bob) = pair(false);
    assert!(bob.prepare_member_maintenance(alice.local_peer(), MemberMaintenanceKind::Finalize).unwrap().is_none());
    let completed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Pex).await;
    assert!(bob.complete_member_maintenance(completed).unwrap());
    assert!(!bob.member_peers.iter().any(|proof| proof.bound));
}

async fn offered_pair() -> (Node, Node) {
    let alice_lib_peer = libp2p::identity::Keypair::ed25519_from_bytes([11; 32]).unwrap().public().to_peer_id();
    let bob_lib_peer = libp2p::identity::Keypair::ed25519_from_bytes([12; 32]).unwrap().public().to_peer_id();
    let (mut alice, mut bob) = pair_with_peers(true, catcoms_net::phase0_peer_id(&alice_lib_peer), catcoms_net::phase0_peer_id(&bob_lib_peer));
    alice.publish_self_record(vec![format!("/ip4/203.0.113.1/tcp/22487/p2p/{alice_lib_peer}")], 65_537).unwrap();
    bob.ingest_peer_record(alice.self_record().unwrap().clone());
    alice.set_switchboard_offered(true);
    let offered = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Offer).await;
    assert!(bob.complete_member_maintenance(offered).unwrap());
    assert_eq!(bob.switchboard_offers.len(), 1);
    (alice, bob)
}

#[tokio::test]
async fn detached_member_maintenance_current_empty_offer_removes_only_current_peer_cache() {
    let (mut alice, mut bob) = offered_pair().await;
    alice.set_switchboard_offered(false);
    let empty = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Offer).await;
    assert!(!bob.complete_member_maintenance(empty).unwrap());
    assert!(bob.switchboard_offers.is_empty());
}

#[tokio::test]
async fn detached_member_maintenance_superseded_empty_offer_cannot_remove_newer_cache() {
    for fault in 0..4 {
        let (mut alice, mut bob) = offered_pair().await;
        alice.set_switchboard_offered(false);
        let empty = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Offer).await;
        match fault {
            0 => {
                alice.publish_self_record(vec![alice.self_record().unwrap().addresses[0].replace("/tcp/22487/", "/tcp/22488/")], 65_538).unwrap();
                bob.ingest_peer_record(alice.self_record().unwrap().clone());
            },
            1 => {
                bob.group_policy_active = false; bob.reconcile_member_maintenance();
                bob.group_policy_active = true; bob.reconcile_member_maintenance();
            },
            2 => { alice.transport = Arc::new(alice.transport.rejoin()); bob.note_peer_disconnected(alice.local_peer()); bob.note_peer_connected(alice.local_peer()); },
            _ => {
                // A fresh owner may already have its own validated cache when an old owner reply
                // arrives. Retain the already verified offer here to isolate the instance check.
                let offer = bob.switchboard_offers.values().next().unwrap().clone();
                bob = Node::restore(&bob.snapshot().unwrap(), Hub::new().join(bob.local_peer()), ChaCha20Rng::seed_from_u64(72), Box::new(ManualClock::new(1_000))).unwrap();
                bob.switchboard_offers.insert(offer.device_id(), offer);
            },
        }
        if fault < 3 {
            alice.set_switchboard_offered(true);
            alice.switchboard_offer_served_at.clear();
            let refreshed = detached_reply(&mut alice, &mut bob, MemberMaintenanceKind::Offer).await;
            assert!(bob.complete_member_maintenance(refreshed).unwrap());
        }
        let expected = bob.switchboard_offers.clone();
        assert!(matches!(bob.complete_member_maintenance(empty), Err(MemberMaintenanceError::Superseded)));
        assert_eq!(bob.switchboard_offers, expected, "stale empty response erased fresh owner cache, fault {fault}");
    }
}
