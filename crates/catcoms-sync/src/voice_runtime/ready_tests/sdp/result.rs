//! Actual public ChannelSync Result boundary with a signed packet shorter-lived than
//! Ready. Original injected Clock places expiry at the final Actor preparation.
use super::*;
use voice_v2::crypto_actions::CryptoOutcome;
use voice_v2::sdp::SdpOpenOutput;

#[tokio::test]
async fn root_sdp_result_refreshes_packet_expiry_after_actual_actor_preparation() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut origin_skip = None;
    for expire in [false, true] {
        let clock = Arc::new(ManualClock::new(1000));
        let hook = Arc::new(ReplySampleClock {
            inner: clock.clone(),
            on_mono: std::sync::Mutex::new(None),
        });
        let initial =
            Box::new(Box::pin(BridgeFixture::new_with_clock(clock.clone(), hook.clone())).await);
        let confirmed = Box::new(Box::pin(confirmed_with_custody_using(initial)).await);
        let mut fixture = Box::new(Box::pin(sdp_fixture_using(confirmed)).await);
        let (f, grants, handshake, low, phases, intents, _, offer) = fixture.as_mut();
        let high = 1 - *low;
        let low_node = if *low == 0 { &f.a } else { &f.c };
        let now = clock.now_ms();
        let delta = 101 + voice_v2::CLOCK_SKEW_MS;
        let short = SignedObject::issue(
            offer.scope(),
            &low_node.device,
            offer.id(),
            now,
            now + 100,
            offer.body().clone(),
        )
        .unwrap();
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        assert!(matches!(
            admit(node, Envelope::Direct(short)).unwrap(),
            EndpointReceipt::FirstOffer(_)
        ));
        let action = node
            .prepare_voice_sdp_open(&grants[high], *handshake, &intents[high])
            .unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let crossed = Arc::new(AtomicBool::new(false));
        if expire {
            let check = crossed.clone();
            let gates = node.voice_transport_actor().unwrap().lock_probe_for_test();
            let same = action.clone();
            let original = clock.clone();
            *hook.on_mono.lock().unwrap() = Some((
                origin_skip.unwrap(),
                Box::new(move || {
                    assert!(gates() && same.result_fence_available_for_test());
                    original.advance_ms(delta);
                    check.store(true, Ordering::Release);
                }),
            ));
        } else {
            *hook.on_mono.lock().unwrap() =
                Some((usize::MAX, Box::new(|| panic!("count-only callback"))));
        }
        let bytes = [b'a'; 5];
        let output = SdpOpenOutput::Plaintext {
            bytes: &bytes,
            claimed_tag: action.output_tag(&bytes).unwrap(),
        };
        let result =
            node.submit_voice_sdp_open_result_scoped(&grants[high], &action, output, |step| {
                if !expire {
                    let remaining = hook.on_mono.lock().unwrap().take().unwrap().0;
                    let samples = usize::MAX - remaining;
                    // The final Action observation immediately follows Actor origin.
                    // Calibrate against the actual successful call, not an invented bool.
                    origin_skip = Some(samples.checked_sub(2).unwrap());
                } else {
                    assert!(crossed.load(Ordering::Acquire));
                }
                *hook.on_mono.lock().unwrap() =
                    Some((0, Box::new(|| panic!("Clock beneath native Result fence"))));
                let committed = step.commit();
                assert!(
                    hook.on_mono.lock().unwrap().take().is_some(),
                    "final tail sampled no Clock"
                );
                committed
            });
        if expire {
            assert_eq!(result.err(), Some(VoiceError::Unauthorized));
            assert!(
                intents[high].current_at(clock.now_ms(), clock.monotonic_ms()),
                "Ready remains current; only the packet expired"
            );
            assert!(action.handoff_status().engine.receipt.is_none());
        } else {
            let result = result.unwrap();
            assert_eq!(result.receipt.outcome, CryptoOutcome::Opened);
            action.acknowledge_result(&result.receipt).unwrap();
        }
        action.settle_engine();
        for phase in phases {
            phase.close();
        }
    }
}

/// Public wrapper history is an original capability observation. In particular it must
/// reject a foreign owner before the original Action Clock (an injected callback seam).
#[tokio::test]
async fn root_sdp_delivery_history_binds_original_grant_group_device_and_resources_before_clock() {
    let clock = Arc::new(ManualClock::new(1000));
    let hook = Arc::new(ReplySampleClock {
        inner: clock.clone(),
        on_mono: std::sync::Mutex::new(None),
    });
    let initial = Box::new(Box::pin(BridgeFixture::new_with_clock(clock, hook.clone())).await);
    let confirmed = Box::new(Box::pin(confirmed_with_custody_using(initial)).await);
    let (mut f, grants, handshake, low, phases, intents, _, offer) =
        Box::pin(sdp_fixture_using(confirmed)).await;
    let high = 1 - low;
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    assert!(matches!(
        admit(node, Envelope::Direct(offer)).unwrap(),
        EndpointReceipt::FirstOffer(_)
    ));
    let action = node
        .prepare_voice_sdp_open(&grants[high], handshake, &intents[high])
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = b"v=0\r\n";
    let output = SdpOpenOutput::Plaintext {
        bytes,
        claimed_tag: action.output_tag(bytes).unwrap(),
    };
    let result = node
        .submit_voice_sdp_open_result(&grants[high], &action, output)
        .unwrap();
    let delivery = result.delivery.unwrap();
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    assert!(action.input().is_err());
    node.with_voice_sdp_delivery_admission(&grants[high], &action, &delivery, false, |step| {
        step.commit()
    })
    .unwrap();
    node.with_voice_sdp_delivery_admission(&grants[high], &action, &delivery, true, |step| {
        step.commit()
    })
    .unwrap();
    node.stop_voice_endpoint(&grants[high]);
    assert!(!grants[high].lease().is_live());
    *hook.on_mono.lock().unwrap() = Some((
        0,
        Box::new(|| panic!("foreign owner reached original Clock")),
    ));
    let scope = grants[high].scope();
    let mut wrong_scope = scope;
    wrong_scope.room += 1;
    let wrong = Arc::new(CallGrant::from_lease(grants[high].lease(), wrong_scope));
    let foreign = Arc::new(CallGrant::from_lease(CallLease::default(), scope));
    for grant in [&wrong, &foreign] {
        assert_eq!(
            node.with_voice_sdp_delivery_admission(grant, &action, &delivery, true, |_| panic!(
                "foreign grant reached publication hook"
            )),
            Err(VoiceError::Unauthorized)
        );
    }
    let original_resources = node.voice_runtime.as_ref().unwrap().resources.clone();
    node.voice_runtime.as_mut().unwrap().resources =
        Arc::new(voice_v2::resources::Resources::default());
    assert_eq!(
        node.with_voice_sdp_delivery_admission(
            &grants[high],
            &action,
            &delivery,
            true,
            |_| panic!("foreign runtime reached publication hook")
        ),
        Err(VoiceError::Unauthorized)
    );
    node.voice_runtime.as_mut().unwrap().resources = original_resources;
    // The other real member has the same group but a different local device/runtime.
    let peer = if low == 0 { &mut f.a } else { &mut f.c };
    assert_eq!(
        peer.with_voice_sdp_delivery_admission(
            &grants[high],
            &action,
            &delivery,
            true,
            |_| panic!("other member reached publication hook")
        ),
        Err(VoiceError::Unauthorized)
    );
    let (_hub, mut unrelated, _) = crate::tests::build_members(1).await;
    assert_eq!(
        unrelated[0].with_voice_sdp_delivery_admission(
            &grants[high],
            &action,
            &delivery,
            true,
            |_| panic!("unrelated group reached publication hook")
        ),
        Err(VoiceError::Unauthorized)
    );
    assert!(
        hook.on_mono.lock().unwrap().take().is_some(),
        "all rejected owners were callback-free"
    );
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    // Historical recovery intentionally binds the actual lease capability, rather than the
    // current grant Arc. A custody adapter over that same revoked lease observes truth only.
    let same = Arc::new(CallGrant::from_lease(grants[high].lease(), scope));
    node.with_voice_sdp_delivery_admission(&same, &action, &delivery, true, |_| {
        panic!("already-Applied is not fresh admission")
    })
    .unwrap();
    assert_eq!(
        node.with_voice_sdp_delivery_admission(
            &grants[high],
            &action,
            &delivery,
            false,
            |_| panic!("End must refuse fresh admission")
        ),
        Err(VoiceError::Unauthorized)
    );
    for phase in phases {
        phase.close();
    }
}
