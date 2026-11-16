//! Actual ID2 negative-path regressions. Raw group objects are RESERVED corruption fixtures,
//! never fake Current/Saved receipts. All tests are authored UNRUN during the compilation pause.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, ChatAdmissionKey, OwnerEvidenceBudget,
    PreparedArchiveBootstrapStore,
};
use crate::Server;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

const SECRET: &[u8] = b"group-wide negative fence fixture";
fn policy(pool: &CriticalCatalogReadPool) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * archive_bootstrap::MAX_SEED_FRAMED)
        .unwrap()
}
fn mount(path: &Path, pool: &CriticalCatalogReadPool, rng: &mut ChaCha20Rng) -> ServerStore {
    let mut opening = PreparedArchiveBootstrapStore::open(path, SECRET, rng, policy(pool)).unwrap();
    opening.retry(rng).unwrap();
    match opening.into_store() {
        Ok(store) => store,
        Err(_) => panic!("namespace incomplete"),
    }
}
fn fixture(
    seed: u64,
) -> (
    tempfile::TempDir,
    ServerStore,
    CriticalCatalogReadPool,
    ChaCha20Rng,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    drop(
        ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let store = mount(directory.path(), &pool, &mut rng);
    (directory, store, pool, rng)
}
fn group_path(store: &ServerStore, group: &[u8]) -> PathBuf {
    let guard = store.archive_bootstrap_root.lock().unwrap();
    let root = guard.as_ref().unwrap();
    let suffix = group_suffix(&root.descriptor.vault, &root.descriptor.root, group).unwrap();
    store
        .dir
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(format!("g-{}", std::str::from_utf8(&suffix).unwrap()))
}
fn key(group: &[u8], signer: u8) -> ChatAdmissionKey {
    ChatAdmissionKey::new(group, [signer; 32]).unwrap()
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("reserved group reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("reserved group reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("reserved group reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("reserved group reached RNG")
    }
}
impl CryptoRng for NeverRng {}
fn budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        8,
        128 * 1024 * 1024,
        128 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
}
fn node(peer: u64) -> Server<MemNetwork, ChaCha20Rng> {
    Server::found(
        Hub::new().join(PeerId::from_u64(peer)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(1)),
        "old numeric node",
    )
    .unwrap()
}

#[test]
fn group_name_matches_reviewed_codec_and_binds_vault_root_full_group_without_signer() {
    use super::super::archive_current_namespace_codec::{group_name_data, CurrentLayoutData};
    let layout = CurrentLayoutData {
        vault: [1; 32],
        root: [2; 32],
    };
    let suffix = group_suffix(&layout.vault, &layout.root, b"same group").unwrap();
    assert_eq!(suffix, group_name_data(layout, b"same group").unwrap());
    assert_ne!(
        suffix,
        group_suffix(&[3; 32], &layout.root, b"same group").unwrap()
    );
    assert_ne!(
        suffix,
        group_suffix(&layout.vault, &[3; 32], b"same group").unwrap()
    );
    assert_ne!(
        suffix,
        group_suffix(&layout.vault, &layout.root, b"same group\0").unwrap()
    );
    assert!(suffix
        .iter()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)));
    assert!(group_suffix(&layout.vault, &layout.root, b"").is_err());
    assert!(group_suffix(&layout.vault, &layout.root, &[1; 257]).is_err());
}

#[test]
fn directory_or_nonregular_group_presence_fences_every_signer_after_cold_id2_reopen() {
    for regular_file in [false, true] {
        let (directory, store, pool, mut rng) = fixture(0xe301);
        let captured = store.capture_chat_admission(key(b"reserved", 1)).unwrap();
        let reserved = group_path(&store, b"reserved");
        if regular_file {
            fs::write(&reserved, b"partial metadata").unwrap();
        } else {
            fs::create_dir(&reserved).unwrap();
        }
        let before = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
        for signer in [1, 2, 255] {
            assert!(store
                .require_no_archive_bootstrap_target(b"reserved", &[signer; 32])
                .is_err());
            assert!(store
                .capture_chat_admission(key(b"reserved", signer))
                .is_err());
        }
        assert!(store
            .prepare_legacy_chat_admission(&captured, b"old source", &mut NeverRng)
            .is_err());
        assert_eq!(
            fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
            before
        );
        drop(store);
        let store = mount(directory.path(), &pool, &mut rng);
        assert!(store.capture_chat_admission(key(b"reserved", 7)).is_err());
        assert!(store.capture_chat_admission(key(b"other group", 7)).is_ok());
        assert!(
            reserved.exists(),
            "cold lookup must preserve unknown reservation"
        );
        assert_eq!(
            fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
            before
        );
    }
}

#[test]
fn precaptured_old_owners_cannot_finish_resolve_or_write_but_deliberate_retirement_repairs() {
    let (directory, store, _pool, mut rng) = fixture(0xe302);
    let initializing_capture = store.capture_chat_admission(key(b"reserved", 1)).unwrap();
    let initializing = store
        .prepare_legacy_chat_admission(&initializing_capture, b"init", &mut rng)
        .unwrap();
    let current_capture = store.capture_chat_admission(key(b"reserved", 2)).unwrap();
    let current = store
        .prepare_legacy_chat_admission(&current_capture, b"current", &mut rng)
        .unwrap();
    let binding = store
        .finish_chat_admission(&current, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    fs::create_dir(group_path(&store, b"reserved")).unwrap();
    let before = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    for preparation in [&initializing, &current] {
        assert!(store
            .finish_chat_admission(preparation, &mut NeverRng, |_, _| panic!("reserved save"))
            .is_err());
        assert!(store
            .resolve_chat_admission(
                &key(
                    b"reserved",
                    if std::ptr::eq(preparation, &initializing) {
                        1
                    } else {
                        2
                    }
                ),
                preparation.nonce()
            )
            .is_err());
    }
    assert!(store
        .prepare_legacy_chat_admission(&current_capture, b"current", &mut NeverRng)
        .is_err());
    assert!(store
        .with_chat_admission_storage(&binding, |_| -> Result<(), AppError> {
            panic!("reserved positive storage")
        })
        .is_err());
    assert_eq!(
        fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
        before
    );
    assert!(store.with_original_chat_store(&binding, |_| Ok(())).is_ok());
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    store
        .retire_chat_preparation(&initializing, &mut rng)
        .unwrap();
    assert!(matches!(
        store
            .resolve_chat_admission(&key(b"reserved", 2), binding.nonce())
            .unwrap(),
        crate::store::ChatAdmissionResolution::Retired
    ));
    let other = store
        .capture_chat_admission(key(b"other group", 2))
        .unwrap();
    let other = store
        .prepare_legacy_chat_admission(&other, b"independent", &mut rng)
        .unwrap();
    assert!(store
        .finish_chat_admission(&other, &mut rng, |_, _| Ok(()))
        .is_ok());
}

#[test]
fn old_numeric_and_mounted_pin_readers_refuse_group_presence_without_per_signer_target() {
    let (_directory, store, _pool, mut rng) = fixture(0xe303);
    let mut a = node(0xe311);
    let mut b = node(0xe312);
    let a_group = a.group_id();
    let a_bytes = a.snapshot().unwrap();
    let b_bytes = b.snapshot().unwrap();
    store.save_server(1, &a_bytes, &mut rng).unwrap();
    store.save_server(2, &b_bytes, &mut rng).unwrap();
    fs::create_dir(group_path(&store, &a_group)).unwrap();
    assert!(store.load_server(1).is_err());
    assert!(store.save_server(1, &a_bytes, &mut NeverRng).is_err());
    assert_eq!(store.load_server(2).unwrap().as_slice(), b_bytes.as_slice());
    store.save_server(2, &b_bytes, &mut rng).unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    let mut initial = store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(0xe313)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0xe313),
            Box::new(ManualClock::new(1)),
            "real finite owner",
            9,
        )
        .unwrap();
    let group = initial.group().unwrap().to_vec();
    let signer = *initial.founder().unwrap();
    let owner = initial.retry(&store, &mut rng).unwrap();
    let pin = store.capture_mounted_owner_profile(&group).unwrap();
    fs::create_dir(group_path(&store, &group)).unwrap();
    assert!(pin.validate_original(&store, &group).is_err());
    assert!(store.capture_mounted_owner_profile(&group).is_err());
    assert!(store.prepare_owner_canonical_restore(pin, signer).is_err());
    drop(owner);
}

#[test]
fn old_constructor_refuses_current_seed_or_marker_before_old_pin_stage_cleaner() {
    for marker in [false, true] {
        let (directory, store, pool, mut rng) = fixture(0xe304);
        let path = if marker {
            store.dir.join(ARCHIVE_BOOTSTRAP_ROOT).join(MARKER)
        } else {
            store.dir.join(DECLARATION_SEED)
        };
        fs::write(&path, b"unknown partial Current declaration").unwrap();
        let pin_stage = store.dir.join("chat-owner-pins.replace.tmp");
        fs::write(&pin_stage, b"retained older stage").unwrap();
        let catalog = fs::read(store.dir.join(Slot::Current.name())).unwrap();
        drop(store);
        assert!(PreparedArchiveBootstrapStore::open(
            directory.path(),
            SECRET,
            &mut rng,
            policy(&pool)
        )
        .is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            b"unknown partial Current declaration"
        );
        assert_eq!(fs::read(&pin_stage).unwrap(), b"retained older stage");
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            catalog
        );
    }
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(DECLARATION_SEED), b"orphan intent").unwrap();
    assert!(ServerStore::open(directory.path(), SECRET, &mut NeverRng).is_err());
    assert!(!directory.path().join("vault.bin").exists());
}

#[test]
fn actual_old_pin_admission_orders_retirement_without_recursive_writer_entry() {
    use std::sync::mpsc;
    use std::time::Duration;
    let (_directory, store, _pool, mut rng) = fixture(0xe305);
    let store = Arc::new(store);
    let capture = store.capture_chat_admission(key(b"pin writer", 3)).unwrap();
    let preparation = store
        .prepare_legacy_chat_admission(&capture, b"same original", &mut rng)
        .unwrap();
    let binding = store
        .finish_chat_admission(&preparation, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    let pin_admission = store
        .lock_old_owner_pin_admission(&preparation, &mut rng)
        .unwrap();
    let (reached_tx, reached_rx) = mpsc::channel();
    store
        .chat_mount
        .observe_next_writer_contention(move |held| {
            let _ = reached_tx.send(held);
        });
    let (done_tx, done_rx) = mpsc::channel();
    let worker_store = store.clone();
    // Detached-capable on harness timeout: never join a possibly blocked worker before its
    // bounded completion. Unwind drops the actual pin guard, so a failed assertion releases it.
    let worker = std::thread::spawn(move || {
        let result =
            worker_store.retire_chat_admission(&binding, &mut ChaCha20Rng::seed_from_u64(0xe306));
        let _ = done_tx.send(result);
    });
    assert!(reached_rx.recv_timeout(Duration::from_secs(10)).unwrap());
    assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(pin_admission);
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn actual_founder_pin_install_refuses_retirement_before_admission_and_revocation_after_it() {
    for after_admission in [false, true] {
        let (directory, store, _pool, mut rng) = fixture(0xe307);
        let store = Arc::new(store);
        let mut private = store
            .private_managed_founder(
                Hub::new().join(PeerId::from_u64(0xe308)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(0xe308),
                Box::new(ManualClock::new(1)),
                "genuine private founder",
            )
            .unwrap();
        let prepared = private
            .server()
            .unwrap()
            .sync
            .prepare_owner_evidence_checkpoint(9)
            .unwrap();
        let original = private.reserve_scope(&store, &mut rng).unwrap();
        let before = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
        if after_admission {
            let revoke = original.revoker();
            *store
                .owner_evidence
                .before_old_owner_pin_lock
                .lock()
                .unwrap() = Some(Box::new(move || revoke.revoke()));
        } else {
            let retire_store = store.clone();
            let retire_original = original.clone();
            *store
                .owner_evidence
                .before_old_owner_pin_admission
                .lock()
                .unwrap() = Some(Box::new(move || {
                retire_store
                    .retire_chat_preparation(
                        &retire_original,
                        &mut ChaCha20Rng::seed_from_u64(0xe309),
                    )
                    .unwrap();
            }));
        }
        assert!(store
            .install_private_founder_pin(&private, &prepared, &mut NeverRng)
            .is_err());
        assert_eq!(
            fs::read(directory.path().join("chat-owner-pins.bin")).unwrap(),
            before
        );
        assert!(
            store
                .capture_mounted_owner_profile(prepared.group())
                .is_err(),
            "revoked source must not install a new immutable pin"
        );
        // The actual negative seam was reached, rather than an unrelated budget or shape check.
        if after_admission {
            assert!(store
                .owner_evidence
                .before_old_owner_pin_lock
                .lock()
                .unwrap()
                .is_none());
        } else {
            assert!(
                store
                    .owner_evidence
                    .before_old_owner_pin_admission
                    .lock()
                    .unwrap()
                    .is_none(),
                "retirement must run after the source precheck, not the earlier format getter"
            );
            assert!(original.reservation_observer().is_cancelled());
        }
    }
}
