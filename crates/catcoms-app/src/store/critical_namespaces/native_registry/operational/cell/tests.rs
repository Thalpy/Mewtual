//! Real opaque mounts and real fixed-name IO. Inputs here are storage fixtures,
//! not native credentials, a network reservation or SDK activation evidence.
use super::super::super::preparation::tests::{
    current_policy, initialize, records, NeverRng, SECRET,
};
use super::*;
use crate::store::{NativeRegistryInstallation, NativeRegistryPolicy, NativeRegistryStore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{fs, path::Path};

fn mount(path: &Path, rng: &mut ChaCha20Rng) -> mount::NativeIndexStore {
    initialize(path, Some(&records()), rng);
    let old_policy = NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut old =
        NativeRegistryInstallation::prepare(path, SECRET, rng, current_policy(), old_policy)
            .unwrap();
    old.retry().unwrap();
    let old: NativeRegistryStore = old.into_native_registry().unwrap();
    let mut upgrade = NativeIndexUpgrade::prepare(old, policy(), rng).unwrap();
    upgrade.retry().unwrap();
    upgrade.into_index_store().unwrap()
}
fn policy() -> NativeIndexPolicy {
    NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity(),
    )
    .unwrap()
}
fn input() -> PendingBirth {
    PendingBirth {
        identity: [91; 32],
        name: "original request".into(),
        is_dm: false,
        birth: data::Birth {
            invocation: [92; 32],
            request: [93; 32],
            signer: [94; 32],
            network: [95; 32],
        },
        network_frame: Zeroizing::new(vec![96; 40]),
    }
}
fn file(path: &Path, name: &str) -> std::path::PathBuf {
    path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(name)
}
fn finish(
    physical: &mount::NativeIndexPhysical,
    input: &PendingBirth,
) -> mount::NativeCreationStatusData {
    let (status, debt) = physical
        .finish_pending_birth(input.birth.invocation, input.birth.request)
        .unwrap();
    // Production must likewise destroy this whole debt outside native/Store
    // guards, rather than dropping its files and last invoice under a writer.
    drop(debt);
    status
}

#[test]
fn native_index2_cell_keeps_same_frame_and_join_debt_at_each_real_write_cut() {
    for (number, cut) in [io::Fault::IndexStage, io::Fault::IndexRename]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc200 + number as u64);
        let mounted = mount(dir.path(), &mut rng);
        let input = input();
        let expected = mounted
            .physical
            .prepare_pending_birth(&input, &mut rng)
            .unwrap();
        assert_eq!(expected.state, mount::NativeCreationStateData::Pending);
        let original_frame = mounted
            .physical
            .cell
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .frame
            .clone();
        io::arm_fault(cut);
        assert!(mounted
            .physical
            .finish_pending_birth(input.birth.invocation, input.birth.request)
            .is_err());
        assert!(io::take_fault_seen());
        let store = mounted.physical.current.cached_store().unwrap();
        assert!(store.require_native_index_idle_cached().is_err());
        assert!(mounted.row_count_data().is_err());
        // A new invocation sees Busy; same-token edited input sees Conflict.
        let mut conflicting = self::input();
        conflicting.birth.request[0] ^= 1;
        assert!(mounted
            .physical
            .prepare_pending_birth(&conflicting, &mut NeverRng)
            .is_err());
        conflicting = self::input();
        conflicting.network_frame[0] ^= 1;
        assert!(mounted
            .physical
            .prepare_pending_birth(&conflicting, &mut NeverRng)
            .is_err());
        assert_eq!(
            mounted
                .physical
                .prepare_pending_birth(&input, &mut NeverRng)
                .unwrap(),
            expected
        );
        assert_eq!(finish(&mounted.physical, &input), expected);
        assert!(store.require_native_index_idle_cached().is_ok());
        assert_eq!(
            fs::read(file(
                dir.path(),
                super::super::super::preparation::INDEX_NAME
            ))
            .unwrap(),
            *original_frame
        );
        assert_eq!(mounted.row_count_data().unwrap(), 2);
        // A completed historical hit cannot start another write or SDK.
        assert_eq!(
            mounted
                .physical
                .prepare_pending_birth(&input, &mut NeverRng)
                .unwrap(),
            expected
        );
        assert!(mounted
            .physical
            .finish_pending_birth(input.birth.invocation, input.birth.request)
            .is_err());
    }
}

#[test]
fn native_index2_cell_caller_loss_before_first_write_cannot_discard_the_claim() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc210);
    let mounted = mount(dir.path(), &mut rng);
    let input = input();
    let original = fs::read(file(
        dir.path(),
        super::super::super::preparation::INDEX_NAME,
    ))
    .unwrap();
    mounted
        .physical
        .prepare_pending_birth(&input, &mut rng)
        .unwrap();
    // Nothing was written yet, but the cell, not an IPC preparation, owns debt.
    assert_eq!(
        fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME
        ))
        .unwrap(),
        original
    );
    assert!(!mounted.physical.handler.gate.idle());
    assert!(mounted
        .lookup_creation_data(input.birth.invocation, input.birth.request)
        .is_err());
    finish(&mounted.physical, &input);
    assert_eq!(mounted.row_count_data().unwrap(), 2);
}

#[test]
fn native_index2_cell_closed_owner_finishes_only_its_original_admitted_birth() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc211);
    let mounted = mount(dir.path(), &mut rng);
    let input = input();
    mounted
        .physical
        .prepare_pending_birth(&input, &mut rng)
        .unwrap();
    let physical = mounted.physical.clone();
    drop(mounted);
    finish(&physical, &input);
    assert!(!physical.handler.gate.idle());
    let mut another = self::input();
    another.birth.invocation = [101; 32];
    another.identity = [102; 32];
    assert!(physical
        .prepare_pending_birth(&another, &mut NeverRng)
        .is_err());
    assert_eq!(physical.row_count_data().unwrap(), 2);
}

#[test]
fn native_index2_cell_late_equal_replacement_is_preserved_without_reparenting() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc212);
    let mounted = mount(dir.path(), &mut rng);
    let input = input();
    mounted
        .physical
        .prepare_pending_birth(&input, &mut rng)
        .unwrap();
    let destination = file(dir.path(), super::super::super::preparation::INDEX_NAME);
    let original = fs::read(&destination).unwrap();
    io::before_target_proof(
        io::RenameTarget::Index,
        Box::new({
            let destination = destination.clone();
            let original = original.clone();
            move || {
                let replacement = destination.with_extension("foreign");
                fs::write(&replacement, original).unwrap();
                fs::rename(replacement, destination).unwrap();
            }
        }),
    );
    assert!(mounted
        .physical
        .finish_pending_birth(input.birth.invocation, input.birth.request)
        .is_err());
    assert!(io::take_rename_hook_seen());
    assert_eq!(fs::read(&destination).unwrap(), original);
    assert!(mounted
        .physical
        .finish_pending_birth(input.birth.invocation, input.birth.request)
        .is_err());
    assert!(!mounted.physical.handler.gate.idle());
    assert!(mounted.row_count_data().is_err());
}

#[test]
fn native_index2_cell_unwind_holds_poisoned_debt_until_exact_cold_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc213);
    let mounted = mount(dir.path(), &mut rng);
    let input = input();
    mounted
        .physical
        .prepare_pending_birth(&input, &mut rng)
        .unwrap();
    io::before_target_proof(
        io::RenameTarget::Index,
        Box::new(|| panic!("after actual Stage")),
    );
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        mounted
            .physical
            .finish_pending_birth(input.birth.invocation, input.birth.request)
    }))
    .is_err());
    assert!(io::take_rename_hook_seen());
    assert!(mounted.physical.cell.is_poisoned());
    assert!(!mounted.physical.handler.gate.idle());
    assert!(mounted
        .physical
        .finish_pending_birth(input.birth.invocation, input.birth.request)
        .is_err());
    assert!(mounted
        .physical
        .prepare_pending_birth(&input, &mut NeverRng)
        .is_err());
    assert!(mounted.row_count_data().is_err());
    drop(mounted);
    let reopened = mount::NativeIndexStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy(),
    )
    .unwrap();
    assert_eq!(reopened.row_count_data().unwrap(), 2);
    assert_eq!(
        reopened
            .lookup_creation_data(input.birth.invocation, input.birth.request)
            .unwrap()
            .unwrap()
            .state,
        mount::NativeCreationStateData::Pending
    );
}

#[test]
fn native_index2_cell_serial_births_preserve_every_unrelated_row_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc214);
    let mounted = mount(dir.path(), &mut rng);
    let first = input();
    mounted
        .physical
        .prepare_pending_birth(&first, &mut rng)
        .unwrap();
    let first_status = finish(&mounted.physical, &first);
    let mut second = input();
    second.identity = [103; 32];
    second.birth.invocation = [104; 32];
    mounted
        .physical
        .prepare_pending_birth(&second, &mut rng)
        .unwrap();
    // Old receipt lookup must not complete the newly admitted transaction.
    assert!(mounted
        .physical
        .finish_pending_birth(first.birth.invocation, first.birth.request)
        .is_err());
    let second_status = finish(&mounted.physical, &second);
    assert!(second_status.id > first_status.id);
    assert_eq!(mounted.row_count_data().unwrap(), 3);
    assert_eq!(
        mounted
            .lookup_creation_data(first.birth.invocation, first.birth.request)
            .unwrap(),
        Some(first_status)
    );
    assert_eq!(
        mounted
            .lookup_creation_data(second.birth.invocation, second.birth.request)
            .unwrap(),
        Some(second_status)
    );
}

#[test]
fn native_index2_cell_oversized_input_refuses_before_rng_or_admission() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc215);
    let mounted = mount(dir.path(), &mut rng);
    let mut input = input();
    input.network_frame.resize(data::MAX_NETWORK_FRAME + 1, 0);
    assert!(mounted
        .physical
        .prepare_pending_birth(&input, &mut NeverRng)
        .is_err());
    assert!(mounted.physical.handler.gate.idle());
    assert_eq!(mounted.row_count_data().unwrap(), 1);
}

#[test]
fn native_index2_cell_saved_hit_keeps_exact_input_and_does_no_io_or_rng() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc216);
    let mounted = mount(dir.path(), &mut rng);
    let original = input();
    mounted
        .physical
        .prepare_pending_birth(&original, &mut rng)
        .unwrap();
    let status = finish(&mounted.physical, &original);
    // Hold the actual catalog lock: the cached reply must neither wait for it
    // nor prepare any fresh snapshot/ciphertext. It is historical DATA only.
    let store = mounted.physical.current.cached_store().unwrap();
    let _catalog = store.pending_join_catalog.lock().unwrap();
    assert_eq!(
        mounted
            .physical
            .prepare_pending_birth(&original, &mut NeverRng)
            .unwrap(),
        status
    );
    let mut changed = input();
    changed.name.push('!');
    assert!(mounted
        .physical
        .prepare_pending_birth(&changed, &mut NeverRng)
        .is_err());
    changed = input();
    changed.birth.signer[0] ^= 1;
    assert!(mounted
        .physical
        .prepare_pending_birth(&changed, &mut NeverRng)
        .is_err());
}

#[test]
fn native_index2_cell_catalog_contention_before_claim_has_no_durable_effect() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc217);
    let mounted = mount(dir.path(), &mut rng);
    let original = input();
    let selected = fs::read(file(
        dir.path(),
        super::super::super::preparation::INDEX_NAME,
    ))
    .unwrap();
    let store = mounted.physical.current.cached_store().unwrap();
    let catalog = store.pending_join_catalog.lock().unwrap();
    assert!(mounted
        .physical
        .prepare_pending_birth(&original, &mut rng)
        .is_err());
    assert!(mounted.physical.handler.gate.idle());
    assert!(mounted.physical.cell.lock().unwrap().pending.is_none());
    assert_eq!(
        fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME
        ))
        .unwrap(),
        selected
    );
    assert!(!file(
        dir.path(),
        super::super::super::preparation::INDEX_STAGE_NAME
    )
    .exists());
    drop(catalog);
    mounted
        .physical
        .prepare_pending_birth(&original, &mut rng)
        .unwrap();
    finish(&mounted.physical, &original);
}

#[test]
fn native_index2_cell_evicted_and_cold_hits_reject_every_changed_pending_field() {
    for cold in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc218 + u64::from(cold));
        let mounted = mount(dir.path(), &mut rng);
        let first = input();
        mounted
            .physical
            .prepare_pending_birth(&first, &mut rng)
            .unwrap();
        let expected = finish(&mounted.physical, &first);
        let mut second = input();
        second.identity = [105; 32];
        second.birth.invocation = [106; 32];
        mounted
            .physical
            .prepare_pending_birth(&second, &mut rng)
            .unwrap();
        finish(&mounted.physical, &second);
        let mounted = if cold {
            drop(mounted);
            mount::NativeIndexStore::open(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy(),
                policy(),
            )
            .unwrap()
        } else {
            mounted
        };
        let selected = fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME,
        ))
        .unwrap();
        for field in 0..7 {
            let mut changed = input();
            match field {
                0 => changed.identity[0] ^= 1,
                1 => changed.name.push('!'),
                2 => changed.is_dm = true,
                3 => changed.birth.signer[0] ^= 1,
                4 => changed.birth.network[0] ^= 1,
                5 => changed.network_frame[0] ^= 1,
                6 => changed.birth.request[0] ^= 1,
                _ => unreachable!(),
            }
            assert!(
                mounted
                    .physical
                    .prepare_pending_birth(&changed, &mut NeverRng)
                    .is_err(),
                "cold={cold} field={field}"
            );
            assert!(mounted.physical.handler.gate.idle());
            assert!(mounted.physical.cell.lock().unwrap().pending.is_none());
            assert_eq!(
                fs::read(file(
                    dir.path(),
                    super::super::super::preparation::INDEX_NAME
                ))
                .unwrap(),
                selected
            );
        }
        assert_eq!(
            mounted
                .physical
                .prepare_pending_birth(&first, &mut NeverRng)
                .unwrap(),
            expected
        );
        assert_eq!(mounted.row_count_data().unwrap(), 3);
    }
}

#[test]
fn native_index2_cell_retained_completed_buffers_exclude_all_fresh_index_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc220);
    let mounted = mount(dir.path(), &mut rng);
    let first = input();
    mounted
        .physical
        .prepare_pending_birth(&first, &mut rng)
        .unwrap();
    let (status, debt) = mounted
        .physical
        .finish_pending_birth(first.birth.invocation, first.birth.request)
        .unwrap();
    assert!(mounted.physical.handler.gate.idle()); // Covered DATA no longer excludes JOIN.
    let mut second = input();
    second.identity = [107; 32];
    second.birth.invocation = [108; 32];
    // No second fee-sized allocation can overlap outgoing debt under the same
    // one fixed reservation, even if a caller keeps its result indefinitely.
    assert!(mounted
        .physical
        .prepare_pending_birth(&second, &mut NeverRng)
        .is_err());
    assert!(mounted.row_count_data().is_err());
    assert!(mounted.physical.row_count_data().is_err());
    assert!(mounted
        .lookup_creation_data(second.birth.invocation, second.birth.request)
        .is_err());
    // The exact no-allocation cached reply remains available.
    assert_eq!(
        mounted
            .physical
            .prepare_pending_birth(&first, &mut NeverRng)
            .unwrap(),
        status
    );
    drop(debt);
    assert_eq!(mounted.row_count_data().unwrap(), 2);
    mounted
        .physical
        .prepare_pending_birth(&second, &mut rng)
        .unwrap();
    finish(&mounted.physical, &second);
    assert_eq!(mounted.row_count_data().unwrap(), 3);
}

#[test]
fn native_index2_cell_retirement_ack_follows_real_file_and_buffer_destruction() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc221);
    let mounted = mount(dir.path(), &mut rng);
    let input = input();
    mounted
        .physical
        .prepare_pending_birth(&input, &mut rng)
        .unwrap();
    let seen = Arc::new(AtomicBool::new(false));
    let done = {
        let mut cell = mounted.physical.cell.lock().unwrap();
        let pending = cell.pending.as_mut().unwrap();
        let done = pending.retirement.clone();
        pending.retirement_observation = Some(io::RetirementObservation::new(Box::new({
            let done = done.clone();
            let seen = seen.clone();
            move || {
                assert!(!done.load(Ordering::Acquire));
                seen.store(true, Ordering::Release);
            }
        })));
        done
    };
    let (_, debt) = mounted
        .physical
        .finish_pending_birth(input.birth.invocation, input.birth.request)
        .unwrap();
    assert!(!seen.load(Ordering::Acquire));
    assert!(!done.load(Ordering::Acquire));
    let refunded = mounted.physical.handler.fee.refund_notice();
    drop(mounted);
    assert!(!refunded.load(Ordering::Acquire));
    drop(debt);
    assert!(seen.load(Ordering::Acquire));
    assert!(done.load(Ordering::Acquire));
    assert!(refunded.load(Ordering::Acquire));
}

#[test]
fn lifecycle3_ownership_preserves_the_original_across_strong_and_weak_refusals() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc330);
    let mounted = mount(dir.path(), &mut rng);
    let address = Arc::as_ptr(&mounted.physical);
    let fee = Arc::as_ptr(&mounted.physical.handler.fee);
    let refunded = mounted.physical.handler.fee.refund_notice();
    let worker = mounted.physical.clone();
    let tail = Arc::downgrade(&worker);
    let mounted = mounted.into_upgrade_source().unwrap_err();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(fee, Arc::as_ptr(&mounted.physical.handler.fee));
    assert!(mounted.physical.handler.gate.idle());
    assert!(!refunded.load(Ordering::Acquire));
    drop(worker);
    // Exactly one strong owner is insufficient while the old weak tail exists.
    assert_eq!(Arc::strong_count(&mounted.physical), 1);
    assert_eq!(Arc::weak_count(&mounted.physical), 1);
    let mounted = mounted.into_upgrade_source().unwrap_err();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(fee, Arc::as_ptr(&mounted.physical.handler.fee));
    assert!(mounted.physical.handler.gate.idle());
    assert!(!refunded.load(Ordering::Acquire));
    drop(tail);
    let mounted = mounted
        .into_upgrade_source()
        .unwrap()
        .into_original_for_test();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(fee, Arc::as_ptr(&mounted.physical.handler.fee));
    assert!(mounted.physical.handler.gate.idle());
    assert!(!refunded.load(Ordering::Acquire));
    drop(mounted);
    assert!(refunded.load(Ordering::Acquire));
}

#[test]
fn lifecycle3_ownership_refuses_pending_and_retained_completed_debt() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc331);
    let mounted = mount(dir.path(), &mut rng);
    let address = Arc::as_ptr(&mounted.physical);
    let fee = Arc::as_ptr(&mounted.physical.handler.fee);
    let original = input();
    mounted
        .physical
        .prepare_pending_birth(&original, &mut rng)
        .unwrap();
    let mounted = mounted.into_upgrade_source().unwrap_err();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(fee, Arc::as_ptr(&mounted.physical.handler.fee));
    // Preparing the original transaction already holds its negative JOIN claim.
    assert!(!mounted.physical.handler.gate.idle());
    let (_, debt) = mounted
        .physical
        .finish_pending_birth(original.birth.invocation, original.birth.request)
        .unwrap();
    assert!(mounted.physical.handler.gate.idle());
    let mounted = mounted.into_upgrade_source().unwrap_err();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(fee, Arc::as_ptr(&mounted.physical.handler.fee));
    assert!(mounted.physical.handler.gate.idle());
    drop(debt);
    // The completed last-result cache and the acknowledged retirement marker
    // are history, not unresolved work; neither should strand an idle upgrade.
    let mounted = mounted
        .into_upgrade_source()
        .unwrap()
        .into_original_for_test();
    assert_eq!(mounted.row_count_data().unwrap(), 2);
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
}

#[test]
fn lifecycle3_ownership_never_clears_poison_or_reopens_a_closed_gate() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc332);
    let mounted = mount(dir.path(), &mut rng);
    let address = Arc::as_ptr(&mounted.physical);
    let fee = Arc::as_ptr(&mounted.physical.handler.fee);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _cell = mounted.physical.cell.lock().unwrap();
        panic!("poison the real original cell");
    }))
    .is_err());
    let mounted = mounted.into_upgrade_source().unwrap_err();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(fee, Arc::as_ptr(&mounted.physical.handler.fee));
    assert!(mounted.physical.cell.is_poisoned());
    assert!(mounted.physical.handler.gate.idle());
    drop(mounted);

    let dir = tempfile::tempdir().unwrap();
    let mounted = mount(dir.path(), &mut rng);
    let address = Arc::as_ptr(&mounted.physical);
    mounted.close_index_admission();
    let mounted = mounted.into_upgrade_source().unwrap_err();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert!(!mounted.physical.handler.gate.idle());
}

fn lifecycle3_policy(
    pool: CriticalCatalogReadPool,
) -> super::super::super::lifecycle::preparation::Policy {
    use super::super::super::lifecycle::preparation::Policy;
    Policy::new(pool, Policy::required_persistent_capacity()).unwrap()
}

#[test]
fn lifecycle3_expected_current_is_prepaid_and_independent_of_mounted_current_cache() {
    use crate::store::persistence::native_registry::lifecycle::{
        ExpectedCurrentInputs, EXPECTED_CURRENT_FEE,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc340);
    let mounted = mount(dir.path(), &mut rng);
    let store = mounted.physical.current.cached_store().unwrap();
    let pool = CriticalCatalogReadPool::new(EXPECTED_CURRENT_FEE).unwrap();
    let short = pool.try_reserve(EXPECTED_CURRENT_FEE - 1).unwrap();
    assert!(ExpectedCurrentInputs::capture(store, &short, &pool).is_err());
    drop(short);
    let paid = pool.try_reserve(EXPECTED_CURRENT_FEE).unwrap();
    let inputs = ExpectedCurrentInputs::capture(store, &paid, &pool).unwrap();
    let layout = store.current_layout.lock().unwrap().take().unwrap();
    // Real cold inspection occurs before this cache exists. Read-only marker
    // authentication must not silently install or depend on that mutable owner.
    let actual = inputs.authenticate(store).unwrap();
    assert_eq!(actual, layout.layout);
    assert!(store.current_layout.lock().unwrap().is_none());
    *store.current_layout.lock().unwrap() = Some(layout);
    inputs.require(store).unwrap();
    assert_eq!(pool.charged_for_test(), EXPECTED_CURRENT_FEE);
    drop(inputs);
    assert_eq!(pool.charged_for_test(), EXPECTED_CURRENT_FEE);
    drop(paid);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn lifecycle3_expected_current_rejects_equal_byte_marker_and_directory_replacement() {
    use crate::store::persistence::native_registry::lifecycle::{
        ExpectedCurrentInputs, EXPECTED_CURRENT_FEE,
    };
    for (number, name) in [
        crate::store::critical_namespaces::archive_bootstrap::ROOT_MARKER,
        crate::store::critical_namespaces::current_layout::CURRENT_MARKER,
        "",
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let replaced = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc341 + number as u64);
        let mounted = mount(dir.path(), &mut rng);
        let store = mounted.physical.current.cached_store().unwrap();
        let pool = CriticalCatalogReadPool::new(EXPECTED_CURRENT_FEE).unwrap();
        let paid = pool.try_reserve(EXPECTED_CURRENT_FEE).unwrap();
        let inputs = ExpectedCurrentInputs::capture(store, &paid, &pool).unwrap();
        let original = if name.is_empty() {
            // Windows rename rejects a source directory with a trailing separator.
            dir.path()
                .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        } else {
            file(dir.path(), name)
        };
        if name.is_empty() {
            if let Err(error) = fs::rename(&original, replaced.path().join("original-root")) {
                // The real Windows Store can hold directory handles which
                // prohibit this rename outright. Pin that OS protection and
                // unchanged proof; where replacement is possible the identity
                // checks below must detect it. Do not weaken handle sharing.
                #[cfg(windows)]
                {
                    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
                    inputs.require(store).unwrap();
                    assert!(original.is_dir());
                    continue;
                }
                #[cfg(not(windows))]
                panic!("unexpected directory replacement refusal: {error}");
            }
            fs::create_dir(&original).unwrap();
        } else {
            let bytes = fs::read(&original).unwrap();
            fs::write(replaced.path().join("equal-marker"), &bytes).unwrap();
            fs::rename(replaced.path().join("equal-marker"), &original).unwrap();
            assert_eq!(fs::read(&original).unwrap(), bytes);
        }
        assert!(inputs.require(store).is_err());
        assert!(inputs.authenticate(store).is_err());
        // A stale proof cannot mutate/repair the substituted object.
        assert!(original.exists());
        assert!(mounted.physical.handler.gate.idle());
    }
}

#[test]
fn lifecycle3_expected_current_rejects_foreign_authenticated_marker_and_new_mount() {
    use crate::store::critical_namespaces::archive_current_namespace_codec as current_codec;
    use crate::store::persistence::native_registry::lifecycle::{
        ExpectedCurrentInputs, EXPECTED_CURRENT_FEE,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc345);
    let mounted = mount(dir.path(), &mut rng);
    let store = mounted.physical.current.cached_store().unwrap();
    let pool = CriticalCatalogReadPool::new(EXPECTED_CURRENT_FEE).unwrap();
    let paid = pool.try_reserve(EXPECTED_CURRENT_FEE).unwrap();
    let inputs = ExpectedCurrentInputs::capture(store, &paid, &pool).unwrap();
    let mut layout = inputs.authenticate(store).unwrap();
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    layout.root[0] ^= 1;
    let foreign = current_codec::seal_marker_data(&key, layout, &mut rng).unwrap();
    let marker = file(
        dir.path(),
        crate::store::critical_namespaces::current_layout::CURRENT_MARKER,
    );
    let original = fs::read(&marker).unwrap();
    fs::write(&marker, &foreign).unwrap();
    assert!(inputs.require(store).is_err());
    let foreign_inputs = ExpectedCurrentInputs::capture(store, &paid, &pool).unwrap();
    assert!(foreign_inputs.authenticate(store).is_err());
    assert_eq!(fs::read(&marker).unwrap(), foreign.as_slice());
    drop(foreign_inputs);
    fs::write(&marker, &original).unwrap();
    inputs.require(store).unwrap();
    // The vault lock prohibits simultaneous positive mounts. Retain only the
    // captured old inputs/payment, retire the whole original owner, then use
    // the genuine opener. Identical paths/bytes/key cannot revive the old proof.
    drop(mounted);
    let reopened = mount::NativeIndexStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy(),
    )
    .unwrap();
    let next = reopened.physical.current.cached_store().unwrap();
    assert!(inputs.require(next).is_err());
    assert!(inputs.authenticate(next).is_err());
    assert_eq!(fs::read(marker).unwrap(), original);
}
fn lifecycle3_pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(super::super::super::lifecycle::preparation::PREPARATION_FEE)
        .unwrap()
}

#[test]
fn lifecycle3_first_torn_proof_cannot_move_to_an_equal_second_inventory() {
    use super::super::super::lifecycle::preparation::first_torn;
    use crate::store::persistence::native_registry::lifecycle::{
        self as physical, upgrade as writer,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc393);
    let mounted = mount(dir.path(), &mut rng);
    let store = mounted.physical.current.cached_store().unwrap();
    let pool = lifecycle3_pool();
    let paid = lifecycle3_policy(pool.clone()).reserve().unwrap();
    let seed = dir.path().join(physical::UPGRADE_SEED_NAME);
    fs::write(&seed, [0; 39]).unwrap();
    let original = writer::UpgradeInventory::capture(store, &paid, &pool).unwrap();
    let second = writer::UpgradeInventory::capture(store, &paid, &pool).unwrap();
    let proof = first_torn::verify(store, &original).unwrap();
    assert!(writer::FirstTornIo::prepare(store, second, proof).is_err());
    assert_eq!(fs::read(&seed).unwrap(), [0; 39]);
    let proof = first_torn::verify(store, &original).unwrap();
    let mut cleanup = writer::FirstTornIo::prepare(store, original, proof).unwrap();
    cleanup.run(store).unwrap();
    assert!(!seed.exists());
}

#[test]
fn lifecycle3_first_torn_short_and_bad_mac_claims_retire_only_empty_fences() {
    use super::super::super::lifecycle::preparation::first_torn;
    use crate::store::persistence::native_registry::lifecycle::{
        self as physical, upgrade as writer,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc390);
    let mounted = mount(dir.path(), &mut rng);
    // An established index's later generation is valid input too. Requiring
    // G1 here would permanently strand claim-window crashes after a birth.
    let pending = input();
    mounted
        .physical
        .prepare_pending_birth(&pending, &mut rng)
        .unwrap();
    finish(&mounted.physical, &pending);
    assert_eq!(mounted.row_count_data().unwrap(), 2);
    let pool = lifecycle3_pool();
    let prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let store = prepared.original_store_for_test();
    let catalog = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Current.name());
    let original = fs::read(&catalog).unwrap();
    let seed = dir.path().join(physical::UPGRADE_SEED_NAME);
    let stage = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Stage.name());
    for length in [0, 1, 23, 24, 39, 40] {
        for with_fence in [false, true] {
            fs::write(&seed, vec![0; length]).unwrap();
            if with_fence {
                fs::write(&stage, []).unwrap();
            }
            let inventory = prepared.inventory_for_test(&pool);
            let proof = first_torn::verify(store, &inventory).unwrap();
            let mut cleanup = writer::FirstTornIo::prepare(store, inventory, proof).unwrap();
            cleanup.run(store).unwrap();
            assert!(!seed.exists());
            assert!(!stage.exists());
            assert_eq!(fs::read(&catalog).unwrap(), original);
        }
    }
}

#[test]
fn lifecycle3_first_torn_unlink_retries_cover_fence_before_seed() {
    use super::super::super::lifecycle::preparation::first_torn;
    use crate::store::persistence::native_registry::lifecycle::{
        self as physical, upgrade as writer,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc391);
    let mounted = mount(dir.path(), &mut rng);
    let pool = lifecycle3_pool();
    let prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let store = prepared.original_store_for_test();
    let seed = dir.path().join(physical::UPGRADE_SEED_NAME);
    let stage = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Stage.name());
    for cut in [
        io::Fault::UpgradeFenceUnlink,
        io::Fault::UpgradeFenceUnlinkCovered,
        io::Fault::UpgradeSeedUnlink,
    ] {
        fs::write(&seed, [0; 39]).unwrap();
        fs::write(&stage, []).unwrap();
        let inventory = prepared.inventory_for_test(&pool);
        let proof = first_torn::verify(store, &inventory).unwrap();
        let mut cleanup = writer::FirstTornIo::prepare(store, inventory, proof).unwrap();
        io::arm_fault(cut);
        assert!(cleanup.run(store).is_err());
        assert!(io::take_fault_seen());
        assert!(!stage.exists());
        assert_eq!(seed.exists(), cut != io::Fault::UpgradeSeedUnlink);
        writer::reset_fence_covers();
        if cut != io::Fault::UpgradeSeedUnlink {
            writer::before_seed_unlink(Box::new({
                let seed = seed.clone();
                move || {
                    assert!(writer::fence_covers() > 0);
                    assert_eq!(fs::read(seed).unwrap(), [0; 39]);
                }
            }));
        }
        cleanup.run(store).unwrap();
        assert_eq!(
            writer::take_seed_hook_seen(),
            cut != io::Fault::UpgradeSeedUnlink
        );
        assert!(!seed.exists());
    }
}

#[test]
fn lifecycle3_first_torn_preserves_unknown_and_authenticated_invalid_roles() {
    use super::super::super::lifecycle::preparation::first_torn;
    use crate::store::persistence::native_registry::lifecycle as physical;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc392);
    let mounted = mount(dir.path(), &mut rng);
    let pool = lifecycle3_pool();
    let prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let store = prepared.original_store_for_test();
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let derived = Zeroizing::new(blake3::derive_key(
        "catcoms/private-native-backend-upgrade-key/v3",
        &key[..],
    ));
    let seed = dir.path().join(physical::UPGRADE_SEED_NAME);
    let stage = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Stage.name());
    for body in [&[][..], &b"future authenticated intent"[..]] {
        let sealed = crate::store::frame(&catcoms_crypto::seal(&derived, body, &mut rng).unwrap());
        fs::write(&seed, &sealed).unwrap();
        fs::write(&stage, []).unwrap();
        let inventory = prepared.inventory_for_test(&pool);
        assert!(first_torn::verify(store, &inventory).is_err());
        assert_eq!(fs::read(&seed).unwrap(), sealed);
        assert!(fs::read(&stage).unwrap().is_empty());
    }
    fs::write(&seed, [0; 39]).unwrap();
    fs::write(&stage, b"unknown nonempty stage").unwrap();
    let inventory = prepared.inventory_for_test(&pool);
    assert!(first_torn::verify(store, &inventory).is_err());
    assert_eq!(fs::read(&stage).unwrap(), b"unknown nonempty stage");
    fs::write(&seed, vec![0; physical::MAX_UPGRADE_SEED_FRAME + 1]).unwrap();
    assert!(prepared.try_inventory_for_test(&pool).is_err());
    assert_eq!(
        fs::metadata(&seed).unwrap().len(),
        (physical::MAX_UPGRADE_SEED_FRAME + 1) as u64
    );
}

#[test]
fn lifecycle3_physical_exact_outputs_survive_every_owned_retry_cut() {
    use crate::store::persistence::native_registry::lifecycle::{
        self as physical, upgrade as writer,
    };
    for (number, cut) in [
        io::Fault::UpgradeClaim,
        io::Fault::UpgradeFence,
        io::Fault::UpgradePartialSeed,
        io::Fault::UpgradeSeed,
        io::Fault::UpgradePartialIndex,
        io::Fault::UpgradeIndexTemp,
        io::Fault::UpgradeIndexRename,
        io::Fault::UpgradePartialMarker,
        io::Fault::UpgradeMarkerTemp,
        io::Fault::UpgradeMarkerRename,
        io::Fault::UpgradePartialCatalog,
        io::Fault::UpgradeCatalogRename,
        io::Fault::UpgradeCovered,
        io::Fault::UpgradeSeedUnlink,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc350 + number as u64);
        let mounted = mount(dir.path(), &mut rng);
        let gate = Arc::downgrade(&mounted.physical.handler.gate);
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let store = prepared.original_store_for_test();
        let inventory = prepared.inventory_for_test(&pool);
        assert_eq!(
            inventory.authenticate_expected(store).unwrap(),
            prepared
                .expected_current_for_test()
                .authenticate(store)
                .unwrap()
        );
        let bytes = prepared.output.bytes();
        let mut work = writer::UpgradeIo::prepare(store, inventory, &bytes).unwrap();
        // A raw storage fixture retains the real old2 exclusion. No handler3
        // or warm3 owner is published by this test or by the IO module.
        let claim = {
            let catalog = store.pending_join_catalog.lock().unwrap();
            assert!(catalog.as_ref().unwrap().pending.is_none());
            prepared.snapshot.require(store).unwrap();
            gate.upgrade().unwrap().claim().unwrap()
        };
        io::arm_fault(cut);
        assert!(work.advance_upgrade(store, &bytes).is_err(), "{cut:?}");
        assert!(io::take_fault_seen(), "{cut:?}");
        // In particular SeedUnlink has visible selected files and no seed,
        // but its final directory barrier has not succeeded yet.
        assert!(work.require_complete(store, &bytes).is_err(), "{cut:?}");
        assert!(store.require_native_index_idle_cached().is_err());
        work.advance_upgrade(store, &bytes).unwrap();
        work.require_complete(store, &bytes).unwrap();
        assert_eq!(
            fs::read(
                dir.path()
                    .join(crate::store::critical_namespaces::Slot::Current.name())
            )
            .unwrap(),
            bytes.catalog
        );
        assert_eq!(
            fs::read(file(
                dir.path(),
                super::super::super::preparation::INDEX_NAME
            ))
            .unwrap(),
            bytes.index
        );
        assert_eq!(
            fs::read(file(
                dir.path(),
                super::super::super::preparation::MARKER_NAME
            ))
            .unwrap(),
            bytes.marker
        );
        assert!(!dir.path().join(physical::UPGRADE_SEED_NAME).exists());
        assert!(!file(dir.path(), physical::UPGRADE_INDEX_NAME).exists());
        assert!(!file(dir.path(), physical::UPGRADE_MARKER_NAME).exists());
        // No data-only success can reopen JOIN on an uninstalled3 handler.
        gate.upgrade().unwrap().close();
        assert_eq!(claim.finish().unwrap(), interlock::ClaimRelease::Closed);
        drop(work);
        drop(prepared);
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn lifecycle3_physical_reconstructed_marker_cut_recovers_output_barrier_before_catalog() {
    use super::super::super::{capability::NativeCapabilityData, lifecycle::preparation::Output};
    use crate::store::persistence::native_registry::lifecycle::upgrade as writer;
    for reconstruct in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc370 + u64::from(reconstruct));
        let mounted = mount(dir.path(), &mut rng);
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let store = prepared.original_store_for_test();
        let original = fs::read(
            dir.path()
                .join(crate::store::critical_namespaces::Slot::Current.name()),
        )
        .unwrap();
        let mut work = writer::UpgradeIo::prepare(
            store,
            prepared.inventory_for_test(&pool),
            &prepared.output.bytes(),
        )
        .unwrap();
        io::arm_fault(io::Fault::UpgradeMarkerRename);
        assert!(work
            .advance_upgrade(store, &prepared.output.bytes())
            .is_err());
        assert!(io::take_fault_seen());
        // This is real raw-role reconstruction under the same retained mount,
        // not an actual cold opener/handler3 acceptance claim.
        let decoded;
        let output = if reconstruct {
            drop(work);
            let inventory = prepared.inventory_for_test(&pool);
            let layout = inventory.authenticate_expected(store).unwrap();
            let expected =
                NativeCapabilityData::for_current_data(layout, &layout.encode_data().unwrap())
                    .unwrap();
            let key = Zeroizing::new(store.keys.db_key().unwrap());
            decoded = Output::decode_seed(&key, expected, inventory.seed().unwrap()).unwrap();
            work = writer::UpgradeIo::recover(store, inventory, &decoded.bytes()).unwrap();
            &decoded
        } else {
            &prepared.output
        };
        writer::reset_output_root_covers();
        io::before_target_proof(
            io::RenameTarget::UpgradeCatalog,
            Box::new({
                let catalog = dir
                    .path()
                    .join(crate::store::critical_namespaces::Slot::Current.name());
                move || {
                    assert!(writer::output_root_covers() > 0);
                    assert_eq!(fs::read(catalog).unwrap(), original);
                }
            }),
        );
        work.advance_upgrade(store, &output.bytes()).unwrap();
        assert!(io::take_rename_hook_seen());
        assert_eq!(
            fs::read(file(
                dir.path(),
                super::super::super::preparation::INDEX_NAME
            ))
            .unwrap(),
            output.bytes().index
        );
    }
}

#[test]
fn lifecycle3_physical_last_target_proofs_reject_replaced_current_marker() {
    use crate::store::persistence::native_registry::lifecycle::upgrade as writer;
    for (number, target) in [
        io::RenameTarget::UpgradeIndex,
        io::RenameTarget::UpgradeMarker,
        io::RenameTarget::UpgradeCatalog,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let replacement = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc380 + number as u64);
        let mounted = mount(dir.path(), &mut rng);
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let store = prepared.original_store_for_test();
        let original_catalog = fs::read(
            dir.path()
                .join(crate::store::critical_namespaces::Slot::Current.name()),
        )
        .unwrap();
        let marker = file(
            dir.path(),
            crate::store::critical_namespaces::current_layout::CURRENT_MARKER,
        );
        let original = fs::read(&marker).unwrap();
        let alternate = replacement.path().join("equal-marker");
        fs::write(&alternate, &original).unwrap();
        let mut work = writer::UpgradeIo::prepare(
            store,
            prepared.inventory_for_test(&pool),
            &prepared.output.bytes(),
        )
        .unwrap();
        io::before_target_proof(
            target,
            Box::new({
                let marker = marker.clone();
                move || fs::rename(alternate, marker).unwrap()
            }),
        );
        assert!(work
            .advance_upgrade(store, &prepared.output.bytes())
            .is_err());
        assert!(io::take_rename_hook_seen());
        assert_eq!(fs::read(marker).unwrap(), original);
        assert_eq!(
            fs::read(
                dir.path()
                    .join(crate::store::critical_namespaces::Slot::Current.name())
            )
            .unwrap(),
            original_catalog
        );
        assert!(work
            .advance_upgrade(store, &prepared.output.bytes())
            .is_err());
    }
}

#[test]
fn lifecycle3_preparation_authenticates_the_held_original_without_changing_any_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc333);
    let mounted = mount(dir.path(), &mut rng);
    let input = input();
    mounted
        .physical
        .prepare_pending_birth(&input, &mut rng)
        .unwrap();
    finish(&mounted.physical, &input);
    let key = Zeroizing::new(
        mounted
            .physical
            .current
            .cached_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    let domain = mounted
        .physical
        .current
        .cached_store()
        .unwrap()
        .chat_vault_domain()
        .unwrap();
    let layout = mounted
        .physical
        .current
        .cached_store()
        .unwrap()
        .current_layout
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .layout;
    let expected = super::super::super::capability::NativeCapabilityData::for_current_data(
        layout,
        &layout.encode_data().unwrap(),
    )
    .unwrap();
    let gate = Arc::downgrade(&mounted.physical.handler.gate);
    let old_refund = mounted.physical.handler.fee.refund_notice();
    let catalog_name = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Current.name());
    let catalog_before = fs::read(&catalog_name).unwrap();
    let index_name = file(dir.path(), super::super::super::preparation::INDEX_NAME);
    let index_before = fs::read(&index_name).unwrap();
    let pool = lifecycle3_pool();
    let mut prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    assert!(gate.upgrade().unwrap().idle()); // Preparation did NOT admit a writing claim.
    assert!(!old_refund.load(Ordering::Acquire));
    assert!(pool.charged_for_test() > 0);
    let retired = Arc::new(AtomicBool::new(false));
    prepared.retirement = Some(io::RetirementObservation::new(Box::new({
        let retired = retired.clone();
        let gate = gate.clone();
        let pool = pool.clone();
        let old_refund = old_refund.clone();
        move || {
            assert!(!gate.upgrade().unwrap().idle());
            assert!(!old_refund.load(Ordering::Acquire));
            assert!(pool.charged_for_test() > 0);
            retired.store(true, Ordering::Release);
        }
    })));
    prepared
        .output
        .assert_original_and_output_for_test(&key, &domain, &prepared.snapshot);
    prepared
        .output
        .assert_seed_reader_matrix_for_test(&key, expected, &mut rng);
    assert_eq!(fs::read(catalog_name).unwrap(), catalog_before);
    assert_eq!(fs::read(index_name).unwrap(), index_before);
    use crate::store::persistence::native_registry::lifecycle as new_io;
    for name in [
        dir.path().join(new_io::UPGRADE_SEED_NAME),
        file(dir.path(), new_io::UPGRADE_INDEX_NAME),
        file(dir.path(), new_io::UPGRADE_MARKER_NAME),
    ] {
        assert!(!name.exists());
    }
    drop(prepared);
    assert!(retired.load(Ordering::Acquire));
    assert!(gate.upgrade().is_none());
    assert_eq!(pool.charged_for_test(), 0);
    assert!(old_refund.load(Ordering::Acquire));
}

#[test]
fn lifecycle3_owning_upgrade_retries_same_intent_and_reopens_without_rng() {
    use super::super::upgrade_source::owning;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc401);
    let mounted = mount(dir.path(), &mut rng);
    let pending = input();
    mounted
        .physical
        .prepare_pending_birth(&pending, &mut rng)
        .unwrap();
    finish(&mounted.physical, &pending);
    let gate = Arc::downgrade(&mounted.physical.handler.gate);
    let old_refund = mounted.physical.handler.fee.refund_notice();
    let pool = lifecycle3_pool();
    let prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let selected = prepared.output.bytes().index.to_vec();
    let mut upgrade = prepared.admit().unwrap();
    assert!(!gate.upgrade().unwrap().idle());
    io::arm_fault(io::Fault::UpgradeSeedUnlink);
    assert!(upgrade.retry().is_err());
    assert!(io::take_fault_seen());
    assert!(!gate.upgrade().unwrap().idle());
    assert!(!old_refund.load(Ordering::Acquire));
    assert_eq!(
        fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME
        ))
        .unwrap(),
        selected
    );
    upgrade.retry().unwrap();
    let mut mounted = upgrade.into_store().unwrap();
    assert!(old_refund.load(Ordering::Acquire));
    assert!(gate.upgrade().is_none());
    assert_eq!(mounted.row_count_data().unwrap(), 2);
    assert!(pool.charged_for_test() > 0);
    drop(mounted);
    assert_eq!(pool.charged_for_test(), 0);
    // This is the real strict3 opener, before Current/JOIN cleanup. None of
    // these storage facts authorize an SDK or represent a joined-member pin.
    let mut cold = owning::Store::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        lifecycle3_policy(pool.clone()),
    )
    .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 2);
    assert_eq!(
        fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME
        ))
        .unwrap(),
        selected
    );
    drop(cold);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn lifecycle3_owning_close_before_admission_has_no_file_effect() {
    use super::super::upgrade_source::owning::{at_edge, Edge};
    for close_new in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc402 + u64::from(close_new));
        let mounted = mount(dir.path(), &mut rng);
        let old = mounted.physical.handler.gate.clone();
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let new = prepared.lifecycle_handler_for_test();
        let catalog = dir
            .path()
            .join(crate::store::critical_namespaces::Slot::Current.name());
        let before = fs::read(&catalog).unwrap();
        at_edge(
            Edge::BeforeAdmission,
            Box::new({
                let old = old.clone();
                let new = new.clone();
                move || {
                    if close_new {
                        new.close();
                    } else {
                        old.close();
                    }
                }
            }),
        );
        let prepared = prepared.admit().unwrap_err();
        assert_eq!(fs::read(&catalog).unwrap(), before);
        assert!(!dir
            .path()
            .join(crate::store::persistence::native_registry::lifecycle::UPGRADE_SEED_NAME)
            .exists());
        assert!(!dir
            .path()
            .join(crate::store::critical_namespaces::Slot::Stage.name())
            .exists());
        drop(prepared);
        assert!(!old.idle());
        assert!(new.require_idle_cached().is_err());
        drop(new);
        drop(old);
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn lifecycle3_owning_close_after_install_never_returns_a_warm_owner() {
    use super::super::upgrade_source::owning::{at_edge, Edge};
    for close_new in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc404 + u64::from(close_new));
        let mounted = mount(dir.path(), &mut rng);
        let old = mounted.physical.handler.gate.clone();
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let new = prepared.lifecycle_handler_for_test();
        let mut upgrade = prepared.admit().unwrap();
        at_edge(
            Edge::AfterInstall,
            Box::new({
                let old = old.clone();
                let new = new.clone();
                move || {
                    if close_new {
                        new.close();
                    } else {
                        old.close();
                    }
                }
            }),
        );
        assert!(upgrade.retry().is_err());
        assert!(!old.idle());
        assert!(new.require_idle_cached().is_err());
        let upgrade = upgrade.into_store().unwrap_err();
        drop(upgrade);
        drop(new);
        drop(old);
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn lifecycle3_owning_unwind_closes_both_before_retiring_inputs_in_either_wrapper() {
    use super::super::upgrade_source::owning::{at_edge, Edge};
    for edge in [Edge::AfterInstall, Edge::AfterDestination] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng =
            ChaCha20Rng::seed_from_u64(0xc406 + u64::from(edge == Edge::AfterDestination));
        let mounted = mount(dir.path(), &mut rng);
        let old = mounted.physical.handler.gate.clone();
        let old_refund = mounted.physical.handler.fee.refund_notice();
        let pool = lifecycle3_pool();
        let mut prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let new = prepared.lifecycle_handler_for_test();
        let observed = Arc::new(AtomicBool::new(false));
        prepared.retirement = Some(io::RetirementObservation::new(Box::new({
            let old = old.clone();
            let new = new.clone();
            let pool = pool.clone();
            let old_refund = old_refund.clone();
            let observed = observed.clone();
            move || {
                // Record only while already unwinding; a second panic here
                // would abort the process instead of reporting a regression.
                observed.store(
                    !old.idle()
                        && new.require_idle_cached().is_err()
                        && !old_refund.load(Ordering::Acquire)
                        && pool.charged_for_test() > 0,
                    Ordering::Release,
                );
            }
        })));
        let mut upgrade = prepared.admit().unwrap();
        at_edge(edge, Box::new(|| panic!("real owning transfer cut")));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            if edge == Edge::AfterDestination {
                let _ = upgrade.into_store();
            } else {
                let _ = upgrade.retry();
            }
        }));
        assert!(result.is_err());
        assert!(observed.load(Ordering::Acquire));
        assert!(!old.idle());
        assert!(new.require_idle_cached().is_err());
        drop(old);
        drop(new);
        assert!(old_refund.load(Ordering::Acquire));
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn lifecycle3_owning_admission_unwind_closes_before_actual_io_retirement() {
    use super::super::upgrade_source::owning::{at_edge, Edge};
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc409);
    let mounted = mount(dir.path(), &mut rng);
    let old = mounted.physical.handler.gate.clone();
    let pool = lifecycle3_pool();
    let mut prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let new = prepared.lifecycle_handler_for_test();
    let observed = Arc::new(AtomicBool::new(false));
    prepared.io_retirement = Some(Box::new({
        let old = old.clone();
        let new = new.clone();
        let pool = pool.clone();
        let observed = observed.clone();
        move || {
            observed.store(
                !old.idle() && new.require_idle_cached().is_err() && pool.charged_for_test() > 0,
                Ordering::Release,
            );
        }
    }));
    at_edge(
        Edge::BeforeAdmission,
        Box::new(|| panic!("actual final admission cut")),
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || prepared.admit())).is_err()
    );
    assert!(observed.load(Ordering::Acquire));
    drop(new);
    drop(old);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn lifecycle3_owning_destination_unwind_keeps_actual_vault_busy_until_io_drains() {
    use super::super::upgrade_source::owning::{self, at_edge, Edge};
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc410);
    let mounted = mount(dir.path(), &mut rng);
    let pool = lifecycle3_pool();
    let mut prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let (entered, seen) = sync_channel(1);
    let (release, wait) = sync_channel(1);
    let retirement = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let retired = retirement.clone();
    prepared.io_retirement = Some(Box::new(move || {
        // This destructor is already unwinding. Never assert/panic or make an
        // unbounded channel send; report the watchdog outcome to the test.
        let ready = entered.try_send(()).is_ok();
        let released = wait.recv_timeout(Duration::from_secs(10)).is_ok();
        retired.store(if ready && released { 1 } else { 2 }, Ordering::Release);
    }));
    let upgrade = prepared.admit().unwrap();
    let worker = std::thread::spawn(move || {
        at_edge(
            Edge::AfterDestination,
            Box::new(|| panic!("destination already constructed")),
        );
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ = upgrade.into_store();
        }))
        .is_err()
    });
    let ready = seen.recv_timeout(Duration::from_secs(10));
    // The destination's real Current has already been destroyed. Only the
    // resource-only lease may now exclude this actual competing constructor.
    let busy = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ready.is_ok()
            && matches!(
                crate::store::ServerStore::open(
                    dir.path(),
                    b"must fail before checking secret",
                    &mut NeverRng
                ),
                Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
            )
    }));
    // Always release and join before any assertion, even on failed readiness,
    // probe refusal, or a panic in the competing constructor.
    let _ = release.try_send(());
    let joined = worker.join();
    assert!(ready.is_ok());
    assert!(joined.unwrap());
    assert_eq!(retirement.load(Ordering::Acquire), 1);
    let busy = busy.unwrap();
    assert!(busy, "retiring IO must still own the physical session");
    assert_eq!(pool.charged_for_test(), 0);
    let mut next = owning::Store::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        lifecycle3_policy(pool.clone()),
    )
    .unwrap();
    assert_eq!(next.row_count_data().unwrap(), 1);
}

#[test]
fn lifecycle3_owning_retry_after_installed_handler_keeps_both_original_claims() {
    use super::super::upgrade_source::owning::{at_edge, Edge};
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc412);
    let mounted = mount(dir.path(), &mut rng);
    let old = mounted.physical.handler.gate.clone();
    let pool = lifecycle3_pool();
    let prepared = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(pool.clone()), &mut rng)
        .unwrap();
    let new = prepared.lifecycle_handler_for_test();
    let index = prepared.output.bytes().index.to_vec();
    let stage = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Stage.name());
    let mut upgrade = prepared.admit().unwrap();
    // A real unexpected file at the last proof, AFTER actual policy replacement.
    // The fault file is test-owned; production never removes ambiguous Stage.
    at_edge(
        Edge::AfterInstall,
        Box::new({
            let stage = stage.clone();
            move || fs::write(stage, [0]).unwrap()
        }),
    );
    assert!(upgrade.retry().is_err());
    assert!(!old.idle());
    assert!(new.require_idle_cached().is_err());
    assert_eq!(
        fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME
        ))
        .unwrap(),
        index
    );
    fs::remove_file(stage).unwrap();
    // Must recognize SAME installed3 handler/summary, not reparse selected3 as2.
    upgrade.retry().unwrap();
    drop(old);
    drop(new);
    let mut selected = upgrade.into_store().unwrap();
    assert_eq!(selected.row_count_data().unwrap(), 1);
    drop(selected);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn lifecycle3_strict_cold_open_never_adopts_equal_replacement_after_inspection() {
    use super::super::upgrade_source::owning::{self, at_edge, Edge};
    for edge in [Edge::AfterInspection, Edge::BeforeColdExposure] {
        for name in [
            super::super::super::preparation::INDEX_NAME,
            super::super::super::preparation::MARKER_NAME,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xc411);
            let mounted = mount(dir.path(), &mut rng);
            let pool = lifecycle3_pool();
            let prepared = mounted
                .into_upgrade_source()
                .unwrap()
                .prepare(lifecycle3_policy(pool.clone()), &mut rng)
                .unwrap();
            drop(prepared.admit().unwrap().into_store().unwrap());
            let target = file(dir.path(), name);
            let backup = target.with_extension("original");
            let before = fs::read(&target).unwrap();
            let replaced = Arc::new(AtomicBool::new(false));
            at_edge(
                edge,
                Box::new({
                    let replaced = replaced.clone();
                    let target = target.clone();
                    let before = before.clone();
                    move || match fs::rename(&target, &backup) {
                        Ok(()) => {
                            fs::write(&target, &before).unwrap();
                            replaced.store(true, Ordering::Release);
                        }
                        Err(error) => {
                            assert!(
                                cfg!(windows)
                                    && error.kind() == std::io::ErrorKind::PermissionDenied,
                                "{error}"
                            );
                            assert_eq!(fs::read(&target).unwrap(), before);
                        }
                    }
                }),
            );
            let opened = owning::Store::open(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy(),
                lifecycle3_policy(pool.clone()),
            );
            if replaced.load(Ordering::Acquire) {
                assert!(opened.is_err());
            } else {
                assert!(cfg!(windows));
                assert!(opened.is_ok());
            }
            drop(opened);
            assert_eq!(pool.charged_for_test(), 0);
        }
    }
}

#[test]
fn lifecycle3_strict_cold_open_cleans_only_first_claim_then_refuses_old2() {
    use super::super::upgrade_source::owning;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc408);
    let mounted = mount(dir.path(), &mut rng);
    drop(mounted);
    let catalog = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Current.name());
    let before = fs::read(&catalog).unwrap();
    let seed = dir
        .path()
        .join(crate::store::persistence::native_registry::lifecycle::UPGRADE_SEED_NAME);
    fs::write(&seed, []).unwrap();
    let pool = lifecycle3_pool();
    assert!(owning::Store::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        lifecycle3_policy(pool.clone())
    )
    .is_err());
    assert!(!seed.exists());
    assert_eq!(fs::read(&catalog).unwrap(), before);
    assert_eq!(pool.charged_for_test(), 0);
    let cold = mount::NativeIndexStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy(),
    )
    .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 1);
}

#[test]
fn lifecycle3_strict_cold_open_recovers_every_real_upgrade_cut_without_rng() {
    use super::super::upgrade_source::owning;
    for (number, cut) in [
        io::Fault::UpgradeClaim,
        io::Fault::UpgradeFence,
        io::Fault::UpgradePartialSeed,
        io::Fault::UpgradeSeed,
        io::Fault::UpgradePartialIndex,
        io::Fault::UpgradeIndexTemp,
        io::Fault::UpgradeIndexRename,
        io::Fault::UpgradePartialMarker,
        io::Fault::UpgradeMarkerTemp,
        io::Fault::UpgradeMarkerRename,
        io::Fault::UpgradePartialCatalog,
        io::Fault::UpgradeCatalogRename,
        io::Fault::UpgradeCovered,
        io::Fault::UpgradeSeedUnlink,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc420 + number as u64);
        let mounted = mount(dir.path(), &mut rng);
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let next_index = prepared.output.bytes().index.to_vec();
        let mut upgrade = prepared.admit().unwrap();
        io::arm_fault(cut);
        assert!(upgrade.retry().is_err(), "{cut:?}");
        assert!(io::take_fault_seen(), "{cut:?}");
        drop(upgrade);
        assert_eq!(pool.charged_for_test(), 0);
        let cold = owning::Store::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
            lifecycle3_policy(pool.clone()),
        );
        if number < 3 {
            // These cuts have no complete authenticated intent. Cleanup leaves
            // old2, so strict3 must refuse rather than upgrade implicitly.
            assert!(cold.is_err(), "{cut:?}");
            let cold2 = mount::NativeIndexStore::open(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy(),
                policy(),
            )
            .unwrap();
            assert_eq!(cold2.row_count_data().unwrap(), 1);
        } else {
            let mut cold = cold.unwrap();
            assert_eq!(cold.row_count_data().unwrap(), 1, "{cut:?}");
            assert_eq!(
                fs::read(file(
                    dir.path(),
                    super::super::super::preparation::INDEX_NAME
                ))
                .unwrap(),
                next_index,
                "{cut:?}"
            );
        }
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn lifecycle3_strict_cold_open_never_recreates_missing_selected_output() {
    use super::super::upgrade_source::owning;
    for missing in [
        super::super::super::preparation::INDEX_NAME,
        super::super::super::preparation::MARKER_NAME,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc440);
        let mounted = mount(dir.path(), &mut rng);
        let pool = lifecycle3_pool();
        let prepared = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap();
        let mut upgrade = prepared.admit().unwrap();
        io::arm_fault(io::Fault::UpgradeCatalogRename);
        assert!(upgrade.retry().is_err());
        assert!(io::take_fault_seen());
        drop(upgrade);
        let target = file(dir.path(), missing);
        fs::remove_file(&target).unwrap();
        let seed = dir
            .path()
            .join(crate::store::persistence::native_registry::lifecycle::UPGRADE_SEED_NAME);
        let before_seed = fs::read(&seed).unwrap();
        assert!(owning::Store::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
            lifecycle3_policy(pool.clone())
        )
        .is_err());
        assert!(!target.exists());
        assert_eq!(fs::read(&seed).unwrap(), before_seed);
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn lifecycle3_preparation_refuses_one_byte_short_and_shared_accounts_before_rng() {
    use super::super::super::lifecycle::preparation::{Policy, PREPARATION_FEE};
    assert!(Policy::new(
        CriticalCatalogReadPool::new(PREPARATION_FEE - 1).unwrap(),
        Policy::required_persistent_capacity()
    )
    .is_err());
    assert!(Policy::new(
        lifecycle3_pool(),
        Policy::required_persistent_capacity() - 1
    )
    .is_err());
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc334);
    let mounted = mount(dir.path(), &mut rng);
    let address = Arc::as_ptr(&mounted.physical);
    let pool = lifecycle3_pool();
    let occupied = pool.try_reserve(1).unwrap();
    let source = mounted.into_upgrade_source().unwrap();
    let source = source
        .prepare(lifecycle3_policy(pool.clone()), &mut NeverRng)
        .unwrap_err();
    let mounted = source.into_original_for_test();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert!(mounted.physical.handler.gate.idle());
    assert_eq!(pool.charged_for_test(), 1);
    drop(occupied);
    let shared = mounted
        .physical
        .current
        .cached_store()
        .unwrap()
        .current_layout_policy
        .as_ref()
        .unwrap()
        .archive
        .pending
        .pool
        .clone();
    let before = shared.charged_for_test();
    let mounted = mounted
        .into_upgrade_source()
        .unwrap()
        .prepare(lifecycle3_policy(shared.clone()), &mut NeverRng)
        .unwrap_err()
        .into_original_for_test();
    assert_eq!(address, Arc::as_ptr(&mounted.physical));
    assert_eq!(shared.charged_for_test(), before);
    assert!(mounted.physical.handler.gate.idle());
    // Quota refusal did not consume/close the source; a genuine paid attempt works.
    drop(
        mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut rng)
            .unwrap(),
    );
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn lifecycle3_preparation_refuses_tampered_originals_without_drawing_or_rewriting() {
    for (number, name) in [
        super::super::super::preparation::INDEX_NAME,
        super::super::super::preparation::MARKER_NAME,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc335 + number as u64);
        let mounted = mount(dir.path(), &mut rng);
        let address = Arc::as_ptr(&mounted.physical);
        let name = file(dir.path(), name);
        let mut bytes = fs::read(&name).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&name, &bytes).unwrap();
        let pool = lifecycle3_pool();
        let mounted = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut NeverRng)
            .unwrap_err()
            .into_original_for_test();
        assert_eq!(address, Arc::as_ptr(&mounted.physical));
        assert!(mounted.physical.handler.gate.idle());
        assert_eq!(pool.charged_for_test(), 0);
        assert_eq!(fs::read(name).unwrap(), bytes);
    }
}

#[test]
fn lifecycle3_preparation_preserves_every_unresolved_role_and_old_reader_refusal() {
    use crate::store::persistence::native_registry::lifecycle as new_io;
    for (root, name) in [
        (false, new_io::UPGRADE_SEED_NAME),
        (true, new_io::UPGRADE_INDEX_NAME),
        (true, new_io::UPGRADE_MARKER_NAME),
        (false, crate::store::critical_namespaces::Slot::Stage.name()),
    ] {
        for directory in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xc337);
            let mounted = mount(dir.path(), &mut rng);
            let name = if root {
                file(dir.path(), name)
            } else {
                dir.path().join(name)
            };
            if directory {
                fs::create_dir(&name).unwrap();
            } else {
                fs::write(&name, []).unwrap();
            }
            let pool = lifecycle3_pool();
            let mounted = mounted
                .into_upgrade_source()
                .unwrap()
                .prepare(lifecycle3_policy(pool.clone()), &mut NeverRng)
                .unwrap_err()
                .into_original_for_test();
            assert_eq!(pool.charged_for_test(), 0);
            assert!(mounted.physical.handler.gate.idle());
            assert!(name.exists());
            assert_eq!(name.is_dir(), directory);
            // A storage reader cannot silently ignore future upgrade objects.
            assert!(mounted.row_count_data().is_err());
        }
    }
}

#[test]
fn lifecycle3_preparation_rechecks_close_after_ownership_was_captured() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc338);
    let mounted = mount(dir.path(), &mut rng);
    let gate = mounted.physical.handler.gate.clone();
    let source = mounted.into_upgrade_source().unwrap();
    gate.close();
    let pool = lifecycle3_pool();
    let mounted = source
        .prepare(lifecycle3_policy(pool.clone()), &mut NeverRng)
        .unwrap_err()
        .into_original_for_test();
    assert!(!mounted.physical.handler.gate.idle());
    assert_eq!(pool.charged_for_test(), 0);
}

/// The cut runs through the actual RNG seam after original authentication, at
/// first output sealing. It creates no runtime callback or positive credential.
struct Lifecycle3SealCut<'a> {
    rng: &'a mut ChaCha20Rng,
    cut: Option<Box<dyn FnOnce()>>,
}
impl Lifecycle3SealCut<'_> {
    fn fire(&mut self) {
        if let Some(cut) = self.cut.take() {
            cut();
        }
    }
}
impl rand_core::RngCore for Lifecycle3SealCut<'_> {
    fn next_u32(&mut self) -> u32 {
        self.fire();
        rand_core::RngCore::next_u32(self.rng)
    }
    fn next_u64(&mut self) -> u64 {
        self.fire();
        rand_core::RngCore::next_u64(self.rng)
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.fire();
        rand_core::RngCore::fill_bytes(self.rng, bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fire();
        rand_core::RngCore::try_fill_bytes(self.rng, bytes)
    }
}
impl rand_core::CryptoRng for Lifecycle3SealCut<'_> {}

#[test]
fn lifecycle3_preparation_never_adopts_equal_replacement_after_authentication() {
    for (root, name) in [
        (true, super::super::super::preparation::INDEX_NAME),
        (true, super::super::super::preparation::MARKER_NAME),
        (
            false,
            crate::store::critical_namespaces::Slot::Current.name(),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let retained = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc339);
        let mounted = mount(dir.path(), &mut rng);
        let address = Arc::as_ptr(&mounted.physical);
        let name = if root {
            file(dir.path(), name)
        } else {
            dir.path().join(name)
        };
        let original = fs::read(&name).unwrap();
        let moved = retained.path().join("original-object");
        let fired = Arc::new(AtomicBool::new(false));
        let mut cut = Lifecycle3SealCut {
            rng: &mut rng,
            cut: Some(Box::new({
                let fired = fired.clone();
                let name = name.clone();
                let original = original.clone();
                move || {
                    fs::rename(&name, moved).unwrap();
                    fs::write(&name, original).unwrap();
                    fired.store(true, Ordering::Release);
                }
            })),
        };
        let pool = lifecycle3_pool();
        let mounted = mounted
            .into_upgrade_source()
            .unwrap()
            .prepare(lifecycle3_policy(pool.clone()), &mut cut)
            .unwrap_err()
            .into_original_for_test();
        assert!(fired.load(Ordering::Acquire));
        assert_eq!(address, Arc::as_ptr(&mounted.physical));
        assert!(mounted.physical.handler.gate.idle());
        assert_eq!(pool.charged_for_test(), 0);
        assert_eq!(fs::read(name).unwrap(), original);
        // Before admission no file effect occurred. An explicitly new capture
        // may authenticate this now-stable tuple; the old capture could not.
        drop(
            mounted
                .into_upgrade_source()
                .unwrap()
                .prepare(lifecycle3_policy(pool.clone()), &mut rng)
                .unwrap(),
        );
        assert_eq!(pool.charged_for_test(), 0);
    }
}

#[test]
fn native_index2_cell_concrete_read_keeps_its_decoded_buffers_inside_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc222);
    let mounted = mount(dir.path(), &mut rng);
    // Harness watchdog only, not protocol time. A regression must fail rather
    // than leave scope waiting forever on a reader whose release was skipped.
    let watchdog = std::time::Duration::from_secs(30);
    let (decoded_tx, decoded_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let reader = scope.spawn({
            let physical = mounted.physical.clone();
            move || {
                AFTER_DECODE.with(|slot| {
                    *slot.borrow_mut() = Some(Box::new(move || {
                        decoded_tx.try_send(()).unwrap();
                        release_rx.recv_timeout(watchdog).unwrap();
                    }))
                });
                physical.row_count_data()
            }
        });
        let ready = decoded_rx.recv_timeout(watchdog);
        // Catch even NeverRng's regression panic, then ALWAYS release and join
        // the actual reader before inspecting any outcome or asserting.
        let probes = ready.as_ref().ok().map(|_| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (
                    mounted.row_count_data().is_err(),
                    mounted.lookup_creation_data([92; 32], [93; 32]).is_err(),
                    mounted
                        .physical
                        .prepare_pending_birth(&input(), &mut NeverRng)
                        .is_err(),
                )
            }))
        });
        let _ = release_tx.try_send(());
        let result = reader.join();
        assert!(
            ready.is_ok(),
            "reader never reached its actual decoded buffer"
        );
        assert!(matches!(probes, Some(Ok((true, true, true)))));
        assert!(matches!(result, Ok(Ok(1))));
    });
    assert_eq!(mounted.row_count_data().unwrap(), 1);
}

fn native_host(
    mounted: mount::NativeIndexStore,
) -> (
    NativeTextVault,
    NativeUiInvocationBudget,
    Arc<std::sync::atomic::AtomicU64>,
    Arc<std::sync::atomic::AtomicU64>,
    Arc<AtomicBool>,
) {
    let budget = NativeUiInvocationBudget::new(
        CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap(),
    )
    .unwrap();
    let generation = Arc::new(std::sync::atomic::AtomicU64::new(7));
    let authorized = Arc::new(std::sync::atomic::AtomicU64::new(7));
    let requested = Arc::new(AtomicBool::new(false));
    let vault = NativeTextVault::from_trusted_host(
        mounted,
        generation.clone(),
        authorized.clone(),
        requested.clone(),
        budget.clone(),
    )
    .unwrap();
    (vault, budget, generation, authorized, requested)
}

#[test]
fn native_index2_facade_reauthorizes_ui_on_same_original_physical_mount() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc230);
    let (vault, budget, generation, authorized, requested) =
        native_host(mount(dir.path(), &mut rng));
    let mut candidate = vault
        .capture_ui(7, budget.prepare().unwrap())
        .finish()
        .unwrap();
    let original = vault.admit_ui(&mut candidate).unwrap();
    assert!(vault.require_ui(&original).is_ok());
    generation.store(8, Ordering::SeqCst);
    requested.store(true, Ordering::SeqCst);
    assert!(vault.require_ui(&original).is_err());
    let retirement = vault.begin_ui_retirement(7).unwrap();
    assert_eq!(retirement.state(), NativeTextUiRetirementState::Started);
    futures::executor::block_on(retirement.into_batch().unwrap().retire());
    drop(vault.collect_retired_ui().unwrap());
    assert_eq!(vault.row_count_data().unwrap(), 1); // Lock did not unmount it.
    authorized.store(8, Ordering::SeqCst);
    requested.store(false, Ordering::SeqCst);
    let mut next = vault
        .capture_ui(8, budget.prepare().unwrap())
        .finish()
        .unwrap();
    let fresh = vault.admit_ui(&mut next).unwrap();
    assert!(vault.require_ui(&fresh).is_ok());
    assert!(vault.require_ui(&original).is_err());
}

#[test]
fn native_index2_facade_drop_fences_ui_and_original_index_before_late_metadata_finish() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc231);
    let mounted = mount(dir.path(), &mut rng);
    let physical = mounted.physical.clone();
    let (vault, budget, _, _, _) = native_host(mounted);
    let late = vault.capture_ui(7, budget.prepare().unwrap());
    assert!(physical.handler.gate.idle());
    drop(vault);
    assert!(!physical.handler.gate.idle());
    assert!(physical.handler.gate.claim().is_err());
    assert!(late.finish().is_err());
    // Finite historical DATA custody still grants no new native/key admission.
    assert_eq!(physical.row_count_data().unwrap(), 1);
}

#[test]
fn native_index2_facade_equal_generation_cannot_select_a_replacement_vault() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc232);
    let (first, first_budget, _, _, _) = native_host(mount(left.path(), &mut rng));
    let (other, other_budget, _, _, _) = native_host(mount(right.path(), &mut rng));
    let mut candidate = first
        .capture_ui(7, first_budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(other.admit_ui(&mut candidate).is_err());
    let original = first.admit_ui(&mut candidate).unwrap();
    assert!(other.require_ui(&original).is_err());
    let mut foreign_fee = first
        .capture_ui(7, other_budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(first.admit_ui(&mut foreign_fee).is_err());
}

struct OrderingDropNotice(Arc<AtomicBool>);
impl Drop for OrderingDropNotice {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn native_index2_mounted_legacy_leases_keep_the_original_native_mutex_and_ordering() {
    use crate::store::MountedVault;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc240);
    let store = crate::ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let mounted = Arc::new(tokio::sync::Mutex::new(Some(MountedVault::from(store))));
    for family in 0..3 {
        let notice = Arc::new(AtomicBool::new(false));
        let guard = futures::executor::block_on(mounted.clone().lock_owned());
        let ordering = OrderingDropNotice(notice.clone());
        let lease: Box<dyn Send> = match family {
            0 => Box::new(
                crate::durable_chat::DurableSendLease::new_mounted(guard, 1, ordering).unwrap(),
            ),
            1 => Box::new(crate::shutdown::ShutdownLease::new_mounted(guard, 1, ordering).unwrap()),
            2 => {
                Box::new(crate::studio::StudioVaultLease::new_mounted(guard, 1, ordering).unwrap())
            }
            _ => unreachable!(),
        };
        assert!(mounted.try_lock().is_err());
        assert!(!notice.load(Ordering::SeqCst));
        drop(lease);
        assert!(mounted.try_lock().is_ok());
        assert!(notice.load(Ordering::SeqCst));
    }
}

fn refused_legacy_family(
    family: u8,
    guard: tokio::sync::OwnedMutexGuard<Option<crate::store::MountedVault>>,
    notice: Arc<AtomicBool>,
) -> crate::store::MountedLeaseRefusal {
    let ordering = OrderingDropNotice(notice);
    match family {
        0 => crate::durable_chat::DurableSendLease::new_mounted(guard, 1, ordering)
            .err()
            .unwrap(),
        1 => crate::shutdown::ShutdownLease::new_mounted(guard, 1, ordering)
            .err()
            .unwrap(),
        2 => crate::studio::StudioVaultLease::new_mounted(guard, 1, ordering)
            .err()
            .unwrap(),
        _ => unreachable!(),
    }
}

#[test]
fn native_index2_mounted_absent_lease_refusal_retains_original_custody() {
    let mounted = Arc::new(tokio::sync::Mutex::new(None));
    for family in 0..3 {
        let notice = Arc::new(AtomicBool::new(false));
        let guard = futures::executor::block_on(mounted.clone().lock_owned());
        let refused = refused_legacy_family(family, guard, notice.clone());
        assert_eq!(
            refused.kind(),
            crate::store::MountedLeaseRefusalKind::NotMounted
        );
        assert!(mounted.try_lock().is_err());
        assert!(!notice.load(Ordering::SeqCst));
        drop(refused);
        assert!(mounted.try_lock().is_ok());
        assert!(notice.load(Ordering::SeqCst));
    }
}

#[test]
fn native_index2_mounted_v2_cannot_supply_a_legacy_send_shutdown_or_studio_lease() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc241);
    let (vault, _, _, _, _) = native_host(mount(dir.path(), &mut rng));
    let original = fs::read(file(
        dir.path(),
        super::super::super::preparation::INDEX_NAME,
    ))
    .unwrap();
    let mounted = Arc::new(tokio::sync::Mutex::new(Some(
        crate::store::MountedVault::TextV2(vault),
    )));
    for family in 0..3 {
        let notice = Arc::new(AtomicBool::new(false));
        let guard = futures::executor::block_on(mounted.clone().lock_owned());
        let refused = refused_legacy_family(family, guard, notice.clone());
        assert_eq!(
            refused.kind(),
            crate::store::MountedLeaseRefusalKind::DifferentBackend
        );
        assert_ne!(refused.message(), "The vault is not open.");
        assert!(mounted.try_lock().is_err());
        assert!(!notice.load(Ordering::SeqCst));
        drop(refused);
        assert!(notice.load(Ordering::SeqCst));
        let guard = mounted.try_lock().unwrap();
        let vault = guard.as_ref().unwrap();
        assert!(vault.as_legacy().is_none());
        assert_eq!(vault.as_text_v2().unwrap().row_count_data().unwrap(), 1);
        assert_eq!(
            fs::read(file(
                dir.path(),
                super::super::super::preparation::INDEX_NAME
            ))
            .unwrap(),
            original
        );
    }
}
