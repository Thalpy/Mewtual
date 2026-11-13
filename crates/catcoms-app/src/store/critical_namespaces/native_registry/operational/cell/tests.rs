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
