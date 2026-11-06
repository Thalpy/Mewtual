//! Genuine saved source -> new ID3 physical mount -> independently paid SDK -> actual original
//! group task. These are UNRUN until Root's one serial pipeline integrates the joint modules.
//! No raw Source/Task/SavedDirectory/watch/status is substituted for its real constructor.
use super::*;
use crate::store::critical_namespaces::{
    archive_bootstrap as archive_codec, archive_current_namespace_codec as layout_codec,
};
use crate::store::{
    ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy, GenesisSeedBudget,
    RetainedBootstrapGenesisSeed,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::owner_v2::FounderSnapshotLimits;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

pub(super) const SECRET: &[u8] = b"actual original group namespace";
const CAPACITY: usize = 1024 * 1024 * 1024;
struct Fixture {
    layout: V2CurrentLayoutStore,
    seed: RetainedBootstrapGenesisSeed,
    pool: CriticalCatalogReadPool,
    group: Vec<u8>,
    signer: [u8; 32],
    // Every File/resource holder drains before the directory is removed by the harness.
    _path: tempfile::TempDir,
}
fn sdk_budget() -> GenesisSeedBudget {
    GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap()
}
pub(super) fn archive(pool: &CriticalCatalogReadPool) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * archive_codec::MAX_SEED_FRAMED)
        .unwrap()
        .with_bootstrap_source_data_read()
}
fn fixture(number: u64) -> Fixture {
    let path = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(number);
    let pool = CriticalCatalogReadPool::new(CAPACITY).unwrap();
    drop(
        ServerStore::open(path.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let mut first =
        PreparedArchiveBootstrapStore::open(path.path(), SECRET, &mut rng, archive(&pool)).unwrap();
    first.retry(&mut rng).unwrap();
    let store = first.into_store().unwrap();
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let mut creator = match store
        .prepare_archive_founder(
            [91; 32],
            "actual saved group origin",
            ArchiveFounderPolicy::new(
                limits,
                3,
                archive_codec::founder_codec::PERSISTENT_CONTENT_BYTES,
            )
            .unwrap(),
            ArchiveFounderInputs::new(
                Hub::new().join(PeerId::from_u64(number)),
                device,
                ChaCha20Rng::seed_from_u64(number),
                Box::new(ManualClock::new(10)),
            ),
        )
        .unwrap()
    {
        ArchiveFounderBegin::Original(owner) => owner,
        _ => panic!("original actual creator required"),
    };
    creator.advance(&store, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&store).unwrap();
    plan.save_founding_intent(&store, &mut rng).unwrap();
    let mut source = plan.into_source_preparation(&store).unwrap();
    source.save_source_terminal(&store, &mut rng).unwrap();
    drop(source);
    drop(store);
    let policy = V2CurrentLayoutPolicy::new(
        archive(&pool),
        CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * layout_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path.path(), SECRET, &mut rng, policy).unwrap();
    owner.retry(&mut rng).unwrap();
    let layout = owner.into_current_layout_store().unwrap();
    let read = layout
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    let seed = layout
        .checked_store()
        .unwrap()
        .prepare_genesis_seed(read, sdk_budget())
        .unwrap();
    Fixture {
        layout,
        seed,
        pool,
        group,
        signer,
        _path: path,
    }
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("negative path reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("negative path reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("negative path reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("negative path reached RNG")
    }
}
impl CryptoRng for NeverRng {}
fn key(group: &[u8], signer: [u8; 32]) -> crate::store::ChatAdmissionKey {
    crate::store::ChatAdmissionKey::new(group, signer).unwrap()
}
fn fault(store: &ServerStore, point: u8) {
    store
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}

// Real independently saved origins for the bounded-capacity test. A fresh physical reopen
// between creations drains the older five-entry warm source registry; it does not erase any
// saved source files, creation debit or SDK attempt. No task/group/source labels are forged.
pub(super) fn additional_saved_origin(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    rng: &mut ChaCha20Rng,
    number: u64,
    invocation: u8,
) -> (Vec<u8>, [u8; 32]) {
    let mut owner = PreparedArchiveBootstrapStore::open(path, SECRET, rng, archive(pool)).unwrap();
    owner.retry(rng).unwrap();
    let store = owner.into_store().unwrap();
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let mut creator = match store
        .prepare_archive_founder(
            [invocation; 32],
            "actual bounded group pool origin",
            ArchiveFounderPolicy::new(
                FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap(),
                3,
                archive_codec::founder_codec::PERSISTENT_CONTENT_BYTES,
            )
            .unwrap(),
            ArchiveFounderInputs::new(
                Hub::new().join(PeerId::from_u64(number)),
                device,
                ChaCha20Rng::seed_from_u64(number),
                Box::new(ManualClock::new(10)),
            ),
        )
        .unwrap()
    {
        ArchiveFounderBegin::Original(owner) => owner,
        _ => panic!("actual original capacity source required"),
    };
    creator.advance(&store, rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&store).unwrap();
    plan.save_founding_intent(&store, rng).unwrap();
    let mut source = plan.into_source_preparation(&store).unwrap();
    source.save_source_terminal(&store, rng).unwrap();
    drop(source);
    drop(store);
    (group, signer)
}

#[test]
fn real_registration_excludes_all_signers_before_mkdir_and_after_task_drop() {
    let f = fixture(0x3b01);
    let store = f.layout.checked_store().unwrap();
    let old = store
        .capture_chat_admission(key(&f.group, [0x92; 32]))
        .unwrap();
    let baseline = f.pool.charged_for_test();
    let task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    assert!(store
        .capture_chat_admission(key(&f.group, f.signer))
        .is_err());
    assert!(store
        .capture_chat_admission(key(&f.group, [0x93; 32]))
        .is_err());
    assert!(store
        .prepare_legacy_chat_admission(&old, b"old captured mirror", &mut NeverRng)
        .is_err());
    assert!(store
        .capture_chat_admission(key(b"unrelated original", [0x94; 32]))
        .is_ok());
    assert!(f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .is_err());
    let observation = task.observe_original();
    let paid = f.pool.charged_for_test();
    assert!(paid > baseline);
    assert!(task.saved_directory().is_err());
    drop(task);
    assert!(store
        .capture_chat_admission(key(&f.group, [0x95; 32]))
        .is_err());
    assert!(
        f.pool.charged_for_test() > baseline,
        "inert slot/output/fee survives owner Drop"
    );
    drop(observation);
}

#[test]
fn actual_directory_parent_barriers_release_slot_but_preserve_original_task_and_permanent_fence() {
    let f = fixture(0x3b02);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    let original = task.observe_original();
    task.retry_directory(store, &f.seed.borrow_source_data())
        .unwrap();
    let saved = task.saved_directory().unwrap().clone();
    let (path, held, identity) = saved.directory_data();
    assert!(path.is_dir());
    assert_eq!(catcoms_storage::kept::session_id(held).unwrap(), identity);
    original.require_same_original(&task).unwrap();
    store
        .with_vacant_v2_genesis_admission(&f.seed.borrow_source_data(), |admission| {
            let guard = task.lock_vacant(admission)?;
            guard.directory().validate_under_archive_root(
                store,
                guard.source(),
                &task,
                guard.archive_root(),
            )
        })
        .unwrap();
    task.retry_directory(store, &f.seed.borrow_source_data())
        .unwrap();
    assert!(
        store.require_no_pending_v2_group(&f.group).is_ok(),
        "actual permanent fence takes over scarce slot"
    );
    assert!(store
        .capture_chat_admission(key(&f.group, [0xa1; 32]))
        .is_err());
    drop(task);
    assert!(store
        .capture_chat_admission(key(&f.group, [0xa2; 32]))
        .is_err());
    assert!(
        path.is_dir(),
        "negative permanent group directory is never deleted"
    );
}

#[test]
fn first_open_uncertainty_is_held_and_same_retained_file_later_failures_retry() {
    for point in [251, 252, 253, 254] {
        let f = fixture(0x3b10 + point as u64);
        let store = f.layout.checked_store().unwrap();
        let mut task = f
            .layout
            .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
            .unwrap();
        fault(store, point);
        assert!(task
            .retry_directory(store, &f.seed.borrow_source_data())
            .is_err());
        assert!(task.saved_directory().is_err());
        fault(store, 0);
        let retry = task.retry_directory(store, &f.seed.borrow_source_data());
        if point == 251 {
            assert!(
                retry.is_err(),
                "no path-only adoption before first held File"
            );
            assert!(store.require_no_pending_v2_group(&f.group).is_err());
        } else {
            retry.unwrap();
            assert!(task.saved_directory().is_ok());
            assert!(store.require_no_pending_v2_group(&f.group).is_ok());
        }
        drop(task);
        assert!(store
            .capture_chat_admission(key(&f.group, [0xb1; 32]))
            .is_err());
    }
}

#[test]
fn another_paid_read_and_sdk_cannot_retarget_original_directory_task() {
    let f = fixture(0x3b21);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    let another = f
        .layout
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let another = store.prepare_genesis_seed(another, sdk_budget()).unwrap();
    assert_eq!(f.seed.data().source, another.data().source);
    assert!(task
        .retry_directory(store, &another.borrow_source_data())
        .is_err());
    assert!(task.saved_directory().is_err());
    task.retry_directory(store, &f.seed.borrow_source_data())
        .unwrap();
}

#[test]
fn group_payment_one_byte_short_has_no_registration_or_first_directory_effect() {
    let f = fixture(0x3b22);
    let baseline = f.pool.charged_for_test();
    let pressure = f
        .pool
        .try_reserve(CAPACITY - baseline - GROUP_TASK_FEE + 1)
        .unwrap();
    assert!(f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .is_err());
    let store = f.layout.checked_store().unwrap();
    assert!(store.require_no_pending_v2_group(&f.group).is_ok());
    assert!(store
        .capture_chat_admission(key(&f.group, [0xc1; 32]))
        .is_ok());
    drop(pressure);
    assert_eq!(f.pool.charged_for_test(), baseline);
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    task.retry_directory(store, &f.seed.borrow_source_data())
        .unwrap();
}

#[test]
fn saved_original_directory_replacement_never_becomes_same_task_namespace() {
    let f = fixture(0x3b23);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    task.retry_directory(store, &f.seed.borrow_source_data())
        .unwrap();
    let path = task
        .saved_directory()
        .unwrap()
        .directory_data()
        .0
        .to_path_buf();
    fs::rename(&path, path.with_file_name("held-original-v2-group")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store
        .with_vacant_v2_genesis_admission(&f.seed.borrow_source_data(), |admission| {
            task.lock_vacant(admission).map(|_| ())
        })
        .is_err());
    assert!(store
        .capture_chat_admission(key(&f.group, [0xc2; 32]))
        .is_err());
}

#[test]
fn directory_changed_during_last_full_proof_cannot_retire_original_negative_slot() {
    for replace in [false, true] {
        let f = fixture(0x3b31 + u64::from(replace));
        let store = f.layout.checked_store().unwrap();
        let mut task = f
            .layout
            .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
            .unwrap();
        let path = task
            .directory_output()
            .unwrap()
            .path_for_test()
            .to_path_buf();
        // Obtain the genuine engine + collector outcome WITHOUT performing the later slot
        // handoff. No raw file/status/receipt constructor is substituted for a saved result.
        store
            .with_vacant_v2_genesis_admission(&f.seed.borrow_source_data(), |admission| {
                let guard = task.lock_creating(admission)?;
                store.write_v2_genesis_group_directory(&guard)
            })
            .unwrap();
        task.set_warm_saved_directory(
            task.directory_output()
                .unwrap()
                .collect_saved_directory(&task)
                .unwrap(),
        )
        .unwrap();
        let changed = path.clone();
        let retired =
            store.with_vacant_v2_genesis_admission(&f.seed.borrow_source_data(), |admission| {
                let mut guard = task.lock_vacant(admission)?;
                // The initial full validation is already complete. Arm ONLY the subsequent
                // receipt proof inside the actual retirement operation, at the former gap.
                *store
                    .owner_evidence
                    .before_v2_saved_group_final_directory_proof
                    .lock()
                    .unwrap() = Some(Box::new(move || {
                    fs::rename(
                        &changed,
                        changed.with_file_name("held-during-final-group-proof"),
                    )
                    .unwrap();
                    if replace {
                        fs::create_dir(&changed).unwrap();
                    }
                }));
                let root = guard.root.as_ref().ok_or_else(refused)?;
                guard.owner.retire_saved_directory(guard.directory, root)
            });
        assert!(retired.is_err());
        assert!(
            task.saved_directory().is_ok(),
            "actual earlier barrier DATA stays retained"
        );
        assert!(
            store.require_no_pending_v2_group(&f.group).is_err(),
            "no stale permanent-fence handoff"
        );
        assert!(store
            .capture_chat_admission(key(&f.group, [0xd1; 32]))
            .is_err());
        drop(task);
        assert!(store.require_no_pending_v2_group(&f.group).is_err());
        assert!(store
            .capture_chat_admission(key(&f.group, [0xd2; 32]))
            .is_err());
        assert_eq!(path.exists(), replace);
    }
}

#[test]
fn directory_changed_after_full_id3_proof_cannot_be_recorded_saved() {
    let f = fixture(0x3b33);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    let changed = task
        .directory_output()
        .unwrap()
        .path_for_test()
        .to_path_buf();
    *store
        .owner_evidence
        .before_v2_group_final_directory_proof
        .lock()
        .unwrap() = Some(Box::new(move || {
        fs::rename(&changed, changed.with_file_name("held-before-group-saved")).unwrap();
    }));
    assert!(task
        .retry_directory(store, &f.seed.borrow_source_data())
        .is_err());
    assert!(task.saved_directory().is_err());
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
}

#[test]
fn concurrent_genuine_same_source_constructors_have_only_one_original_task() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let f = Arc::new(fixture(0x3b41));
    let store = f.layout.checked_store().unwrap();
    let (workers, done_rx) = store
        .chat_mount
        .with_writer_gate_for_test(|| {
            let (started_tx, started_rx) = channel();
            let (done_tx, done_rx) = channel();
            let mut workers = Vec::new();
            for _ in 0..2 {
                let owned = f.clone();
                let started = started_tx.clone();
                let done = done_tx.clone();
                // Owned Arc fixture permits detaching on a timeout/panic; there is no scoped-thread
                // unwind join that could hang the mandatory suite behind a regressed mutex path.
                workers.push(std::thread::spawn(move || {
                    started.send(()).unwrap();
                    let result = owned
                        .layout
                        .prepare_genesis_group_namespace(&owned.seed.borrow_source_data());
                    done.send(result).unwrap();
                }));
            }
            drop(started_tx);
            drop(done_tx);
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            (workers, done_rx)
        })
        .unwrap();
    let first = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "exactly one task may install this original group"
    );
    let task = first.or(second).unwrap();
    assert!(task.saved_directory().is_err());
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    drop(task);
    assert!(
        store.require_no_pending_v2_group(&f.group).is_err(),
        "owner Drop does not renew the slot"
    );
}

#[test]
fn sixteen_unresolved_genuine_groups_never_evict_and_permanent_handoff_allows_next_group() {
    let path = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3b51);
    let pool = CriticalCatalogReadPool::new(CAPACITY).unwrap();
    drop(
        ServerStore::open(path.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let mut origins = Vec::new();
    for index in 0..17u8 {
        origins.push(additional_saved_origin(
            path.path(),
            &pool,
            &mut rng,
            0x3b5100 + index as u64,
            index + 1,
        ));
    }
    let policy = V2CurrentLayoutPolicy::new(
        archive(&pool),
        CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * layout_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path.path(), SECRET, &mut rng, policy).unwrap();
    owner.retry(&mut rng).unwrap();
    let layout = owner.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    let budget = GenesisSeedBudget::new(
        1024 * 1024,
        512 * 1024,
        512 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let mut seeds = Vec::new();
    for (group, signer) in &origins {
        let read = layout
            .read_saved_bootstrap_source_data(group, *signer)
            .unwrap();
        seeds.push(store.prepare_genesis_seed(read, budget).unwrap());
    }
    let mut tasks = Vec::new();
    for seed in &seeds[..16] {
        tasks.push(
            layout
                .prepare_genesis_group_namespace(&seed.borrow_source_data())
                .unwrap(),
        );
    }
    assert!(layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .is_err());
    assert!(store
        .capture_chat_admission(key(b"independently available group", [0xe1; 32]))
        .is_ok());
    // Drop cannot make an ambiguous source available. Keep the corresponding first slot;
    // release a DIFFERENT one only through its real permanent-directory/root barriers.
    drop(tasks.pop());
    assert!(layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .is_err());
    tasks[0]
        .retry_directory(store, &seeds[0].borrow_source_data())
        .unwrap();
    assert!(layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .is_ok());
    assert!(
        store.require_no_pending_v2_group(&origins[15].0).is_err(),
        "expired original stays reserved"
    );
    assert!(
        store
            .capture_chat_admission(key(&origins[0].0, [0xe2; 32]))
            .is_err(),
        "retired slot has actual disk fence"
    );
}

#[test]
fn file_captured_before_identity_failure_never_adopts_equal_replacement_directory() {
    let f = fixture(0x3b61);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    fault(store, 252); // File retained, original session-id query has not run yet.
    assert!(task
        .retry_directory(store, &f.seed.borrow_source_data())
        .is_err());
    let path = task
        .directory_output()
        .unwrap()
        .path_for_test()
        .to_path_buf();
    let displaced = path.with_file_name("held-before-first-group-identity");
    fs::rename(&path, &displaced).unwrap();
    fs::create_dir(&path).unwrap();
    fault(store, 0);
    assert!(task
        .retry_directory(store, &f.seed.borrow_source_data())
        .is_err());
    assert!(task.saved_directory().is_err());
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    // Restore THAT same original object's name; retry may query/use the retained File.
    fs::remove_dir(&path).unwrap();
    fs::rename(&displaced, &path).unwrap();
    task.retry_directory(store, &f.seed.borrow_source_data())
        .unwrap();
    assert!(task.saved_directory().is_ok());
}

#[test]
fn original_cancellation_after_last_full_proof_refuses_saved_handoff_without_erasing_debt() {
    let f = fixture(0x3b62);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    let revoker = store.chat_mount_revoker();
    *store
        .owner_evidence
        .before_v2_group_final_directory_proof
        .lock()
        .unwrap() = Some(Box::new(move || revoker.revoke()));
    assert!(task
        .retry_directory(store, &f.seed.borrow_source_data())
        .is_err());
    assert!(task.saved_directory().is_err());
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert!(task.directory_output().unwrap().path_for_test().is_dir());
    drop(task);
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
}

#[test]
fn task_cancel_is_one_original_negative_before_source_and_runtime_payload_drain() {
    let f = fixture(0x3b63);
    let store = f.layout.checked_store().unwrap();
    let mut task = f
        .layout
        .prepare_genesis_group_namespace(&f.seed.borrow_source_data())
        .unwrap();
    let observed = task.observe_original();
    let charged = f.pool.charged_for_test();
    task.cancel_original();
    task.cancel_original();
    assert_eq!(
        f.pool.charged_for_test(),
        charged,
        "cancel itself neither reserves nor refunds"
    );
    assert!(observed.require_same_original(&task).is_err());
    assert!(task
        .retry_directory(store, &f.seed.borrow_source_data())
        .is_err());
    assert!(!task.directory_output().unwrap().path_for_test().exists());
    f.seed.validate_original(store).unwrap(); // independent source/SDK payload still owned
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert!(store
        .capture_chat_admission(key(b"unrelated after original task cancel", [0xf1; 32]))
        .is_ok());
}
