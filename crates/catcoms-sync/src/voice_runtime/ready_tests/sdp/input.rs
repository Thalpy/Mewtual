//! Actual ChannelSync/MLS admission. No renderer, HTTP publication or WebCrypto is mocked
//! as completed: these tests cover only the scoped immutable input-read boundary.
use super::*;
use voice_v2::crypto_actions::{SdpAdmittedInput, SdpInputRead};

type Fixture = (Box<SdpFixture>, Arc<CryptoAction>);
async fn opened_fixture() -> Fixture {
    // Keep the existing four-node fixture/future on the heap. Nesting another
    // by-value async return would duplicate it in the test thread's small stack.
    let mut fixture = Box::new(Box::pin(sdp_fixture()).await);
    let (f, grants, handshake, low, _, intents, _, offer) = fixture.as_mut();
    let high = 1 - *low;
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    assert!(matches!(
        admit(node, Envelope::Direct(offer.clone())).unwrap(),
        EndpointReceipt::FirstOffer(_)
    ));
    let action = node
        .prepare_voice_sdp_open(&grants[high], *handshake, &intents[high])
        .unwrap();
    (fixture, action)
}
fn data(read: SdpInputRead) -> SdpAdmittedInput {
    match read {
        SdpInputRead::Data(input) => input,
        SdpInputRead::History(_) => panic!("expected current original input"),
    }
}
#[tokio::test]
async fn root_sdp_input_admission_lends_original_bytes_without_engine_or_token_effect() {
    let (mut fixture, action) = opened_fixture().await;
    let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
    let high = 1 - *low;
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    let ordinary = fill_ordinary(node, &grants[high]);
    let before = observations(node);
    let input = data(
        node.with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| {
            step.commit()
        })
        .unwrap(),
    );
    assert!(input.belongs_to(&action));
    let CryptoInput::OpenSdp(packet) = input.input() else {
        panic!("original Open input")
    };
    let tag = packet.object_hash();
    assert!(!action.handoff_status().engine.settled);
    assert!(action.handoff_status().engine.receipt.is_none());
    let again = data(
        node.with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| {
            step.commit()
        })
        .unwrap(),
    );
    let CryptoInput::OpenSdp(same) = again.input() else {
        panic!("same input")
    };
    assert_eq!(same.object_hash(), tag);
    assert_eq!(
        observations(node),
        before,
        "read never rebuilds the roster/directory or borrows ordinary capacity"
    );
    // Neither read admitted an engine: unstarted retirement can still close it. The
    // already admitted loan retains the exact paid bytes through later PC/End teardown.
    action.retire_if_unstarted();
    assert!(action.handoff_status().engine.settled);
    assert!(action.input().is_err());
    phases[high].request_close();
    node.stop_voice_endpoint(&grants[high]);
    let CryptoInput::OpenSdp(held) = input.input() else {
        panic!("retained actual input")
    };
    assert_eq!(held.object_hash(), tag);
    assert!(matches!(
        node.with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| step
            .commit())
            .unwrap(),
        SdpInputRead::History(_)
    ));
    drop((input, again, ordinary));
    for phase in phases {
        phase.close();
    }
}
#[tokio::test]
async fn root_sdp_input_rejects_actual_mls_epoch_before_retained_intent_reconciliation() {
    let (mut fixture, action) = opened_fixture().await;
    let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
    let high = 1 - *low;
    let removed = f.d.device_id();
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    let before = observations(node);
    node.group.remove_member(&node.device, &removed).unwrap();
    assert!(
        intents[high].current_at(f.clock.now_ms(), f.clock.monotonic_ms()),
        "the stale retained observation is exactly what must not authorize input"
    );
    assert_eq!(
        node.with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| step
            .commit())
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        observations(node),
        before,
        "refusal must not refresh an old grant/view"
    );
    assert!(action.handoff_status().engine.receipt.is_none());
    action.retire_if_unstarted();
    for phase in phases {
        phase.close();
    }
}
#[tokio::test]
async fn root_sdp_input_settlement_and_pc_close_win_between_prepare_and_final_admission() {
    for settle in [false, true] {
        let (mut fixture, action) = opened_fixture().await;
        let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
        let high = 1 - *low;
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        let result =
            node.with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| {
                if settle {
                    action.settle_engine();
                } else {
                    phases[high].request_close();
                }
                step.commit()
            });
        if settle {
            let SdpInputRead::History(history) = result.unwrap() else {
                panic!("settlement must win")
            };
            assert!(history.engine.settled);
            assert!(history.engine.receipt.is_none());
        } else {
            assert_eq!(result.err(), Some(VoiceError::Unauthorized));
        }
        action.retire_if_unstarted();
        for phase in phases {
            phase.close();
        }
    }
}
#[tokio::test]
async fn root_sdp_input_terminal_history_survives_ack_end_and_compaction() {
    let (mut fixture, action) = opened_fixture().await;
    let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
    let high = 1 - *low;
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = [b'a'; 5]; // original sdp_fixture plaintext b"v=0\r\n"
    let output = voice_v2::sdp::SdpOpenOutput::Plaintext {
        bytes: &bytes,
        claimed_tag: action.output_tag(&bytes).unwrap(),
    };
    // Install the real Root terminal result and fixed delivery, then compact its input.
    // Historical read admission must not need a surviving participant or plaintext.
    let result = node
        .submit_voice_sdp_open_result(&grants[high], &action, output)
        .unwrap();
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    assert!(action.input().is_err());
    node.stop_voice_endpoint(&grants[high]);
    let SdpInputRead::History(history) = node
        .with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| {
            step.commit()
        })
        .unwrap()
    else {
        panic!("known terminal after End")
    };
    assert!(history.engine.settled && history.acknowledged);
    assert!(history
        .engine
        .receipt
        .unwrap()
        .same_transition(&result.receipt));
    for phase in phases {
        phase.close();
    }
}

#[tokio::test]
async fn root_sdp_input_refreshes_action_expiry_after_actor_preparation_clock_callback() {
    use std::sync::atomic::{AtomicBool, Ordering};
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
    let expiry_advance = 101 + voice_v2::CLOCK_SKEW_MS;
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
    assert!(
        intents[high]
            .remaining_at(now, clock.monotonic_ms())
            .unwrap()
            > expiry_advance
    );
    let crossed = Arc::new(AtomicBool::new(false));
    let check = crossed.clone();
    let action_probe = action.clone();
    let gates = node.voice_transport_actor().unwrap().lock_probe_for_test();
    let original = clock.clone();
    // Actual calls: original Action history (2 source monotonic reads), MLS context,
    // grant/source context, current context (3), then Actor origin's first read (6).
    // Advance only at origin preparation, AFTER the original Action observation.
    *hook.on_mono.lock().unwrap() = Some((
        5,
        Box::new(move || {
            assert!(gates(), "Clock must precede actual Group/Policy guards");
            assert!(
                action_probe.result_fence_available_for_test(),
                "Clock must precede Engine"
            );
            original.advance_ms(expiry_advance);
            check.store(true, Ordering::Release);
        }),
    ));
    let before = observations(node);
    let result =
        node.with_voice_sdp_input_admission(&grants[high], &intents[high], &action, |step| {
            assert!(
                crossed.load(Ordering::Acquire),
                "the actual preparation callback ran"
            );
            // The native callback is the final fence. A callback-bearing observation here
            // would violate it, even if its numerical timestamp happened to be unchanged.
            *hook.on_mono.lock().unwrap() = Some((
                0,
                Box::new(|| panic!("Clock beneath native final admission")),
            ));
            step.commit()
        });
    assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    assert!(
        hook.on_mono.lock().unwrap().take().is_some(),
        "commit sampled no Clock"
    );
    assert!(
        intents[high].current_at(clock.now_ms(), clock.monotonic_ms()),
        "Ready/PC is still current; only the packet expired"
    );
    assert_eq!(observations(node), before);
    assert!(action.handoff_status().engine.receipt.is_none());
    action.retire_if_unstarted();
    for phase in phases {
        phase.close();
    }
}
