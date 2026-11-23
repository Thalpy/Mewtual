//! Actual same-operation bindings and separate original native purpose. The
//! test native atoms are NOT real AppState credentials or Root covering saves.
use super::*;

#[tokio::test]
async fn v2_join_request_and_welcome_complete_with_actual_add() {
    complete_original_welcome(false).await;
}

#[tokio::test]
async fn native_joiner_welcome_requires_the_original_device_source_and_sdk_cut() {
    complete_original_welcome(true).await;
}

fn verify_later_governing_join_prefix(
    node: &Node,
    first: &[u8],
    anchor: &GenesisAnchorBindingData,
    replay: &catcoms_replication::chat::owner_state::transition::ReplayMath,
) -> Vec<u8> {
    use crate::{
        V2GoverningJoinPrefixData, V2GoverningJoinStepData, MAX_V2_GOVERNING_JOIN_PREFIX_BYTES,
    };
    use catcoms_replication::chat::owner_profile::{AdmissionOrigin, AdmissionSpec};
    let next_public_add =
        |replay: &catcoms_replication::chat::owner_state::transition::ReplayMath, marker: u8| {
            let subject = MlsDevice::generate()
                .unwrap()
                .public_key_bytes()
                .try_into()
                .unwrap();
            let before_epoch = replay.mls_epoch_data();
            let body = AdmissionBody::new(AdmissionSpec {
                group: anchor.group.clone(),
                profile: replay.profile().id(),
                subject,
                origin: AdmissionOrigin::Add {
                    commit: [marker; 32],
                    before_epoch,
                    after_epoch: before_epoch + 1,
                    key_package: [marker + 1; 32],
                    credential: [marker + 2; 32],
                },
            })
            .unwrap();
            let output = replay.prepare_admission_data(&body).unwrap();
            let header =
                SignedOwnerHeader::sign_using(output.header_spec_data().clone(), |bytes| {
                    node.node
                        .seed
                        .body
                        .device
                        .sign_retained_ed25519_bounded(
                            &anchor.founder,
                            bytes,
                            catcoms_replication::chat::owner_header::MAX_HEADER_BYTES,
                        )
                        .map_err(|_| catcoms_replication::chat::ChatError::Dependency)
                })
                .unwrap();
            (body, header, output)
        };
    let (second, second_header, second_output) = next_public_add(replay, 110);
    let second_selected = replay
        .derive_successor(
            &second_header,
            ReplayBody::Admission(&second),
            second_output.image_data(),
        )
        .unwrap()
        .next;
    let (third, third_header, third_output) = next_public_add(&second_selected, 120);
    let second_step = || V2GoverningJoinStepData {
        header: &second_header,
        image: second_output.image_data(),
        body: ReplayBody::Admission(&second),
    };
    let third_step = || V2GoverningJoinStepData {
        header: &third_header,
        image: third_output.image_data(),
        body: ReplayBody::Admission(&third),
    };
    let encoded = V2GoverningJoinPrefixData::encode(first, &[second_step(), third_step()]).unwrap();
    let proof = V2GoverningJoinPrefixData::decode(&encoded).unwrap();
    let selected = proof
        .verify_selected_add(anchor, third_header.id(), third.admission_id())
        .unwrap();
    assert_eq!(selected.mls_epoch_data(), 3);
    assert_eq!(selected.image(), third_output.image_data());
    assert!(proof
        .verify_selected_add(anchor, second_header.id(), second.admission_id())
        .is_err());
    assert!(proof
        .verify_selected_add(anchor, third_header.id(), second.admission_id())
        .is_err());
    let mut other_anchor = anchor.clone();
    other_anchor.state[0] ^= 1;
    assert!(proof
        .verify_selected_add(&other_anchor, third_header.id(), third.admission_id())
        .is_err());
    let skipped = V2GoverningJoinPrefixData::encode(first, &[third_step()]).unwrap();
    assert!(V2GoverningJoinPrefixData::decode(&skipped)
        .unwrap()
        .verify_selected_add(anchor, third_header.id(), third.admission_id())
        .is_err());
    let reordered =
        V2GoverningJoinPrefixData::encode(first, &[third_step(), second_step()]).unwrap();
    assert!(V2GoverningJoinPrefixData::decode(&reordered)
        .unwrap()
        .verify_selected_add(anchor, second_header.id(), second.admission_id())
        .is_err());
    assert!(V2GoverningJoinPrefixData::decode(&encoded[..encoded.len() - 1]).is_err());
    assert!(
        V2GoverningJoinPrefixData::decode(&vec![0; MAX_V2_GOVERNING_JOIN_PREFIX_BYTES + 1])
            .is_err()
    );
    encoded
}

fn complete_second_governing_welcome<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
    node: &mut Node,
    original: &OpenedBootstrapSourceData,
    first: &[u8],
    verifier: &mut catcoms_replication::chat::V2GeneralPrefixVerifier,
    now: u64,
    joined: &crate::ClosedV2JoinedNode<T, R>,
    joined_prefix: &mut crate::PreparedV2JoinedPrefix,
    foreign_prefix: &mut crate::PreparedV2JoinedPrefix,
) {
    use crate::{
        DirectJoinLimits, PreparedV2JoinRequest, V2AdmissionStepData, V2FirstAddProofData,
        V2FirstJoinDeliveryData, V2GoverningJoinDeliveryData, V2GoverningJoinPrefixData,
        V2GoverningJoinStepData, V2JoinRequestPacketData, MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES,
    };
    use catcoms_mls::persist::{
        TextCurrentSdkGraphReservation, TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
    };
    struct RestorePayment(TextCurrentSdkRestoreLimits);
    impl TextCurrentSdkGraphReservation for RestorePayment {
        fn limits(&self) -> TextCurrentSdkRestoreLimits {
            self.0
        }
    }
    let replay = verifier.replay_data();
    let used = node.installed_invites[0].unwrap();
    let mut reused = Request::new(node, used[0]);
    assert_eq!(reused.invite.spec_data().nonce, used);
    reused.change_invite(node, |spec| spec.expires_at_ms = now + 90_000);
    let limits = TextCurrentAddLimits::governed_text_v2();
    let repeat = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        &reused.invite.spec_data().anchor,
        &reused.invite,
        [94; 32],
        now,
        DirectJoinLimits::new(RAW, RAW, 256 * 1024).unwrap(),
    )
    .unwrap();
    let packet = V2JoinRequestPacketData::decode(repeat.request_bytes_data()).unwrap();
    let request = packet.founder_add_data();
    node.node.check_add_package(request, limits).unwrap();
    assert!(node
        .preflight_subsequent_add_package_data(replay, request, limits)
        .is_err());
    let refused_source = Native::new();
    let refused_payment = Arc::new(AtomicUsize::new(0));
    assert!(node
        .prepare_add_data(
            request,
            refused_source.source.observe(),
            super::admission(&refused_payment),
            refused_source.cut(NativeChatKeyPurpose::AddSdk),
            refused_source.cut(NativeChatKeyPurpose::GoverningHeader),
            refused_source.cut(NativeChatKeyPurpose::JoinWelcome),
        )
        .is_err());
    assert!(node.pending.is_none());
    assert_eq!(refused_payment.load(SeqCst), 0);
    let mut fixture = Request::new(node, 96);
    fixture.change_invite(node, |spec| spec.expires_at_ms = now + 60_000);
    let anchor = &fixture.invite.spec_data().anchor;
    let mut joiner = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        anchor,
        &fixture.invite,
        [97; 32],
        now,
        DirectJoinLimits::new(RAW, RAW, 256 * 1024).unwrap(),
    )
    .unwrap();
    let wire = joiner.request_bytes_data().to_vec();
    let request = V2JoinRequestPacketData::decode(&wire).unwrap();
    let request = request.founder_add_data();
    let genesis = node.genesis_data();
    let stale = catcoms_replication::chat::owner_state::transition::ReplayMath::from_genesis(
        genesis.profile,
        genesis.admission,
        genesis.header,
        genesis.state,
    )
    .unwrap();
    assert!(node
        .preflight_subsequent_add_package_data(&stale, request, limits)
        .is_err());
    node.preflight_subsequent_add_package_data(replay, request, limits)
        .unwrap();
    let native = Native::new();
    let payment = Arc::new(AtomicUsize::new(0));
    begin(node, &native, &payment, request);
    node.execute_add_data_native(request).unwrap();
    node.prepare_admission_data(replay).unwrap();
    node.sign_admission_native().unwrap();
    node.materialize_admission_data().unwrap();
    node.prepare_welcome_data().unwrap();
    node.sign_welcome_native().unwrap();
    node.materialize_welcome_data().unwrap();
    let header = node.signed_admission_data().unwrap();
    let (admission, output) = node.unsigned_admission_data().unwrap();
    let encoded = V2GoverningJoinPrefixData::encode(
        first,
        &[V2GoverningJoinStepData {
            header,
            image: output.image_data(),
            body: ReplayBody::Admission(admission),
        }],
    )
    .unwrap();
    let result = node.completed_add_data().unwrap();
    let step = || V2AdmissionStepData {
        admission,
        header,
        image: output.image_data(),
    };
    let commit = result.commit_data();
    let previously_selected = joined.selected_join_data().unwrap();
    assert!(joined
        .prepare_member_add_commit(foreign_prefix, step(), commit)
        .is_err());
    assert!(joined
        .prepare_member_add_commit(joined_prefix, step(), &commit[..commit.len() - 1])
        .is_err());
    let selected = joined
        .prepare_member_add_commit(joined_prefix, step(), commit)
        .unwrap();
    assert_eq!(selected.header_data().id(), header.id());
    assert_eq!(selected.image_data(), output.image_data());
    drop(selected);
    let member_payment = Arc::new(AtomicUsize::new(0));
    let member_candidate = joined
        .prepare_member_add_candidate(
            joined_prefix,
            step(),
            commit,
            super::admission(&member_payment),
        )
        .unwrap();
    assert_eq!(member_candidate.header_data().id(), header.id());
    assert_eq!(
        member_candidate.selected_epoch_data(),
        (2, *result.after_authenticator_data())
    );
    assert!(!member_candidate.snapshot_data().is_empty());
    drop(member_candidate);
    assert_eq!(joined.selected_join_data().unwrap(), previously_selected);
    let response = node.join_response_data().unwrap();
    let only_first = V2FirstAddProofData::decode(first).unwrap();
    assert!(joiner.finish_first_welcome(response, &only_first).is_err());
    let packet = response.encode_packet_data().unwrap();
    let wire = V2GoverningJoinDeliveryData::encode(&packet, &encoded).unwrap();
    assert!(V2FirstJoinDeliveryData::decode(&wire).is_err());
    assert!(V2GoverningJoinDeliveryData::decode(&wire[..wire.len() - 1]).is_err());
    assert!(V2GoverningJoinDeliveryData::encode(
        &packet,
        &vec![0; MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES]
    )
    .is_err());
    let delivery = V2GoverningJoinDeliveryData::decode(&wire).unwrap();
    assert_eq!(delivery.governing_proof_data().1, encoded);
    let expected = V2InstalledFounderAddData {
        epoch: result.after_epoch_data(),
        header: header.id(),
        state: header.spec().state,
        authenticator: *result.after_authenticator_data(),
    };
    let payment = || {
        TextCurrentSdkRestoreAdmission::new(RestorePayment(TextCurrentSdkRestoreLimits {
            max_typed_bytes: limits.restore_typed_bytes,
            max_work: limits.restore_work,
        }))
        .unwrap()
    };
    let proof = delivery.governing_proof_data().0;
    let reopened = PreparedV2FounderRecoveredAdd::restore_from_governing_prefix(
        original,
        expected,
        admission.admission_id(),
        proof,
        result.secret_snapshot_data(),
        payment(),
    )
    .unwrap();
    assert_eq!(reopened.fingerprint_data(), expected);
    let mut wrong = expected;
    wrong.authenticator[0] ^= 1;
    assert!(
        PreparedV2FounderRecoveredAdd::restore_from_governing_prefix(
            original,
            wrong,
            admission.admission_id(),
            proof,
            result.secret_snapshot_data(),
            payment(),
        )
        .is_err()
    );
    assert!(
        PreparedV2FounderRecoveredAdd::restore_from_governing_prefix(
            original,
            expected,
            node.genesis_data().admission.admission_id(),
            proof,
            result.secret_snapshot_data(),
            payment(),
        )
        .is_err()
    );
    let candidate = joiner.finish_governing_join_delivery(&delivery).unwrap();
    candidate.validate_initial_state_data().unwrap();
    assert_eq!(candidate.facts_data().join_epoch_data(), 2);
    assert_eq!(
        candidate.facts_data().admission_state_data(),
        result.after_authenticator_data()
    );
    assert_eq!(
        candidate.facts_data().key_package_data(),
        &request.request.spec_data().key_package_reference
    );
    let original = candidate as *const _;
    assert_eq!(
        joiner.finish_governing_join_delivery(&delivery).unwrap() as *const _,
        original
    );

    assert!(joined.selected_member_topic_data(joined_prefix, 42).is_ok());
    let mut sibling_body = admission.spec().clone();
    let AdmissionOrigin::Add {
        commit: different, ..
    } = &mut sibling_body.origin
    else {
        panic!("the founder prepared an Add");
    };
    different[0] ^= 1;
    let sibling_body = AdmissionBody::new(sibling_body).unwrap();
    let sibling = replay.prepare_admission_data(&sibling_body).unwrap();
    let signed_sibling =
        SignedOwnerHeader::sign_using(sibling.header_spec_data().clone(), |bytes| {
            node.node
                .seed
                .body
                .device
                .sign_retained_ed25519_bounded(
                    &anchor.founder,
                    bytes,
                    catcoms_replication::chat::owner_header::MAX_HEADER_BYTES,
                )
                .map_err(|_| catcoms_replication::chat::ChatError::Dependency)
        })
        .unwrap();
    let mismatched = V2GoverningJoinPrefixData::encode(
        first,
        &[V2GoverningJoinStepData {
            header: &signed_sibling,
            image: sibling.image_data(),
            body: ReplayBody::Admission(&sibling_body),
        }],
    )
    .unwrap();
    assert!(V2GoverningJoinDeliveryData::encode(&packet, &mismatched).is_err());
    assert!(joined
        .prepare_member_add_commit(
            joined_prefix,
            V2AdmissionStepData {
                admission: &sibling_body,
                header: &signed_sibling,
                image: sibling.image_data(),
            },
            &[]
        )
        .is_err());
    assert!(joined
        .selected_member_topic_data(joined_prefix, 42)
        .is_err());
    assert_eq!(joined.selected_join_data().unwrap(), previously_selected);

    let previous = node.installed_add_data().unwrap();
    let original_founder = node.member.as_ref().unwrap().clone();
    let (_, old_transport) = node.prepare_member_subscription(verifier, 42).unwrap();
    let expected_header = header.id();
    let expected_authenticator = *result.after_authenticator_data();
    let mut next = Some(
        verifier
            .derive_successor(
                header,
                ReplayBody::Admission(admission),
                output.image_data(),
            )
            .unwrap(),
    );
    let (_, prepared) = node.prepare_install_data().unwrap();
    assert!(prepared
        .check_host_selection_data(
            foreign_prefix.selected_member_verifier().unwrap(),
            next.as_ref().unwrap()
        )
        .is_err());
    assert!(prepared.finish_host_selected(verifier, &mut None).is_err());
    assert_eq!(node.installed_add_data().unwrap(), previous);
    let (_, prepared) = node.prepare_install_data().unwrap();
    prepared
        .check_host_selection_data(verifier, next.as_ref().unwrap())
        .unwrap();
    prepared.finish_host_selected(verifier, &mut next).unwrap();
    assert!(next.is_none());
    drop(node.take_retired_install_data().unwrap());
    let installed = node.installed_add_data().unwrap();
    assert_eq!(installed.epoch, 2);
    assert_eq!(installed.header, expected_header);
    assert_eq!(installed.authenticator, expected_authenticator);
    assert_eq!(
        node.installed_invites[1],
        Some(fixture.invite.spec_data().nonce)
    );
    assert!(node
        .member
        .as_ref()
        .unwrap()
        .same_original(&original_founder));
    assert_eq!(verifier.replay_data().slots().len(), 3);
    assert!(!old_transport.is_current_cached());
    assert!(node.prepare_member_subscription(verifier, 42).is_ok());
}

async fn complete_original_welcome(native_joiner: bool) {
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
        ActionDraft, ActionSpec, ActionState, Body, ChatLedger, GuardedProjection, LedgerLimits,
        ProjectionCertainty, ProjectionGroupView, Realm,
    };
    use catcoms_rt::{
        MeshTransport, PublishOnceError, PublishSubmission, RequestCancellation, TransportEvent,
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
    let (original, _reservation, opened, _old_fee) =
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
    assert!(node.prepare_member_subscription(&g.verifier, 42).is_err());
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
    if !native_joiner {
        let later =
            verify_later_governing_join_prefix(&node, &proof, anchor, g.verifier.replay_data());
        let later = crate::V2GoverningJoinPrefixData::decode(&later).unwrap();
        assert!(prepared.finish_governing_welcome(response, &later).is_err());
    }
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
    drop(warm_prefix.finish_host_selected(&selected).unwrap());
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
    let mut joined_node = prepared
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
    let mut cold_node = recovered
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

    let member_plan = |selected: &crate::PreparedV2JoinedPrefix,
                       group: &ProjectionGroupView,
                       (presented_ms, nonce)| {
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
                nonce,
                presented_ms,
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
    let (mut warm_ledger, warm_channel, warm_draft, warm_plan) =
        member_plan(&warm_prefix, &warm_view, (now, [109; 16]));
    let (mut cold_ledger, cold_channel, cold_draft, cold_plan) =
        member_plan(&prefix, &member_view, (now, [109; 16]));
    assert_eq!(warm_draft.action_id_data(), cold_draft.action_id_data());
    assert_eq!(warm_plan.admission_id_data(), cold_plan.admission_id_data());
    assert!(joined_node.admit_member_save(&cold_plan).is_err());
    assert!(cold_node.admit_member_save(&warm_plan).is_err());
    let original = joined_node.admit_member_save(&warm_plan).unwrap();
    assert!(joined_node.admit_member_save(&warm_plan).is_err());
    drop(original);
    drop(cold_node.admit_member_save(&cold_plan).unwrap());
    let mut revoked_submission = None;
    let mut founder_after_drop = None;
    if let Some(joiner) = &joiner {
        let mut inputs = crate::owner_v2::V2MemberDraftInputs::new();
        let drawn = joined_node.draw_member_draft_inputs(&mut inputs).unwrap();
        assert_eq!(
            joined_node.draw_member_draft_inputs(&mut inputs).unwrap(),
            drawn
        );
        let (_ledger, _view, draft, mut plan) = member_plan(&warm_prefix, &warm_view, drawn);
        let original = joiner.source.observe();
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        let foreign_source = Native::new();
        let mut foreign = foreign_source.cut(NativeChatKeyPurpose::MemberAction);
        assert!(joined_node
            .sign_member_action_native(
                &inputs,
                &draft,
                &mut plan,
                &foreign_source.source.observe(),
                &mut foreign,
                &mut ends
            )
            .is_err());
        let mut wrong = joiner.cut(NativeChatKeyPurpose::JoinerWelcomeSdk);
        assert!(joined_node
            .sign_member_action_native(&inputs, &draft, &mut plan, &original, &mut wrong, &mut ends)
            .is_err());
        let mut cut = joiner.cut(NativeChatKeyPurpose::MemberAction);
        assert!(cold_node
            .sign_member_action_native(&inputs, &draft, &mut plan, &original, &mut cut, &mut ends)
            .is_err());
        assert!(!ends.holds_admitted_purpose_data(&original, NativeChatKeyPurpose::MemberAction));
        drop(
            joined_node
                .sign_member_action_native(
                    &inputs, &draft, &mut plan, &original, &mut cut, &mut ends,
                )
                .unwrap(),
        );
        assert!(ends.holds_admitted_purpose_data(&original, NativeChatKeyPurpose::MemberAction));
        let action = plan
            .complete_cached_raw64_data(&plan.observe_original(), &draft)
            .unwrap();
        assert!(action.verify());
        assert_eq!(action.id(), draft.action_id_data());
        let admission = warm_prefix.own_admission_data();
        let raw = action.encode();
        assert!(joined_node
            .seal_selected_member_envelope(&prefix, 42, admission, &raw)
            .is_err());
        assert!(joined_node
            .prepare_member_submission(&prefix, 42, admission, &raw)
            .is_err());
        let wire = joined_node
            .seal_selected_member_envelope(&warm_prefix, 42, admission, &raw)
            .unwrap();
        let topic = cold_node.selected_member_topic_data(&prefix, 42).unwrap();
        assert!(cold_node
            .open_selected_member_envelope(&warm_prefix, 42, &topic, &wire)
            .is_err());
        assert!(cold_node
            .open_selected_member_envelope(&prefix, 42, &catcoms_rt::Topic::new("foreign"), &wire)
            .is_err());
        assert_eq!(
            joined_node
                .selected_member_topic_data(&warm_prefix, 42)
                .unwrap(),
            topic
        );
        let opened = cold_node
            .open_selected_member_envelope(&prefix, 42, &topic, &wire)
            .unwrap();
        assert_eq!(opened.original_data(), raw);
        let verifier = prefix.selected_member_verifier().unwrap();
        assert!(opened
            .provisional_authority(verifier, &[], &cold_channel)
            .is_ok());
        assert!(opened
            .provisional_authority(verifier, &[], &warm_channel)
            .is_err());
        assert!(opened
            .project_provisional(verifier, &mut warm_ledger, &cold_channel, &[], 8)
            .is_err());
        assert!(opened
            .project_provisional(
                warm_prefix.selected_member_verifier().unwrap(),
                &mut warm_ledger,
                &warm_channel,
                &[],
                8
            )
            .is_err());
        assert!(warm_ledger.action(action.id()).is_none());
        assert!(matches!(
            opened
                .project_provisional(verifier, &mut cold_ledger, &cold_channel, &[], 0)
                .unwrap(),
            ActionState::Pending(_)
        ));
        assert!(!matches!(
            cold_ledger
                .guarded_projection(action.id(), 8)
                .unwrap()
                .with_publication(Clone::clone)
                .unwrap(),
            GuardedProjection::Visible { .. }
        ));
        assert_eq!(
            opened
                .project_provisional(verifier, &mut cold_ledger, &cold_channel, &[], 8)
                .unwrap(),
            ActionState::Valid
        );
        assert!(
            matches!(cold_ledger.guarded_projection(action.id(), 8).unwrap().with_publication(Clone::clone).unwrap(),
            GuardedProjection::Visible { text, certainty: ProjectionCertainty::Provisional, .. } if text == "Joined member draft")
        );
        let receiver = peer(18);
        receiver.subscribe(topic.clone()).await.unwrap();
        let (cancel, signal) = tokio::sync::watch::channel(true);
        let submission = joined_node
            .prepare_member_submission(&warm_prefix, 42, admission, &raw)
            .unwrap();
        assert!(matches!(
            submission
                .submit(RequestCancellation::new(signal, None))
                .await,
            Err(crate::SyncError::Publication(PublishOnceError::Revoked))
        ));
        drop(cancel);
        let (_cancel, signal) = tokio::sync::watch::channel(false);
        let caller = RequestCancellation::new(signal, None);
        let submission = joined_node
            .prepare_member_submission(&warm_prefix, 42, admission, &raw)
            .unwrap();
        assert!(submission.is_current_cached());
        assert_eq!(
            submission.submit(caller).await.unwrap(),
            PublishSubmission::Submitted
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.next_event())
            .await
            .unwrap()
            .unwrap();
        let TransportEvent::Gossip {
            topic: observed,
            from,
            data,
        } = event
        else {
            panic!("expected ordinary member gossip");
        };
        assert_eq!(observed, topic);
        assert_eq!(from, catcoms_rt::PeerId::from_u64(11));
        let received = cold_node
            .open_selected_member_envelope(&prefix, 42, &observed, &data)
            .unwrap();
        assert_eq!(received.original_data(), raw);
        assert_eq!(
            received
                .project_provisional(verifier, &mut cold_ledger, &cold_channel, &[], 8)
                .unwrap(),
            ActionState::Valid
        );
        assert_eq!(cold_ledger.action_ids().count(), 1);
        assert_eq!(cold_ledger.action(action.id()).unwrap().encode(), raw);

        // The installed founder and the joined device use the same selected
        // topic. Only the founder's original Core verifier may authorize its
        // own SDK transport, even if the joined verifier replayed equal bytes.
        let (founder_topic, founder_fence) =
            node.prepare_member_subscription(&g.verifier, 42).unwrap();
        assert_eq!(founder_topic, observed);
        assert!(founder_fence.is_current_cached());
        assert!(node.prepare_member_subscription(verifier, 42).is_err());
        assert!(node
            .open_selected_member_envelope(&g.verifier, 43, &observed, &data)
            .is_err());
        let founder_opened = node
            .open_selected_member_envelope(&g.verifier, 42, &observed, &data)
            .unwrap();
        assert!(founder_opened
            .provisional_authority(verifier, &[], &cold_channel)
            .is_err());
        assert_eq!(
            founder_opened
                .project_provisional(&g.verifier, &mut g._ledger, &g.view, &[], 8)
                .unwrap(),
            ActionState::Valid,
        );
        {
            let founder_keys = Native::new();
            let mut founder_inputs = crate::owner_v2::V2MemberDraftInputs::new();
            let (presented_ms, nonce) = node
                .draw_ordinary_draft_inputs(&mut founder_inputs)
                .unwrap();
            let labels = g
                .founder
                .admission_data()
                .ordinary_labels(&joined_realm)
                .unwrap();
            let draft = ActionDraft::new(
                ActionSpec {
                    realm: joined_realm.clone(),
                    basis: labels.basis,
                    grant: labels.grant,
                    nonce,
                    presented_ms,
                    parents: vec![],
                    body: Body::Create {
                        text: "Original founder transport".into(),
                        reply_to: None,
                    },
                },
                node.ordinary_member_signer_data().unwrap(),
            )
            .unwrap();
            let mut plan = g
                .verifier
                .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
                .unwrap();
            let signer = node.ordinary_member_signer_data().unwrap();
            let guard = plan.admit_signing(&draft, &signer).unwrap();
            let mut cut = founder_keys.cut(NativeChatKeyPurpose::MemberAction);
            let mut ends = NativeChatKeyEnds::new();
            drop(
                node.sign_ordinary_admitted_native(
                    &founder_inputs,
                    guard,
                    &founder_keys.source.observe(),
                    &mut cut,
                    &mut ends,
                )
                .unwrap(),
            );
            let founder = plan
                .complete_cached_raw64_data(&plan.observe_original(), &draft)
                .unwrap();
            let founder_admission = g.founder.admission_data().admission_id();
            let founder_raw = founder.encode();
            assert!(node
                .prepare_member_submission(&g.verifier, 42, admission, &founder_raw)
                .is_err());
            let submission = node
                .prepare_member_submission(&g.verifier, 42, founder_admission, &founder_raw)
                .unwrap();
            assert_eq!(submission.topic, observed);
            let opened = cold_node
                .open_selected_member_envelope(&prefix, 42, &submission.topic, &submission.wire)
                .unwrap();
            assert_eq!(opened.original_data(), founder_raw);
            assert_eq!(
                opened
                    .project_provisional(verifier, &mut cold_ledger, &cold_channel, &[], 8)
                    .unwrap(),
                ActionState::Valid
            );
            founder_after_drop = Some(submission);
        }

        let route = crate::owner_v2::V2MemberDependencyRoute {
            requester: cold_node.local_peer_data(),
            provider: joined_node.local_peer_data(),
        };
        let mut wire_rng = rand_chacha::ChaCha20Rng::seed_from_u64(9001);
        assert!(cold_node
            .prepare_member_dependency_request(&warm_prefix, 42, route, action.id(), &mut wire_rng)
            .is_err());
        let missing = cold_node
            .prepare_member_dependency_request(&prefix, 42, route, action.id(), &mut wire_rng)
            .unwrap();
        assert!(cold_node
            .open_member_dependency_request(&prefix, 42, route, missing.wire_data())
            .is_err());
        let dependency = joined_node
            .open_member_dependency_request(&warm_prefix, 42, route, missing.wire_data())
            .unwrap();
        assert_eq!(dependency.action_id_data(), action.id());
        let response = joined_node
            .seal_member_dependency_reply(
                &warm_prefix,
                42,
                route,
                &dependency,
                Some((admission, &raw)),
                &mut wire_rng,
            )
            .unwrap();
        let recovered = cold_node
            .open_member_dependency_reply(&prefix, 42, route, &missing, &response)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.original_data(), raw);
        assert!(recovered
            .project_from_ledger(
                warm_prefix.selected_member_verifier().unwrap(),
                &mut warm_ledger,
                &warm_channel,
                8
            )
            .is_err());
        assert!(warm_ledger.action(action.id()).is_none());
        revoked_submission = Some(
            joined_node
                .prepare_member_submission(&warm_prefix, 42, admission, &raw)
                .unwrap(),
        );
        drop(ends);
        let mut retry_ends = catcoms_rt::NativeChatKeyEnds::new();
        drop(
            joined_node
                .sign_member_action_native(
                    &inputs,
                    &draft,
                    &mut plan,
                    &original,
                    &mut cut,
                    &mut retry_ends,
                )
                .unwrap(),
        );
        assert!(
            !retry_ends.holds_admitted_purpose_data(&original, NativeChatKeyPurpose::MemberAction)
        );
        assert_eq!(
            plan.complete_cached_raw64_data(&plan.observe_original(), &draft)
                .unwrap()
                .encode(),
            action.encode()
        );
        let restarted = Native::new();
        let source = restarted.source.observe();
        let member = restarted.cut(NativeChatKeyPurpose::MemberAction);
        assert!(cold_node
            .bind_recovered_member_source(&warm_prefix, &source, &member)
            .is_err());
        let sdk = restarted.cut(NativeChatKeyPurpose::JoinerWelcomeSdk);
        assert!(cold_node
            .bind_recovered_member_source(&prefix, &source, &sdk)
            .is_err());
        cold_node
            .bind_recovered_member_source(&prefix, &source, &member)
            .unwrap();
        cold_node
            .bind_recovered_member_source(&prefix, &source, &member)
            .unwrap();
        let old_member = joiner.cut(NativeChatKeyPurpose::MemberAction);
        assert!(cold_node
            .bind_recovered_member_source(&prefix, &original, &old_member)
            .is_err());
    }
    if !native_joiner {
        complete_second_governing_welcome(
            &mut node,
            &opened,
            &proof,
            &mut g.verifier,
            now,
            &joined_node,
            &mut warm_prefix,
            &mut prefix,
        );
    }
    drop(warm_view.revoker().close());
    if let Some(submission) = revoked_submission {
        assert!(!submission.is_current_cached());
        let (_cancel, signal) = tokio::sync::watch::channel(false);
        assert!(matches!(
            submission
                .submit(RequestCancellation::new(signal, None))
                .await,
            Err(crate::SyncError::Publication(PublishOnceError::Revoked))
        ));
    }
    assert!(joined_node.admit_member_save(&warm_plan).is_err());
    drop(cold_node.admit_member_save(&cold_plan).unwrap());
    drop(node);
    if let Some(submission) = founder_after_drop {
        assert!(!submission.is_current_cached());
        let (_cancel, signal) = tokio::sync::watch::channel(false);
        assert!(matches!(
            submission
                .submit(RequestCancellation::new(signal, None))
                .await,
            Err(crate::SyncError::Publication(PublishOnceError::Revoked)),
        ));
    }
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
