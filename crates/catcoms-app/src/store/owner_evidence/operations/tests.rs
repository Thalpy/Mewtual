//! Actual v3/v4 stores, fixed intent stages and constructor recovery; no save-success booleans.
use super::*;
use crate::store::owner_evidence::canonical::{OwnerEvidenceBudget, OwnerRestoreClassification};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"actual owner operations Store upgrade";
fn v3(path: &Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng).unwrap();
    store.enable_chat_scope_management(SECRET, rng).unwrap();
    assert_eq!(store.chat_vault_format().unwrap(), VaultFormat::ChatScope);
    store
}
fn v4(path: &Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = v3(path, rng)
        .into_owner_evidence_store(SECRET, rng)
        .unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    store
}
fn budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        4,
        16 * 1024 * 1024,
        16 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
}
fn begin(
    store: &ServerStore,
    peer: u64,
) -> super::super::canonical::OwnerFounderInitialization<catcoms_rt::MemNetwork, ChaCha20Rng> {
    store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1000)),
            "operation vault fixture",
            1,
        )
        .unwrap()
}
fn canonical_bytes(path: &Path) -> BTreeMap<std::ffi::OsString, Vec<u8>> {
    fs::read_dir(path.join("owner-state"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect()
}

fn contained_snapshot(plain: &[u8]) -> &[u8] {
    let mut decoder = Decoder::new(plain);
    assert_eq!(
        decoder.get_str().unwrap(),
        "catcoms/owner-canonical-snapshot/v1"
    );
    for _ in 0..5 {
        decoder.get_bytes().unwrap();
    }
    decoder.get_u64().unwrap();
    decoder.get_u64().unwrap();
    decoder.get_bytes().unwrap();
    decoder.get_bytes().unwrap();
    let snapshot = decoder.get_bytes().unwrap();
    decoder.finish().unwrap();
    snapshot
}

#[test]
fn operations_upgrade_initializes_legitimate_v3_absence_before_intent_and_cold_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe001);
    let store = v3(directory.path(), &mut rng);
    assert!(!directory.path().join("chat-owner-pins.bin").exists());
    let domain = store.chat_vault_domain().unwrap();
    let upgraded = store.into_owner_operations_store(SECRET, &mut rng).unwrap();
    assert_eq!(
        upgraded.chat_vault_format().unwrap(),
        VaultFormat::OwnerOperations
    );
    assert_eq!(upgraded.chat_vault_domain().unwrap(), domain);
    assert!(directory.path().join("chat-owner-pins.bin").is_file());
    assert!(directory.path().join(FILE).is_file());
    drop(upgraded);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::OwnerOperations
    );
    assert_eq!(reopened.chat_vault_domain().unwrap(), domain);
}

#[test]
fn operations_upgrade_preserves_current_retired_initializing_heads_and_original_native_owner() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe002);
    let store = v4(directory.path(), &mut rng);
    let mut current = begin(&store, 0xe003);
    let current = current.retry(&store, &mut rng).unwrap();
    let current_group = current.group();
    let current_signer = *current.founder();
    let current_profile = current.profile();
    let mut retired = begin(&store, 0xe004);
    let retired = retired.retry(&store, &mut rng).unwrap();
    let retired_group = retired.group();
    let retired_signer = *retired.founder();
    retired.retire_local_owner(&store, &mut rng).unwrap();
    let mut initializing = begin(&store, 0xe005);
    let initializing_group = initializing.group().unwrap().to_vec();
    let initializing_signer = *initializing.founder().unwrap();
    store
        .owner_evidence
        .canonical_fault
        .store(2, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        initializing.retry(&store, &mut rng),
        Err(AppError::CommittedButNotDurable(_))
    ));
    let original_files = canonical_bytes(directory.path());
    let domain = store.chat_vault_domain().unwrap();
    let store = store.into_owner_operations_store(SECRET, &mut rng).unwrap();
    assert!(
        current.is_live(),
        "format transition preserves the original mounted writer"
    );
    assert!(!retired.is_live());
    assert_eq!(store.chat_vault_domain().unwrap(), domain);
    assert_eq!(canonical_bytes(directory.path()), original_files);
    let raw = original_files.values().next().unwrap();
    let plain =
        Zeroizing::new(unseal(&store.keys.db_key().unwrap(), &unframe(raw).unwrap()).unwrap());
    assert!(
        store
            .save_server(99, contained_snapshot(&plain), &mut rng)
            .is_err(),
        "v5 must not reopen a pinned numeric alias"
    );
    assert!(!directory.path().join("servers/99.bin").exists());
    assert_eq!(
        store
            .capture_mounted_owner_profile(&current_group)
            .unwrap()
            .profile(),
        current_profile
    );
    drop(initializing);
    drop(retired);
    drop(current);
    drop(store);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    reopened.configure_owner_evidence_budget(budget()).unwrap();
    assert!(matches!(
        reopened
            .prepare_owner_evidence_restore(&current_group, current_signer)
            .unwrap(),
        OwnerRestoreClassification::Current(_)
    ));
    assert!(matches!(
        reopened
            .prepare_owner_evidence_restore(&retired_group, retired_signer)
            .unwrap(),
        OwnerRestoreClassification::Retired(_)
    ));
    assert!(matches!(
        reopened
            .prepare_owner_evidence_restore(&initializing_group, initializing_signer)
            .unwrap(),
        OwnerRestoreClassification::Initializing(_)
    ));
    assert_eq!(canonical_bytes(directory.path()), original_files);
}

#[test]
fn operations_marker_real_pre_and_post_rename_failures_never_return_a_usable_old_mount() {
    for fault in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe006 + u64::from(fault));
        let store = v4(directory.path(), &mut rng);
        let original_domain = store.chat_vault_domain().unwrap();
        let original_mount = store.chat_mount.observer();
        store
            .owner_evidence
            .operations_marker_fault
            .store(fault, std::sync::atomic::Ordering::SeqCst);
        let result = store.into_owner_operations_store(SECRET, &mut rng);
        assert!(result.is_err());
        assert_eq!(
            matches!(result, Err(AppError::CommittedButNotDurable(_))),
            fault == 2
        );
        assert!(original_mount.is_cancelled());
        let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
        assert_eq!(reopened.chat_vault_domain().unwrap(), original_domain);
        assert_eq!(
            reopened.chat_vault_format().unwrap(),
            if fault == 2 {
                VaultFormat::OwnerOperations
            } else {
                VaultFormat::OwnerEvidence
            }
        );
    }
}

#[test]
fn operations_valid_staged_intent_is_promoted_by_actual_constructor_after_process_loss() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe009);
    let store = v4(directory.path(), &mut rng);
    let domain = store.chat_vault_domain().unwrap();
    store
        .owner_evidence
        .operations_marker_fault
        .store(3, std::sync::atomic::Ordering::SeqCst);
    assert!(store.into_owner_operations_store(SECRET, &mut rng).is_err());
    assert!(!directory.path().join(FILE).exists());
    let original_stage = fs::read(directory.path().join(STAGE)).unwrap();
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert_eq!(reopened.chat_vault_domain().unwrap(), domain);
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::OwnerOperations
    );
    assert_eq!(
        fs::read(directory.path().join(FILE)).unwrap(),
        original_stage
    );
    assert!(!directory.path().join(STAGE).exists());
}

#[test]
fn operations_intent_with_replayed_authentic_v3_wrapper_requires_existing_nonempty_owner_pins() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe00a);
    let store = v3(directory.path(), &mut rng);
    let old_v3 = fs::read(directory.path().join("vault.bin")).unwrap();
    let store = store.into_owner_evidence_store(SECRET, &mut rng).unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    let mut attempt = begin(&store, 0xe00b);
    let owned = attempt.retry(&store, &mut rng).unwrap();
    let group = owned.group();
    let profile = owned.profile();
    let original_files = canonical_bytes(directory.path());
    store
        .owner_evidence
        .operations_marker_fault
        .store(3, std::sync::atomic::Ordering::SeqCst);
    assert!(store.into_owner_operations_store(SECRET, &mut rng).is_err());
    drop(owned);
    drop(attempt);
    fs::write(directory.path().join("vault.bin"), old_v3).unwrap();
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::OwnerOperations
    );
    assert_eq!(
        reopened
            .capture_mounted_owner_profile(&group)
            .unwrap()
            .profile(),
        profile
    );
    assert_eq!(canonical_bytes(directory.path()), original_files);
}

#[test]
fn operations_pending_intent_never_manufactures_a_missing_mandatory_owner_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe00c);
    let store = v4(directory.path(), &mut rng);
    store
        .owner_evidence
        .operations_marker_fault
        .store(3, std::sync::atomic::Ordering::SeqCst);
    assert!(store.into_owner_operations_store(SECRET, &mut rng).is_err());
    fs::remove_file(directory.path().join("chat-owner-pins.bin")).unwrap();
    let wrapper = fs::read(directory.path().join("vault.bin")).unwrap();
    let stage = fs::read(directory.path().join(STAGE)).unwrap();
    assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
    assert_eq!(
        fs::read(directory.path().join("vault.bin")).unwrap(),
        wrapper
    );
    assert_eq!(fs::read(directory.path().join(STAGE)).unwrap(), stage);
    assert!(!directory.path().join("chat-owner-pins.bin").exists());
}

#[test]
fn operations_missing_vault_with_current_or_stage_intent_refuses_before_generating_an_identity() {
    for name in [FILE, STAGE] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe00d);
        fs::write(directory.path().join(name), b"present unsupported intent").unwrap();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert!(!directory.path().join("vault.bin").exists());
        assert_eq!(
            fs::read(directory.path().join(name)).unwrap(),
            b"present unsupported intent"
        );
    }
}

#[cfg(unix)]
#[test]
fn operations_broken_intent_link_is_present_before_any_new_vault_creation() {
    for name in [FILE, STAGE] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe011);
        std::os::unix::fs::symlink(
            directory.path().join("missing-target"),
            directory.path().join(name),
        )
        .unwrap();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert!(!directory.path().join("vault.bin").exists());
        assert!(fs::symlink_metadata(directory.path().join(name))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn operations_dangling_vault_link_with_actual_retained_intent_never_generates_a_new_dek() {
    for marker in [FILE, STAGE] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe012);
        let store = v4(directory.path(), &mut rng)
            .into_owner_operations_store(SECRET, &mut rng)
            .unwrap();
        let admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
        let pins = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
        drop(store);
        if marker == STAGE {
            fs::rename(directory.path().join(FILE), directory.path().join(STAGE)).unwrap();
        }
        let intent = fs::read(directory.path().join(marker)).unwrap();
        fs::remove_file(directory.path().join("vault.bin")).unwrap();
        std::os::unix::fs::symlink(
            directory.path().join("missing-vault-target"),
            directory.path().join("vault.bin"),
        )
        .unwrap();
        let mut expected_rng = rng.clone();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(
            rand_core::RngCore::next_u64(&mut rng),
            rand_core::RngCore::next_u64(&mut expected_rng),
            "DEK/salt/nonce generation was reached"
        );
        assert!(fs::symlink_metadata(directory.path().join("vault.bin"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(directory.path().join(marker)).unwrap(), intent);
        assert_eq!(
            fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
            admissions
        );
        assert_eq!(
            fs::read(directory.path().join("chat-owner-pins.bin")).unwrap(),
            pins
        );
    }
}

#[test]
fn operations_invalid_intent_refuses_before_owner_root_stage_cleanup_and_preserves_all_bytes() {
    for name in [FILE, STAGE] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe00e);
        drop(v4(directory.path(), &mut rng));
        fs::write(directory.path().join(name), b"invalid intent").unwrap();
        fs::write(
            directory.path().join("chat-owner-pins.replace.tmp"),
            b"must not clean",
        )
        .unwrap();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(
            fs::read(directory.path().join(name)).unwrap(),
            b"invalid intent"
        );
        assert_eq!(
            fs::read(directory.path().join("chat-owner-pins.replace.tmp")).unwrap(),
            b"must not clean"
        );
    }
}

#[test]
fn operations_duplicate_valid_stage_is_settled_and_v5_missing_intent_never_downgrades() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe00f);
    let store = v4(directory.path(), &mut rng)
        .into_owner_operations_store(SECRET, &mut rng)
        .unwrap();
    let current = fs::read(directory.path().join(FILE)).unwrap();
    fs::write(directory.path().join(STAGE), &current).unwrap();
    drop(store);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert!(!directory.path().join(STAGE).exists());
    drop(store);
    fs::remove_file(directory.path().join(FILE)).unwrap();
    let wrapper = fs::read(directory.path().join("vault.bin")).unwrap();
    assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
    assert_eq!(
        fs::read(directory.path().join("vault.bin")).unwrap(),
        wrapper
    );
}

#[test]
fn operations_foreign_domain_oversized_and_nonfile_intents_remain_present_refusals() {
    for variant in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe010 + variant);
        let store = v4(directory.path(), &mut rng);
        let path = directory.path().join(FILE);
        if variant == 0 {
            let mut encoder = Encoder::new();
            encoder.put_str(TAG).unwrap();
            encoder.put_bytes(&[0; 32]).unwrap();
            let sealed = seal(&store.keys.db_key().unwrap(), &encoder.finish(), &mut rng).unwrap();
            fs::write(&path, frame(&sealed)).unwrap();
        } else if variant == 1 {
            fs::write(&path, vec![0; MAX_FRAMED + 1]).unwrap();
        } else {
            fs::create_dir(&path).unwrap();
        }
        drop(store);
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert!(fs::symlink_metadata(path).is_ok());
    }
}
