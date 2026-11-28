//! Bounded locator queries, not native serving/authoring authority.
use super::*;

#[test]
fn indexed_archive_page_start_empty_eight_and_ninth_keep_exact_order() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let work = || pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1321);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let empty = disk
        .indexed_action_page(&KEY, None, None, &session, work())
        .unwrap();
    assert_eq!(empty.scanned_rows_data(), 0);
    assert!(empty.rows_data().is_empty());
    assert!(empty.next_cursor().is_none());
    empty.require_current_data(&session).unwrap();
    let mut expected = Vec::new();
    for nonce in 1..=8 {
        let (selected, bytes) = bundle(&create(nonce), OwnerFactId::from_bytes([21; 32]), &mut rng);
        disk.prepare_save(&KEY, selected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
        expected.push(selected.action);
    }
    assert!(matches!(
        empty.require_current_data(&session),
        Err(IndexedActionPageError::Stale)
    ));
    drop(empty);
    expected.sort_unstable();
    let eight = disk
        .indexed_action_page(&KEY, None, None, &session, work())
        .unwrap();
    assert_eq!(eight.scanned_rows_data(), 8);
    assert!(eight.next_cursor().is_none());
    assert_eq!(
        eight
            .rows_data()
            .iter()
            .map(|row| row.action)
            .collect::<Vec<_>>(),
        expected
    );
    let (ninth, bytes) = bundle(&create(9), OwnerFactId::from_bytes([21; 32]), &mut rng);
    disk.prepare_save(&KEY, ninth, &bytes, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    assert!(matches!(
        eight.require_current_data(&session),
        Err(IndexedActionPageError::Stale)
    ));
    drop(eight);
    expected.push(ninth.action);
    expected.sort_unstable();
    let first = disk
        .indexed_action_page(&KEY, None, None, &session, work())
        .unwrap();
    let last = disk
        .indexed_action_page(&KEY, first.next_cursor(), None, &session, work())
        .unwrap();
    assert_eq!(first.scanned_rows_data(), 8);
    assert!(first.next_cursor().is_some());
    assert_eq!(last.scanned_rows_data(), 1);
    assert!(last.next_cursor().is_none());
    let found = first
        .rows_data()
        .iter()
        .chain(last.rows_data())
        .map(|row| row.action)
        .collect::<Vec<_>>();
    assert_eq!(found, expected);
    first.require_current_data(&session).unwrap();
    last.require_current_data(&session).unwrap();
}

#[test]
fn indexed_archive_page_filtered_empty_turns_advance_across_a_leaf_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let work = || pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1322);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let included = OwnerFactId::from_bytes([22; 32]);
    let excluded = OwnerFactId::from_bytes([23; 32]);
    let mut actions = (1..=41).map(create).collect::<Vec<_>>();
    actions.sort_unstable_by_key(SignedAction::id);
    let expected = actions[34].id();
    for (index, action) in actions.iter().enumerate() {
        let (selected, bytes) = bundle(
            action,
            if index == 34 { included } else { excluded },
            &mut rng,
        );
        disk.prepare_save(&KEY, selected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
    }
    let filter = Some(included);
    let mut page = disk
        .indexed_action_page(&KEY, None, filter, &session, work())
        .unwrap();
    let mut scanned = 0;
    let mut turns = 0;
    let mut found = Vec::new();
    loop {
        turns += 1;
        assert!(turns <= 6);
        assert!(page.scanned_rows_data() <= INDEXED_ACTION_PAGE_SCAN);
        scanned += page.scanned_rows_data();
        found.extend(page.rows_data().iter().map(|row| row.action));
        if turns <= 4 {
            assert!(page.rows_data().is_empty());
            assert!(page.next_cursor().is_some());
        }
        let Some(cursor) = page.next_cursor() else {
            break;
        };
        let next = disk
            .indexed_action_page(&KEY, Some(cursor), filter, &session, work())
            .unwrap();
        page = next;
    }
    assert_eq!((scanned, turns), (41, 6));
    assert_eq!(found, vec![expected]);
}

#[test]
fn indexed_archive_page_stale_filter_owner_and_new_head_return_no_page() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let work = || pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1323);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    for nonce in 1..=9 {
        let (selected, bytes) = bundle(&create(nonce), OwnerFactId::from_bytes([24; 32]), &mut rng);
        disk.prepare_save(&KEY, selected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
    }
    let first = disk
        .indexed_action_page(&KEY, None, None, &session, work())
        .unwrap();
    assert!(matches!(
        disk.indexed_action_page(
            &KEY,
            first.next_cursor(),
            Some(OwnerFactId::from_bytes([24; 32])),
            &session,
            work()
        ),
        Err(IndexedActionPageError::Stale)
    ));
    // A separately cold-captured controller has no right to adopt this
    // original warm query, despite equal scope, files and catalogue labels.
    let foreign = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    assert!(matches!(
        foreign.indexed_action_page(&KEY, first.next_cursor(), None, &session, work()),
        Err(IndexedActionPageError::Stale)
    ));
    drop(foreign);
    let head_path = root.child(INDEXED_HEAD_NAME);
    let head_bytes = std::fs::read(&head_path).unwrap();
    let parked = temp.path().join("actual-page-head");
    std::fs::rename(&head_path, &parked).unwrap();
    std::fs::write(&head_path, &head_bytes).unwrap();
    assert!(matches!(
        disk.indexed_action_page(&KEY, first.next_cursor(), None, &session, work()),
        Err(IndexedActionPageError::Unavailable(_))
    ));
    assert!(matches!(
        first.require_current_data(&session),
        Err(IndexedActionPageError::Unavailable(_))
    ));
    assert_eq!(std::fs::read(&head_path).unwrap(), head_bytes);
    std::fs::remove_file(&head_path).unwrap();
    std::fs::rename(parked, &head_path).unwrap();
    first.require_current_data(&session).unwrap();
    let (selected, bytes) = bundle(&create(10), OwnerFactId::from_bytes([24; 32]), &mut rng);
    disk.prepare_save(&KEY, selected, &bytes, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    assert!(matches!(
        disk.indexed_action_page(&KEY, first.next_cursor(), None, &session, work()),
        Err(IndexedActionPageError::Stale)
    ));
}

#[test]
fn indexed_archive_page_missing_branch_and_unrelated_controls_never_mean_empty() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
    let pay = || pool.try_reserve(INDEXED_DISK_FEE).unwrap();
    let work = || pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1324);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    for nonce in 1..=33 {
        let (selected, bytes) = bundle(&create(nonce), OwnerFactId::from_bytes([25; 32]), &mut rng);
        disk.prepare_save(&KEY, selected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
    }
    let (_, head) = disk.indexed_head(&KEY).unwrap();
    assert_eq!(head.root.unwrap().summary.height, 1);
    let branch = root.child(&index_name(head.root.unwrap().page));
    let bytes = std::fs::read(&branch).unwrap();
    std::fs::remove_file(&branch).unwrap();
    assert!(matches!(
        disk.indexed_action_page(&KEY, None, None, &session, work()),
        Err(IndexedActionPageError::Unavailable(_))
    ));
    assert!(!branch.exists());
    std::fs::write(&branch, &bytes).unwrap();
    let page = disk
        .indexed_action_page(&KEY, None, None, &session, work())
        .unwrap();
    for name in [WAL, STAGE] {
        std::fs::write(root.child(name), b"unknown partial control").unwrap();
        assert!(matches!(
            disk.indexed_action_page(&KEY, None, None, &session, work()),
            Err(IndexedActionPageError::Unavailable(_))
        ));
        assert!(matches!(
            page.require_current_data(&session),
            Err(IndexedActionPageError::Unavailable(_))
        ));
        assert_eq!(
            std::fs::read(root.child(name)).unwrap(),
            b"unknown partial control"
        );
        std::fs::remove_file(root.child(name)).unwrap();
    }
    page.require_current_data(&session).unwrap();
}

#[test]
fn indexed_archive_page_work_and_retained_credit_are_independent_and_precede_reads() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let root = HeldDirectory::open(path.clone()).unwrap();
    let session = acquire_vault_session(temp.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(
        INDEXED_ACTION_PAGE_WORK_FEE + INDEXED_ACTION_PAGE_RETAINED_FEE,
    )
    .unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1d1325);
    initialize(&root, &mut rng);
    let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
    let before = pool.read_entries_for_test();
    let short = pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE - 1).unwrap();
    assert!(matches!(
        disk.indexed_action_page(&KEY, None, None, &session, short),
        Err(IndexedActionPageError::Unavailable(_))
    ));
    assert_eq!(pool.read_entries_for_test(), before);
    let foreign = CriticalCatalogReadPool::new(
        INDEXED_ACTION_PAGE_WORK_FEE + INDEXED_ACTION_PAGE_RETAINED_FEE,
    )
    .unwrap();
    assert!(matches!(
        disk.indexed_action_page(
            &KEY,
            None,
            None,
            &session,
            foreign.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap()
        ),
        Err(IndexedActionPageError::Unavailable(_))
    ));
    assert_eq!(pool.read_entries_for_test(), before);
    assert_eq!(foreign.charged_for_test(), 0);
    let held = disk.gate.try_lock().unwrap();
    assert!(matches!(
        disk.indexed_action_page(
            &KEY,
            None,
            None,
            &session,
            pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap()
        ),
        Err(IndexedActionPageError::Unavailable(
            AppError::ChatV2ArchiveBusy
        ))
    ));
    drop(held);
    assert_eq!(pool.charged_for_test(), 0);
    let released = Arc::new(AtomicBool::new(false));
    let mut work = pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap();
    work.observe_drop_for_test(released.clone());
    let page = disk
        .indexed_action_page(&KEY, None, None, &session, work)
        .unwrap();
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(pool.charged_for_test(), INDEXED_ACTION_PAGE_RETAINED_FEE);
    let before = pool.read_entries_for_test();
    let work = pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap();
    assert!(matches!(
        disk.indexed_action_page(&KEY, None, None, &session, work),
        Err(IndexedActionPageError::Unavailable(_))
    ));
    assert_eq!(pool.read_entries_for_test(), before);
    page.require_current_data(&session).unwrap();
    drop(page);
    assert_eq!(pool.charged_for_test(), 0);
    let page = disk
        .indexed_action_page(
            &KEY,
            None,
            None,
            &session,
            pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap(),
        )
        .unwrap();
    assert!(page.rows_data().is_empty());
    drop(disk);
    assert!(matches!(
        page.require_current_data(&session),
        Err(IndexedActionPageError::Stale)
    ));
    assert_eq!(pool.charged_for_test(), INDEXED_ACTION_PAGE_RETAINED_FEE);
    drop(page);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn indexed_archive_page_poisoned_gate_or_head_is_unavailable_not_busy() {
    for poison_head in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("received");
        std::fs::create_dir(&path).unwrap();
        let root = HeldDirectory::open(path.clone()).unwrap();
        let session = acquire_vault_session(temp.path()).unwrap();
        let pool = CriticalCatalogReadPool::new(
            2 * INDEXED_ACTION_PAGE_WORK_FEE + 2 * INDEXED_ACTION_PAGE_RETAINED_FEE,
        )
        .unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x1d1326);
        initialize(&root, &mut rng);
        let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
        let page = disk
            .indexed_action_page(
                &KEY,
                None,
                None,
                &session,
                pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap(),
            )
            .unwrap();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if poison_head {
                let _guard = disk.indexed.as_ref().unwrap().head.lock().unwrap();
                panic!("poison the actual indexed Head fixture guard");
            } else {
                let _guard = disk.gate.lock().unwrap();
                panic!("poison the actual indexed archive fixture guard");
            }
        }));
        assert!(poisoned.is_err());
        assert!(matches!(
            page.require_current_data(&session),
            Err(IndexedActionPageError::Unavailable(AppError::Invalid(_)))
        ));
        assert!(matches!(
            disk.indexed_action_page(
                &KEY,
                None,
                None,
                &session,
                pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE).unwrap()
            ),
            Err(IndexedActionPageError::Unavailable(AppError::Invalid(_)))
        ));
    }
}

#[test]
fn indexed_archive_page_compatibility_final_guard_rejects_equal_head_replacement() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for relay in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("received");
        std::fs::create_dir(&path).unwrap();
        let root = HeldDirectory::open(path.clone()).unwrap();
        let session = acquire_vault_session(temp.path()).unwrap();
        let pool = CriticalCatalogReadPool::new(4 * INDEXED_DISK_FEE).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x1d1327);
        initialize(&root, &mut rng);
        let disk = owner(&path, &pool, SharedHistoryDiskQuota::native()).unwrap();
        let head = root.child(INDEXED_HEAD_NAME);
        let original = std::fs::read(&head).unwrap();
        let parked = temp.path().join("actual-compatibility-head");
        let reached = Arc::new(AtomicBool::new(false));
        let (change, source, held, bytes) = (
            reached.clone(),
            head.clone(),
            parked.clone(),
            original.clone(),
        );
        PAGE_FINAL_CUT.with(|slot| {
            assert!(slot
                .borrow_mut()
                .replace(Box::new(move || {
                    std::fs::rename(&source, held).unwrap();
                    std::fs::write(source, bytes).unwrap();
                    change.store(true, Ordering::SeqCst);
                }))
                .is_none());
        });
        let failed = if relay {
            SharedHistoryRelayIndex::start(
                &disk,
                &KEY,
                &session,
                pool.try_reserve(RELAY_INDEX_FEE).unwrap(),
                &pool.try_reserve(INDEXED_DISK_FEE).unwrap(),
            )
            .is_err()
        } else {
            disk.action_page(
                &KEY,
                SharedHistoryActionQuery {
                    target: None,
                    after: None,
                    limit: 8,
                },
                &session,
                pool.try_reserve(INDEXED_DISK_FEE).unwrap(),
            )
            .is_err()
        };
        let dangling = PAGE_FINAL_CUT.with(|slot| slot.borrow_mut().take());
        assert!(dangling.is_none());
        assert!(reached.load(Ordering::SeqCst));
        assert!(failed);
        assert!(parked.is_file());
        assert_eq!(std::fs::read(&head).unwrap(), original);
    }
}
