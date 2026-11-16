//! Genuine warm SourceSaved/task/Seed/scope admissions for backing lifetime. No kernel
//! generation fixture, manufactured Head or status substitutes for the actual installer.
use super::*;
use crate::store::chat_admission::v2_current::V2CurrentPreparation;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::owner_evidence::canonical::v2_registration::PreparedV2WriterRegistration;
use crate::store::persistence::v2_genesis_activation::PreparedV2GenesisActivation;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

struct Initializing {
    task: PreparedV2GenesisGroupNamespace,
    activation: PreparedV2GenesisActivation,
    scope: V2CurrentPreparation,
}

/// All three returned owners are genuine constructor results. This intentionally stops
/// before preparing/installing any canonical writer so two callers can both see no map.
fn initialize(
    layout: &V2CurrentLayoutStore,
    seed: &RetainedBootstrapGenesisSeed,
    buffers: &CatalogReadReservation,
    rng: &mut ChaCha20Rng,
) -> Initializing {
    let store = layout.checked_store().unwrap();
    let source = seed.borrow_source_data();
    let mut task = layout.prepare_genesis_group_namespace(&source).unwrap();
    task.retry_directory(store, &source).unwrap();
    let mut activation = PreparedV2GenesisActivation::prepare(store, &source, rng).unwrap();
    let saved = store
        .with_vacant_v2_genesis_admission(&source, |admission| {
            activation.save_seed(&task.lock_vacant(admission)?)
        })
        .unwrap();
    let mut scope = None;
    store
        .reserve_v2_genesis_initializing(&source, &task, saved, &mut scope, buffers, rng)
        .unwrap();
    Initializing {
        task,
        activation,
        scope: scope.unwrap(),
    }
}

/// Actual scope1 installer, actual pin/head barriers and scope2 transition. The same
/// prepared instance is borrowed all the way through Root, never replaced after a drain.
fn finish(
    store: &ServerStore,
    seed: &RetainedBootstrapGenesisSeed,
    original: &mut Initializing,
    writer: &mut PreparedV2WriterRegistration,
    buffers: &CatalogReadReservation,
    rng: &mut ChaCha20Rng,
) {
    let source = seed.borrow_source_data();
    let Initializing {
        task,
        activation,
        scope,
    } = original;
    store
        .finish_v2_genesis_current(scope, &source, task, buffers, rng, |admission, _| {
            let mut owner = task.lock_initializing_owner_namespace(admission)?;
            owner.install_writer(writer)?;
            let guard = owner.lock_archive_root_for_writer(writer)?;
            activation.save_head(&guard)
        })
        .unwrap();
}

#[test]
fn warm_preinstall_drain_preserves_original_backing_until_genuine_install_and_current() {
    let f = fixture(0x3b70);
    let store = f.layout.checked_store().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3b70);
    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    let mut original = initialize(&f.layout, &f.seed, &buffers, &mut rng);
    let source = f.seed.borrow_source_data();
    let mut writer =
        PreparedV2WriterRegistration::prepare(store, &source, &original.task, &original.scope)
            .unwrap();
    let observed = writer.observe_original();
    let owner = original.scope.owner_identity();
    let nonce = original.scope.nonce();
    let charged = f.pool.charged_for_test();
    // This is the genuine caller epilogue after a failure BEFORE first installation. The
    // original non-evicting map does not exist yet; an expansion is still required work.
    writer.drain_unused_outside_writers();
    assert_eq!(
        f.pool.charged_for_test(),
        charged,
        "pre-install cleanup must retain its actual prepaid backing"
    );
    writer.require_same_observation(&observed).unwrap();
    finish(
        store,
        &f.seed,
        &mut original,
        &mut writer,
        &buffers,
        &mut rng,
    );
    writer.require_same_observation(&observed).unwrap();
    assert!(Arc::ptr_eq(&owner, &original.scope.owner_identity()));
    assert_eq!(nonce, original.scope.nonce());
    let head = original.scope.saved_head().unwrap();
    writer
        .require_same_observation(head.writer_observation())
        .unwrap();
    // Refund/drain precedes the FINAL actual physical/installed-generation proof.
    drop(buffers);
    writer.drain_unused_outside_writers();
    store
        .validate_v2_genesis_current(&original.scope, &source, &original.task)
        .unwrap();
}

#[test]
fn two_genuine_warm_preparations_refund_only_redundant_backing_after_each_actual_install() {
    let directory = tempfile::tempdir().unwrap();
    let pool = CriticalCatalogReadPool::new(CAPACITY).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3b71);
    drop(
        ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let first = additional_saved_origin(directory.path(), &pool, &mut rng, 0x3b7100, 0x71);
    let second = additional_saved_origin(directory.path(), &pool, &mut rng, 0x3b7101, 0x72);
    assert_ne!(
        first.0, second.0,
        "fixture needs two genuine independent groups"
    );
    let policy = V2CurrentLayoutPolicy::new(
        archive(&pool),
        CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * layout_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, policy).unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    let budget = GenesisSeedBudget::new(
        1024 * 1024,
        512 * 1024,
        512 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let seed_a = store
        .prepare_genesis_seed(
            layout
                .read_saved_bootstrap_source_data(&first.0, first.1)
                .unwrap(),
            budget,
        )
        .unwrap();
    let seed_b = store
        .prepare_genesis_seed(
            layout
                .read_saved_bootstrap_source_data(&second.0, second.1)
                .unwrap(),
            budget,
        )
        .unwrap();
    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    let mut a = initialize(&layout, &seed_a, &buffers, &mut rng);
    let mut b = initialize(&layout, &seed_b, &buffers, &mut rng);
    // Neither initialization installs a writer. Both REAL preparations must therefore pay
    // their own backing; the later winner does not silently borrow the earlier allocation.
    let before_a = pool.charged_for_test();
    let mut writer_a = PreparedV2WriterRegistration::prepare(
        store,
        &seed_a.borrow_source_data(),
        &a.task,
        &a.scope,
    )
    .unwrap();
    let cost_a = pool.charged_for_test() - before_a;
    let before_b = pool.charged_for_test();
    let mut writer_b = PreparedV2WriterRegistration::prepare(
        store,
        &seed_b.borrow_source_data(),
        &b.task,
        &b.scope,
    )
    .unwrap();
    let cost_b = pool.charged_for_test() - before_b;
    assert_eq!(cost_a, cost_b);
    // Frozen actual generation invoice in v2_registration.rs. This pins the requested
    // class, not allocator capacity/RSS. The measured remainder is the actual buffer fee.
    let backing = cost_b.checked_sub(8 * 1024).unwrap();
    assert!(backing > 0);
    let observed_a = writer_a.observe_original();
    let observed_b = writer_b.observe_original();
    let owner_a = a.scope.owner_identity();
    let owner_b = b.scope.owner_identity();
    let nonce_a = a.scope.nonce();
    let nonce_b = b.scope.nonce();
    let held = pool.charged_for_test();
    writer_a.drain_unused_outside_writers();
    writer_b.drain_unused_outside_writers();
    assert_eq!(
        pool.charged_for_test(),
        held,
        "both unstarted originals still owe backing"
    );
    finish(store, &seed_a, &mut a, &mut writer_a, &buffers, &mut rng);
    let before_first_drain = pool.charged_for_test();
    writer_a.drain_unused_outside_writers();
    assert_eq!(
        pool.charged_for_test(),
        before_first_drain,
        "first backing transferred into the actual Runtime, not refunded"
    );
    // The original second writer uses that SAME mounted Runtime and retains its separately
    // paid expansion UNTIL successful insertion makes it actually redundant.
    finish(store, &seed_b, &mut b, &mut writer_b, &buffers, &mut rng);
    let before_second_drain = pool.charged_for_test();
    writer_b.drain_unused_outside_writers();
    assert_eq!(before_second_drain - pool.charged_for_test(), backing);
    writer_a.require_same_observation(&observed_a).unwrap();
    writer_b.require_same_observation(&observed_b).unwrap();
    assert!(Arc::ptr_eq(&owner_a, &a.scope.owner_identity()));
    assert!(Arc::ptr_eq(&owner_b, &b.scope.owner_identity()));
    assert_eq!(nonce_a, a.scope.nonce());
    assert_eq!(nonce_b, b.scope.nonce());
    writer_a
        .require_same_observation(a.scope.saved_head().unwrap().writer_observation())
        .unwrap();
    writer_b
        .require_same_observation(b.scope.saved_head().unwrap().writer_observation())
        .unwrap();
    drop(buffers);
    store
        .validate_v2_genesis_current(&a.scope, &seed_a.borrow_source_data(), &a.task)
        .unwrap();
    store
        .validate_v2_genesis_current(&b.scope, &seed_b.borrow_source_data(), &b.task)
        .unwrap();
}
