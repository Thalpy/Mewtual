//! Actual ChannelSync/MLS engine admission. SDK execution is intentionally absent:
//! a returned original permit is not evidence of WebCrypto, Result, delivery or RTC.
use super::*;
use voice_v2::crypto_actions::SdpEngineRead;

fn permit(read: SdpEngineRead) -> u64 {
    match read {
        SdpEngineRead::Permit(serial) => serial,
        SdpEngineRead::History(_) => panic!("pending original engine"),
    }
}

#[tokio::test]
async fn root_sdp_engine_actual_actor_reuses_one_permit_under_full_ordinary_pressure() {
    let (mut fixture, action) = input::opened_fixture().await;
    let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
    let high = 1 - *low;
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    let ordinary = fill_ordinary(node, &grants[high]);
    let before = observations(node);
    let resources = node.voice_runtime.as_ref().unwrap().resources.clone();
    for attempt in 0..4 {
        let original = node
            .with_voice_sdp_engine_admission(
                &grants[high],
                &intents[high],
                &action,
                action.engine_bytes(),
                |step| step.commit(),
            )
            .unwrap();
        assert_eq!(
            permit(original),
            action.serial(),
            "lost reply retries reuse the SAME serial"
        );
        let charged = resources.counts();
        if attempt > 0 {
            let again = node
                .with_voice_sdp_engine_admission(
                    &grants[high],
                    &intents[high],
                    &action,
                    action.engine_bytes(),
                    |step| step.commit(),
                )
                .unwrap();
            assert_eq!(permit(again), action.serial());
            assert_eq!(resources.counts(), charged, "no duplicate engine charge");
        }
    }
    assert_eq!(
        observations(node),
        before,
        "no ordinary view/directory rebuild"
    );
    action.retire_if_unstarted();
    assert!(
        !action.handoff_status().engine.settled,
        "admitted engine survives unstarted cleanup"
    );
    action.settle_engine();
    assert!(matches!(
        node.with_voice_sdp_engine_admission(
            &grants[high],
            &intents[high],
            &action,
            action.engine_bytes(),
            |step| step.commit(),
        )
        .unwrap(),
        SdpEngineRead::History(_)
    ));
    drop(ordinary);
    for phase in phases {
        phase.close();
    }
}

#[tokio::test]
async fn root_sdp_engine_rejects_unreconciled_actual_mls_epoch_despite_live_intent() {
    let (mut fixture, action) = input::opened_fixture().await;
    let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
    let high = 1 - *low;
    let removed = f.d.device_id();
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    let before = observations(node);
    node.group.remove_member(&node.device, &removed).unwrap();
    assert!(intents[high].current_at(f.clock.now_ms(), f.clock.monotonic_ms()));
    assert_eq!(
        node.with_voice_sdp_engine_admission(
            &grants[high],
            &intents[high],
            &action,
            action.engine_bytes(),
            |step| step.commit(),
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(observations(node), before);
    action.retire_if_unstarted();
    assert!(
        action.handoff_status().engine.settled,
        "stale intent installed no permit"
    );
    for phase in phases {
        phase.close();
    }
}

#[tokio::test]
async fn root_sdp_engine_terminal_and_settlement_races_win_before_current_admission() {
    for terminal in [false, true] {
        let (mut fixture, action) = input::opened_fixture().await;
        let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
        let high = 1 - *low;
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        let original = if terminal {
            action.admit_engine(action.engine_bytes()).unwrap();
            let bytes = [b'a'; 5];
            let output = voice_v2::sdp::SdpOpenOutput::Plaintext {
                bytes: &bytes,
                claimed_tag: action.output_tag(&bytes).unwrap(),
            };
            let result = node
                .submit_voice_sdp_open_result(&grants[high], &action, output)
                .unwrap();
            action.acknowledge_result(&result.receipt).unwrap();
            action.settle_engine();
            node.stop_voice_endpoint(&grants[high]);
            Some(result.receipt)
        } else {
            None
        };
        let read = node
            .with_voice_sdp_engine_admission(
                &grants[high],
                &intents[high],
                &action,
                action.engine_bytes(),
                |step| {
                    if !terminal {
                        action.settle_engine();
                        phases[high].request_close();
                    }
                    step.commit()
                },
            )
            .unwrap();
        let SdpEngineRead::History(history) = read else {
            panic!("history wins")
        };
        assert!(history.engine.settled);
        if let Some(receipt) = original {
            assert!(history.acknowledged);
            assert!(history.engine.receipt.unwrap().same_transition(&receipt));
        } else {
            assert!(history.engine.receipt.is_none());
        }
        for phase in phases {
            phase.close();
        }
    }
}

#[tokio::test]
async fn root_sdp_engine_refusal_omitted_commit_and_unwind_return_original_candidate() {
    for mode in 0..3 {
        let (mut fixture, action) = input::opened_fixture().await;
        let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
        let high = 1 - *low;
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        let resources = node.voice_runtime.as_ref().unwrap().resources.clone();
        let before = resources.counts();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            node.with_voice_sdp_engine_admission(
                &grants[high],
                &intents[high],
                &action,
                action.engine_bytes(),
                |_step| -> Result<(), VoiceError> {
                    match mode {
                        0 => Err(VoiceError::Capacity),
                        1 => Ok(()), // callback success is not an engine permit
                        _ => panic!("native engine callback unwind"),
                    }
                },
            )
        }));
        match mode {
            0 => assert_eq!(result.unwrap().err(), Some(VoiceError::Capacity)),
            1 => assert!(result.unwrap().is_ok()),
            _ => assert!(result.is_err()),
        }
        assert_eq!(
            resources.counts(),
            before,
            "no unused candidate escapes callback"
        );
        action.retire_if_unstarted();
        assert!(action.handoff_status().engine.settled);
        for phase in phases {
            phase.close();
        }
    }
}

#[tokio::test]
async fn root_sdp_engine_pending_close_end_and_wrong_payment_refuse_first_permit() {
    for mode in 0..3 {
        let (mut fixture, action) = input::opened_fixture().await;
        let (f, grants, _, low, phases, intents, _, _) = fixture.as_mut();
        let high = 1 - *low;
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        if mode == 1 {
            node.stop_voice_endpoint(&grants[high]);
        }
        let bytes = action.engine_bytes() + usize::from(mode == 2);
        let result = node.with_voice_sdp_engine_admission(
            &grants[high],
            &intents[high],
            &action,
            bytes,
            |step| {
                if mode == 0 {
                    phases[high].request_close();
                }
                step.commit()
            },
        );
        assert_eq!(
            result.err(),
            Some(if mode == 2 {
                VoiceError::RetryMismatch
            } else {
                VoiceError::Unauthorized
            })
        );
        action.retire_if_unstarted();
        assert!(action.handoff_status().engine.settled);
        for phase in phases {
            phase.close();
        }
    }
}

#[tokio::test]
async fn root_sdp_engine_refreshes_original_packet_expiry_after_actual_actor_clock_callback() {
    input::packet_expiry_during_actor_preparation(true).await;
}
