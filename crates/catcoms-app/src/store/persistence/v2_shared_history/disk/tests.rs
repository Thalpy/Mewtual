use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::{ActionSpec, BasisId, Body, GrantId, Realm};
use catcoms_storage::acquire_vault_session;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [90; 32];

fn creates(after: Option<ActionId>, limit: usize) -> SharedHistoryActionQuery {
    SharedHistoryActionQuery {
        target: None,
        after,
        limit,
    }
}

fn original(scope: &SharedHistoryScope, byte: u8) -> SignedAction {
    SignedAction::sign(
        &DeviceKeypair::from_seed(&[1; 32]),
        ActionSpec {
            realm: Realm::new(&scope.group, scope.channel).unwrap(),
            basis: BasisId::from_bytes([2; 32]),
            grant: GrantId::from_bytes([3; 32]),
            nonce: [byte; 16],
            presented_ms: u64::from(byte),
            parents: vec![],
            body: Body::Create {
                text: format!("received original {byte}"),
                reply_to: None,
            },
        },
    )
    .unwrap()
}

fn encode(
    scope: &SharedHistoryScope,
    action: &SignedAction,
    admission: OwnerFactId,
    rng: &mut ChaCha20Rng,
) -> (SelectedHistoryRecord, Zeroizing<Vec<u8>>) {
    let prepared =
        PreparedSharedHistoryRecord::prepare(scope, &KEY, admission, action, &action.encode(), rng)
            .unwrap();
    (
        SelectedHistoryRecord {
            action: prepared.action,
            manifest: prepared.manifest.id(),
        },
        prepared.encode_bundle().unwrap(),
    )
}

#[test]
fn shared_history_disk_selects_exact_originals_and_finishes_only_its_authenticated_wal() {
    let scope = SharedHistoryScope {
        vault: [10; 32],
        group: [11; 16],
        profile: OwnerFactId::from_bytes([12; 32]),
        tenure: OwnerTenureId::from_bytes([13; 32]),
        creation: [14; 32],
        channel: 15,
    };
    let admission = OwnerFactId::from_bytes([16; 32]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let session = acquire_vault_session(directory.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(2 * DISK_FEE).unwrap();
    let pay = || pool.try_reserve(DISK_FEE).unwrap();
    let owner = || {
        SharedHistoryDisk::from_held(
            HeldDirectory::open(path.clone()).unwrap(),
            scope,
            SharedHistoryDiskQuota::new(3, 3 * (SELECTOR_BYTES + MAX_SHARED_HISTORY_BUNDLE))
                .unwrap(),
            pool.clone(),
            &KEY,
        )
        .unwrap()
    };
    let mut rng = ChaCha20Rng::seed_from_u64(17);
    let first = original(&scope, 18);
    let (expected, bytes) = encode(&scope, &first, admission, &mut rng);
    let disk = owner();
    assert_eq!(
        disk.prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap(),
        expected
    );
    let (retry, resealed) = encode(&scope, &first, admission, &mut rng);
    assert_ne!(retry, expected);
    assert_eq!(
        disk.prepare_save(&KEY, retry, &resealed, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap(),
        expected
    );
    let (conflict, altered) = encode(&scope, &first, OwnerFactId::from_bytes([19; 32]), &mut rng);
    assert!(disk
        .prepare_save(&KEY, conflict, &altered, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .is_err());
    assert!(disk
        .read(&[255; 32], expected.action, &session, pay())
        .is_err());
    drop(disk);

    let disk = owner();
    let saved = disk
        .read(&KEY, expected.action, &session, pay())
        .unwrap()
        .unwrap();
    assert_eq!(saved.selected, expected);
    assert_eq!(saved.admission, admission);
    assert_eq!(saved.original.original_bytes(), first.encode());
    drop(saved);

    let second = original(&scope, 20);
    let (selected, bundle) = encode(&scope, &second, admission, &mut rng);
    let selector = disk.seal_selector(&KEY, selected, None, &mut rng).unwrap();
    assert_eq!(selector.len(), LEGACY_SELECTOR_BYTES);
    let wal = encode_wal(&selector, &bundle).unwrap();
    let wal_name = disk.directory.child(WAL_NAME);
    let mut original_wal = ensure_exact_held(&wal_name, &wal).unwrap();
    disk.directory.sync().unwrap();
    let parked = disk.directory.child("parked-original");
    std::fs::rename(&wal_name, &parked).unwrap();
    std::fs::write(&wal_name, &wal[..]).unwrap();
    assert!(disk.finish_wal(&KEY, &wal, &mut original_wal).is_err());
    assert!(!disk.bundle_path(selected.manifest).exists());
    assert!(!disk.selector_path(&KEY, selected.action).exists());
    std::fs::remove_file(&wal_name).unwrap();
    std::fs::rename(&parked, &wal_name).unwrap();
    // Simulate a power cut in the exact bundle and then in selector output.
    std::fs::write(disk.bundle_path(selected.manifest), &bundle[..17]).unwrap();
    std::fs::write(disk.selector_path(&KEY, selected.action), &selector[..15]).unwrap();
    assert!(disk.read(&KEY, selected.action, &session, pay()).is_err());
    assert!(disk.read(&KEY, first.id(), &session, pay()).is_err());
    assert!(disk
        .read_ancestor(&KEY, selected.action, &session, pay())
        .is_err());
    let earlier = disk
        .read_ancestor(&KEY, first.id(), &session, pay())
        .unwrap()
        .unwrap();
    assert_eq!(earlier.original.original_bytes(), first.encode());
    drop(earlier);

    // Neither a partial WAL nor the record it might still select supplies
    // evidence. Restore only the original bytes before the cold control.
    std::fs::write(&wal_name, &wal[..80]).unwrap();
    assert!(disk
        .read_ancestor(&KEY, first.id(), &session, pay())
        .is_err());
    assert_eq!(std::fs::read(&wal_name).unwrap(), &wal[..80]);
    std::fs::write(&wal_name, &wal[..]).unwrap();
    assert!(disk
        .action_page(&KEY, creates(None, 3), &session, pay())
        .is_err());
    let (_owner, negative) = tokio::sync::watch::channel(false);
    let caller = catcoms_rt::RequestCancellation::new(negative, None);
    assert!(disk
        .selected_audit_page(
            &KEY,
            SharedHistoryActionQuery {
                target: Some(first.id()),
                after: None,
                limit: 8
            },
            &session,
            &caller,
        )
        .is_err());
    assert!(SharedHistoryRelayIndex::start(
        &disk,
        &session,
        pool.try_reserve(RELAY_INDEX_FEE).unwrap(),
        &pay()
    )
    .is_err());
    assert_eq!(std::fs::read(&wal_name).unwrap(), &wal[..]);
    drop(disk);
    let disk = owner();
    let earlier = disk
        .read_ancestor(&KEY, first.id(), &session, pay())
        .unwrap()
        .unwrap();
    assert_eq!(earlier.original.original_bytes(), first.encode());
    assert!(disk
        .read_ancestor(&KEY, selected.action, &session, pay())
        .is_err());
    assert_eq!(std::fs::read(&wal_name).unwrap(), &wal[..]);
    drop(earlier);
    assert_eq!(disk.recover(&KEY, &session, pay()).unwrap(), Some(selected));
    assert_eq!(disk.recover(&KEY, &session, pay()).unwrap(), None);
    let saved = disk
        .read(&KEY, selected.action, &session, pay())
        .unwrap()
        .unwrap();
    assert_eq!(saved.original.original_bytes(), second.encode());
    drop(saved);
    let second_selected = selected;
    let third = original(&scope, 21);
    let (selected, bundle) = encode(&scope, &third, admission, &mut rng);
    let mut prepared = disk
        .prepare_save(&KEY, selected, &bundle, &mut rng, &session, pay())
        .unwrap();
    let (mut held, identity) = open_exact_write(&wal_name, WriteMode::Create).unwrap();
    use std::io::Write;
    held.write_all(&prepared.wal[..80]).unwrap();
    prepared.attempt = Some((held, identity));
    let mut reencoded = disk
        .prepare_save(&KEY, selected, &bundle, &mut rng, &session, pay())
        .unwrap();
    assert!(reencoded.commit(&disk, &KEY).is_err());
    assert_eq!(std::fs::read(&wal_name).unwrap(), &prepared.wal[..80]);
    drop(reencoded);
    assert!(disk.recover(&KEY, &session, pay()).is_err());
    std::fs::rename(&wal_name, &parked).unwrap();
    std::fs::write(&wal_name, &prepared.wal[..80]).unwrap();
    assert!(prepared.commit(&disk, &KEY).is_err());
    assert!(!disk.selector_path(&KEY, selected.action).exists());
    std::fs::remove_file(&wal_name).unwrap();
    std::fs::rename(&parked, &wal_name).unwrap();
    assert_eq!(prepared.commit(&disk, &KEY).unwrap(), selected);
    assert!(!wal_name.exists());
    drop(prepared);
    drop(disk);

    let disk = owner();
    let mut expected_actions = [expected.action, second_selected.action, selected.action];
    expected_actions.sort_unstable();
    let scratch = pay();
    let mut relay = SharedHistoryRelayIndex::start(
        &disk,
        &session,
        pool.try_reserve(RELAY_INDEX_FEE).unwrap(),
        &scratch,
    )
    .unwrap();
    assert!(relay.entries().is_none());
    assert!(relay.advance(&disk, &KEY, &session, &scratch).unwrap());
    assert_eq!(
        relay
            .entries()
            .unwrap()
            .map(|entry| (entry.action, entry.admission))
            .collect::<Vec<_>>(),
        expected_actions.map(|action| (action, admission)),
    );
    drop((relay, scratch));
    let first_page = disk
        .action_page(&KEY, creates(None, 1), &session, pay())
        .unwrap();
    assert_eq!(first_page.actions, expected_actions[..1]);
    assert!(first_page.more);
    let last_page = disk
        .action_page(
            &KEY,
            creates(first_page.actions.first().copied(), 2),
            &session,
            pay(),
        )
        .unwrap();
    assert_eq!(last_page.actions, expected_actions[1..]);
    assert!(!last_page.more);
    drop((first_page, last_page));
    let at_end = disk
        .action_page(
            &KEY,
            creates(expected_actions.last().copied(), 1),
            &session,
            pay(),
        )
        .unwrap();
    assert!(at_end.actions.is_empty());
    assert!(!at_end.more);
    drop(at_end);
    assert!(disk
        .action_page(&KEY, creates(None, 0), &session, pay())
        .is_err());
    assert!(disk
        .action_page(&KEY, creates(None, 65), &session, pay())
        .is_err());
    assert!(disk
        .action_page(&[255; 32], creates(None, 1), &session, pay())
        .is_err());
    let expected_bundle = disk.bundle_path(expected.manifest);
    let foreign_bundle = disk.bundle_path(PrivateChatPageId::from_bytes([254; 32]));
    std::fs::rename(&expected_bundle, &foreign_bundle).unwrap();
    assert!(disk
        .action_page(&KEY, creates(None, 1), &session, pay())
        .is_err());
    let scratch = pay();
    let offered = expected_actions.map(|action| ArchiveRelayEntry { action, admission });
    let mut readable = Vec::new();
    disk.visit_relay_page(
        &KEY,
        &session,
        &scratch,
        &offered,
        || Ok(()),
        |entry, _| {
            readable.push(entry.action);
            Ok(())
        },
    )
    .unwrap();
    let intact = expected_actions
        .into_iter()
        .filter(|action| *action != expected.action)
        .collect::<Vec<_>>();
    assert_eq!(readable, intact);
    let mut relay = SharedHistoryRelayIndex::start(
        &disk,
        &session,
        pool.try_reserve(RELAY_INDEX_FEE).unwrap(),
        &scratch,
    )
    .unwrap();
    assert!(relay.advance(&disk, &KEY, &session, &scratch).unwrap());
    assert_eq!(
        relay
            .entries()
            .unwrap()
            .map(|entry| entry.action)
            .collect::<Vec<_>>(),
        intact
    );
    std::fs::rename(&foreign_bundle, &expected_bundle).unwrap();
    assert!(relay.advance(&disk, &KEY, &session, &scratch).unwrap());
    assert_eq!(
        relay
            .entries()
            .unwrap()
            .map(|entry| entry.action)
            .collect::<Vec<_>>(),
        expected_actions,
    );
    drop((relay, scratch));

    let (fourth, bundle) = encode(&scope, &original(&scope, 22), admission, &mut rng);
    assert!(disk
        .prepare_save(&KEY, fourth, &bundle, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .is_err());
    assert!(!wal_name.exists());
    assert!(!disk.bundle_path(fourth.manifest).exists());
    assert!(!disk.selector_path(&KEY, fourth.action).exists());
    assert!(disk
        .read(&KEY, selected.action, &session, pay())
        .unwrap()
        .is_some());
    assert_eq!(
        disk.prepare_save(&KEY, retry, &resealed, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap(),
        expected
    );
    drop(disk);

    // A separate byte limit refuses the same append even with record slots left.
    let disk = SharedHistoryDisk::from_held(
        HeldDirectory::open(path.clone()).unwrap(),
        scope,
        SharedHistoryDiskQuota::new(5, bundle.len() + SELECTOR_BYTES).unwrap(),
        pool.clone(),
        &KEY,
    )
    .unwrap();
    assert!(disk
        .prepare_save(&KEY, fourth, &bundle, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .is_err());
    assert!(!wal_name.exists());
    assert_eq!(
        disk.read(&KEY, expected.action, &session, pay())
            .unwrap()
            .unwrap()
            .original
            .original_bytes(),
        first.encode()
    );
    let relocated = directory.path().join("relocated");
    std::fs::rename(&path, &relocated).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(disk.read(&KEY, expected.action, &session, pay()).is_err());
    assert!(disk
        .action_page(&KEY, creates(None, 1), &session, pay())
        .is_err());
    assert!(disk
        .prepare_save(&KEY, fourth, &bundle, &mut rng, &session, pay())
        .unwrap()
        .commit(&disk, &KEY)
        .is_err());
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&relocated, &path).unwrap();
    assert!(disk
        .read(&KEY, expected.action, &session, pay())
        .unwrap()
        .is_some());
}

#[test]
fn selected_shared_history_audit_reopens_exact_create_edits_and_delete_in_bounded_pages() {
    let scope = SharedHistoryScope {
        vault: [43; 32],
        group: [44; 16],
        profile: OwnerFactId::from_bytes([45; 32]),
        tenure: OwnerTenureId::from_bytes([46; 32]),
        creation: [47; 32],
        channel: 48,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("received");
    std::fs::create_dir(&path).unwrap();
    let session = acquire_vault_session(directory.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(2 * DISK_FEE + 8 * SHARED_HISTORY_RECORD_FEE).unwrap();
    let owner = || {
        SharedHistoryDisk::from_held(
            HeldDirectory::open(path.clone()).unwrap(),
            scope,
            SharedHistoryDiskQuota::new(16, 16 * (SELECTOR_BYTES + MAX_SHARED_HISTORY_BUNDLE))
                .unwrap(),
            pool.clone(),
            &KEY,
        )
        .unwrap()
    };
    let disk = owner();
    let mut rng = ChaCha20Rng::seed_from_u64(49);
    let admission = OwnerFactId::from_bytes([50; 32]);
    let first = original(&scope, 51);
    let create = first.id();
    let mut previous = create;
    let mut actions = vec![first];
    for revision in 1u8..=9 {
        let mut spec = actions[0].spec().clone();
        spec.nonce = [revision; 16];
        spec.presented_ms = u64::from(revision);
        spec.parents = vec![previous];
        spec.body = if revision == 9 {
            Body::Delete { create }
        } else {
            Body::Edit {
                create,
                text: format!("Revision {revision}"),
            }
        };
        let action = SignedAction::sign(&DeviceKeypair::from_seed(&[1; 32]), spec).unwrap();
        previous = action.id();
        actions.push(action);
    }
    for action in actions.iter().chain([&original(&scope, 52)]) {
        let (selected, bundle) = encode(&scope, action, admission, &mut rng);
        disk.prepare_save(
            &KEY,
            selected,
            &bundle,
            &mut rng,
            &session,
            pool.try_reserve(DISK_FEE).unwrap(),
        )
        .unwrap()
        .commit(&disk, &KEY)
        .unwrap();
    }
    drop(disk);

    let disk = owner();
    let (_owner, negative) = tokio::sync::watch::channel(false);
    let caller = catcoms_rt::RequestCancellation::new(negative, None);
    let query = |after, limit| SharedHistoryActionQuery {
        target: Some(create),
        after,
        limit,
    };
    assert!(disk
        .selected_audit_page(&KEY, query(None, 9), &session, &caller)
        .is_err());
    actions.sort_unstable_by_key(SignedAction::id);
    let first_page = disk
        .selected_audit_page(&KEY, query(None, 8), &session, &caller)
        .unwrap();
    assert_eq!(first_page.next, Some(actions[7].id()));
    let next_page = disk
        .selected_audit_page(&KEY, query(first_page.next, 8), &session, &caller)
        .unwrap();
    assert!(next_page.next.is_none());
    assert_eq!(first_page.records.len(), 8);
    assert_eq!(next_page.records.len(), 2);
    for ((saved_admission, saved), expected) in first_page
        .records
        .iter()
        .chain(&next_page.records)
        .zip(&actions)
    {
        assert_eq!(*saved_admission, admission);
        assert_eq!(saved.original_bytes(), expected.encode());
        assert_eq!(saved.action().signer(), expected.signer());
    }
    drop((first_page, next_page, disk));

    let disk = owner();
    let selected = disk
        .read(&KEY, create, &session, pool.try_reserve(DISK_FEE).unwrap())
        .unwrap()
        .unwrap()
        .selected;
    std::fs::write(disk.bundle_path(selected.manifest), b"incomplete").unwrap();
    let after = (create > actions[7].id()).then(|| actions[7].id());
    assert!(disk
        .selected_audit_page(&KEY, query(after, 8), &session, &caller)
        .is_err());
}

#[test]
fn shared_history_disk_resolves_exact_creation_targets_across_selector_versions() {
    let scope = SharedHistoryScope {
        vault: [31; 32],
        group: [32; 16],
        profile: OwnerFactId::from_bytes([33; 32]),
        tenure: OwnerTenureId::from_bytes([34; 32]),
        creation: [35; 32],
        channel: 36,
    };
    let path = tempfile::tempdir().unwrap();
    let session = acquire_vault_session(path.path()).unwrap();
    let directory = path.path().join("received");
    std::fs::create_dir(&directory).unwrap();
    let pool = CriticalCatalogReadPool::new(DISK_FEE).unwrap();
    let pay = || pool.try_reserve(DISK_FEE).unwrap();
    let disk = SharedHistoryDisk::from_held(
        HeldDirectory::open(directory).unwrap(),
        scope,
        SharedHistoryDiskQuota::native(),
        pool.clone(),
        &KEY,
    )
    .unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(37);
    let admission = OwnerFactId::from_bytes([38; 32]);
    let create = original(&scope, 39);
    let unrelated = original(&scope, 40);
    let mutation = |nonce, body| {
        let mut spec = create.spec().clone();
        spec.nonce = [nonce; 16];
        spec.parents = vec![create.id()];
        spec.body = body;
        SignedAction::sign(&DeviceKeypair::from_seed(&[1; 32]), spec).unwrap()
    };
    let edit = mutation(
        41,
        Body::Edit {
            create: create.id(),
            text: "edited".into(),
        },
    );
    let delete = mutation(
        42,
        Body::Delete {
            create: create.id(),
        },
    );
    for action in [&create, &unrelated, &delete] {
        let (expected, bytes) = encode(&scope, action, admission, &mut rng);
        disk.prepare_save(&KEY, expected, &bytes, &mut rng, &session, pay())
            .unwrap()
            .commit(&disk, &KEY)
            .unwrap();
    }
    let (expected, bytes) = encode(&scope, &edit, admission, &mut rng);
    let wal_path = disk.directory.child(WAL_NAME);
    let wrong = disk
        .seal_selector(&KEY, expected, Some(unrelated.id()), &mut rng)
        .unwrap();
    let wal = encode_wal(&wrong, &bytes).unwrap();
    std::fs::write(&wal_path, &wal[..]).unwrap();
    assert!(disk.recover(&KEY, &session, pay()).is_err());
    assert!(!disk.selector_path(&KEY, edit.id()).exists());
    assert!(!disk.bundle_path(expected.manifest).exists());
    assert_eq!(std::fs::read(&wal_path).unwrap(), &wal[..]);
    std::fs::remove_file(&wal_path).unwrap();

    let legacy = disk.seal_selector(&KEY, expected, None, &mut rng).unwrap();
    std::fs::write(&wal_path, &encode_wal(&legacy, &bytes).unwrap()[..]).unwrap();
    assert_eq!(disk.recover(&KEY, &session, pay()).unwrap(), Some(expected));
    let mut expected_creates = [create.id(), unrelated.id()];
    expected_creates.sort_unstable();
    let page = disk
        .action_page(&KEY, creates(None, 8), &session, pay())
        .unwrap();
    assert_eq!(page.actions, expected_creates);
    assert!(!page.more);
    drop(page);
    let mut expected_target = [create.id(), edit.id(), delete.id()];
    expected_target.sort_unstable();
    let page = disk
        .action_page(
            &KEY,
            SharedHistoryActionQuery {
                target: Some(create.id()),
                after: None,
                limit: 8,
            },
            &session,
            pay(),
        )
        .unwrap();
    assert_eq!(page.actions, expected_target);
    assert!(!page.more);
    drop(page);
    std::fs::write(disk.selector_path(&KEY, edit.id()), &wrong[..]).unwrap();
    assert!(disk.read(&KEY, edit.id(), &session, pay()).is_err());
}
