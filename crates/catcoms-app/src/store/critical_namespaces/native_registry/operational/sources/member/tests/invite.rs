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

#[derive(Debug)]
struct DependencyTestScope(RequestCancellation);

#[async_trait::async_trait]
impl catcoms_rt::RequestAuthority for DependencyTestScope {
    fn authorized(&self) -> bool {
        !self.0.is_cancelled()
    }

    async fn revoked(&self) {
        self.0.clone().cancelled().await;
    }
}

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

#[tokio::test]
async fn native_runtime_invite_returns_only_the_original_installed_join_response_to_reopened_pending(
) {
    native_governed_add_fixture(JoinCase::ColdPending).await;
}

#[tokio::test]
async fn mounted_join_request_uses_the_original_native_command_and_retains_its_pending() {
    mounted_join_fixture(false).await;
}

#[tokio::test]
async fn mounted_join_request_cold_recovers_the_exact_registry_stage_only_with_its_welcome() {
    mounted_join_fixture(true).await;
}

#[tokio::test]
async fn cold_native_founder_reuses_the_covered_source_and_listener() {
    use super::super::super::tests::selected_joined;
    use crate::store::critical_namespaces::native_registry::preparation::tests::{
        current_policy, SECRET,
    };
    use catcoms_rt::MeshTransport as _;
    use rand_core::SeedableRng;
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_joined(dir.path());
    let (founder, row) = active(&vault, &budget, 0xe727);
    let (_alive, signal) = tokio::sync::watch::channel(false);
    let caller = RequestCancellation::new(signal, None);
    let saved = prepare(&founder, &row, &vault, &budget)
        .save_invite([63; 32], 60_000, &caller)
        .unwrap();
    let invite =
        crate::store::NativeTextInvitePacketData::decode(saved.packet_bytes_data()).unwrap();
    let id = row.id_data();
    let mut retirement = founder.prepare_retirement().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while retirement.try_retire().unwrap()
            == crate::store::NativeTextSourceRetirementStatus::Pending
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the original founder actor drained before restart");
    drop((retirement, row, founder, vault));

    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0xe728);
    let storage = crate::store::NativeTextJoinedStorage::open_for_native_startup(
        dir.path(),
        SECRET,
        &mut rng,
        current_policy(),
    )
    .unwrap();
    let network = dir
        .path()
        .join("servers")
        .join(format!("{id}.native-net-v1"));
    let parked = network.with_extension("held-before-founder-recovery");
    std::fs::rename(&network, &parked).unwrap();
    std::fs::copy(&parked, &network).unwrap();
    let storage = match storage.prepare_recovered_founder() {
        Ok(_) => panic!("equal bytes replaced the cold-selected network file"),
        Err((original, _)) => original,
    };
    std::fs::remove_file(&network).unwrap();
    std::fs::rename(parked, &network).unwrap();
    let mut recovered = storage
        .prepare_recovered_founder()
        .unwrap_or_else(|(_, error)| panic!("the original cold founder refused: {error}"));
    assert_eq!(recovered.registration_data().0.id, id);
    assert_eq!(recovered.registration_data().1, "Original");
    let (peer, port) = recovered.transport_data();
    assert_eq!(peer, invite.connected_peer_data());
    let address = invite.direct_address_data(std::net::Ipv4Addr::LOCALHOST.into());
    assert!(address.contains(&format!("/tcp/{port}/")));
    assert!(recovered.restore_first_add(&caller).is_err());
    let genesis = catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes(
        invite.binding().spec_data().anchor.header,
    );
    assert_eq!(recovered.prove_promoted_genesis(&caller).unwrap(), genesis);
    let genesis_budget = crate::store::GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        recovered
            .restore_promoted_genesis_sdk(genesis_budget, &caller)
            .unwrap(),
        genesis
    );
    let head = only_named(dir.path(), "text.seed").with_file_name("head.bin");
    let fence = head.with_file_name("text.wal.stage.v1");
    std::fs::write(&fence, b"interrupted Add fence").unwrap();
    assert!(recovered.prove_promoted_genesis(&caller).is_err());
    assert_eq!(std::fs::read(&fence).unwrap(), b"interrupted Add fence");
    std::fs::remove_file(&fence).unwrap();
    assert_eq!(recovered.prove_promoted_genesis(&caller).unwrap(), genesis);
    let original_head = head.with_extension("held-before-original-genesis-check");
    std::fs::rename(&head, &original_head).unwrap();
    std::fs::copy(&original_head, &head).unwrap();
    assert!(recovered.prove_promoted_genesis(&caller).is_err());
    assert!(recovered
        .restore_promoted_genesis_sdk(genesis_budget, &caller)
        .is_err());
    std::fs::remove_file(&head).unwrap();
    std::fs::rename(original_head, head).unwrap();
    assert_eq!(recovered.prove_promoted_genesis(&caller).unwrap(), genesis);
    assert_eq!(
        recovered
            .restore_promoted_genesis_sdk(genesis_budget, &caller)
            .unwrap(),
        genesis
    );
    let (_closed, signal) = tokio::sync::watch::channel(true);
    assert!(recovered
        .bind_original_tcp(&RequestCancellation::new(signal, None))
        .is_err());

    let source = only_named(dir.path(), "origin");
    let parked = source.with_extension("held-before-founder-transport");
    std::fs::rename(&source, &parked).unwrap();
    std::fs::copy(&parked, &source).unwrap();
    assert!(recovered.bind_original_tcp(&caller).is_err());
    std::fs::remove_file(&source).unwrap();
    std::fs::rename(parked, source).unwrap();
    recovered.bind_original_tcp(&caller).unwrap();
    recovered.bind_original_tcp(&caller).unwrap();
    let (probe, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
    assert_eq!(
        probe.dial_outcome(address.parse().unwrap()).await.unwrap(),
        catcoms_rt::DialSubmission::Submitted
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(8),
        probe.wait_for_peer_connected(peer),
    )
    .await
    .expect("the recovered founder listened at its exact signed address")
    .unwrap();
}

async fn mounted_join_fixture(recover_stage: bool) {
    use super::super::super::tests::{policy, selected_joined};
    use crate::store::critical_namespaces::native_registry::preparation::tests::{
        current_policy, SECRET,
    };
    use rand_core::{RngCore, SeedableRng};

    let founder_dir = tempfile::tempdir().unwrap();
    let (founder, founder_budget) = selected_joined(founder_dir.path());
    let (owner, row) = activate(
        prepared_mesh_founder_with_clock(
            &founder,
            &founder_budget,
            if recover_stage { 0xe725 } else { 0xe723 },
            Box::new(catcoms_rt::SystemClock),
        ),
        &founder,
        &founder_budget,
    );
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let negative = RequestCancellation::new(receiver, None);
    let saved = prepare(&owner, &row, &founder, &founder_budget)
        .save_invite([58; 32], 60_000, &negative)
        .unwrap();
    let invite =
        crate::store::NativeTextInvitePacketData::decode(saved.packet_bytes_data()).unwrap();
    let anchor = &invite.binding().spec_data().anchor;
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_joined(dir.path());
    let original = vault
        .capture_native_sources::<catcoms_net::MeshService, rand_chacha::ChaCha20Rng>(
            policy().prepare().unwrap(),
        )
        .finish()
        .unwrap();
    let capture = || original.capture_join_request(command(&vault, &budget));
    let mut rng =
        rand_chacha::ChaCha20Rng::seed_from_u64(if recover_stage { 0xe726 } else { 0xe724 });
    let now = saved.expires_at_ms() - 1;
    let token = [61; 32];
    let path = dir
        .path()
        .join("chat-critical-namespaces.joined-pending.bin");

    assert!(owner
        .capture_join_request(command(&founder, &founder_budget))
        .cover(&invite, anchor, token, now, &negative, &mut rng)
        .is_err());
    assert!(!founder_dir.path().join(path.file_name().unwrap()).exists());
    assert!(original
        .capture_join_request(command(&founder, &founder_budget))
        .cover(&invite, anchor, token, now, &negative, &mut rng)
        .is_err());
    let held = capture();
    assert!(capture()
        .cover(&invite, anchor, token, now, &negative, &mut rng)
        .is_err());
    let mut foreign = anchor.clone();
    foreign.state[0] ^= 1;
    assert!(held
        .cover(&invite, &foreign, token, now, &negative, &mut rng)
        .is_err());
    assert!(!path.exists());

    let current = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Current.name());
    let parked = dir.path().join("original-before-mounted-join-reply");
    let catalog = std::fs::read(&current).unwrap();
    let (file, saved_file) = (current.clone(), parked.clone());
    crate::store::persistence::governance_layout::joined::AFTER_PIN_INDEX_SELECT.with(|hook| {
        hook.set(Some(Box::new(move || {
            std::fs::rename(&file, saved_file).unwrap();
            std::fs::write(file, catalog).unwrap();
        })));
    });
    assert!(capture()
        .cover(&invite, anchor, token, now, &negative, &mut rng)
        .is_err());
    assert!(parked.is_file());
    assert!(path.is_file());
    let ciphertext = std::fs::read(&path).unwrap();
    std::fs::remove_file(&current).unwrap();
    std::fs::rename(parked, current).unwrap();

    let mut retry_rng = rng.clone();
    let selected = capture()
        .cover(&invite, anchor, token, now, &negative, &mut rng)
        .unwrap();
    let after = saved.expires_at_ms();
    assert!(capture()
        .cover(&invite, anchor, [62; 32], after, &negative, &mut rng)
        .is_err());
    assert!(capture()
        .cover(&invite, anchor, token, after, &negative, &mut rng)
        .is_ok_and(|original| original == selected));
    assert_eq!(rng.next_u64(), retry_rng.next_u64());
    assert_eq!(std::fs::read(&path).unwrap(), ciphertext);

    let waiting = capture()
        .prepare_exchange(&invite, negative.clone())
        .unwrap();
    let local_transport = waiting.local_transport_data();
    assert!(waiting.receive().await.is_err());
    let (original, vault) = if recover_stage {
        let mut retirement = original.prepare_retirement().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while retirement.try_retire().unwrap()
                == crate::store::NativeTextSourceRetirementStatus::Pending
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the original pending TCP actor drained before restart");
        drop(retirement);
        drop(original);
        drop(vault);
        let mut current = current_policy();
        current.archive = current
            .archive
            .with_checkpoint_join_execution(2, 10_000)
            .unwrap();
        let cold = crate::store::NativeTextJoinedStorage::open_for_native_startup(
            dir.path(),
            SECRET,
            &mut rng,
            current,
        )
        .unwrap();
        assert_eq!(cold.row_count_data().unwrap(), 0);
        assert!(cold.pending_join_data().unwrap() == Some(selected));
        assert_eq!(
            cold.recovered_join_transport_data().unwrap(),
            Some(local_transport)
        );
        let vault = cold
            .into_native_vault_without_registered_row(
                (
                    Arc::new(std::sync::atomic::AtomicU64::new(7)),
                    Arc::new(std::sync::atomic::AtomicU64::new(7)),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ),
                budget.clone(),
            )
            .unwrap();
        let original = vault
            .capture_native_sources(policy().prepare().unwrap())
            .finish()
            .unwrap();
        assert!(original
            .prepare_creation(command(&vault, &budget), request(1, "Wrong role"), inputs())
            .is_err());
        let capture = || original.capture_join_request(command(&vault, &budget));
        let mut untouched_rng = rng.clone();
        assert!(capture()
            .cover(&invite, anchor, [62; 32], after, &negative, &mut rng)
            .is_err());
        assert!(capture()
            .cover(&invite, anchor, token, after, &negative, &mut rng)
            .is_ok_and(|original| original == selected));
        assert_eq!(rng.next_u64(), untouched_rng.next_u64());
        assert_eq!(std::fs::read(&path).unwrap(), ciphertext);
        (original, vault)
    } else {
        (original, vault)
    };
    let capture = || original.capture_join_request(command(&vault, &budget));
    let founder_capture = || {
        owner.capture_member(
            &row,
            command(&founder, &founder_budget),
            owner.policy.prepare_member().unwrap(),
        )
    };
    let mut requests = founder_capture()
        .open_connected_join_requests(negative.clone())
        .unwrap();
    let mut covered_request = None;
    let mut serve = async || {
        let Some(catcoms_rt::TransportEvent::Request {
            from,
            data,
            responder,
            proto,
            ..
        }) = requests.next().await
        else {
            panic!("expected the original mounted request");
        };
        assert_eq!(from, local_transport.0);
        assert_eq!(blake3::hash(&data[1..]).as_bytes(), &selected.0);
        covered_request = Some(data.clone());
        founder_capture()
            .serve_connected_join_request(proto, &data, responder, negative.clone())
            .unwrap();
    };
    let (revocation, signal) = tokio::sync::watch::channel(false);
    let revoked = capture()
        .prepare_exchange(&invite, RequestCancellation::new(signal, None))
        .unwrap();
    assert_eq!(revoked.local_transport_data(), local_transport);
    assert_ne!(local_transport.0, invite.connected_peer_data());
    assert_ne!(local_transport.1, 0);
    assert!(capture()
        .prepare_exchange(&invite, negative.clone())
        .is_err());
    assert!(capture()
        .cover(&invite, anchor, token, after, &negative, &mut rng)
        .is_ok_and(|original| original == selected));
    let scheduler = catcoms_discovery::EndpointDialScheduler::new(Default::default());
    revoked
        .connect_approved_host(&invite, std::net::Ipv4Addr::LOCALHOST.into(), &scheduler)
        .await
        .unwrap();
    let mut receive = async |original: crate::store::NativeTextJoinedExchange| {
        let (delivery, ()) = tokio::join!(original.receive(), serve());
        delivery.unwrap()
    };
    let delivery = tokio::time::timeout(std::time::Duration::from_secs(10), receive(revoked))
        .await
        .unwrap();
    revocation.send(true).unwrap();
    assert!(delivery.cover(&mut rng).is_err());
    assert!(find_named(
        dir.path(),
        "chat-critical-namespaces.joined-welcome-entry.bin"
    )
    .is_empty());

    let exact = capture()
        .prepare_exchange(&invite, negative.clone())
        .unwrap();
    assert_eq!(exact.local_transport_data(), local_transport);
    exact
        .connect_approved_host(&invite, std::net::Ipv4Addr::LOCALHOST.into(), &scheduler)
        .await
        .unwrap();
    let delivery = tokio::time::timeout(std::time::Duration::from_secs(10), receive(exact))
        .await
        .unwrap();
    let covered_delivery = delivery.wire_for_test().to_vec();

    let mut delivery = Some(delivery);
    let mut inputs = Some((
        rng.clone(),
        Box::new(catcoms_rt::SystemClock) as Box<dyn catcoms_rt::Clock + Send>,
    ));
    assert!(owner
        .capture_join_installation(&mut delivery, &mut inputs)
        .is_err());
    assert!(delivery.is_some() && inputs.is_some());
    let mut installation = original
        .capture_join_installation(&mut delivery, &mut inputs)
        .unwrap();
    assert!(delivery.is_none() && inputs.is_none());
    let current = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Current.name());
    let parked = current.with_extension("before-original-joined-attachment");
    let catalog = std::fs::read(&current).unwrap();
    std::fs::rename(&current, &parked).unwrap();
    std::fs::write(&current, &catalog).unwrap();
    assert!(installation.install().is_err());
    assert!(find_named(
        dir.path(),
        "chat-critical-namespaces.joined-welcome-entry.bin"
    )
    .is_empty());
    std::fs::remove_file(&current).unwrap();
    std::fs::rename(parked, current).unwrap();
    let (head, own_add, epoch) = installation.install().unwrap();
    assert_eq!(installation.install().unwrap(), (head, own_add, epoch));
    let outcome = (head, epoch);

    sender.send(true).unwrap();
    let inspect = || original.capture_joined_selection(command(&vault, &budget));
    assert!(inspect().inspect(&invite, token, &negative).is_err());
    let (_fresh_sender, fresh) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(fresh, None);
    assert_eq!(
        inspect().inspect(&invite, token, &fresh).unwrap(),
        Some((head, own_add, epoch))
    );
    let registration = inspect()
        .inspect_registration(&invite, token, &fresh)
        .unwrap()
        .unwrap();
    assert_eq!(registration.transport_data(), local_transport);
    assert!(registration.context == selected.1);
    assert_eq!(registration.request, selected.0);
    assert_eq!(registration.invocation, token);
    assert_eq!(registration.profile, anchor.profile);
    assert!(inspect().inspect(&invite, [62; 32], &fresh).is_err());
    let file = dir
        .path()
        .join("chat-critical-namespaces.joined-welcome-outcome.bin");
    let inspection_parked = file.with_extension("before-parked-join-inspection");
    let contents = std::fs::read(&file).unwrap();
    std::fs::rename(&file, &inspection_parked).unwrap();
    std::fs::write(&file, &contents).unwrap();
    assert!(inspect().inspect(&invite, token, &fresh).is_err());
    std::fs::remove_file(&file).unwrap();
    std::fs::rename(inspection_parked, file).unwrap();
    assert_eq!(
        inspect().inspect(&invite, token, &fresh).unwrap(),
        Some((head, own_add, epoch))
    );

    let index = only_named(dir.path(), "native-registry.bin");
    let stage = index.with_extension("stage");
    let prior_index = std::fs::read(&index).unwrap();
    let outcome_path = dir
        .path()
        .join("chat-critical-namespaces.joined-welcome-outcome.bin");
    let registration_parked = outcome_path.with_extension("held-original-at-index-selection");
    let (file, held) = (outcome_path.clone(), registration_parked.clone());
    let raw = std::fs::read(&outcome_path).unwrap();
    use crate::store::persistence::native_registry::operational::{
        before_target_proof, RenameTarget,
    };
    before_target_proof(
        RenameTarget::Index,
        Box::new(move || {
            std::fs::rename(&file, held).unwrap();
            std::fs::write(file, raw).unwrap();
        }),
    );
    let register = |rng: &mut rand_chacha::ChaCha20Rng| {
        inspect().register(
            &invite,
            token,
            "Original Room".into(),
            "Original Member".into(),
            &fresh,
            rng,
        )
    };
    assert!(register(&mut rng).is_err());
    assert!(registration_parked.is_file());
    let saved_stage = std::fs::read(&stage).unwrap();
    assert_eq!(std::fs::read(&index).unwrap(), prior_index);
    std::fs::remove_file(&outcome_path).unwrap();
    std::fs::rename(&registration_parked, &outcome_path).unwrap();
    let original_member_outbox = *registration
        .member_outbox_context_data()
        .unwrap()
        .id()
        .as_bytes();
    let body = catcoms_replication::chat::Body::Create {
        text: "Original joined message".into(),
        reply_to: None,
    };
    let request = || crate::store::V2MemberSignRequest {
        token: [142; 32],
        channel: 42,
        body: &body,
        parents: &[],
        dependencies: &[],
    };
    let outbox_path = dir
        .path()
        .join("chat-owner-v2")
        .join("member-outbox-v1")
        .join(format!("mo-{}", hex::encode(original_member_outbox)));
    let mut saved = None;
    let mut founder_archived = None;
    let mut founder_reply = None;
    let mut founder_debt = None;
    if !recover_stage {
        let mut after = rng.clone();
        let row = register(&mut rng).unwrap();
        assert_eq!(row.id, 1);
        let view = inspect().select_registered(&fresh).unwrap();
        assert!(view.is_from_original_vault_cached(&vault));
        assert_eq!(view.id_data(), row.id);
        assert_eq!(view.name_data(), "Original Room");
        assert_eq!(view.display_name_data(), Some("Original Member"));
        assert_eq!(view.member_outbox_context_id_data(), original_member_outbox);
        assert_eq!(
            inspect()
                .lookup_saved_member(&view, &request(), &fresh, &mut rng)
                .unwrap(),
            None
        );
        assert!(!outbox_path.exists());
        assert!(view.same_original(&inspect().select_registered(&fresh).unwrap()));
        assert_eq!(register(&mut rng).unwrap(), row);
        assert_eq!(rng.next_u64(), after.next_u64());
        assert_eq!(std::fs::read(&index).unwrap(), saved_stage);
        assert!(!stage.exists());

        use crate::store::{
            MemberSaveJobReport, MemberSaveNativeAdmission, NativeTextJoinedMemberFinalPreparation,
            V2MemberSaveOutcome,
        };
        use catcoms_net::NativeChatMemberGossipEvent;
        use catcoms_replication::chat::{ActionState, V2OrdinaryReceiveStatus};
        let founder_text = catcoms_replication::chat::Body::Create {
            text: "Original founder message over TCP".into(),
            reply_to: None,
        };
        let founder_request = || V2MemberSignRequest {
            channel: request().channel,
            token: [141; 32],
            ..sign_request(&founder_text)
        };
        assert_eq!(
            founder_capture()
                .finish()
                .unwrap()
                .prepare_member_save(founder_request(), &fresh)
                .unwrap(),
            V2MemberSaveOutcome::Ready,
        );
        let pending = founder_capture()
            .finish_admission(founder_request().token)
            .unwrap();
        let commit = Arc::new(tokio::sync::Mutex::new(()));
        let (custody, _) = super::admission::custody(&commit, &fresh).await;
        assert_eq!(
            pending.try_admit(custody).unwrap().run(),
            MemberSaveJobReport::SavedLocal
        );
        let (cancel, signal) = tokio::sync::watch::channel(false);
        let cancelled = founder_capture()
            .prepare_founder_member_publication(RequestCancellation::new(signal, None))
            .unwrap()
            .unwrap();
        cancel.send_replace(true);
        assert!(matches!(
            cancelled.submit().await,
            Err(catcoms_sync::SyncError::Publication(
                catcoms_rt::PublishOnceError::Revoked
            ))
        ));
        let channel = request().channel;
        let mut receiver = inspect()
            .prepare_member_ingress(&view, channel, fresh.clone())
            .unwrap()
            .open()
            .await
            .unwrap();
        let mut ledger = inspect()
            .prepare_member_ledger(&view, channel, fresh.clone())
            .unwrap();
        let (receipt, original, founder_topic) =
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                let receipt = loop {
                    let prepared = founder_capture()
                        .prepare_founder_member_publication(fresh.clone())
                        .unwrap()
                        .expect("the cancelled transmission must leave the exact debt");
                    match prepared.submit().await {
                        Ok(Some(receipt)) => break receipt,
                        Err(catcoms_sync::SyncError::Publication(
                            catcoms_rt::PublishOnceError::NoPeers,
                        )) => {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                        _ => panic!("expected the founder debt to reach the native mesh"),
                    }
                };
                let Some(NativeChatMemberGossipEvent::Frame(frame)) = receiver.next().await else {
                    panic!("expected the founder's original on the joined actor");
                };
                let topic = frame.current_data().unwrap().0.clone();
                let (original, status) = inspect()
                    .receive_member_frame(&mut ledger, &frame, &fresh)
                    .unwrap();
                assert_eq!(
                    status,
                    V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
                );
                (receipt, original, topic)
            })
            .await
            .expect("bounded founder publication through both actual TCP actors");
        assert_eq!(
            founder_capture()
                .checkpoint_founder_member_publication(receipt, &fresh)
                .unwrap(),
            1
        );
        assert!(founder_capture()
            .prepare_founder_member_publication(fresh.clone())
            .unwrap()
            .is_none());
        assert!(inspect()
            .archive_received_member(&ledger, &original, &fresh, &mut rng)
            .unwrap()
            .is_some());
        founder_archived = Some(original.action_id_data());
        drop((receiver, ledger));
        let parent = original.action_id_data();
        let reply = catcoms_replication::chat::Body::Create {
            text: "Joined reply to the founder's saved original".into(),
            reply_to: Some(parent),
        };
        let reply_request = || V2MemberSignRequest {
            token: [145; 32],
            channel,
            body: &reply,
            parents: std::slice::from_ref(&parent),
            dependencies: &[],
        };
        assert_eq!(
            inspect()
                .prepare_member_save(&view, reply_request(), &fresh, &mut rng)
                .unwrap(),
            V2MemberSaveOutcome::Ready
        );
        let mut final_reply = Some(
            inspect()
                .finish_member_admission(view.clone(), reply_request().token, &fresh)
                .unwrap(),
        );
        let job = NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
            &mut final_reply,
            &fresh,
            MemberSaveNativeAdmission::new((), fresh.clone()),
        )
        .unwrap();
        assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
        let Some((reply_id, _)) = inspect()
            .lookup_saved_member(&view, &reply_request(), &fresh, &mut rng)
            .unwrap()
        else {
            panic!("the original joined reply must be saved");
        };
        drop(original);
        let mut founder_receiver = founder_capture()
            .prepare_founder_member_ingress(channel, fresh.clone())
            .unwrap()
            .open()
            .await
            .unwrap();
        let receipt = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let receipt = loop {
                let prepared = inspect()
                    .prepare_member_publication(&view, fresh.clone(), &mut rng)
                    .unwrap()
                    .expect("the original joined reply is owed");
                match prepared.submit().await {
                    Ok(Some(receipt)) => break receipt,
                    Err(catcoms_sync::SyncError::Publication(
                        catcoms_rt::PublishOnceError::NoPeers,
                    )) => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    _ => panic!("expected the reply to reach the founder mesh"),
                }
            };
            let Some(NativeChatMemberGossipEvent::Frame(frame)) = founder_receiver.next().await
            else {
                panic!("expected the original joined reply at the founder");
            };
            let received = founder_capture()
                .receive_founder_member(&founder_receiver, &frame)
                .unwrap()
                .unwrap();
            assert_eq!(received.action, reply_id);
            assert_eq!(received.projection_target, reply_id);
            assert_eq!(received.archived, Some(true));
            receipt
        })
        .await
        .expect("bounded joined reply over the original TCP connection");
        let projected = founder_capture()
            .read_founder_member_projection(channel, reply_id, &fresh)
            .unwrap()
            .expect("the founder loaded its own exact saved parent");
        assert!(projected
            .with_publication(|view| matches!(
                view, catcoms_replication::chat::GuardedProjection::Visible { text, .. }
                if text == "Joined reply to the founder's saved original"
            ))
            .unwrap());
        let parent_projection = founder_capture()
            .read_founder_member_projection(channel, parent, &fresh)
            .unwrap()
            .expect("the original parent was replayed for its semantics");
        assert!(parent_projection
            .with_publication(|view| matches!(
                view,
                catcoms_replication::chat::GuardedProjection::Held { .. }
            ))
            .unwrap());
        assert_eq!(
            inspect()
                .checkpoint_member_publication(receipt, &fresh, &mut rng)
                .unwrap(),
            1
        );
        drop((founder_receiver, projected, parent_projection));
        assert!(founder_capture()
            .read_founder_member_projection(channel, reply_id, &fresh)
            .unwrap()
            .is_none());
        let page = founder_capture()
            .archived_founder_action_page(channel, None, 8, &fresh)
            .unwrap()
            .unwrap();
        assert_eq!(page.actions(), &[reply_id]);
        let (end_read, signal) = tokio::sync::watch::channel(false);
        let read_scope = RequestCancellation::new(signal, None);
        let archived = founder_capture()
            .archived_founder_message_projection(channel, reply_id, &read_scope)
            .unwrap()
            .unwrap();
        assert!(archived
            .with_publication(|view| matches!(
                view, catcoms_replication::chat::GuardedProjection::Visible { text, .. }
                if text == "Joined reply to the founder's saved original"
            ))
            .unwrap());
        assert!(founder_capture()
            .archived_founder_message_projection(channel + 1, reply_id, &fresh)
            .is_err());
        end_read.send_replace(true);
        assert!(archived
            .with_publication(|_| panic!("retired founder archive body escaped"))
            .is_err());
        drop((archived, page));

        let mut provider = founder_capture()
            .attach_founder_dependency_provider(channel, fresh.clone())
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            for (wanted, expected) in [(parent, Some(parent)), (reply_id, None)] {
                let exchange = inspect()
                    .prepare_dependency_exchange(
                        &view,
                        channel,
                        invite.connected_peer_data(),
                        wanted,
                        fresh.clone(),
                        &mut rng,
                    )
                    .unwrap();
                let serve = async {
                    let incoming = provider
                        .next()
                        .await
                        .expect("the exact connected founder request");
                    founder_capture()
                        .serve_founder_dependency_request(&provider, incoming)
                        .unwrap();
                };
                let (answer, ()) = tokio::join!(exchange.receive(), serve);
                let opened = inspect()
                    .open_dependency_response(&view, answer.unwrap(), &fresh)
                    .unwrap();
                assert_eq!(opened.map(|original| original.action_id_data()), expected);
            }
        })
        .await
        .expect("the founder serves its own selected outbox and keeps the archive private");
        drop(provider);

        let nested_text = "Joined reply with a parent missed by the original receiver";
        let (stop_recovery, recovery_scope) = tokio::sync::watch::channel(false);
        let recovery_scope = RequestCancellation::new(recovery_scope, None);
        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            let mut receiver = loop {
                let prepared = founder_capture()
                    .prepare_founder_member_ingress(channel, recovery_scope.clone())
                    .unwrap();
                match prepared.open().await {
                    Ok(receiver) => break receiver,
                    Err(catcoms_net::NativeChatMemberGossipError::Busy) => {
                        tokio::task::yield_now().await
                    }
                    Err(error) => panic!("the original founder listener could not reopen: {error}"),
                }
            };
            let mut provider = inspect()
                .attach_dependency_provider(&view, fresh.clone())
                .unwrap();
            let mut previous = reply_id;
            let mut skipped = None;
            for (token, text) in [
                (147, "Grandparent missed by the founder"),
                (148, "Parent missed by the founder"),
                (149, nested_text),
            ] {
                let draft = catcoms_replication::chat::Body::Create {
                    text: text.into(),
                    reply_to: Some(previous),
                };
                let reply = || V2MemberSignRequest {
                    token: [token; 32],
                    channel,
                    body: &draft,
                    parents: std::slice::from_ref(&previous),
                    dependencies: &[],
                };
                assert_eq!(
                    inspect()
                        .prepare_member_save(&view, reply(), &fresh, &mut rng)
                        .unwrap(),
                    V2MemberSaveOutcome::Ready
                );
                let mut work = Some(
                    inspect()
                        .finish_member_admission(view.clone(), reply().token, &fresh)
                        .unwrap(),
                );
                let job = NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
                    &mut work,
                    &fresh,
                    MemberSaveNativeAdmission::new((), fresh.clone()),
                )
                .unwrap();
                assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
                let (action, ordinal) = inspect()
                    .lookup_saved_member(&view, &reply(), &fresh, &mut rng)
                    .unwrap()
                    .unwrap();
                let receipt = loop {
                    let prepared = inspect()
                        .prepare_member_publication(&view, fresh.clone(), &mut rng)
                        .unwrap()
                        .unwrap();
                    match prepared.submit().await {
                        Ok(Some(receipt)) => break receipt,
                        Err(catcoms_sync::SyncError::Publication(
                            catcoms_rt::PublishOnceError::NoPeers,
                        )) => {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                        _ => panic!("expected the joined original to reach the founder mesh"),
                    }
                };
                let original =
                    tokio::time::timeout(std::time::Duration::from_secs(12), receiver.next())
                        .await
                        .unwrap_or_else(|_| {
                            panic!("joined gossip original {token} did not arrive")
                        });
                let Some(NativeChatMemberGossipEvent::Frame(frame)) = original else {
                    panic!("the founder original did not arrive");
                };
                assert_eq!(
                    inspect()
                        .checkpoint_member_publication(receipt, &fresh, &mut rng)
                        .unwrap(),
                    ordinal + 1
                );
                if token != 149 {
                    if token == 147 {
                        skipped = Some(action);
                    }
                    previous = action;
                    continue;
                }
                let crate::store::NativeTextFounderFrameOutcome::NeedsDependency(exchange) =
                    founder_capture()
                        .receive_founder_member_with_recovery(&receiver, &frame)
                        .unwrap()
                else {
                    panic!("the founder should need the intentionally skipped parent");
                };
                assert!(founder_capture()
                    .archived_founder_message_projection(channel, previous, &fresh)
                    .unwrap()
                    .is_none());
                let mut next = Some(exchange);
                let mut responses = Vec::with_capacity(2);
                for round in 0..2 {
                    let exchange = next
                        .take()
                        .expect("both skipped signed originals require the original carrier");
                    let serve = async {
                        let incoming = provider
                            .next()
                            .await
                            .expect("the original joined provider receives the addressed request");
                        inspect()
                            .serve_dependency_request(
                                &view,
                                channel,
                                incoming,
                                fresh.clone(),
                                &mut rng,
                            )
                            .unwrap();
                    };
                    let (response, ()) =
                        tokio::time::timeout(std::time::Duration::from_secs(12), async {
                            tokio::join!(exchange.receive(), serve)
                        })
                        .await
                        .unwrap_or_else(|_| {
                            panic!("addressed founder dependency round {round} did not finish")
                        });
                    let response = response.unwrap();
                    match founder_capture()
                        .recover_founder_dependency_with_followup(&receiver, &response, true)
                        .unwrap()
                    {
                        crate::store::NativeTextFounderFrameOutcome::NeedsDependency(exchange)
                            if round == 0 =>
                        {
                            next = Some(exchange)
                        }
                        crate::store::NativeTextFounderFrameOutcome::Retained(receipt)
                            if round == 1 =>
                        {
                            assert_eq!(Some(receipt.action), skipped)
                        }
                        _ => panic!("unexpected outcome while restoring the two signed ancestors"),
                    }
                    responses.push(response);
                }
                assert!(founder_capture()
                    .recover_founder_dependency(&receiver, &responses[0])
                    .unwrap());
                let received = founder_capture()
                    .receive_founder_member(&receiver, &frame)
                    .unwrap()
                    .expect("the original child can now use the exact missing parent");
                assert_eq!(received.action, action);
                assert_eq!(received.archived, Some(true));
                let (topic, _, ciphertext) = frame.current_data().unwrap();
                founder_reply = Some((
                    [reply_id, skipped.unwrap(), previous, action],
                    action,
                    nested_text,
                    topic.clone(),
                    bytes::Bytes::copy_from_slice(ciphertext),
                ));
                stop_recovery.send_replace(true);
                assert!(founder_capture()
                    .recover_founder_dependency(&receiver, &responses[0])
                    .is_err());
            }
        })
        .await
        .expect("bounded addressed dependency and original reply over the real TCP connection");

        let deferred = super::body("Founder action saved before the process closed");
        let deferred_request = || V2MemberSignRequest {
            channel,
            token: [146; 32],
            ..sign_request(&deferred)
        };
        assert_eq!(
            founder_capture()
                .finish()
                .unwrap()
                .prepare_member_save(deferred_request(), &fresh)
                .unwrap(),
            V2MemberSaveOutcome::Ready,
        );
        let pending = founder_capture()
            .finish_admission(deferred_request().token)
            .unwrap();
        let commit = Arc::new(tokio::sync::Mutex::new(()));
        let (custody, _) = super::admission::custody(&commit, &fresh).await;
        assert_eq!(
            pending.try_admit(custody).unwrap().run(),
            MemberSaveJobReport::SavedLocal
        );
        let V2MemberSaveOutcome::Saved { action, ordinal } = founder_capture()
            .finish()
            .unwrap()
            .prepare_member_save(deferred_request(), &fresh)
            .unwrap()
        else {
            panic!("the exact founder debt must survive the restart");
        };
        founder_debt = Some((action, ordinal, founder_topic));
        assert_eq!(
            inspect()
                .prepare_member_save(&view, request(), &fresh, &mut rng)
                .unwrap(),
            V2MemberSaveOutcome::Ready
        );
        assert!(inspect()
            .lookup_saved_member(&view, &request(), &fresh, &mut rng)
            .is_err());
        let new_final = || {
            Some(
                inspect()
                    .finish_member_admission(view.clone(), request().token, &fresh)
                    .unwrap(),
            )
        };
        let mut final_work = new_final();
        let (_cancel, signal) = tokio::sync::watch::channel(true);
        assert!(
            NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
                &mut final_work,
                &fresh,
                MemberSaveNativeAdmission::new((), RequestCancellation::new(signal, None)),
            )
            .is_err()
        );
        assert!(final_work.is_some());
        let job = NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
            &mut final_work,
            &fresh,
            MemberSaveNativeAdmission::new((), fresh.clone()),
        )
        .unwrap();
        assert!(final_work.is_none());
        drop(job);
        let job = NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
            &mut new_final(),
            &fresh,
            MemberSaveNativeAdmission::new((), fresh.clone()),
        )
        .unwrap();
        assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
        let receipt = inspect()
            .prepare_member_save(&view, request(), &fresh, &mut rng)
            .unwrap();
        let V2MemberSaveOutcome::Saved { action, ordinal } = receipt else {
            panic!("original joined action was not saved")
        };
        saved = Some((action, ordinal));
        assert_eq!(
            inspect()
                .lookup_saved_member(&view, &request(), &fresh, &mut rng)
                .unwrap(),
            saved
        );
    }
    assert!(capture()
        .cover(&invite, anchor, token, after, &negative, &mut rng)
        .is_err());
    let mut retirement = original.prepare_retirement().unwrap();
    assert_eq!(
        retirement.try_retire().unwrap(),
        crate::store::NativeTextSourceRetirementStatus::Pending
    );
    drop(installation);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while retirement.try_retire().unwrap()
            == crate::store::NativeTextSourceRetirementStatus::Pending
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("original mounted TCP actor drained after the closed runtime");
    drop(retirement);
    drop(original);
    drop(vault);
    if recover_stage {
        std::fs::rename(&outcome_path, &registration_parked).unwrap();
        assert!(
            crate::store::NativeTextJoinedStorage::open_for_native_startup(
                dir.path(),
                SECRET,
                &mut rng,
                current_policy()
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&index).unwrap(), prior_index);
        assert_eq!(std::fs::read(&stage).unwrap(), saved_stage);
        std::fs::rename(&registration_parked, &outcome_path).unwrap();
    }
    let cold = crate::store::NativeTextJoinedStorage::open_for_native_startup(
        dir.path(),
        SECRET,
        &mut rng,
        current_policy(),
    )
    .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 1);
    assert_eq!(std::fs::read(&index).unwrap(), saved_stage);
    assert!(!stage.exists());
    assert!(cold.pending_join_data().unwrap() == Some(selected));
    assert_eq!(cold.recovered_join_outcome_data().unwrap(), Some(outcome));
    assert_eq!(
        cold.recovered_join_registration_data().unwrap(),
        Some(registration)
    );
    assert_eq!(
        cold.recovered_join_transport_data().unwrap(),
        Some(local_transport)
    );
    assert_eq!(std::fs::read(&path).unwrap(), ciphertext);
    let (foreign, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
    let cold = match cold.bind_recovered_join_runtime(
        foreign,
        rng.clone(),
        Box::new(catcoms_rt::SystemClock),
    ) {
        Err((original, _)) => original,
        Ok(_) => panic!("a foreign TCP actor cannot replace the covered joiner identity"),
    };
    let mut reopened = cold
        .bind_recovered_native_runtime(rng, Box::new(catcoms_rt::SystemClock), &fresh)
        .map_err(|(_, error)| error)
        .unwrap();
    assert_eq!(
        reopened.complete_recovery(&fresh).unwrap(),
        (head, own_add, epoch)
    );
    assert_eq!(reopened.local_peer_data(), Some(local_transport.0));
    use std::sync::atomic::{AtomicBool, AtomicU64};
    let (mounted_vault, mounted_sources) = reopened
        .into_recovered_native_sources(
            (
                Arc::new(AtomicU64::new(7)),
                Arc::new(AtomicU64::new(7)),
                Arc::new(AtomicBool::new(false)),
            ),
            budget.clone(),
            &policy(),
            &fresh,
        )
        .unwrap();
    assert!(mounted_sources.is_from_original_vault_cached(&mounted_vault));
    let recovered = mounted_sources
        .capture_joined_selection(command(&mounted_vault, &budget))
        .inspect_registration(&invite, token, &fresh)
        .unwrap();
    assert_eq!(recovered, Some(registration));
    let recovered_row =
        || mounted_sources.capture_joined_selection(command(&mounted_vault, &budget));
    let selection = recovered_row().select_registered(&fresh).unwrap();
    assert_eq!(selection.id_data(), 1);
    assert_eq!(selection.name_data(), "Original Room");
    assert_eq!(selection.display_name_data(), Some("Original Member"));
    assert_eq!(selection.request_data(), registration.request);
    assert_eq!(
        selection.member_outbox_context_id_data(),
        original_member_outbox
    );
    let mut history_rng = catcoms_rt::OsCryptoRng;
    let (retire_receiver, signal) = tokio::sync::watch::channel(false);
    let prepare_receiver = recovered_row()
        .prepare_member_ingress(
            &selection,
            request().channel,
            RequestCancellation::new(signal, None),
        )
        .unwrap();
    assert_eq!(
        recovered_row()
            .lookup_saved_member(&selection, &request(), &fresh, &mut history_rng)
            .unwrap(),
        saved,
        "the pending ingress must release native metadata before any queue or socket work"
    );
    let mut receiver = prepare_receiver.open().await.unwrap();
    retire_receiver.send_replace(true);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receiver.next())
            .await
            .unwrap()
            .is_none()
    );
    drop(receiver);
    assert_eq!(
        recovered_row()
            .lookup_saved_member(&selection, &request(), &fresh, &mut history_rng)
            .unwrap(),
        saved
    );
    if let Some((action, ordinal)) = saved {
        assert_eq!(
            recovered_row()
                .prepare_member_save(&selection, request(), &fresh, &mut history_rng)
                .unwrap(),
            crate::store::V2MemberSaveOutcome::Saved { action, ordinal }
        );
        let dependency = recovered_row()
            .read_member_dependency(
                &selection,
                request().channel,
                action,
                &fresh,
                &mut history_rng,
            )
            .unwrap()
            .unwrap();
        let delete_body = catcoms_replication::chat::Body::Delete { create: action };
        let next = crate::store::V2MemberSignRequest {
            token: [143; 32],
            channel: request().channel,
            body: &delete_body,
            parents: &[action],
            dependencies: &[crate::store::V2MemberDependency {
                action: dependency.action(),
                admission: dependency.receipt().admission,
            }],
        };
        let next_token = next.token;
        assert_eq!(
            recovered_row()
                .prepare_member_save(&selection, next, &fresh, &mut history_rng)
                .unwrap(),
            crate::store::V2MemberSaveOutcome::Ready
        );
        drop(dependency);
        let mut final_work = Some(
            recovered_row()
                .finish_member_admission(selection.clone(), next_token, &fresh)
                .unwrap(),
        );
        let job = crate::store::NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
            &mut final_work,
            &fresh,
            crate::store::MemberSaveNativeAdmission::new((), fresh.clone()),
        )
        .unwrap();
        assert_eq!(job.run(), crate::store::MemberSaveJobReport::SavedLocal);
        assert_eq!(
            recovered_row()
                .lookup_saved_member(&selection, &request(), &fresh, &mut history_rng)
                .unwrap(),
            saved
        );
        let head_path = outbox_path.join("head.bin");
        let head = std::fs::read(&head_path).unwrap();
        let (cancel, signal) = tokio::sync::watch::channel(false);
        let pending = recovered_row()
            .prepare_member_publication(
                &selection,
                RequestCancellation::new(signal, None),
                &mut history_rng,
            )
            .unwrap()
            .expect("the original saved action is still owed");
        assert_eq!(
            recovered_row()
                .lookup_saved_member(&selection, &request(), &fresh, &mut history_rng)
                .unwrap(),
            saved,
            "the prepared network attempt leaves the physical metadata slot available"
        );
        cancel.send_replace(true);
        assert!(matches!(
            pending.submit().await,
            Err(catcoms_sync::SyncError::Publication(
                catcoms_rt::PublishOnceError::Revoked
            ))
        ));
        let offline = recovered_row()
            .prepare_member_publication(&selection, fresh.clone(), &mut history_rng)
            .unwrap()
            .expect("cancellation preserves debt on the original action");
        assert!(matches!(
            offline.submit().await,
            Err(catcoms_sync::SyncError::Publication(
                catcoms_rt::PublishOnceError::NoPeers
            ))
        ));
        assert_eq!(std::fs::read(&head_path).unwrap(), head);
        assert!(
            recovered_row()
                .prepare_member_publication(&selection, fresh.clone(), &mut history_rng)
                .unwrap()
                .is_some(),
            "the no-peers result preserves debt for a fresh attempt"
        );
        // Relay the exact ciphertext back through a second authenticated TCP
        // mesh. The relay forwards bytes and never acquires the MLS signer.
        use catcoms_net::{NativeChatMemberGossipError, NativeChatMemberGossipEvent};
        use catcoms_replication::chat::{
            ActionState, GuardedProjection, ProjectionCertainty, V2OrdinaryReceiveStatus,
        };
        use catcoms_rt::{MeshTransport as _, PublishOnceError, PublishSubmission, TransportEvent};
        tokio::time::timeout(std::time::Duration::from_secs(25), async {
            let (retire, signal) = tokio::sync::watch::channel(false);
            let live = RequestCancellation::new(signal, None);
            let channel = request().channel;
            let founder_archived = founder_archived.expect("the joined row saved a founder original before close");
            let from_reopened_archive = recovered_row()
                .archived_message_projection(&selection, channel, founder_archived, live.clone(), &mut history_rng)
                .unwrap().unwrap();
            assert!(from_reopened_archive.with_publication(|view| matches!(
                view, GuardedProjection::Visible { text, certainty: ProjectionCertainty::Provisional, .. }
                if text == "Original founder message over TCP"
            )).unwrap());
            drop(from_reopened_archive);
            let (mut receiver, topic) = loop {
                let prepared = recovered_row()
                    .prepare_member_ingress(&selection, channel, live.clone())
                    .unwrap();
                let topic = prepared.topic_for_test();
                match prepared.open().await {
                    Ok(receiver) => break (receiver, topic),
                    Err(NativeChatMemberGossipError::Busy) => tokio::task::yield_now().await,
                    Err(error) => panic!("fresh original receiver attachment: {error}"),
                }
            };
            let mut ledger = recovered_row()
                .prepare_member_ledger(&selection, channel, live.clone())
                .unwrap();
            assert!(ledger.prepare_message_projection(action).is_err());
            let (relay, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
            relay.subscribe(topic.clone()).await.unwrap();
            let peer = recovered_row().mesh_peer_name_for_test().unwrap();
            let address = format!("/ip4/127.0.0.1/tcp/{}/p2p/{peer}", local_transport.1);
            assert_eq!(
                relay.dial_outcome(address.parse().unwrap()).await.unwrap(),
                catcoms_rt::DialSubmission::Submitted
            );
            relay.wait_for_peer_connected(local_transport.0).await.unwrap();
            let receipt = loop {
                let transmission = recovered_row()
                    .prepare_member_publication(&selection, live.clone(), &mut history_rng)
                    .unwrap()
                    .expect("the first original is still pending");
                match transmission.submit().await {
                    Ok(Some(receipt)) => break receipt,
                    Err(catcoms_sync::SyncError::Publication(PublishOnceError::NoPeers)) => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    _ => panic!("expected one acknowledgement for the first original"),
                }
            };
            let encrypted = loop {
                if let TransportEvent::Gossip { topic: arrived, from, data } =
                    relay.next_event().await.expect("the relay remains connected")
                {
                    assert_eq!(arrived, topic);
                    assert_eq!(from, local_transport.0);
                    break data;
                }
            };
            assert_eq!(
                relay.publish_once(topic.clone(), encrypted).await.unwrap(),
                PublishSubmission::Submitted
            );
            let Some(NativeChatMemberGossipEvent::Frame(frame)) = receiver.next().await else {
                panic!("expected the relayed original");
            };
            assert!(recovered_row().open_member_frame(&selection, channel + 1, &frame, &live).is_err());
            let (received, status) = recovered_row()
                .receive_member_frame(&mut ledger, &frame, &live)
                .unwrap();
            assert_eq!(received.action_id_data(), action);
            assert!(matches!(
                recovered_row().prepare_received_dependency_exchange(
                    &received, action, live.clone(), &mut history_rng,
                ),
                Err(crate::store::V2MemberSaveRefusal::Failed)
            ));
            assert_eq!(
                status,
                V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
            );
            let prepared_history = recovered_row()
                .prepare_shared_history_record(&ledger, &received, &live, &mut history_rng)
                .unwrap();
            assert_eq!(prepared_history.action, action);
            assert_ne!(*prepared_history.manifest.as_bytes(), [0; 32]);
            assert!(!prepared_history.bundle.is_empty());
            drop(prepared_history);
            assert!(recovered_row()
                .archive_received_member(&ledger, &received, &live, &mut history_rng)
                .unwrap().is_some());
            let page = recovered_row()
                .archived_action_page(&selection, channel, None, 2, &live)
                .unwrap().unwrap();
            let mut expected = [action, founder_archived];
            expected.sort_unstable();
            assert_eq!(page.actions(), &expected);
            assert!(!page.has_more());
            drop(page);
            let archived = recovered_row()
                .archived_message_projection(&selection, channel, action, live.clone(), &mut history_rng)
                .unwrap().expect("independent paid replay over the actual archive");
            assert!(archived.with_publication(|view| matches!(
                view, GuardedProjection::Visible { text, certainty: ProjectionCertainty::Provisional, .. }
                if text == "Original joined message"
            )).unwrap());
            assert!(recovered_row()
                .archived_message_projection(&selection, channel + 1, action, live.clone(), &mut history_rng)
                .is_err());
            let result = ledger.prepare_message_projection(action).unwrap();
            assert!(result.with_publication(|view| matches!(
                view, GuardedProjection::Visible { text, certainty: ProjectionCertainty::Provisional, .. }
                if text == "Original joined message"
            )).unwrap());
            let front = recovered_row()
                .checkpoint_member_publication(receipt, &live, &mut history_rng)
                .unwrap();
            assert_eq!(front, ordinal + 1);
            assert_ne!(std::fs::read(&head_path).unwrap(), head);
            let (retire_publication, publication_scope) = tokio::sync::watch::channel(false);
            let receipt = recovered_row()
                .prepare_member_publication(
                    &selection, RequestCancellation::new(publication_scope, None), &mut history_rng,
                )
                .unwrap().expect("exact archived Delete is owed")
                .submit().await.unwrap().unwrap();
            let deleted_wire = loop {
                if let TransportEvent::Gossip { topic: arrived, from, data } =
                    relay.next_event().await.expect("the relay remains connected")
                {
                    assert_eq!(arrived, topic);
                    assert_eq!(from, local_transport.0);
                    break data;
                }
            };
            assert_eq!(relay.publish_once(topic, deleted_wire).await.unwrap(), PublishSubmission::Submitted);
            let Some(NativeChatMemberGossipEvent::Frame(frame)) = receiver.next().await else {
                panic!("expected the relayed Delete");
            };
            let (deleted, status) = recovered_row()
                .receive_member_frame(&mut ledger, &frame, &live)
                .unwrap();
            let delete_id = deleted.action_id_data();
            assert_eq!(deleted.projection_target_data(), action);
            assert_eq!(status, V2OrdinaryReceiveStatus::Retained(ActionState::Valid));
            assert!(recovered_row()
                .archive_received_member(&ledger, &deleted, &live, &mut history_rng)
                .unwrap().is_some());
            assert!(archived.with_publication(|_| panic!("pre-delete archive projection escaped")).is_err());
            drop(archived);
            let archived = recovered_row()
                .archived_message_projection(&selection, channel, action, live.clone(), &mut history_rng)
                .unwrap().unwrap();
            assert!(archived.with_publication(|view| matches!(
                view, GuardedProjection::Deleted { create, certainty: ProjectionCertainty::Provisional, .. }
                if *create == action
            )).unwrap());
            let original_head = std::fs::read(&head_path).unwrap();
            retire_publication.send_replace(true);
            assert_eq!(
                recovered_row().checkpoint_member_publication(receipt, &live, &mut history_rng),
                Err(V2MemberSaveRefusal::Cancelled),
            );
            assert_eq!(std::fs::read(&head_path).unwrap(), original_head);
            let receipt = recovered_row()
                .prepare_member_publication(&selection, live.clone(), &mut history_rng)
                .unwrap().expect("the same Delete remains pending")
                .submit().await.unwrap().unwrap();
            assert_eq!(
                recovered_row().checkpoint_member_publication(receipt, &live, &mut history_rng).unwrap(),
                front + 1,
            );
            drop(deleted);
            let recovered_original = recovered_row()
                .read_member_dependency(&selection, channel, action, &live, &mut history_rng)
                .unwrap().expect("settled parents remain readable by exact ActionId");
            assert_eq!(recovered_original.receipt().action, action);
            drop(recovered_original);
            let mut replay = recovered_row()
                .prepare_member_ledger(&selection, channel, live.clone()).unwrap();
            assert!(replay.prepare_message_projection(action).is_err());
            assert_eq!(
                recovered_row().replay_saved_member_dependency(&mut replay, delete_id, &live, &mut history_rng).unwrap(),
                Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
            );
            assert!(replay.prepare_message_projection(action).unwrap().with_publication(|view| {
                matches!(view, GuardedProjection::Held { .. })
            }).unwrap());
            drop(replay);
            assert!(recovered_row().read_member_dependency(&selection, channel + 1, action, &live, &mut history_rng).is_err());
            let mut provider = recovered_row().attach_dependency_provider(&selection, live.clone()).unwrap();
            let malformed = relay.request_connected(
                local_transport.0,
                crate::store::critical_namespaces::native_registry::operational::governed::JOIN_PROTOCOL,
                bytes::Bytes::from_static(&[catcoms_rt::CHAT_V2_DEPENDENCY_KIND, 1]),
            );
            let serve = async {
                let request = provider.next().await.expect("the mounted exact dependency lane");
                assert!(recovered_row()
                    .serve_dependency_request(&selection, channel, request, live.clone(), &mut history_rng)
                    .is_err());
            };
            let (reply, ()) = tokio::join!(malformed, serve);
            assert!(matches!(reply, Err(catcoms_rt::TransportError::NoResponse)));
            retire.send_replace(true);
            assert!(provider.next().await.is_none());
            assert!(archived.with_publication(|_| panic!("retired archived original escaped")).is_err());
            assert!(result.with_publication(|_| panic!("retired original view escaped")).is_err());
            assert!(!received.is_for_original_row_cached(&selection));
        }).await.expect("bounded joined message and exact acknowledgement over native TCP");
    } else {
        assert!(recovered_row()
            .prepare_member_publication(&selection, fresh.clone(), &mut history_rng)
            .unwrap()
            .is_none());
        assert!(!outbox_path.exists());
        std::fs::create_dir(&outbox_path).unwrap();
        let damaged = outbox_path.join("head.bin");
        std::fs::write(&damaged, b"damaged").unwrap();
        assert!(recovered_row()
            .lookup_saved_member(&selection, &request(), &fresh, &mut history_rng)
            .is_err());
        assert!(recovered_row()
            .prepare_member_publication(&selection, fresh.clone(), &mut history_rng)
            .is_err());
        assert_eq!(std::fs::read(&damaged).unwrap(), b"damaged");
    }
    assert!(selection.is_from_original_vault_cached(&mounted_vault));
    assert!(selection.same_original(&recovered_row().select_registered(&fresh).unwrap()));
    let host = std::net::Ipv4Addr::LOCALHOST.into();
    assert!(recovered_row()
        .prepare_reconnect(&selection, &invite, [62; 32], host, fresh.clone())
        .is_err());
    let (cancel_contact, contact_scope) = tokio::sync::watch::channel(false);
    let reconnect = recovered_row()
        .prepare_reconnect(
            &selection,
            &invite,
            token,
            host,
            RequestCancellation::new(contact_scope, None),
        )
        .unwrap();
    cancel_contact.send_replace(true);
    assert!(reconnect.connect(&scheduler).await.is_err());
    let close_joined = || {
        selection.close_original_cached();
        assert!(!selection.is_current_cached());
        assert!(recovered_row().select_registered(&fresh).is_err());
    };
    {
        drop(receive);
        drop(serve);
        drop(requests);
        let mut retirement = owner.prepare_retirement().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            while retirement.try_retire().unwrap()
                == crate::store::NativeTextSourceRetirementStatus::Pending
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the original founder drained before the first Add was reopened");
        drop((retirement, row, owner, founder));
        let mut founder_rng = rand_chacha::ChaCha20Rng::seed_from_u64(0xe729);
        let cold = crate::store::NativeTextJoinedStorage::open_for_native_startup(
            founder_dir.path(),
            SECRET,
            &mut founder_rng,
            current_policy(),
        )
        .unwrap();
        let mut original = cold
            .prepare_recovered_founder()
            .unwrap_or_else(|(_, error)| panic!("the original founder refused: {error}"));
        assert_eq!(original.restore_first_add(&fresh).unwrap(), (head, epoch));
        assert!(original.prove_promoted_genesis(&fresh).is_err());
        let path =
            only_named(founder_dir.path(), "text.add.response.v2").with_file_name("head.bin");
        let parked = path.with_extension("held-after-restore");
        std::fs::rename(&path, &parked).unwrap();
        std::fs::copy(&parked, &path).unwrap();
        assert!(original.restore_first_add(&fresh).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(parked, path).unwrap();
        assert_eq!(original.restore_first_add(&fresh).unwrap(), (head, epoch));
        if let Some((mut expected, reply_id, reply_text, reply_topic, reply_wire)) = founder_reply {
            let channel = request().channel;
            let page = original
                .archived_action_page(channel, None, 8, &fresh)
                .unwrap()
                .unwrap();
            expected.sort_unstable();
            assert_eq!(page.actions(), &expected);
            let archived = original
                .archived_message_projection(channel, reply_id, &fresh, &mut founder_rng)
                .unwrap()
                .unwrap();
            assert!(archived
                .with_publication(|view| matches!(
                    view, catcoms_replication::chat::GuardedProjection::Visible { text, .. }
                    if text == reply_text
                ))
                .unwrap());
            assert!(original
                .archived_message_projection(channel + 1, reply_id, &fresh, &mut founder_rng)
                .is_err());
            let (action, ordinal, topic) = founder_debt.expect("unsent original founder action");
            let exact_body = super::body("Founder action saved before the process closed");
            let retry = V2MemberSignRequest {
                channel,
                token: [146; 32],
                body: &exact_body,
                parents: &[],
                dependencies: &[],
            };
            assert_eq!(
                original
                    .lookup_saved_member(&retry, &fresh, &mut founder_rng)
                    .unwrap(),
                Some((action, ordinal))
            );
            let altered_body = super::body("Changed text for the same token");
            let altered = V2MemberSignRequest {
                body: &altered_body,
                ..retry
            };
            assert_eq!(
                original.lookup_saved_member(&altered, &fresh, &mut founder_rng),
                Err(V2MemberSaveRefusal::Conflict)
            );
            let other_token = V2MemberSignRequest {
                token: [247; 32],
                ..retry
            };
            assert_eq!(
                original
                    .lookup_saved_member(&other_token, &fresh, &mut founder_rng)
                    .unwrap(),
                None
            );
            original.bind_original_tcp(&fresh).unwrap();
            recovered_row()
                .prepare_reconnect(&selection, &invite, token, host, fresh.clone())
                .unwrap()
                .connect(&scheduler)
                .await
                .unwrap();
            let lane = original.runtime_dependency_lane().unwrap();
            let mut requests = lane
                .try_attach(Arc::new(DependencyTestScope(fresh.clone())))
                .unwrap();
            let exchange = recovered_row()
                .prepare_dependency_exchange(
                    &selection,
                    channel,
                    invite.connected_peer_data(),
                    action,
                    fresh.clone(),
                    &mut history_rng,
                )
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(8), async {
                let serve = async {
                    let incoming = requests
                        .next()
                        .await
                        .expect("the reopened founder has the original connected request");
                    let reply = original
                        .prepare_dependency_reply(channel, &incoming, &fresh, &mut founder_rng)
                        .unwrap();
                    incoming
                        .respond(
                            reply.wire,
                            Arc::new(DependencyTestScope(
                                reply.observation.constrain(fresh.clone()),
                            )),
                            Arc::new(()),
                        )
                        .unwrap();
                    reply.fee
                };
                let (response, payment) = tokio::join!(exchange.receive(), serve);
                let exact = recovered_row()
                    .open_dependency_response(&selection, response.unwrap(), &fresh)
                    .unwrap()
                    .unwrap();
                assert_eq!(exact.action_id_data(), action);
                drop(payment);
            })
            .await
            .expect("the authentic recovered founder can answer the reopened joined member");
            drop(requests);
            close_joined();
            let (cancelled, signal) = tokio::sync::watch::channel(false);
            let pending = original
                .prepare_member_publication(
                    RequestCancellation::new(signal, None),
                    &mut founder_rng,
                )
                .unwrap()
                .unwrap();
            cancelled.send_replace(true);
            assert!(matches!(
                pending.submit().await,
                Err(catcoms_sync::SyncError::Publication(
                    catcoms_rt::PublishOnceError::Revoked
                )),
            ));
            let (probe, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
            let address = invite.direct_address_data(std::net::Ipv4Addr::LOCALHOST.into());
            let peer = invite.connected_peer_data();
            probe.subscribe(topic.clone()).await.unwrap();
            let (stop, scope) = tokio::sync::watch::channel(false);
            let mut cold_receiver = original
                .prepare_member_ingress(channel, RequestCancellation::new(scope, None))
                .unwrap()
                .open()
                .await
                .unwrap();
            assert_eq!(
                probe.dial_outcome(address.parse().unwrap()).await.unwrap(),
                catcoms_rt::DialSubmission::Submitted,
            );
            let (receipt, wire) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                probe.wait_for_peer_connected(peer).await.unwrap();
                let receipt = loop {
                    let debt = original
                        .prepare_member_publication(fresh.clone(), &mut founder_rng)
                        .unwrap()
                        .expect("cancellation leaves the same founder debt");
                    match debt.submit().await {
                        Ok(Some(accepted)) => break accepted,
                        Err(catcoms_sync::SyncError::Publication(
                            catcoms_rt::PublishOnceError::NoPeers,
                        )) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
                        _ => panic!("expected the restarted founder to publish its original debt"),
                    }
                };
                loop {
                    if let Some(catcoms_rt::TransportEvent::Gossip {
                        topic: received,
                        from,
                        data,
                    }) = probe.next_event().await
                    {
                        assert_eq!(received, topic);
                        assert_eq!(from, peer);
                        break (receipt, data);
                    }
                }
            })
            .await
            .expect("the original founder message reached the mesh after restart");
            assert_eq!(
                original
                    .opened_action_for_test(channel, &topic, &wire)
                    .unwrap(),
                action
            );
            assert_eq!(
                original
                    .checkpoint_member_publication(receipt, &fresh, &mut founder_rng)
                    .unwrap(),
                ordinal + 1
            );
            assert!(original
                .prepare_member_publication(fresh.clone(), &mut founder_rng)
                .unwrap()
                .is_none());
            assert_eq!(
                original
                    .lookup_saved_member(&retry, &fresh, &mut founder_rng)
                    .unwrap(),
                Some((action, ordinal))
            );
            assert_eq!(reply_topic, topic);
            let mut damaged = reply_wire.to_vec();
            *damaged.last_mut().unwrap() ^= 1;
            let (damaged, original_frame) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                let mut frames = Vec::new();
                for packet in [bytes::Bytes::from(damaged), reply_wire] {
                    loop {
                        match probe.publish_once(reply_topic.clone(), packet.clone()).await {
                            Ok(catcoms_rt::PublishSubmission::Submitted) => break,
                            Err(catcoms_rt::PublishOnceError::NoPeers) => {
                                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            }
                            _ => panic!("expected the TCP probe to carry the joined ciphertext"),
                        }
                    }
                    let Some(catcoms_net::NativeChatMemberGossipEvent::Frame(frame)) = cold_receiver.next().await
                    else {
                        panic!("expected the recovered founder's original listener to receive the frame");
                    };
                    frames.push(frame);
                }
                (frames.remove(0), frames.remove(0))
            }).await.expect("bounded ordinary subscriber after the founder restarted");
            assert!(original
                .receive_member_frame(&cold_receiver, &damaged, &mut founder_rng)
                .is_err());
            let received = original
                .receive_member_frame(&cold_receiver, &original_frame, &mut founder_rng)
                .unwrap()
                .expect("the recovered founder loaded its original signed reply and saved parent");
            assert_eq!(received.action, reply_id);
            assert_eq!(received.projection_target, reply_id);
            assert_eq!(received.archived, Some(true));
            let live = original
                .read_member_projection(channel, reply_id, &fresh)
                .unwrap()
                .unwrap();
            assert!(live
                .with_publication(|view| matches!(
                    view, catcoms_replication::chat::GuardedProjection::Visible { text, .. }
                    if text == reply_text
                ))
                .unwrap());
            stop.send_replace(true);
            assert!(original
                .receive_member_frame(&cold_receiver, &original_frame, &mut founder_rng)
                .is_err());
            assert!(live
                .with_publication(|_| panic!("a cancelled recovered receiver exposed a body"))
                .is_err());
            let vault = original
                .take_native_vault(
                    (
                        Arc::new(std::sync::atomic::AtomicU64::new(7)),
                        Arc::new(std::sync::atomic::AtomicU64::new(7)),
                        Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    ),
                    founder_budget.clone(),
                    &fresh,
                )
                .unwrap();
            let authentication = vault
                .prepare_native_authentication(founder_budget.prepare().unwrap())
                .verify(SECRET)
                .unwrap();
            assert!(authentication.is_for_original_vault(&vault));
            let command = command(&vault, &founder_budget);
            let pool = policy().pool;
            let key_owner = || {
                let payment = Arc::new(RuntimeMetadata {
                    pool: pool.clone(),
                    _debit: pool.try_reserve(ENTRY_BYTES).unwrap(),
                });
                catcoms_rt::NativeChatSourceOwner::for_trusted_source(&command.ui.command, payment)
                    .unwrap()
            };
            let native_owner = key_owner();
            original.sign_recovered_member_for_test(
                channel,
                &native_owner,
                &key_owner(),
                &command.ui.command,
                &fresh,
                &mut founder_rng,
            );
            let body = super::body("New founder message saved after the physical restart");
            let send = || V2MemberSignRequest {
                channel,
                token: [156; 32],
                body: &body,
                parents: &[],
                dependencies: &[],
            };
            let clock = catcoms_rt::ManualClock::new(10_124);
            let mut draws = rand_chacha::ChaCha20Rng::seed_from_u64(56031);
            let mut prepare_save =
                |original: &mut crate::store::NativeTextColdFounderSource,
                 rng: &mut rand_chacha::ChaCha20Rng| {
                    original.prepare_member_save(
                        send(),
                        &native_owner,
                        &command.ui.command,
                        &fresh,
                        (&clock, &mut draws),
                        rng,
                    )
                };
            assert_eq!(
                prepare_save(&mut original, &mut founder_rng).unwrap(),
                V2MemberSaveOutcome::Ready
            );
            let commit = Arc::new(tokio::sync::Mutex::new(()));
            let (stop_save, stop_signal) = tokio::sync::watch::channel(false);
            let cancelled = RequestCancellation::new(stop_signal, None);
            let first = original
                .finish_member_save(send().token, &native_owner, &command.ui.command, cancelled)
                .unwrap();
            let (custody, _) = super::admission::custody(&commit, &fresh).await;
            stop_save.send_replace(true);
            let result = original.admit_member_save::<rand_chacha::ChaCha20Rng>(&first, custody);
            assert!(matches!(&result, Err((_, V2MemberSaveRefusal::Cancelled))));
            drop(result);
            assert_eq!(
                prepare_save(&mut original, &mut founder_rng).unwrap(),
                V2MemberSaveOutcome::Ready
            );
            let final_save = original
                .finish_member_save(
                    send().token,
                    &native_owner,
                    &command.ui.command,
                    fresh.clone(),
                )
                .unwrap();
            let (custody, _) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                super::admission::custody(&commit, &fresh),
            )
            .await
            .expect("cancelled admission released the original commit guard");
            let job = original
                .admit_member_save::<rand_chacha::ChaCha20Rng>(&final_save, custody)
                .unwrap_or_else(|(_, error)| {
                    panic!("recovered founder save admission refused: {error:?}")
                });
            assert_eq!(job.run(), crate::store::MemberSaveJobReport::SavedLocal);
            let (action, ordinal) = original
                .lookup_saved_member(&send(), &fresh, &mut founder_rng)
                .unwrap()
                .unwrap();
            assert_eq!(
                prepare_save(&mut original, &mut founder_rng).unwrap(),
                V2MemberSaveOutcome::Saved { action, ordinal }
            );
            drop(prepare_save);
            native_owner.close();
            assert!(original
                .finish_member_save(
                    send().token,
                    &native_owner,
                    &command.ui.command,
                    fresh.clone()
                )
                .is_err());
            drop((command, vault));
            drop(original);
            assert!(archived
                .with_publication(|_| panic!("the closed restored founder published a body"))
                .is_err());
            return;
        }
        close_joined();
        original.bind_original_tcp(&fresh).unwrap();
        let mut requests = original
            .open_recovered_join_requests(fresh.clone())
            .unwrap();
        let (probe, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
        use catcoms_rt::MeshTransport as _;
        let address = invite.direct_address_data(std::net::Ipv4Addr::LOCALHOST.into());
        assert_eq!(
            probe.dial_outcome(address.parse().unwrap()).await.unwrap(),
            catcoms_rt::DialSubmission::Submitted,
        );
        let peer = invite.connected_peer_data();
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            probe.wait_for_peer_connected(peer).await.unwrap();
            let request = covered_request.expect("the original request reached the warm founder");
            let mut wrong = request.to_vec();
            *wrong.last_mut().unwrap() ^= 1;
            for (bytes, valid) in [(bytes::Bytes::from(wrong), false), (request, true)] {
                let send = probe.request_connected(
                    peer,
                    crate::store::critical_namespaces::native_registry::operational::governed::JOIN_PROTOCOL,
                    bytes,
                );
                let serve = async {
                    original.serve_recovered_join_request(requests.next().await.unwrap())
                };
                let (response, served) = tokio::join!(send, serve);
                if valid {
                    served.unwrap();
                    assert_eq!(response.unwrap().as_ref(), covered_delivery.as_slice());
                } else {
                    assert!(served.is_err());
                    assert!(matches!(response, Err(catcoms_rt::TransportError::NoResponse)));
                }
            }
        }).await.expect("the recovered founder served only the original exact delivery");
        drop(requests);
        let (member_topic, _, _topic_payment) =
            original.prepare_member_subscription(42, &fresh).unwrap();
        probe.subscribe(member_topic).await.unwrap();
        let expected = original.registration_data().0;
        let authorized = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (vault, sources, recovered_row, authentication) = original
            .into_recovered_native_sources(
                (
                    Arc::new(std::sync::atomic::AtomicU64::new(7)),
                    authorized.clone(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ),
                founder_budget.clone(),
                &policy(),
                SECRET,
                super::inputs(),
                &fresh,
            )
            .unwrap();
        assert_eq!(recovered_row.id_data(), expected.id);
        assert!(authentication.is_for_original_vault(&vault));
        assert!(recovered_row.is_recovered_from_original_vault_cached(&vault));
        authorized.store(7, std::sync::atomic::Ordering::SeqCst);
        let capture = || {
            sources.capture_member(
                &recovered_row,
                command(&vault, &founder_budget),
                sources.policy.prepare_member().unwrap(),
            )
        };
        let body = super::body("Saved through the reopened native Source");
        let request = || V2MemberSignRequest {
            channel: 42,
            token: [161; 32],
            body: &body,
            parents: &[],
            dependencies: &[],
        };
        assert_eq!(
            capture()
                .finish()
                .unwrap()
                .prepare_member_save(request(), &fresh)
                .unwrap(),
            V2MemberSaveOutcome::Ready
        );
        let admission = capture().finish_admission(request().token).unwrap();
        let commit = Arc::new(tokio::sync::Mutex::new(()));
        let (custody, _) = super::admission::custody(&commit, &fresh).await;
        assert_eq!(
            admission
                .try_admit(custody)
                .unwrap_or_else(|error| panic!("native cold founder save: {:?}", error.refusal()))
                .run(),
            crate::store::MemberSaveJobReport::SavedLocal,
        );
        assert!(matches!(
            capture()
                .finish()
                .unwrap()
                .prepare_member_save(request(), &fresh)
                .unwrap(),
            V2MemberSaveOutcome::Saved { .. },
        ));
        let (stop_publication, publication_signal) = tokio::sync::watch::channel(false);
        let publication_scope = RequestCancellation::new(publication_signal, None);
        let receipt = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let original = capture()
                    .prepare_founder_member_publication(publication_scope.clone())
                    .unwrap()
                    .expect("the new founder message remains in its original outbox");
                match original.submit().await {
                    Ok(Some(receipt)) => break receipt,
                    Err(catcoms_sync::SyncError::Publication(
                        catcoms_rt::PublishOnceError::NoPeers,
                    )) => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    _ => panic!("expected the recovered founder's original driver acknowledgement"),
                }
            }
        })
        .await
        .expect("the original recovered actor submitted to its subscribed peer");
        stop_publication.send_replace(true);
        assert_eq!(
            capture().checkpoint_founder_member_publication(receipt, &fresh),
            Err(V2MemberSaveRefusal::Cancelled),
        );
        let retry = capture()
            .prepare_founder_member_publication(fresh.clone())
            .unwrap()
            .expect("a cancelled acknowledgement leaves the same debt pending");
        let receipt = tokio::time::timeout(std::time::Duration::from_secs(10), retry.submit())
            .await
            .unwrap()
            .unwrap()
            .expect("the original actor acknowledged a fresh publication");
        assert!(capture()
            .checkpoint_founder_member_publication(receipt, &fresh)
            .is_ok());
        assert!(capture()
            .prepare_founder_member_publication(fresh.clone())
            .unwrap()
            .is_none());
        recovered_row.close_original_cached();
        assert!(capture().finish().is_err());
        let mut retirement = sources.prepare_retirement().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            while retirement.try_retire().unwrap()
                == crate::store::NativeTextSourceRetirementStatus::Pending
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the genuine recovered Source and TCP actor drained");
    }
}

#[derive(PartialEq, Eq)]
enum JoinCase {
    Staged,
    InterruptedInstall,
    NativeResponse,
    WarmRuntime,
    ColdPending,
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
        JoinCase::ColdPending => 0xe722,
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
    let native_joiner = matches!(
        case,
        JoinCase::NativeResponse | JoinCase::WarmRuntime | JoinCase::ColdPending
    );
    let request = (!native_joiner).then(|| {
        PreparedV2JoinRequest::prepare(
            catcoms_mls::MlsDevice::generate().unwrap(),
            &invite.binding().spec_data().anchor,
            invite.binding(),
            [59; 32],
            saved.expires_at_ms() - 1,
            DirectJoinLimits::new(512 * 1024, 512 * 1024, 64 * 1024).unwrap(),
        )
        .unwrap()
    });
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
        let anchor = &invite.binding().spec_data().anchor;
        let mut other = anchor.clone();
        other.state[0] ^= 1;
        let cover = |joined: &mut NativeTextJoinedStorage,
                     accepted,
                     token,
                     now,
                     rng: &mut rand_chacha::ChaCha20Rng| {
            joined.cover_accepted_first_join(&invite, accepted, token, now, &negative, rng)
        };
        let now = saved.expires_at_ms() - 1;
        assert!(cover(&mut joined, &other, [59; 32], now, &mut join_rng).is_err());
        assert!(cover(
            &mut joined,
            anchor,
            [59; 32],
            saved.expires_at_ms(),
            &mut join_rng
        )
        .is_err());
        assert!(joined.pending_join_data().unwrap().is_none());
        cover(&mut joined, anchor, [59; 32], now, &mut join_rng).unwrap();
        let original = joined.first_join_request_data().unwrap().to_vec();
        assert!(cover(&mut joined, anchor, [60; 32], now, &mut join_rng).is_err());
        cover(
            &mut joined,
            anchor,
            [59; 32],
            saved.expires_at_ms(),
            &mut join_rng,
        )
        .unwrap();
        assert_eq!(
            joined.first_join_request_data().unwrap(),
            original.as_slice()
        );
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
        tokio::task::yield_now().await;
        let scheduler = catcoms_discovery::EndpointDialScheduler::new(Default::default());
        assert!(joined
            .connect_accepted_invite_host(
                &invite,
                std::net::Ipv4Addr::UNSPECIFIED.into(),
                &client,
                &scheduler,
                negative.clone(),
            )
            .await
            .is_err());
        joined
            .connect_accepted_invite_host(
                &invite,
                std::net::Ipv4Addr::LOCALHOST.into(),
                &client,
                &scheduler,
                negative.clone(),
            )
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
        if case == JoinCase::ColdPending {
            let identity = *blake3::hash(&original_request).as_bytes();
            let pending = only_named(join_dir, "chat-critical-namespaces.joined-pending.bin");
            let raw = std::fs::read(&pending).unwrap();
            drop(joined_storage.take());
            let cold =
                NativeTextJoinedStorage::open(join_dir, SECRET, &mut join_rng, current_policy())
                    .unwrap();
            assert_eq!(cold.pending_join_data().unwrap().unwrap().0, identity);
            let (client, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
            cold.connect_accepted_invite_host(
                &invite,
                std::net::Ipv4Addr::LOCALHOST.into(),
                &client,
                &scheduler,
                negative.clone(),
            )
            .await
            .unwrap();
            let entry = join_dir.join(entry_name);
            std::fs::write(&entry, [1]).unwrap();
            assert!(tokio::time::timeout(
                std::time::Duration::from_secs(2),
                cold.replay_recovered_first_delivery(&invite, &client, negative.clone())
            )
            .await
            .unwrap()
            .is_err());
            std::fs::remove_file(&entry).unwrap();
            let (received, ()) = tokio::time::timeout(std::time::Duration::from_secs(8), async {
                tokio::join!(
                    cold.replay_recovered_first_delivery(&invite, &client, negative.clone()),
                    serve
                )
            })
            .await
            .unwrap();
            let original = accept(&original_request)
                .unwrap()
                .encode_delivery_data()
                .unwrap();
            assert_eq!(received.unwrap().as_ref(), original.as_slice());
            assert_eq!(std::fs::read(&pending).unwrap(), raw);
            assert!(!entry.exists());
            assert!(!join_dir.join(outcome_name).exists());
            assert!(cold.recovered_join_outcome_data().unwrap().is_none());
            return;
        }
        if case == JoinCase::WarmRuntime {
            use crate::store::persistence::governance_layout::joined::BEFORE_WELCOME_SDK;
            let mut runtime = joined_storage
                .take()
                .unwrap()
                .bind_first_join_runtime(client, join_rng, Box::new(catcoms_rt::SystemClock))
                .ok()
                .unwrap();
            let (cancel, pending) = tokio::sync::watch::channel(false);
            BEFORE_WELCOME_SDK.with(|slot| {
                assert!(slot
                    .replace(Some(Box::new(move || {
                        cancel.send_replace(true);
                    })))
                    .is_none());
            });
            let (interrupted, ()) =
                tokio::time::timeout(std::time::Duration::from_secs(8), async {
                    tokio::join!(
                        runtime
                            .complete_first_join(&invite, RequestCancellation::new(pending, None)),
                        serve
                    )
                })
                .await
                .unwrap();
            assert!(interrupted.is_err());
            let entry = only_named(join_dir, entry_name);
            let original_entry = std::fs::read(&entry).unwrap();
            assert!(find_named(join_dir, outcome_name).is_empty());
            let retry_serve = async {
                let Some(TransportEvent::Request {
                    data,
                    responder,
                    proto,
                    ..
                }) = requests.next().await
                else {
                    panic!("exact connected retry");
                };
                capture()
                    .serve_connected_join_request(proto, &data, responder, negative.clone())
                    .unwrap();
            };
            let (completed, ()) = tokio::time::timeout(std::time::Duration::from_secs(8), async {
                tokio::join!(
                    runtime.complete_first_join(&invite, negative.clone()),
                    retry_serve
                )
            })
            .await
            .unwrap();
            let (head, own, epoch) = completed.unwrap();
            assert_eq!(std::fs::read(&entry).unwrap(), original_entry);
            assert_eq!(epoch, 1);
            drop(requests);
            let saved = accept(&original_request).unwrap();
            let packet =
                catcoms_sync::V2JoinResponsePacketData::decode(saved.packet_bytes_data()).unwrap();
            let expected_proof = saved.governing_proof_bytes_data();
            let genesis = OwnerHeaderId::from_bytes(invite.binding().spec_data().anchor.header);
            assert_eq!(
                runtime
                    .first_selected_governing_proof(genesis, head, &negative)
                    .unwrap()
                    .as_ref(),
                expected_proof
            );
            assert!(runtime
                .first_selected_governing_proof(head, head, &negative)
                .is_err());
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
            let (transport, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
            let mut restored = cold
                .bind_recovered_join_runtime(
                    transport,
                    rand_chacha::ChaCha20Rng::seed_from_u64(0xe721),
                    Box::new(catcoms_rt::SystemClock),
                )
                .ok()
                .unwrap();
            assert_eq!(
                restored.complete_recovery(&negative).unwrap(),
                (head, own, epoch)
            );
            assert_eq!(
                restored
                    .first_selected_governing_proof(genesis, head, &negative)
                    .unwrap()
                    .as_ref(),
                expected_proof
            );
            let outcome = only_named(join_dir, outcome_name);
            let parked = outcome.with_extension("original-cold-outcome");
            let original_outcome = std::fs::read(&outcome).unwrap();
            std::fs::rename(&outcome, &parked).unwrap();
            std::fs::write(&outcome, original_outcome).unwrap();
            assert!(restored.complete_recovery(&negative).is_err());
            std::fs::remove_file(&outcome).unwrap();
            std::fs::rename(&parked, &outcome).unwrap();
            assert_eq!(
                restored.complete_recovery(&negative).unwrap(),
                (head, own, epoch)
            );
            let current = join_dir.join(crate::store::critical_namespaces::Slot::Current.name());
            let held_current = join_dir.join("original-cold-current");
            let original_current = std::fs::read(&current).unwrap();
            std::fs::rename(&current, &held_current).unwrap();
            std::fs::write(&current, original_current).unwrap();
            assert!(restored.complete_recovery(&negative).is_err());
            assert!(restored
                .first_selected_governing_proof(genesis, head, &negative)
                .is_err());
            std::fs::remove_file(&current).unwrap();
            std::fs::rename(&held_current, &current).unwrap();
            assert_eq!(
                restored.complete_recovery(&negative).unwrap(),
                (head, own, epoch)
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
        assert!(NativeTextJoinedStorage::open_for_native_startup(
            join_dir,
            SECRET,
            &mut join_rng,
            current_policy()
        )
        .is_err());
        assert_eq!(std::fs::read(&entry).unwrap(), exact_entry);
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
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(receiver, None);
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([51; 32], 60_000, &fresh)
        .is_ok());
    assert_eq!(std::fs::read(path).unwrap(), frame);
    let publication = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_publication([51; 32], 60_000, fresh.clone())
        .unwrap();
    assert!(publication.is_current_cached());
    let exact = publication.packet_bytes_data().to_vec();
    assert!(matches!(
        owner
            .capture_member(
                &row,
                command(&vault, &budget),
                owner.policy.prepare_member().unwrap()
            )
            .finish(),
        Err(V2MemberSaveRefusal::Busy)
    ));
    sender.send_replace(true);
    assert!(!publication.is_current_cached());
    drop(publication);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let retried = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_publication([51; 32], 60_000, RequestCancellation::new(receiver, None))
        .unwrap();
    assert_eq!(retried.packet_bytes_data(), exact.as_slice());
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
