//! Real whole native runtime, held Current4 and the same member verifier.
//! The harness covers unsigned and signed local storage and exact retry.
use super::super::super::network::tests::prepared_mesh_founder_with_clock;
use super::super::super::tests::selected_governed;
use super::*;
use crate::store::critical_namespaces::{
    current_layout::set_after_original_source_hook_for_test, governance_layout::GOVERNANCE_MARKER,
    ARCHIVE_BOOTSTRAP_ROOT,
};
use crate::store::NativeTextInviteRecovery;

fn find_named(root: &std::path::Path, name: &str) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, name: &str, depth: u8, result: &mut Vec<std::path::PathBuf>) {
        assert!(depth < 12, "bounded isolated fixture tree");
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == name {
                result.push(entry.path());
            }
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), name, depth + 1, result);
            }
        }
    }
    let mut result = Vec::new();
    visit(root, name, 0, &mut result);
    result
}
fn only_named(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let mut paths = find_named(root, name);
    assert_eq!(paths.len(), 1);
    paths.pop().unwrap()
}

#[tokio::test]
async fn native_runtime_invite_join_request_requires_the_actual_signed_journal() {
    native_governed_add_fixture(JoinCase::Staged).await;
}

#[tokio::test]
async fn native_runtime_invite_governed_add_resumes_install_after_a_lost_head_save_reply() {
    native_governed_add_fixture(JoinCase::InterruptedInstall).await;
}

#[tokio::test]
async fn native_runtime_invite_returns_only_the_original_installed_join_response() {
    native_governed_add_fixture(JoinCase::NativeResponse).await;
}

#[tokio::test]
async fn native_runtime_invite_returns_only_the_original_installed_join_response_into_one_joined_runtime(
) {
    native_governed_add_fixture(JoinCase::WarmRuntime).await;
}

#[derive(PartialEq, Eq)]
enum JoinCase {
    Staged,
    InterruptedInstall,
    NativeResponse,
    WarmRuntime,
}

async fn native_governed_add_fixture(case: JoinCase) {
    use crate::store::critical_namespaces::native_registry::preparation::tests::{
        current_policy, SECRET,
    };
    use crate::store::{NativeTextGovernedStorage, NativeTextJoinedStorage};
    use catcoms_replication::chat::owner_header::OwnerHeaderId;
    use catcoms_sync::{DirectJoinLimits, PreparedV2JoinRequest};
    use rand_core::SeedableRng;
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let seed = match case {
        JoinCase::Staged => 0xe710,
        JoinCase::InterruptedInstall => 0xe717,
        JoinCase::NativeResponse => 0xe718,
        JoinCase::WarmRuntime => 0xe719,
    };
    let (owner, row) = activate(
        prepared_mesh_founder_with_clock(&vault, &budget, seed, Box::new(catcoms_rt::SystemClock)),
        &vault,
        &budget,
    );
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let negative = RequestCancellation::new(receiver, None);
    let saved = prepare(&owner, &row, &vault, &budget)
        .save_invite([58; 32], 60_000, &negative)
        .unwrap();
    let invite =
        crate::store::NativeTextInvitePacketData::decode(saved.packet_bytes_data()).unwrap();
    let mut request = Some(
        PreparedV2JoinRequest::prepare(
            catcoms_mls::MlsDevice::generate().unwrap(),
            &invite.binding().spec_data().anchor,
            invite.binding(),
            [59; 32],
            saved.expires_at_ms() - 1,
            DirectJoinLimits::new(512 * 1024, 512 * 1024, 64 * 1024).unwrap(),
        )
        .unwrap(),
    );
    let native_joiner = matches!(case, JoinCase::NativeResponse | JoinCase::WarmRuntime);
    let join_directory = native_joiner.then(|| tempfile::tempdir().unwrap());
    let mut join_rng = rand_chacha::ChaCha20Rng::seed_from_u64(0xe715);
    let mut joined_storage = join_directory.as_ref().map(|join_dir| {
        drop(selected_governed(join_dir.path()));
        let mut original = NativeTextGovernedStorage::open(
            join_dir.path(),
            SECRET,
            &mut join_rng,
            current_policy(),
        )
        .unwrap();
        original.stage_joined_pin_layout(&mut join_rng).unwrap();
        drop(original);
        let mut joined =
            NativeTextJoinedStorage::open(join_dir.path(), SECRET, &mut join_rng, current_policy())
                .unwrap();
        joined
            .stage_first_join(request.take().unwrap(), invite.binding(), &mut join_rng)
            .unwrap();
        joined.select_first_join().unwrap();
        joined
    });
    let bytes = if let Some(joined) = &joined_storage {
        joined.first_join_request_data().unwrap()
    } else {
        request.as_ref().unwrap().request_bytes_data()
    };
    let raw = only_named(directory.path(), "invite.raw.v1");
    let parked = raw.with_extension("held");
    let original = std::fs::read(&raw).unwrap();
    let check =
        || prepare(&owner, &row, &vault, &budget).check_received_join_request(bytes, &negative);
    let checked = check().unwrap();
    assert_eq!(
        checked.founder_add_data().invite.id_data(),
        saved.binding_id()
    );
    assert_eq!(
        checked.founder_add_data().request.spec_data().request_token,
        [59; 32]
    );
    std::fs::rename(&raw, &parked).unwrap();
    assert!(check().is_err());
    assert!(!raw.exists());
    std::fs::rename(&parked, &raw).unwrap();
    let mut changed = bytes.to_vec();
    *changed.last_mut().unwrap() ^= 1;
    assert!(prepare(&owner, &row, &vault, &budget)
        .check_received_join_request(&changed, &negative)
        .is_err());
    assert!(check().is_ok());
    assert_eq!(std::fs::read(&raw).unwrap(), original);

    let prior = body("finish this member request before changing the group");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&prior), &negative),
        Ok(V2MemberSaveOutcome::Ready),
    );
    assert!(
        check().is_err(),
        "presealed work holds the lane before any save End"
    );
    let final_prepared = super::admission::final_preparation(&owner, &row, &vault, &budget);
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let (native, _) = super::admission::custody(&commit, &negative).await;
    assert_eq!(
        final_prepared.try_admit(native).unwrap().run(),
        crate::store::MemberSaveJobReport::SavedLocal,
    );
    assert!(
        check().is_err(),
        "the lane still owes reconciliation after the End drains"
    );
    assert!(matches!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&prior), &negative),
        Ok(V2MemberSaveOutcome::Saved { .. }),
    ));
    assert!(check().is_ok());

    // Cover the fixed fence on the actual original governed Current. The
    // native fixture has already reconciled its one existing member lane.
    let packet = check().unwrap();
    let replacement_request = PreparedV2JoinRequest::prepare(
        catcoms_mls::MlsDevice::generate().unwrap(),
        &invite.binding().spec_data().anchor,
        invite.binding(),
        [60; 32],
        saved.expires_at_ms() - 1,
        DirectJoinLimits::new(512 * 1024, 512 * 1024, 64 * 1024).unwrap(),
    )
    .unwrap();
    let replacement = prepare(&owner, &row, &vault, &budget)
        .check_received_join_request(replacement_request.request_bytes_data(), &negative)
        .unwrap();
    let accept = |wire: &[u8]| {
        owner
            .capture_member(
                &row,
                command(&vault, &budget),
                owner.policy.prepare_member().unwrap(),
            )
            .accept_join_request(wire, &negative)
    };
    if native_joiner {
        let expected_request = packet.founder_add_data().request.id_data();
        let original_request = bytes.to_vec();
        let joined = joined_storage.as_mut().unwrap();
        let join_dir = join_directory.as_ref().unwrap().path();
        assert!(joined.finish_first_join(&[0], &mut join_rng).is_err());
        let entry_name = "chat-critical-namespaces.joined-welcome-entry.bin";
        let outcome_name = "chat-critical-namespaces.joined-welcome-outcome.bin";
        assert!(find_named(join_dir, entry_name).is_empty());
        assert_eq!(joined.warm_join_outcome_data().unwrap(), None);
        use catcoms_rt::{Responder, TransportEvent};
        let (client, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
        assert!(joined
            .exchange_first_join(&invite, &client, negative.clone(), &mut join_rng)
            .await
            .is_err());
        let capture = || {
            owner.capture_member(
                &row,
                command(&vault, &budget),
                owner.policy.prepare_member().unwrap(),
            )
        };
        let mut requests = capture()
            .open_connected_join_requests(negative.clone())
            .unwrap();
        assert!(matches!(
            capture().open_connected_join_requests(negative.clone()),
            Err(V2MemberSaveRefusal::Busy)
        ));
        let address = {
            let cell = row.original.runtime.upgrade().unwrap();
            let state = cell.state.lock().unwrap();
            state
                .body
                .as_ref()
                .unwrap()
                .network
                .as_ref()
                .unwrap()
                .loopback_dial_address_for_test()
        };
        tokio::task::yield_now().await;
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            client.dial(address.parse().unwrap()).await.unwrap();
            client
                .wait_for_peer_connected(invite.connected_peer_data())
                .await
                .unwrap();
        })
        .await
        .unwrap();
        let serve = async {
            let Some(TransportEvent::Request {
                data,
                responder,
                proto,
                ..
            }) = requests.next().await
            else {
                panic!("connected exact JoinRequest");
            };
            let (abandoned, receiver) = Responder::channel();
            drop(receiver);
            assert!(matches!(
                capture().serve_connected_join_request(proto, &data, abandoned, negative.clone()),
                Err(V2MemberSaveRefusal::Cancelled)
            ));
            assert!(find_named(directory.path(), "text.add.response.v2").is_empty());
            capture()
                .serve_connected_join_request(proto, &data, responder, negative.clone())
                .unwrap();
        };
        if case == JoinCase::WarmRuntime {
            let mut runtime = joined_storage
                .take()
                .unwrap()
                .bind_first_join_runtime(client, join_rng, Box::new(catcoms_rt::SystemClock))
                .ok()
                .unwrap();
            let (canceled, retired) = tokio::sync::watch::channel(true);
            assert!(runtime
                .complete_first_join(&invite, RequestCancellation::new(retired, None))
                .await
                .is_err());
            drop(canceled);
            let (completed, ()) = tokio::time::timeout(std::time::Duration::from_secs(8), async {
                tokio::join!(
                    runtime.complete_first_join(&invite, negative.clone()),
                    serve
                )
            })
            .await
            .unwrap();
            let (head, own, epoch) = completed.unwrap();
            assert_eq!(epoch, 1);
            drop(requests);
            let saved = accept(&original_request).unwrap();
            let packet =
                catcoms_sync::V2JoinResponsePacketData::decode(saved.packet_bytes_data()).unwrap();
            assert_eq!(
                packet.response_data().binding.spec_data().admission,
                *own.as_bytes()
            );
            assert_eq!(
                packet.response_data().binding.spec_data().add_header,
                *head.as_bytes()
            );
            assert_eq!(
                runtime
                    .complete_first_join(&invite, negative.clone())
                    .await
                    .unwrap(),
                (head, own, epoch)
            );
            drop(runtime);
            let cold = NativeTextJoinedStorage::open(
                join_dir,
                SECRET,
                &mut rand_chacha::ChaCha20Rng::seed_from_u64(0xe720),
                current_policy(),
            )
            .unwrap();
            assert_eq!(
                cold.recovered_join_outcome_data().unwrap(),
                Some((head, epoch))
            );
            return;
        }
        let (joined_result, ()) = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            tokio::join!(
                joined.exchange_first_join(&invite, &client, negative.clone(), &mut join_rng),
                serve,
            )
        })
        .await
        .unwrap();
        joined_result.unwrap();
        drop(requests);
        let saved = accept(&original_request).unwrap();
        let decoded =
            catcoms_sync::V2JoinResponsePacketData::decode(saved.packet_bytes_data()).unwrap();
        assert_eq!(
            decoded.response_data().binding.spec_data().request_binding,
            expected_request
        );
        assert!(find_named(directory.path(), "text.add.head-stage.v2").is_empty());
        assert!(accept(replacement.packet_bytes_data()).is_err());
        let retry = accept(&original_request).unwrap();
        assert_eq!(
            saved.packet_bytes_data().as_ptr(),
            retry.packet_bytes_data().as_ptr()
        );
        assert_eq!(
            saved.governing_proof_bytes_data().as_ptr(),
            retry.governing_proof_bytes_data().as_ptr()
        );
        let proof_bytes = saved.governing_proof_bytes_data();
        assert!(
            catcoms_sync::V2FirstAddProofData::decode(&proof_bytes[..proof_bytes.len() - 1])
                .is_err()
        );
        catcoms_sync::V2FirstAddProofData::decode(proof_bytes).unwrap();
        let delivery = saved.encode_delivery_data().unwrap();
        let entry = only_named(join_dir, entry_name);
        let outcome = only_named(join_dir, outcome_name);
        let exact_entry = std::fs::read(&entry).unwrap();
        let exact_outcome = std::fs::read(&outcome).unwrap();
        assert!(joined
            .finish_first_join(&delivery[..delivery.len() - 1], &mut join_rng)
            .is_err());
        joined.finish_first_join(&delivery, &mut join_rng).unwrap();
        assert_eq!(std::fs::read(&entry).unwrap(), exact_entry);
        assert_eq!(std::fs::read(&outcome).unwrap(), exact_outcome);
        let add_header =
            OwnerHeaderId::from_bytes(decoded.response_data().binding.spec_data().add_header);
        assert_eq!(
            joined.warm_join_outcome_data().unwrap(),
            Some((add_header, 1))
        );
        let parked_outcome = outcome.with_extension("held-original");
        std::fs::rename(&outcome, &parked_outcome).unwrap();
        std::fs::write(&outcome, &exact_outcome).unwrap();
        assert!(joined.warm_join_outcome_data().is_err());
        std::fs::remove_file(&outcome).unwrap();
        std::fs::rename(&parked_outcome, &outcome).unwrap();
        assert_eq!(
            joined.warm_join_outcome_data().unwrap(),
            Some((add_header, 1))
        );
        drop(joined_storage.take());
        std::fs::remove_file(&outcome).unwrap();
        let ambiguous =
            NativeTextJoinedStorage::open(join_dir, SECRET, &mut join_rng, current_policy())
                .unwrap();
        assert!(ambiguous.recovered_join_outcome_data().unwrap().is_none());
        assert!(ambiguous.first_join_request_data().is_err());
        drop(ambiguous);
        let mut corrupt = exact_outcome.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        std::fs::write(&outcome, &corrupt).unwrap();
        assert!(
            NativeTextJoinedStorage::open(join_dir, SECRET, &mut join_rng, current_policy())
                .is_err()
        );
        assert_eq!(std::fs::read(&outcome).unwrap(), corrupt);
        std::fs::write(&outcome, exact_outcome).unwrap();
        let cold = NativeTextJoinedStorage::open(join_dir, SECRET, &mut join_rng, current_policy())
            .unwrap();
        assert_eq!(
            cold.recovered_join_outcome_data().unwrap(),
            Some((add_header, 1))
        );
        let message = body("founder message after governed Add");
        let request = || V2MemberSignRequest {
            token: [0xe8; 32],
            ..sign_request(&message)
        };
        let head = only_named(directory.path(), "text.add.response.v2").with_file_name("head.bin");
        let parked_head = head.with_extension("test-parked");
        std::fs::rename(&head, &parked_head).unwrap();
        std::fs::copy(&parked_head, &head).unwrap();
        assert!(prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(request(), &negative)
            .is_err());
        std::fs::remove_file(&head).unwrap();
        std::fs::rename(&parked_head, &head).unwrap();
        assert_eq!(
            prepare(&owner, &row, &vault, &budget).prepare_member_save(request(), &negative),
            Ok(V2MemberSaveOutcome::Ready),
        );
        let pending = owner
            .capture_member(
                &row,
                command(&vault, &budget),
                owner.policy.prepare_member().unwrap(),
            )
            .finish_admission([0xe8; 32])
            .unwrap();
        let commit = Arc::new(tokio::sync::Mutex::new(()));
        let (custody, _) = super::admission::custody(&commit, &negative).await;
        assert_eq!(
            pending.try_admit(custody).unwrap().run(),
            crate::store::MemberSaveJobReport::SavedLocal
        );
        let mut close = owner.prepare_retirement().unwrap();
        close.try_retire().unwrap();
        assert_eq!(saved.packet_bytes_data(), retry.packet_bytes_data());
        return;
    }
    let preparation = owner.capture_member(
        &row,
        command(&vault, &budget),
        owner.policy.prepare_member().unwrap(),
    );
    let (cell, (keys, cuts)) = preparation
        .prepare_using_issuer(|source, command| {
            use catcoms_rt::NativeChatKeyPurpose as Purpose;
            let cut = |purpose| source.prepare_cut(command, purpose).map_err(|_| refused());
            Ok((
                NativeFounderKeys::prepare(source, command)?,
                [
                    cut(Purpose::AddSdk)?,
                    cut(Purpose::GoverningHeader)?,
                    cut(Purpose::JoinWelcome)?,
                ],
            ))
        })
        .unwrap();
    let mut job = preparation.finish_prepared((cell, keys)).unwrap();
    let runtime = job.inner.loan.body.as_mut().unwrap();
    let driver = runtime.driver.as_mut().unwrap();
    let route = runtime
        .network
        .as_ref()
        .unwrap()
        .invite_routing_plan_data(driver.binding_data().unwrap())
        .unwrap();
    driver
        .host_for_governed_add_test()
        .cover_governed_add_fence_for_test(
            runtime.work.current_for_governed_add_test(),
            &packet,
            &replacement,
            &route,
            &job.inner.keys,
            runtime.rng.as_mut().unwrap(),
            &negative,
            cuts,
            |name, present| {
                if name == "invite.raw.v1" {
                    if present {
                        std::fs::rename(&raw, &parked).unwrap();
                    } else {
                        std::fs::rename(&parked, &raw).unwrap();
                        assert_eq!(std::fs::read(&raw).unwrap(), original);
                    }
                    return;
                }
                if name == "text.add.raw-welcome.v2" {
                    let raw_welcome =
                        only_named(directory.path(), "text.add.sdk-entry.v2").with_file_name(name);
                    let parked = raw_welcome.with_extension("test-parked");
                    if present {
                        std::fs::rename(&raw_welcome, &parked).unwrap();
                    } else {
                        std::fs::rename(&parked, &raw_welcome).unwrap();
                    }
                    return;
                }
                assert_eq!(name, "text.add.outcome.v2");
                let outcome = only_named(directory.path(), "text.add.sdk-entry.v2")
                    .with_file_name("text.add.outcome.v2");
                if present {
                    std::fs::write(&outcome, vec![3; 4096]).unwrap();
                } else {
                    assert_eq!(std::fs::read(&outcome).unwrap(), vec![3; 4096]);
                    std::fs::remove_file(&outcome).unwrap();
                }
            },
        )
        .unwrap();
    job.inner.loan.recorded = true;
    drop(job);
    let fence = only_named(directory.path(), "text.wal.stage.v1");
    let frozen = std::fs::read(&fence).unwrap();
    assert!(frozen.len() > 40);
    let request = only_named(directory.path(), "text.add.request.v2");
    let request_bytes = std::fs::read(&request).unwrap();
    let entry = only_named(directory.path(), "text.add.sdk-entry.v2");
    let entry_bytes = std::fs::read(&entry).unwrap();
    assert!(entry_bytes.len() > 40);
    let outcome = only_named(directory.path(), "text.add.outcome.v2");
    let outcome_bytes = std::fs::read(&outcome).unwrap();
    assert!(outcome_bytes.len() > 40);
    let header_attempt = only_named(directory.path(), "text.add.header-attempt.v2");
    let raw_header = only_named(directory.path(), "text.add.raw-header.v2");
    let governing = only_named(directory.path(), "text.add.governing.v2");
    let welcome_intent = only_named(directory.path(), "text.add.welcome-intent.v2");
    let welcome_attempt = only_named(directory.path(), "text.add.welcome-attempt.v2");
    let raw_welcome = only_named(directory.path(), "text.add.raw-welcome.v2");
    let response = only_named(directory.path(), "text.add.response.v2");
    let head_stage = only_named(directory.path(), "text.add.head-stage.v2");
    let header_attempt_bytes = std::fs::read(&header_attempt).unwrap();
    let raw_header_bytes = std::fs::read(&raw_header).unwrap();
    let governing_bytes = std::fs::read(&governing).unwrap();
    let welcome_intent_bytes = std::fs::read(&welcome_intent).unwrap();
    let welcome_attempt_bytes = std::fs::read(&welcome_attempt).unwrap();
    let raw_welcome_bytes = std::fs::read(&raw_welcome).unwrap();
    let response_bytes = std::fs::read(&response).unwrap();
    let head_stage_bytes = std::fs::read(&head_stage).unwrap();
    assert!(raw_header_bytes.len() > header_attempt_bytes.len());
    assert!(check().is_err());
    let parked_fence = fence.with_extension("held");
    let parked_request = request.with_extension("held");
    let parked_entry = entry.with_extension("held");
    let parked_outcome = outcome.with_extension("held");
    let parked_header_attempt = header_attempt.with_extension("held");
    let parked_raw_header = raw_header.with_extension("held");
    let parked_governing = governing.with_extension("held");
    let parked_welcome_intent = welcome_intent.with_extension("held");
    let parked_welcome_attempt = welcome_attempt.with_extension("held");
    let parked_raw_welcome = raw_welcome.with_extension("held");
    let parked_response = response.with_extension("held");
    let parked_head_stage = head_stage.with_extension("held");
    std::fs::rename(&fence, &parked_fence).unwrap();
    std::fs::rename(&request, &parked_request).unwrap();
    std::fs::rename(&entry, &parked_entry).unwrap();
    std::fs::rename(&outcome, &parked_outcome).unwrap();
    std::fs::rename(&header_attempt, &parked_header_attempt).unwrap();
    std::fs::rename(&raw_header, &parked_raw_header).unwrap();
    std::fs::rename(&governing, &parked_governing).unwrap();
    std::fs::rename(&welcome_intent, &parked_welcome_intent).unwrap();
    std::fs::rename(&welcome_attempt, &parked_welcome_attempt).unwrap();
    std::fs::rename(&raw_welcome, &parked_raw_welcome).unwrap();
    std::fs::rename(&response, &parked_response).unwrap();
    std::fs::rename(&head_stage, &parked_head_stage).unwrap();
    assert!(check().is_ok());
    let orphan = fence.with_file_name("text.add.outcome.v2");
    std::fs::write(&orphan, b"incomplete held outcome").unwrap();
    assert!(check().is_err());
    assert_eq!(std::fs::read(&orphan).unwrap(), b"incomplete held outcome");
    std::fs::remove_file(&orphan).unwrap();
    assert!(check().is_ok());
    std::fs::rename(&parked_fence, &fence).unwrap();
    std::fs::rename(&parked_request, &request).unwrap();
    std::fs::rename(&parked_entry, &entry).unwrap();
    std::fs::rename(&parked_outcome, &outcome).unwrap();
    std::fs::rename(&parked_header_attempt, &header_attempt).unwrap();
    std::fs::rename(&parked_raw_header, &raw_header).unwrap();
    std::fs::rename(&parked_governing, &governing).unwrap();
    std::fs::rename(&parked_welcome_intent, &welcome_intent).unwrap();
    std::fs::rename(&parked_welcome_attempt, &welcome_attempt).unwrap();
    std::fs::rename(&parked_raw_welcome, &raw_welcome).unwrap();
    std::fs::rename(&parked_response, &response).unwrap();
    std::fs::rename(&parked_head_stage, &head_stage).unwrap();
    assert_eq!(std::fs::read(fence).unwrap(), frozen);
    assert_eq!(std::fs::read(request).unwrap(), request_bytes);
    assert_eq!(std::fs::read(entry).unwrap(), entry_bytes);
    assert_eq!(std::fs::read(outcome).unwrap(), outcome_bytes);
    assert_eq!(std::fs::read(header_attempt).unwrap(), header_attempt_bytes);
    assert_eq!(std::fs::read(raw_header).unwrap(), raw_header_bytes);
    assert_eq!(std::fs::read(governing).unwrap(), governing_bytes);
    assert_eq!(std::fs::read(welcome_intent).unwrap(), welcome_intent_bytes);
    assert_eq!(
        std::fs::read(welcome_attempt).unwrap(),
        welcome_attempt_bytes
    );
    assert_eq!(std::fs::read(raw_welcome).unwrap(), raw_welcome_bytes);
    assert_eq!(std::fs::read(response).unwrap(), response_bytes);
    assert_eq!(std::fs::read(&head_stage).unwrap(), head_stage_bytes);

    if case == JoinCase::InterruptedInstall {
        let head = head_stage.with_file_name("head.bin");
        let original_head = head.with_extension("test-parked");
        let mut job = prepare(&owner, &row, &vault, &budget);
        let runtime = job.inner.loan.body.as_mut().unwrap();
        runtime
            .driver
            .as_mut()
            .unwrap()
            .host_for_governed_add_test()
            .install_governed_add_for_test(
                runtime.work.current_for_governed_add_test(),
                &packet,
                &route,
                &job.inner.keys,
                runtime.rng.as_mut().unwrap(),
                &negative,
                |replace| {
                    if replace {
                        std::fs::rename(&head, &original_head).unwrap();
                        std::fs::write(&head, &head_stage_bytes).unwrap();
                    } else {
                        std::fs::remove_file(&head).unwrap();
                        std::fs::rename(&original_head, &head).unwrap();
                    }
                },
            )
            .unwrap();
        job.inner.loan.recorded = true;
        drop(job);
        assert!(!head_stage.exists());
        assert_eq!(
            std::fs::read(head_stage.with_file_name("head.bin")).unwrap(),
            head_stage_bytes
        );
        assert!(check().is_err());
        assert!(accept(replacement.packet_bytes_data()).is_err());
        assert!(accept(bytes).is_ok());
        let mut close = owner.prepare_retirement().unwrap();
        close.try_retire().unwrap();
        return;
    }

    let key_work = {
        let entries = owner.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        entries[0].owner.observe_retirement()
    };
    let unpolled = prepare(&owner, &row, &vault, &budget);
    let cleanup = owner.capture_ui_add_retirement();
    let retiring = vault.begin_ui_retirement(7).unwrap();
    let ui = retiring.observe_drain().unwrap();
    let mut batch = retiring.into_batch().unwrap();
    assert!(!cleanup.try_retire(&ui).unwrap());
    batch.close_owners();
    assert!(!ui.is_drained_cached());
    assert!(
        !cleanup.try_retire(&ui).unwrap(),
        "the original runtime is still loaned"
    );
    let original_cell = unpolled.inner.loan.cell.clone();
    drop(unpolled);
    assert!(cleanup.try_retire(&ui).unwrap());
    assert!(ui.is_drained_cached());
    assert!(original_cell.state.lock().unwrap().interrupted);
    assert!(
        !key_work.is_closed_cached(),
        "UI lock keeps the original Source open"
    );
    let mut close = owner.prepare_retirement().unwrap();
    close.try_retire().unwrap();
    assert!(key_work.is_drained());
}

#[tokio::test]
async fn native_runtime_invite_recovery_reports_exact_original_and_holds_missing_raw() {
    type Host = crate::store::PreparedV2TextHost<MeshService, ChaCha20Rng>;
    use NativeTextInviteRecovery as Recovery;
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe70e);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let source = RequestCancellation::new(receiver, None);
    let token = [55; 32];
    let inspect = |token, lifetime| {
        prepare(&owner, &row, &vault, &budget).inspect_invite(token, lifetime, &source)
    };
    assert!(inspect(token, 60_000).is_err());
    assert!(find_named(directory.path(), "invite.intent.v1").is_empty());
    prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent(token, 60_000, &source)
        .unwrap();
    assert_eq!(inspect(token, 60_000), Ok(Recovery::IntentOnly));
    assert!(inspect([56; 32], 60_000).is_err());
    assert!(inspect(token, 60_001).is_err());

    let intent = only_named(directory.path(), "invite.intent.v1");
    let attempt = intent.with_file_name("invite.attempt.v1");
    let raw = intent.with_file_name("invite.raw.v1");
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let cancelled = RequestCancellation::new(receiver, None);
    Host::after_invite_key_for_test(Box::new(move || {
        sender.send_replace(true);
    }));
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).save_invite(token, 60_000, &cancelled),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    let attempt_frame = std::fs::read(&attempt).unwrap();
    assert_eq!(inspect(token, 60_000), Ok(Recovery::KeyAttemptHeld));
    assert!(!raw.exists());
    assert_eq!(std::fs::read(&attempt).unwrap(), attempt_frame);

    let saved = prepare(&owner, &row, &vault, &budget)
        .save_invite(token, 60_000, &source)
        .unwrap();
    let signed = Recovery::SignatureRecovered {
        binding_id: saved.binding_id(),
        expires_at_ms: saved.expires_at_ms(),
    };
    assert_eq!(inspect(token, 60_000), Ok(signed));
    let attempt_parked = attempt.with_extension("held");
    let raw_frame = std::fs::read(&raw).unwrap();
    std::fs::rename(&attempt, &attempt_parked).unwrap();
    assert!(
        inspect(token, 60_000).is_err(),
        "Raw cannot outlive its attempted parent"
    );
    assert_eq!(std::fs::read(&raw).unwrap(), raw_frame);
    std::fs::rename(&attempt_parked, &attempt).unwrap();
    assert_eq!(inspect(token, 60_000), Ok(signed));

    let raw_parked = raw.with_extension("held");
    std::fs::rename(&raw, &raw_parked).unwrap();
    assert_eq!(inspect(token, 60_000), Ok(Recovery::KeyAttemptHeld));
    assert!(prepare(&owner, &row, &vault, &budget)
        .save_invite(token, 60_000, &source)
        .is_err());
    assert!(!raw.exists());
    assert_eq!(std::fs::read(raw_parked).unwrap(), raw_frame);
}

#[tokio::test]
async fn native_runtime_invite_recovery_rechecks_file_identity_after_authentication() {
    type Host = crate::store::PreparedV2TextHost<MeshService, ChaCha20Rng>;
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe70f);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let negative = RequestCancellation::new(receiver, None);
    let token = [57; 32];
    prepare(&owner, &row, &vault, &budget)
        .save_invite(token, 60_000, &negative)
        .unwrap();
    let raw = only_named(directory.path(), "invite.raw.v1");
    let raw_frame = std::fs::read(&raw).unwrap();
    let held = raw.with_extension("held");
    let held_for_hook = held.clone();
    let path = raw.clone();
    let copy = raw_frame.clone();
    Host::after_invite_read_for_test(Box::new(move || {
        std::fs::rename(&path, &held_for_hook).unwrap();
        std::fs::write(path, copy).unwrap();
    }));
    assert!(prepare(&owner, &row, &vault, &budget)
        .inspect_invite(token, 60_000, &negative)
        .is_err());
    assert_eq!(std::fs::read(&raw).unwrap(), raw_frame);
    assert_eq!(std::fs::read(&held).unwrap(), raw_frame);
    std::fs::remove_file(&raw).unwrap();
    std::fs::rename(held, &raw).unwrap();

    // This replacement has a valid tag under the same domain, context and
    // intent. It still cannot satisfy Raw's phase and parent grammar.
    let attempt = raw.with_file_name("invite.attempt.v1");
    let wrong_phase = std::fs::read(attempt).unwrap();
    std::fs::write(&raw, &wrong_phase).unwrap();
    assert!(prepare(&owner, &row, &vault, &budget)
        .inspect_invite(token, 60_000, &negative)
        .is_err());
    assert_eq!(std::fs::read(&raw).unwrap(), wrong_phase);

    // A present torn record is a refusal, not evidence that Raw is absent.
    std::fs::write(&raw, &raw_frame[..128]).unwrap();
    assert!(prepare(&owner, &row, &vault, &budget)
        .inspect_invite(token, 60_000, &negative)
        .is_err());
    assert_eq!(std::fs::read(raw).unwrap(), &raw_frame[..128]);
}

#[tokio::test]
async fn native_runtime_invite_signing_requires_covered_attempt_and_replays_only_covered_raw() {
    type Host = crate::store::PreparedV2TextHost<MeshService, ChaCha20Rng>;
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe70b);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let token = [52; 32];
    let (_, expires) = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent(token, 60_000, &session)
        .unwrap();
    let intent = only_named(directory.path(), "invite.intent.v1");
    let attempt = intent.with_file_name("invite.attempt.v1");
    let raw = intent.with_file_name("invite.raw.v1");
    let original = std::fs::read(&intent).unwrap();

    // Leave the attempt torn. The key hook must survive this first failure.
    Host::fail_partial_invite_write_for_test();
    let key_entries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_entries = key_entries.clone();
    Host::after_invite_key_for_test(Box::new(move || {
        observed_entries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Host::fail_partial_invite_write_for_test();
    }));
    let save = || prepare(&owner, &row, &vault, &budget).save_invite(token, 60_000, &session);
    assert!(save().is_err());
    assert_eq!(key_entries.load(std::sync::atomic::Ordering::SeqCst), 0);
    let partial_attempt = std::fs::read(&attempt).unwrap();
    assert_eq!(partial_attempt.len(), 128);
    assert!(!raw.exists());

    // The key returns once; the first Raw write loses its result after a prefix.
    assert!(save().is_err());
    assert_eq!(key_entries.load(std::sync::atomic::Ordering::SeqCst), 1);
    let covered_attempt = std::fs::read(&attempt).unwrap();
    assert!(covered_attempt.starts_with(&partial_attempt));
    assert!(covered_attempt.len() > partial_attempt.len());
    let partial_raw = std::fs::read(&raw).unwrap();
    assert_eq!(partial_raw.len(), 128);

    let saved = save().unwrap();
    assert_ne!(saved.binding_id(), [0; 32]);
    assert_eq!(saved.expires_at_ms(), expires);
    let packet =
        crate::store::NativeTextInvitePacketData::decode(saved.packet_bytes_data()).unwrap();
    assert_eq!(packet.binding().id_data(), saved.binding_id());
    assert_eq!(packet.binding().spec_data().expires_at_ms, expires);
    assert_eq!(save(), Ok(saved));
    assert_eq!(std::fs::read(&intent).unwrap(), original);
    assert_eq!(std::fs::read(&attempt).unwrap(), covered_attempt);
    let covered_raw = std::fs::read(&raw).unwrap();
    assert!(covered_raw.starts_with(&partial_raw));
    assert!(covered_raw.len() > partial_raw.len());
    std::fs::write(&raw, &partial_raw).unwrap();
    assert!(
        save().is_err(),
        "a covered Raw cannot be repaired as an in-flight prefix"
    );
    assert_eq!(std::fs::read(&raw).unwrap(), partial_raw);
}

#[tokio::test]
async fn native_runtime_invite_signing_refuses_preexisting_attempt_or_orphan_raw() {
    for (name, other) in [
        ("invite.attempt.v1", "invite.raw.v1"),
        ("invite.raw.v1", "invite.attempt.v1"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (vault, budget) = selected_governed(directory.path());
        let (owner, row) = active(&vault, &budget, 0xe70c);
        let (_sender, receiver) = tokio::sync::watch::channel(false);
        let session = RequestCancellation::new(receiver, None);
        let token = [53; 32];
        prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent(token, 60_000, &session)
            .unwrap();
        let intent = only_named(directory.path(), "invite.intent.v1");
        let path = intent.with_file_name(name);
        std::fs::write(&path, b"unknown previous signing state").unwrap();
        let save = || prepare(&owner, &row, &vault, &budget).save_invite(token, 60_000, &session);
        assert!(save().is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"unknown previous signing state"
        );
        assert!(!intent.with_file_name(other).exists());
        std::fs::remove_file(&path).unwrap();
        assert!(
            save().is_err(),
            "observed unknown state must survive removal"
        );
        assert!(!path.exists());
        assert!(!intent.with_file_name(other).exists());
    }
}

#[tokio::test]
async fn native_runtime_invite_signing_late_cancel_keeps_the_exact_saved_result() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe70d);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    super::super::super::invite::AFTER_COVER.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            sender.send_replace(true);
        }));
    });
    let token = [54; 32];
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).save_invite(token, 60_000, &session),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    let path = only_named(directory.path(), "invite.raw.v1");
    let covered = std::fs::read(&path).unwrap();
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(receiver, None);
    assert!(prepare(&owner, &row, &vault, &budget)
        .save_invite(token, 60_001, &fresh)
        .is_err());
    let saved = prepare(&owner, &row, &vault, &budget)
        .save_invite(token, 60_000, &fresh)
        .unwrap();
    assert_ne!(saved.binding_id(), [0; 32]);
    assert_eq!(std::fs::read(path).unwrap(), covered);
}

#[tokio::test]
async fn native_runtime_invite_original_uses_current4_and_does_not_block_ordinary_member() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe701);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let original = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([41; 32], 60_000, &session)
        .unwrap();
    assert_ne!(original.0, [0; 32]);
    assert!(original.1 >= 60_000);
    let journal = only_named(directory.path(), "invite.intent.v1");
    let frame = std::fs::read(&journal).unwrap();
    assert!(frame.len() > 40 && frame.len() <= 8192);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([41; 32], 60_000, &session),
        Ok(original)
    );
    assert_eq!(
        std::fs::read(&journal).unwrap(),
        frame,
        "exact retry preserves the same sealed frame"
    );
    // Different token/settings cannot replace the retained draw.
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([42; 32], 60_000, &session)
        .is_err());
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([41; 32], 60_001, &session)
        .is_err());
    let text = body("ordinary authoring after unsigned invite preparation");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([41; 32], 60_000, &session),
        Ok(original)
    );
    assert_eq!(
        std::fs::read(journal).unwrap(),
        frame,
        "ordinary authoring preserves the invitation family"
    );
}

#[tokio::test]
async fn native_runtime_invite_partial_write_reuses_the_same_original_and_ciphertext() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe706);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    crate::store::PreparedV2TextHost::<MeshService, ChaCha20Rng>::fail_partial_invite_write_for_test();
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([47; 32], 60_000, &session)
        .is_err());
    let path = only_named(directory.path(), "invite.intent.v1");
    let prefix = std::fs::read(&path).unwrap();
    assert_eq!(prefix.len(), 128);
    let original = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([47; 32], 60_000, &session)
        .unwrap();
    let frame = std::fs::read(&path).unwrap();
    assert!(frame.len() > prefix.len() && frame.starts_with(&prefix));
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([47; 32], 60_000, &session),
        Ok(original)
    );
    assert_eq!(std::fs::read(path).unwrap(), frame);
}

#[tokio::test]
async fn native_runtime_invite_corrupt_or_missing_covered_journal_never_becomes_empty_usage() {
    for missing in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (vault, budget) = selected_governed(directory.path());
        let (owner, row) = active(&vault, &budget, 0xe707 + u64::from(missing));
        let (_sender, receiver) = tokio::sync::watch::channel(false);
        let session = RequestCancellation::new(receiver, None);
        prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([48; 32], 60_000, &session)
            .unwrap();
        let path = only_named(directory.path(), "invite.intent.v1");
        let original = std::fs::read(&path).unwrap();
        if missing {
            std::fs::rename(&path, path.with_extension("held")).unwrap();
        } else {
            std::fs::write(&path, b"torn quota/intent").unwrap();
        }
        assert!(prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([48; 32], 60_000, &session)
            .is_err());
        assert!(prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([49; 32], 60_000, &session)
            .is_err());
        if missing {
            assert!(!path.exists());
        } else {
            assert_eq!(std::fs::read(&path).unwrap(), b"torn quota/intent");
        }
        assert!(original.len() > 40);
    }
}

#[tokio::test]
async fn native_runtime_invite_unknown_preexisting_state_is_preserved_and_ordinary_saves_still_work(
) {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe709);
    // Enter the real ordinary lane first to ensure its generation-1 pages exist.
    let text = body("ordinary state before the invitation journal");
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let path = only_named(directory.path(), "text.pages").join("invite.intent.v1");
    std::fs::write(&path, b"unknown/future journal").unwrap();
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([50; 32], 60_000, &session)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unknown/future journal");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    std::fs::remove_file(&path).unwrap();
    assert!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([50; 32], 60_000, &session)
            .is_err(),
        "removing previously unknown quota evidence cannot reset usage"
    );
    assert!(!path.exists());
}

#[tokio::test]
async fn native_runtime_invite_lost_reply_after_cover_retries_without_rewriting_the_intent() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe70a);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    super::super::super::invite::AFTER_COVER.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            sender.send_replace(true);
        }));
    });
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([51; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    let path = only_named(directory.path(), "invite.intent.v1");
    let frame = std::fs::read(&path).unwrap();
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(receiver, None);
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([51; 32], 60_000, &fresh)
        .is_ok());
    assert_eq!(std::fs::read(path).unwrap(), frame);
}

#[tokio::test]
async fn native_runtime_invite_refuses_plain_native3_without_disabling_ordinary_member() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe702);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([43; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Failed)
    );
    let text = body("old-family member authoring still works");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
}

#[tokio::test]
async fn native_runtime_invite_requires_original4_after_outer_admission() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe703);
    let marker = directory
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(GOVERNANCE_MARKER);
    let parked = directory.path().join("invite-original4.held");
    let target = marker.clone();
    let keep = parked.clone();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = ran.clone();
    set_after_original_source_hook_for_test(Box::new(move || {
        std::fs::rename(&target, keep).unwrap();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }));
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([44; 32], 60_000, &session)
        .is_err());
    assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    std::fs::rename(parked, marker).unwrap();
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([44; 32], 60_000, &session)
        .is_ok());
}

#[tokio::test]
async fn native_runtime_invite_cancelled_or_closed_source_never_returns_prepared_success() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe704);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let job = prepare(&owner, &row, &vault, &budget);
    sender.send_replace(true);
    assert_eq!(
        job.prepare_invite_intent([45; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    sender.send_replace(false);
    let job = prepare(&owner, &row, &vault, &budget);
    owner.entries.lock().unwrap()[0].owner.close();
    assert_eq!(
        job.prepare_invite_intent([45; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Unavailable)
    );
}
