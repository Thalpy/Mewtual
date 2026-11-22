//! Counter/metadata drainage is not a whole Root Source-close acknowledgement.
//! These use genuine closed Cells but synthetic native command credentials.
use super::*;

#[test]
fn governed_add_whole_source_retirement_destroys_partial_candidate_before_key_drain() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec10);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 95);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    hook(TestCut::BeforeSdkExecution, || {
        panic!("after the first native End")
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.execute_add_data_native(request.data())
    }))
    .is_err());
    assert!(node.pending.as_ref().unwrap().cell.is_some());
    let drain = native.source.observe_retirement();
    assert!(node.discard_add_for_closed_source(&drain).is_err());
    let foreign = Native::new();
    foreign.source.close();
    assert!(node
        .discard_add_for_closed_source(&foreign.source.observe_retirement())
        .is_err());
    let ui_drain = native.ui.retire();
    assert!(node.discard_add_for_closed_source(&ui_drain).is_err());
    assert_eq!(count.load(SeqCst), 1);

    native.source.close();
    assert!(!drain.is_drained());
    node.discard_add_for_closed_source(&drain).unwrap();
    assert!(drain.is_drained());
    assert!(ui_drain.is_drained());
    assert_eq!(count.load(SeqCst), 0);
    assert!(node.pending.is_none());
    assert!(node.require_sdk_current_data().is_err());
    node.discard_add_for_closed_source(&drain).unwrap();
}

#[test]
fn partial_sdk_keeps_original_end_and_finishes_after_lock_without_new_admission() {
    let clock = ManualClock::new(NOW);
    let (original, _reservation, _opened, _old_fee) =
        prepared_node_with_clock(0xec01, Box::new(clock.clone()));
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let old_ui = native.ui.observe_retirement();
    let request = Request::new(&node, 81);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    hook(TestCut::BeforeSdkExecution, || {
        panic!("lost return with private prepared Cell")
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.execute_add_data_native(request.data())
    }))
    .is_err());
    assert!(node.pending.as_ref().unwrap().phase == Phase::SdkEntered);
    assert!(node.pending.as_ref().unwrap().cell.is_some());
    assert!(node.completed_add_data().is_err());
    assert!(node.retire_closed_add_native_data().is_err());
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    clock.advance_ms(120_000); // cannot refresh admission time or re-expire the admitted unit
    assert!(!old_ui.is_drained());
    assert!(node
        .pending
        .as_ref()
        .unwrap()
        .ends
        .holds_admitted_purpose_data(&native.source.observe(), NativeChatKeyPurpose::AddSdk,));
    node.finish_admitted_add_data_native().unwrap();
    assert_eq!(node.pending.as_ref().unwrap().admitted_at_ms, Some(NOW));
    let sealed = node
        .completed_add_data()
        .unwrap()
        .sealed_routing_transfer_data()
        .unwrap()
        .to_vec();
    let rng = node.node._parts.body._rng.get_word_pos();
    let retired = node.retire_closed_add_native_data().unwrap();
    assert!(!old_ui.is_drained()); // custody moved, not silently dropped
    assert!(node.finish_admitted_add_data_native().is_err());
    assert!(node.execute_add_data_native(request.data()).is_err());
    assert_eq!(node.node._parts.body._rng.get_word_pos(), rng);
    drop(retired);
    assert!(old_ui.is_drained());
    assert_eq!(
        node.completed_add_data()
            .unwrap()
            .sealed_routing_transfer_data()
            .unwrap(),
        sealed
    );
    signed_request::require_joiner_routing(&node, &request);
    assert_eq!(count.load(SeqCst), 1); // historical closed graph still pays its SDK lease
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn spent_cut_without_its_actual_retained_end_cannot_continue_partial_sdk() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec02);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 82);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    hook(TestCut::BeforeSdkExecution, || {
        panic!("stop after constructor")
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.execute_add_data_native(request.data())
    }))
    .is_err());
    // Test-only corruption models a caller mistakenly separating finite work
    // from its End. Production has no partial-Cell End extraction API.
    let original_ends = std::mem::take(&mut node.pending.as_mut().unwrap().ends);
    assert!(node.finish_admitted_add_data_native().is_err());
    assert!(node.completed_add_data().is_err());
    assert!(node.retire_closed_add_native_data().is_err());
    node.pending.as_mut().unwrap().ends = original_ends;
    node.finish_admitted_add_data_native().unwrap();
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
}

#[test]
fn closed_data_does_not_keep_obsolete_generation_cut_payments_after_retirement() {
    struct UiPayment(Arc<AtomicUsize>);
    impl Drop for UiPayment {
        fn drop(&mut self) {
            self.0.fetch_add(1, SeqCst);
        }
    }
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec03);
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    native.ui = native
        ._binding
        .open_session(7, Arc::new(UiPayment(dropped.clone())))
        .unwrap();
    let old_ui = native.ui.observe_retirement();
    let request = Request::new(&node, 83);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    let retired = node.retire_closed_add_native_data().unwrap();
    assert_eq!(dropped.load(SeqCst), 0);
    drop(retired);
    assert!(old_ui.is_drained());
    // The observer itself also carries a paid counter. After its actual wait
    // finishes and it drops, the retained closed SDK graph pins no old UI fee.
    drop(old_ui);
    assert_eq!(dropped.load(SeqCst), 1);
    assert!(node.completed_add_data().is_ok());
    assert!(node.pending.as_ref().unwrap().sdk_cut.is_none());
    assert!(node.pending.as_ref().unwrap().header_cut.is_none());
    assert!(node.pending.as_ref().unwrap().welcome_cut.is_none());
}

#[test]
fn fresh_same_source_authorization_resumes_only_unentered_keys_after_closed_sdk_retirement() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec04);
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let request = Request::new(&node, 84);
    let count = Arc::new(AtomicUsize::new(0));
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    let snapshot = Zeroizing::new(
        node.completed_add_data()
            .unwrap()
            .secret_snapshot_data()
            .to_vec(),
    );
    let rng = node.node._parts.body._rng.get_word_pos();
    let stale_header_cut = native.cut(NativeChatKeyPurpose::GoverningHeader);
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    // The actual old-cut invocation returns local Unavailable BEFORE the key.
    assert!(node.sign_admission_native().is_err());
    assert!(node.sign_admission_native().is_err());
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .header_native
            .attempts_for_test(),
        1
    );
    drop(node.retire_closed_add_native_data().unwrap());
    assert!(node
        .prepare_add_native_stage_data(V2FounderAddNativeStage::GoverningHeader, stale_header_cut)
        .is_err());
    let foreign = Native::new();
    assert!(node
        .prepare_add_native_stage_data(
            V2FounderAddNativeStage::GoverningHeader,
            foreign.cut(NativeChatKeyPurpose::GoverningHeader)
        )
        .is_err());
    assert!(node
        .prepare_add_native_stage_data(
            V2FounderAddNativeStage::GoverningHeader,
            native.cut(NativeChatKeyPurpose::JoinWelcome)
        )
        .is_err());
    let mut spent = native.cut(NativeChatKeyPurpose::GoverningHeader);
    let mut outer_ends = NativeChatKeyEnds::new();
    spent
        .enter(
            &native.source.observe(),
            NativeChatKeyPurpose::GoverningHeader,
            &mut outer_ends,
        )
        .unwrap();
    assert!(node
        .prepare_add_native_stage_data(V2FounderAddNativeStage::GoverningHeader, spent)
        .is_err());
    drop(outer_ends);
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::GoverningHeader,
        native.cut(NativeChatKeyPurpose::GoverningHeader),
    )
    .unwrap();
    node.sign_admission_native().unwrap();
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .header_native
            .attempts_for_test(),
        2
    );
    node.materialize_admission_data().unwrap();
    node.prepare_welcome_data().unwrap();
    drop(node.retire_closed_add_native_data().unwrap());
    native.generation.store(9, SeqCst);
    native.reauthenticate();
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::Welcome,
        native.cut(NativeChatKeyPurpose::JoinWelcome),
    )
    .unwrap();
    node.sign_welcome_native().unwrap();
    node.materialize_welcome_data().unwrap();
    drop(node.retire_closed_add_native_data().unwrap());
    assert!(node.signed_welcome_data().unwrap().verify());
    assert_eq!(node.node._parts.body._rng.get_word_pos(), rng);
    assert!(node.completed_add_data().unwrap().secret_snapshot_data() == snapshot.as_slice());
    assert!(node
        .prepare_add_native_stage_data(
            V2FounderAddNativeStage::Sdk,
            native.cut(NativeChatKeyPurpose::AddSdk)
        )
        .is_err());
    assert!(node
        .prepare_add_native_stage_data(
            V2FounderAddNativeStage::GoverningHeader,
            native.cut(NativeChatKeyPurpose::GoverningHeader)
        )
        .is_err());
}

#[test]
fn local_native_sdk_refusal_can_reauthorize_before_constructor_without_resetting_attempts() {
    let clock = ManualClock::new(NOW);
    let (original, _reservation, _opened, _old_fee) =
        prepared_node_with_clock(0xec05, Box::new(clock.clone()));
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let request = Request::new(&node, 85);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    assert!(node.execute_add_data_native(request.data()).is_err());
    let p = node.pending.as_ref().unwrap();
    assert!(p.phase == Phase::Prepared && p.cell.is_none());
    assert_eq!(p.admitted_at_ms, None);
    assert_eq!(p.sdk_native.attempts_for_test(), 1);
    drop(node.retire_unstarted_add_native_data().unwrap());
    clock.advance_ms(333);
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::Sdk,
        native.cut(NativeChatKeyPurpose::AddSdk),
    )
    .unwrap();
    node.execute_add_data_native(request.data()).unwrap();
    let p = node.pending.as_ref().unwrap();
    assert_eq!(p.sdk_native.attempts_for_test(), 2);
    assert_eq!(p.admitted_at_ms, Some(NOW + 333));
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    drop(node.retire_closed_add_native_data().unwrap());
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::GoverningHeader,
        native.cut(NativeChatKeyPurpose::GoverningHeader),
    )
    .unwrap();
    node.sign_admission_native().unwrap();
    node.materialize_admission_data().unwrap();
    node.prepare_welcome_data().unwrap();
    drop(node.retire_closed_add_native_data().unwrap());
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::Welcome,
        native.cut(NativeChatKeyPurpose::JoinWelcome),
    )
    .unwrap();
    node.sign_welcome_native().unwrap();
    node.materialize_welcome_data().unwrap();
    assert!(node.signed_welcome_data().unwrap().verify());
}

#[test]
fn panic_during_native_admission_cannot_be_laundered_into_a_local_refusal_witness() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec06);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 86);
    let count = Arc::new(AtomicUsize::new(0));
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    hook(TestCut::NativeAdmissionAttempting, || {
        panic!("native admission did not return")
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.sign_admission_native()
    }))
    .is_err());
    assert!(node.sign_admission_native().is_err());
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .header_native
            .attempts_for_test(),
        1
    );
    drop(node.retire_closed_add_native_data().unwrap());
    assert!(node
        .prepare_add_native_stage_data(
            V2FounderAddNativeStage::GoverningHeader,
            native.cut(NativeChatKeyPurpose::GoverningHeader)
        )
        .is_err());
    assert!(node.sign_admission_native().is_err());
    assert!(node.pending.as_ref().unwrap().raw.is_none());
}

#[test]
fn actual_partial_native_busy_refusal_retires_its_ui_end_before_fresh_sdk_authorization() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec07);
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    // Real RT counter pressure on the original source, held by the old UI.
    // Each End is its own batch because one batch has one slot per purpose.
    let mut pressure = Vec::new();
    for _ in 0..32 {
        let mut ends = NativeChatKeyEnds::new();
        native
            .cut(NativeChatKeyPurpose::AddSdk)
            .enter(
                &native.source.observe(),
                NativeChatKeyPurpose::AddSdk,
                &mut ends,
            )
            .unwrap();
        pressure.push(ends);
    }
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    let request = Request::new(&node, 87);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    // The new UI counter admits, then the actual full source counter refuses.
    assert!(node.execute_add_data_native(request.data()).is_err());
    let p = node.pending.as_ref().unwrap();
    assert!(p.phase == Phase::Prepared && p.cell.is_none());
    assert!(p
        .sdk_cut
        .as_ref()
        .unwrap()
        .require_never_entered_data(&p.original, NativeChatKeyPurpose::AddSdk,)
        .is_err());
    assert!(!p
        .ends
        .holds_admitted_purpose_data(&p.original, NativeChatKeyPurpose::AddSdk));
    assert_eq!(p.sdk_native.attempts_for_test(), 1);
    drop(node.retire_unstarted_add_native_data().unwrap());
    drop(pressure);
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::Sdk,
        native.cut(NativeChatKeyPurpose::AddSdk),
    )
    .unwrap();
    node.execute_add_data_native(request.data()).unwrap();
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .sdk_native
            .attempts_for_test(),
        2
    );
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
}

#[test]
fn welcome_local_refusal_resumes_with_fresh_authorization_but_never_repeats_a_cached_key() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec08);
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let request = Request::new(&node, 88);
    let count = Arc::new(AtomicUsize::new(0));
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    node.sign_admission_native().unwrap();
    node.materialize_admission_data().unwrap();
    node.prepare_welcome_data().unwrap();
    native.generation.store(8, SeqCst);
    native.reauthenticate();
    assert!(node.sign_welcome_native().is_err());
    assert!(node.sign_welcome_native().is_err());
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .welcome_native
            .attempts_for_test(),
        1
    );
    drop(node.retire_closed_add_native_data().unwrap());
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::Welcome,
        native.cut(NativeChatKeyPurpose::JoinWelcome),
    )
    .unwrap();
    node.sign_welcome_native().unwrap();
    node.materialize_welcome_data().unwrap();
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .welcome_native
            .attempts_for_test(),
        2
    );
    let signed = node.signed_welcome_data().unwrap().encode();
    drop(node.retire_closed_add_native_data().unwrap());
    assert!(node
        .prepare_add_native_stage_data(
            V2FounderAddNativeStage::Welcome,
            native.cut(NativeChatKeyPurpose::JoinWelcome)
        )
        .is_err());
    node.sign_welcome_native().unwrap(); // cached DATA bypasses every key
    assert_eq!(node.signed_welcome_data().unwrap().encode(), signed);
}

#[test]
fn repeated_ui_generations_release_each_unused_cut_payment_without_dropping_closed_sdk_data() {
    struct UiPayment(Arc<AtomicUsize>);
    impl Drop for UiPayment {
        fn drop(&mut self) {
            self.0.fetch_add(1, SeqCst);
        }
    }
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xec09);
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let request = Request::new(&node, 89);
    let count = Arc::new(AtomicUsize::new(0));
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    begin(&mut node, &native, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    drop(node.retire_closed_add_native_data().unwrap());
    let dropped = Arc::new(AtomicUsize::new(0));
    let rng = node.node._parts.body._rng.get_word_pos();
    for turn in 0..12 {
        let generation = 8 + turn as u64;
        native.generation.store(generation, SeqCst);
        native.reauthenticate();
        native.ui = native
            ._binding
            .open_session(generation, Arc::new(UiPayment(dropped.clone())))
            .unwrap();
        node.prepare_add_native_stage_data(
            V2FounderAddNativeStage::GoverningHeader,
            native.cut(NativeChatKeyPurpose::GoverningHeader),
        )
        .unwrap();
        native.generation.store(generation + 1, SeqCst);
        native.reauthenticate();
        assert_eq!(dropped.load(SeqCst), turn);
        let retired = node.retire_closed_add_native_data().unwrap();
        assert_eq!(dropped.load(SeqCst), turn);
        drop(retired);
        assert_eq!(dropped.load(SeqCst), turn + 1);
        assert!(node.completed_add_data().is_ok());
    }
    assert_eq!(node.node._parts.body._rng.get_word_pos(), rng);
    node.prepare_add_native_stage_data(
        V2FounderAddNativeStage::GoverningHeader,
        native.cut(NativeChatKeyPurpose::GoverningHeader),
    )
    .unwrap();
    node.sign_admission_native().unwrap();
    assert_eq!(
        node.pending
            .as_ref()
            .unwrap()
            .header_native
            .attempts_for_test(),
        1
    );
}
