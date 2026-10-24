use super::*;
use crate::tests::{build_members, converge_and_publish_test_routes};
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[tokio::test]
async fn detached_fetch_authenticates_each_response_before_storage() {
    let (_hub, mut members, ids) = build_members(3).await;
    converge_and_publish_test_routes(&mut members);
    let mut members = members.into_iter();
    let alice = members.next().unwrap();
    let mut bob = members.next().unwrap();
    let carol = members.next().unwrap();
    let outsider = MlsDevice::generate().unwrap();
    let peer = alice.local_peer();
    bob.promote_member_peer(peer, ids[0]);
    let cid = Cid::of(b"pixels");
    // Even another current member cannot answer an attempt prepared for Alice. The successful
    // final response also proves rejected responses have not poisoned the CID or provider state.
    for fault in [
        "signature",
        "cid",
        "nonce",
        "epoch",
        "group",
        "member",
        "size",
        "unproven-outsider",
        "ok",
    ] {
        if fault == "unproven-outsider" {
            // Restart bootstrap has no expected signer, but still requires current membership.
            bob.member_peers.clear();
            bob.remember_peer(peer);
        } else {
            bob.promote_member_peer(peer, ids[0]);
        }
        let max = if fault == "size" { 5 } else { 6 };
        let request = bob.prepare_blob_fetch(peer, cid, max).unwrap();
        let (_signal, receiver) = tokio::sync::watch::channel(false);
        let (completed, ()) = tokio::join!(
            request.fetch(RequestCancellation::new(receiver, None)),
            async {
                let Some(TransportEvent::Request {
                    data, responder, ..
                }) = alice.transport.next_event().await
                else {
                    panic!("expected blob request")
                };
                assert_eq!(data[0], KIND_BLOB_FETCH);
                let (_, requester, ts, mut nonce, mut epoch, _) =
                    decode_authed_request(&data[1..]).unwrap();
                if fault == "nonce" {
                    nonce[0] ^= 1;
                }
                if fault == "epoch" {
                    epoch += 1;
                }
                let mut group = alice.group.group_id();
                if fault == "group" {
                    group[0] ^= 1;
                }
                let blob = if fault == "cid" { b"wrong!" } else { b"pixels" };
                let signer = if fault == "member" {
                    &carol.device
                } else if fault == "unproven-outsider" {
                    &outsider
                } else {
                    &alice.device
                };
                let transcript =
                    blob_fetch_resp_transcript(&group, &requester, ts, &nonce, epoch, blob);
                let mut signature = signer.sign(&transcript).unwrap();
                if fault == "signature" {
                    signature[0] ^= 1;
                }
                responder.respond(Bytes::from(encode_signed_commit_resp(
                    &signer.public_key_bytes(),
                    &signature,
                    blob,
                )));
            }
        );
        let result = bob.complete_blob_fetch(completed);
        if fault == "ok" {
            assert_eq!(result.unwrap(), Some(roles::fingerprint(&ids[0])));
            assert_eq!(bob.get_blob(&cid).unwrap(), b"pixels");
        } else {
            assert!(result.is_err(), "{fault}");
            if fault == "unproven-outsider" {
                assert!(bob.member_peers.is_empty());
            }
            assert!(
                bob.blob_cids().is_empty(),
                "{fault} cannot write either CID"
            );
        }
    }
}

#[tokio::test]
async fn detached_response_cannot_be_committed_to_a_restored_owner() {
    let (_hub, members, ids) = build_members(2).await;
    let mut members = members.into_iter();
    let mut alice = members.next().unwrap();
    let mut bob = members.next().unwrap();
    let cid = alice.put_blob(b"pixels").unwrap();
    bob.promote_member_peer(alice.local_peer(), ids[0]);
    let request = bob.prepare_blob_fetch(alice.local_peer(), cid, 6).unwrap();
    let (_signal, receiver) = tokio::sync::watch::channel(false);
    let (completed, _) = tokio::join!(
        request.fetch(RequestCancellation::new(receiver, None)),
        alice.run_once()
    );
    let snapshot = bob.snapshot().unwrap();
    let mut restored = ChannelSync::restore(
        &snapshot,
        bob.transport().clone(),
        ChaCha20Rng::seed_from_u64(44),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert_eq!(restored.epoch(), bob.epoch());
    assert!(restored.complete_blob_fetch(completed).is_err());
    assert!(restored.blob_cids().is_empty());
}

#[tokio::test]
async fn detached_provider_selection_bounds_and_prefers_live_member_proofs() {
    let (_hub, mut members, ids) = build_members(6).await;
    converge_and_publish_test_routes(&mut members);
    let peers: Vec<_> = members.iter().map(ChannelSync::local_peer).collect();
    let owner = &mut members[0];
    owner.known_peers.clear();
    // A descriptor alone is not a source. The explicit-read bootstrap requires a known live peer.
    assert!(owner
        .prepare_blob_fetch(peers[1], Cid::of(b"x"), 1)
        .is_err());
    owner.remember_peer(peers[1]);
    assert!(owner.prepare_blob_fetch(peers[1], Cid::of(b"x"), 1).is_ok());
    for i in 1..6 {
        owner.promote_member_peer(peers[i], ids[i]);
    }
    let selected = owner.blob_fetch_peers();
    assert_eq!(selected.len(), MAX_BLOB_FETCH_PEERS);
    assert_eq!(
        selected,
        peers[2..].iter().rev().copied().collect::<Vec<_>>()
    );
    assert!(owner
        .prepare_blob_fetch(selected[0], Cid::of(b"x"), MAX_BOUNDED_BLOB_BYTES + 1)
        .is_err());
    let outsider = PeerId::from_u64(999);
    owner.promote_member_peer(outsider, ids[1]);
    assert!(
        !owner.blob_fetch_peers().contains(&outsider),
        "a stale proof cannot create a connection"
    );
}

#[tokio::test]
async fn detached_connected_candidate_bootstraps_bytes_without_promoting_endpoint_trust() {
    let (_hub, members, _ids) = build_members(2).await;
    let mut members = members.into_iter();
    let mut alice = members.next().unwrap();
    let mut bob = members.next().unwrap();
    bob.member_peers.clear();
    bob.remember_peer(alice.local_peer());
    let cid = alice.put_blob(b"restart bytes").unwrap();
    let request = bob.prepare_blob_fetch(alice.local_peer(), cid, 13).unwrap();
    let (_signal, receiver) = tokio::sync::watch::channel(false);
    let (completed, _) = tokio::join!(
        request.fetch(RequestCancellation::new(receiver, None)),
        alice.run_once()
    );
    assert!(bob.complete_blob_fetch(completed).unwrap().is_some());
    assert_eq!(bob.get_blob(&cid).unwrap(), b"restart bytes");
    assert!(
        bob.member_peers.is_empty(),
        "a signed blob is not endpoint-bound membership proof"
    );
}

#[tokio::test]
async fn membership_change_rejects_a_completed_response_before_storage() {
    let (_hub, members, ids) = build_members(2).await;
    let mut members = members.into_iter();
    let mut alice = members.next().unwrap();
    let mut bob = members.next().unwrap();
    let cid = bob.put_blob(b"pixels").unwrap();
    alice.promote_member_peer(bob.local_peer(), ids[1]);
    let request = alice.prepare_blob_fetch(bob.local_peer(), cid, 6).unwrap();
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let (completed, _) = tokio::join!(
        request.fetch(RequestCancellation::new(receiver, None)),
        bob.run_once()
    );
    // Remove first stages a contest; only its committed epoch invalidates the response.
    let clock = ManualClock::new(alice.clock.now_ms());
    alice.clock = Arc::new(clock.clone());
    let previous_epoch = alice.epoch();
    alice.remove(&ids[1]).await.unwrap();
    clock.advance_ms(alice.config.stage_decision_window_ms);
    assert!(alice.resolve_pending_if_expired());
    assert!(alice.epoch() > previous_epoch);
    assert!(!alice.contains_member(&ids[1]));
    assert!(alice.complete_blob_fetch(completed).is_err());
    assert!(alice.blob_cids().is_empty());
}
