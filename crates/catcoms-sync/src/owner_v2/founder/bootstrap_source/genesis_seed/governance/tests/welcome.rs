//! Actual same-operation bindings and separate original native purpose. The
//! test native atoms are NOT real AppState credentials or Root covering saves.
use super::*;

#[test]
fn v2_join_request_and_welcome_complete_with_actual_add() {
    use crate::{
        DirectJoinLimits, PreparedV2JoinRequest, V2JoinRequestPacketData, V2JoinResponseData,
        V2JoinResponsePacketData,
    };
    // MLS itself places a lifetime on the live KeyPackage. Use the same
    // current clock for the founder fixture, then keep it fixed through Add.
    let now = catcoms_rt::Clock::now_ms(&catcoms_rt::SystemClock);
    let (original, _reservation, _opened, _old_fee) =
        prepared_node_with_clock(0xeb10, Box::new(ManualClock::new(now)));
    let original_seed = original
        .borrow_genesis_seed_data()
        .observe_original_seed_data();
    let mut original = Some(original);
    let mut destination = None;
    UnboundV2GenesisNode::prepare_governance_into_data(&mut original, &mut destination).unwrap();
    assert!(original.is_none());
    let (other, _other_reservation, _other_opened, _other_fee) = prepared_node(0xeb11);
    let other_seed = other
        .borrow_genesis_seed_data()
        .observe_original_seed_data();
    let mut other = Some(other);
    assert!(
        UnboundV2GenesisNode::prepare_governance_into_data(&mut other, &mut destination).is_err()
    );
    other_seed
        .require_same_seed_data(other.as_ref().unwrap().borrow_genesis_seed_data())
        .unwrap();
    let mut node = destination.take().unwrap();
    original_seed
        .require_same_seed_data(node.borrow_genesis_seed_data())
        .unwrap();
    let native = Native::new();
    let mut fixture = Request::new(&node, 75);
    fixture.change_invite(&node, |spec| spec.expires_at_ms = now + 60_000);
    let mut prepared = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        &fixture.invite.spec_data().anchor,
        &fixture.invite,
        [76; 32],
        now,
        DirectJoinLimits::new(RAW, RAW, 256 * 1024).unwrap(),
    )
    .unwrap();
    let wire = prepared.request_bytes_data().to_vec();
    let packet = V2JoinRequestPacketData::decode(&wire).unwrap();
    let request = packet.founder_add_data();
    let count = Arc::new(AtomicUsize::new(0));
    let d = node.genesis_data();
    let mut g = governance(d.profile, d.admission, d.header, d.state);
    begin(&mut node, &native, &count, request);
    node.execute_add_data_native(request).unwrap();
    assert!(node.outcome_data().is_err());
    node.prepare_admission_data(g.verifier.replay_data())
        .unwrap();
    let outcome = node.outcome_data().unwrap();
    outcome.require_request_data(request).unwrap();
    let other_request = Request::new(&node, 95);
    assert!(outcome.require_request_data(other_request.data()).is_err());
    let before_header = outcome.result_data().secret_snapshot_data().as_ptr();
    let unsigned_intent = outcome.unsigned_data().intent_commitment_data();
    assert_eq!(
        outcome.admission_data().spec().subject,
        request.request.spec_data().joiner
    );
    node.sign_admission_native().unwrap();
    node.materialize_admission_data().unwrap();
    node.prepare_welcome_data().unwrap();
    assert!(node.raw_welcome_signature_data().is_err());
    assert!(node.join_response_data().is_err());
    node.sign_welcome_native().unwrap();
    let raw = *node.raw_welcome_signature_data().unwrap();
    assert!(node.join_response_data().is_err());
    node.materialize_welcome_data().unwrap();
    let completed = node.completed_add_data().unwrap();
    let saved_outcome = node.outcome_data().unwrap();
    saved_outcome
        .result_data()
        .require_same_cell_data(&completed)
        .unwrap();
    assert_eq!(
        saved_outcome.result_data().secret_snapshot_data().as_ptr(),
        before_header
    );
    assert_eq!(
        saved_outcome.unsigned_data().intent_commitment_data(),
        unsigned_intent
    );
    let response = node.join_response_data().unwrap();
    assert!(response.binding.encode().ends_with(&raw));
    let response_wire = response.encode_packet_data().unwrap();
    assert!(V2JoinResponsePacketData::decode(&response_wire[..response_wire.len() - 1]).is_err());
    let response_packet = V2JoinResponsePacketData::decode(&response_wire).unwrap();
    let response = response_packet.response_data();
    assert!(prepared.finish_welcome(response, &g.verifier).is_err());

    // Only the genuine resulting Add advances the fixture's verified prefix.
    let header = node.signed_admission_data().unwrap();
    let (admission, output) = node.unsigned_admission_data().unwrap();
    let mut candidate = Some(
        g.verifier
            .derive_successor(
                header,
                ReplayBody::Admission(admission),
                output.image_data(),
            )
            .unwrap(),
    );
    let selection = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    drop(
        g.verifier
            .advance_host_selected(&selection, &mut candidate)
            .unwrap(),
    );
    let mut other_routing = response.sealed_routing.to_vec();
    *other_routing.last_mut().unwrap() ^= 1;
    assert!(prepared
        .finish_welcome(
            V2JoinResponseData {
                sealed_routing: &other_routing,
                ..response
            },
            &g.verifier,
        )
        .is_err());
    let joined = prepared.finish_welcome(response, &g.verifier).unwrap();
    let facts = joined.facts_data();
    assert_eq!(
        facts.key_package_data(),
        &request.request.spec_data().key_package_reference
    );
    assert_eq!(
        facts.credential_data(),
        &request.request.spec_data().credential
    );
    assert_eq!(facts.join_epoch_data(), 1);
    assert_eq!(
        facts.admission_state_data(),
        completed.after_authenticator_data()
    );
    assert_eq!(
        joined.policy_data().mode(),
        catcoms_mls::GroupMode::PeerToPeer
    );
    let original = joined as *const _;
    let retry = prepared.finish_welcome(response, &g.verifier).unwrap();
    assert_eq!(original, retry as *const _);
    prepared.validate_retained_package_data().unwrap();
    assert_eq!(prepared.request_bytes_data(), &wire[..]);
}

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
    assert!(node.raw_welcome_signature_data().is_err());
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
    assert!(node.raw_welcome_signature_data().is_err());
    assert!(node.join_response_data().is_err());
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
    let raw = *node.raw_welcome_signature_data().unwrap();
    assert!(node.join_response_data().is_err());
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
    assert!(bytes.ends_with(&raw));
    assert_eq!(node.raw_welcome_signature_data().unwrap(), &raw);
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
