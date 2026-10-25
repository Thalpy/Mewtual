//! Original outcome and actual Root tail tests on PC/Phase/Action fixtures. Actor and
//! production native/browser integration remain separate unfinished coverage.
use super::*;
use crate::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use crate::voice_v2::sdp::{
    plaintext_commitment,
    publication::{History, Prepared},
};

#[test]
fn recorded_selection_defeats_prepared_unrecorded_tail_and_cached_recovery() {
    let (mut p, phase, intent, exchange) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let body = b"v=0\r\n";
    let tag = Some(plaintext_commitment(body).unwrap());
    let before = p.f.low.join.ledger.initial_offer_state_for_test(p.f.id);
    let mut namespace = ReceiptNamespace::default();
    let mut candidate = None;
    // The old factory passed its early check and prepared actual input, packet and
    // sequence capacity. A recorded candidate wins before its first final mutation.
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, body, |_, _| {
            candidate = Some(Prepared::new(intent.clone(), fee.clone(), tag).unwrap());
            Ok(())
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(!exchange.is_live());
    assert_eq!(
        p.f.low.join.ledger.initial_offer_state_for_test(p.f.id),
        before
    );
    let slot = candidate.unwrap().install();
    let mut writer = slot.try_writer().unwrap();
    writer.enter(&intent, tag).unwrap();
    let action = offer_with_outcome(
        &mut p,
        &intent,
        &mut namespace,
        body,
        |_, _| Ok(()),
        Some(&mut writer),
    )
    .unwrap();
    assert!(matches!(slot.history(), History::Published(_)));
    // Even exact cached history must not cross the profile boundary.
    assert_eq!(
        offer(&mut p, &intent, &mut namespace, body, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    action.settle_engine();
    phase.close();
    assert_eq!(
        intent.admit_publication_profile(false),
        Err(VoiceError::Unauthorized)
    );
}

#[test]
fn actual_unrecorded_publication_defeats_later_recorded_candidate() {
    let (mut p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let body = b"v=0\r\n";
    let tag = Some(plaintext_commitment(body).unwrap());
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, body, |_, _| Ok(())).unwrap();
    assert!(matches!(
        Prepared::new(intent.clone(), fee.clone(), tag),
        Err(VoiceError::Unauthorized)
    ));
    let recovered = offer(&mut p, &intent, &mut namespace, body, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&action, &recovered));
    action.settle_engine();
    phase.close();
    assert!(matches!(
        Prepared::new(intent, fee, tag),
        Err(VoiceError::Unauthorized)
    ));
}

#[test]
fn original_outcome_never_entered_and_entered_unwind_are_distinct_after_close() {
    let (_p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let tag = Some(plaintext_commitment(b"v=0\r\n").unwrap());
    let slot = Prepared::new(intent.clone(), fee.clone(), tag)
        .unwrap()
        .install();
    assert!(matches!(slot.history(), History::Idle));
    let writer = slot.try_writer().unwrap();
    assert_eq!(slot.try_writer().err(), Some(VoiceError::Capacity));
    drop(writer);
    assert!(matches!(slot.history(), History::NeverEntered));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut writer = slot.try_writer().unwrap();
        writer.enter(&intent, tag).unwrap();
        panic!("entered Root invocation without publication/refusal evidence");
    }));
    assert!(result.is_err());
    assert!(matches!(slot.history(), History::Uncertain));
    assert_eq!(slot.try_writer().err(), Some(VoiceError::Unauthorized));
    phase.close();
    assert!(fee.is_collected());
    assert!(matches!(slot.history(), History::Uncertain));
}

#[test]
fn explicit_capacity_retry_reuses_slot_without_old_writer_drop_corrupting_new_attempt() {
    let (_p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let tag = Some(plaintext_commitment(b"v=0\r\n").unwrap());
    let slot = Prepared::new(intent.clone(), fee, tag).unwrap().install();
    let mut old = slot.try_writer().unwrap();
    old.enter(&intent, tag).unwrap();
    old.refused(VoiceError::Capacity);
    assert!(matches!(
        slot.history(),
        History::Refused(VoiceError::Capacity)
    ));
    let mut current = slot.try_writer().unwrap();
    assert_eq!(old.enter(&intent, tag), Err(VoiceError::Unauthorized));
    old.refused(VoiceError::Bounds); // retired token cannot rewrite its successor
    drop(old);
    current.enter(&intent, tag).unwrap();
    current.refused(VoiceError::Capacity);
    drop(current);
    phase.close();
    assert!(matches!(
        slot.history(),
        History::Refused(VoiceError::Capacity)
    ));
    assert_eq!(slot.try_writer().err(), Some(VoiceError::Unauthorized));
}

#[test]
fn negative_outcome_does_not_collect_actual_bound_io_or_retain_phase() {
    let (_p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let tag = Some(plaintext_commitment(b"v=0\r\n").unwrap());
    let slot = Prepared::new(intent, fee.clone(), tag).unwrap().install();
    let writer = slot.try_writer().unwrap();
    let mut copy = phase.copy(1).unwrap();
    copy.bind_native_history(fee.clone()).unwrap();
    drop(writer);
    phase.close();
    assert!(matches!(slot.history(), History::NeverEntered));
    assert!(
        !fee.is_collected(),
        "known negative is not actual IO collection"
    );
    drop(copy);
    assert!(fee.is_collected());
    let weak = Arc::downgrade(&phase);
    drop(phase);
    assert!(weak.upgrade().is_none(), "fixed slot must not retain Phase");
}

#[test]
fn original_outcome_refuses_foreign_equal_label_intent_fee_purpose_and_changed_body() {
    let (_p, phase, intent, _) = fixture();
    let (_other, other_phase, foreign, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let tag = Some(plaintext_commitment(b"v=0\r\n").unwrap());
    assert!(matches!(
        Prepared::new(foreign.clone(), fee.clone(), tag),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        Prepared::new(intent.clone(), fee.clone(), None),
        Err(VoiceError::Unauthorized)
    ));
    assert!(intent.check_publication_profile(false).is_ok());
    assert!(foreign.check_publication_profile(false).is_ok());
    let slot = Prepared::new(intent.clone(), fee, tag).unwrap().install();
    let mut writer = slot.try_writer().unwrap();
    assert_eq!(writer.enter(&foreign, tag), Err(VoiceError::Unauthorized));
    assert_eq!(
        writer.enter(&intent, Some(plaintext_commitment(b"v=1\r\n").unwrap())),
        Err(VoiceError::Unauthorized)
    );
    assert!(matches!(slot.history(), History::InFlight));
    drop(writer);
    assert!(matches!(slot.history(), History::NeverEntered));
    phase.close();
    other_phase.close();
}

#[test]
fn positive_original_slot_keeps_same_real_action_after_input_and_join_teardown() {
    let (mut p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let body = b"v=0\r\n";
    let tag = Some(plaintext_commitment(body).unwrap());
    let slot = Prepared::new(intent.clone(), fee, tag).unwrap().install();
    let mut writer = slot.try_writer().unwrap();
    writer.enter(&intent, tag).unwrap();
    let mut namespace = ReceiptNamespace::default();
    let action = offer_with_outcome(
        &mut p,
        &intent,
        &mut namespace,
        body,
        |_, _| Ok(()),
        Some(&mut writer),
    )
    .unwrap();
    writer.refused(VoiceError::Capacity);
    drop(writer);
    action.settle_engine();
    assert!(action.input().is_err());
    phase.close();
    drop(p);
    let weak = Arc::downgrade(&action);
    drop(action);
    assert!(!intent.current_at(1_000, 1_000));
    let History::Published(recovered) = slot.history() else {
        panic!("retained actual publication");
    };
    assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), &recovered));
    assert!(recovered.matches_sdp_plaintext_commitment(&plaintext_commitment(body).unwrap()));
    assert_eq!(slot.try_writer().err(), Some(VoiceError::Unauthorized));
    drop(slot);
    assert!(weak.upgrade().is_some());
    drop(recovered);
    assert!(weak.upgrade().is_none(), "actual last Action owner retired");
}

#[test]
fn close_between_funding_observation_and_reservation_never_returns_a_writer() {
    let (_p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let tag = Some(plaintext_commitment(b"v=0\r\n").unwrap());
    let slot = Prepared::new(intent, fee.clone(), tag).unwrap().install();
    let observed = std::sync::Barrier::new(2);
    let release = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            slot.try_writer_after_observation_for_test(|| {
                observed.wait();
                release.wait();
            })
        });
        observed.wait();
        drop(fee.close_io());
        assert!(fee.is_collected());
        release.wait();
        assert_eq!(task.join().unwrap().err(), Some(VoiceError::Unauthorized));
    });
    assert!(matches!(slot.history(), History::NeverEntered));
    phase.close();
}

#[test]
fn low_root_factory_records_actual_publication_and_explicit_capacity_retries_same_slot() {
    let (mut p, phase, intent, _) = fixture();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let body = b"v=0\r\n";
    let tag = Some(plaintext_commitment(body).unwrap());
    let slot = Prepared::new(intent.clone(), fee, tag).unwrap().install();
    let mut namespace = ReceiptNamespace::default();
    // The real final origin refuses before any Action exists. This direct factory test
    // explicitly supplies the normal-return refusal; the recorded Root endpoint does that
    // automatically. An entered unwind would stay uncertain instead.
    let mut first = slot.try_writer().unwrap();
    first.enter(&intent, tag).unwrap();
    let position = namespace.reserve(0, 1).unwrap();
    let refused = p.f.low.join.prepare_low_sdp_offer(
        &p.low,
        &room(&p.f.low),
        p.f.id,
        &intent,
        body,
        Some(position),
        &namespace,
        &ca(&p.f.low.context, &p.f.low.lease),
        p._runtimes[0].clock(),
        &mut p.f.rng,
        |_, _| Err(VoiceError::Capacity),
        Some(&mut first),
    );
    assert_eq!(refused.err(), Some(VoiceError::Capacity));
    first.refused(VoiceError::Capacity);
    drop(first);
    assert!(matches!(
        slot.history(),
        History::Refused(VoiceError::Capacity)
    ));
    let mut writer = slot.try_writer().unwrap();
    writer.enter(&intent, tag).unwrap();
    let position = namespace.reserve(0, 1).unwrap();
    let action =
        p.f.low
            .join
            .prepare_low_sdp_offer(
                &p.low,
                &room(&p.f.low),
                p.f.id,
                &intent,
                body,
                Some(position),
                &namespace,
                &ca(&p.f.low.context, &p.f.low.lease),
                p._runtimes[0].clock(),
                &mut p.f.rng,
                |_, _| Ok(()),
                Some(&mut writer),
            )
            .unwrap();
    // No test writer.published call: ONLY the actual Root final tail can satisfy this.
    let History::Published(original) = slot.history() else {
        panic!("Root tail did not capture publication");
    };
    assert!(Arc::ptr_eq(&original, &action));
    drop(original);
    drop(writer);
    action.settle_engine();
    phase.close();
    drop(p);
    let weak = Arc::downgrade(&action);
    drop(action);
    let History::Published(recovered) = slot.history() else {
        panic!("original lost after teardown");
    };
    assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), &recovered));
    drop(slot);
    drop(recovered);
    assert!(weak.upgrade().is_none());
}
