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
            .commit(&KEY)
            .unwrap(),
        expected
    );
    let (retry, resealed) = encode(&scope, &first, admission, &mut rng);
    assert_ne!(retry, expected);
    assert_eq!(
        disk.prepare_save(&KEY, retry, &resealed, &mut rng, &session, pay())
            .unwrap()
            .commit(&KEY)
            .unwrap(),
        expected
    );
    let (conflict, altered) = encode(&scope, &first, OwnerFactId::from_bytes([19; 32]), &mut rng);
    assert!(disk
        .prepare_save(&KEY, conflict, &altered, &mut rng, &session, pay())
        .unwrap()
        .commit(&KEY)
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
    assert!(disk
        .action_page(&KEY, creates(None, 3), &session, pay())
        .is_err());
    drop(disk);
    let disk = owner();
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
    assert!(reencoded.commit(&KEY).is_err());
    assert_eq!(std::fs::read(&wal_name).unwrap(), &prepared.wal[..80]);
    drop(reencoded);
    assert!(disk.recover(&KEY, &session, pay()).is_err());
    std::fs::rename(&wal_name, &parked).unwrap();
    std::fs::write(&wal_name, &prepared.wal[..80]).unwrap();
    assert!(prepared.commit(&KEY).is_err());
    assert!(!disk.selector_path(&KEY, selected.action).exists());
    std::fs::remove_file(&wal_name).unwrap();
    std::fs::rename(&parked, &wal_name).unwrap();
    assert_eq!(prepared.commit(&KEY).unwrap(), selected);
    assert!(!wal_name.exists());
    drop(prepared);
    drop(disk);

    let disk = owner();
    let mut expected_actions = [expected.action, second_selected.action, selected.action];
    expected_actions.sort_unstable();
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
    std::fs::rename(&foreign_bundle, &expected_bundle).unwrap();

    let (fourth, bundle) = encode(&scope, &original(&scope, 22), admission, &mut rng);
    assert!(disk
        .prepare_save(&KEY, fourth, &bundle, &mut rng, &session, pay())
        .unwrap()
        .commit(&KEY)
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
            .commit(&KEY)
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
        .commit(&KEY)
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
        .commit(&KEY)
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
            .commit(&KEY)
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
