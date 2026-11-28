//! Actual crypto/files/physical lease tests. This disk fixture supplies no
//! native row, Source or catalog authority; the real typed mount owns those.
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::{ActionSpec, BasisId, Body, GrantId, Realm};
use catcoms_storage::acquire_vault_session;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

mod page;

const KEY: [u8; 32] = [73; 32];
const LAYOUT: IndexedHistoryLayout = IndexedHistoryLayout([74; 32]);
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("saved retry drew entropy")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("saved retry drew entropy")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("saved retry drew entropy")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("saved retry drew entropy")
    }
}
impl rand_core::CryptoRng for NeverRng {}

fn scope() -> SharedHistoryScope {
    SharedHistoryScope {
        vault: [1; 32],
        group: [2; 16],
        profile: OwnerFactId::from_bytes([3; 32]),
        tenure: OwnerTenureId::from_bytes([4; 32]),
        creation: [5; 32],
        channel: 6,
    }
}
fn action(nonce: u8, body: Body) -> SignedAction {
    SignedAction::sign(
        &DeviceKeypair::from_seed(&[7; 32]),
        ActionSpec {
            realm: Realm::new(&scope().group, scope().channel).unwrap(),
            basis: BasisId::from_bytes([8; 32]),
            grant: GrantId::from_bytes([9; 32]),
            nonce: [nonce; 16],
            presented_ms: u64::from(nonce),
            parents: vec![],
            body,
        },
    )
    .unwrap()
}
fn bundle(
    action: &SignedAction,
    admission: OwnerFactId,
    rng: &mut ChaCha20Rng,
) -> (SelectedHistoryRecord, Zeroizing<Vec<u8>>) {
    let prepared = PreparedSharedHistoryRecord::prepare(
        &scope(),
        &KEY,
        admission,
        action,
        &action.encode(),
        rng,
    )
    .unwrap();
    (
        SelectedHistoryRecord {
            action: action.id(),
            manifest: prepared.manifest.id(),
        },
        prepared.encode_bundle().unwrap(),
    )
}
fn create(nonce: u8) -> SignedAction {
    action(
        nonce,
        Body::Create {
            text: format!("exact {nonce}"),
            reply_to: None,
        },
    )
}
fn owner(
    path: &std::path::Path,
    pool: &CriticalCatalogReadPool,
    quota: SharedHistoryDiskQuota,
) -> Result<SharedHistoryDisk, AppError> {
    let directory = HeldDirectory::open(path.to_owned())?;
    let indexed = IndexedHeadOwner::capture(&directory, &KEY, LAYOUT)?;
    SharedHistoryDisk::from_indexed_held(directory, scope(), quota, pool.clone(), &KEY, indexed)
}
fn initialize(root: &HeldDirectory, rng: &mut ChaCha20Rng) {
    let empty = prepare_empty_indexed_history_head(&KEY, LAYOUT.0, rng).unwrap();
    verify_empty_indexed_history_head(&KEY, LAYOUT.0, &empty).unwrap();
    let mut held = ensure_exact_held(&root.child(INDEXED_HEAD_NAME), &empty).unwrap();
    root.sync().unwrap();
    require_exact_held(&root.child(INDEXED_HEAD_NAME), &mut held, &empty).unwrap();
}

#[test]
fn indexed_archive_requires_its_declared_head_and_never_synthesizes_an_empty_selection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let pool = CriticalCatalogReadPool::new(2 * INDEXED_DISK_FEE).unwrap();
    assert!(owner(&path, &pool, SharedHistoryDiskQuota::native()).is_err());
    assert!(!root.child(INDEXED_HEAD_NAME).exists());
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1301);
    initialize(&root, &mut rng);
    let bytes = std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap();
    assert!(verify_empty_indexed_history_head(&KEY, [75; 32], &bytes).is_err());
    assert!(owner(&path, &pool, SharedHistoryDiskQuota::native()).is_ok());
    std::fs::write(root.child(INDEXED_HEAD_NAME), b"torn").unwrap();
    assert!(owner(&path, &pool, SharedHistoryDiskQuota::native()).is_err());
    assert_eq!(
        std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap(),
        b"torn"
    );
}

#[test]
fn indexed_archive_exact_cow_append_reopen_audit_and_saved_retry_keep_all_originals() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1302);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let admission = OwnerFactId::from_bytes([10; 32]);
    let mut originals = Vec::new();
    for nonce in 1..=35 {
        let action = create(nonce);
        let (expected, bytes) = bundle(&action, admission, &mut rng);
        assert_eq!(
            disk.prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
                .unwrap()
                .commit(&disk, &KEY)
                .unwrap(),
            expected
        );
        originals.push((expected, action));
    }
    let (mut head_file, head) = disk.indexed_head(&KEY).unwrap();
    head_file.require().unwrap();
    assert_eq!(head.count, 35);
    assert_eq!(head.root.unwrap().summary.height, 1);
    let (expected, action) = &originals[0];
    let (resealed, bytes) = bundle(action, admission, &mut rng);
    assert_ne!(*expected, resealed);
    assert_eq!(
        disk.prepare_save(&KEY, resealed, &bytes, &mut NeverRng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap(),
        *expected
    );
    drop(disk);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    for (expected, action) in &originals {
        let saved = disk
            .read(&KEY, expected.action, &session, pay())
            .unwrap()
            .unwrap();
        assert_eq!(saved.selected, *expected);
        assert_eq!(saved.original.original_bytes(), action.encode());
    }
    let (_owner, negative) = tokio::sync::watch::channel(false);
    let caller = catcoms_rt::RequestCancellation::new(negative, None);
    let page = disk
        .selected_audit_page(
            &KEY,
            SharedHistoryActionQuery {
                target: None,
                after: None,
                limit: 8,
            },
            &session,
            &caller,
        )
        .unwrap();
    assert_eq!(page.records.len(), 8);
    assert!(page.next.is_some());
    drop(page);
    let scratch = pay();
    let mut relay = SharedHistoryRelayIndex::start(
        &disk,
        &KEY,
        &session,
        pool.try_reserve(RELAY_INDEX_FEE).unwrap(),
        &scratch,
    )
    .unwrap();
    for _ in 0..5 {
        relay.advance(&disk, &KEY, &session, &scratch).unwrap();
    }
    assert_eq!(relay.entries().unwrap().len(), 35);
    // Originals stay immutable; only obsolete derived index pages retire.
    assert_eq!(
        std::fs::read_dir(&path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|item| item
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("b-")))
            .count(),
        35
    );
}

#[test]
fn indexed_archive_cold_wal_finishes_after_head_and_partial_derived_retirement() {
    for fault in [
        write::IndexedHistoryFault::AfterHead,
        write::IndexedHistoryFault::AfterPageRetirement,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("received");
        std::fs::create_dir(&path).unwrap();
        let root = HeldDirectory::open(path.clone()).unwrap();
        let session = acquire_vault_session(temp.path()).unwrap();
        let pool = CriticalCatalogReadPool::new(3 * INDEXED_DISK_FEE).unwrap();
        let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x1d1303);
        initialize(&root, &mut rng);
        let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
        let first = create(1);
        let (a, bytes) = bundle(&first, OwnerFactId::from_bytes([11; 32]), &mut rng);
        disk.prepare_save(&KEY, a, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
        let second = create(2);
        let (b, bytes) = bundle(&second, OwnerFactId::from_bytes([11; 32]), &mut rng);
        let mut pending = disk
            .prepare_save(&KEY, b, &bytes, &mut rng, &session, pay())
            .unwrap();
        write::FAULT.with(|slot| slot.set(Some(fault)));
        assert!(pending.commit(&disk, &KEY).is_err());
        assert!(root.child(WAL).is_file());
        let covered = std::fs::read(root.child(WAL)).unwrap();
        let mut altered = covered.clone();
        *altered.last_mut().unwrap() ^= 1;
        std::fs::write(root.child(WAL), &altered).unwrap();
        assert!(disk.recover(&KEY, &session, pay()).is_err());
        assert_eq!(std::fs::read(root.child(WAL)).unwrap(), altered);
        // Restore exactly the independently retained authenticated source;
        // neither failed read nor recovery is allowed to rewrite it.
        std::fs::write(root.child(WAL), &covered).unwrap();
        assert!(disk.read(&KEY, b.action, &session, pay()).is_err());
        assert_eq!(
            disk.read_ancestor(&KEY, a.action, &session, pay())
                .unwrap()
                .unwrap()
                .original
                .original_bytes(),
            first.encode()
        );
        drop(pending);
        drop(disk);
        let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
        assert_eq!(disk.recover(&KEY, &session, pay()).unwrap(), Some(b));
        assert!(!root.child(WAL).exists());
        assert_eq!(
            disk.read(&KEY, b.action, &session, pay())
                .unwrap()
                .unwrap()
                .original
                .original_bytes(),
            second.encode()
        );
    }
}

#[test]
fn indexed_archive_partial_wal_stays_held_but_original_warm_owner_finishes_exact_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(3 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1304);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let action = create(3);
    let (expected, bytes) = bundle(&action, OwnerFactId::from_bytes([12; 32]), &mut rng);
    let mut pending = disk
        .prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
        .unwrap();
    fail_next_history_wal_write_for_test();
    assert!(pending.commit(&disk, &KEY).is_err());
    let torn = std::fs::read(root.child(WAL)).unwrap();
    assert!(disk.recover(&KEY, &session, pay()).is_err());
    assert_eq!(std::fs::read(root.child(WAL)).unwrap(), torn);
    assert_eq!(pending.commit(&disk, &KEY).unwrap(), expected);
    assert_eq!(
        disk.read(&KEY, expected.action, &session, pay())
            .unwrap()
            .unwrap()
            .original
            .original_bytes(),
        action.encode()
    );
}

#[test]
fn indexed_archive_missing_selected_bundle_and_stale_prepared_head_never_become_absence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1305);
    initialize(&root, &mut rng);
    let disk = owner(
        &path,
        &pool,
        SharedHistoryDiskQuota::new(2, 16 * 1024 * 1024).unwrap(),
    )
    .unwrap();
    let (a, bytes_a) = bundle(&create(4), OwnerFactId::from_bytes([13; 32]), &mut rng);
    let (b, bytes_b) = bundle(&create(5), OwnerFactId::from_bytes([13; 32]), &mut rng);
    let mut first = disk
        .prepare_save(&KEY, a, &bytes_a, &mut rng, &session, pay())
        .unwrap();
    let mut stale = disk
        .prepare_save(&KEY, b, &bytes_b, &mut rng, &session, pay())
        .unwrap();
    let head_path = root.child(INDEXED_HEAD_NAME);
    let head = std::fs::read(&head_path).unwrap();
    let parked = temp.path().join("actual-original-head");
    std::fs::rename(&head_path, &parked).unwrap();
    std::fs::write(&head_path, &head).unwrap();
    assert!(first.commit(&disk, &KEY).is_err());
    assert_eq!(std::fs::read(&head_path).unwrap(), head);
    assert!(!root.child(WAL).exists());
    std::fs::remove_file(&head_path).unwrap();
    std::fs::rename(&parked, &head_path).unwrap();
    first.commit(&disk, &KEY).unwrap();
    assert!(stale.commit(&disk, &KEY).is_err());
    assert!(!root.child(WAL).exists());
    drop(stale);
    drop(first);
    std::fs::remove_file(disk.bundle_path(a.manifest)).unwrap();
    assert!(disk.read(&KEY, a.action, &session, pay()).is_err());
    let (_owner, negative) = tokio::sync::watch::channel(false);
    let caller = catcoms_rt::RequestCancellation::new(negative, None);
    assert!(disk
        .selected_audit_page(
            &KEY,
            SharedHistoryActionQuery {
                target: Some(a.action),
                after: None,
                limit: 8
            },
            &session,
            &caller
        )
        .is_err());
    assert!(!disk.bundle_path(a.manifest).exists());
}

#[test]
fn indexed_archive_target_audit_preserves_the_exact_edit_delete_chain_and_quota() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1306);
    initialize(&root, &mut rng);
    let disk = owner(
        &path,
        &pool,
        SharedHistoryDiskQuota::new(3, 16 * 1024 * 1024).unwrap(),
    )
    .unwrap();
    let first = create(6);
    let mut edit = first.spec().clone();
    edit.nonce = [7; 16];
    edit.parents = vec![first.id()];
    edit.body = Body::Edit {
        create: first.id(),
        text: "exact revision".into(),
    };
    let edit = SignedAction::sign(&DeviceKeypair::from_seed(&[7; 32]), edit).unwrap();
    let mut delete = first.spec().clone();
    delete.nonce = [8; 16];
    delete.parents = vec![edit.id()];
    delete.body = Body::Delete { create: first.id() };
    let delete = SignedAction::sign(&DeviceKeypair::from_seed(&[7; 32]), delete).unwrap();
    let actions = [first, edit, delete];
    for action in &actions {
        let (expected, bytes) = bundle(action, OwnerFactId::from_bytes([14; 32]), &mut rng);
        disk.prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
    }
    let target = actions[0].id();
    let page = disk
        .action_page(
            &KEY,
            SharedHistoryActionQuery {
                target: None,
                after: None,
                limit: 8,
            },
            &session,
            pay(),
        )
        .unwrap();
    assert_eq!(page.actions, vec![target]);
    drop(page);
    let (_owner, negative) = tokio::sync::watch::channel(false);
    let caller = catcoms_rt::RequestCancellation::new(negative, None);
    let page = disk
        .selected_audit_page(
            &KEY,
            SharedHistoryActionQuery {
                target: Some(target),
                after: None,
                limit: 8,
            },
            &session,
            &caller,
        )
        .unwrap();
    assert_eq!(page.records.len(), 3);
    assert!(page.next.is_none());
    for action in &actions {
        assert!(page
            .records
            .iter()
            .any(|(_, original)| original.original_bytes() == action.encode()));
    }
    drop(page);
    let (over, bytes) = bundle(&create(9), OwnerFactId::from_bytes([14; 32]), &mut rng);
    let head = std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap();
    assert!(disk
        .prepare_save(&KEY, over, &bytes, &mut NeverRng, &session, pay())
        .is_err());
    assert_eq!(std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap(), head);
    assert!(!root.child(WAL).exists());
}

#[test]
fn indexed_archive_same_mount_temporary_reads_reject_an_older_authentic_head() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1307);
    initialize(&root, &mut rng);
    let empty = std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap();
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let temporary = SharedHistoryDisk::from_indexed_held(
        root.try_clone().unwrap(),
        scope(),
        SharedHistoryDiskQuota::native(),
        pool.clone(),
        &KEY,
        disk.indexed.as_ref().unwrap().clone(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&disk.gate, &temporary.gate));
    let first = create(10);
    let (expected, bytes) = bundle(&first, OwnerFactId::from_bytes([15; 32]), &mut rng);
    let before = temporary
        .action_page(
            &KEY,
            SharedHistoryActionQuery {
                target: None,
                after: None,
                limit: 8,
            },
            &session,
            pay(),
        )
        .unwrap();
    disk.prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    assert!(before.observation.with_publication(|| ()).is_err());
    drop(before);
    let latest = std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap();
    std::fs::write(root.child(INDEXED_HEAD_NAME), &empty).unwrap();
    assert!(temporary
        .read(&KEY, expected.action, &session, pay())
        .is_err());
    assert!(disk
        .action_page(
            &KEY,
            SharedHistoryActionQuery {
                target: None,
                after: None,
                limit: 8
            },
            &session,
            pay()
        )
        .is_err());
    std::fs::write(root.child(INDEXED_HEAD_NAME), &latest).unwrap();
    assert!(temporary
        .read(&KEY, expected.action, &session, pay())
        .unwrap()
        .is_some());
    let parked = temp.path().join("original-selected-head");
    std::fs::rename(root.child(INDEXED_HEAD_NAME), &parked).unwrap();
    std::fs::write(root.child(INDEXED_HEAD_NAME), &latest).unwrap();
    assert!(temporary
        .read(&KEY, expected.action, &session, pay())
        .is_err());
    assert_eq!(
        std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap(),
        latest
    );
}

#[test]
fn indexed_archive_payload_identity_survives_the_final_blocking_read_check() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(3 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1308);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let (expected, bytes) = bundle(&create(11), OwnerFactId::from_bytes([16; 32]), &mut rng);
    disk.prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    let payload = disk.bundle_path(expected.manifest);
    let parked = temp.path().join("original-exact-payload");
    let mut checked = false;
    assert!(disk
        .indexed_read_selected_checked(&KEY, expected.action, None, || {
            checked = true;
            std::fs::rename(&payload, &parked).unwrap();
            std::fs::write(&payload, &bytes).unwrap();
            Ok(())
        })
        .is_err());
    assert!(checked);
    assert_eq!(std::fs::read(&payload).unwrap(), bytes.as_slice());
    std::fs::remove_file(&payload).unwrap();
    std::fs::rename(&parked, &payload).unwrap();
    assert!(disk
        .read(&KEY, expected.action, &session, pay())
        .unwrap()
        .is_some());
}

#[test]
fn indexed_archive_known_wal_unlink_retains_cleanup_until_true_absence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(3 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1309);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let (expected, bytes) = bundle(&create(12), OwnerFactId::from_bytes([17; 32]), &mut rng);
    let mut pending = disk
        .prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
        .unwrap();
    write::FAULT.with(|slot| slot.set(Some(write::IndexedHistoryFault::AfterWalUnlink)));
    assert!(pending.commit(&disk, &KEY).is_err());
    assert!(!root.child(WAL).exists());
    assert!(pending.has_wal());
    assert!(disk.read(&KEY, expected.action, &session, pay()).is_err());
    let head = std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap();
    let (_, selected) = disk.indexed_head(&KEY).unwrap();
    let leaf = root.child(&index_name(selected.root.unwrap().page));
    let exact_leaf = std::fs::read(&leaf).unwrap();
    std::fs::remove_file(&leaf).unwrap();
    // Head plus immutable bundle cannot replace a missing selected path.
    assert!(pending.commit(&disk, &KEY).is_err());
    assert!(pending.has_wal());
    assert!(!root.child(WAL).exists());
    std::fs::write(&leaf, &exact_leaf).unwrap();
    assert_eq!(pending.commit(&disk, &KEY).unwrap(), expected);
    assert!(!pending.has_wal());
    assert!(!root.child(WAL).exists());
    assert_eq!(std::fs::read(root.child(INDEXED_HEAD_NAME)).unwrap(), head);
    assert!(disk
        .read(&KEY, expected.action, &session, pay())
        .unwrap()
        .is_some());
}

#[test]
fn indexed_archive_ancestor_refuses_a_valid_wal_from_another_selected_generation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1310);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let admission = OwnerFactId::from_bytes([18; 32]);
    let (first, bytes) = bundle(&create(13), admission, &mut rng);
    disk.prepare_save(&KEY, first, &bytes, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    let (second, bytes) = bundle(&create(14), admission, &mut rng);
    let mut pending = disk
        .prepare_save(&KEY, second, &bytes, &mut rng, &session, pay())
        .unwrap();
    write::FAULT.with(|slot| slot.set(Some(write::IndexedHistoryFault::AfterHead)));
    assert!(pending.commit(&disk, &KEY).is_err());
    let prior_wal = std::fs::read(root.child(WAL)).unwrap();
    let mut absent_stage = disk.indexed_pending_action(&KEY).unwrap().unwrap();
    std::fs::write(root.child(STAGE), b"unattributed stage prefix").unwrap();
    assert!(absent_stage.require(&root).is_err());
    drop(absent_stage);
    assert!(disk
        .read_ancestor(&KEY, first.action, &session, pay())
        .is_err());
    assert_eq!(
        std::fs::read(root.child(STAGE)).unwrap(),
        b"unattributed stage prefix"
    );
    std::fs::remove_file(root.child(STAGE)).unwrap();
    let next = codec::Wal::decode(&prior_wal).unwrap().next;
    std::fs::write(root.child(STAGE), next).unwrap();
    let mut original_stage = disk.indexed_pending_action(&KEY).unwrap().unwrap();
    let parked = temp.path().join("original-pending-stage");
    std::fs::rename(root.child(STAGE), &parked).unwrap();
    std::fs::write(root.child(STAGE), next).unwrap();
    assert!(original_stage.require(&root).is_err());
    drop(original_stage);
    std::fs::remove_file(root.child(STAGE)).unwrap();
    std::fs::rename(parked, root.child(STAGE)).unwrap();
    pending.commit(&disk, &KEY).unwrap();
    drop(pending);
    let (third, bytes) = bundle(&create(15), admission, &mut rng);
    disk.prepare_save(&KEY, third, &bytes, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    std::fs::write(root.child(WAL), &prior_wal).unwrap();
    assert!(disk
        .read_ancestor(&KEY, first.action, &session, pay())
        .is_err());
    assert_eq!(std::fs::read(root.child(WAL)).unwrap(), prior_wal);
}
