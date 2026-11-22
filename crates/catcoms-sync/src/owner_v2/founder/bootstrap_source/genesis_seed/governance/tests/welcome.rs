//! Actual same-operation bindings and separate original native purpose. The
//! test native atoms are NOT real AppState credentials or Root covering saves.
use super::*;

#[test]
fn v2_join_request_and_welcome_complete_with_actual_add() {
    complete_original_welcome(false);
}

#[test]
fn native_joiner_welcome_requires_the_original_device_source_and_sdk_cut() {
    complete_original_welcome(true);
}

fn complete_original_welcome(native_joiner: bool) {
    use crate::{
        DirectJoinLimits, NativeV2JoinKeyEntry, PreparedV2JoinRequest, RecoveredV2JoinedOutcome,
        RecoveredV2PendingJoinSource, SealedV2JoinedOutcome, SealedV2PendingJoinSource,
        V2AdmissionStepData, V2FirstAddProofData, V2FirstJoinDeliveryData, V2JoinRequestPacketData,
        V2JoinResponseData, V2JoinResponsePacketData,
    };
    use catcoms_mls::persist::{
        text_current_add::TextCurrentAddLimits, TextCurrentSdkGraphReservation,
        TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
    };
    use catcoms_replication::chat::{
        ActionDraft, ActionSpec, Body, ChatLedger, LedgerLimits, ProjectionGroupView, Realm,
    };
    use rand_core::SeedableRng;
    struct Paid(TextCurrentSdkRestoreLimits);
    impl TextCurrentSdkGraphReservation for Paid {
        fn limits(&self) -> TextCurrentSdkRestoreLimits {
            self.0
        }
    }
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
    let limits = DirectJoinLimits::new(RAW, RAW, 256 * 1024).unwrap();
    let joiner = native_joiner.then(Native::new);
    let anchor = &fixture.invite.spec_data().anchor;
    let mut prepared = if let Some(joiner) = &joiner {
        let source = joiner.source.observe();
        let mut device = joiner.cut(NativeChatKeyPurpose::JoinerDevice);
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        PreparedV2JoinRequest::prepare_native(
            anchor,
            &fixture.invite,
            [76; 32],
            now,
            limits,
            NativeV2JoinKeyEntry::new(&source, &mut device, &mut ends),
        )
        .unwrap()
    } else {
        PreparedV2JoinRequest::prepare(
            MlsDevice::generate().unwrap(),
            anchor,
            &fixture.invite,
            [76; 32],
            now,
            limits,
        )
        .unwrap()
    };
    let pending = crate::V2PendingJoinContextData {
        vault: [101; 32],
        root: [102; 32],
        original_pending: [103; 32],
        pin: [104; 32],
    };
    let vault_key = [105; 32];
    let mut journal_rng = rand_chacha::ChaCha20Rng::seed_from_u64(106);
    assert!(prepared
        .seal_joined_outcome(&vault_key, pending, &mut journal_rng)
        .is_err());
    let sealed_pending = prepared
        .seal_pending_source(&vault_key, pending, &mut journal_rng)
        .unwrap();
    let (nonce, cipher) = sealed_pending.encrypted_parts();
    let original_pending = SealedV2PendingJoinSource::from_encrypted_parts(*nonce, cipher).unwrap();
    let recovered_pending = RecoveredV2PendingJoinSource::open(
        &original_pending,
        &vault_key,
        pending,
        &fixture.invite,
        limits,
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
    let after_authenticator = *completed.after_authenticator_data();
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
    let joined_realm = Realm::new(&fixture.invite.spec_data().anchor.group, 42).unwrap();
    let joined_labels = admission.ordinary_labels(&joined_realm).unwrap();
    let joined_signer = admission.spec().subject;
    let genesis = node.genesis_data();
    let proof = V2FirstAddProofData::encode(
        genesis.profile,
        V2AdmissionStepData {
            admission: genesis.admission,
            header: genesis.header,
            image: genesis.state,
        },
        V2AdmissionStepData {
            admission,
            header,
            image: output.image_data(),
        },
    )
    .unwrap();
    let delivery = V2FirstJoinDeliveryData::encode(&response_wire, &proof).unwrap();
    assert!(prepared
        .validate_first_join_delivery_data(&delivery[..delivery.len() - 1])
        .is_err());
    prepared
        .validate_first_join_delivery_data(&delivery)
        .unwrap();
    assert!(prepared
        .seal_joined_outcome(&vault_key, pending, &mut journal_rng)
        .is_err());
    let mut candidate = Some(
        g.verifier
            .derive_successor(
                header,
                ReplayBody::Admission(admission),
                output.image_data(),
            )
            .unwrap(),
    );
    let header_id = header.id();
    let (fields, install) = node.prepare_install_data().unwrap();
    original_seed.require_same_source_fields(fields).unwrap();
    assert!(other_seed.require_same_source_fields(fields).is_err());
    assert_eq!(install.expected_data().header, header_id);
    assert!(install
        .finish_host_selected(&mut g.verifier, &mut None)
        .is_err());
    assert!(node.installed_add_data().is_none());
    assert_eq!(
        node.completed_add_data()
            .unwrap()
            .secret_snapshot_data()
            .as_ptr(),
        before_header
    );
    let (_, install) = node.prepare_install_data().unwrap();
    install
        .finish_host_selected(&mut g.verifier, &mut candidate)
        .unwrap();
    assert!(candidate.is_none());
    assert_eq!(
        node.installed_add_data().unwrap().authenticator,
        after_authenticator
    );
    assert!(node.completed_add_data().is_err());
    node.require_sdk_current_data().unwrap();
    assert!(node.prepare_install_data().is_err());
    let closed = native.ui.retire();
    assert!(!closed.is_drained());
    drop(
        node.retire_add_for_closed_ui(&[Some(&closed)])
            .unwrap()
            .unwrap(),
    );
    assert!(closed.is_drained());
    drop(node.take_retired_install_data().unwrap());
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
    let (_retire, signal) = tokio::sync::watch::channel(true);
    let canceled = catcoms_rt::RequestCancellation::new(signal, None);
    assert!(prepared
        .finish_first_join_delivery_cancellable(&delivery, &canceled)
        .is_err());
    let (_current, signal) = tokio::sync::watch::channel(false);
    let current = catcoms_rt::RequestCancellation::new(signal, None);
    let mut native_cut = joiner
        .as_ref()
        .map(|joiner| joiner.cut(NativeChatKeyPurpose::JoinerWelcomeSdk));
    let original_source = joiner.as_ref().map(|joiner| joiner.source.observe());
    let mut ends = catcoms_rt::NativeChatKeyEnds::new();
    let joined = if let (Some(source), Some(cut)) = (&original_source, &mut native_cut) {
        assert!(prepared.finish_welcome(response, &g.verifier).is_err());
        assert!(prepared
            .finish_first_join_delivery_cancellable(&delivery, &current)
            .is_err());
        let foreign = Native::new();
        for (owner, purpose) in [
            (&foreign, NativeChatKeyPurpose::JoinerWelcomeSdk),
            (joiner.as_ref().unwrap(), NativeChatKeyPurpose::JoinWelcome),
        ] {
            let original = owner.source.observe();
            let mut wrong = owner.cut(purpose);
            assert!(prepared
                .finish_native_first_join_delivery(
                    &delivery,
                    &current,
                    NativeV2JoinKeyEntry::new(&original, &mut wrong, &mut ends),
                )
                .is_err());
        }
        prepared
            .finish_native_first_join_delivery(
                &delivery,
                &current,
                NativeV2JoinKeyEntry::new(source, cut, &mut ends),
            )
            .unwrap()
    } else {
        prepared.finish_welcome(response, &g.verifier).unwrap()
    };
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
    assert_eq!(facts.admission_state_data(), &after_authenticator);
    assert_eq!(
        joined.policy_data().mode(),
        catcoms_mls::GroupMode::PeerToPeer
    );
    let original = joined as *const _;
    let retry = if let (Some(source), Some(cut)) = (&original_source, &mut native_cut) {
        prepared
            .finish_native_first_join_delivery(
                &delivery,
                &current,
                NativeV2JoinKeyEntry::new(source, cut, &mut ends),
            )
            .unwrap()
    } else {
        prepared.finish_welcome(response, &g.verifier).unwrap()
    };
    assert_eq!(original, retry as *const _);
    prepared.validate_retained_package_data().unwrap();
    assert_eq!(prepared.request_bytes_data(), &wire[..]);
    let sealed = prepared
        .seal_joined_outcome(&vault_key, pending, &mut journal_rng)
        .unwrap();
    let original = sealed.encrypted_parts().1.as_ptr();
    let (nonce, cipher) = sealed.encrypted_parts();
    let immutable_outcome = SealedV2JoinedOutcome::from_encrypted_parts(*nonce, cipher).unwrap();
    let governed = TextCurrentAddLimits::governed_text_v2();
    let graph_limits = TextCurrentSdkRestoreLimits {
        max_typed_bytes: governed.restore_typed_bytes,
        max_work: governed.restore_work,
    };
    let restore = |outcome: &SealedV2JoinedOutcome, delivery: &[u8], fee| {
        RecoveredV2JoinedOutcome::open(
            outcome,
            &vault_key,
            &recovered_pending,
            delivery,
            TextCurrentSdkRestoreAdmission::new(Paid(fee)).unwrap(),
        )
    };
    let mut wrong_nonce = *nonce;
    wrong_nonce[0] ^= 1;
    let tampered = SealedV2JoinedOutcome::from_encrypted_parts(wrong_nonce, cipher).unwrap();
    assert!(restore(&tampered, &delivery, graph_limits).is_err());
    assert!(restore(
        &immutable_outcome,
        &delivery[..delivery.len() - 1],
        graph_limits
    )
    .is_err());
    assert!(restore(
        &immutable_outcome,
        &delivery,
        TextCurrentSdkRestoreLimits {
            max_work: 1,
            ..graph_limits
        },
    )
    .is_err());
    let mut recovered = restore(&immutable_outcome, &delivery, graph_limits).unwrap();
    assert_eq!(recovered.governing_head_data(), header_id);
    assert_eq!(recovered.epoch_data(), 1);
    use catcoms_replication::chat::{
        ProjectionGroup, V2HostSelectionExpectation, V2ProjectionOrigin,
    };
    let mut member_group = ProjectionGroup::new(&fixture.invite.spec_data().anchor.group).unwrap();
    let mut member_view = member_group.begin_view();
    assert!(member_view.try_ready());
    let origin = V2ProjectionOrigin::new(g.live.clone());
    assert!(recovered
        .prepare_initial_prefix(
            &delivery[..delivery.len() - 1],
            &member_view,
            origin.clone()
        )
        .is_err());
    let mut prefix = recovered
        .prepare_initial_prefix(&delivery, &member_view, origin.clone())
        .unwrap();
    assert!(recovered
        .prepare_initial_prefix(&delivery, &member_view, origin)
        .is_err());
    assert_eq!(
        prefix.own_admission_data().as_bytes(),
        &response.binding.spec_data().admission
    );
    assert!(prefix.selected_replay_data().is_none());
    assert!(prefix.pending_history_data().is_some());
    let state = g.verifier.replay_data().header().spec().state;
    let wrong = V2HostSelectionExpectation::from_trusted_host(
        catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes(
            fixture.invite.spec_data().anchor.header,
        ),
        state,
    );
    assert!(prefix.finish_host_selected(&wrong).is_err());
    let exact = V2HostSelectionExpectation::from_trusted_host(header_id, state);
    drop(prefix.finish_host_selected(&exact).unwrap());
    let replay = prefix.selected_replay_data().unwrap();
    assert_eq!(replay.header().id(), header_id);
    assert_eq!(replay.mls_epoch_data(), 1);

    let mut warm_group = ProjectionGroup::new(&fixture.invite.spec_data().anchor.group).unwrap();
    let mut warm_view = warm_group.begin_view();
    assert!(warm_view.try_ready());
    let warm_origin = V2ProjectionOrigin::new(g.live.clone());
    assert!(prepared
        .prepare_initial_prefix(
            &delivery[..delivery.len() - 1],
            &warm_view,
            warm_origin.clone()
        )
        .is_err());
    let (mut warm_prefix, selected) = prepared
        .prepare_initial_prefix(&delivery, &warm_view, warm_origin.clone())
        .unwrap();
    assert!(prepared
        .prepare_initial_prefix(&delivery, &warm_view, warm_origin)
        .is_err());
    assert_eq!(
        warm_prefix.own_admission_data(),
        prefix.own_admission_data()
    );
    assert!(warm_prefix.finish_host_selected(&wrong).is_err());
    let _warm_parent = warm_prefix.finish_host_selected(&selected).unwrap();
    assert_eq!(
        warm_prefix.selected_replay_data().unwrap().header().id(),
        header_id
    );
    let retry = prepared
        .seal_joined_outcome(&vault_key, pending, &mut journal_rng)
        .unwrap();
    assert_eq!(retry.encrypted_parts().1.as_ptr(), original);
    let other = crate::V2PendingJoinContextData {
        pin: [107; 32],
        ..pending
    };
    assert!(prepared
        .seal_joined_outcome(&vault_key, other, &mut journal_rng)
        .is_err());
    let hub = catcoms_rt::Hub::new();
    let peer = |id| hub.join(catcoms_rt::PeerId::from_u64(id));
    let rng = || rand_chacha::ChaCha20Rng::seed_from_u64(108);
    let clock = || -> Box<dyn catcoms_rt::Clock + Send> { Box::new(ManualClock::new(now)) };
    let mut warm_inputs = Some((peer(11), rng(), clock()));
    assert!(
        prepared
            .take_closed_warm_node(&prefix, &mut warm_inputs)
            .is_err(),
        "the independently restored prefix has the same bytes but no warm origin"
    );
    assert!(warm_inputs.is_some());
    let joined_node = prepared
        .take_closed_warm_node(&warm_prefix, &mut warm_inputs)
        .unwrap();
    assert!(warm_inputs.is_none());
    assert_eq!(
        joined_node.selected_join_data().unwrap(),
        (header_id, prefix.own_admission_data(), 1)
    );
    assert!(!joined_node.was_recovered_data());
    let mut unused = Some((peer(13), rng(), clock()));
    assert!(prepared
        .take_closed_warm_node(&warm_prefix, &mut unused)
        .is_err());
    assert!(unused.is_some());
    let mut cold_inputs = Some((peer(14), rng(), clock()));
    assert!(recovered
        .take_closed_recovered_node(&warm_prefix, &mut cold_inputs)
        .is_err());
    assert!(cold_inputs.is_some());
    let cold_node = recovered
        .take_closed_recovered_node(&prefix, &mut cold_inputs)
        .unwrap();
    assert!(cold_inputs.is_none());
    assert!(cold_node.was_recovered_data());
    assert_eq!(
        cold_node.selected_join_data().unwrap(),
        joined_node.selected_join_data().unwrap()
    );
    assert!(recovered
        .take_closed_recovered_node(&prefix, &mut unused)
        .is_err());
    assert!(unused.is_some());

    let member_plan = |selected: &crate::PreparedV2JoinedPrefix, group: &ProjectionGroupView| {
        let mut ledger = ChatLedger::new(
            joined_realm.clone(),
            LedgerLimits {
                max_actions: 8,
                max_action_bytes: 4096,
            },
        )
        .unwrap();
        let mut view = ledger.begin_projection_view_in(group).unwrap();
        assert!(view.try_ready());
        let draft = ActionDraft::new(
            ActionSpec {
                realm: joined_realm.clone(),
                basis: joined_labels.basis,
                grant: joined_labels.grant,
                nonce: [109; 16],
                presented_ms: now,
                parents: vec![],
                body: Body::Create {
                    text: "Joined member draft".into(),
                    reply_to: None,
                },
            },
            joined_signer,
        )
        .unwrap();
        let plan = selected.prepare_member_plan(&draft, &[], &view).unwrap();
        (ledger, view, draft, plan)
    };
    let (_warm_ledger, _warm_channel, warm_draft, warm_plan) =
        member_plan(&warm_prefix, &warm_view);
    let (_cold_ledger, _cold_channel, cold_draft, cold_plan) = member_plan(&prefix, &member_view);
    assert_eq!(warm_draft.action_id_data(), cold_draft.action_id_data());
    assert_eq!(warm_plan.admission_id_data(), cold_plan.admission_id_data());
    assert!(joined_node.admit_member_save(&cold_plan).is_err());
    assert!(cold_node.admit_member_save(&warm_plan).is_err());
    let original = joined_node.admit_member_save(&warm_plan).unwrap();
    assert!(joined_node.admit_member_save(&warm_plan).is_err());
    drop(original);
    drop(cold_node.admit_member_save(&cold_plan).unwrap());
    drop(warm_view.revoker().close());
    assert!(joined_node.admit_member_save(&warm_plan).is_err());
    drop(cold_node.admit_member_save(&cold_plan).unwrap());
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
