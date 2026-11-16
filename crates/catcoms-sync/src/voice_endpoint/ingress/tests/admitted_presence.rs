use super::*;
use voice_v2::call_activation::{ActivationObserver, ActivationOwner};

fn key(join: u8) -> JoinKey {
    JoinKey::new([join; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap()
}
fn activate(node: &mut Node, join: u8) -> (Arc<CallGrant>, ActivationOwner, ActivationObserver) {
    let (grant, owner) = node.prepare_admitted_voice_call(1).unwrap().into_parts();
    let original = owner.observer();
    node.select_admitted_voice_call(&grant, &original)
        .unwrap()
        .commit_native(&grant)
        .unwrap();
    let key = key(join);
    let receipt = node
        .start_admitted_voice_presence(grant.clone(), original.clone(), key.join, key.public_key)
        .unwrap();
    assert!(receipt.matches(&original, &grant, key.join, key.public_key));
    (grant, owner, original)
}
fn source_objects(node: &mut Node) -> Vec<SignedObject> {
    node.pump_voice_endpoint().unwrap();
    let actor = node.voice_transport_actor().unwrap();
    actor
        .source()
        .unwrap()
        .take_ready(actor, &node.voice_bound_routes())
        .unwrap()
        .into_iter()
        .map(|job| {
            assert!(job.authority().authorized());
            job.object().clone()
        })
        .collect()
}
fn one(objects: Vec<SignedObject>, accepts: impl Fn(&Body) -> bool) -> SignedObject {
    let mut selected = objects.into_iter().filter(|object| accepts(object.body()));
    let object = selected.next().expect("actual shared discovery source");
    assert!(selected.next().is_none());
    object
}

#[tokio::test]
async fn admitted_presence_requires_exact_native_active_original_and_freezes_first_key() {
    let (mut foreign_root, mut node, runtime, clock) = fixture().await;
    let prepared = node.prepare_admitted_voice_call(1).unwrap();
    let original = prepared.observer();
    let key = key(17);
    let before = runtime.resources().counts();
    assert_eq!(
        node.start_admitted_voice_presence(
            prepared.grant().clone(),
            original.clone(),
            key.join,
            key.public_key
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(runtime.resources().counts(), before);
    let selected = node
        .select_admitted_voice_call(prepared.grant(), &original)
        .unwrap();
    assert_eq!(
        node.start_admitted_voice_presence(
            prepared.grant().clone(),
            original.clone(),
            key.join,
            key.public_key
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    selected.commit_native(prepared.grant()).unwrap();
    let foreign_runtime = Runtime::new(clock);
    attach(&mut foreign_root, &foreign_runtime);
    assert_eq!(
        foreign_root
            .start_admitted_voice_presence(
                prepared.grant().clone(),
                original.clone(),
                key.join,
                key.public_key
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let counterfeit = Arc::new(CallGrant::from_lease(
        prepared.grant().lease(),
        prepared.grant().scope(),
    ));
    assert_eq!(
        node.start_admitted_voice_presence(
            counterfeit.clone(),
            original.clone(),
            key.join,
            key.public_key
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    // Do not drop this deliberately counterfeit affine wrapper while testing the
    // real original: its Drop correctly closes their SAME lease.
    let receipt = node
        .start_admitted_voice_presence(
            prepared.grant().clone(),
            original.clone(),
            key.join,
            key.public_key,
        )
        .unwrap();
    let charged = runtime.resources().counts();
    for _ in 0..12 {
        let retry = node
            .start_admitted_voice_presence(
                prepared.grant().clone(),
                original.clone(),
                key.join,
                key.public_key,
            )
            .unwrap();
        assert!(retry.matches(&original, prepared.grant(), key.join, key.public_key));
    }
    assert_eq!(runtime.resources().counts(), charged);
    assert_eq!(
        node.start_admitted_voice_presence(
            prepared.grant().clone(),
            original.clone(),
            [18; 32],
            key.public_key
        )
        .err(),
        Some(VoiceError::RetryMismatch)
    );
    let context = node.voice_v2_context(1).unwrap();
    assert_eq!(
        node.start_voice_endpoint(prepared.grant().clone(), key.join, key.public_key),
        Err(VoiceError::Unauthorized)
    );
    assert!(LocalJoin::new(
        &context,
        key.clone(),
        prepared.grant().lease(),
        runtime.resources()
    )
    .is_err());
    assert!(voice_v2::packets::PacketLedger::new(
        &context,
        key.join,
        key.key_ref,
        prepared.grant().lease(),
        runtime.resources()
    )
    .is_err());
    assert!(WatchLease::default()
        .with_parent(prepared.grant().lease())
        .is_err());
    let owners = &node.voice_runtime.as_ref().unwrap().endpoints;
    assert!(owners.participant.is_none() && owners.observers.is_empty());
    node.clear_admitted_voice_call(&original).unwrap();
    assert!(original.root_returned().is_some());
    assert!(
        receipt.matches(&original, prepared.grant(), key.join, key.public_key),
        "accepted history survives End without becoming current authority"
    );
    assert!(node
        .admitted_voice_presence_peers(prepared.grant(), &original)
        .is_err());
    drop(counterfeit);
}

#[tokio::test]
async fn admitted_presence_uses_actual_shared_discovery_sources_and_neutral_fresh_peers() {
    let (mut existing, mut admitted, runtime, clock) = fixture().await;
    let existing_runtime = Runtime::new(clock.clone());
    attach(&mut existing, &existing_runtime);
    let existing_grant = Arc::new(existing.voice_call_grant(1).unwrap());
    let existing_key = key(21);
    existing
        .start_voice_endpoint(
            existing_grant.clone(),
            existing_key.join,
            existing_key.public_key,
        )
        .unwrap();
    let (grant, _owner, original) = activate(&mut admitted, 22);
    let admitted_announcement = one(source_objects(&mut admitted), |body| {
        matches!(body, Body::Announcement { .. })
    });
    let existing_announcement = one(source_objects(&mut existing), |body| {
        matches!(body, Body::Announcement { .. })
    });
    // Real signed canonical objects and actual native ingress/source machinery;
    // this deterministic test does not claim transport forwarding or V2 RTC.
    for object in [&admitted_announcement, &existing_announcement] {
        assert_eq!(SignedObject::decode(&object.encode()).unwrap(), *object);
    }
    drop(admit(
        &mut admitted,
        Envelope::Direct(existing_announcement),
    ));
    drop(admit(
        &mut existing,
        Envelope::Direct(admitted_announcement),
    ));
    let admitted_challenge = one(source_objects(&mut admitted), |body| {
        matches!(body, Body::WatchChallenge { .. })
    });
    let existing_challenge = one(source_objects(&mut existing), |body| {
        matches!(body, Body::WatchChallenge { .. })
    });
    drop(admit(&mut existing, Envelope::Direct(admitted_challenge)));
    drop(admit(&mut admitted, Envelope::Direct(existing_challenge)));
    let admitted_proof = one(source_objects(&mut admitted), |body| {
        matches!(body, Body::JoinProof { .. })
    });
    let existing_proof = one(source_objects(&mut existing), |body| {
        matches!(body, Body::JoinProof { .. })
    });
    drop(admit(&mut existing, Envelope::Direct(admitted_proof)));
    drop(admit(&mut admitted, Envelope::Direct(existing_proof)));
    admitted.pump_voice_endpoint().unwrap();
    existing.pump_voice_endpoint().unwrap();
    let peers = admitted
        .admitted_voice_presence_peers(&grant, &original)
        .unwrap();
    assert_eq!(peers.len(), 1);
    assert!(
        matches!(peers[0].announcement().body(), Body::Announcement { key, .. } if key == &existing_key)
    );
    assert_eq!(
        existing
            .voice_participant_peers(&existing_grant)
            .unwrap()
            .len(),
        1
    );
    let snapshot = admitted.admitted_voice_presence_peer_snapshot(
        &grant,
        &original,
        voice_v2::presence::PeerSnapshotPermit::new(&runtime.resources(), grant.scope()).unwrap(),
    );
    assert_eq!(snapshot.result().unwrap().1.len(), 1);
    let wrong_pool = admitted.admitted_voice_presence_peer_snapshot(
        &grant,
        &original,
        voice_v2::presence::PeerSnapshotPermit::new(&existing_runtime.resources(), grant.scope())
            .unwrap(),
    );
    assert_eq!(wrong_pool.result().err(), Some(VoiceError::Unauthorized));
    assert!(
        admitted.voice_participant_peers(&grant).is_err(),
        "old pair selection never consumes admitted liveness"
    );
    let source = admitted.voice_transport_actor().unwrap().source().unwrap();
    source.retire_existing_endpoint_sources();
    assert!(
        original.is_active(),
        "Existing retirement must skip admitted discovery"
    );
    original.close();
    assert!(admitted
        .admitted_voice_presence_peers(&grant, &original)
        .is_err());
    assert!(peers[0]
        .authorize(
            &admitted.voice_v2_context(1).unwrap(),
            &|_| true,
            clock.as_ref()
        )
        .is_err());
    assert!(
        runtime.resources().counts().0 > 0,
        "retained observations/owners still pay actual custody"
    );
}

#[tokio::test]
async fn admitted_presence_wrong_room_announcement_and_challenge_never_enter_public_queue() {
    let (sender, mut node, runtime, _clock) =
        fixture_with_directory(|clock| clock, |_| Ok(BTreeSet::from([1, 2]))).await;
    let (_grant, _owner, original) = activate(&mut node, 41);
    let other = node.voice_v2_context(2).unwrap();
    let key = key(41);
    let bodies = [
        Body::Announcement {
            key: key.clone(),
            epoch: other.epoch,
            renewal: 1,
        },
        Body::WatchChallenge {
            watcher: [77; 32],
            target: node.device_id(),
            join: key.join,
            key_ref: key.key_ref,
            challenge: [78; 32],
        },
    ];
    drop(
        node.voice_transport_actor()
            .unwrap()
            .admit_transport_verification(sender.device_id(), b"warm")
            .unwrap(),
    );
    for (index, body) in bodies.into_iter().enumerate() {
        let room = node.voice_room_authority(2).unwrap();
        let envelope = Envelope::Direct(
            SignedObject::issue(
                other.scope,
                &sender.device,
                [88 + index as u8; 32],
                1000,
                16_000,
                body,
            )
            .unwrap(),
        );
        let before = runtime.resources().counts();
        let ticket = node
            .voice_transport_actor()
            .unwrap()
            .admit_transport_verification(sender.device_id(), &envelope.encode())
            .unwrap();
        assert_eq!(
            node.admit_voice_endpoint_public(ticket, envelope, &other, room)
                .err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(runtime.resources().counts(), before);
        assert!(node
            .voice_runtime
            .as_ref()
            .unwrap()
            .endpoints
            .public
            .packets
            .is_empty());
        assert!(original.is_active());
    }
}

#[tokio::test]
async fn admitted_presence_old_public_recipient_cannot_adopt_a_replacement() {
    let (sender, mut node, _runtime, _clock) = fixture().await;
    let (_grant, _owner, original) = activate(&mut node, 31);
    let envelope = announcement(&sender, original.scope());
    let receipt = admit(&mut node, envelope.clone());
    assert!(receipt.authorized());
    node.clear_admitted_voice_call(&original).unwrap();
    assert!(!receipt.authorized());
    let (next_grant, _next_owner, next) = activate(&mut node, 32);
    let sources = source_objects(&mut node);
    assert!(
        sources
            .iter()
            .all(|object| matches!(object.body(), Body::Announcement { .. })),
        "retained old candidate cannot challenge under replacement admitted watch"
    );
    assert!(node
        .admitted_voice_presence_peers(&next_grant, &next)
        .unwrap()
        .is_empty());
    let context = node.voice_v2_context(1).unwrap();
    let room = node.voice_room_authority(1).unwrap();
    let ticket = node
        .voice_transport_actor()
        .unwrap()
        .admit_transport_verification(sender.device_id(), &envelope.encode())
        .unwrap();
    assert_eq!(
        node.admit_voice_endpoint_public(ticket, envelope, &context, room)
            .err(),
        Some(VoiceError::Unauthorized)
    );
}

#[tokio::test]
async fn admitted_presence_payment_survives_actual_watching_lower_after_root_return() {
    let (sender, mut node, runtime, clock) = fixture().await;
    let (_grant, _owner, original) = activate(&mut node, 51);
    drop(admit(&mut node, announcement(&sender, original.scope())));
    node.pump_voice_endpoint().unwrap();
    let actor = node.voice_transport_actor().unwrap();
    let source = actor.source().unwrap();
    let mut jobs = source
        .take_ready(actor, &node.voice_bound_routes())
        .unwrap();
    let position = jobs
        .iter()
        .position(|job| matches!(job.object().body(), Body::WatchChallenge { .. }))
        .unwrap();
    let watching_job = jobs.swap_remove(position);
    drop(jobs);
    let actual_lower = watching_job.keepalive();
    assert!(watching_job.authority().authorized());
    assert_eq!(
        source.complete(watching_job.finish(None)).unwrap(),
        voice_v2::source::SourceOutcome::Pending
    );
    node.clear_admitted_voice_call(&original).unwrap();
    assert!(
        original.root_returned().is_some(),
        "unique residence/Box actually returned"
    );
    // Expire the real public packet, Source maps/cursors and native caches. Only
    // the actual retained Watching Lower keeps its private child and SAME fee.
    clock.advance_ms(60_001);
    node.collect_expired_voice();
    source.counts();
    let held = runtime.resources().counts().0;
    assert!(held >= voice_v2::admitted_presence::PRESENCE_BYTES);
    drop(actual_lower);
    source.counts();
    assert!(
        held - runtime.resources().counts().0 >= voice_v2::admitted_presence::PRESENCE_BYTES,
        "last actual Watching Lower returns original Presence payer after lease/watch deallocation"
    );
}
