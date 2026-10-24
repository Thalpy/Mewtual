use super::*;
use catcoms_rt::ManualClock;
use voice_v2::limits::OriginLimits;
use voice_v2::packets::Authority;
use voice_v2::presence::WatchAuthority;
use voice_v2::runtime::Runtime;

fn key(n: u8) -> JoinKey {
    JoinKey::new([n; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap()
}
pub(super) type Node = ChannelSync<catcoms_rt::mem::MemNetwork, rand_chacha::ChaCha20Rng>;
pub(super) fn attach(node: &mut Node, runtime: &Arc<Runtime>) {
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    node.prepare_voice_actor(
        runtime,
        &ticket,
        991,
        |_| Ok(BTreeSet::from([1])),
        |_| Ok(BTreeSet::new()),
    )
    .unwrap()
    .register()
    .unwrap();
}

#[tokio::test]
async fn endpoint_watch_reply_custody_parent_revocation_and_metadata_wakes_are_bounded() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let parent = CallLease::default();
    let mut changes = node.voice_endpoint_changes().unwrap();
    let grant = node.start_voice_watch(1, parent.clone()).unwrap();
    node.validate_voice_watch(&grant).unwrap();
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    // Dropped reply has no strong actor owner: pruning frees the actual Watcher allocation.
    let (reply, receiver) = tokio::sync::oneshot::channel();
    drop(receiver);
    drop(reply.send(grant));
    node.reconcile_voice_actor();
    assert!(node
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .observers
        .is_empty());
    let baseline = runtime.resources().counts();
    let mut held = Vec::new();
    for _ in 0..WATCHES {
        held.push(node.start_voice_watch(1, parent.clone()).unwrap());
    }
    assert_eq!(
        node.start_voice_watch(1, parent.clone()).err(),
        Some(VoiceError::Capacity)
    );
    // An unread UI subscriber coalesces arbitrary metadata updates; it never backpressures Root.
    for _ in 0..1024 {
        node.voice_runtime.as_ref().unwrap().endpoints.changed();
    }
    assert!(changes.has_changed().unwrap());
    parent.revoke();
    for grant in &held {
        assert!(!grant.is_live());
        assert!(node.validate_voice_watch(grant).is_err());
    }
    node.reconcile_voice_actor();
    assert!(node
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .observers
        .is_empty());
    assert!(
        runtime.resources().counts().0 > baseline.0,
        "actual native grant clones retain their own charge"
    );
    drop(held);
    assert_eq!(runtime.resources().counts(), baseline);
}

#[tokio::test]
async fn endpoint_exact_start_retry_key_binding_and_owner_retirement_preserve_one_allocator() {
    let (_hub, mut nodes, _) = crate::tests::build_members(1).await;
    let node = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(node, &runtime);
    let grant = Arc::new(node.voice_call_grant(1).unwrap());
    let key = key(1);
    node.start_voice_endpoint(grant.clone(), key.join, key.public_key)
        .unwrap();
    let charged = runtime.resources().counts();
    for _ in 0..12 {
        node.start_voice_endpoint(grant.clone(), key.join, key.public_key)
            .unwrap();
    }
    assert_eq!(runtime.resources().counts(), charged);
    assert_eq!(
        node.voice_runtime
            .as_ref()
            .unwrap()
            .endpoints
            .observers
            .len(),
        1
    );
    assert_eq!(
        node.start_voice_endpoint(grant.clone(), [2; 32], key.public_key),
        Err(VoiceError::RetryMismatch)
    );
    let foreign = Arc::new(CallGrant::from_lease(CallLease::default(), grant.scope()));
    assert_eq!(
        node.start_voice_endpoint(foreign, key.join, key.public_key),
        Err(VoiceError::Unauthorized)
    );
    grant.lease().revoke();
    node.reconcile_voice_actor();
    assert!(node
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .is_none());
    let fresh = Arc::new(node.voice_call_grant(1).unwrap());
    node.start_voice_endpoint(fresh.clone(), [2; 32], key.public_key)
        .unwrap();
    runtime.revoke_session();
    assert!(!fresh.lease().is_live());
    node.reconcile_voice_actor();
    assert!(node
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .is_none());
}

#[tokio::test]
async fn endpoint_actual_mls_remove_readd_retires_retained_presence_pair_and_cached_proof() {
    let (_hub, mut nodes, ids) = crate::tests::build_members(2).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    let mut remote = nodes.pop().unwrap();
    let local = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(local, &runtime);
    let grant = Arc::new(local.voice_call_grant(1).unwrap());
    let local_key = key(1);
    local
        .start_voice_endpoint(grant.clone(), local_key.join, local_key.public_key)
        .unwrap();
    let context = local.voice_v2_context(1).unwrap();
    let remote_context = voice_v2::Context {
        local: ids[1],
        ..context
    };
    let remote_lease = CallLease::default();
    let remote_pool = Arc::new(voice_v2::resources::Resources::default());
    let mut remote_join = LocalJoin::new(
        &remote_context,
        key(2),
        remote_lease.clone(),
        remote_pool.clone(),
    )
    .unwrap();
    let remote_authority = Authority {
        context: &remote_context,
        lease: &remote_lease,
        member: &|_| true,
    };
    let clock = runtime.clock();
    let remote_ann = remote_join
        .announce(
            &remote.device,
            &remote_authority,
            clock.as_ref(),
            &mut remote.rng,
        )
        .unwrap();
    let local_lease = grant.lease();
    let local_authority = Authority {
        context: &context,
        lease: &local_lease,
        member: &|_| true,
    };
    let voice = local.voice_runtime.as_mut().unwrap();
    let participant = voice.endpoints.participant.as_mut().unwrap();
    let local_ann = participant
        .join
        .announce(
            &local.device,
            &local_authority,
            clock.as_ref(),
            &mut local.rng,
        )
        .unwrap();
    let observer = voice
        .endpoints
        .observers
        .get_mut(&participant.watch.id)
        .unwrap();
    let watch_authority = WatchAuthority {
        context: &context,
        lease: &observer.lease,
        member: &|_| true,
    };
    let challenge = voice
        .current
        .as_ref()
        .unwrap()
        .with_limits(|limits| {
            observer.watcher.challenge(
                &remote_ann,
                &local.device,
                &watch_authority,
                limits,
                clock.as_ref(),
                &mut local.rng,
            )
        })
        .unwrap();
    let proof = remote_join
        .answer(
            &challenge,
            &remote.device,
            &remote_authority,
            clock.as_ref(),
            &mut remote.rng,
        )
        .unwrap();
    let fresh = observer
        .watcher
        .prove(&proof, &watch_authority, clock.as_ref())
        .unwrap();
    let pair = voice
        .current
        .as_ref()
        .unwrap()
        .with_limits(|limits| {
            participant
                .join
                .begin_pair(&fresh, &local_authority, limits, clock.as_ref())
        })
        .unwrap();
    let remote_watch = WatchLease::default();
    let mut watcher = Watcher::new(
        &remote_context,
        remote_watch.clone(),
        remote_pool.clone(),
        &mut remote.rng,
    )
    .unwrap();
    let remote_watch_authority = WatchAuthority {
        context: &remote_context,
        lease: &remote_watch,
        member: &|_| true,
    };
    let mut limits = OriginLimits::new(context.scope.group, remote_pool);
    let remote_challenge = watcher
        .challenge(
            &local_ann,
            &remote.device,
            &remote_watch_authority,
            &mut limits,
            clock.as_ref(),
            &mut remote.rng,
        )
        .unwrap();
    let cached_proof = participant
        .join
        .answer(
            &remote_challenge,
            &local.device,
            &local_authority,
            clock.as_ref(),
            &mut local.rng,
        )
        .unwrap();
    assert_eq!(
        participant
            .join
            .answer(
                &remote_challenge,
                &local.device,
                &local_authority,
                clock.as_ref(),
                &mut local.rng
            )
            .unwrap(),
        cached_proof
    );

    local.request_remove(&ids[1]).await.unwrap();
    assert!(!local.group.contains_device(&ids[1]));
    assert!(
        grant.lease().is_live(),
        "removing another member does not end the local call"
    );
    // Real authorized re-admission uses the same full device. We inspect the inviter half;
    // the remote OpenMLS provider need not install a duplicate same-group Welcome for this test.
    let invite = local.mint_invite([97; 16], 10_000, vec![]).unwrap();
    let package = remote
        .device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let request = encode_join_req(&invite, &serialize_key_package(&package).unwrap());
    local.serve_join(remote.local_peer(), &request).unwrap();
    assert!(local.group.contains_device(&ids[1]));
    let context = local.voice_v2_context(1).unwrap();
    assert!(
        fresh
            .authorize(&context, &|_| true, clock.as_ref())
            .is_err(),
        "retained observation cannot revive after re-add"
    );
    let authority = Authority {
        context: &context,
        lease: &local_lease,
        member: &|_| true,
    };
    let participant = local
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .participant
        .as_mut()
        .unwrap();
    assert_eq!(
        participant
            .join
            .pair_setup(pair.handshake, &authority, clock.as_ref())
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let new_proof = participant
        .join
        .answer(
            &remote_challenge,
            &local.device,
            &authority,
            clock.as_ref(),
            &mut local.rng,
        )
        .unwrap();
    assert_ne!(
        new_proof.id(),
        cached_proof.id(),
        "removal retired the old cached proof even before any room poll"
    );
}

#[tokio::test]
async fn endpoint_lost_last_native_call_owner_revokes_queued_source_without_a_root_turn() {
    let (_hub, mut nodes, _) = crate::tests::build_members(2).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    nodes[0]
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    nodes[0].snapshot().unwrap();
    nodes[0].publish_group_policy().unwrap();
    let policy = nodes[0].group_policy.as_ref().unwrap().encode();
    nodes[1].on_group_policy(&policy);
    let mut remote = nodes.pop().unwrap();
    let local = &mut nodes[0];
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(local, &runtime);
    let (ok, served) = tokio::join!(
        local.finalize_member_connection(remote.local_peer()),
        remote.run_once()
    );
    assert!(ok.unwrap() && served.unwrap());
    let grant = Arc::new(local.voice_call_grant(1).unwrap());
    let lease = grant.lease();
    let key = key(4);
    local
        .start_voice_endpoint(grant.clone(), key.join, key.public_key)
        .unwrap();
    let context = local.voice_v2_context(1).unwrap();
    let authority = Authority {
        context: &context,
        lease: &lease,
        member: &|_| true,
    };
    let participant = local
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .participant
        .as_mut()
        .unwrap();
    let object = participant
        .join
        .announce(
            &local.device,
            &authority,
            runtime.clock().as_ref(),
            &mut local.rng,
        )
        .unwrap();
    let activity = participant.join.source_activity(&authority).unwrap();
    local.queue_voice_source(object, activity).unwrap();
    let actor = local.voice_transport_actor().unwrap();
    let ledger = actor.source().unwrap();
    let job = ledger
        .take_ready(actor, &local.voice_bound_routes())
        .unwrap()
        .pop()
        .unwrap();
    let lower = job.keepalive();
    let request_authority = job.authority();
    assert!(request_authority.authorized());
    let before = runtime.resources().counts().0;
    drop(grant); // No End command or actor turn. Root is only a weak custody observer.
    assert!(!lease.is_live());
    assert!(!request_authority.authorized());
    assert_eq!(
        ledger.complete(job.finish(None)).unwrap(),
        voice_v2::source::SourceOutcome::Retired
    );
    assert_eq!(
        ledger.counts(),
        (1, 1, 1),
        "actual lower owner still holds the source flight"
    );
    assert_eq!(
        runtime.resources().counts().0,
        before - 256,
        "only final native grant custody retires here"
    );
    local.reconcile_voice_actor();
    let endpoints = &local.voice_runtime.as_ref().unwrap().endpoints;
    assert!(endpoints.participant.is_none() && endpoints.observers.is_empty());
    let retained = runtime.resources().counts().0;
    drop(lower);
    assert_eq!(
        ledger.counts(),
        (0, 0, 0),
        "ended source history collects only after actual Lower drainage"
    );
    assert!(
        runtime.resources().counts().0 < retained,
        "lower charge releases only on its own final drop"
    );
}
