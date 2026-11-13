//! Actual opaque v1/v2 owners and real fixed files. These establish storage
//! custody only; no arbitrary fixture supplies native credentials or an SDK.
use super::super::super::preparation::tests::{
    current_policy, initialize, records, NeverRng, SECRET,
};
use super::*;
use crate::store::{NativeRegistryInstallation, NativeRegistryPolicy, ServerStore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, path::Path};

fn policy() -> NativeIndexPolicy {
    NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity(),
    )
    .unwrap()
}
fn original(path: &Path, rng: &mut ChaCha20Rng) -> NativeRegistryStore {
    initialize(path, Some(&records()), rng);
    let policy = NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut installation =
        NativeRegistryInstallation::prepare(path, SECRET, rng, current_policy(), policy).unwrap();
    installation.retry().unwrap();
    installation.into_native_registry().unwrap()
}
fn file(path: &Path, name: &str) -> std::path::PathBuf {
    if name == io::UPGRADE_SEED_NAME
        || name == crate::store::critical_namespaces::Slot::Current.name()
        || name == crate::store::critical_namespaces::Slot::Stage.name()
    {
        path.join(name)
    } else {
        path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
            .join(name)
    }
}

#[test]
fn native_index2_real_opaque_upgrade_and_cold_reopen_keep_legacy_data_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7920);
    let previous = original(dir.path(), &mut rng);
    let legacy = fs::read(dir.path().join("registry.bin")).unwrap();
    let mut upgrade = NativeIndexUpgrade::prepare(previous, policy(), &mut rng).unwrap();
    let index = upgrade.output.index.clone();
    let seed = upgrade.output.seed.clone();
    assert!(!file(dir.path(), io::UPGRADE_SEED_NAME).exists());
    upgrade.retry().unwrap();
    upgrade.retry().unwrap();
    assert_eq!(upgrade.output.seed, seed);
    let mounted = upgrade.into_index_store().unwrap();
    assert_eq!(mounted.row_count_data().unwrap(), 1);
    assert!(mounted
        .lookup_creation_data([1; 32], [2; 32])
        .unwrap()
        .is_none());
    assert_eq!(
        fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
        index.as_slice()
    );
    assert_eq!(fs::read(dir.path().join("registry.bin")).unwrap(), legacy);
    drop(mounted);
    let mounted = NativeIndexStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy(),
    )
    .unwrap();
    assert_eq!(mounted.row_count_data().unwrap(), 1);
    drop(mounted);
    // New source's v1 and ordinary public readers both preserve/refuse v2.
    let before = fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap();
    assert!(ServerStore::open(dir.path(), SECRET, &mut NeverRng).is_err());
    let old_policy = NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap();
    assert!(NativeRegistryStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        old_policy
    )
    .is_err());
    assert_eq!(
        fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
        before
    );
}

#[test]
fn native_index2_policy_payment_refuses_before_rng_or_new_fixed_files() {
    assert!(NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity() - 1).unwrap(),
        NativeIndexPolicy::required_persistent_capacity()
    )
    .is_err());
    assert!(NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity() - 1
    )
    .is_err());
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7921);
    let previous = original(dir.path(), &mut rng);
    let catalog = fs::read(file(
        dir.path(),
        crate::store::critical_namespaces::Slot::Current.name(),
    ))
    .unwrap();
    let native = policy();
    let pressure = native.pool.try_reserve(1).unwrap();
    assert!(NativeIndexUpgrade::prepare(previous, native.clone(), &mut NeverRng).is_err());
    assert!(!file(dir.path(), io::UPGRADE_SEED_NAME).exists());
    assert!(!file(dir.path(), io::UPGRADE_INDEX_NAME).exists());
    assert!(!file(dir.path(), io::UPGRADE_MARKER_NAME).exists());
    assert_eq!(
        fs::read(file(
            dir.path(),
            crate::store::critical_namespaces::Slot::Current.name()
        ))
        .unwrap(),
        catalog
    );
    drop(pressure);
}

#[test]
fn native_index2_unconverted_upgrade_drop_keeps_files_frames_and_mount_paid() {
    for written in [false, true] {
        observe_actual_retirement(written, false);
    }
}

#[test]
fn native_index2_retained_claim_keeps_exact_payment_after_original_owner_close() {
    observe_actual_retirement(true, true);
}

fn observe_actual_retirement(written: bool, keep_claim: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7930 + u64::from(written));
    let previous = original(dir.path(), &mut rng);
    let native = policy();
    let account = native.pool.clone();
    let mut upgrade = NativeIndexUpgrade::prepare(previous, native, &mut rng).unwrap();
    if written {
        upgrade.retry().unwrap();
    }
    let refunded = upgrade.handler.fee.refund_notice();
    let io_done = Arc::new(AtomicBool::new(false));
    let io_valid = Arc::new(AtomicBool::new(false));
    let frames_done = Arc::new(AtomicBool::new(false));
    let frames_valid = Arc::new(AtomicBool::new(false));
    let fee_valid = Arc::new(AtomicBool::new(false));
    let original_mount = upgrade
        .current
        .cached_store()
        .unwrap()
        .chat_mount
        .observer();
    assert!(!original_mount.is_cancelled());
    upgrade.io.observe_retirement(Box::new({
        let refunded = refunded.clone();
        let done = io_done.clone();
        let valid = io_valid.clone();
        let mount = upgrade
            .current
            .cached_store()
            .unwrap()
            .chat_mount
            .observer();
        move || {
            // The observation is the LAST field after real held File/input
            // destructors; no synthetic "work is gone" flag enters production.
            valid.store(
                !refunded.load(Ordering::SeqCst) && !mount.is_cancelled(),
                Ordering::SeqCst,
            );
            done.store(true, Ordering::SeqCst);
        }
    }));
    upgrade.output.retirement = Some(io::RetirementObservation::new(Box::new({
        let refunded = refunded.clone();
        let io_done = io_done.clone();
        let done = frames_done.clone();
        let valid = frames_valid.clone();
        let mount = upgrade
            .current
            .cached_store()
            .unwrap()
            .chat_mount
            .observer();
        move || {
            valid.store(
                io_done.load(Ordering::SeqCst)
                    && !refunded.load(Ordering::SeqCst)
                    && !mount.is_cancelled(),
                Ordering::SeqCst,
            );
            done.store(true, Ordering::SeqCst);
        }
    })));
    upgrade.handler.fee.observe_before_refund(Box::new({
        let io_done = io_done.clone();
        let frames_done = frames_done.clone();
        let valid = fee_valid.clone();
        let mount = upgrade
            .current
            .cached_store()
            .unwrap()
            .chat_mount
            .observer();
        move || {
            valid.store(
                io_done.load(Ordering::SeqCst)
                    && frames_done.load(Ordering::SeqCst)
                    && mount.is_cancelled(),
                Ordering::SeqCst,
            );
        }
    }));
    // This private negative-only claim proves exact payment lifetime, not
    // production native/index authorization. Admission coverage is separate.
    let claim = if keep_claim {
        let catalog = upgrade
            .current
            .cached_store()
            .unwrap()
            .pending_join_catalog
            .lock()
            .unwrap();
        let claim = upgrade.handler.gate.claim().unwrap();
        drop(catalog);
        Some(claim)
    } else {
        None
    };
    drop(upgrade);
    assert!(io_done.load(Ordering::SeqCst) && io_valid.load(Ordering::SeqCst));
    assert!(frames_done.load(Ordering::SeqCst) && frames_valid.load(Ordering::SeqCst));
    assert!(original_mount.is_cancelled());
    if let Some(claim) = claim {
        assert!(!refunded.load(Ordering::SeqCst));
        assert!(account.try_reserve(OPERATIONAL_FEE).is_err());
        assert_eq!(claim.finish().unwrap(), interlock::ClaimRelease::Closed);
    }
    assert!(refunded.load(Ordering::SeqCst) && fee_valid.load(Ordering::SeqCst));
    drop(account.try_reserve(OPERATIONAL_FEE).unwrap());
}

const CUTS: [io::Fault; 14] = [
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
];

fn interrupted_upgrade(path: &Path, rng: &mut ChaCha20Rng, point: io::Fault) -> NativeIndexUpgrade {
    let previous = original(path, rng);
    let mut upgrade = NativeIndexUpgrade::prepare(previous, policy(), rng).unwrap();
    io::arm_fault(point);
    assert!(upgrade.retry().is_err(), "cut {point:?}");
    assert!(io::take_fault_seen(), "dead cut {point:?}");
    upgrade
}
fn reopened(path: &Path) -> Result<NativeIndexStore, AppError> {
    NativeIndexStore::open(path, SECRET, &mut NeverRng, current_policy(), policy())
}
fn tree(path: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn walk(base: &Path, at: &Path, result: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, result);
            } else {
                result.push((
                    path.strip_prefix(base).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut result = Vec::new();
    walk(path, path, &mut result);
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

#[test]
fn native_index2_every_real_upgrade_cut_retries_original_cached_frames() {
    for (number, point) in CUTS.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7950 + number as u64);
        let mut upgrade = interrupted_upgrade(dir.path(), &mut rng, point);
        let seed = upgrade.output.seed.clone();
        let index = upgrade.output.index.clone();
        upgrade
            .retry()
            .unwrap_or_else(|error| panic!("warm {point:?}: {error:?}"));
        assert_eq!(upgrade.output.seed, seed);
        assert_eq!(
            fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
            index.as_slice()
        );
        assert_eq!(
            upgrade
                .into_index_store()
                .unwrap()
                .row_count_data()
                .unwrap(),
            1
        );
    }
}

#[test]
fn native_index2_complete_upgrade_seed_repairs_every_cold_cut_without_rng() {
    for (number, point) in CUTS.into_iter().enumerate().skip(3) {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7970 + number as u64);
        let upgrade = interrupted_upgrade(dir.path(), &mut rng, point);
        let index = upgrade.output.index.clone();
        drop(upgrade);
        let mounted =
            reopened(dir.path()).unwrap_or_else(|error| panic!("cold {point:?}: {error:?}"));
        assert_eq!(mounted.row_count_data().unwrap(), 1);
        assert_eq!(
            fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
            index.as_slice()
        );
        for name in [
            io::UPGRADE_SEED_NAME,
            io::UPGRADE_INDEX_NAME,
            io::UPGRADE_MARKER_NAME,
            crate::store::critical_namespaces::Slot::Stage.name(),
        ] {
            assert!(!file(dir.path(), name).exists());
        }
    }
}

#[test]
fn native_index2_first_torn_upgrade_abandons_no_data_and_can_upgrade_fresh() {
    for (number, point) in CUTS.into_iter().take(3).enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7990 + number as u64);
        let upgrade = interrupted_upgrade(dir.path(), &mut rng, point);
        let old = upgrade.output.old_index.clone();
        drop(upgrade);
        // This API opens ALREADY-v2 only. It cleans known pre-content scratch,
        // then refuses the retained v1 rather than manufacturing a v2 owner.
        assert!(reopened(dir.path()).is_err());
        assert!(!file(dir.path(), io::UPGRADE_SEED_NAME).exists());
        assert!(!file(
            dir.path(),
            crate::store::critical_namespaces::Slot::Stage.name()
        )
        .exists());
        assert_eq!(
            fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
            old.as_slice()
        );
        let old_policy = NativeRegistryPolicy::new(
            CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
            NativeRegistryPolicy::required_persistent_capacity(),
        )
        .unwrap();
        let previous = NativeRegistryStore::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
            old_policy,
        )
        .unwrap();
        let mut fresh = NativeIndexUpgrade::prepare(previous, policy(), &mut rng).unwrap();
        fresh.retry().unwrap();
        assert_eq!(
            fresh.into_index_store().unwrap().row_count_data().unwrap(),
            1
        );
    }
}

#[test]
fn native_index2_authenticated_empty_and_future_upgrade_seeds_are_preserved() {
    for (number, body) in [&[][..], &b"future upgrade grammar"[..]]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x79b0 + number as u64);
        let upgrade = interrupted_upgrade(dir.path(), &mut rng, io::Fault::UpgradeFence);
        let key = Zeroizing::new(
            upgrade
                .current
                .cached_store()
                .unwrap()
                .keys
                .db_key()
                .unwrap(),
        );
        drop(upgrade);
        let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, &key[..]));
        let raw = frame(&seal(&derived, body, &mut rng).unwrap());
        fs::write(file(dir.path(), io::UPGRADE_SEED_NAME), raw).unwrap();
        let before = tree(dir.path());
        assert!(reopened(dir.path()).is_err());
        assert_eq!(tree(dir.path()), before);
    }
}

#[test]
fn native_index2_promoted_catalog_never_recreates_missing_committed_output() {
    for (number, name) in [previous::INDEX_NAME, previous::MARKER_NAME]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x79c0 + number as u64);
        drop(interrupted_upgrade(
            dir.path(),
            &mut rng,
            io::Fault::UpgradeCatalogRename,
        ));
        fs::remove_file(file(dir.path(), name)).unwrap();
        let before = tree(dir.path());
        assert!(reopened(dir.path()).is_err());
        assert_eq!(tree(dir.path()), before);
    }
}

#[test]
fn native_index2_upgrade_last_target_proofs_preserve_late_equal_replacements() {
    for (number, (target, name)) in [
        (io::RenameTarget::UpgradeIndex, previous::INDEX_NAME),
        (io::RenameTarget::UpgradeMarker, previous::MARKER_NAME),
        (
            io::RenameTarget::UpgradeCatalog,
            crate::store::critical_namespaces::Slot::Current.name(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x79d0 + number as u64);
        let previous = original(dir.path(), &mut rng);
        let mut upgrade = NativeIndexUpgrade::prepare(previous, policy(), &mut rng).unwrap();
        let destination = file(dir.path(), name);
        let foreign = fs::read(&destination).unwrap();
        io::before_target_proof(
            target,
            Box::new({
                let destination = destination.clone();
                let bytes = foreign.clone();
                move || {
                    let replacement = destination.with_extension("equal-foreign");
                    fs::write(&replacement, bytes).unwrap();
                    fs::rename(replacement, destination).unwrap();
                }
            }),
        );
        assert!(upgrade.retry().is_err());
        assert!(io::take_rename_hook_seen());
        assert_eq!(fs::read(destination).unwrap(), foreign);
    }
}

fn mounted(path: &Path, rng: &mut ChaCha20Rng) -> NativeIndexStore {
    let previous = original(path, rng);
    let mut upgrade = NativeIndexUpgrade::prepare(previous, policy(), rng).unwrap();
    upgrade.retry().unwrap();
    upgrade.into_index_store().unwrap()
}

#[test]
fn native_index2_owner_close_precedes_a_late_private_physical_loan() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7a31);
    let mounted = mounted(dir.path(), &mut rng);
    // Model the actual storage loan a future admitted native worker will hold.
    // It receives no AdmissionOwner; cloning physical custody cannot clone close
    // ownership or reopen the original interlock after the facade disappears.
    let physical = Arc::clone(&mounted.physical);
    let gate = physical.handler.gate.clone();
    let refunded = physical.handler.fee.refund_notice();
    let mount = physical
        .current
        .cached_store()
        .unwrap()
        .chat_mount
        .observer();
    assert!(gate.idle());
    drop(mounted);
    assert!(!gate.idle());
    assert!(gate.claim().is_err());
    assert!(!refunded.load(Ordering::Acquire));
    // Physical custody is intentionally still live for finite admitted DATA
    // completion. It grants no native/key/source authority, and is not an API
    // available to callers of the opaque public facade.
    assert!(!mount.is_cancelled());
    {
        assert_eq!(physical.row_count_data().unwrap(), 1);
    }
    assert!(NativeIndexStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy()
    )
    .is_err());
    drop(physical);
    assert!(mount.is_cancelled());
    // Even the remaining negative metadata retains its own exact invoice.
    assert!(!refunded.load(Ordering::Acquire));
    drop(gate);
    assert!(refunded.load(Ordering::Acquire));
    assert_eq!(reopened(dir.path()).unwrap().row_count_data().unwrap(), 1);
}
fn pending_data(
    mounted: &NativeIndexStore,
    rng: &mut ChaCha20Rng,
) -> (io::IndexCommitIo, Zeroizing<Vec<u8>>) {
    let store = mounted.physical.current.checked_store().unwrap();
    let snapshot = io::IndexSnapshot::capture(store).unwrap();
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let cap = capability_data(
        &catalog_plain(&key, snapshot.catalog()).unwrap(),
        &store.chat_vault_domain().unwrap(),
    )
    .unwrap();
    let plain = authenticate(&key, INDEX_KEY, snapshot.index(), data::MAX_FRAME).unwrap();
    let old = data::Index::decode_data(&plain, cap).unwrap();
    let mut next = data::Index::decode_data(&plain, cap).unwrap();
    next.generation += 1;
    next.parent = Some(*blake3::hash(snapshot.index()).as_bytes());
    next.last_issued_id += 1;
    next.rows.push(data::Row {
        id: next.last_issued_id,
        identity: [40; 32],
        name: "storage fixture".into(),
        is_dm: false,
        backend: data::Backend::Pending {
            birth: data::Birth {
                invocation: [41; 32],
                request: [42; 32],
                signer: [43; 32],
                network: [44; 32],
            },
            // Grammar fixture ONLY: this is not a network reservation or a native
            // authoring credential, and can never establish actual SDK admission.
            network_frame: Zeroizing::new(vec![45; 40]),
        },
    });
    assert_eq!(
        old.successor_data(&next, next.parent.unwrap()).unwrap(),
        data::Change::Birth
    );
    let frame = sealed(
        &key,
        INDEX_KEY,
        &next.encode_data().unwrap(),
        data::MAX_FRAME,
        rng,
    )
    .unwrap();
    (io::IndexCommitIo::prepare(snapshot, &frame).unwrap(), frame)
}

#[test]
fn native_index2_admitted_data_debt_finishes_after_its_affine_owner_closes() {
    for (number, cut) in [io::Fault::IndexStage, io::Fault::IndexRename]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7a40 + number as u64);
        let mounted = mounted(dir.path(), &mut rng);
        let physical = Arc::clone(&mounted.physical);
        let store = physical.current.checked_store().unwrap();
        let (mut commit, frame) = pending_data(&mounted, &mut rng);
        let claim = {
            let _catalog = store.pending_join_catalog.lock().unwrap();
            physical.handler.gate.claim().unwrap()
        };
        io::arm_fault(cut);
        assert!(commit.commit_index(store, &frame).is_err());
        assert!(io::take_fault_seen());
        drop(mounted);
        assert!(physical.handler.gate.claim().is_err());
        // Preserve the exact admitted file/ciphertext/phase through Stage or
        // post-rename uncertainty. Closing admission does not revoke physical
        // custody that an already admitted finite DATA effect needs to finish.
        commit.commit_index(store, &frame).unwrap();
        assert_eq!(
            claim.finish().unwrap(),
            super::super::interlock::ClaimRelease::Closed
        );
        assert!(store.require_native_index_idle_cached().is_err());
        assert_eq!(
            fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
            *frame
        );
        drop(commit);
        drop(physical);
        let recovered = reopened(dir.path()).unwrap();
        assert_eq!(recovered.row_count_data().unwrap(), 2);
        assert_eq!(
            recovered
                .lookup_creation_data([41; 32], [42; 32])
                .unwrap()
                .unwrap()
                .state,
            super::super::NativeCreationStateData::Pending
        );
    }
}

#[test]
fn native_index2_index_last_target_proof_preserves_late_equal_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x79e0);
    let mounted = mounted(dir.path(), &mut rng);
    let (mut commit, frame) = pending_data(&mounted, &mut rng);
    let destination = file(dir.path(), previous::INDEX_NAME);
    let foreign = fs::read(&destination).unwrap();
    io::before_target_proof(
        io::RenameTarget::Index,
        Box::new({
            let destination = destination.clone();
            let bytes = foreign.clone();
            move || {
                let replacement = destination.with_extension("equal-foreign");
                fs::write(&replacement, bytes).unwrap();
                fs::rename(replacement, destination).unwrap();
            }
        }),
    );
    assert!(commit
        .commit_index(mounted.physical.current.checked_store().unwrap(), &frame)
        .is_err());
    assert!(io::take_rename_hook_seen());
    assert_eq!(fs::read(destination).unwrap(), foreign);
}

#[test]
fn native_index2_real_index_stage_and_selected_cut_recover_data_without_sdk_or_rng() {
    for (number, cut) in [io::Fault::IndexStage, io::Fault::IndexRename]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x79f0 + number as u64);
        let mounted = mounted(dir.path(), &mut rng);
        let (mut commit, frame) = pending_data(&mounted, &mut rng);
        io::arm_fault(cut);
        assert!(commit
            .commit_index(mounted.physical.current.checked_store().unwrap(), &frame)
            .is_err());
        assert!(io::take_fault_seen());
        drop(commit);
        drop(mounted);
        let mounted = reopened(dir.path()).unwrap();
        assert_eq!(mounted.row_count_data().unwrap(), 2);
        assert_eq!(
            mounted
                .lookup_creation_data([41; 32], [42; 32])
                .unwrap()
                .unwrap()
                .state,
            super::super::NativeCreationStateData::Pending
        );
        assert_eq!(
            fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap(),
            frame.as_slice()
        );
        assert!(!file(dir.path(), previous::INDEX_STAGE_NAME).exists());
    }
}

#[test]
fn native_index2_unlink_refusal_retains_original_cleanup_and_excludes_join_until_barrier() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7a00);
    let mounted = mounted(dir.path(), &mut rng);
    let store = mounted.physical.current.checked_store().unwrap();
    fs::write(
        file(dir.path(), previous::INDEX_STAGE_NAME),
        b"torn index ciphertext",
    )
    .unwrap();
    let snapshot = io::IndexSnapshot::capture(store).unwrap();
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    assert!(authenticate(&key, INDEX_KEY, snapshot.stage().unwrap(), data::MAX_FRAME).is_err());
    let mut cleanup = io::IndexStageCleanup::prepare_unauthenticated(snapshot).unwrap();
    let catalog = store.pending_join_catalog.lock().unwrap();
    let claim = mounted.physical.handler.gate.claim().unwrap();
    drop(catalog);
    io::arm_fault(io::Fault::IndexStageUnlink);
    assert!(cleanup.run(store).is_err());
    assert!(io::take_fault_seen());
    assert!(!file(dir.path(), previous::INDEX_STAGE_NAME).exists());
    assert!(store.require_native_index_idle_cached().is_err());
    cleanup.run(store).unwrap();
    // Clearing the flag before that retry's real covering barrier would let JOIN
    // overtake an unlink that can still reappear after a crash.
    assert!(store.require_native_index_idle_cached().is_err());
    claim.finish().unwrap();
    assert!(store.require_native_index_idle_cached().is_ok());
}

#[test]
fn native_index2_join_compatible_duplicate_stage_settles_but_dual_stages_stay_untouched() {
    for dual in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7a10 + u64::from(dual));
        let mounted = mounted(dir.path(), &mut rng);
        let catalog = fs::read(file(
            dir.path(),
            crate::store::critical_namespaces::Slot::Current.name(),
        ))
        .unwrap();
        fs::write(
            file(
                dir.path(),
                crate::store::critical_namespaces::Slot::Stage.name(),
            ),
            catalog,
        )
        .unwrap();
        if dual {
            let index = fs::read(file(dir.path(), previous::INDEX_NAME)).unwrap();
            fs::write(file(dir.path(), previous::INDEX_STAGE_NAME), index).unwrap();
        }
        drop(mounted);
        let before = tree(dir.path());
        if dual {
            assert!(reopened(dir.path()).is_err());
            assert_eq!(tree(dir.path()), before);
        } else {
            assert_eq!(reopened(dir.path()).unwrap().row_count_data().unwrap(), 1);
            assert!(!file(
                dir.path(),
                crate::store::critical_namespaces::Slot::Stage.name()
            )
            .exists());
        }
    }
}

#[test]
fn native_index2_authenticated_empty_and_future_index_stages_are_preserved() {
    for (number, body) in [&[][..], &b"future index grammar"[..]]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7a20 + number as u64);
        let mounted = mounted(dir.path(), &mut rng);
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
        drop(mounted);
        let derived = Zeroizing::new(blake3::derive_key(INDEX_KEY, &key[..]));
        fs::write(
            file(dir.path(), previous::INDEX_STAGE_NAME),
            frame(&seal(&derived, body, &mut rng).unwrap()),
        )
        .unwrap();
        let before = tree(dir.path());
        assert!(reopened(dir.path()).is_err());
        assert_eq!(tree(dir.path()), before);
    }
}
