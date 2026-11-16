//! Actual constructor ordering and fixed-source barriers. All tests are authored, UNRUN while
//! compilation is paused. Sealed fixtures never stand in for Current/Saved/fresh authority.
use super::*;
use crate::store::{OwnerEvidenceBudget, OwnerRestoreClassification};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::collections::BTreeMap;

const SECRET: &[u8] = b"critical namespace catalog fixture";

fn v5(path: &Path, rng: &mut ChaCha20Rng) -> ServerStore {
    ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_owner_operations_store(SECRET, rng)
        .unwrap()
}
fn v6(path: &Path, rng: &mut ChaCha20Rng) -> ServerStore {
    ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, rng)
        .unwrap()
}
fn path(root: &Path, slot: Slot) -> PathBuf {
    root.join(slot.name())
}
fn install_fixture(
    store: &ServerStore,
    slot: Slot,
    plain: &[u8],
    rng: &mut ChaCha20Rng,
) -> Vec<u8> {
    let framed = frame(&seal(&store.keys.db_key().unwrap(), plain, rng).unwrap());
    fs::write(path(&store.dir, slot), &framed).unwrap();
    framed
}
fn record(domain: &[u8; 32], generation: u64, entries: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let mut encoder = Encoder::new();
    encoder.put_str(TAG).unwrap();
    encoder.put_bytes(domain).unwrap();
    encoder.put_u64(generation);
    encoder.put_u32(entries.len() as u32);
    for (id, version, payload) in entries {
        encoder.put_u32(*id);
        encoder.put_u32(*version);
        encoder.put_bytes(payload).unwrap();
    }
    encoder.finish()
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
) -> crate::store::OwnerFounderInitialization<catcoms_rt::MemNetwork, ChaCha20Rng> {
    store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1000)),
            "catalog source",
            1,
        )
        .unwrap()
}

#[test]
fn catalog_codec_refuses_all_nonempty_generations_duplicate_ids_and_malformed_frames() {
    let domain = [3; 32];
    let empty = record(&domain, 1, &[]);
    assert!(validate_empty(&empty, &domain).is_ok());
    for bytes in [
        record(&domain, 0, &[]),
        record(&domain, 2, &[]),
        record(&domain, u64::MAX, &[]),
        record(&domain, 1, &[(1, 1, b"future join")]),
        record(&domain, 1, &[(2, 1, b"future archive")]),
        record(&domain, 1, &[(1, 1, b""), (1, 1, b"")]),
        record(&domain, 1, &[(1, 0, b"")]),
        record(&domain, 1, &[(0, 1, b"")]),
        record(&[4; 32], 1, &[]),
    ] {
        assert!(validate_empty(&bytes, &domain).is_err());
    }
    for length in 0..empty.len() {
        assert!(validate_empty(&empty[..length], &domain).is_err());
    }
    let mut trailing = empty.clone();
    trailing.push(0);
    assert!(validate_empty(&trailing, &domain).is_err());
    let mut count = empty;
    let last = count.len() - 4;
    count[last..].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(validate_empty(&count, &domain).is_err());
    assert!(validate_empty(&vec![0; MAX_PLAIN + 1], &domain).is_err());
}

#[test]
fn catalog_consuming_upgrade_and_cold_reopen_preserve_keys_and_sticky_old_activation() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0001);
    let store = v5(directory.path(), &mut rng);
    let domain = store.chat_vault_domain().unwrap();
    let admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
    let observer = store.chat_mount.observer();
    let store = store
        .into_critical_namespace_store(SECRET, &mut rng)
        .unwrap();
    assert!(!observer.is_cancelled());
    assert_eq!(
        store.chat_vault_format().unwrap(),
        VaultFormat::CriticalNamespaces
    );
    assert_eq!(store.chat_vault_domain().unwrap(), domain);
    let after_admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    assert_eq!(
        unseal(
            &store.keys.db_key().unwrap(),
            &unframe(&after_admissions).unwrap()
        )
        .unwrap(),
        unseal(
            &store.keys.db_key().unwrap(),
            &unframe(&admissions).unwrap()
        )
        .unwrap()
    );
    // Existing activation may reseal the SAME table; inspect its original logical bytes too.
    let before_pin_plain = unseal(&store.keys.db_key().unwrap(), &unframe(&pins).unwrap()).unwrap();
    let after_pin = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
    assert_eq!(
        unseal(&store.keys.db_key().unwrap(), &unframe(&after_pin).unwrap()).unwrap(),
        before_pin_plain
    );
    let catalog = fs::read(path(directory.path(), Slot::Current)).unwrap();
    let store = store.into_owner_operations_store(SECRET, &mut rng).unwrap();
    assert_eq!(
        store.chat_vault_format().unwrap(),
        VaultFormat::CriticalNamespaces
    );
    assert_eq!(
        fs::read(path(directory.path(), Slot::Current)).unwrap(),
        catalog
    );
    drop(store);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::CriticalNamespaces
    );
    assert_eq!(reopened.chat_vault_domain().unwrap(), domain);
}

#[test]
fn catalog_retained_stage_and_actual_postrename_parent_loss_recover_on_cold_open() {
    for fault in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0002 + u64::from(fault));
        let store = v5(directory.path(), &mut rng);
        let domain = store.chat_vault_domain().unwrap();
        store
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(store
            .into_critical_namespace_store(SECRET, &mut rng)
            .is_err());
        let retained = path(
            directory.path(),
            if fault == 1 {
                Slot::Stage
            } else {
                Slot::Current
            },
        );
        assert!(retained.is_file());
        assert_eq!(fs::read(directory.path().join("vault.bin")).unwrap()[0], 5);
        let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
        assert_eq!(
            reopened.chat_vault_format().unwrap(),
            VaultFormat::CriticalNamespaces
        );
        assert_eq!(reopened.chat_vault_domain().unwrap(), domain);
        assert!(path(directory.path(), Slot::Current).is_file());
        assert!(!path(directory.path(), Slot::Stage).exists());
    }
}

#[test]
fn catalog_equivalent_plaintext_duplicate_stage_does_not_require_equal_ciphertext() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0005);
    let store = v6(directory.path(), &mut rng);
    let current = fs::read(path(directory.path(), Slot::Current)).unwrap();
    let plain = encode_empty(&store.chat_vault_domain().unwrap()).unwrap();
    let stage = install_fixture(&store, Slot::Stage, &plain, &mut rng);
    assert_ne!(current, stage);
    drop(store);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::CriticalNamespaces
    );
    assert_eq!(
        fs::read(path(directory.path(), Slot::Current)).unwrap(),
        current
    );
    assert!(!path(directory.path(), Slot::Stage).exists());
}

#[test]
fn catalog_unknown_current_or_stage_refuses_before_old_pin_stage_cleaner_and_preserves_all_bytes() {
    for slot in [Slot::Current, Slot::Stage] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0006);
        let store = ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_owner_evidence_store(SECRET, &mut rng)
            .unwrap();
        let old_v4 = fs::read(directory.path().join("vault.bin")).unwrap();
        let store = store
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap();
        let unknown = record(
            &store.chat_vault_domain().unwrap(),
            1,
            &[(1, 1, b"not supported")],
        );
        let original = install_fixture(&store, slot, &unknown, &mut rng);
        let pin_stage = directory.path().join("chat-owner-pins.replace.tmp");
        fs::write(&pin_stage, b"must not enter an older cleaner").unwrap();
        // Replay an authentic older wrapper and remove its later operations intent so the old
        // gate WOULD enter the v4 pin-stage cleaner if it ran before critical inspection. Merely
        // leaving v6+ops intent would also refuse the pin stage and miss this ordering defect.
        fs::write(directory.path().join("vault.bin"), &old_v4).unwrap();
        fs::remove_file(directory.path().join("chat-owner-operations.bin")).unwrap();
        drop(store);
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(fs::read(path(directory.path(), slot)).unwrap(), original);
        assert_eq!(
            fs::read(pin_stage).unwrap(),
            b"must not enter an older cleaner"
        );
        assert_eq!(
            fs::read(directory.path().join("vault.bin")).unwrap(),
            old_v4
        );
    }
}

#[test]
fn catalog_valid_empty_intent_with_old_wrapper_cannot_recreate_missing_v5_intent_or_run_old_cleaner(
) {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0011);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    let old_v4 = fs::read(directory.path().join("vault.bin")).unwrap();
    let store = store
        .into_critical_namespace_store(SECRET, &mut rng)
        .unwrap();
    let catalog = fs::read(path(directory.path(), Slot::Current)).unwrap();
    let pin_stage = directory.path().join("chat-owner-pins.replace.tmp");
    fs::write(&pin_stage, b"original unknown pin candidate").unwrap();
    fs::remove_file(directory.path().join("chat-owner-operations.bin")).unwrap();
    fs::write(directory.path().join("vault.bin"), &old_v4).unwrap();
    drop(store);
    assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
    assert_eq!(
        fs::read(pin_stage).unwrap(),
        b"original unknown pin candidate"
    );
    assert!(!directory.path().join("chat-owner-operations.bin").exists());
    assert_eq!(
        fs::read(path(directory.path(), Slot::Current)).unwrap(),
        catalog
    );
    assert_eq!(
        fs::read(directory.path().join("vault.bin")).unwrap(),
        old_v4
    );
}

#[test]
fn catalog_missing_mandatory_catalog_operation_intent_or_old_inventory_never_initializes_absence() {
    for missing in [
        Slot::Current.name(),
        "chat-owner-operations.bin",
        "chat-admissions.bin",
        "chat-owner-pins.bin",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0007);
        let store = v6(directory.path(), &mut rng);
        let wrapper = fs::read(directory.path().join("vault.bin")).unwrap();
        fs::remove_file(directory.path().join(missing)).unwrap();
        drop(store);
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert!(!directory.path().join(missing).exists());
        assert_eq!(
            fs::read(directory.path().join("vault.bin")).unwrap(),
            wrapper
        );
    }
}

#[test]
fn catalog_foreign_domain_truncation_corruption_and_successor_generation_are_not_settled() {
    for case in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0008 + case);
        let store = v6(directory.path(), &mut rng);
        let domain = store.chat_vault_domain().unwrap();
        let mut bytes = match case {
            0 => record(&[9; 32], 1, &[]),
            1 => record(&domain, 1, &[]),
            2 => record(&domain, 2, &[]),
            _ => record(&domain, 1, &[]),
        };
        if case == 1 {
            bytes.pop();
        }
        let mut framed = install_fixture(&store, Slot::Stage, &bytes, &mut rng);
        if case == 3 {
            framed[0] ^= 1;
            fs::write(path(directory.path(), Slot::Stage), &framed).unwrap();
        }
        let current = fs::read(path(directory.path(), Slot::Current)).unwrap();
        drop(store);
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(
            fs::read(path(directory.path(), Slot::Stage)).unwrap(),
            framed
        );
        assert_eq!(
            fs::read(path(directory.path(), Slot::Current)).unwrap(),
            current
        );
    }
}

#[test]
fn catalog_orphan_names_refuse_before_rng_or_new_dek() {
    for slot in [Slot::Current, Slot::Stage] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_000c);
        let mut unchanged = rng.clone();
        fs::write(path(directory.path(), slot), b"retained uncertain source").unwrap();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert!(!directory.path().join("vault.bin").exists());
        assert_eq!(rng.next_u64(), unchanged.next_u64());
        assert_eq!(
            fs::read(path(directory.path(), slot)).unwrap(),
            b"retained uncertain source"
        );
    }
}

#[test]
fn undeclared_archive_objects_refuse_before_new_vault_or_old_cleaner() {
    // A private namespace may have reached disk while its catalog transaction was interrupted.
    // Neither the default nor Pending-only handler can turn its absence from an Empty catalog
    // into permission to clean it or initialize another identity. These are real constructors,
    // not a codec-only assertion that an unknown descriptor fails to parse.
    for name in [ARCHIVE_BOOTSTRAP_ROOT, ARCHIVE_DECLARATION_SEED] {
        for directory_object in [false, true] {
            let fresh = tempfile::tempdir().unwrap();
            let object = fresh.path().join(name);
            if directory_object {
                fs::create_dir(&object).unwrap();
            } else {
                fs::write(&object, b"unknown archive source").unwrap();
            }
            let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0020);
            let mut unchanged = rng.clone();
            assert!(ServerStore::open(fresh.path(), SECRET, &mut rng).is_err());
            assert!(!fresh.path().join("vault.bin").exists());
            assert_eq!(rng.next_u64(), unchanged.next_u64());
            assert_eq!(
                fs::symlink_metadata(&object).unwrap().is_dir(),
                directory_object
            );

            for pending_only in [false, true] {
                let old = tempfile::tempdir().unwrap();
                let store = ServerStore::open(old.path(), SECRET, &mut rng)
                    .unwrap()
                    .into_owner_evidence_store(SECRET, &mut rng)
                    .unwrap();
                let vault = fs::read(old.path().join("vault.bin")).unwrap();
                let store = store
                    .into_critical_namespace_store(SECRET, &mut rng)
                    .unwrap();
                let catalog = fs::read(path(old.path(), Slot::Current)).unwrap();
                let pin_stage = old.path().join("chat-owner-pins.replace.tmp");
                fs::write(&pin_stage, b"older cleaner must not run").unwrap();
                // With this authentic v4 wrapper and no operations intent, the lower gate
                // WOULD clean the pin stage. An unchanged v6 wrapper would also refuse it,
                // so that weaker fixture would not establish the archive preflight order.
                fs::write(old.path().join("vault.bin"), &vault).unwrap();
                fs::remove_file(old.path().join("chat-owner-operations.bin")).unwrap();
                let object = old.path().join(name);
                if directory_object {
                    fs::create_dir(&object).unwrap();
                    fs::write(object.join("retained"), b"private history").unwrap();
                } else {
                    fs::write(&object, b"unknown archive source").unwrap();
                }
                drop(store);
                let mut unchanged = rng.clone();
                let opened = if pending_only {
                    let pool = CriticalCatalogReadPool::new(256 * 1024 * 1024).unwrap();
                    let policy = PendingJoinCatalogPolicy::new(
                        pool,
                        2 * pending_join_codec::MAX_CATALOG_FRAMED,
                    )
                    .unwrap();
                    ServerStore::open_with_pending_join_catalog(
                        old.path(),
                        SECRET,
                        &mut rng,
                        policy,
                    )
                } else {
                    ServerStore::open(old.path(), SECRET, &mut rng)
                };
                assert!(opened.is_err());
                assert_eq!(rng.next_u64(), unchanged.next_u64());
                assert_eq!(fs::read(&pin_stage).unwrap(), b"older cleaner must not run");
                assert_eq!(fs::read(path(old.path(), Slot::Current)).unwrap(), catalog);
                assert_eq!(fs::read(old.path().join("vault.bin")).unwrap(), vault);
                if directory_object {
                    assert_eq!(
                        fs::read(object.join("retained")).unwrap(),
                        b"private history"
                    );
                } else {
                    assert_eq!(fs::read(object).unwrap(), b"unknown archive source");
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn dangling_archive_objects_are_present_and_never_new_vault_authority() {
    for name in [ARCHIVE_BOOTSTRAP_ROOT, ARCHIVE_DECLARATION_SEED] {
        let directory = tempfile::tempdir().unwrap();
        let object = directory.path().join(name);
        std::os::unix::fs::symlink(directory.path().join("missing-target"), &object).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0021);
        let mut unchanged = rng.clone();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(rng.next_u64(), unchanged.next_u64());
        assert!(!directory.path().join("vault.bin").exists());
        assert!(fs::symlink_metadata(object)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[test]
fn catalog_nonregular_fixed_source_refuses_and_never_removes_it() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_000d);
    let store = v6(directory.path(), &mut rng);
    fs::create_dir(path(directory.path(), Slot::Stage)).unwrap();
    drop(store);
    assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
    assert!(path(directory.path(), Slot::Stage).is_dir());
}

#[cfg(unix)]
#[test]
fn catalog_dangling_wrapper_and_catalog_links_are_present_not_new_identity_authority() {
    for dangling_wrapper in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_000e);
        let store = v6(directory.path(), &mut rng);
        drop(store);
        let replaced = if dangling_wrapper {
            directory.path().join("vault.bin")
        } else {
            path(directory.path(), Slot::Current)
        };
        fs::remove_file(&replaced).unwrap();
        std::os::unix::fs::symlink(directory.path().join("missing-target"), &replaced).unwrap();
        let mut unchanged = rng.clone();
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(rng.next_u64(), unchanged.next_u64());
        assert!(fs::symlink_metadata(replaced)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[test]
fn catalog_upgrade_preserves_current_retired_initializing_canonical_sources_and_positive_restore() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_000f);
    let store = v5(directory.path(), &mut rng);
    store.configure_owner_evidence_budget(budget()).unwrap();
    let mut founder = begin(&store, 0x6001);
    let owner = founder.retry(&store, &mut rng).unwrap();
    let group = owner.group();
    let signer = *owner.founder();
    drop(owner);
    drop(founder);
    let mut retired = begin(&store, 0x6002);
    let retired_owner = retired.retry(&store, &mut rng).unwrap();
    let retired_group = retired_owner.group();
    let retired_signer = *retired_owner.founder();
    retired_owner.retire_local_owner(&store, &mut rng).unwrap();
    drop(retired_owner);
    drop(retired);
    let mut initializing = begin(&store, 0x6003);
    let initializing_group = initializing.group().unwrap().to_vec();
    let initializing_signer = *initializing.founder().unwrap();
    store
        .owner_evidence
        .canonical_fault
        .store(2, Ordering::SeqCst);
    assert!(initializing.retry(&store, &mut rng).is_err());
    drop(initializing);
    let canonical = fs::read_dir(directory.path().join("owner-state"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    let store = store
        .into_critical_namespace_store(SECRET, &mut rng)
        .unwrap();
    for (name, bytes) in &canonical {
        assert_eq!(
            fs::read(directory.path().join("owner-state").join(name)).unwrap(),
            *bytes
        );
    }
    drop(store);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    reopened.configure_owner_evidence_budget(budget()).unwrap();
    assert!(matches!(
        reopened
            .prepare_owner_evidence_restore(&group, signer)
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
}

#[test]
fn catalog_v6_does_not_widen_v4_source_absent_negative_or_first_stage_fallback() {
    for staged in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_0010);
        let store = v5(directory.path(), &mut rng);
        store.configure_owner_evidence_budget(budget()).unwrap();
        let mut founder = begin(&store, 0x6004);
        let group = founder.group().unwrap().to_vec();
        let signer = *founder.founder().unwrap();
        store
            .owner_evidence
            .canonical_fault
            .store(if staged { 2 } else { 1 }, Ordering::SeqCst);
        assert!(founder.retry(&store, &mut rng).is_err());
        drop(founder);
        if staged {
            let canonical = fs::read_dir(directory.path().join("owner-state"))
                .unwrap()
                .find_map(|entry| {
                    let entry = entry.unwrap();
                    (entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "bin"))
                    .then(|| entry.path())
                })
                .unwrap();
            let filename = canonical.file_stem().unwrap().to_str().unwrap();
            let stage = canonical.with_file_name(format!("{filename}.replace.tmp"));
            fs::rename(canonical, stage).unwrap();
        }
        let store = store
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap();
        drop(store);
        let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
        reopened.configure_owner_evidence_budget(budget()).unwrap();
        assert!(reopened
            .prepare_owner_evidence_restore(&group, signer)
            .is_err());
        let key = crate::store::ChatAdmissionKey::new(&group, signer).unwrap();
        assert!(reopened
            .try_prepare_unexposed_owner_initialization(&key)
            .is_err());
        if staged {
            assert!(fs::read_dir(directory.path().join("owner-state"))
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".replace.tmp")));
        }
    }
}
