//! Actual factory/physical-debit regressions. These exercise the real Server::found entry,
//! original held files and namespace, rather than a caller-supplied paid/authorized boolean.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::sync::atomic::Ordering;

const SECRET: &[u8] = b"original founder physical fixture";
type Creation = ArchiveFounderCreation<MemNetwork, ChaCha20Rng>;
type Inputs = ArchiveFounderInputs<MemNetwork, ChaCha20Rng>;

fn configured() -> ArchiveFounderPolicy {
    ArchiveFounderPolicy::new(
        FounderSnapshotLimits::new(
            8 * 1024 * 1024 + 64 * 1024,
            8 * 1024 * 1024,
            8 * 1024 * 1024,
        )
        .unwrap(),
        3,
        codec::PERSISTENT_CONTENT_BYTES,
    )
    .unwrap()
}
fn inputs(device: MlsDevice, peer: u64) -> Inputs {
    Inputs::new(
        Hub::new().join(PeerId::from_u64(peer)),
        device,
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(10)),
    )
}
fn open(path: &Path, rng: &mut ChaCha20Rng, pool: &CriticalCatalogReadPool) -> ServerStore {
    let policy = ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * MAX_SEED_FRAMED).unwrap();
    let mut owner = PreparedArchiveBootstrapStore::open(path, SECRET, rng, policy).unwrap();
    owner.retry(rng).unwrap();
    match owner.into_store() {
        Ok(store) => store,
        Err(_) => panic!("namespace barriers incomplete"),
    }
}
fn fixture(
    seed: u64,
) -> (
    tempfile::TempDir,
    ServerStore,
    ChaCha20Rng,
    CriticalCatalogReadPool,
) {
    let path = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    drop(
        ServerStore::open(path.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let store = open(path.path(), &mut rng, &pool);
    SDK_ENTRIES.with(|n| n.set(0));
    (path, store, rng, pool)
}
fn begin(store: &ServerStore, invocation: u8, device: MlsDevice) -> Creation {
    match store
        .prepare_archive_founder(
            [invocation; 32],
            "retained original",
            configured(),
            inputs(device, invocation as u64),
        )
        .unwrap()
    {
        ArchiveFounderBegin::Original(owner) => owner,
        ArchiveFounderBegin::Existing { .. } => panic!("expected actual original"),
    }
}
fn debit_path(store: &ServerStore, owner: &Creation) -> PathBuf {
    let name = owner.original.data().invocation.name();
    store
        .dir
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(format!("{}.debit", std::str::from_utf8(&name).unwrap()))
}
fn entries() -> usize {
    SDK_ENTRIES.with(|n| n.get())
}
fn fault(store: &ServerStore, point: u8) {
    store
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}

#[test]
fn original_debit_faults_retain_exact_bytes_and_enter_actual_sdk_only_after_barriers() {
    for point in [121, 122, 123, 124, 125] {
        let (path, store, mut rng, _pool) = fixture(0xa100 + point as u64);
        let admissions = fs::read(path.path().join("chat-admissions.bin")).unwrap();
        let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
        let mut owner = begin(&store, 1, MlsDevice::generate().unwrap());
        fault(&store, point);
        assert!(
            owner.advance(&store, &mut rng).is_err(),
            "physical barrier {point}"
        );
        assert_eq!(entries(), 0);
        assert!(owner.inputs.is_some());
        assert!(!owner.sdk_started);
        let sealed = owner.sealed_debit.as_ref().unwrap().to_vec();
        let mut unchanged = rng.clone();
        fault(&store, 0);
        assert_eq!(
            owner.advance(&store, &mut rng).unwrap(),
            ArchiveFounderProgress::GroupCreatedUnexposed
        );
        assert_eq!(entries(), 1);
        assert_eq!(fs::read(debit_path(&store, &owner)).unwrap(), sealed);
        assert_eq!(
            rng.next_u64(),
            unchanged.next_u64(),
            "retry must not reseal"
        );
        assert!(owner.inputs.is_none());
        assert!(!owner.group_data().unwrap().is_empty());
        assert_eq!(
            owner.advance(&store, &mut rng).unwrap(),
            ArchiveFounderProgress::GroupCreatedUnexposed
        );
        assert_eq!(entries(), 1, "successful retry is not another SDK creation");
        assert_eq!(
            fs::read(path.path().join("chat-admissions.bin")).unwrap(),
            admissions
        );
        assert_eq!(
            fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
            pins
        );
    }
}

#[test]
fn cold_spent_or_partial_debit_returns_inputs_and_never_recreates_a_group() {
    for point in [122, 123, 125] {
        let (path, store, mut rng, pool) = fixture(0xa200 + point as u64);
        let device = MlsDevice::generate().unwrap();
        let duplicate = device.duplicate().unwrap();
        let mut owner = begin(&store, 2, device);
        fault(&store, point);
        assert!(owner.advance(&store, &mut rng).is_err());
        let debit = fs::read(debit_path(&store, &owner)).unwrap();
        drop(owner);
        drop(store);
        let store = open(path.path(), &mut rng, &pool);
        let result = store
            .prepare_archive_founder(
                [2; 32],
                "retained original",
                configured(),
                inputs(duplicate, 2),
            )
            .unwrap();
        match result {
            ArchiveFounderBegin::Existing { status, inputs } => {
                assert!(!status.is_local_in_progress());
                assert_eq!(status.is_incomplete(), point != 125);
                inputs.device.preflight_fresh_founder_input().unwrap();
            }
            ArchiveFounderBegin::Original(_) => {
                panic!("cold bytes cannot mint original SDK admission")
            }
        }
        assert_eq!(entries(), 0);
        let root = path.path().join(ARCHIVE_BOOTSTRAP_ROOT);
        let found = fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().ends_with(".debit"))
            .unwrap();
        assert_eq!(fs::read(found.path()).unwrap(), debit);
    }
}

#[test]
fn same_request_while_warm_returns_original_inputs_but_never_a_second_creator() {
    let (_path, store, mut rng, _pool) = fixture(0xa301);
    let device = MlsDevice::generate().unwrap();
    let duplicate = device.duplicate().unwrap();
    let mut owner = begin(&store, 3, device);
    match store
        .prepare_archive_founder(
            [3; 32],
            "retained original",
            configured(),
            inputs(duplicate, 3),
        )
        .unwrap()
    {
        ArchiveFounderBegin::Existing { status, inputs } => {
            assert!(status.is_local_in_progress());
            inputs.device.preflight_fresh_founder_input().unwrap();
        }
        ArchiveFounderBegin::Original(_) => panic!("same invocation has two creators"),
    }
    assert_eq!(entries(), 0);
    owner.advance(&store, &mut rng).unwrap();
    assert_eq!(entries(), 1);
}

#[test]
fn changed_request_cannot_adopt_the_original_invocation_even_with_same_signer() {
    let (_path, store, mut rng, _pool) = fixture(0xa302);
    let device = MlsDevice::generate().unwrap();
    let duplicate = device.duplicate().unwrap();
    let mut owner = begin(&store, 4, device);
    let returned = store
        .prepare_archive_founder([4; 32], "changed name", configured(), inputs(duplicate, 4))
        .unwrap_err();
    returned
        .into_parts()
        .0
        .device
        .preflight_fresh_founder_input()
        .unwrap();
    assert!(store
        .prepare_archive_founder(
            [4; 32],
            "retained original",
            configured(),
            inputs(MlsDevice::generate().unwrap(), 4)
        )
        .is_err());
    owner.advance(&store, &mut rng).unwrap();
    assert_eq!(entries(), 1);
}

#[test]
fn lost_saved_return_replays_only_original_unconsumed_admission_and_is_shared_one_shot() {
    let (_path, store, mut rng, _pool) = fixture(0xa303);
    let mut owner = begin(&store, 5, MlsDevice::generate().unwrap());
    fault(&store, 121);
    assert!(owner.advance(&store, &mut rng).is_err());
    fault(&store, 0);
    let prepared = owner.prepared_debit.as_mut().unwrap();
    let lost = store.write_original_creation_debit(prepared).unwrap();
    drop(lost);
    let first = store.write_original_creation_debit(prepared).unwrap();
    let duplicate = store.write_original_creation_debit(prepared).unwrap();
    first
        .consume_for_sdk(&store, &owner.original, None)
        .unwrap();
    assert!(duplicate
        .consume_for_sdk(&store, &owner.original, None)
        .is_err());
    assert!(store
        .write_original_creation_debit(owner.prepared_debit.as_mut().unwrap())
        .is_err());
    assert!(owner.advance(&store, &mut rng).is_err());
    assert_eq!(entries(), 0, "test admission consumption did not call SDK");
    assert!(owner.inputs.is_some());
}

#[path = "founder_native_tests.rs"]
mod native;

#[test]
fn foreign_identical_complete_or_prefix_debit_is_preserved_without_sdk_admission() {
    for prefix in [false, true] {
        let (_path, store, mut rng, _pool) = fixture(0xa304 + u64::from(prefix));
        let mut owner = begin(&store, 6, MlsDevice::generate().unwrap());
        fault(&store, 121);
        assert!(owner.advance(&store, &mut rng).is_err());
        let sealed = owner.sealed_debit.as_ref().unwrap();
        let bytes = if prefix {
            sealed[..sealed.len() / 2].to_vec()
        } else {
            sealed.to_vec()
        };
        let path = debit_path(&store, &owner);
        fs::write(&path, &bytes).unwrap();
        fault(&store, 0);
        assert!(owner.advance(&store, &mut rng).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(entries(), 0);
        assert!(owner.inputs.is_some());
    }
}

#[test]
fn close_before_final_admission_preserves_debit_and_prevents_actual_sdk_entry() {
    let (_path, store, mut rng, _pool) = fixture(0xa306);
    let mut owner = begin(&store, 7, MlsDevice::generate().unwrap());
    let revoke = owner.original.revoker();
    *store.founder_before_sdk_consume.lock().unwrap() = Some(Box::new(move || revoke.revoke()));
    assert!(owner.advance(&store, &mut rng).is_err());
    assert!(store.founder_before_sdk_consume.lock().unwrap().is_none());
    assert_eq!(entries(), 0);
    assert!(owner.inputs.is_some());
    assert!(!owner.sdk_started);
    assert!(debit_path(&store, &owner).exists());
    assert!(owner.advance(&store, &mut rng).is_err());
}

#[test]
fn same_inode_marker_change_after_actual_debit_read_refuses_final_sdk_admission() {
    let (path, store, mut rng, _pool) = fixture(0xa307);
    let mut owner = begin(&store, 8, MlsDevice::generate().unwrap());
    let marker = path.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(ROOT_MARKER);
    let before = catcoms_storage::kept::session_id(&File::open(&marker).unwrap()).unwrap();
    *store.founder_before_sdk_consume.lock().unwrap() = Some(Box::new(move || {
        let mut bytes = fs::read(&marker).unwrap();
        bytes[0] ^= 1;
        fs::write(&marker, bytes).unwrap();
        assert_eq!(
            catcoms_storage::kept::session_id(&File::open(&marker).unwrap()).unwrap(),
            before
        );
    }));
    assert!(owner.advance(&store, &mut rng).is_err());
    assert!(store.founder_before_sdk_consume.lock().unwrap().is_none());
    assert_eq!(entries(), 0);
    assert!(owner.inputs.is_some());
}

#[test]
fn original_cannot_cross_actual_remount_or_closed_creator_and_debt_keeps_its_fee() {
    let (path, store, mut rng, pool) = fixture(0xa308);
    let baseline = pool.charged_for_test();
    let mut owner = begin(&store, 9, MlsDevice::generate().unwrap());
    fault(&store, 122);
    assert!(owner.advance(&store, &mut rng).is_err());
    let actual = fs::read(debit_path(&store, &owner)).unwrap();
    let negative = owner.original.observer();
    drop(owner);
    assert!(
        negative.is_cancelled(),
        "Store debt is not a creator keepalive"
    );
    assert_eq!(pool.charged_for_test(), baseline + codec::CONTROL_FEE);
    drop(store);
    assert_eq!(
        pool.charged_for_test(),
        0,
        "last inert debt and file drain before fee"
    );
    let store = open(path.path(), &mut rng, &pool);
    let mut new_owner = begin(&store, 10, MlsDevice::generate().unwrap());
    // The actual vault lock prevents simultaneous physical mounts. Keep only the old
    // creator, then drop its Store/session before opening the replacement mount.
    drop(store);
    let other = open(path.path(), &mut rng, &pool);
    assert!(new_owner.advance(&other, &mut rng).is_err());
    assert_eq!(entries(), 0);
    assert!(new_owner.inputs.is_some());
    assert_eq!(actual.len(), 0);
}

#[test]
fn dropped_never_debited_captures_reclaim_slots_but_unknown_control_bytes_do_not() {
    let (_path, store, _rng, _pool) = fixture(0xa309);
    for invocation in 11..24 {
        let owner = begin(&store, invocation, MlsDevice::generate().unwrap());
        assert!(!owner.original.observer().is_cancelled());
        drop(owner);
    }
    for invocation in 24..29 {
        let owner = begin(&store, invocation, MlsDevice::generate().unwrap());
        fs::write(debit_path(&store, &owner), b"unknown partial debit").unwrap();
        drop(owner);
    }
    assert!(store
        .prepare_archive_founder(
            [29; 32],
            "retained original",
            configured(),
            inputs(MlsDevice::generate().unwrap(), 29)
        )
        .is_err());
    assert_eq!(entries(), 0);
}

#[test]
fn supplied_device_with_key_package_is_refused_before_capture_debit_or_sdk() {
    let (path, store, mut rng, pool) = fixture(0xa310);
    let device = MlsDevice::generate().unwrap();
    let _package = device.key_package().unwrap();
    let baseline = pool.charged_for_test();
    let unchanged = rng.clone();
    let returned = store
        .prepare_archive_founder(
            [30; 32],
            "retained original",
            configured(),
            inputs(device, 30),
        )
        .unwrap_err();
    assert_eq!(entries(), 0);
    assert_eq!(pool.charged_for_test(), baseline);
    assert!(returned
        .into_parts()
        .0
        .device
        .preflight_fresh_founder_input()
        .is_err());
    assert_eq!(rng.next_u64(), unchanged.clone().next_u64());
    assert_eq!(
        fs::read_dir(path.path().join(ARCHIVE_BOOTSTRAP_ROOT))
            .unwrap()
            .count(),
        1
    );
}
