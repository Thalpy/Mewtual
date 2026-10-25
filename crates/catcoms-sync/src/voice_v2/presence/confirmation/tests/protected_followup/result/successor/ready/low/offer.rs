//! Actual LOW Ready and original PC intent, including inert lane and Root receipt pressure.
use super::*;
use crate::voice_v2::chain::sdp_handoff::Phase;
use crate::voice_v2::crypto_actions::{CryptoAction, CryptoInput, CryptoOperation};
use crate::voice_v2::negotiation::{namespace::ReceiptNamespace, Exchange};
use crate::voice_v2::sdp::InitialSdpIntent;
mod answer;
mod result;

/// Native packet-boundary fixture only: AEAD/result IPC is intentionally not simulated
/// by this helper. It publishes actual signed bytes to exercise downstream intake/Source.
fn sign_offer(p: &mut ProtectedFixture, action: &Arc<CryptoAction>) -> SignedObject {
    let input = action.input().unwrap();
    let CryptoInput::SealSdp(flight) = input.as_input() else {
        panic!("actual LOW Offer")
    };
    p.f.low
        .join
        .ledger
        .submit(
            flight.reservation.id,
            &vec![42; flight.plaintext.len() + 16],
            &p.f.low.signer,
            &ca(&p.f.low.context, &p.f.low.lease),
            &p.f.clock,
        )
        .unwrap()
}

#[tokio::test]
async fn low_sdp_source_observes_actual_pc_close_while_exchange_itself_stays_live() {
    use catcoms_rt::RequestAuthority;
    use std::future::Future;
    use std::task::Poll;
    let (mut p, phase, intent, exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).unwrap();
    let signed = sign_offer(&mut p, &action);
    let source =
        p.f.low
            .join
            .ledger
            .source_authority(
                &signed,
                &ca(&p.f.low.context, &p.f.low.lease),
                p._runtimes[0].clock(),
            )
            .unwrap();
    assert!(source.authorized());
    let mut revoked = std::pin::pin!(source.revoked());
    std::future::poll_fn(|cx| {
        assert!(revoked.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    phase.request_close();
    assert!(
        exchange.is_live(),
        "PC close is an independent effect boundary"
    );
    assert!(
        !source.authorized(),
        "queued source cannot use Exchange liveness as PC authority"
    );
    // Native's pure final gate flips under its UI fence; the affine CloseTicket
    // notifies outside that fence. Verify immediate refusal and actual notification.
    phase.close();
    std::future::poll_fn(|cx| {
        assert!(revoked.as_mut().poll(cx).is_ready());
        Poll::Ready(())
    })
    .await;
    assert_eq!(
        action.admit_engine(action.engine_bytes()),
        Err(VoiceError::Unauthorized)
    );
    action.settle_engine();
    phase.close();
}

fn fixture() -> (
    ProtectedFixture,
    Arc<Phase>,
    Arc<InitialSdpIntent>,
    Arc<Exchange>,
) {
    let (mut p, keys) = confirmed_with_low_keys();
    let ready = install(&mut p, &[]);
    prepay(&mut p);
    admit_ready(&mut p, &ready).unwrap();
    let successor = advance(&mut p).unwrap();
    let phase = keys.prepare_sdp_handoff().unwrap();
    phase.arm().unwrap();
    let intent =
        p.f.low
            .join
            .prepare_initial_sdp_intent(
                &p.low,
                &room(&p.f.low),
                p.f.id,
                &keys,
                phase.effect(),
                1,
                &ca(&p.f.low.context, &p.f.low.lease),
            )
            .unwrap();
    (p, phase, intent, successor.owner)
}
fn offer(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    namespace: &mut ReceiptNamespace,
    body: &[u8],
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<CryptoAction>, VoiceError> {
    let position = if p.f.low.join.has_initial_sdp_seal(p.f.id) {
        None
    } else {
        Some(namespace.reserve(0, 1)?)
    };
    p.f.low.join.prepare_low_sdp_offer(
        &p.low,
        &room(&p.f.low),
        p.f.id,
        intent,
        body,
        position,
        namespace,
        &ca(&p.f.low.context, &p.f.low.lease),
        p._runtimes[0].clock(),
        &mut p.f.rng,
        final_owner,
    )
}

#[test]
fn low_sdp_offer_final_refusal_restores_original_capacity_without_sequence_or_activation() {
    let (mut p, phase, intent, exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let before = p.f.low.join.ledger.initial_offer_state_for_test(p.f.id);
    let resources = p._runtimes[0].resources();
    let counts = resources.counts();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    for _ in 0..3 {
        assert_eq!(
            offer(&mut p, &intent, &mut namespace, &bytes, |_, _| {
                assert!(!exchange.is_live(), "preparation is not activation");
                Err(VoiceError::Capacity)
            })
            .err(),
            Some(VoiceError::Capacity)
        );
        assert!(!exchange.is_live());
        assert_eq!(
            p.f.low.join.ledger.initial_offer_state_for_test(p.f.id),
            before
        );
        assert_eq!(resources.counts(), counts);
        assert_eq!(namespace.total_for_test(0), 0);
    }
    let action = offer(&mut p, &intent, &mut namespace, &bytes, |_, _| Ok(())).unwrap();
    assert_eq!(action.operation(), CryptoOperation::SealSdp);
    assert!(exchange.is_live());
    let input = action.input().unwrap();
    let CryptoInput::SealSdp(flight) = input.as_input() else {
        panic!("actual native seal")
    };
    assert_eq!(flight.plaintext, bytes);
    assert_eq!(flight.reservation.sequence, before.0 + 1);
    assert_eq!(
        p.f.low.join.ledger.initial_offer_state_for_test(p.f.id),
        (before.0 + 1, Some(flight.reservation.id))
    );
    assert_eq!(
        namespace.total_for_test(0),
        1,
        "future Answer owns shared namespace position"
    );
    let retry = offer(&mut p, &intent, &mut namespace, &bytes, |_, _| {
        panic!("one publication")
    })
    .unwrap();
    assert!(Arc::ptr_eq(&action, &retry));
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, b"changed SDP", |_, _| Ok(
            ()
        ))
        .err(),
        Some(VoiceError::RetryMismatch)
    );
    action.settle_engine();
    phase.close();
}

#[test]
fn low_sdp_offer_requires_its_own_root_position_and_close_wins_before_final_publication() {
    let (mut p, phase, intent, exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let blocker = namespace.reserve(0, 1).unwrap();
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::Capacity)
    );
    drop(blocker);
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| {
            phase.request_close();
            Ok(())
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(!exchange.is_live());
    assert_eq!(namespace.total_for_test(0), 0);
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(!p.f.low.join.has_initial_sdp_seal(p.f.id));
    phase.close();
}

#[test]
fn low_sdp_offer_recovers_same_fixed_action_after_input_compaction_without_new_nonce() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let body = b"v=0\r\n";
    let action = offer(&mut p, &intent, &mut namespace, body, |_, _| Ok(())).unwrap();
    let exact = crate::voice_v2::sdp::plaintext_commitment(body).unwrap();
    let changed = crate::voice_v2::sdp::plaintext_commitment(b"v=1\r\n").unwrap();
    assert!(action.matches_sdp_plaintext_commitment(&exact));
    assert!(!action.matches_sdp_plaintext_commitment(&changed));
    action.settle_engine();
    assert!(
        action.input().is_err(),
        "proven unstarted retirement compacts actual body"
    );
    let before = p.f.low.join.ledger.initial_offer_state_for_test(p.f.id);
    assert!(Arc::ptr_eq(
        &action,
        &offer(&mut p, &intent, &mut namespace, body, |_, _| panic!(
            "historical retry"
        ))
        .unwrap()
    ));
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, b"v=1\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(
        p.f.low.join.ledger.initial_offer_state_for_test(p.f.id),
        before
    );
    assert_eq!(
        action.admit_engine(action.engine_bytes()),
        Err(VoiceError::Unauthorized)
    );
    phase.close();
    assert!(!intent.current_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms()));
    assert!(
        action.matches_sdp_plaintext_commitment(&exact),
        "pure history survives End and input compaction"
    );
    assert!(!action.matches_sdp_plaintext_commitment(&changed));
}
