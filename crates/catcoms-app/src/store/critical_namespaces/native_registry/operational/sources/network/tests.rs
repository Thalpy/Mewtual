//! Genuine selected3, original device cuts and covered Pending/network IO.
//! No native ACL, transport, SDK or Active backend acceptance is claimed.
use super::super::tests::{command, inputs, request, selected, sources};
use super::*;

mod compatibility;

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_cached_active_still_requires_the_same_original_network_file() {
    for replace in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = prepared_mesh_founder(&vault, &budget, 0xe071 + 10 * u64::from(replace));
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .cover_founder_activation()
            .unwrap();
        let job = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let network = job.loan.body.as_ref().unwrap().network.as_ref().unwrap();
        assert!(
            network.active.is_some(),
            "exercise the cached completion path"
        );
        let file = dir
            .path()
            .join("servers")
            .join(format!("{}.native-net-v1", network.status.unwrap().id));
        let bytes = std::fs::read(&file).unwrap();
        let selected = std::fs::read(selected_index(dir.path())).unwrap();
        std::fs::rename(&file, file.with_extension("held-original")).unwrap();
        if replace {
            std::fs::write(&file, bytes).unwrap();
        }
        assert_eq!(
            job.cover_founder_activation(),
            Err(NativeTextCreationRefusal::Storage)
        );
        assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), selected);
        assert!(!index_stage(dir.path()).exists());
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_native_activation_cannot_use_data_path_or_foreign_original_dependency() {
    use crate::store::critical_namespaces::native_registry::preparation::tests::NeverRng;
    let first_dir = tempfile::tempdir().unwrap();
    let (first_vault, first_budget) = selected(first_dir.path());
    let first = prepared_mesh_founder(&first_vault, &first_budget, 0xe011);
    first
        .prepare_creation(
            command(&first_vault, &first_budget),
            request(1, "Original"),
            inputs(),
        )
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    let first_job = first
        .prepare_creation(
            command(&first_vault, &first_budget),
            request(1, "Original"),
            inputs(),
        )
        .unwrap();
    let foreign = first_job
        .loan
        .body
        .as_ref()
        .unwrap()
        .network
        .as_ref()
        .unwrap()
        .activation
        .as_ref()
        .unwrap()
        .clone();
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = prepared_mesh_founder(&vault, &budget, 0xe041);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let body = job.loan.body.as_ref().unwrap();
    let wanted = activation_request(body);
    let before = std::fs::read(selected_index(dir.path())).unwrap();
    assert!(body
        .work
        .activate_data_only_for_test(&wanted, &mut NeverRng)
        .is_err());
    assert!(body
        .work
        .activate_native_founder(&wanted, &foreign, &mut NeverRng)
        .is_err());
    assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), before);
    assert!(!index_stage(dir.path()).exists());
    drop(job);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
}

thread_local! {
    static FACTORY_INPUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TRANSPORT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub(super) fn after_factory_input_move() {
    if FACTORY_INPUT.with(|slot| slot.replace(false)) {
        panic!("actual factory input handoff interrupted");
    }
}
pub(super) fn transport_construction() {
    TRANSPORT_CALLS.with(|calls| calls.set(calls.get() + 1));
}
fn mesh_inputs(seed: u64) -> NativeTextCreationInputs<rand_chacha::ChaCha20Rng> {
    use rand_core::SeedableRng;
    NativeTextCreationInputs::new(
        rand_chacha::ChaCha20Rng::seed_from_u64(seed),
        rand_chacha::ChaCha20Rng::seed_from_u64(seed + 1),
        Box::new(catcoms_rt::ManualClock::new(100)),
    )
}

#[cfg(any(windows, target_os = "linux"))]
fn prepared_mesh_founder(
    vault: &NativeTextVault,
    budget: &crate::store::NativeUiInvocationBudget,
    seed: u64,
) -> NativeTextSources<catcoms_net::MeshService, rand_chacha::ChaCha20Rng> {
    let owner = vault
        .capture_native_sources(
            super::super::tests::policy()
                .prepare::<catcoms_net::MeshService, rand_chacha::ChaCha20Rng>()
                .unwrap(),
        )
        .finish()
        .unwrap();
    owner
        .prepare_creation(
            command(vault, budget),
            request(1, "Original"),
            mesh_inputs(seed),
        )
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_transport()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_founder()
        .unwrap();
    owner
}
fn selected_index(path: &std::path::Path) -> std::path::PathBuf {
    path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::native_registry::preparation::INDEX_NAME)
}
fn index_stage(path: &std::path::Path) -> std::path::PathBuf {
    path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::native_registry::preparation::INDEX_STAGE_NAME)
}
fn activation_request<T: MeshTransport, R: CryptoRngCore>(
    body: &Runtime<T, R>,
) -> founder::Activate {
    let network = body.network.as_ref().unwrap();
    let binding = body.driver.as_ref().unwrap().binding_data().unwrap();
    founder::Activate {
        id: network.status.unwrap().id,
        identity: network.identity,
        birth: network.pending.as_ref().unwrap().birth.clone(),
        binding: super::super::super::super::data::FounderBindingData {
            vault: *binding.vault_layout_data().0,
            layout: *binding.vault_layout_data().1,
            group: binding.group_data().to_vec(),
            signer: *binding.signer_data(),
            source: *binding.source_data(),
            profile: *binding.profile_data(),
            pin: *binding.pin_data(),
        },
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_genuine_activation_wal_cuts_retry_without_rng_and_keep_dependency_allocation(
) {
    use crate::store::critical_namespaces::native_registry::preparation::tests::NeverRng;
    use crate::store::persistence::native_registry::operational::{
        arm_fault, take_fault_seen, Fault,
    };
    for (number, cut) in [Fault::IndexStage, Fault::IndexRename, Fault::Index3Covered]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = prepared_mesh_founder(&vault, &budget, 0xdf01 + 10 * number as u64);
        arm_fault(cut);
        assert_eq!(
            owner
                .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
                .unwrap()
                .cover_founder_activation(),
            Err(NativeTextCreationRefusal::Storage)
        );
        assert!(take_fault_seen());
        let retry = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let body = retry.loan.body.as_ref().unwrap();
        let proof = body
            .network
            .as_ref()
            .unwrap()
            .activation
            .as_ref()
            .unwrap()
            .clone();
        let wanted = activation_request(body);
        let status = body
            .work
            .activate_native_founder(&wanted, &proof, &mut NeverRng)
            .unwrap();
        assert_eq!(status.state, crate::store::NativeCreationStateData::Founder);
        assert_eq!(
            body.work
                .activate_native_founder(&wanted, &proof, &mut NeverRng)
                .unwrap(),
            status
        );
        assert!(Arc::ptr_eq(
            body.network.as_ref().unwrap().activation.as_ref().unwrap(),
            &proof
        ));
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_cold_native_activation_commits_exact_stage_and_preserves_missing_dependency(
) {
    use crate::store::critical_namespaces::native_registry::preparation::tests::{
        current_policy, NeverRng, SECRET,
    };
    use crate::store::persistence::native_registry::operational::{
        arm_fault, take_fault_seen, Fault,
    };
    for missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = prepared_mesh_founder(&vault, &budget, 0xdf41 + 10 * u64::from(missing));
        let original = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let body = original.loan.body.as_ref().unwrap();
        let token = body.request.as_ref().unwrap().invocation;
        let digest = body.request.as_ref().unwrap().digest;
        let row = body.network.as_ref().unwrap().status.unwrap().id;
        drop(original);
        arm_fault(Fault::IndexStage);
        assert_eq!(
            owner
                .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
                .unwrap()
                .cover_founder_activation(),
            Err(NativeTextCreationRefusal::Storage)
        );
        assert!(take_fault_seen());
        let selected = std::fs::read(selected_index(dir.path())).unwrap();
        let stage = std::fs::read(index_stage(dir.path())).unwrap();
        drop(owner);
        drop(vault);
        if missing {
            std::fs::remove_file(
                dir.path()
                    .join("servers")
                    .join(format!("{row}.native-net-v1")),
            )
            .unwrap();
        }
        let cold = crate::store::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        );
        if missing {
            assert!(cold.is_err());
            assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), selected);
            assert_eq!(std::fs::read(index_stage(dir.path())).unwrap(), stage);
        } else {
            let cold = cold.unwrap();
            assert_eq!(
                cold.lookup_creation_data(token, digest)
                    .unwrap()
                    .unwrap()
                    .state,
                crate::store::NativeCreationStateData::Founder
            );
            assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), stage);
            assert!(!index_stage(dir.path()).exists());
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_cold_exposure_refuses_equal_network_replacement_after_actual_inspection() {
    use crate::store::critical_namespaces::native_registry::operational::upgrade_source::owning::{
        at_edge, Edge,
    };
    use crate::store::critical_namespaces::native_registry::preparation::tests::{
        current_policy, NeverRng, SECRET,
    };
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = prepared_mesh_founder(&vault, &budget, 0xdf71);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let row = job
        .loan
        .body
        .as_ref()
        .unwrap()
        .network
        .as_ref()
        .unwrap()
        .status
        .unwrap()
        .id;
    drop(job);
    drop(owner);
    drop(vault);
    let before = std::fs::read(selected_index(dir.path())).unwrap();
    let file = dir
        .path()
        .join("servers")
        .join(format!("{row}.native-net-v1"));
    let bytes = std::fs::read(&file).unwrap();
    at_edge(
        Edge::AfterInspection,
        Box::new(move || {
            std::fs::rename(&file, file.with_extension("held-original")).unwrap();
            std::fs::write(&file, &bytes).unwrap();
        }),
    );
    assert!(crate::store::NativeIndexStore::open_lifecycle3_for_test(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy()
    )
    .is_err());
    assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), before);
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_activation_refuses_missing_or_equal_replaced_original_network_before_any_index_effect(
) {
    for replace in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = prepared_mesh_founder(&vault, &budget, 0xde31 + 10 * u64::from(replace));
        let job = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let row = job
            .loan
            .body
            .as_ref()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .status
            .unwrap()
            .id;
        let file = dir
            .path()
            .join("servers")
            .join(format!("{row}.native-net-v1"));
        let before = std::fs::read(selected_index(dir.path())).unwrap();
        let original = std::fs::read(&file).unwrap();
        std::fs::rename(&file, file.with_extension("held-original")).unwrap();
        if replace {
            std::fs::write(&file, &original).unwrap();
        }
        assert_eq!(
            job.cover_founder_activation(),
            Err(NativeTextCreationRefusal::Storage)
        );
        assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), before);
        let job = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        assert!(job
            .loan
            .body
            .as_ref()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .active
            .is_none());
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_activation_rechecks_same_network_after_final_admission_and_preselection_hooks(
) {
    use crate::store::critical_namespaces::native_registry::operational::upgrade_source::owning::{
        at_edge, Edge,
    };
    use crate::store::persistence::native_registry::operational::{
        before_target_proof, RenameTarget,
    };
    for at_selection in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = prepared_mesh_founder(&vault, &budget, 0xde61 + 10 * u64::from(at_selection));
        let job = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let row = job
            .loan
            .body
            .as_ref()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .status
            .unwrap()
            .id;
        let file = dir
            .path()
            .join("servers")
            .join(format!("{row}.native-net-v1"));
        let original = std::fs::read(&file).unwrap();
        let before = std::fs::read(selected_index(dir.path())).unwrap();
        let replace = Box::new(move || {
            std::fs::rename(&file, file.with_extension("held-original")).unwrap();
            std::fs::write(&file, &original).unwrap();
        });
        if at_selection {
            before_target_proof(RenameTarget::Index, replace);
        } else {
            at_edge(Edge::BeforeMetadataAdmission, replace);
        }
        assert_eq!(
            job.cover_founder_activation(),
            Err(NativeTextCreationRefusal::Storage)
        );
        assert_eq!(std::fs::read(selected_index(dir.path())).unwrap(), before);
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_exact_minimum_native_invoice_reopens_genuine_pending_and_active_without_extra_credit(
) {
    use crate::store::critical_namespaces::native_registry::{
        lifecycle::preparation::Policy,
        preparation::tests::{NeverRng, SECRET},
    };
    for active in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = prepared_mesh_founder(&vault, &budget, 0xde91 + 10 * u64::from(active));
        let job = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let body = job.loan.body.as_ref().unwrap();
        let token = body.request.as_ref().unwrap().invocation;
        let digest = body.request.as_ref().unwrap().digest;
        let row = body.network.as_ref().unwrap().status.unwrap().id;
        if active {
            job.cover_founder_activation().unwrap();
        } else {
            drop(job);
        }
        drop(owner);
        drop(vault);
        if !active {
            std::fs::remove_file(
                dir.path()
                    .join("servers")
                    .join(format!("{row}.native-net-v1")),
            )
            .unwrap();
        }
        let current =
            crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(
            );
        let cold = crate::store::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current,
        )
        .unwrap();
        assert_eq!(
            cold.lookup_creation_data(token, digest)
                .unwrap()
                .unwrap()
                .state,
            if active {
                crate::store::NativeCreationStateData::Founder
            } else {
                crate::store::NativeCreationStateData::Pending
            }
        );
    }
    let required =
        crate::store::critical_namespaces::native_registry::lifecycle::preparation::PREPARATION_FEE;
    assert!(Policy::new(
        CriticalCatalogReadPool::new(required - 1).unwrap(),
        Policy::required_persistent_capacity()
    )
    .is_err());
    assert!(Policy::new(
        CriticalCatalogReadPool::new(required).unwrap(),
        Policy::required_persistent_capacity()
    )
    .is_ok());
    assert!(Policy::new(
        CriticalCatalogReadPool::new(required).unwrap(),
        Policy::required_persistent_capacity() - 1
    )
    .is_err());
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_factory_unwind_cannot_construct_a_second_transport_for_terminal_original() {
    use catcoms_net::MeshService;
    use rand_chacha::ChaCha20Rng;
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = vault
        .capture_native_sources(
            super::super::tests::policy()
                .prepare::<MeshService, ChaCha20Rng>()
                .unwrap(),
        )
        .finish()
        .unwrap();
    owner
        .prepare_creation(
            command(&vault, &budget),
            request(1, "Original"),
            mesh_inputs(0xde11),
        )
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_transport()
        .unwrap();
    let count = TRANSPORT_CALLS.with(|calls| calls.get());
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    FACTORY_INPUT.with(|slot| {
        assert!(!slot.get());
        slot.set(true);
    });
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.prepare_founder())).is_err()
    );
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    owner.require_command(&retry.command).unwrap();
    let body = retry.loan.body.as_ref().unwrap();
    assert!(
        body.network.as_ref().unwrap().factory_started
            && body.driver.is_none()
            && body.transport.is_none()
    );
    assert_eq!(
        retry.prepare_transport(),
        Err(NativeTextCreationRefusal::NetworkOutcomeUnknown)
    );
    assert_eq!(TRANSPORT_CALLS.with(|calls| calls.get()), count);
}

#[test]
fn native_sources_network_retry_keeps_seed_complete_key_and_exact_initial_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let network = retry.loan.body.as_ref().unwrap().network.as_ref().unwrap();
    let original_seed = Zeroizing::new(*network.seed);
    let original_public = network.public.clone().unwrap();
    let original_frame = network.frame.clone().unwrap();
    retry.prepare_network().unwrap();
    let next = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let retained = next.loan.body.as_ref().unwrap().network.as_ref().unwrap();
    assert_eq!(retained.seed.as_slice(), original_seed.as_slice());
    assert_eq!(retained.public.as_ref().unwrap(), &original_public);
    assert_eq!(retained.frame.as_ref().unwrap(), &original_frame);
    assert!(next.loan.body.as_ref().unwrap().transport.is_none());
}

#[test]
fn native_sources_real_encrypted_reservation_and_pending_bind_full_signer_and_frozen_port() {
    use catcoms_wire::Decoder;
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let body = job.loan.body.as_ref().unwrap();
    let device = body.device.as_ref().unwrap();
    let signer = device.public_key_bytes();
    let network = body.network.as_ref().unwrap();
    let plain = body
        .work
        .native_network_plain_for_test(network.frame.as_ref().unwrap())
        .unwrap();
    let mut input = Decoder::new(&plain);
    assert_eq!(input.get_bytes().unwrap(), TAG);
    assert_eq!(input.get_u16().unwrap(), 1);
    let context = body.work.native_network_context().unwrap();
    assert_eq!(input.get_bytes().unwrap(), context.0);
    assert_eq!(input.get_bytes().unwrap(), context.1);
    assert_eq!(
        input.get_bytes().unwrap(),
        body.request.as_ref().unwrap().invocation
    );
    assert_eq!(
        input.get_bytes().unwrap(),
        body.request.as_ref().unwrap().digest
    );
    let encoded_signer = input.get_bytes().unwrap();
    assert_eq!(encoded_signer, signer);
    assert_ne!(encoded_signer, device.device_id().as_bytes());
    assert_eq!(
        network.pending.as_ref().unwrap().birth.signer.as_slice(),
        signer
    );
    assert_eq!(input.get_bytes().unwrap(), network.identity);
    let public = seeded_transport_identity_data(&network.seed).unwrap();
    assert_eq!(input.get_bytes().unwrap(), public.commitment_data());
    assert_eq!(input.get_bytes().unwrap(), public.public_key_data());
    assert_eq!(input.get_bytes().unwrap(), network.seed.as_slice());
    assert_eq!(
        input.get_u16().unwrap(),
        crate::store::ServerNet::derived_port_for_seed(&network.seed)
    );
    assert_eq!(input.get_u64().unwrap(), 65_536);
    for _ in 0..3 {
        assert!(input.get_bytes().unwrap().is_empty());
    }
    input.finish().unwrap();
}

#[test]
fn native_sources_local_preseal_refusal_retries_original_seed_without_poisoning_unentered_seal() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    job.loan
        .body
        .as_ref()
        .unwrap()
        .work
        .refuse_next_network_seal_for_test();
    assert_eq!(
        job.prepare_network(),
        Err(NativeTextCreationRefusal::Storage)
    );
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let network = retry.loan.body.as_ref().unwrap().network.as_ref().unwrap();
    assert!(
        network.started && network.generated && !network.seal_started && network.frame.is_none()
    );
    let original = Zeroizing::new(*network.seed);
    retry.prepare_network().unwrap();
    let final_job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let network = final_job
        .loan
        .body
        .as_ref()
        .unwrap()
        .network
        .as_ref()
        .unwrap();
    assert_eq!(network.seed.as_slice(), original.as_slice());
    assert!(network.seal_started && network.frame.is_some());
}

#[test]
fn native_sources_covered_pending_writes_exact_frozen_record_and_never_installs_a_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let network = retry.loan.body.as_ref().unwrap().network.as_ref().unwrap();
    let status = network.status.unwrap();
    let original = network.frame.clone().unwrap();
    let file = dir
        .path()
        .join("servers")
        .join(format!("{}.native-net-v1", status.id));
    assert_eq!(std::fs::read(&file).unwrap(), original.as_slice());
    assert_eq!(status.state, crate::store::NativeCreationStateData::Pending);
    assert!(retry.loan.body.as_ref().unwrap().driver.is_none());
    assert!(retry.loan.body.as_ref().unwrap().transport.is_none());
    retry.cover_pending_network().unwrap();
    assert_eq!(std::fs::read(file).unwrap(), original.as_slice());
}

#[test]
fn native_sources_network_unknown_seed_outcome_is_retained_without_redraw() {
    use catcoms_rt::MemNetwork;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use rand_core::{CryptoRng, RngCore};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct InterruptedRng {
        calls: Arc<AtomicUsize>,
        inner: ChaCha20Rng,
    }
    impl CryptoRng for InterruptedRng {}
    impl RngCore for InterruptedRng {
        fn next_u32(&mut self) -> u32 {
            self.inner.next_u32()
        }
        fn next_u64(&mut self) -> u64 {
            self.inner.next_u64()
        }
        fn fill_bytes(&mut self, out: &mut [u8]) {
            self.try_fill_bytes(out).unwrap();
        }
        fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !out.is_empty() {
                out[0] = 42;
            }
            panic!("actual injected RNG interruption after partial seed output");
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let candidate = || {
        NativeTextCreationInputs::new(
            InterruptedRng {
                calls: calls.clone(),
                inner: ChaCha20Rng::seed_from_u64(44),
            },
            InterruptedRng {
                calls: calls.clone(),
                inner: ChaCha20Rng::seed_from_u64(45),
            },
            Box::new(catcoms_rt::ManualClock::new(100)),
        )
    };
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = vault
        .capture_native_sources(
            super::super::tests::policy()
                .prepare::<MemNetwork, InterruptedRng>()
                .unwrap(),
        )
        .finish()
        .unwrap();
    owner
        .prepare_creation(
            command(&vault, &budget),
            request(1, "Original"),
            candidate(),
        )
        .unwrap()
        .prepare_device()
        .unwrap();
    let job = owner
        .prepare_creation(
            command(&vault, &budget),
            request(1, "Original"),
            candidate(),
        )
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.prepare_network())).is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let retry = owner
        .prepare_creation(
            command(&vault, &budget),
            request(1, "Original"),
            candidate(),
        )
        .unwrap();
    let retained = retry.loan.body.as_ref().unwrap().network.as_ref().unwrap();
    assert!(retained.started && !retained.generated && retained.frame.is_none());
    assert_eq!(retained.seed[0], 42);
    assert_eq!(
        retry.prepare_network(),
        Err(NativeTextCreationRefusal::NetworkOutcomeUnknown)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn native_sources_network_actual_short_write_and_cover_errors_retry_same_inode_and_ciphertext() {
    use crate::store::persistence::native_registry::network::{arm, Fault};
    for fault in [Fault::PartialWrite, Fault::BeforeCover, Fault::AfterCover] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let owner = sources(&vault);
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .prepare_device()
            .unwrap();
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .prepare_network()
            .unwrap();
        arm(fault);
        assert_eq!(
            owner
                .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
                .unwrap()
                .cover_pending_network(),
            Err(NativeTextCreationRefusal::Storage)
        );
        let retry = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap();
        let network = retry.loan.body.as_ref().unwrap().network.as_ref().unwrap();
        let expected = network.frame.clone().unwrap();
        let file = dir
            .path()
            .join("servers")
            .join(format!("{}.native-net-v1", network.status.unwrap().id));
        let before = std::fs::File::open(&file).unwrap();
        let identity = catcoms_storage::kept::session_id(&before).unwrap();
        if fault == Fault::PartialWrite {
            assert_eq!(
                before.metadata().unwrap().len() as usize,
                expected.len() / 2
            );
        }
        retry.cover_pending_network().unwrap();
        let after = std::fs::File::open(&file).unwrap();
        assert_eq!(catcoms_storage::kept::session_id(&after).unwrap(), identity);
        assert_eq!(std::fs::read(file).unwrap(), expected.as_slice());
    }
}

#[test]
fn native_sources_network_equal_byte_replacement_cannot_become_original_retry_custody() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let network = job.loan.body.as_ref().unwrap().network.as_ref().unwrap();
    let file = dir
        .path()
        .join("servers")
        .join(format!("{}.native-net-v1", network.status.unwrap().id));
    let bytes = std::fs::read(&file).unwrap();
    std::fs::rename(&file, file.with_extension("held-original")).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    assert_eq!(
        job.cover_pending_network(),
        Err(NativeTextCreationRefusal::Storage)
    );
    assert_eq!(
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .cover_pending_network(),
        Err(NativeTextCreationRefusal::Storage)
    );
    assert_eq!(std::fs::read(file).unwrap(), bytes);
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn native_sources_actual_mesh_whole_native_founder_and_activation_keep_original_full_signer()
{
    use catcoms_net::MeshService;
    use rand_chacha::ChaCha20Rng;
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = vault
        .capture_native_sources(
            super::super::tests::policy()
                .prepare::<MeshService, ChaCha20Rng>()
                .unwrap(),
        )
        .finish()
        .unwrap();
    owner
        .prepare_creation(
            command(&vault, &budget),
            request(1, "Original"),
            mesh_inputs(0xde01),
        )
        .unwrap()
        .prepare_device()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let signer = job
        .loan
        .body
        .as_ref()
        .unwrap()
        .device
        .as_ref()
        .unwrap()
        .public_key_bytes();
    job.prepare_network().unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_transport()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let body = job.loan.body.as_ref().unwrap();
    let network = body.network.as_ref().unwrap();
    assert_eq!(
        network.actual_peer.as_deref(),
        Some(network.public.as_ref().unwrap().peer_id_data())
    );
    assert_eq!(
        body.transport.as_ref().unwrap().local_peer(),
        network.public.as_ref().unwrap().phase0_peer_data()
    );
    job.prepare_founder().unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let body = job.loan.body.as_ref().unwrap();
    assert!(
        body.device.is_none()
            && body.transport.is_none()
            && body.group_rng.is_none()
            && body.clock.is_none()
    );
    assert_eq!(
        body.driver
            .as_ref()
            .unwrap()
            .binding_data()
            .unwrap()
            .signer_data()
            .as_slice(),
        signer
    );
    job.cover_founder_activation().unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let body = job.loan.body.as_ref().unwrap();
    let network = body.network.as_ref().unwrap();
    assert_eq!(
        network.active.unwrap().state,
        crate::store::NativeCreationStateData::Founder
    );
    assert_eq!(
        body.work
            .lookup_creation(
                body.request.as_ref().unwrap().invocation,
                body.request.as_ref().unwrap().digest
            )
            .unwrap(),
        network.active
    );
    // Retried device observation follows the whole original ownership handoff;
    // it cannot generate a second key merely because the old input slot moved.
    job.prepare_device().unwrap();
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    // This is genuine Mesh/SDK/storage composition under synthetic UI atoms,
    // not Tauri ACL, remote delivery, listener or physical-network acceptance.
}
