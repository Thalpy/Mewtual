//! Actual Root framing, connected transport, signed remote refusal and retained ACK verifier.
//! LOW Ready admission is intentionally not installed yet; this proves capacity and lifecycle,
//! not successful LOW readiness or SDP/RTC negotiation.
use super::*;
use futures::FutureExt;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

#[tokio::test]
async fn root_low_successor_unknown_handshake_and_wrong_role_refuse_without_payment_or_panic() {
    let (mut f, handshake) = confirmed().await;
    let before = f.runtime.resources().counts();
    assert_eq!(
        f.receiver.prepay_voice_low_sdp(&f.grant, [241; 32]),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.receiver.prepare_voice_low_sdp(&f.grant, [241; 32]),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.receiver.prepay_voice_low_sdp(&f.grant, handshake),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.receiver.prepare_voice_low_sdp(&f.grant, handshake),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(f.runtime.resources().counts(), before);
}

struct AckClock {
    inner: Arc<ManualClock>,
    probe: Mutex<Option<Arc<dyn Fn() -> bool + Send + Sync>>>,
    outside: AtomicBool,
    samples: AtomicUsize,
}
impl std::fmt::Debug for AckClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AckClock")
    }
}
impl AckClock {
    fn observe(&self) {
        if let Some(probe) = self.probe.lock().unwrap().as_ref() {
            self.outside.fetch_and(probe(), Ordering::AcqRel);
            self.samples.fetch_add(1, Ordering::AcqRel);
        }
    }
}
impl Clock for AckClock {
    fn now_ms(&self) -> u64 {
        self.observe();
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        self.observe();
        self.inner.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}

#[tokio::test]
async fn key_release_before_ready_preparation_refuses_without_new_capacity_or_receipt() {
    let (mut f, handshake, keys) = confirmed_with_keys(fixture().await).await;
    let before = f.runtime.resources().counts();
    let positions_before = positions(&f);
    keys.close_successor_effects();
    keys.close_successor_effects(); // No rearm or new retirement obligation on retry.
    assert_eq!(
        f.receiver
            .prepare_voice_high_ready(&f.grant, handshake, &[]),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.receiver.queue_voice_high_ready(&f.grant, handshake),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(f.runtime.resources().counts(), before);
    assert_eq!(positions(&f), positions_before);
}

#[tokio::test]
async fn key_release_at_final_ready_source_clock_fences_publication_without_origin_debit() {
    let clock_slot = Arc::new(Mutex::new(None));
    let slot = clock_slot.clone();
    let fixture = fixture_in_room_with_clock(1, move |inner| {
        let clock = Arc::new(AckClock {
            inner,
            probe: Mutex::new(None),
            outside: AtomicBool::new(true),
            samples: AtomicUsize::new(0),
        });
        *slot.lock().unwrap() = Some(clock.clone());
        clock
    })
    .await;
    let clock = clock_slot.lock().unwrap().take().unwrap();
    let (mut f, handshake, keys) = confirmed_with_keys(fixture).await;
    f.receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap();
    let pool = f.runtime.resources();
    let before = pool.counts();
    let source = f
        .receiver
        .voice_transport_actor()
        .unwrap()
        .source()
        .unwrap();
    let counts = source.counts();
    let context = f.receiver.voice_v2_context(1).unwrap();
    let room = f.receiver.voice_room_authority(1).unwrap();
    let voice = f.receiver.voice_runtime.as_ref().unwrap();
    let actor = voice.current.as_ref().unwrap();
    let participant = voice.endpoints.participant.as_ref().unwrap();
    let auth = authority(&context, &participant.lease);
    let (object, activity) = participant
        .join
        .ready_source_activity(handshake, voice.endpoints.public.namespace(), &auth)
        .unwrap();
    let reached_final = AtomicBool::new(false);
    assert_eq!(
        source.admit_borrowed_before_final_for_test(actor, object, activity, room, || {
            assert!(
                keys.is_current(),
                "every actual Source preflight/payment has passed live"
            );
            reached_final.store(true, Ordering::Release);
            // Arm ONLY after the actual immutable Ready authority, Source credit and
            // retained copy exist. The next Clock sample belongs to commit=true final
            // authorization, rather than ReadySourceAuthority's earlier constructor.
            *clock.probe.lock().unwrap() = Some(Arc::new(move || {
                keys.close_successor_effects();
                true
            }));
        }),
        Err(VoiceError::Unauthorized)
    );
    assert!(reached_final.load(Ordering::Acquire));
    assert!(clock.samples.load(Ordering::Acquire) > 0);
    *clock.probe.lock().unwrap() = None;
    assert_eq!(
        source.counts(),
        counts,
        "no Source record or origin cursor may publish"
    );
    assert_eq!(
        pool.counts(),
        before,
        "local refusal returns only its own preparation"
    );
}

#[tokio::test]
async fn key_release_after_ready_queue_revokes_actual_lower_without_early_refund() {
    let (mut f, handshake, keys) = confirmed_with_keys(fixture().await).await;
    f.receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap();
    f.receiver
        .queue_voice_high_ready(&f.grant, handshake)
        .unwrap();
    let pending = f.receiver.prepare_voice_sources().unwrap().pop().unwrap();
    // This is the actual prepared transport's Lower custody, not a synthetic held credit.
    let lower = pending.keepalive_for_test();
    let pool = f.runtime.resources();
    let before = pool.counts();
    keys.close_successor_effects();
    assert_eq!(
        pool.counts(),
        before,
        "closure must not refund an actual retained Lower"
    );
    let (_, signal) = tokio::sync::watch::channel(false);
    let completed = pending.execute(signal).await;
    assert_eq!(
        pool.counts(),
        before,
        "completion and transport still own the real Lower"
    );
    let failure = f.receiver.complete_voice_source(completed).unwrap_err();
    assert_eq!(failure.error(), &VoiceError::Unauthorized);
    assert!(
        failure.into_retry().is_none(),
        "released authority cannot admit a late ACK"
    );
    assert_eq!(
        pool.counts(),
        before,
        "the independent transport owner still retains its payment"
    );
    // Closure alone does not return unused reservation bytes. Retire the actual endpoint
    // so the reserve closes, while the separately retained real transport owner remains.
    f.receiver.stop_voice_endpoint(&f.grant);
    let closing = pool.counts();
    drop(lower);
    assert!(
        pool.counts().0 < closing.0,
        "last actual Lower Drop releases its payment"
    );
    assert!(f.receiver.prepare_voice_sources().unwrap().is_empty());
}

#[tokio::test]
async fn root_ready_source_actual_signed_ack_uses_original_capacity_with_full_ordinary_crypto_and_bytes(
) {
    let clock_slot = Arc::new(Mutex::new(None));
    let slot = clock_slot.clone();
    let fixture = fixture_in_room_with_clock(1, move |inner| {
        let clock = Arc::new(AckClock {
            inner,
            probe: Mutex::new(None),
            outside: AtomicBool::new(true),
            samples: AtomicUsize::new(0),
        });
        *slot.lock().unwrap() = Some(clock.clone());
        clock
    })
    .await;
    let clock = clock_slot.lock().unwrap().take().unwrap();
    let (mut f, handshake) = confirmed_in(fixture).await;
    assert_eq!(
        f.receiver.queue_voice_high_ready(&f.grant, handshake),
        Err(VoiceError::Unauthorized)
    );
    f.receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap();
    let pool = f.runtime.resources();
    let group = f.grant.scope().group;
    let crypto: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let pressure = pool
        .charge(
            group,
            voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    assert_eq!(
        f.receiver.queue_voice_high_ready(&f.grant, handshake),
        Ok(false)
    );
    for _ in 0..4 {
        assert_eq!(
            f.receiver.queue_voice_high_ready(&f.grant, handshake),
            Ok(true)
        );
    }
    assert_eq!(pool.counts(), full);
    let pending = f
        .receiver
        .prepare_voice_sources()
        .unwrap()
        .pop()
        .expect("actual Ready Source route");
    assert_eq!(pending.peer(), f.sender.local_peer());
    let (_cancel, signal) = tokio::sync::watch::channel(false);
    let mut flight = Box::pin(pending.execute(signal));
    assert!(
        futures::poll!(&mut flight).is_pending(),
        "pending Ready must not use generic inactive Exchange cancellation"
    );
    let request = (0..32)
        .find_map(|_| {
            match f
                .sender
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
                None => panic!("transport closed before request"),
            }
        })
        .expect("the native Ready request is in the bounded ready prefix");
    f.sender
        .handle_voice_transport_request(request.0, &request.1, request.2);
    let completed = flight.await;
    *clock.probe.lock().unwrap() = Some(
        f.receiver
            .voice_transport_actor()
            .unwrap()
            .lock_probe_for_test(),
    );
    assert_eq!(
        f.receiver.complete_voice_source(completed).unwrap(),
        (
            crate::VoiceTransportStatus::Refused,
            voice_v2::source::SourceOutcome::Pending
        )
    );
    assert!(clock.samples.load(Ordering::Acquire) > 0);
    assert!(
        clock.outside.load(Ordering::Acquire),
        "every actual ACK completion Clock callback must run outside Group/Policy"
    );
    *clock.probe.lock().unwrap() = None;
    assert_eq!(pool.counts(), full);
    let source = f
        .receiver
        .voice_transport_actor()
        .unwrap()
        .source()
        .unwrap();
    assert_eq!(source.counts(), (1, 1, 0));
    assert_eq!(
        f.receiver.queue_voice_high_ready(&f.grant, handshake),
        Ok(true)
    );
    drop((crypto, pressure));
}
