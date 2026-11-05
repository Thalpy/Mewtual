//! Actual source creator/terminal/new physical open/paid read. No paid flag, raw source
//! constructor, SDK restore or Current result is substituted. Authored UNRUN while paused.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy,
    CriticalCatalogReadPool, PreparedArchiveBootstrapStore,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::owner_v2::FounderSnapshotLimits;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

const SECRET: &[u8] = b"actual cold source reader";
pub(super) struct Fixture {
    pub(super) path: tempfile::TempDir,
    pub(super) store: ServerStore,
    pub(super) rng: ChaCha20Rng,
    pub(super) pool: CriticalCatalogReadPool,
    pub(super) group: Vec<u8>,
    pub(super) signer: [u8; 32],
    pub(super) source_id: [u8; 32],
}
fn open(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    rng: &mut ChaCha20Rng,
    enabled: bool,
) -> ServerStore {
    let mut policy = ArchiveBootstrapCatalogPolicy::new(
        pool.clone(),
        4 * critical_namespaces::archive_bootstrap::MAX_SEED_FRAMED,
    )
    .unwrap();
    if enabled {
        policy = policy.with_bootstrap_source_data_read();
    }
    let mut opening = PreparedArchiveBootstrapStore::open(path, SECRET, rng, policy).unwrap();
    opening.retry(rng).unwrap();
    match opening.into_store() {
        Ok(store) => store,
        Err(_) => panic!("namespace barriers"),
    }
}
pub(super) fn fixture(seed: u64) -> Fixture {
    let path = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    drop(
        ServerStore::open(path.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let store = open(path.path(), &pool, &mut rng, false);
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let policy = ArchiveFounderPolicy::new(limits, 3, founder::PERSISTENT_CONTENT_BYTES).unwrap();
    let inputs = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(seed)),
        device,
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(10)),
    );
    let mut creator = match store
        .prepare_archive_founder([123; 32], "actual cold source", policy, inputs)
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("actual original"),
    };
    creator.advance(&store, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&store).unwrap();
    plan.save_founding_intent(&store, &mut rng).unwrap();
    let mut source = plan.into_source_preparation(&store).unwrap();
    source.save_source_terminal(&store, &mut rng).unwrap();
    let source_id = store
        .retained_bootstrap_source_for_test([123; 32])
        .unwrap()
        .1;
    assert!(
        store
            .read_saved_bootstrap_source_data(&group, signer)
            .is_err(),
        "policy is a new-open opt-in"
    );
    drop(source);
    drop(store);
    let store = open(path.path(), &pool, &mut rng, true);
    assert!(store
        .retained_bootstrap_source_for_test([123; 32])
        .is_none());
    Fixture {
        path,
        store,
        rng,
        pool,
        group,
        signer,
        source_id,
    }
}
fn target(f: &Fixture) -> PathBuf {
    let root = f.store.archive_bootstrap_root.lock().unwrap();
    let root = root.as_ref().unwrap();
    let name = critical_namespaces::archive_bootstrap::target_name(
        &root.descriptor.vault,
        &f.group,
        &f.signer,
    )
    .unwrap();
    f.path
        .path()
        .join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&name).unwrap())
}

#[test]
fn actual_cold_read_retains_original_source_and_both_separate_genesis_artifacts_with_real_fee() {
    let f = fixture(0xc001);
    let before = f.pool.charged_for_test();
    let data = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    assert_eq!(f.pool.charged_for_test(), before + READ_FEE);
    assert_eq!(data.capture.source_id.as_bytes(), &f.source_id);
    let view = data.data().view_data();
    assert_eq!(view.context.spec_data().group, f.group.as_slice());
    assert_eq!(view.context.spec_data().signer, f.signer);
    assert_eq!(
        fs::read(target(&f).join("records.bin")).unwrap(),
        view.records_frame
    );
    assert_eq!(
        fs::read(target(&f).join("manifest.bin")).unwrap(),
        view.manifest_frame
    );
    assert!(!view.snapshot.is_empty());
    data.validate_original(&f.store).unwrap();
    {
        let root = f.store.archive_bootstrap_root.lock().unwrap();
        data.validate_under_archive_root(&f.store, root.as_ref().unwrap())
            .unwrap();
    }
    drop(data);
    assert_eq!(f.pool.charged_for_test(), before);
}

#[test]
fn observation_binds_one_actual_paid_read_not_another_capture_of_identical_files() {
    let f = fixture(0xc00a);
    let first = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let observation = first.observe_original();
    observation.require_same_original(&first).unwrap();
    let second = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    assert_eq!(
        first.data().source_id_data(),
        second.data().source_id_data()
    );
    assert_eq!(
        first.data().view_data().snapshot,
        second.data().view_data().snapshot
    );
    first.validate_original(&f.store).unwrap();
    second.validate_original(&f.store).unwrap();
    assert!(observation.require_same_original(&second).is_err());
    observation.require_same_original(&first).unwrap();
}

#[test]
fn weak_read_observation_keeps_neither_plaintext_nor_payment_alive() {
    let f = fixture(0xc00b);
    let baseline = f.pool.charged_for_test();
    let first = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let observation = first.observe_original();
    let copied_observation = observation.clone();
    let capture = Arc::downgrade(&first.capture);
    assert_eq!(Arc::strong_count(&first.capture.identity), 1);
    assert_eq!(
        std::mem::size_of_val(&observation),
        std::mem::size_of::<Weak<()>>()
    );
    assert!(ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES >= std::mem::size_of_val(&observation) + 64);
    assert_eq!(f.pool.charged_for_test(), baseline + READ_FEE);
    drop(first);
    assert_eq!(f.pool.charged_for_test(), baseline);
    assert!(capture.upgrade().is_none());
    // A live Weak<ReadCapture> here is test instrumentation only; production observers keep
    // only Weak<()> and cannot retain that much larger allocation after the fee drains.
    drop(capture);
    assert!(observation.identity.upgrade().is_none());
    assert!(copied_observation.identity.upgrade().is_none());
    let later = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    // Even the same unchanged on-disk source cannot revive an expired original task.
    assert!(observation.require_same_original(&later).is_err());
    assert!(copied_observation.require_same_original(&later).is_err());
}

#[test]
fn whole_marker_locator_and_actual_debit_bind_the_complete_group_and_signer() {
    let f = fixture(0xc002);
    let mut wrong = f.signer;
    wrong[31] ^= 1;
    assert!(f
        .store
        .read_saved_bootstrap_source_data(&f.group, wrong)
        .is_err());
    let mut group = f.group.clone();
    group[0] ^= 1;
    assert!(f
        .store
        .read_saved_bootstrap_source_data(&group, f.signer)
        .is_err());
    let original = fs::read(target(&f).join("origin")).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fs::write(target(&f).join("origin"), &changed).unwrap();
    assert!(f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .is_err());
    assert_eq!(fs::read(target(&f).join("origin")).unwrap(), changed);
}

#[test]
fn partial_or_unknown_control_presence_never_becomes_a_read_or_a_cleaner() {
    for suffix in [".intent-stage", ".intent-seed"] {
        let f = fixture(if suffix == ".intent-stage" {
            0xc003
        } else {
            0xc004
        });
        let work = f
            .store
            .capture_bootstrap_source_read(&f.group, f.signer)
            .unwrap();
        let path = control_path(
            &work.capture.root,
            &work.capture.invocation,
            if suffix == ".intent-stage" {
                FounderControlSlot::Stage
            } else {
                FounderControlSlot::Seed
            },
        )
        .unwrap();
        drop(work);
        fs::write(&path, b"unproven control").unwrap();
        assert!(f
            .store
            .read_saved_bootstrap_source_data(&f.group, f.signer)
            .is_err());
        assert_eq!(fs::read(path).unwrap(), b"unproven control");
    }
}

#[test]
fn missing_corrupt_and_oversized_separate_artifacts_remain_incomplete_and_preserved() {
    for name in ["source.bin", "records.bin", "manifest.bin"] {
        let f = fixture(0xc010 + name.len() as u64);
        let path = target(&f).join(name);
        let bytes = fs::read(&path).unwrap();
        let mut corrupt = bytes.clone();
        corrupt[0] ^= 1;
        fs::write(&path, &corrupt).unwrap();
        assert!(f
            .store
            .read_saved_bootstrap_source_data(&f.group, f.signer)
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        fs::remove_file(&path).unwrap();
        assert!(f
            .store
            .read_saved_bootstrap_source_data(&f.group, f.signer)
            .is_err());
        assert!(!path.exists());
        fs::write(&path, &bytes).unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES as u64 + 1)
            .unwrap();
        assert!(f
            .store
            .read_saved_bootstrap_source_data(&f.group, f.signer)
            .is_err());
        assert_eq!(
            file.metadata().unwrap().len(),
            catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES as u64 + 1
        );
    }
}

#[test]
fn shared_original_read_consumes_once_and_retains_payment_after_store_drop() {
    let f = fixture(0xc020);
    let work = f
        .store
        .capture_bootstrap_source_read(&f.group, f.signer)
        .unwrap();
    let duplicate = ReadWork {
        capture: work.capture.clone(),
        key: Zeroizing::new(f.store.keys.db_key().unwrap()),
        before_read: None,
        after_open: None,
    };
    let data = work.run().unwrap();
    assert!(duplicate.run().is_err());
    let Fixture { store, pool, .. } = f;
    drop(store);
    assert_eq!(pool.charged_for_test(), READ_FEE);
    assert!(
        !data.data().view_data().snapshot.is_empty(),
        "DATA retention is not authority"
    );
    drop(data);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn read_result_refuses_current_or_named_file_replacement_after_large_open_wait() {
    for replace_page in [false, true] {
        let f = fixture(if replace_page { 0xc031 } else { 0xc030 });
        let mut work = f
            .store
            .capture_bootstrap_source_read(&f.group, f.signer)
            .unwrap();
        let path = if replace_page {
            work.capture.target.join("manifest.bin")
        } else {
            control_path(
                &work.capture.root,
                &work.capture.invocation,
                FounderControlSlot::Intent,
            )
            .unwrap()
        };
        let bytes = fs::read(&path).unwrap();
        let replacement = bytes.clone();
        let changed = path.clone();
        let held = f.path.path().join("held-original-cold-read-object");
        work.after_open = Some(Box::new(move || {
            fs::rename(&changed, held).unwrap();
            fs::write(&changed, replacement).unwrap();
        }));
        assert!(work.run().is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

struct BlockedRead {
    release: Option<Sender<()>>,
    result: Receiver<Result<(), String>>,
    join: Option<JoinHandle<()>>,
}
impl Drop for BlockedRead {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
impl BlockedRead {
    fn start(mut work: ReadWork) -> Self {
        let (ready_tx, ready) = channel();
        let (release, advance) = channel();
        let (done, result) = channel();
        work.after_open = Some(Box::new(move || {
            ready_tx.send(()).unwrap();
            advance.recv_timeout(Duration::from_secs(10)).unwrap();
        }));
        let join = std::thread::spawn(move || {
            let _ = done.send(work.run().map(drop).map_err(|e| e.to_string()));
        });
        let running = Self {
            release: Some(release),
            result,
            join: Some(join),
        };
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        running
    }
    fn drain(mut self) -> Result<(), String> {
        self.release.take().unwrap().send(()).unwrap();
        let result = self.result.recv_timeout(Duration::from_secs(10)).unwrap();
        self.join.take().unwrap().join().unwrap();
        result
    }
}

#[test]
fn readonly_work_allows_a_new_mount_but_never_reauthorizes_the_old_result() {
    let f = fixture(0xc040);
    let work = f
        .store
        .capture_bootstrap_source_read(&f.group, f.signer)
        .unwrap();
    let negative = work.capture.negative.clone();
    let running = BlockedRead::start(work);
    let Fixture {
        path,
        store,
        mut rng,
        pool,
        ..
    } = f;
    drop(store);
    assert!(negative.is_cancelled());
    assert_eq!(pool.charged_for_test(), READ_FEE);
    // No mutation to exclude: a new real namespace open can proceed while the old complete
    // opener result is retained, but the original read cannot return it as a live capture.
    let next = open(path.path(), &pool, &mut rng, true);
    let baseline = pool.charged_for_test() - READ_FEE;
    assert!(running.drain().is_err());
    assert_eq!(pool.charged_for_test(), baseline);
    drop(next);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn one_byte_short_actual_pool_refuses_before_any_source_control_buffer() {
    let f = fixture(0xc050);
    let baseline = f.pool.charged_for_test();
    let pressure = f
        .pool
        .try_reserve(1024 * 1024 * 1024 - baseline - READ_FEE + 1)
        .unwrap();
    READ_BUFFER_ENTRIES.with(|entries| entries.set(0));
    assert!(f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .is_err());
    assert_eq!(READ_BUFFER_ENTRIES.with(|entries| entries.get()), 0);
    drop(pressure);
    assert_eq!(f.pool.charged_for_test(), baseline);
    let data = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    assert!(READ_BUFFER_ENTRIES.with(|entries| entries.get()) > 0);
    drop(data);
    assert_eq!(f.pool.charged_for_test(), baseline);
}

#[test]
fn another_groups_actual_source_writer_progresses_during_a_completed_opener_wait() {
    let mut f = fixture(0xc060);
    let work = f
        .store
        .capture_bootstrap_source_read(&f.group, f.signer)
        .unwrap();
    let running = BlockedRead::start(work);
    let device = MlsDevice::generate().unwrap();
    let signer: [u8; 32] = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let policy = ArchiveFounderPolicy::new(limits, 3, founder::PERSISTENT_CONTENT_BYTES).unwrap();
    let inputs = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(0xc061)),
        device,
        ChaCha20Rng::seed_from_u64(0xc061),
        Box::new(ManualClock::new(10)),
    );
    let mut creator = match f
        .store
        .prepare_archive_founder([124; 32], "other actual writer", policy, inputs)
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("actual original"),
    };
    creator.advance(&f.store, &mut f.rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&f.store).unwrap();
    plan.save_founding_intent(&f.store, &mut f.rng).unwrap();
    let mut source = plan.into_source_preparation(&f.store).unwrap();
    source.save_source_terminal(&f.store, &mut f.rng).unwrap();
    drop(source);
    let other = f
        .store
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    assert_eq!(
        other.data().view_data().context.spec_data().group,
        group.as_slice()
    );
    running.drain().unwrap();
}
