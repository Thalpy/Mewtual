//! Genuine new-mount partial Init1 captures and exact task/owner handoff. These tests
//! construct no row, receipt, watch, saved directory, owner or installed success flag.
#![cfg(windows)]
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture_at_cut, ColdFixture, ColdFixtureCut,
};
use crate::store::owner_evidence::canonical::current_vacancy::pending_group_registration_fee_data_for_test;
use crate::store::persistence::v2_genesis_activation::initializing_repair::PreparedColdV2GenesisRepairCapture;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

fn captured(number: u64) -> (ColdFixture, Arc<ColdV2GenesisRepairSeed>) {
    let f = fixture_at_cut(number, ColdFixtureCut::ShortStage);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let seed = store
        .with_cold_v2_genesis_initializing_read(&source, |a| {
            let guard = store.lock_cold_initializing_genesis_group(a, &resources)?;
            capture.capture_initializing(&guard)
        })
        .unwrap();
    (f, seed)
}

#[test]
fn genuine_initializing_claim_drop_keeps_exact_paid_debt_and_cannot_renew_or_adopt_warm() {
    let (f, seed) = captured(0xc840);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let before = f.pool.charged_for_test();
    let mut task =
        PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed.clone())
            .unwrap();
    task.require_registered_cold_initializing_claim(store, &source, &seed)
        .unwrap();
    assert!(task.directory_output().is_err());
    assert!(task.saved_directory().is_err());
    assert!(task.retry_directory(store, &source).is_err());
    assert!(task.cold_current_head_data().is_none());
    assert!(
        PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed.clone())
            .is_err()
    );
    let original = task.observe_original();
    drop(task);
    assert!(f.pool.charged_for_test() > before);
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert!(PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed).is_err());
    drop(original);
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert!(store
        .capture_chat_admission(
            crate::store::ChatAdmissionKey::new(b"independent Init1 group", [0xc8; 32]).unwrap()
        )
        .is_ok());
}

#[test]
fn all_three_actual_initializing_registration_payments_fail_before_slot_and_retry_same_source() {
    const CAPACITY: usize = 1024 * 1024 * 1024;
    for phase in 0..3 {
        let (f, seed) = captured(0xc841 + phase);
        let store = f.layout.checked_store().unwrap();
        let source = f.source.borrow_source_data();
        let (backing, entry) = pending_group_registration_fee_data_for_test();
        let needed = INITIALIZING_TASK_FEE
            + if phase > 0 { backing } else { 0 }
            + if phase > 1 { entry } else { 0 };
        let before = f.pool.charged_for_test();
        let pressure = f
            .pool
            .try_reserve(CAPACITY - before - (needed - 1))
            .unwrap();
        let held = f.pool.charged_for_test();
        assert!(PreparedV2GenesisGroupNamespace::from_cold_initializing(
            store,
            &source,
            seed.clone()
        )
        .is_err());
        assert!(store.require_no_pending_v2_group(&f.group).is_ok());
        assert_eq!(f.pool.charged_for_test(), held);
        drop(pressure);
        assert_eq!(f.pool.charged_for_test(), before);
        let task =
            PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed.clone())
                .unwrap();
        task.require_registered_cold_initializing_claim(store, &source, &seed)
            .unwrap();
    }
}

#[test]
fn failed_original_prefix_proof_preserves_unset_once_lock_and_same_owner_can_handoff() {
    let (f, seed) = captured(0xc844);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed.clone())
            .unwrap();
    let mut scope = None;
    store
        .prepare_cold_v2_initializing_scope(&source, &task, seed.clone(), &mut scope)
        .unwrap();
    let scope = scope.as_ref().unwrap();
    let owner = scope.owner_identity();
    let nonce = scope.nonce();
    let path = seed.directory_data().0.join("pin.bin");
    let original = std::fs::read(&path).unwrap();
    let seed_path = seed.directory_data().0.join("activation.seed");
    let journal = std::fs::read(&seed_path).unwrap();
    let mut wrong = original.clone();
    wrong[0] ^= 1;
    std::fs::write(&path, wrong).unwrap();
    let charged = f.pool.charged_for_test();
    assert!(store
        .with_cold_initializing_admission(scope, &source, &task, |a| task
            .handoff_cold_initializing_owner(a))
        .is_err());
    assert!(task.cold_initializing().unwrap().owner.get().is_none());
    task.require_registered_cold_initializing_claim(store, &source, &seed)
        .unwrap();
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert_eq!(charged, f.pool.charged_for_test());
    assert_eq!(std::fs::read(&seed_path).unwrap(), journal);
    // Same bytes in SAME selected original File; no equal-replacement or new claim adoption.
    std::fs::write(&path, original).unwrap();
    let retired = store
        .with_cold_initializing_admission(scope, &source, &task, |a| {
            task.handoff_cold_initializing_owner(a)
        })
        .unwrap();
    drop(retired);
    assert!(task.cold_initializing().unwrap().owner.get().is_some());
    assert!(store.require_no_pending_v2_group(&f.group).is_ok());
    assert!(task
        .require_registered_cold_initializing_claim(store, &source, &seed)
        .is_err());
    assert!(Arc::ptr_eq(&owner, &scope.owner_identity()));
    assert_eq!(nonce, scope.nonce());
    // Repeated exact handoff remains idempotent; it cannot mint a second scope owner.
    let repeated = store
        .with_cold_initializing_admission(scope, &source, &task, |a| {
            task.handoff_cold_initializing_owner(a)
        })
        .unwrap();
    assert!(repeated.is_none());
}

#[test]
fn cancelled_real_initializing_owner_cannot_handoff_or_renew_pending_claim() {
    let (f, seed) = captured(0xc845);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed.clone())
            .unwrap();
    let mut scope = None;
    store
        .prepare_cold_v2_initializing_scope(&source, &task, seed.clone(), &mut scope)
        .unwrap();
    let scope = scope.as_ref().unwrap();
    let owner = scope.owner_identity();
    let nonce = scope.nonce();
    scope.cancel_original();
    assert!(store
        .with_cold_initializing_admission(scope, &source, &task, |a| task
            .handoff_cold_initializing_owner(a))
        .is_err());
    assert!(task.cold_initializing().unwrap().owner.get().is_none());
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert!(PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, seed).is_err());
    assert!(Arc::ptr_eq(&owner, &scope.owner_identity()));
    assert_eq!(nonce, scope.nonce());
}

#[test]
fn initializing_claim_and_warm_share_exact_sixteen_until_actual_owner_handoff() {
    use super::super::tests::{additional_saved_origin, archive, SECRET};
    use crate::store::critical_namespaces::{
        archive_current_namespace_codec as layout_codec, read_pool::CriticalCatalogReadPool,
    };
    use crate::store::persistence::v2_genesis_activation::PreparedV2GenesisActivation;
    use crate::store::{
        CanonicalFilenameInventoryPolicy, GenesisSeedBudget, PreparedV2CurrentLayoutStore,
        V2CurrentLayoutPolicy,
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let directory = tempfile::tempdir().unwrap();
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc850);
    drop(
        ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    // Generate every source with the real producer before ID3; original source limits and
    // full source/debit files survive each physical reopen, never a synthetic GroupId list.
    let mut origins = Vec::new();
    for index in 0..17u8 {
        origins.push(additional_saved_origin(
            directory.path(),
            &pool,
            &mut rng,
            0xc85000 + u64::from(index),
            index + 1,
        ));
    }
    let policy = || {
        V2CurrentLayoutPolicy::new(
            archive(&pool),
            CanonicalFilenameInventoryPolicy::new(40, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
            4 * layout_codec::MAX_DECLARATION_FRAME,
        )
        .unwrap()
    };
    let budget = GenesisSeedBudget::new(
        1024 * 1024,
        512 * 1024,
        512 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, policy()).unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    {
        let store = layout.checked_store().unwrap();
        let seed = store
            .prepare_genesis_seed(
                layout
                    .read_saved_bootstrap_source_data(&origins[0].0, origins[0].1)
                    .unwrap(),
                budget,
            )
            .unwrap();
        let source = seed.borrow_source_data();
        let mut task = layout.prepare_genesis_group_namespace(&source).unwrap();
        task.retry_directory(store, &source).unwrap();
        let mut activation =
            PreparedV2GenesisActivation::prepare(store, &source, &mut rng).unwrap();
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        let saved = store
            .with_vacant_v2_genesis_admission(&source, |admission| {
                activation.save_seed(&task.lock_vacant(admission)?)
            })
            .unwrap();
        let mut scope = None;
        store
            .reserve_v2_genesis_initializing(&source, &task, saved, &mut scope, &buffers, &mut rng)
            .unwrap();
        let scope = scope.as_mut().unwrap();
        let mut writer = crate::store::owner_evidence::canonical::v2_registration::PreparedV2WriterRegistration
            ::prepare(store, &source, &task, scope).unwrap();
        store
            .finish_v2_genesis_current(scope, &source, &task, &buffers, &mut rng, |admission, _| {
                let mut owner = task.lock_initializing_owner_namespace(admission)?;
                owner.install_writer(&mut writer)?;
                let _guard = owner.lock_archive_root_for_writer(&writer)?;
                // A real scope1/installed-writer producer cut before the first pin open.
                // Keep the genuine complete seed; no test row or file state is fabricated.
                Err(crate::store::critical_namespaces::refused())
            })
            .unwrap_err();
    }
    drop(layout);
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, policy()).unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    let mut seeds = Vec::new();
    for (group, signer) in &origins {
        seeds.push(
            store
                .prepare_genesis_seed(
                    layout
                        .read_saved_bootstrap_source_data(group, *signer)
                        .unwrap(),
                    budget,
                )
                .unwrap(),
        );
    }
    let source = seeds[0].borrow_source_data();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let head = store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            capture.capture_initializing(
                &store.lock_cold_initializing_genesis_group(admission, &resources)?,
            )
        })
        .unwrap();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_initializing(store, &source, head.clone())
            .unwrap();
    let mut warm = Vec::new();
    for seed in &seeds[1..16] {
        warm.push(
            layout
                .prepare_genesis_group_namespace(&seed.borrow_source_data())
                .unwrap(),
        );
    }
    assert!(layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .is_err());
    let mut scope = None;
    store
        .prepare_cold_v2_initializing_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    assert!(
        layout
            .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
            .is_err(),
        "actual new owner alone cannot retire the pending claim"
    );
    let retired = store
        .with_cold_initializing_admission(scope.as_ref().unwrap(), &source, &task, |admission| {
            task.handoff_cold_initializing_owner(admission)
        })
        .unwrap();
    assert!(retired.is_some());
    drop(retired);
    let next = layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .unwrap();
    task.require_registered_cold_initializing_claim(store, &source, &head)
        .unwrap_err();
    store
        .with_cold_initializing_admission(scope.as_ref().unwrap(), &source, &task, |admission| {
            task.require_cold_initializing_owned_handoff(admission)
        })
        .unwrap();
    drop(next);
    drop(warm);
}
