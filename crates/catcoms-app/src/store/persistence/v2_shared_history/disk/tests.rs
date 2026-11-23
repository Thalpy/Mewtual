use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::{ActionSpec, BasisId, Body, GrantId, Realm};
use catcoms_storage::acquire_vault_session;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [90; 32];

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
    let selector = disk.seal_selector(&KEY, selected, &mut rng).unwrap();
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
    let third = original(&scope, 21);
    let (selected, bundle) = encode(&scope, &third, admission, &mut rng);
    let mut prepared = disk
        .prepare_save(&KEY, selected, &bundle, &mut rng, &session, pay())
        .unwrap();
    let (mut held, identity) = open_exact_write(&wal_name, true).unwrap();
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
}
