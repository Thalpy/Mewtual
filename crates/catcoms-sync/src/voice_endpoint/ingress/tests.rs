use super::*;
use crate::voice_endpoint::tests::{attach, Node};
use catcoms_rt::ManualClock;
use voice_v2::runtime::Runtime;
mod protected;
mod protected_start;

async fn fixture() -> (Node, Node, Arc<Runtime>, Arc<ManualClock>) {
    fixture_with_clock(|clock| clock).await
}
async fn fixture_with_clock(
    make_clock: impl FnOnce(Arc<ManualClock>) -> Arc<dyn Clock>,
) -> (Node, Node, Arc<Runtime>, Arc<ManualClock>) {
    let (_hub, mut nodes, _) = crate::tests::build_members(2).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    nodes[0]
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    nodes[0].snapshot().unwrap();
    nodes[0].publish_group_policy().unwrap();
    let policy = nodes[0].group_policy.as_ref().unwrap().encode();
    nodes[1].on_group_policy(&policy);
    // Keep sender/receiver canonical for protected Hello tests without replacing either actor.
    nodes.sort_by_key(|node| node.device_id());
    let mut receiver = nodes.pop().unwrap();
    let mut sender = nodes.pop().unwrap();
    let clock = Arc::new(ManualClock::new(1000));
    sender.clock = clock.clone();
    receiver.clock = clock.clone();
    let runtime = Runtime::new(make_clock(clock.clone()));
    attach(&mut receiver, &runtime);
    let (ok, served) = tokio::join!(
        sender.finalize_member_connection(receiver.local_peer()),
        receiver.run_once()
    );
    assert!(ok.unwrap() && served.unwrap());
    // Both MemNetwork handles retain the hub; this fixture does not alter connectivity.
    (sender, receiver, runtime, clock)
}

#[tokio::test]
async fn causal_ready_never_enters_queued_public_intake_or_produces_generic_receipt() {
    let (mut low, high, _high_runtime, clock) = fixture().await;
    let runtime = Runtime::new(clock);
    attach(&mut low, &runtime);
    let grant = low.start_voice_watch(1, CallLease::default()).unwrap();
    let envelope = Envelope::Direct(
        SignedObject::issue(
            grant.scope(),
            &high.device,
            [71; 32],
            1000,
            16_000,
            Body::PairReady {
                header: voice_v2::ReadyHeader {
                    epoch: low.group.epoch(),
                    target: low.device_id(),
                    source_join: [1; 32],
                    target_join: [2; 32],
                    source_key_ref: [3; 32],
                    target_key_ref: [4; 32],
                    handshake: [5; 32],
                    finish_message: [6; 32],
                    finish_object_hash: [7; 32],
                    helpers: vec![],
                },
            },
        )
        .unwrap(),
    );
    let context = low.voice_v2_context(grant.scope().room).unwrap();
    assert_eq!(
        envelope.authorize(high.device_id(), &context, |_| true),
        Ok(voice_v2::Disposition::Deliver)
    );
    for public_only in [false, true] {
        let room = low.voice_room_authority(context.scope.room).unwrap();
        let ticket = low
            .voice_transport_actor()
            .unwrap()
            .admit_transport_verification(high.device_id(), &envelope.encode())
            .unwrap();
        let error = if public_only {
            low.admit_voice_endpoint_public(ticket, envelope.clone(), &context, room)
                .err()
        } else {
            low.admit_voice_endpoint(ticket, envelope.clone(), &context, room)
                .err()
        };
        assert_eq!(error, Some(VoiceError::Unauthorized));
        let inbox = &low.voice_runtime.as_ref().unwrap().endpoints.public;
        assert!(inbox.packets.is_empty() && inbox.encrypted.is_empty());
    }
}

#[tokio::test]
async fn locked_endpoint_housekeeping_releases_expired_public_cache_after_actual_receipt_drop() {
    let (sender, mut receiver, runtime, clock) = fixture().await;
    let grant = receiver.start_voice_watch(1, CallLease::default()).unwrap();
    let envelope = announcement(&sender, grant.scope());
    let receipt = admit(&mut receiver, envelope);
    runtime.revoke_session();
    clock.advance_ms(60_001);
    receiver.collect_expired_voice();
    assert!(!receipt.authorized());
    let owners = &receiver.voice_runtime.as_ref().unwrap().endpoints;
    assert!(owners.observers.is_empty());
    assert_eq!(
        owners.public.packets.len(),
        1,
        "actual receipt still owns queue custody"
    );
    let held = runtime.resources().counts().0;
    drop(receipt);
    clock.advance_ms(1000);
    receiver.collect_expired_voice();
    assert!(receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .public
        .packets
        .is_empty());
    assert!(runtime.resources().counts().0 < held);
    assert_eq!(
        receiver.voice_housekeeping_deadline(),
        Some(clock.monotonic_ms() + 1000)
    );
}

#[tokio::test]
async fn public_receipt_waits_for_all_original_watchers_without_spinning_on_the_first_retired_one()
{
    use futures::FutureExt;
    let (sender, mut receiver, _runtime, _clock) = fixture().await;
    let first_parent = CallLease::default();
    let second_parent = CallLease::default();
    let first = receiver.start_voice_watch(1, first_parent.clone()).unwrap();
    let _second = receiver
        .start_voice_watch(1, second_parent.clone())
        .unwrap();
    let receipt = admit(&mut receiver, announcement(&sender, first.scope()));
    let mut waiting = Box::pin(receipt.revoked());
    assert!(waiting.as_mut().now_or_never().is_none());
    first_parent.revoke();
    assert!(
        receipt.authorized(),
        "the second original watcher still admitted this packet"
    );
    assert!(waiting.as_mut().now_or_never().is_none());
    // A second poll must stay Pending as well, rather than repeatedly selecting the same
    // already-revoked watcher while another original target remains live.
    assert!(waiting.as_mut().now_or_never().is_none());
    second_parent.revoke();
    assert!(waiting.as_mut().now_or_never().is_some());
    assert!(!receipt.authorized());
}
fn announcement(sender: &Node, scope: Scope) -> Envelope {
    let key = JoinKey::new([9; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap();
    Envelope::Direct(
        SignedObject::issue(
            scope,
            &sender.device,
            [41; 32],
            1000,
            31_000,
            Body::Announcement {
                key,
                epoch: sender.group.epoch(),
                renewal: 1,
            },
        )
        .unwrap(),
    )
}
fn admit(receiver: &mut Node, envelope: Envelope) -> PublicReceipt {
    let context = receiver
        .voice_v2_context(envelope.object().scope().room)
        .unwrap();
    // This unit starts at the private native intake seam, after real bound-route establishment.
    // Other transport tests cover outer framing/signatures and the final ACK return path.
    assert!(receiver
        .voice_bound_routes()
        .iter()
        .any(|route| route.device == envelope.object().issuer()));
    let room = receiver.voice_room_authority(context.scope.room).unwrap();
    let ticket = receiver
        .voice_transport_actor()
        .unwrap()
        .admit_transport_verification(envelope.object().issuer(), &envelope.encode())
        .unwrap();
    receiver
        .admit_voice_endpoint_public(ticket, envelope, &context, room)
        .unwrap()
}

#[tokio::test]
async fn public_source_capacity_refusal_reuses_exact_native_challenge_without_new_nonces() {
    let (sender, mut receiver, runtime, _clock) = fixture().await;
    let grant = receiver.start_voice_watch(1, CallLease::default()).unwrap();
    let envelope = announcement(&sender, grant.scope());
    let key = (envelope.object().issuer(), envelope.object().id());
    drop(admit(&mut receiver, envelope));
    let expected = receiver
        .process_voice_public(key, grant.id)
        .unwrap()
        .unwrap()
        .0;
    let pool = runtime.resources();
    let baseline = pool.counts().0;
    // Allow native source canonical scratch but insufficient persistent source retention.
    // Its exact challenge has already been created; no crypto/nonce work may be repeated.
    let full = pool
        .charge(
            grant.scope.group,
            crate::voice_v2::resources::GROUP_BYTES - baseline - 6144 - 512,
            ResourceKind::Retained,
        )
        .unwrap();
    receiver.pump_voice_endpoint().unwrap();
    let ledger = receiver.voice_transport_actor().unwrap().source().unwrap();
    assert_eq!(ledger.counts(), (0, 0, 0));
    assert_eq!(
        receiver
            .process_voice_public(key, grant.id)
            .unwrap()
            .unwrap()
            .0,
        expected
    );
    assert!(receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .public
        .packets[&key]
        .processed
        .is_empty());
    drop(full);
    receiver.pump_voice_endpoint().unwrap();
    assert_eq!(ledger.counts(), (1, 0, 0));
    let actor = receiver.voice_transport_actor().unwrap();
    let job = ledger
        .take_ready(actor, &receiver.voice_bound_routes())
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(job.object(), &expected);
    assert_eq!(ledger.counts(), (1, 1, 1));
}

#[tokio::test]
async fn public_expiry_rollback_and_retained_receipt_cannot_seed_a_replacement_watcher() {
    let (sender, mut receiver, runtime, clock) = fixture().await;
    let grant = receiver.start_voice_watch(1, CallLease::default()).unwrap();
    let envelope = announcement(&sender, grant.scope());
    // Install the independent requester record before sampling retained payload bytes.
    drop(
        receiver
            .voice_transport_actor()
            .unwrap()
            .admit_transport_verification(sender.device_id(), b"warm")
            .unwrap(),
    );
    let baseline = runtime.resources().counts().0;
    let receipt = admit(&mut receiver, envelope);
    let retained_bytes = runtime.resources().counts().0 - baseline;
    assert!(receipt.authorized());
    clock.advance_ms(35_001);
    receiver.pump_voice_endpoint().unwrap();
    assert!(!receipt.authorized());
    assert_eq!(
        runtime.resources().counts().0,
        baseline + retained_bytes,
        "held response retains its actual packet charge"
    );
    clock.set_wall_ms(1000);
    let replacement = receiver.start_voice_watch(1, CallLease::default()).unwrap();
    receiver.pump_voice_endpoint().unwrap();
    assert!(
        !receipt.authorized(),
        "wall rollback never revives historical admission"
    );
    assert!(receiver.voice_watch_peers(&replacement).unwrap().is_empty());
    assert_eq!(
        receiver
            .voice_transport_actor()
            .unwrap()
            .source()
            .unwrap()
            .counts(),
        (0, 0, 0),
        "expired candidate cannot create fresh watcher source work"
    );
    let held = runtime.resources().counts().0;
    drop(receipt);
    receiver.pump_voice_endpoint().unwrap();
    assert!(receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .public
        .packets
        .is_empty());
    assert_eq!(runtime.resources().counts().0, held - retained_bytes);
}
