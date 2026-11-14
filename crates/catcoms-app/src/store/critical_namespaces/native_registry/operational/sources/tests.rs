//! Real selected3 capture custody with synthetic UI atomics. These tests do
//! not establish Tauri authentication, source readiness or SDK acceptance.
use super::*;
use crate::store::{
    ArchiveFounderPolicy, GenesisSeedBudget, NativeUiInvocationBudget, V2TextHostPolicy,
};
use catcoms_rt::MemNetwork;
use catcoms_sync::owner_v2::FounderSnapshotLimits;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::{AtomicBool, AtomicU64};

thread_local! {
    static DEVICE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static AFTER_DEVICE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn device_generation() {
    DEVICE_CALLS.with(|calls| calls.set(calls.get() + 1));
}
pub(super) fn after_device_cache() {
    let hook = AFTER_DEVICE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
fn at_device_cache(hook: impl FnOnce() + 'static) {
    AFTER_DEVICE.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

fn founder() -> crate::store::FreshV2TextFounderPolicy {
    crate::store::FreshV2TextFounderPolicy::new(
        ArchiveFounderPolicy::new(
            FounderSnapshotLimits::new(
                8 * 1024 * 1024 + 64 * 1024,
                8 * 1024 * 1024,
                8 * 1024 * 1024,
            )
            .unwrap(),
            3,
            1024 * 1024 * 1024,
        )
        .unwrap(),
        GenesisSeedBudget::new(
            8 * 1024 * 1024 + 64 * 1024,
            8 * 1024 * 1024,
            8 * 1024 * 1024,
            1024 * 1024,
            16 * 1024 * 1024,
        )
        .unwrap(),
        V2TextHostPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap(),
    )
    .unwrap()
}
pub(super) fn policy() -> NativeTextSourcePolicy {
    NativeTextSourcePolicy::new(
        CriticalCatalogReadPool::new(NativeTextSourcePolicy::required_capacity(4).unwrap())
            .unwrap(),
        4,
        founder(),
    )
    .unwrap()
}
pub(super) fn selected(path: &std::path::Path) -> (NativeTextVault, NativeUiInvocationBudget) {
    selected_configured(path, true)
}
fn selected_configured(
    path: &std::path::Path,
    execution: bool,
) -> (NativeTextVault, NativeUiInvocationBudget) {
    let mut rng = ChaCha20Rng::seed_from_u64(0xcd01);
    let mut current =
        crate::store::critical_namespaces::native_registry::preparation::tests::current_policy();
    if execution {
        current.archive = current
            .archive
            .with_checkpoint_join_execution(2, 10_000)
            .unwrap();
    }
    let index = super::super::facade::upgrade::tests::index2_with_policy(path, &mut rng, current);
    let budget = NativeUiInvocationBudget::new(
        CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap(),
    )
    .unwrap();
    let mut upgrade = crate::store::NativeTextVaultUpgrade::prepare(
        index,
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicBool::new(false)),
        budget.clone(),
    )
    .unwrap();
    upgrade.retry(&mut rng).unwrap();
    (upgrade.into_vault().unwrap(), budget)
}

#[test]
fn native_sources_data_only_mount_cannot_retain_an_authoring_entry_or_generate_a_device() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_configured(dir.path(), false);
    let owner = sources(&vault);
    let before = DEVICE_CALLS.with(|calls| calls.get());
    assert!(matches!(
        owner.prepare_creation(command(&vault, &budget), request(1, "Original"), inputs()),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert!(owner.entries.lock().unwrap().is_empty());
    assert_eq!(DEVICE_CALLS.with(|calls| calls.get()), before);
}
pub(super) fn command(
    vault: &NativeTextVault,
    budget: &NativeUiInvocationBudget,
) -> NativeTextSourceCommand {
    let mut candidate = vault
        .capture_ui(7, budget.prepare().unwrap())
        .finish()
        .unwrap();
    let mut capture = Some(vault.admit_ui(&mut candidate).unwrap());
    vault.bind_native_source_command(&mut capture).unwrap()
}
pub(super) fn sources(vault: &NativeTextVault) -> NativeTextSources<MemNetwork, ChaCha20Rng> {
    match vault
        .capture_native_sources(policy().prepare().unwrap())
        .finish()
    {
        Ok(owner) => owner,
        Err(_) => panic!("genuine original collection refused"),
    }
}

#[test]
fn native_sources_policy_refuses_unbounded_or_unpaid_collection_before_capture() {
    for maximum in [0, 65, usize::MAX] {
        assert!(NativeTextSourcePolicy::new(
            CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap(),
            maximum,
            founder()
        )
        .is_err());
    }
    assert!(
        NativeTextSourcePolicy::new(CriticalCatalogReadPool::new(4096).unwrap(), 4, founder())
            .is_err()
    );
}

#[test]
fn native_sources_genuine_factory_binds_original_command_and_claims_once() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner.require_command(&command(&vault, &budget)).unwrap();
    assert!(vault
        .capture_native_sources(policy().prepare::<MemNetwork, ChaCha20Rng>().unwrap())
        .finish()
        .is_err());
    let auth = vault.prepare_native_authentication(budget.prepare().unwrap());
    assert!(auth
        .verify(crate::store::critical_namespaces::native_registry::preparation::tests::SECRET)
        .is_ok());
}

#[test]
fn native_sources_foreign_collection_cannot_use_equal_generation_command() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let (first, first_budget) = selected(a.path());
    let (second, second_budget) = selected(b.path());
    let first_sources = sources(&first);
    let second_sources = sources(&second);
    first_sources
        .require_command(&command(&first, &first_budget))
        .unwrap();
    assert!(first_sources
        .require_command(&command(&second, &second_budget))
        .is_err());
    second_sources
        .require_command(&command(&second, &second_budget))
        .unwrap();
}

#[test]
fn native_sources_retirement_cannot_be_reopened_while_facade_remains() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let old = command(&vault, &budget);
    owner.close_admission();
    assert!(owner.require_command(&old).is_err());
    assert!(vault
        .capture_native_sources(policy().prepare::<MemNetwork, ChaCha20Rng>().unwrap())
        .finish()
        .is_err());
    drop(owner);
    assert!(vault.bind_native_source_command(&mut None).is_err());
}

#[test]
fn native_sources_deferred_ui_retirement_withdraws_command_before_rt_counter_close() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let original = command(&vault, &budget);
    owner.require_command(&original).unwrap();
    // Keep the actual batch unretired. The original UI atomics still say7,
    // and its RT counter is open, but canonical admission has been withdrawn.
    let batch = vault
        .begin_ui_retirement(7)
        .unwrap()
        .into_batch()
        .expect("original UI owner");
    assert!(owner.require_command(&original).is_err());
    drop(batch);
}

#[test]
fn native_sources_invoice_counts_actual_generic_runtime_not_only_arc_pointer() {
    struct HugeRng {
        _storage: [u8; 1024 * 1024],
    }
    impl rand_core::CryptoRng for HugeRng {}
    impl rand_core::RngCore for HugeRng {
        fn next_u32(&mut self) -> u32 {
            panic!("unpaid source generated input")
        }
        fn next_u64(&mut self) -> u64 {
            panic!("unpaid source generated input")
        }
        fn fill_bytes(&mut self, _: &mut [u8]) {
            panic!("unpaid source generated input")
        }
        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
            panic!("unpaid source generated input")
        }
    }
    assert!(policy().prepare::<MemNetwork, HugeRng>().is_err());
    assert!(policy().prepare::<MemNetwork, ChaCha20Rng>().is_ok());
}

#[test]
fn native_sources_runtime_loan_restores_exact_body_and_retains_original_files_after_owner_drop() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, _) = selected(dir.path());
    let owner = sources(&vault);
    let payment = Arc::new(RuntimeMetadata {
        pool: owner.policy.pool.clone(),
        _debit: owner.policy.pool.try_reserve(ENTRY_BYTES).unwrap(),
    });
    let original = owner.work.for_paid_runtime(&payment).unwrap();
    let body: Runtime<MemNetwork, ChaCha20Rng> = Runtime {
        founder_policy: owner.policy.founder,
        request: Some(request(1, "Original retained request")),
        device_started: false,
        driver: None,
        device: None,
        transport: None,
        rng: Some(ChaCha20Rng::seed_from_u64(0xcd09)),
        group_rng: Some(ChaCha20Rng::seed_from_u64(0xcd19)),
        clock: None,
        network: None,
        work: original,
        _metadata: payment,
    };
    let cell = Arc::new(RuntimeCell {
        state: Mutex::new(RuntimeSlot {
            body: None,
            running: true,
            interrupted: false,
            poisoned: false,
        }),
        returned: tokio::sync::Notify::new(),
    });
    let loan = RuntimeLoan {
        body: Some(body),
        cell: cell.clone(),
        recorded: false,
    };
    drop(owner);
    drop(vault);
    assert!(!loan.body.as_ref().unwrap().work.is_live_cached());
    assert!(matches!(
        crate::store::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
            &mut ChaCha20Rng::seed_from_u64(0xcd0a),
            crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(
            ),
        ),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    ));
    drop(loan); // Actual unpolled whole loan: records before outer Keys may leave.
    {
        let state = cell.state.lock().unwrap();
        assert!(!state.running && state.interrupted && !state.poisoned);
        assert_eq!(
            state
                .body
                .as_ref()
                .unwrap()
                .request
                .as_ref()
                .unwrap()
                .settings
                .server_name,
            "Original retained request"
        );
    }
    drop(cell); // The actual final source Current/files drop HERE, not with the collection.
    drop(
        crate::store::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
            &mut ChaCha20Rng::seed_from_u64(0xcd0b),
            crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(
            ),
        )
        .unwrap(),
    );
}

pub(super) fn request(token: u8, name: &str) -> NativeTextCreationRequest {
    NativeTextCreationRequest::new(
        [token; 32],
        NativeTextCreationSettings {
            display_name: "Alice".into(),
            server_name: name.into(),
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            is_dm: false,
        },
    )
    .unwrap()
}
pub(super) fn inputs() -> NativeTextCreationInputs<ChaCha20Rng> {
    NativeTextCreationInputs::new(
        ChaCha20Rng::seed_from_u64(0xcd11),
        ChaCha20Rng::seed_from_u64(0xcd21),
        Box::new(catcoms_rt::ManualClock::new(100)),
    )
}

#[test]
fn native_sources_creation_names_match_downstream_byte_limits_and_fallback_before_digest() {
    let mut first = request(1, "");
    assert_eq!(first.settings.server_name, "Alice");
    assert_eq!(first.digest, request(1, "Alice").digest);
    first.settings.server_name = "é".repeat(128);
    first.settings.display_name = "x".repeat(256);
    let mut accepted = NativeTextCreationRequest::new([1; 32], first.settings).unwrap();
    assert_eq!(accepted.settings.server_name.len(), 256);
    accepted.settings.display_name.push('x');
    assert!(NativeTextCreationRequest::new([1; 32], accepted.settings).is_err());
    let mut too_long = request(1, "Alice").settings;
    too_long.server_name = "x".repeat(257);
    assert!(NativeTextCreationRequest::new([1; 32], too_long).is_err());
    // Validation has no source, RNG or device input and cannot admit any work.
}

#[test]
fn native_sources_actual_device_cache_survives_post_cache_unwind_without_second_generation() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let before = DEVICE_CALLS.with(|calls| calls.get());
    at_device_cache(|| panic!("after actual device output is cached"));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.prepare_device())).is_err()
    );
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let original = cell
        .state
        .lock()
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .device
        .as_ref()
        .unwrap()
        .public_key_bytes();
    assert_eq!(DEVICE_CALLS.with(|calls| calls.get()), before + 1);
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    assert_eq!(
        retry
            .loan
            .body
            .as_ref()
            .unwrap()
            .device
            .as_ref()
            .unwrap()
            .public_key_bytes(),
        original
    );
    retry.prepare_device().unwrap();
    assert_eq!(DEVICE_CALLS.with(|calls| calls.get()), before + 1);
    assert_eq!(
        cell.state
            .lock()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .device
            .as_ref()
            .unwrap()
            .public_key_bytes(),
        original
    );
}

#[test]
fn native_sources_late_source_close_keeps_actual_cached_device_but_refuses_progress() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let entry = owner.entries.lock().unwrap()[0].clone();
    let closing = entry.clone();
    let before = DEVICE_CALLS.with(|calls| calls.get());
    at_device_cache(move || closing.owner.close());
    assert_eq!(
        job.prepare_device(),
        Err(NativeTextCreationRefusal::Unavailable)
    );
    let state = entry.runtime.state.lock().unwrap();
    let body = state.body.as_ref().unwrap();
    assert!(body.device_started && body.device.is_some() && state.interrupted && !state.running);
    assert_eq!(DEVICE_CALLS.with(|calls| calls.get()), before + 1);
    assert!(owner.scope.is_open() && body.work.is_live_cached());
}

#[test]
fn native_sources_creation_retries_retain_the_actual_original_device_and_request() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    assert_eq!(
        job.prepare_device(),
        Ok(NativeTextCreationProgress::DevicePrepared)
    );
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let original = cell
        .state
        .lock()
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .device
        .as_ref()
        .unwrap()
        .device_id();
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    assert_eq!(
        retry
            .loan
            .body
            .as_ref()
            .unwrap()
            .device
            .as_ref()
            .unwrap()
            .device_id(),
        original
    );
    assert_eq!(
        retry.prepare_device(),
        Ok(NativeTextCreationProgress::DevicePrepared)
    );
    let state = cell.state.lock().unwrap();
    let body = state.body.as_ref().unwrap();
    assert!(body.device_started && !state.running && !state.interrupted);
    assert_eq!(
        body.request.as_ref().unwrap().settings.server_name,
        "Original"
    );
    assert_eq!(body.device.as_ref().unwrap().device_id(), original);
}

#[test]
fn native_sources_cached_device_refuses_after_only_its_original_source_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    let retry = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let original = retry
        .loan
        .body
        .as_ref()
        .unwrap()
        .device
        .as_ref()
        .unwrap()
        .device_id();
    let entry = owner.entries.lock().unwrap()[0].clone();
    entry.owner.close(); // No global, UI, Host or physical revocation.
    owner.require_command(&retry.command).unwrap();
    assert!(retry.loan.body.as_ref().unwrap().work.is_live_cached());
    assert_eq!(
        retry.prepare_device(),
        Err(NativeTextCreationRefusal::Unavailable)
    );
    assert_eq!(
        entry
            .runtime
            .state
            .lock()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .device
            .as_ref()
            .unwrap()
            .device_id(),
        original
    );
}

#[test]
fn native_sources_creation_conflict_and_busy_cannot_replace_or_generate_another_original() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    assert!(matches!(
        owner.prepare_creation(command(&vault, &budget), request(1, "Edited"), inputs()),
        Err(NativeTextCreationRefusal::Conflict)
    ));
    assert!(matches!(
        owner.prepare_creation(command(&vault, &budget), request(1, "Original"), inputs()),
        Err(NativeTextCreationRefusal::Busy)
    ));
    assert_eq!(owner.entries.lock().unwrap().len(), 1);
    drop(job); // An unrun loan returns the exact not-yet-generated body.
    let next = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    assert!(!next.loan.body.as_ref().unwrap().device_started);
    assert!(next.loan.body.as_ref().unwrap().device.is_none());
    assert_eq!(
        next.prepare_device(),
        Ok(NativeTextCreationProgress::DevicePrepared)
    );
}

#[test]
fn native_sources_creation_retirement_before_primitive_keeps_unstarted_device_without_revival() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let batch = vault.begin_ui_retirement(7).unwrap().into_batch().unwrap();
    assert_eq!(
        job.prepare_device(),
        Err(NativeTextCreationRefusal::Unavailable)
    );
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let state = cell.state.lock().unwrap();
    let body = state.body.as_ref().unwrap();
    assert!(!body.device_started && body.device.is_none());
    drop(state);
    drop(batch);
    owner.close_admission();
    assert!(vault
        .capture_native_sources(policy().prepare::<MemNetwork, ChaCha20Rng>().unwrap())
        .finish()
        .is_err());
}

#[test]
fn native_sources_creation_escaped_real_job_keeps_files_after_collection_and_facade_retire() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let cell = job.loan.cell.clone();
    drop(owner);
    drop(vault);
    assert_eq!(
        job.prepare_device(),
        Err(NativeTextCreationRefusal::Unavailable)
    );
    assert!(
        !cell
            .state
            .lock()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .device_started
    );
    assert!(matches!(
        crate::store::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
            &mut ChaCha20Rng::seed_from_u64(0xcd12),
            crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(
            ),
        ),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    ));
    drop(cell);
    drop(
        crate::store::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
            &mut ChaCha20Rng::seed_from_u64(0xcd13),
            crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(
            ),
        )
        .unwrap(),
    );
}
