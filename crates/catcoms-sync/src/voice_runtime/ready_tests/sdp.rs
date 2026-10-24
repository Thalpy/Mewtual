//! Actual Root ingress/namespace regression, using real membership/confirmed native keys.
//! AEAD is mocked and namespace pressure is synthetic: this does not claim256 complete
//! paid packets fit together, browser delivery, real blocked TCP or audible media.
use super::*;
use voice_v2::chain::sdp_handoff::Phase;
use voice_v2::sdp::InitialSdpIntent;

fn observations(node: &VoiceTestNode) -> (u64, u64) {
    let runtime = node.voice_runtime.as_ref().unwrap();
    (runtime.view_builds, runtime.directory_reads)
}
fn fill_ordinary(node: &VoiceTestNode, grant: &Arc<CallGrant>) -> voice_v2::resources::Credit {
    let resources = node.voice_runtime.as_ref().unwrap().resources.clone();
    let group = grant.scope().group;
    resources
        .charge(
            group,
            voice_v2::resources::GROUP_BYTES - resources.group_counts(group).unwrap().0,
            voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap()
}

type SdpFixture = (
    BridgeFixture,
    [Arc<CallGrant>; 2],
    [u8; 32],
    usize,
    [Arc<Phase>; 2],
    [Arc<InitialSdpIntent>; 2],
    Arc<CryptoAction>,
    SignedObject,
);
async fn sdp_fixture() -> SdpFixture {
    let (mut f, grants, handshake, low, keys) = confirmed_with_custody().await;
    let phases = std::array::from_fn(|i| {
        let phase = keys[i].prepare_sdp_handoff().unwrap();
        phase.arm().unwrap();
        phase
    });
    let mut intents: [Option<Arc<InitialSdpIntent>>; 2] = std::array::from_fn(|_| None);
    for _ in 0..80 {
        // This Root-only fixture has no BoundServer maintenance task. Drive its actual
        // production preparation/queue commands, just as that native task does; never
        // inject Ready/receipt state or rely on transport ticks to issue native work.
        let high = 1 - low;
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        match node.prepare_voice_high_ready_connected(&grants[high], handshake) {
            Ok(true) if intents[high].is_some() => {
                match node.queue_voice_high_ready(&grants[high], handshake) {
                    Ok(_) | Err(VoiceError::Capacity) => {}
                    other => panic!("actual HIGH Ready queue: {other:?}"),
                }
            }
            Ok(true) => {}
            Ok(false) | Err(VoiceError::Capacity) => {}
            other => panic!("actual HIGH Ready preparation: {other:?}"),
        }
        let node = if low == 0 { &mut f.a } else { &mut f.c };
        match node.prepay_voice_low_sdp(&grants[low], handshake) {
            Ok(()) | Err(VoiceError::Capacity) => {}
            other => panic!("actual LOW Ready prepayment: {other:?}"),
        }
        match node.prepare_voice_low_sdp(&grants[low], handshake) {
            Ok(_) | Err(VoiceError::Capacity) | Err(VoiceError::Unauthorized) => {}
            other => panic!("actual LOW Ready preparation: {other:?}"),
        }
        f.tick(3).await;
        for i in 0..2 {
            if intents[i].is_some() {
                continue;
            }
            let node = if i == 0 { &mut f.a } else { &mut f.c };
            let ready = node
                .voice_runtime
                .as_ref()
                .unwrap()
                .endpoints
                .participant_join_for_test()
                .unwrap()
                .sdp_round_for_test(handshake);
            if ready.is_some() {
                intents[i] = Some(
                    node.prepare_voice_initial_sdp_intent(
                        &grants[i],
                        handshake,
                        keys[i].clone(),
                        phases[i].effect(),
                        1,
                    )
                    .unwrap(),
                );
            }
        }
        if intents.iter().all(Option::is_some) {
            break;
        }
        f.clock.advance_ms(25);
    }
    let intents = [
        intents[0].take().expect("actual A Ready"),
        intents[1].take().expect("actual C Ready"),
    ];
    assert_eq!(intents[0].round(), intents[1].round());
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    let action = low_node
        .prepare_voice_low_sdp_offer(&grants[low], handshake, &intents[low], b"v=0\r\n")
        .unwrap();
    let input = action.input().unwrap();
    let CryptoInput::SealSdp(flight) = input.as_input() else {
        panic!("original LOW seal")
    };
    let ciphertext = vec![42; flight.plaintext.len() + 16];
    let ordinary = fill_ordinary(low_node, &grants[low]);
    let before = observations(low_node);
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = low_node
        .submit_voice_sdp_result(
            &grants[low],
            &action,
            &ciphertext,
            action.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    drop(input);
    let retry = low_node
        .submit_voice_sdp_result(
            &grants[low],
            &action,
            &ciphertext,
            action.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    assert!(retry.same_transition(&receipt));
    assert_eq!(
        observations(low_node),
        before,
        "seal and exact retry use the already-paid Actor/directory, even at full ordinary capacity"
    );
    drop(ordinary);
    let offer = low_node.voice_runtime.as_ref().unwrap().endpoints.participant_join_for_test().unwrap().published_source_packets()
        .find(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_offer())).unwrap().clone();
    (f, grants, handshake, low, phases, intents, action, offer)
}
fn answer(
    node: &VoiceTestNode,
    offer: &SignedObject,
    id: u8,
    edit: impl FnOnce(&mut voice_v2::EncryptedHeader),
) -> SignedObject {
    let Body::Encrypted { header, .. } = offer.body() else {
        panic!("Offer")
    };
    let mut header = voice_v2::EncryptedHeader {
        source_join: header.target_join,
        target: offer.issuer(),
        target_join: header.source_join,
        source_key_ref: header.target_key_ref,
        target_key_ref: header.source_key_ref,
        handshake: header.handshake,
        low_to_high: false,
        kind: EncryptedKind::AnswerRound(header.kind.round().unwrap()),
        sequence: 4,
        offer: header.offer,
    };
    edit(&mut header);
    SignedObject::issue(
        offer.scope(),
        &node.device,
        [id; 32],
        offer.issued(),
        offer.expires(),
        Body::Encrypted {
            header,
            ciphertext: vec![42; 32],
        },
    )
    .unwrap()
}

#[tokio::test]
async fn root_answer_reserved_slot_survives_full_namespace_and_recovers_after_actual_token_consumption(
) {
    let (mut f, grants, handshake, low, phases, intents, _, offer) = sdp_fixture().await;
    let high = 1 - low;
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    let object = answer(high_node, &offer, 224, |_| {});
    let wrong_round = answer(high_node, &offer, 225, |header| {
        let mut round = header.kind.round().unwrap();
        round.ready[0] ^= 1;
        header.kind = EncryptedKind::AnswerRound(round);
    });
    let wrong_offer = answer(high_node, &offer, 226, |header| {
        header.offer.as_mut().unwrap().message[0] ^= 1;
    });
    let old_kind = answer(high_node, &offer, 227, |header| {
        header.kind = EncryptedKind::Answer;
    });
    let collision = SignedObject::issue(
        object.scope(),
        &high_node.device,
        object.id(),
        object.issued(),
        object.expires(),
        Body::Announcement {
            key: if high == 0 {
                public_join_key(141)
            } else {
                public_join_key(142)
            },
            epoch: high_node.group.epoch(),
            renewal: 2,
        },
    )
    .unwrap();
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    let held = low_node.occupy_voice_receipt_namespace_for_test();
    assert!(!held.is_empty());
    for unrelated in [wrong_round, wrong_offer, old_kind] {
        assert_eq!(
            admit(low_node, Envelope::Direct(unrelated)).err(),
            Some(VoiceError::Capacity),
            "unrelated signed input gets no reserved Answer exemption"
        );
    }
    let EndpointReceipt::Answer(first) = admit(low_node, Envelope::Direct(object.clone())).unwrap()
    else {
        panic!("actual Root Answer variant")
    };
    assert_eq!(
        admit(low_node, Envelope::Direct(collision)).err(),
        Some(VoiceError::RetryMismatch)
    );
    let action = low_node
        .prepare_voice_sdp_open(&grants[low], handshake, &intents[low])
        .unwrap();
    let ordinary = fill_ordinary(low_node, &grants[low]);
    let before = observations(low_node);
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = [b'a'; 16];
    let output = voice_v2::sdp::SdpOpenOutput::Plaintext {
        bytes: &bytes,
        claimed_tag: action.output_tag(&bytes).unwrap(),
    };
    let result = low_node
        .submit_voice_sdp_open_result(&grants[low], &action, output)
        .unwrap();
    assert_eq!(
        result.receipt.outcome,
        voice_v2::crypto_actions::CryptoOutcome::Opened
    );
    let delivery = result.delivery.unwrap();
    assert!(!delivery.was_admitted());
    low_node
        .admit_voice_sdp_delivery(&grants[low], &action, &delivery)
        .unwrap();
    assert!(delivery.was_admitted());
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    assert!(action.input().is_err());
    let exact = low_node
        .submit_voice_sdp_open_result(&grants[low], &action, output)
        .unwrap();
    assert!(exact.receipt.same_transition(&result.receipt));
    assert!(Arc::ptr_eq(&delivery, &exact.delivery.unwrap()));
    assert_eq!(
        observations(low_node),
        before,
        "Open/recovery/delivery never rebuild roster or directory under pressure"
    );
    drop(ordinary);
    let EndpointReceipt::Answer(retry) = admit(low_node, Envelope::Direct(object)).unwrap() else {
        panic!("same Root Answer receipt")
    };
    assert!(Arc::ptr_eq(&first, &retry));
    phases[low].close();
    assert_eq!(
        low_node.admit_voice_sdp_delivery(&grants[low], &action, &delivery),
        Err(VoiceError::Unauthorized)
    );
    drop(held);
    for phase in phases {
        phase.close();
    }
}

#[tokio::test]
async fn root_unused_answer_namespace_position_returns_after_actual_pc_close_and_end() {
    for end in [false, true] {
        let (mut f, grants, _, low, phases, _, _, _) = sdp_fixture().await;
        let node = if low == 0 { &mut f.a } else { &mut f.c };
        let before = node.occupy_voice_receipt_namespace_for_test();
        assert!(!before.is_empty());
        if end {
            node.stop_voice_endpoint(&grants[low]);
        } else {
            phases[low].close();
            node.voice_runtime
                .as_mut()
                .unwrap()
                .endpoints
                .participant_join_mut_for_test()
                .unwrap()
                .collect_expired(f.clock.as_ref());
        }
        let returned = node.occupy_voice_receipt_namespace_for_test();
        assert_eq!(
            returned.len(),
            1,
            "exact unused Answer namespace position returned"
        );
        drop((before, returned));
        for phase in phases {
            phase.close();
        }
    }
}

#[tokio::test]
async fn root_original_offer_open_applied_answer_seal_and_answer_open_complete_without_legacy_reservation(
) {
    let (mut f, grants, handshake, low, phases, intents, _, offer) = sdp_fixture().await;
    let high = 1 - low;
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    assert!(matches!(
        admit(high_node, Envelope::Direct(offer.clone())).unwrap(),
        EndpointReceipt::FirstOffer(_)
    ));
    let opened = high_node
        .prepare_voice_sdp_open(&grants[high], handshake, &intents[high])
        .unwrap();
    let ordinary = fill_ordinary(high_node, &grants[high]);
    let before = observations(high_node);
    opened.admit_engine(opened.engine_bytes()).unwrap();
    let plaintext = b"v=0\r\n";
    let result = high_node
        .submit_voice_sdp_open_result(
            &grants[high],
            &opened,
            voice_v2::sdp::SdpOpenOutput::Plaintext {
                bytes: plaintext,
                claimed_tag: opened.output_tag(plaintext).unwrap(),
            },
        )
        .unwrap();
    let delivery = result.delivery.unwrap();
    assert_eq!(
        high_node
            .prepare_voice_high_sdp_answer(
                &grants[high],
                handshake,
                &intents[high],
                &delivery,
                plaintext
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    high_node
        .admit_voice_sdp_delivery(&grants[high], &opened, &delivery)
        .unwrap();
    assert_eq!(
        high_node
            .prepare_voice_high_sdp_answer(
                &grants[high],
                handshake,
                &intents[high],
                &delivery,
                plaintext
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    // Test-only trusted SDK success boundary. No browser or real RTC is implied by this
    // native completion marker; its actual private producer remains an integration task.
    high_node
        .complete_voice_sdp_delivery(&grants[high], &opened, &delivery)
        .unwrap();
    opened.acknowledge_result(&result.receipt).unwrap();
    opened.settle_engine();
    high_node
        .admit_voice_sdp_delivery(&grants[high], &opened, &delivery)
        .unwrap();
    high_node
        .complete_voice_sdp_delivery(&grants[high], &opened, &delivery)
        .unwrap();
    assert!(delivery.was_applied());
    let seal = high_node
        .prepare_voice_high_sdp_answer(
            &grants[high],
            handshake,
            &intents[high],
            &delivery,
            plaintext,
        )
        .unwrap();
    seal.admit_engine(seal.engine_bytes()).unwrap();
    let ciphertext = [43; 21];
    let sealed = high_node
        .submit_voice_sdp_result(
            &grants[high],
            &seal,
            &ciphertext,
            seal.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    seal.acknowledge_result(&sealed).unwrap();
    seal.settle_engine();
    assert!(Arc::ptr_eq(
        &seal,
        &high_node
            .prepare_voice_high_sdp_answer(
                &grants[high],
                handshake,
                &intents[high],
                &delivery,
                plaintext
            )
            .unwrap()
    ));
    assert_eq!(
        observations(high_node),
        before,
        "HIGH full original round uses paid Actor/directory only"
    );
    let answer = high_node.voice_runtime.as_ref().unwrap().endpoints.participant_join_for_test().unwrap()
        .published_source_packets().find(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind == EncryptedKind::AnswerRound(intents[high].round()))).unwrap().clone();
    let Body::Encrypted { header, .. } = answer.body() else {
        panic!("actual signed Answer")
    };
    assert_eq!(header.offer, Some(delivery.offer()));
    assert_eq!(header.kind.round(), Some(intents[low].round()));
    drop(ordinary);
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    assert!(matches!(
        admit(low_node, Envelope::Direct(answer)).unwrap(),
        EndpointReceipt::Answer(_)
    ));
    let opened = low_node
        .prepare_voice_sdp_open(&grants[low], handshake, &intents[low])
        .unwrap();
    let ordinary = fill_ordinary(low_node, &grants[low]);
    opened.admit_engine(opened.engine_bytes()).unwrap();
    let result = low_node
        .submit_voice_sdp_open_result(
            &grants[low],
            &opened,
            voice_v2::sdp::SdpOpenOutput::Plaintext {
                bytes: plaintext,
                claimed_tag: opened.output_tag(plaintext).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        result.receipt.outcome,
        voice_v2::crypto_actions::CryptoOutcome::Opened
    );
    let delivery = result.delivery.unwrap();
    low_node
        .admit_voice_sdp_delivery(&grants[low], &opened, &delivery)
        .unwrap();
    low_node
        .complete_voice_sdp_delivery(&grants[low], &opened, &delivery)
        .unwrap();
    assert_eq!(
        low_node
            .prepare_voice_high_sdp_answer(
                &grants[low],
                handshake,
                &intents[low],
                &delivery,
                plaintext
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    opened.acknowledge_result(&result.receipt).unwrap();
    opened.settle_engine();
    drop(ordinary);
    for phase in phases {
        phase.close();
    }
}
