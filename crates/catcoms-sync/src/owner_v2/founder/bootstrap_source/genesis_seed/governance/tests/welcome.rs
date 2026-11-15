//! Actual same-operation bindings and separate original native purpose. The
//! test native atoms are NOT real AppState credentials or Root covering saves.
use super::*;

#[test]
fn welcome_is_derived_only_from_same_request_actual_outputs_and_signed_add() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xeb01);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 71);
    let count = Arc::new(AtomicUsize::new(0));
    let d = node.genesis_data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    begin(&mut node, &native, &count, request.data());
    assert!(node.prepare_welcome_data().is_err());
    assert!(node.sign_welcome_native().is_err());
    node.execute_add_data_native(request.data()).unwrap();
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    assert!(node.prepare_welcome_data().is_err());
    node.sign_admission_native().unwrap();
    assert!(node.prepare_welcome_data().is_err());
    node.materialize_admission_data().unwrap();
    node.prepare_welcome_data().unwrap();
    let frozen = node.unsigned_welcome_data().unwrap().1.to_vec();
    node.sign_welcome_native().unwrap();
    node.change_welcome_materialization_spec_for_test();
    assert!(node.materialize_welcome_data().is_err());
    // Failure after raw64 does not lose the spent result or enter the key
    // again. Restore the exact frozen spec and materialize the SAME signature.
    node.sign_welcome_native().unwrap();
    node.change_welcome_materialization_spec_for_test();
    node.materialize_welcome_data().unwrap();
    assert_eq!(node.unsigned_welcome_data().unwrap().1, frozen);
    let result = node.completed_add_data().unwrap();
    let signed = node.signed_welcome_data().unwrap();
    let d = node.genesis_data();
    signed
        .check_links_data(
            &request.invite,
            &request.request,
            d.profile,
            node.unsigned_admission_data().unwrap().0,
            node.signed_admission_data().unwrap(),
        )
        .unwrap();
    signed
        .check_payloads_data(
            result.welcome_data(),
            result.sealed_routing_transfer_data().unwrap(),
        )
        .unwrap();
    assert!(signed
        .check_payloads_data(result.welcome_data(), b"different routing")
        .is_err());
    signed_request::require_joiner_routing(&node, &request);
}

#[test]
fn welcome_cut_cannot_come_from_a_different_original_native_source() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xeb02);
    let mut node = original.prepare_governance_data().unwrap();
    let a = Native::new();
    let b = Native::new();
    let request = Request::new(&node, 72);
    let count = Arc::new(AtomicUsize::new(0));
    for (source, purpose) in [
        (&b, NativeChatKeyPurpose::JoinWelcome),
        (&a, NativeChatKeyPurpose::GoverningHeader),
    ] {
        let refused = node
            .prepare_add_data(
                request.data(),
                a.source.observe(),
                admission(&count),
                a.cut(NativeChatKeyPurpose::AddSdk),
                a.cut(NativeChatKeyPurpose::GoverningHeader),
                source.cut(purpose),
            )
            .unwrap_err();
        assert!(node.pending.is_none());
        drop(refused);
        assert_eq!(count.load(SeqCst), 0);
    }
    begin(&mut node, &a, &count, request.data());
    node.execute_add_data_native(request.data()).unwrap();
}

#[test]
fn interrupted_welcome_entry_without_raw_is_terminal_but_keeps_actual_add_and_intent() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xeb03);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 73);
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
    let frozen = node.unsigned_welcome_data().unwrap().1.to_vec();
    hook(TestCut::WelcomeEntered, || {
        panic!("interrupted Welcome signer")
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.sign_welcome_native()
    }))
    .is_err());
    assert!(node.sign_welcome_native().is_err());
    assert!(node.materialize_welcome_data().is_err());
    assert_eq!(node.unsigned_welcome_data().unwrap().1, frozen);
    assert!(node.completed_add_data().is_ok());
    assert_eq!(count.load(SeqCst), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn cached_welcome_raw_survives_lost_reply_lock_and_materializes_without_reentering_key() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xeb04);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 74);
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
    let generation = native.generation.clone();
    hook(TestCut::WelcomeRawCached, move || {
        generation.store(8, SeqCst);
        panic!("lost Welcome key reply after caching raw");
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.sign_welcome_native()
    }))
    .is_err());
    hook(TestCut::WelcomeEntered, || {
        panic!("cached raw must not reenter")
    });
    node.sign_welcome_native().unwrap();
    node.materialize_welcome_data().unwrap();
    assert!(node.signed_welcome_data().unwrap().verify());
    let bytes = node.signed_welcome_data().unwrap().encode();
    node.prepare_welcome_data().unwrap();
    node.sign_welcome_native().unwrap();
    node.materialize_welcome_data().unwrap();
    assert_eq!(node.signed_welcome_data().unwrap().encode(), bytes);
    TEST_CUT.with(|slot| {
        slot.borrow_mut().take();
    });
}

#[test]
fn lock_before_welcome_cut_cannot_be_revived_by_unlock_or_fresh_native_cuts() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xeb05);
    let mut node = original.prepare_governance_data().unwrap();
    let mut native = Native::new();
    let request = Request::new(&node, 75);
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
    let refused = node
        .prepare_add_data(
            request.data(),
            native.source.observe(),
            admission(&count),
            native.cut(NativeChatKeyPurpose::AddSdk),
            native.cut(NativeChatKeyPurpose::GoverningHeader),
            native.cut(NativeChatKeyPurpose::JoinWelcome),
        )
        .unwrap_err();
    drop(refused);
    assert!(node.sign_welcome_native().is_err());
    assert!(node.materialize_welcome_data().is_err());
    assert!(node.completed_add_data().is_ok());
}
