//! Real protected Root publication -> Source preparation -> authenticated transport ACK under
//! full ordinary capacity. Ciphertext is a unit fixture; this proves no WebCrypto/RTC outcome.
use super::*;
use futures::FutureExt;

#[tokio::test]
async fn source_pump_prunes_ended_participant_without_blocking_independent_live_observer() {
    let ((mut root, _remote, _runtime, _clock, grant), observer) =
        low_discovery_for_watch(1, true).await;
    let observer = observer.unwrap();
    grant.lease().revoke();
    assert_eq!(root.pump_voice_packet_sources(), Ok(0));
    assert!(root
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .is_none());
    root.validate_voice_watch(&observer).unwrap();
    root.pump_voice_endpoint().unwrap();
    assert!(
        observer.is_live(),
        "the separate observer remains serviceable after End"
    );
}

#[tokio::test]
async fn source_pump_borrows_current_fifty_member_native_view_under_full_ordinary_pressure() {
    let (mut root, remote, runtime, _clock, grant, handshake) = low_fixture().await;
    let action = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("Hello seal")
    };
    let handoff = action.borrow_handoff(16 * 1024).unwrap();
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = root
        .submit_voice_confirmation_result(
            &grant,
            &action,
            &ciphertext,
            action.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    let context = root.voice_v2_context(1).unwrap();
    // Exercise the actual already-paid Actor map independently of the older MLS fixture.
    // Production's outer pump reconciles MLS first; this private-pass test deliberately
    // installs its post-reconciliation view without allocating a second native roster.
    let members: std::collections::BTreeSet<_> = std::iter::once(root.device_id())
        .chain(
            (1..=49).map(|seed| catcoms_crypto::DeviceKeypair::from_seed(&[seed; 32]).device_id()),
        )
        .filter(|id| *id != remote.device_id())
        .collect();
    assert_eq!(members.len(), 50);
    assert!(root.group.member_device_ids().contains(&remote.device_id()));
    root.voice_runtime
        .as_mut()
        .unwrap()
        .current
        .as_mut()
        .unwrap()
        .sync_view(voice_v2::runtime::View {
            epoch: context.epoch + 1,
            members,
            policy: context.scope.policy,
            eligible: true,
            indirect: context.indirect,
        })
        .unwrap();
    let source = root.voice_transport_actor().unwrap().source().unwrap();
    let pool = runtime.resources();
    let fill = pool
        .charge(
            context.scope.group,
            voice_v2::resources::GROUP_BYTES - pool.counts().0,
            voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    assert_eq!(root.pump_voice_packet_sources(), Ok(0));
    assert_eq!(
        source.counts(),
        (0, 0, 0),
        "removed endpoint cannot enter Source"
    );
    assert_eq!(
        pool.counts(),
        full,
        "the borrowed member check creates no paid owners"
    );
    drop((fill, input, handoff, action));
}

#[tokio::test]
async fn protected_source_and_signed_ack_keep_original_capacity_under_full_ordinary_pressure() {
    let (mut root, mut remote, runtime, clock, grant, handshake) = low_fixture().await;
    // The discovery-only parent fixture does not retain the remote Runtime after returning.
    // Actual transport verification needs a live production actor, not its old weak projection.
    remote.retire_voice_actor();
    let remote_runtime = Runtime::new(clock.clone());
    attach(&mut remote, &remote_runtime);
    let action = root
        .start_voice_confirmation(&grant, handshake, None)
        .unwrap();
    let control = action.confirmation_control().unwrap();
    // Pay this fixture's ciphertext and independently retained inspection copy BEFORE
    // creating them. The production pump itself borrows the original native cached packet.
    let handoff = action.borrow_handoff(16 * 1024).unwrap();
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("Hello seal")
    };
    let id = flight.reservation.id;
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = root
        .submit_voice_confirmation_result(
            &grant,
            &action,
            &ciphertext,
            action.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    let context = root.voice_v2_context(1).unwrap();
    let lease = grant.lease();
    let authority = authority(&context, &lease);
    let (object, activity) = {
        let participant = root
            .voice_runtime
            .as_mut()
            .unwrap()
            .endpoints
            .participant
            .as_mut()
            .unwrap();
        // Exact cached bytes after actual protected publication; this cannot sign a new packet.
        let object = participant
            .join
            .packet_ledger_for_test()
            .submit(id, &ciphertext, &root.device, &authority, clock.as_ref())
            .unwrap();
        let activity = participant
            .join
            .source_packet_activity(&object, &authority, runtime.clock())
            .unwrap();
        (object, activity)
    };
    let source = root.voice_transport_actor().unwrap().source().unwrap();
    let pool = runtime.resources();
    let fill = pool
        .charge(
            context.scope.group,
            voice_v2::resources::GROUP_BYTES - pool.counts().0,
            voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    assert_eq!(root.pump_voice_packet_sources(), Ok(0));
    // The actual sealing engine still owns the original Crypto lane. The native pump
    // retains the original cache for retry without creating another Source payload copy.
    assert_eq!(source.counts(), (0, 0, 0));
    assert_eq!(pool.counts(), before);
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    assert_eq!(root.pump_voice_packet_sources(), Ok(1));
    let queued = pool.counts();
    for _ in 0..8 {
        assert_eq!(root.pump_voice_packet_sources(), Ok(0));
    }
    assert_eq!(pool.counts(), queued, "exact retries create no new owners");
    assert_eq!(source.counts(), (1, 0, 0));
    let pending = root
        .prepare_voice_sources()
        .unwrap()
        .pop()
        .expect("prepaid Source Lower and Root route");
    assert_eq!(pending.peer(), remote.local_peer());
    let late_lower = pending.keepalive_for_test();
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let mut flight = Box::pin(pending.execute(signal));
    assert!(
        futures::poll!(&mut flight).is_pending(),
        "request enters the actual Mem transport"
    );
    // The first poll queued the request synchronously. Drain only a bounded ready prefix,
    // failing immediately if preparation stopped early instead of awaiting a missing event.
    let request = (0..32)
        .find_map(|_| {
            match remote
                .transport
                .next_event()
                .now_or_never()
                .expect("queued transport event")
            {
                Some(catcoms_rt::TransportEvent::Request {
                    from,
                    data,
                    responder,
                    ..
                }) => Some((from, data, responder)),
                Some(_) => None,
                None => panic!("transport closed before its queued request"),
            }
        })
        .expect("the queued native request is in the bounded ready prefix");
    remote.handle_voice_transport_request(request.0, &request.1, request.2);
    let completed = flight.await;
    // The remote has no native participant. Its ACTUAL signed refusal is authenticated using
    // this immutable Source's Workflow, without pretending it admitted or opened Hello.
    assert_eq!(
        root.complete_voice_source(completed).unwrap(),
        (
            crate::VoiceTransportStatus::Refused,
            voice_v2::source::SourceOutcome::Pending
        )
    );
    assert_eq!(source.counts(), (1, 1, 1));
    assert_eq!(root.pump_voice_packet_sources(), Ok(0));
    assert_eq!(
        source.counts(),
        (1, 1, 1),
        "a pump never resets spent attempts"
    );
    grant.lease().revoke();
    root.stop_voice_endpoint(&grant);
    let retirement = control.retire();
    drop((activity, object, input, handoff, action, ciphertext, fill));
    assert!(
        !retirement.is_drained(),
        "retained Source history is real Workflow custody after End"
    );
    runtime.collect_expired();
    assert_eq!(
        source.counts(),
        (1, 1, 1),
        "End never refunds actual retained transport work"
    );
    drop(late_lower);
    runtime.collect_expired();
    assert_eq!(
        clock.monotonic_ms(),
        1000,
        "permanently ended work needs no expiry wait"
    );
    assert_eq!(source.counts(), (0, 0, 0));
    assert!(
        retirement.is_drained(),
        "actual Source collection releases the original Workflow"
    );
}
