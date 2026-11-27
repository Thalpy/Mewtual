//! Actual same-operation bindings and separate original native purpose. The
//! test native atoms are NOT real AppState credentials or Root covering saves.
use super::*;

#[tokio::test]
async fn v2_join_request_and_welcome_complete_with_actual_add() {
    complete_original_welcome(false).await;
}

#[test]
fn native_rejoin_request_uses_only_the_original_signer_and_a_fresh_package() {
    use crate::{
        DirectJoinLimits, NativeV2JoinKeyEntry, PreparedV2JoinRequest,
        RecoveredV2PendingJoinSource, SealedV2PendingJoinSource, V2JoinRequestPacketData,
    };
    use rand_core::SeedableRng;
    let (node, _reservation, _opened, _fee) = prepared_node(0x5e31);
    let mut origin = Some(node);
    let mut governed = None;
    UnboundV2GenesisNode::prepare_governance_into_data(&mut origin, &mut governed).unwrap();
    let node = governed.take().unwrap();
    let a1_invite = Request::new(&node, 75).invite;
    let a2_invite = Request::new(&node, 78).invite;
    let limits = DirectJoinLimits::new(RAW, RAW, 256 * 1024).unwrap();
    let mut a1 = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        &a1_invite.spec_data().anchor,
        &a1_invite,
        [76; 32],
        NOW,
        limits,
    )
    .unwrap();
    let a1_wire = a1.request_bytes_data().to_vec();
    let a1_packet = V2JoinRequestPacketData::decode(&a1_wire).unwrap();
    let a1_request = a1_packet.founder_add_data().request.spec_data();
    let context = crate::V2PendingJoinContextData {
        vault: [101; 32],
        root: [102; 32],
        original_pending: [103; 32],
        pin: [104; 32],
    };
    let vault_key = [105; 32];
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(106);
    let sealed = a1
        .seal_pending_source(&vault_key, context, &mut rng)
        .unwrap();
    let (nonce, ciphertext) = sealed.encrypted_parts();
    let saved = SealedV2PendingJoinSource::from_encrypted_parts(*nonce, ciphertext).unwrap();
    let previous_remove = (
        OwnerHeaderId::from_bytes([79; 32]),
        OwnerFactId::from_bytes([80; 32]),
        2,
    );
    let input = |invite, token| crate::NativeV2RejoinInputs {
        previous_invite: &a1_invite,
        previous_context: context,
        previous_request: &a1_wire,
        previous_signer: a1_request.joiner,
        previous_remove,
        invite,
        token,
        now_ms: NOW,
        limits,
    };
    let native = Native::new();
    let original = native.source.observe();
    let mut cut = native.cut(NativeChatKeyPurpose::JoinerDevice);
    let mut ends = catcoms_rt::NativeChatKeyEnds::new();
    assert!(saved
        .prepare_native_rejoin(
            &vault_key,
            input(&a1_invite, [79; 32]),
            NativeV2JoinKeyEntry::new(&original, &mut cut, &mut ends),
        )
        .is_err());
    assert!(saved
        .prepare_native_rejoin(
            &vault_key,
            input(&a2_invite, [76; 32]),
            NativeV2JoinKeyEntry::new(&original, &mut cut, &mut ends),
        )
        .is_err());
    let mut a2 = saved
        .prepare_native_rejoin(
            &vault_key,
            input(&a2_invite, [79; 32]),
            NativeV2JoinKeyEntry::new(&original, &mut cut, &mut ends),
        )
        .unwrap();
    a2.validate_retained_package_data().unwrap();
    let a2_packet = V2JoinRequestPacketData::decode(a2.request_bytes_data()).unwrap();
    let a2_request = a2_packet.founder_add_data().request.spec_data();
    assert_eq!(a2_request.joiner, a1_request.joiner);
    assert_ne!(
        a2_request.key_package_reference,
        a1_request.key_package_reference
    );
    assert_ne!(a2_request.request_token, a1_request.request_token);
    assert_eq!(saved.encrypted_parts().1, ciphertext);

    let a2_context = crate::V2PendingJoinContextData {
        original_pending: [107; 32],
        ..context
    };
    assert_eq!(a2.rejoin_remove_data(), Some(previous_remove));
    let sealed_rejoin = a2
        .seal_pending_source(&vault_key, a2_context, &mut rng)
        .unwrap();
    assert!(RecoveredV2PendingJoinSource::open(
        sealed_rejoin,
        &vault_key,
        context,
        &a2_invite,
        limits,
    )
    .is_err());
    let restarted = RecoveredV2PendingJoinSource::open(
        sealed_rejoin,
        &vault_key,
        a2_context,
        &a2_invite,
        limits,
    )
    .unwrap();
    assert_eq!(restarted.rejoin_remove_data(), Some(previous_remove));
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
    (joined, joined_prefix): (
        &crate::ClosedV2JoinedNode<T, R>,
        &mut crate::PreparedV2JoinedPrefix,
    ),
    (recovered, foreign_prefix): (
        &mut crate::ClosedV2JoinedNode<T, R>,
        &mut crate::PreparedV2JoinedPrefix,
    ),
) {
    use crate::{
        DirectJoinLimits, PreparedV2JoinRequest, RecoveredV2JoinedOutcome,
        RecoveredV2PendingJoinSource, V2AdmissionStepData, V2FirstAddProofData,
        V2FirstJoinDeliveryData, V2GoverningJoinDeliveryData, V2GoverningJoinPrefixData,
        V2GoverningJoinStepData, V2JoinDeliveryData, V2JoinRequestPacketData, V2MemberAddFrameData,
        V2MemberAddRequestData, V2MemberTransitionFrameData, V2PendingJoinContextData,
        MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES,
    };
    use catcoms_mls::persist::{
        TextCurrentSdkGraphReservation, TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
    };
    use catcoms_replication::chat::{V2OriginNegative, V2ProjectionOrigin};
    use rand_core::SeedableRng;
    #[derive(Debug)]
    struct Live;
    impl V2OriginNegative for Live {
        fn is_live(&self) -> bool {
            true
        }
    }
    struct RestorePayment(TextCurrentSdkRestoreLimits);
    impl TextCurrentSdkGraphReservation for RestorePayment {
        fn limits(&self) -> TextCurrentSdkRestoreLimits {
            self.0
        }
    }
    let replay = verifier.replay_data();
    let old_route = node.member_add_request_data(verifier).unwrap();
    let member_route = joined.member_add_request_data(joined_prefix).unwrap();
    let request_bytes = member_route.encode().unwrap();
    let request = V2MemberAddRequestData::decode(&request_bytes).unwrap();
    assert_eq!(request.parent_data(), (replay.header().id(), 1));
    assert!(request.matches_verifier_data(old_route.verifier_data()));
    let mut wrong = request_bytes;
    *wrong.last_mut().unwrap() ^= 1;
    let wrong = V2MemberAddRequestData::decode(&wrong).unwrap();
    assert!(!wrong.matches_verifier_data(old_route.verifier_data()));
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
    let issuer = Native::new();
    let invite_source = issuer.source.observe();
    let mut invite = None;
    let request = crate::owner_v2::V2FounderInviteRequestData {
        token: [96; 32],
        lifetime_ms: 60_000,
        public_routing_plan: fixture.invite.spec_data().public_routing_plan,
    };
    assert!(node
        .prepare_selected_invite_data(
            foreign_prefix.selected_member_verifier().unwrap(),
            request,
            &invite_source,
            &mut invite
        )
        .is_err());
    assert!(invite.is_none());
    node.prepare_selected_invite_data(verifier, request, &invite_source, &mut invite)
        .unwrap();
    assert!(node
        .prepare_selected_invite_data(
            verifier,
            crate::owner_v2::V2FounderInviteRequestData {
                token: [97; 32],
                ..request
            },
            &invite_source,
            &mut invite
        )
        .is_err());
    let invite = invite.as_mut().unwrap();
    node.preflight_selected_invite_signing(verifier, invite)
        .unwrap();
    let mut wrong = issuer.cut(NativeChatKeyPurpose::JoinWelcome);
    let mut ends = catcoms_rt::NativeChatKeyEnds::new();
    assert!(node
        .sign_selected_invite_native(verifier, invite, &mut wrong, &mut ends)
        .is_err());
    assert!(invite.raw_signature_data().is_err());
    let mut cut = issuer.cut(NativeChatKeyPurpose::JoinInvite);
    node.sign_selected_invite_native(verifier, invite, &mut cut, &mut ends)
        .unwrap();
    invite.materialize_data().unwrap();
    fixture.invite =
        SignedInviteBindingData::decode(&invite.signed_data().unwrap().encode()).unwrap();
    assert_ne!(fixture.invite.spec_data().nonce, used);
    assert_eq!(fixture.invite.spec_data().expires_at_ms, now + 60_000);
    let anchor = &fixture.invite.spec_data().anchor;
    let join_limits = DirectJoinLimits::new(RAW, RAW, 256 * 1024).unwrap();
    let mut joiner = PreparedV2JoinRequest::prepare(
        MlsDevice::generate().unwrap(),
        anchor,
        &fixture.invite,
        [97; 32],
        now,
        join_limits,
    )
    .unwrap();
    let keys = [111; 32];
    let context = V2PendingJoinContextData {
        vault: [112; 32],
        root: [113; 32],
        original_pending: [114; 32],
        pin: [115; 32],
    };
    let mut journal_rng = rand_chacha::ChaCha20Rng::seed_from_u64(303);
    let source = joiner
        .seal_pending_source(&keys, context, &mut journal_rng)
        .unwrap();
    let pending =
        RecoveredV2PendingJoinSource::open(source, &keys, context, &fixture.invite, join_limits)
            .unwrap();
    let (nonce, ciphertext) = source.encrypted_parts();
    let previous_source =
        crate::SealedV2PendingJoinSource::from_encrypted_parts(*nonce, ciphertext).unwrap();
    let previous_request = joiner.request_bytes_data().to_vec();
    let request = V2JoinRequestPacketData::decode(&previous_request).unwrap();
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
    let remove_admission = admission.id();
    let removed_id = admission.admission_id();
    let retained_later = (
        header.clone(),
        admission.clone(),
        output.image_data().to_vec(),
    );
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
    let original_frame = V2MemberAddFrameData::encode(step(), commit).unwrap();
    let truncated =
        V2MemberAddFrameData::decode(&original_frame[..original_frame.len() - 1]).unwrap();
    assert!(joined
        .prepare_member_add_frame(joined_prefix, &truncated, super::admission(&member_payment))
        .is_err());
    assert_eq!(member_payment.load(SeqCst), 0);
    let verified_frame = V2MemberAddFrameData::decode(&original_frame).unwrap();
    let warm_candidate = joined
        .prepare_member_add_frame(
            joined_prefix,
            &verified_frame,
            super::admission(&member_payment),
        )
        .unwrap();
    assert!(warm_candidate.matches_frame_data(&original_frame));
    let mut changed_frame = original_frame.clone();
    *changed_frame.last_mut().unwrap() ^= 1;
    assert!(!warm_candidate.matches_frame_data(&changed_frame));
    assert!(!warm_candidate.matches_frame_data(&original_frame[..original_frame.len() - 1]));
    assert_eq!(warm_candidate.header_data().id(), header.id());
    assert_eq!(
        warm_candidate.selected_epoch_data(),
        (2, *result.after_authenticator_data())
    );
    drop(warm_candidate);
    let member_candidate = recovered
        .prepare_member_add_candidate(
            foreign_prefix,
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
    assert_eq!(joined.selected_join_data().unwrap(), previously_selected);
    assert_eq!(recovered.selected_join_data().unwrap(), previously_selected);
    let original_member = foreign_prefix
        .selected_member_verifier()
        .unwrap()
        .admission(previously_selected.1)
        .unwrap();
    let member_observation = recovered
        .observe_member_publication(foreign_prefix)
        .unwrap();
    let previous_topic = recovered
        .selected_member_topic_data(foreign_prefix, 42)
        .unwrap();
    let mut member_candidate = Some(member_candidate);
    let sdk_selection = (2, *result.after_authenticator_data());
    let core_selection =
        V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let wrong_selection =
        V2HostSelectionExpectation::from_trusted_host(previously_selected.0, header.spec().state);
    assert!(recovered
        .install_host_selected_add(
            joined_prefix,
            &mut member_candidate,
            &core_selection,
            sdk_selection
        )
        .is_err());
    assert!(recovered
        .install_host_selected_add(
            foreign_prefix,
            &mut member_candidate,
            &wrong_selection,
            sdk_selection
        )
        .is_err());
    assert!(recovered
        .install_host_selected_add(
            foreign_prefix,
            &mut member_candidate,
            &core_selection,
            (2, [0; 32])
        )
        .is_err());
    assert!(member_observation.is_current_cached());
    assert_eq!(recovered.selected_join_data().unwrap(), previously_selected);
    drop(
        recovered
            .install_host_selected_add(
                foreign_prefix,
                &mut member_candidate,
                &core_selection,
                sdk_selection,
            )
            .unwrap(),
    );
    assert!(member_candidate.is_none());
    assert_eq!(
        recovered.selected_join_data().unwrap(),
        (header.id(), previously_selected.1, 2)
    );
    assert!(foreign_prefix
        .selected_member_verifier()
        .unwrap()
        .admission(previously_selected.1)
        .unwrap()
        .same_original(&original_member));
    assert!(!member_observation.is_current_cached());
    assert_ne!(
        recovered
            .selected_member_topic_data(foreign_prefix, 42)
            .unwrap(),
        previous_topic
    );
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
    let decoded = V2JoinDeliveryData::decode(&wire).unwrap();
    assert_eq!(
        decoded.proof_data().unwrap(),
        ((header.spec().parent, header.id()), encoded.as_slice())
    );
    assert!(pending.validate_first_join_delivery_data(&wire).is_err());
    pending.validate_pinned_join_delivery_data(&wire).unwrap();
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
    joiner.fund_warm_sdk_graph(payment()).unwrap();
    let candidate = joiner
        .finish_pinned_join_delivery(&wire, None, None)
        .unwrap();
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
    let original_warm_welcome = candidate as *const _;
    assert_eq!(
        joiner.finish_governing_join_delivery(&delivery).unwrap() as *const _,
        original_warm_welcome
    );
    let sealed = joiner
        .seal_joined_outcome(&keys, context, &mut journal_rng)
        .unwrap();
    assert!(RecoveredV2JoinedOutcome::open(
        sealed,
        &keys,
        &pending,
        &wire[..wire.len() - 1],
        payment()
    )
    .is_err());
    let mut later =
        RecoveredV2JoinedOutcome::open(sealed, &keys, &pending, &wire, payment()).unwrap();
    assert_eq!(later.epoch_data(), 2);
    let mut warm_group = catcoms_replication::chat::ProjectionGroup::new(&anchor.group).unwrap();
    let mut warm_view = warm_group.begin_view();
    assert!(warm_view.try_ready());
    let mut cold_group = catcoms_replication::chat::ProjectionGroup::new(&anchor.group).unwrap();
    let mut cold_view = cold_group.begin_view();
    assert!(cold_view.try_ready());
    let (mut warm, selection) = joiner
        .prepare_initial_prefix(&wire, &warm_view, V2ProjectionOrigin::new(Arc::new(Live)))
        .unwrap();
    let mut cold = later
        .prepare_initial_prefix(&wire, &cold_view, V2ProjectionOrigin::new(Arc::new(Live)))
        .unwrap();
    assert!(warm.selected_member_verifier().is_err());
    assert!(cold.selected_member_verifier().is_err());
    let later_own = admission.admission_id();
    assert_eq!(warm.own_admission_data(), later_own);
    assert_eq!(cold.own_admission_data(), later_own);
    drop(warm.finish_host_selected(&selection).unwrap());
    drop(cold.finish_host_selected(&selection).unwrap());
    let mesh = catcoms_rt::Hub::new();
    let inputs = |n| {
        Some((
            mesh.join(catcoms_rt::PeerId::from_u64(n)),
            rand_chacha::ChaCha20Rng::seed_from_u64(n),
            Box::new(ManualClock::new(now)) as Box<dyn catcoms_rt::Clock + Send>,
        ))
    };
    let mut warm_inputs = inputs(121);
    assert!(joiner
        .take_closed_warm_node(&cold, &mut warm_inputs)
        .is_err());
    let mut installed_warm = joiner
        .take_closed_warm_node(&warm, &mut warm_inputs)
        .unwrap();
    let mut cold_inputs = inputs(122);
    assert!(later
        .take_closed_recovered_node(&warm, &mut cold_inputs)
        .is_err());
    let installed_cold = later
        .take_closed_recovered_node(&cold, &mut cold_inputs)
        .unwrap();
    assert_eq!(
        installed_warm.selected_join_data().unwrap(),
        (header.id(), later_own, 2)
    );
    assert_eq!(
        installed_cold.selected_join_data().unwrap(),
        (header.id(), later_own, 2)
    );
    assert_eq!(
        installed_warm
            .selected_member_topic_data(&warm, 42)
            .unwrap(),
        installed_cold
            .selected_member_topic_data(&cold, 42)
            .unwrap()
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
    let sibling_frame = V2MemberAddFrameData::encode(
        V2AdmissionStepData {
            admission,
            header: &signed_sibling,
            image: output.image_data(),
        },
        commit,
    )
    .unwrap();
    let mut decoder = catcoms_wire::Decoder::new(&sibling_frame);
    decoder.get_bytes().unwrap();
    decoder.get_u16().unwrap();
    decoder.get_bytes().unwrap();
    let conflicting_header =
        V2MemberAddFrameData::decode(&sibling_frame[..sibling_frame.len() - decoder.remaining()])
            .unwrap();
    assert!(joined
        .prepare_member_add_frame(
            joined_prefix,
            &conflicting_header,
            super::admission(&member_payment)
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
    let new_route = node.member_add_request_data(verifier).unwrap();
    assert!(recovered
        .member_add_request_data(foreign_prefix)
        .unwrap()
        .matches_verifier_data(new_route.verifier_data()));
    assert!(!new_route.matches_verifier_data(old_route.verifier_data()));
    assert!(node
        .preflight_selected_invite_signing(verifier, invite)
        .is_err());
    let (new_topic, _) = node.prepare_member_subscription(verifier, 42).unwrap();
    assert_eq!(
        recovered
            .selected_member_topic_data(foreign_prefix, 42)
            .unwrap(),
        new_topic
    );
    let original_native = native.source.observe();
    let founder_body = node.genesis_data().admission.id();
    assert!(node
        .prepare_removal_data(verifier, founder_body, original_native.clone())
        .is_err());
    assert!(node
        .prepare_removal_data(
            foreign_prefix.selected_member_verifier().unwrap(),
            remove_admission,
            original_native.clone()
        )
        .is_err());
    node.prepare_removal_data(verifier, remove_admission, original_native.clone())
        .unwrap();
    assert!(node
        .prepare_removal_data(verifier, remove_admission, invite_source.clone())
        .is_err());
    assert!(node.ordinary_member_signer_data().is_err());
    let mut remove_payment = Some(super::admission(&member_payment));
    let mut remove_ends = catcoms_rt::NativeChatKeyEnds::new();
    let mut wrong = native.cut(NativeChatKeyPurpose::AddSdk);
    assert!(node
        .execute_removal_native(
            verifier,
            remove_admission,
            &original_native,
            &mut remove_payment,
            &mut wrong,
            &mut remove_ends
        )
        .is_err());
    assert!(remove_payment.is_some());
    let mut sdk_cut = native.cut(NativeChatKeyPurpose::RemoveSdk);
    node.execute_removal_native(
        verifier,
        remove_admission,
        &original_native,
        &mut remove_payment,
        &mut sdk_cut,
        &mut remove_ends,
    )
    .unwrap();
    let commit = node
        .removal_outcome_data()
        .unwrap()
        .result
        .commit_data()
        .as_ptr();
    assert!(remove_payment.is_none());
    node.execute_removal_native(
        verifier,
        remove_admission,
        &original_native,
        &mut remove_payment,
        &mut sdk_cut,
        &mut remove_ends,
    )
    .unwrap();
    assert_eq!(
        node.removal_outcome_data()
            .unwrap()
            .result
            .commit_data()
            .as_ptr(),
        commit
    );
    let (_, mut signing) = node.removal_signing_parts_data(verifier).unwrap();
    let mut wrong = native.cut(NativeChatKeyPurpose::GoverningHeader);
    assert!(signing
        .sign_header_native(&original_native, &mut wrong, &mut remove_ends)
        .is_err());
    assert!(node.raw_removal_signature_data().is_none());
    let (_, mut signing) = node.removal_signing_parts_data(verifier).unwrap();
    let mut header_cut = native.cut(NativeChatKeyPurpose::RemovalHeader);
    signing
        .sign_header_native(&original_native, &mut header_cut, &mut remove_ends)
        .unwrap();
    let raw = *node.raw_removal_signature_data().unwrap();
    let (_, mut signing) = node.removal_signing_parts_data(verifier).unwrap();
    signing
        .sign_header_native(&original_native, &mut header_cut, &mut remove_ends)
        .unwrap();
    assert_eq!(node.raw_removal_signature_data(), Some(&raw));
    let header = node.materialize_signed_removal_data().unwrap().clone();
    let removal = node.removal_outcome_data().unwrap();
    let selected_remove_record = (
        header.clone(),
        removal.unsigned.body_data().clone(),
        removal.unsigned.image_data().to_vec(),
    );
    let old = recovered.selected_join_data().unwrap();
    let old_publication = recovered
        .observe_member_publication(foreign_prefix)
        .unwrap();
    let wire = V2MemberTransitionFrameData::encode_removal(
        &header,
        removal.unsigned.body_data(),
        removal.unsigned.image_data(),
        removal.result.commit_data(),
    )
    .unwrap();
    assert!(V2MemberAddFrameData::decode(&wire).is_err());
    let missing_commit_byte = &wire[..wire.len() - 1];
    let incomplete = V2MemberTransitionFrameData::decode(missing_commit_byte).unwrap();
    assert_eq!(incomplete.header_data(), &header);
    assert!(recovered
        .prepare_member_transition_frame(
            foreign_prefix,
            &incomplete,
            super::admission(&member_payment),
        )
        .is_err());
    let frame = V2MemberTransitionFrameData::decode(&wire).unwrap();
    let mut candidate = Some(
        recovered
            .prepare_member_transition_frame(
                foreign_prefix,
                &frame,
                super::admission(&member_payment),
            )
            .unwrap(),
    );
    assert!(candidate.as_ref().unwrap().matches_frame_data(&wire));
    assert!(!candidate
        .as_ref()
        .unwrap()
        .matches_frame_data(missing_commit_byte));
    assert_eq!(recovered.selected_join_data().unwrap(), old);
    let epoch = removal.result.selected_epoch_data();
    let mixed = V2GoverningJoinPrefixData::encode(
        first,
        &[
            V2GoverningJoinStepData {
                header: &retained_later.0,
                body: ReplayBody::Admission(&retained_later.1),
                image: &retained_later.2,
            },
            V2GoverningJoinStepData {
                header: &header,
                body: ReplayBody::Removal(removal.unsigned.body_data()),
                image: removal.unsigned.image_data(),
            },
        ],
    )
    .unwrap();
    assert!(V2GoverningJoinDeliveryData::encode(&packet, &mixed).is_err());
    let mixed = V2GoverningJoinPrefixData::decode(&mixed).unwrap();
    let selected_remove = V2InstalledFounderAddData {
        header: header.id(),
        state: header.spec().state,
        epoch: epoch.0,
        authenticator: epoch.1,
    };
    let snapshot = removal.result.secret_snapshot_data();
    assert!(
        PreparedV2FounderRecoveredAdd::restore_from_governing_prefix(
            original,
            selected_remove,
            removed_id,
            &mixed,
            snapshot,
            payment(),
        )
        .is_err()
    );
    assert!(PreparedV2FounderRecoveredAdd::restore_selected_removal(
        original,
        selected_remove,
        proof,
        snapshot,
        payment(),
    )
    .is_err());
    let mut restored = PreparedV2FounderRecoveredAdd::restore_selected_removal(
        original,
        selected_remove,
        &mixed,
        snapshot,
        payment(),
    )
    .unwrap();
    assert_eq!(restored.fingerprint_data(), selected_remove);
    assert_eq!(candidate.as_ref().unwrap().selected_epoch_data(), epoch);
    let selected = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    assert!(recovered
        .install_host_selected_add(foreign_prefix, &mut candidate, &selected, epoch)
        .is_err());
    let failed_epoch = (epoch.0 - 1, epoch.1);
    assert!(recovered
        .install_host_selected_membership(foreign_prefix, &mut candidate, &selected, failed_epoch,)
        .is_err());
    assert!(old_publication.is_current_cached());
    let retired = recovered
        .install_host_selected_membership(foreign_prefix, &mut candidate, &selected, epoch)
        .unwrap();
    assert!(retired.frozen_admission_data().is_some());
    assert!(candidate.is_none());
    assert!(!old_publication.is_current_cached());
    assert_eq!(
        recovered.selected_join_data().unwrap(),
        (header.id(), old.1, epoch.0)
    );
    drop(retired);
    let evicted_original = installed_warm.selected_join_data().unwrap();
    let evicted_publication = installed_warm.observe_member_publication(&warm).unwrap();
    assert!(installed_warm
        .prepare_member_transition_frame(&mut warm, &frame, super::admission(&member_payment))
        .is_err());
    assert!(recovered
        .prepare_member_eviction_frame(foreign_prefix, &frame, super::admission(&member_payment))
        .is_err());
    let mut eviction = Some(
        installed_warm
            .prepare_member_eviction_frame(&mut warm, &frame, super::admission(&member_payment))
            .unwrap(),
    );
    assert!(eviction.as_ref().unwrap().matches_frame_data(&wire));
    assert!(!eviction
        .as_ref()
        .unwrap()
        .matches_frame_data(missing_commit_byte));
    let removed = eviction.as_ref().unwrap().removed_data();
    assert_eq!(removed.0, epoch.0);
    assert!(installed_warm
        .install_host_selected_eviction(&mut warm, &mut eviction, &selected, (removed.0, [0; 32]),)
        .is_err());
    assert_eq!(
        installed_warm.selected_join_data().unwrap(),
        evicted_original
    );
    assert!(evicted_publication.is_current_cached());
    let mut retired_own = installed_warm
        .install_host_selected_eviction(&mut warm, &mut eviction, &selected, removed)
        .unwrap();
    assert!(eviction.is_none());
    assert!(retired_own.take_close_completion().unwrap().try_complete());
    assert!(retired_own.take_frozen_admission().is_some());
    assert!(!evicted_publication.is_current_cached());
    assert!(installed_warm.selected_join_data().is_err());
    assert!(installed_warm
        .selected_member_topic_data(&warm, 42)
        .is_err());
    assert_eq!(installed_warm.original_join_data(), evicted_original);
    drop(retired_own);
    let old_founder = node.prepare_member_subscription(verifier, 42).unwrap().1;
    let old_target = verifier.observe_live_member(removed_id).unwrap();
    let mut next = Some(node.derive_removal_selection_data(verifier).unwrap());
    let (_, prepared) = node.prepare_removal_install_data(verifier).unwrap();
    assert!(prepared
        .check_host_selection_data(
            foreign_prefix.selected_member_verifier().unwrap(),
            next.as_ref().unwrap()
        )
        .is_err());
    assert!(prepared.finish_host_selected(verifier, &mut None).is_err());
    assert!(old_target.is_live_cached());
    assert!(old_founder.is_current_cached());
    assert_eq!(node.installed_add_data(), Some(installed));
    let (_, prepared) = node.prepare_removal_install_data(verifier).unwrap();
    let fingerprint = prepared.expected_data();
    prepared.finish_host_selected(verifier, &mut next).unwrap();
    assert!(next.is_none());
    assert!(verifier.admission(removed_id).is_err());
    assert!(!old_target.is_live_cached());
    assert!(!old_founder.is_current_cached());
    let mut retired = node.take_retired_removal_data().unwrap();
    let mut close = retired.take_close_completion().unwrap();
    assert!(close.try_complete());
    assert!(retired.take_frozen_data().is_some());
    drop(retired);
    assert_eq!(node.installed_add_data(), Some(fingerprint));
    restored.bind_selected_member(verifier).unwrap();
    let mut next_invite = Request::new(node, 97);
    let expiry = now + 60_000;
    next_invite.change_invite(node, |spec| spec.expires_at_ms = expiry);
    let rejoining = Native::new();
    let rejoining_source = rejoining.source.observe();
    let mut device_cut = rejoining.cut(NativeChatKeyPurpose::JoinerDevice);
    let mut device_ends = catcoms_rt::NativeChatKeyEnds::new();
    let mut next_join = previous_source
        .prepare_native_rejoin(
            &keys,
            crate::NativeV2RejoinInputs {
                previous_invite: &fixture.invite,
                previous_context: context,
                previous_request: &previous_request,
                previous_signer: retained_later.1.spec().subject,
                previous_remove: (fingerprint.header, removed_id, fingerprint.epoch),
                invite: &next_invite.invite,
                token: [98; 32],
                now_ms: now,
                limits: join_limits,
            },
            crate::NativeV2JoinKeyEntry::new(&rejoining_source, &mut device_cut, &mut device_ends),
        )
        .unwrap();
    let next_wire = next_join.request_bytes_data().to_vec();
    let packet = V2JoinRequestPacketData::decode(&next_wire).unwrap();
    let next_request = packet.founder_add_data();
    let seed = &node.node.seed.body;
    let (policy, l0, file_key) = next_request.verify_recovered(original).unwrap();
    assert_eq!(policy, next_request.verify_seed(seed).unwrap());
    assert!(l0 == &*seed.metadata.routing[0].1);
    assert!(file_key == &*seed.metadata._file_wrap_key);
    assert!(restored
        .preflight_recovered_add_package_data(
            foreign_prefix.selected_member_verifier().unwrap(),
            original,
            next_request,
            &ManualClock::new(now),
            limits,
        )
        .is_err());
    restored
        .preflight_recovered_add_package_data(
            verifier,
            original,
            next_request,
            &ManualClock::new(now),
            limits,
        )
        .unwrap();
    assert!(restored
        .preflight_recovered_add_package_data(
            verifier,
            original,
            next_request,
            &ManualClock::new(expiry),
            limits,
        )
        .is_err());
    let owner = Native::new();
    let other = Native::new();
    let observation = owner.source.observe();
    restored
        .bind_native_member_source(
            verifier,
            &observation,
            &owner.cut(NativeChatKeyPurpose::MemberAction),
        )
        .unwrap();
    let paid = Arc::new(AtomicUsize::new(0));
    let native_inputs = |origin: &Native| {
        (
            origin.source.observe(),
            super::admission(&paid),
            [
                origin.cut(NativeChatKeyPurpose::AddSdk),
                origin.cut(NativeChatKeyPurpose::GoverningHeader),
                origin.cut(NativeChatKeyPurpose::JoinWelcome),
            ],
        )
    };
    assert!(restored
        .prepare_recovered_add_data((verifier, original), next_request, native_inputs(&other))
        .is_err());
    assert_eq!(paid.load(SeqCst), 0);
    restored
        .prepare_recovered_add_data((verifier, original), next_request, native_inputs(&owner))
        .ok()
        .unwrap();
    assert!(restored
        .prepare_recovered_add_data((verifier, original), next_request, native_inputs(&owner))
        .is_err());
    let mut rng = ChaCha20Rng::seed_from_u64(98);
    let clock = ManualClock::new(now);
    assert!(restored
        .execute_recovered_add_native(
            (verifier, original),
            V2FounderAddRequestData {
                key_package: &[],
                ..next_request
            },
            (&clock, &mut rng),
        )
        .is_err());
    assert!(restored.completed_recovered_add_data().is_err());
    restored
        .execute_recovered_add_native((verifier, original), next_request, (&clock, &mut rng))
        .unwrap();
    let outcome = restored.completed_recovered_add_data().unwrap();
    assert_eq!(outcome.after_epoch_data(), fingerprint.epoch + 1);
    let commit = blake3::hash(outcome.commit_data());
    restored
        .execute_recovered_add_native((verifier, original), next_request, (&clock, &mut rng))
        .unwrap();
    assert_eq!(
        blake3::hash(
            restored
                .completed_recovered_add_data()
                .unwrap()
                .commit_data()
        ),
        commit,
    );
    assert_eq!(restored.fingerprint_data(), fingerprint);
    assert!(restored.recovered_add_outcome_data().is_err());
    restored
        .prepare_recovered_admission_data(verifier, original)
        .unwrap();
    let outcome = restored.recovered_add_outcome_data().unwrap();
    assert_eq!(
        outcome.unsigned_data().header_spec_data().parent,
        fingerprint.header
    );
    assert_eq!(blake3::hash(outcome.result_data().commit_data()), commit);
    outcome.require_request_data(next_request).unwrap();
    let mut key = restored
        .recovered_add_signing_parts(verifier, original)
        .unwrap();
    assert!(key.sign_welcome_native().is_err());
    key.sign_header_native().unwrap();
    let header_raw = *restored.raw_recovered_add_header_data().unwrap();
    restored.prepare_recovered_welcome_data().unwrap();
    assert!(!restored
        .recovered_welcome_intent_data()
        .unwrap()
        .1
        .is_empty());
    let mut key = restored
        .recovered_add_signing_parts(verifier, original)
        .unwrap();
    key.sign_header_native().unwrap();
    key.sign_welcome_native().unwrap();
    let welcome_raw = *restored.raw_recovered_welcome_data().unwrap();
    restored.materialize_recovered_welcome_data().unwrap();
    let (header, response) = restored.recovered_join_response_data().unwrap();
    assert_eq!(header.spec().parent, fingerprint.header);
    let outcome = restored.recovered_add_outcome_data().unwrap();
    let selected_rejoin_admission = outcome.admission_data().admission_id();
    let with_ancestor = V2GoverningJoinPrefixData::encode(
        first,
        &[
            V2GoverningJoinStepData {
                header: &retained_later.0,
                body: ReplayBody::Admission(&retained_later.1),
                image: &retained_later.2,
            },
            V2GoverningJoinStepData {
                header: &selected_remove_record.0,
                body: ReplayBody::Removal(&selected_remove_record.1),
                image: &selected_remove_record.2,
            },
            V2GoverningJoinStepData {
                header,
                body: ReplayBody::Admission(outcome.admission_data()),
                image: outcome.unsigned_data().image_data(),
            },
        ],
    )
    .unwrap();
    let rejoin_wire = V2GoverningJoinDeliveryData::encode(
        &response.encode_packet_data().unwrap(),
        &with_ancestor,
    )
    .unwrap();
    let with_ancestor = V2GoverningJoinPrefixData::decode(&with_ancestor).unwrap();
    let accepted = &next_invite.invite.spec_data().anchor;
    let new_admission = (header.id(), selected_rejoin_admission);
    let old_remove = (fingerprint.header, removed_id, fingerprint.epoch);
    assert!(with_ancestor
        .verify_rejoin_ancestry(accepted, new_admission, old_remove)
        .is_ok());
    for unrelated in [
        (retained_later.0.id(), removed_id, fingerprint.epoch),
        (fingerprint.header, old.1, fingerprint.epoch),
        (fingerprint.header, removed_id, fingerprint.epoch - 1),
    ] {
        assert!(with_ancestor
            .verify_rejoin_ancestry(accepted, new_admission, unrelated)
            .is_err());
    }
    assert_eq!(
        response.binding.spec_data().add_header,
        *header.id().as_bytes()
    );
    next_join
        .validate_pinned_join_delivery_data(&rejoin_wire)
        .unwrap();
    assert!(next_join
        .validate_first_join_delivery_data(&rejoin_wire)
        .is_err());
    let next_context = V2PendingJoinContextData {
        original_pending: [116; 32],
        ..context
    };
    let next_source = next_join
        .seal_pending_source(&keys, next_context, &mut journal_rng)
        .unwrap();
    let next_pending = RecoveredV2PendingJoinSource::open(
        next_source,
        &keys,
        next_context,
        &next_invite.invite,
        join_limits,
    )
    .unwrap();
    next_pending
        .validate_pinned_join_delivery_data(&rejoin_wire)
        .unwrap();
    let (_rejoin_cancel, signal) = tokio::sync::watch::channel(false);
    let rejoin_caller = catcoms_rt::RequestCancellation::new(signal, None);
    let wrong_native = other.source.observe();
    let mut wrong_cut = other.cut(NativeChatKeyPurpose::JoinerWelcomeSdk);
    let mut welcome_ends = catcoms_rt::NativeChatKeyEnds::new();
    assert!(next_join
        .finish_pinned_join_delivery(
            &rejoin_wire,
            Some(&rejoin_caller),
            Some(crate::NativeV2JoinKeyEntry::new(
                &wrong_native,
                &mut wrong_cut,
                &mut welcome_ends
            )),
        )
        .is_err());
    next_join.fund_warm_sdk_graph(payment()).unwrap();
    let mut welcome_cut = rejoining.cut(NativeChatKeyPurpose::JoinerWelcomeSdk);
    let next_completed = next_join
        .finish_pinned_join_delivery(
            &rejoin_wire,
            Some(&rejoin_caller),
            Some(crate::NativeV2JoinKeyEntry::new(
                &rejoining_source,
                &mut welcome_cut,
                &mut welcome_ends,
            )),
        )
        .unwrap();
    assert_eq!(
        next_completed.facts_data().signer_data(),
        &retained_later.1.spec().subject
    );
    assert_eq!(
        next_completed.facts_data().join_epoch_data(),
        fingerprint.epoch + 1
    );
    let sealed_outcome = next_join
        .seal_joined_outcome(&keys, next_context, &mut journal_rng)
        .unwrap();
    let cold_join = RecoveredV2JoinedOutcome::open(
        sealed_outcome,
        &keys,
        &next_pending,
        &rejoin_wire,
        payment(),
    )
    .unwrap();
    assert_eq!(cold_join.governing_head_data(), new_admission.0);
    assert_eq!(cold_join.epoch_data(), fingerprint.epoch + 1);
    assert_eq!(
        *restored.raw_recovered_add_header_data().unwrap(),
        header_raw
    );
    assert_eq!(*restored.raw_recovered_welcome_data().unwrap(), welcome_raw);
    let foreign = other.ui.retire();
    assert!(restored
        .retire_recovered_add_for_closed_ui(&[Some(&foreign)], &mut rng)
        .unwrap()
        .is_none());
    let ui_close = owner.ui.retire();
    assert!(!ui_close.is_drained());
    let retired = restored
        .retire_recovered_add_for_closed_ui(&[Some(&ui_close)], &mut rng)
        .unwrap()
        .unwrap();
    assert!(!ui_close.is_drained());
    drop(retired);
    assert!(ui_close.is_drained());
    assert!(restored.recovered_add_outcome_data().is_ok());
    let source_close = owner.source.observe_retirement();
    assert!(restored
        .discard_recovered_add_for_closed_source(&source_close)
        .is_err());
    assert_eq!(
        node.prepare_member_subscription(verifier, 42).unwrap().0,
        recovered
            .selected_member_topic_data(foreign_prefix, 42)
            .unwrap()
    );
    let (_source, signal) = tokio::sync::watch::channel(false);
    let source = catcoms_rt::RequestCancellation::new(signal, None);
    let old_selection = restored
        .prepare_member_topic(verifier, 42, source.clone())
        .unwrap()
        .1;
    let mut selected = Some(
        restored
            .derive_recovered_add_selection_data(verifier, original)
            .unwrap(),
    );
    let install = restored
        .prepare_recovered_add_install(verifier, original)
        .unwrap();
    assert!(install
        .check_host_selection_data(
            foreign_prefix.selected_member_verifier().unwrap(),
            selected.as_ref().unwrap(),
        )
        .is_err());
    assert!(install.finish_host_selected(verifier, &mut None).is_err());
    assert_eq!(restored.fingerprint_data(), fingerprint);
    assert!(old_selection.is_current_cached());
    let install = restored
        .prepare_recovered_add_install(verifier, original)
        .unwrap();
    install
        .check_host_selection_data(verifier, selected.as_ref().unwrap())
        .unwrap();
    let expected = install.expected_data();
    install
        .finish_host_selected(verifier, &mut selected)
        .unwrap();
    assert!(selected.is_none());
    assert_eq!(expected.epoch, fingerprint.epoch + 1);
    assert_eq!(verifier.replay_data().header().id(), expected.header);
    assert_eq!(restored.fingerprint_data(), expected);
    assert!(!old_selection.is_current_cached());
    drop(restored.take_retired_recovered_add_install().unwrap());
    assert!(restored
        .prepare_member_topic(verifier, 42, source)
        .unwrap()
        .1
        .is_current_cached());
    owner.source.close();
    restored
        .discard_recovered_add_for_closed_source(&source_close)
        .unwrap();
    assert!(source_close.is_drained());
    assert!(restored.recovered_add_outcome_data().is_err());
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
    let governed = TextCurrentAddLimits::governed_text_v2();
    let graph_limits = TextCurrentSdkRestoreLimits {
        max_typed_bytes: governed.restore_typed_bytes,
        max_work: governed.restore_work,
    };
    let payment = || TextCurrentSdkRestoreAdmission::new(Paid(graph_limits)).unwrap();
    prepared.fund_warm_sdk_graph(payment()).unwrap();
    assert!(prepared.fund_warm_sdk_graph(payment()).is_err());
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
            (&joined_node, &mut warm_prefix),
            (&mut cold_node, &mut prefix),
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
