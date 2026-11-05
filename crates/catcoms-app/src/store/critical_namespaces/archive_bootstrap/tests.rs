//! Actual private physical-open transaction/fence regressions. All are authored UNRUN while
//! compilation is paused. Raw target fixtures prove negative reservation, never a V2 source.
use super::*;
use crate::store::{ChatAdmissionKey, OwnerEvidenceBudget};
use crate::Server;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::sync::atomic::Ordering;

const SECRET: &[u8] = b"private archive declaration fixture";
fn pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap()
}
fn policy(pool: &CriticalCatalogReadPool) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * MAX_SEED_FRAMED).unwrap()
}
fn initialize(path: &Path, rng: &mut ChaCha20Rng) {
    drop(
        ServerStore::open(path, SECRET, rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, rng)
            .unwrap(),
    );
}
fn open(
    path: &Path,
    rng: &mut ChaCha20Rng,
    pool: &CriticalCatalogReadPool,
) -> PreparedArchiveBootstrapStore {
    PreparedArchiveBootstrapStore::open(path, SECRET, rng, policy(pool)).unwrap()
}
fn activate(path: &Path, rng: &mut ChaCha20Rng, pool: &CriticalCatalogReadPool) -> ServerStore {
    let mut owner = open(path, rng, pool);
    owner.retry(rng).unwrap();
    match owner.into_store() {
        Ok(store) => store,
        Err(_) => panic!("actual namespace barriers did not finish"),
    }
}
// Test-only inspection/fault seams do not exist on the public wrapper. Production cannot
// borrow a Store before completion or construct a declaration from raw scalar/cipher inputs.
fn private(owner: &PreparedArchiveBootstrapStore) -> &ServerStore {
    owner.store.as_ref().unwrap()
}
fn fault(owner: &PreparedArchiveBootstrapStore, point: u8) {
    private(owner)
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}
fn target(store: &ServerStore, group: &[u8], signer: &[u8; 32]) -> PathBuf {
    let name = target_name(&store.chat_vault_domain().unwrap(), group, signer).unwrap();
    store
        .dir
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&name).unwrap())
}
fn node(peer: u64) -> Server<MemNetwork, ChaCha20Rng> {
    Server::found(
        Hub::new().join(PeerId::from_u64(peer)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(10)),
        "numeric fixture",
    )
    .unwrap()
}

#[test]
fn archive_private_open_requires_real_v6_and_prepaid_capacity_before_rng() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9910);
    let shared = pool();
    let mut unchanged = rng.clone();
    assert!(PreparedArchiveBootstrapStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        policy(&shared)
    )
    .is_err());
    assert_eq!(rng.next_u64(), unchanged.next_u64());
    assert!(!directory.path().join("vault.bin").exists());
    initialize(directory.path(), &mut rng);
    let vault = fs::read(directory.path().join("vault.bin")).unwrap();
    let tiny = CriticalCatalogReadPool::new(DECLARATION_FEE - 1).unwrap();
    let mut unchanged = rng.clone();
    assert!(
        PreparedArchiveBootstrapStore::open(directory.path(), SECRET, &mut rng, policy(&tiny))
            .is_err()
    );
    assert_eq!(rng.next_u64(), unchanged.next_u64());
    assert_eq!(fs::read(directory.path().join("vault.bin")).unwrap(), vault);
    assert!(!directory.path().join(ARCHIVE_BOOTSTRAP_ROOT).exists());
}

#[test]
fn archive_incomplete_conversion_returns_same_owner_and_success_survives_cold_open() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9911);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let before_admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    let before_pins = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
    let owner = open(directory.path(), &mut rng, &shared);
    let identity = private(&owner).registry_mount();
    let mut owner = match owner.into_store() {
        Err(owner) => owner,
        Ok(_) => panic!("exposed incomplete Store"),
    };
    assert!(Arc::ptr_eq(&identity, &private(&owner).registry_mount()));
    owner.retry(&mut rng).unwrap();
    let expected = owner.declaration.as_ref().unwrap().candidate.to_vec();
    let mut unchanged = rng.clone();
    owner.retry(&mut rng).unwrap();
    assert_eq!(rng.next_u64(), unchanged.next_u64());
    let store = match owner.into_store() {
        Ok(store) => store,
        Err(_) => panic!("namespace unavailable"),
    };
    assert!(store
        .require_no_archive_bootstrap_target(b"vacant", &[3; 32])
        .is_ok());
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        expected
    );
    assert_eq!(
        fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
        before_admissions
    );
    assert_eq!(
        fs::read(directory.path().join("chat-owner-pins.bin")).unwrap(),
        before_pins
    );
    assert!(!directory.path().join(ARCHIVE_DECLARATION_SEED).exists());
    drop(store);
    assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
    let pending = PendingJoinCatalogPolicy::new(
        shared.clone(),
        2 * super::super::pending_join_codec::MAX_CATALOG_FRAMED,
    )
    .unwrap();
    assert!(ServerStore::open_with_pending_join_catalog(
        directory.path(),
        SECRET,
        &mut rng,
        pending
    )
    .is_err());
    let owner = open(directory.path(), &mut rng, &shared);
    assert!(owner.complete);
    assert!(owner.declaration.is_none());
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        expected
    );
}

#[test]
fn archive_warm_fault_matrix_reuses_exact_ciphertext_handles_and_rng() {
    for point in [101, 111, 102, 103, 104, 105, 107, 108, 109, 110] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x9920 + point as u64);
        let shared = pool();
        initialize(directory.path(), &mut rng);
        let mut owner = open(directory.path(), &mut rng, &shared);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err(), "actual fault {point}");
        assert!(!owner.complete);
        let original = owner.declaration.as_ref().unwrap();
        let candidate = original.candidate.to_vec();
        let marker = original.marker.to_vec();
        let seed = original.seed.to_vec();
        let mut unchanged = rng.clone();
        fault(&owner, 0);
        owner.retry(&mut rng).unwrap();
        assert_eq!(
            rng.next_u64(),
            unchanged.next_u64(),
            "no reseal after fault {point}"
        );
        let original = owner.declaration.as_ref().unwrap();
        assert_eq!(original.candidate.as_slice(), candidate);
        assert_eq!(original.marker.as_slice(), marker);
        assert_eq!(original.seed.as_slice(), seed);
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            candidate
        );
        assert_eq!(
            fs::read(
                directory
                    .path()
                    .join(ARCHIVE_BOOTSTRAP_ROOT)
                    .join(ROOT_MARKER)
            )
            .unwrap(),
            marker
        );
        assert!(!directory.path().join(ARCHIVE_DECLARATION_SEED).exists());
        assert!(!directory.path().join(Slot::Stage.name()).exists());
    }
}

#[test]
fn archive_cold_complete_seed_repairs_exact_stage_marker_and_postrename_barriers() {
    for point in [103, 104, 105, 107, 108, 109, 110] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x9930 + point as u64);
        let shared = pool();
        initialize(directory.path(), &mut rng);
        let mut owner = open(directory.path(), &mut rng, &shared);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err());
        let candidate = owner.declaration.as_ref().unwrap().candidate.to_vec();
        let marker = owner.declaration.as_ref().unwrap().marker.to_vec();
        drop(owner);
        let mut unchanged = rng.clone();
        let owner = open(directory.path(), &mut rng, &shared);
        assert!(owner.complete, "cold phase {point}");
        assert_eq!(
            rng.next_u64(),
            unchanged.next_u64(),
            "cold completion never reseals {point}"
        );
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            candidate
        );
        assert_eq!(
            fs::read(
                directory
                    .path()
                    .join(ARCHIVE_BOOTSTRAP_ROOT)
                    .join(ROOT_MARKER)
            )
            .unwrap(),
            marker
        );
        assert!(!directory.path().join(ARCHIVE_DECLARATION_SEED).exists());
    }
}

#[test]
fn archive_cold_partial_first_seed_is_preserved_and_never_renewed() {
    for point in [111, 102] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x9940 + point as u64);
        let shared = pool();
        initialize(directory.path(), &mut rng);
        let mut owner = open(directory.path(), &mut rng, &shared);
        let current = fs::read(directory.path().join(Slot::Current.name())).unwrap();
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err());
        let seed = fs::read(directory.path().join(ARCHIVE_DECLARATION_SEED)).unwrap();
        drop(owner);
        for _ in 0..2 {
            let mut unchanged = rng.clone();
            assert!(PreparedArchiveBootstrapStore::open(
                directory.path(),
                SECRET,
                &mut rng,
                policy(&shared)
            )
            .is_err());
            assert_eq!(rng.next_u64(), unchanged.next_u64());
            assert_eq!(
                fs::read(directory.path().join(ARCHIVE_DECLARATION_SEED)).unwrap(),
                seed
            );
            assert_eq!(
                fs::read(directory.path().join(Slot::Current.name())).unwrap(),
                current
            );
            assert!(!directory.path().join(ARCHIVE_BOOTSTRAP_ROOT).exists());
        }
    }
}

#[test]
fn archive_foreign_partial_seed_never_becomes_owned_after_a_failed_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9950);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let mut owner = open(directory.path(), &mut rng, &shared);
    fault(&owner, 101); // cached original output exists, but this process created NO seed file
    assert!(owner.retry(&mut rng).is_err());
    let original = owner.declaration.as_ref().unwrap();
    assert!(original.seed_file.is_none());
    let foreign = original.seed[..original.seed.len() / 2].to_vec();
    fs::write(directory.path().join(ARCHIVE_DECLARATION_SEED), &foreign).unwrap();
    fault(&owner, 0);
    for _ in 0..3 {
        let mut unchanged = rng.clone();
        assert!(owner.retry(&mut rng).is_err());
        assert_eq!(rng.next_u64(), unchanged.next_u64());
        assert!(
            owner.declaration.as_ref().unwrap().seed_file.is_none(),
            "failed read cannot launder ownership"
        );
        assert_eq!(
            fs::read(directory.path().join(ARCHIVE_DECLARATION_SEED)).unwrap(),
            foreign
        );
        assert!(!directory.path().join(ARCHIVE_BOOTSTRAP_ROOT).exists());
    }
}

#[test]
fn archive_changed_complete_parent_refuses_despite_equal_parent_plaintext() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9951);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let mut owner = open(directory.path(), &mut rng, &shared);
    fault(&owner, 103);
    assert!(owner.retry(&mut rng).is_err());
    let seed = fs::read(directory.path().join(ARCHIVE_DECLARATION_SEED)).unwrap();
    let key = private(&owner).keys.db_key().unwrap();
    let old = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&old).unwrap()).unwrap());
    let replacement = frame(&seal(&key, &plain, &mut rng).unwrap());
    assert_ne!(old, replacement);
    fs::write(directory.path().join(Slot::Current.name()), &replacement).unwrap();
    drop(owner);
    assert!(PreparedArchiveBootstrapStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        policy(&shared)
    )
    .is_err());
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        replacement
    );
    assert_eq!(
        fs::read(directory.path().join(ARCHIVE_DECLARATION_SEED)).unwrap(),
        seed
    );
    assert!(!directory.path().join(ARCHIVE_BOOTSTRAP_ROOT).exists());
}

#[test]
fn archive_current_declaration_with_missing_root_is_never_recreated_from_its_seed() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9952);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let mut owner = open(directory.path(), &mut rng, &shared);
    fault(&owner, 109); // actual Current rename already occurred; its exact seed remains
    assert!(owner.retry(&mut rng).is_err());
    let root = directory.path().join(ARCHIVE_BOOTSTRAP_ROOT);
    let retained = directory.path().join("retained-original-archive-root");
    fs::rename(&root, &retained).unwrap();
    fault(&owner, 0);
    assert!(owner.retry(&mut rng).is_err());
    assert!(!root.exists());
    drop(owner);
    assert!(PreparedArchiveBootstrapStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        policy(&shared)
    )
    .is_err());
    assert!(!root.exists());
    assert!(retained.join(ROOT_MARKER).is_file());
}

#[test]
fn archive_target_notfound_requires_the_same_held_root_and_marker_objects() {
    for replacement in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x9953);
        let shared = pool();
        initialize(directory.path(), &mut rng);
        let store = activate(directory.path(), &mut rng, &shared);
        assert!(store
            .require_no_archive_bootstrap_target(b"vacant", &[4; 32])
            .is_ok());
        let capture = store
            .capture_chat_admission(ChatAdmissionKey::new(b"vacant", [4; 32]).unwrap())
            .unwrap();
        let root = directory.path().join(ARCHIVE_BOOTSTRAP_ROOT);
        let marker = fs::read(root.join(ROOT_MARKER)).unwrap();
        if replacement == 1 {
            fs::rename(root.join(ROOT_MARKER), root.join("retained-marker")).unwrap();
            fs::write(root.join(ROOT_MARKER), &marker).unwrap();
        } else if replacement == 0 {
            fs::rename(&root, directory.path().join("retained-root")).unwrap();
            fs::create_dir(&root).unwrap();
            fs::write(root.join(ROOT_MARKER), &marker).unwrap();
        } else {
            let identity = {
                store
                    .archive_bootstrap_root
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .marker_id
            };
            let mut corrupted = marker;
            corrupted[0] ^= 1;
            // fs::write truncates the SAME existing object. This test specifically pins the
            // whole-frame digest check, independently of the object/session checks above.
            fs::write(root.join(ROOT_MARKER), &corrupted).unwrap();
            let file = File::open(root.join(ROOT_MARKER)).unwrap();
            assert_eq!(catcoms_storage::kept::session_id(&file).unwrap(), identity);
        }
        assert!(store
            .require_no_archive_bootstrap_target(b"vacant", &[4; 32])
            .is_err());
        assert!(store
            .capture_chat_admission(ChatAdmissionKey::new(b"vacant", [4; 32]).unwrap())
            .is_err());
        let mut unchanged = rng.clone();
        assert!(store
            .prepare_legacy_chat_admission(&capture, b"old candidate", &mut rng)
            .is_err());
        assert_eq!(rng.next_u64(), unchanged.next_u64());
    }
}

#[test]
fn archive_numeric_save_and_load_fence_a_without_a_v1_pin_and_leave_b_available() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9960);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let store = activate(directory.path(), &mut rng, &shared);
    let mut a = node(0x9961);
    let mut b = node(0x9962);
    let a_group = a.group_id();
    let a_signer: [u8; 32] = a.sync.my_public_key().try_into().unwrap();
    let a_bytes = a.snapshot().unwrap();
    let b_bytes = b.snapshot().unwrap();
    store.save_server(1, &a_bytes, &mut rng).unwrap();
    store.save_server(2, &b_bytes, &mut rng).unwrap();
    // initialize/activate created no owner profile; the actual fresh node snapshots above
    // remain unpinned. Keep the complete original pin inventory as independent evidence.
    let pins = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
    fs::write(
        target(&store, &a_group, &a_signer),
        b"unknown retained V2 intent",
    )
    .unwrap();
    let mut unchanged = rng.clone();
    assert!(store.save_server(1, &a_bytes, &mut rng).is_err());
    assert_eq!(rng.next_u64(), unchanged.next_u64());
    assert!(store.load_server(1).is_err());
    assert_eq!(store.load_server(2).unwrap().as_slice(), b_bytes.as_slice());
    store.save_server(2, &b_bytes, &mut rng).unwrap();
    assert_eq!(
        fs::read(target(&store, &a_group, &a_signer)).unwrap(),
        b"unknown retained V2 intent"
    );
    assert_eq!(
        fs::read(directory.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
}

#[test]
fn archive_replacement_after_target_notfound_is_rejected_before_old_issuance() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9964);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let store = activate(directory.path(), &mut rng, &shared);
    let capture = store
        .capture_chat_admission(ChatAdmissionKey::new(b"vacant-race", [8; 32]).unwrap())
        .unwrap();
    let root = directory.path().join(ARCHIVE_BOOTSTRAP_ROOT);
    let marker = fs::read(root.join(ROOT_MARKER)).unwrap();
    let retained = directory.path().join("retained-root-after-lookup");
    *store.archive_after_target_lookup.lock().unwrap() = Some(Box::new(move || {
        // The real first lookup just saw target NotFound under the original root. A second
        // root with byte-identical marker must not turn that observation into admission.
        fs::rename(&root, &retained).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join(ROOT_MARKER), marker).unwrap();
    }));
    let admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    let mut unchanged = rng.clone();
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"old candidate", &mut rng)
        .is_err());
    assert_eq!(rng.next_u64(), unchanged.next_u64());
    assert_eq!(
        fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
        admissions
    );
    assert!(
        store.archive_after_target_lookup.lock().unwrap().is_none(),
        "actual during-check seam ran"
    );
}

#[test]
fn archive_unknown_a_blocks_old_issued_retry_but_an_actual_b_founder_can_save() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x9970);
    let shared = pool();
    initialize(directory.path(), &mut rng);
    let store = activate(directory.path(), &mut rng, &shared);
    let capture = store
        .capture_chat_admission(ChatAdmissionKey::new(b"held-a", [7; 32]).unwrap())
        .unwrap();
    store
        .prepare_legacy_chat_admission(&capture, b"original old source", &mut rng)
        .unwrap();
    fs::create_dir(target(&store, b"held-a", &[7; 32])).unwrap();
    let admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    let mut unchanged = rng.clone();
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"original old source", &mut rng)
        .is_err());
    assert_eq!(rng.next_u64(), unchanged.next_u64());
    assert_eq!(
        fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
        admissions
    );
    store
        .configure_owner_evidence_budget(
            OwnerEvidenceBudget::checkpoint(
                8,
                128 * 1024 * 1024,
                128 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                512 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
    let mut b = store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(0x9971)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x9971),
            Box::new(ManualClock::new(10)),
            "independent B",
            1,
        )
        .unwrap();
    let b = b.retry(&store, &mut rng).unwrap();
    assert!(
        b.is_live(),
        "known bad A must not block unrelated real saved/current B"
    );
}
