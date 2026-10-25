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
