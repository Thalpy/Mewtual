use super::super::runtime::Runtime;
use super::*;
use catcoms_rt::ManualClock;
use std::sync::atomic::Ordering;

type Node = crate::ChannelSync<catcoms_rt::mem::MemNetwork, rand_chacha::ChaCha20Rng>;
type Fixture = (
    Node,
    Arc<Runtime>,
    Arc<ManualClock>,
    Arc<CallGrant>,
    super::super::call_activation::ActivationOwner,
    ActivationObserver,
    JoinKey,
    Node,
);
async fn fixture() -> Fixture {
    let (_, mut nodes, _) = crate::tests::build_members(2).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    // Finalization refuses immediately without actual P2P policy. Pairing that
    // refusal with an idle remote.run_once in join! would wait forever because
    // there is no request and the deterministic application Clock does not tick.
    nodes[0]
        .initialize_group_policy(crate::GroupMode::PeerToPeer)
        .unwrap();
    nodes[0].snapshot().unwrap();
    nodes[0].publish_group_policy().unwrap();
    let policy = nodes[0].group_policy.as_ref().unwrap().encode();
    nodes[1].on_group_policy(&policy);
    let mut remote = nodes.pop().unwrap();
    let mut node = nodes.pop().unwrap();
    let clock = Arc::new(ManualClock::new(1000));
    node.clock = clock.clone();
    remote.clock = clock.clone();
    assert!(node.policy_allows_member_mesh() && remote.policy_allows_member_mesh());
    // Only the outer harness watchdog uses the sanctioned real Clock seam.
    // Protocol freshness, retries and ownership still use ManualClock below.
    let watchdog = catcoms_rt::SystemClock;
    let (connected, served) = tokio::select! {
        result = async { tokio::join!(
            node.finalize_member_connection(remote.local_peer()), remote.run_once()
        ) } => result,
        _ = watchdog.sleep(std::time::Duration::from_secs(30)) => panic!("admitted presence fixture finalization stalled"),
    };
    assert!(connected.unwrap() && served.unwrap());
    let runtime = Runtime::new(clock.clone());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    node.prepare_voice_actor(
        &runtime,
        &ticket,
        991,
        |_| Ok(std::collections::BTreeSet::from([1])),
        |_| Ok(std::collections::BTreeSet::new()),
    )
    .unwrap()
    .register()
    .unwrap();
    let (grant, owner) = node.prepare_admitted_voice_call(1).unwrap().into_parts();
    let original = owner.observer();
    node.select_admitted_voice_call(&grant, &original)
        .unwrap()
        .commit_native(&grant)
        .unwrap();
    let key = JoinKey::new([61; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap();
    drop(
        node.start_admitted_voice_presence(
            grant.clone(),
            original.clone(),
            key.join,
            key.public_key,
        )
        .unwrap(),
    );
    (node, runtime, clock, grant, owner, original, key, remote)
}

#[tokio::test]
async fn admitted_presence_actual_epoch_transition_closes_original_discovery_and_retry() {
    let (mut node, _runtime, clock, grant, _owner, original, key, _remote) = fixture().await;
    let context = node.voice_v2_context(1).unwrap();
    let (object, activity) = announcement(&mut node, &context, clock.as_ref());
    let room = node.voice_room_authority(1).unwrap();
    let members = node.group.member_device_ids().into_iter().collect();
    node.voice_runtime
        .as_mut()
        .unwrap()
        .current
        .as_mut()
        .unwrap()
        .sync_view(super::super::runtime::View {
            epoch: context.epoch + 1,
            members,
            policy: context.scope.policy,
            eligible: true,
            indirect: context.indirect,
        })
        .unwrap();
    assert!(original.is_closed());
    let actor = node
        .voice_runtime
        .as_ref()
        .unwrap()
        .current
        .as_ref()
        .unwrap();
    assert_eq!(
        actor
            .source()
            .unwrap()
            .admit_admitted_discovery(actor, object, activity, room),
        Err(VoiceError::Unauthorized)
    );
    assert!(node
        .start_admitted_voice_presence(grant, original, key.join, key.public_key)
        .is_err());
}
fn announcement(
    node: &mut Node,
    context: &Context,
    clock: &dyn Clock,
) -> (SignedObject, AdmittedDiscoveryActivity) {
    let presence = node
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .admitted_presence_for_test()
        .unwrap();
    let object = presence
        .announce(&node.device, context, &|_| true, clock, &mut node.rng)
        .unwrap();
    (object, presence.activity().unwrap())
}

#[tokio::test]
async fn admitted_discovery_final_publication_and_live_flights_check_native_active_phase() {
    for close_before_publication in [true, false] {
        let (mut node, runtime, clock, grant, _owner, original, key, _remote) = fixture().await;
        let context = node.voice_v2_context(1).unwrap();
        let (object, activity) = announcement(&mut node, &context, clock.as_ref());
        let room = node.voice_room_authority(1).unwrap();
        let actor = node.voice_transport_actor().unwrap();
        let source = actor.source().unwrap();
        let before = runtime.resources().counts();
        // Even possession of internal matching bytes/lease cannot enter through
        // the ordinary Source API. Existing factories stay profile-rejecting.
        let raw = SourceActivity::joined(context.scope, context.local, key, grant.lease());
        assert_eq!(
            source.admit(actor, object.clone(), raw, room.clone()),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(runtime.resources().counts(), before);
        let lease = grant.lease();
        let result = source.admit_admitted_discovery_before_final_for_test(
            actor,
            object,
            activity,
            room,
            || {
                if close_before_publication {
                    // Model the real invalidate store ordering, stopping in its tiny
                    // CLOSED-before-live=false interval. A live-bit check would fail.
                    lease
                        .activation_phase()
                        .store(super::super::call_activation::CLOSED, Ordering::Release);
                }
            },
        );
        if close_before_publication {
            assert_eq!(result, Err(VoiceError::Unauthorized));
            assert!(lease.is_live());
            assert_eq!(source.counts(), (0, 0, 0));
            assert_eq!(runtime.resources().counts(), before);
        } else {
            assert_eq!(result, Ok(false));
            let job = source
                .take_ready(actor, &node.voice_bound_routes())
                .unwrap()
                .pop()
                .unwrap();
            let actual_lower = job.keepalive();
            assert!(job.authority().authorized());
            let charged = runtime.resources().counts();
            lease
                .activation_phase()
                .store(super::super::call_activation::CLOSED, Ordering::Release);
            assert!(lease.is_live());
            assert!(!job.authority().authorized());
            assert!(source
                .take_ready(actor, &node.voice_bound_routes())
                .unwrap()
                .is_empty());
            assert_eq!(
                source.complete(job.finish(None)).unwrap(),
                super::super::source::SourceOutcome::Retired
            );
            assert_eq!(
                runtime.resources().counts(),
                charged,
                "closure does not refund an actual Lower owner"
            );
            drop(actual_lower);
        }
        original.close();
    }
}

#[tokio::test]
async fn admitted_presence_actual_root_clear_returns_box_and_watcher_before_residence_marker() {
    let (mut node, runtime, _clock, grant, _owner, original, key, _remote) = fixture().await;
    let receipt = node
        .start_admitted_voice_presence(grant.clone(), original.clone(), key.join, key.public_key)
        .unwrap();
    assert!(original.root_returned().is_none());
    let retained = runtime.resources().counts().0;
    node.clear_admitted_voice_call(&original).unwrap();
    assert!(original.root_returned().unwrap().matches(&original));
    assert!(
        retained - runtime.resources().counts().0 >= PRESENCE_BYTES + RECORD_BYTES,
        "same Root clear has actually dropped presence storage and watcher fees"
    );
    assert!(receipt.matches(&original, &grant, key.join, key.public_key));
    // Historical receipt/issuer aliases retain their original activation payment,
    // independently of the now-returned unique Root presence allocation.
    assert!(runtime.resources().counts().0 >= super::super::call_activation::ACTIVATION_BYTES);
}

#[test]
fn admitted_presence_box_watcher_and_result_have_an_independent_growth_guard() {
    use std::mem::size_of;
    let (bytes, proof) = inventory_bytes();
    eprintln!("admitted Presence prepared/Box/receipt graph {bytes}/{PRESENCE_BYTES}; body {}; receipt {}",
        size_of::<AdmittedPresence>(), size_of::<AdmittedPresenceReceipt>());
    assert!(bytes <= PRESENCE_BYTES);
    // The record's payment is outside its Box; all Vec backing and response
    // overlap belong to the original Presence fee above. No payload Vec here.
    eprintln!("admitted proof record {proof}/{RECORD_BYTES}");
    assert!(proof <= RECORD_BYTES);
}

#[tokio::test]
async fn admitted_presence_preallocation_guard_refuses_underfunding_without_new_charge() {
    let (_node, runtime, _clock, _grant, _owner, original, _key, _remote) = fixture().await;
    let before = runtime.resources().counts();
    let (presence, proof) = inventory_bytes();
    assert!(matches!(
        PresenceStorage::reserve_checked(&original, presence - 1, RECORD_BYTES),
        Err(VoiceError::Bounds)
    ));
    assert!(matches!(
        PresenceStorage::reserve_checked(&original, PRESENCE_BYTES, proof - 1),
        Err(VoiceError::Bounds)
    ));
    assert_eq!(
        runtime.resources().counts(),
        before,
        "actual production preallocation guard precedes charging/Funding allocation"
    );
}
