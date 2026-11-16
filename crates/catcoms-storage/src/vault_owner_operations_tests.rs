//! Real wrapper barriers/KDF compatibility. Host marker/inventory obligations live in App tests.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"critical owner operations wrapper";
fn classified(dir: &Path, rng: &mut ChaCha20Rng) -> (KeyHierarchy, VaultFormat) {
    open_or_create_vault_classified(dir, SECRET, rng)
        .unwrap()
        .into_parts()
}

#[test]
fn operations_wrapper_preserves_dek_and_is_sticky_across_old_activation_and_secret_paths() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x05ca_1101);
    let original = classified(directory.path(), &mut rng).0.db_key().unwrap();
    activate_chat_scope_vault(directory.path(), SECRET, &mut rng).unwrap();
    activate_owner_evidence_vault(directory.path(), SECRET, &mut rng).unwrap();
    activate_owner_operations_vault(directory.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        classified(directory.path(), &mut rng).1,
        VaultFormat::OwnerOperations
    );
    activate_chat_scope_vault(directory.path(), SECRET, &mut rng).unwrap();
    activate_owner_evidence_vault(directory.path(), SECRET, &mut rng).unwrap();
    let (same, format) = classified(directory.path(), &mut rng);
    assert_eq!(format, VaultFormat::OwnerOperations);
    assert!(format.requires_owner_evidence());
    assert_eq!(same.db_key().unwrap(), original);
    change_vault_passphrase(directory.path(), SECRET, b"new operation secret", &mut rng).unwrap();
    let (same, format) =
        open_or_create_vault_classified(directory.path(), b"new operation secret", &mut rng)
            .unwrap()
            .into_parts();
    assert_eq!(format, VaultFormat::OwnerOperations);
    assert_eq!(same.db_key().unwrap(), original);
    assert_eq!(
        std::fs::read(directory.path().join(VAULT_FILE)).unwrap()[0],
        VAULT_VERSION_OWNER_OPERATIONS
    );
}

#[test]
fn operations_wrapper_cannot_be_downgraded_by_changing_its_header() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x05ca_1102);
    let _ = classified(directory.path(), &mut rng);
    activate_owner_operations_vault(directory.path(), SECRET, &mut rng).unwrap();
    let path = directory.path().join(VAULT_FILE);
    let original = std::fs::read(&path).unwrap();
    assert_eq!(original.len(), VAULT_ENCODED_LEN);
    for version in [
        VAULT_VERSION_RAW_SECRET,
        VAULT_VERSION_PREHASHED_SECRET,
        VAULT_VERSION_CHAT_SCOPE,
        VAULT_VERSION_OWNER_EVIDENCE,
    ] {
        let mut forged = original.clone();
        forged[0] = version;
        std::fs::write(&path, &forged).unwrap();
        assert!(open_or_create_vault_classified(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), forged);
    }
    std::fs::write(path, original).unwrap();
    assert_eq!(
        classified(directory.path(), &mut rng).1,
        VaultFormat::OwnerOperations
    );
}

#[test]
fn operations_wrapper_actual_rename_orderings_preserve_old_or_require_reopen_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x05ca_1103);
    let old_key = classified(directory.path(), &mut rng).0.db_key().unwrap();
    activate_owner_evidence_vault(directory.path(), SECRET, &mut rng).unwrap();
    let path = directory.path().join(VAULT_FILE);
    let old = std::fs::read(&path).unwrap();
    let failed = activate_managed_vault_with_hooks(
        directory.path(),
        SECRET,
        &mut rng,
        VAULT_VERSION_OWNER_OPERATIONS,
        |phase, staged| {
            if phase == AtomicWritePhase::TempSynced {
                std::fs::remove_file(staged).unwrap();
            }
        },
        sync_managed_directory,
    );
    assert!(failed.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), old);
    let missing = directory.path().join("missing-operations-directory");
    let failed = activate_managed_vault_with_hooks(
        directory.path(),
        SECRET,
        &mut rng,
        VAULT_VERSION_OWNER_OPERATIONS,
        |_, _| {},
        |_| sync_managed_directory(&missing),
    );
    assert!(matches!(
        failed,
        Err(StorageError::CommittedButNotDurable(_))
    ));
    assert_eq!(
        std::fs::read(path).unwrap()[0],
        VAULT_VERSION_OWNER_OPERATIONS
    );
    let failed_reopen = open_classified_with_hooks(
        directory.path(),
        SECRET,
        &mut rng,
        || {},
        || {},
        |_| {},
        |_| sync_managed_directory(&missing),
    );
    assert!(matches!(failed_reopen, Err(StorageError::Io(_))));
    assert_eq!(
        classified(directory.path(), &mut rng).0.db_key().unwrap(),
        old_key
    );
}

#[test]
fn operations_wrapper_version_gate_is_not_an_authentic_old_wrapper_rollback_anchor() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x05ca_1104);
    let key = classified(directory.path(), &mut rng).0.db_key().unwrap();
    activate_owner_evidence_vault(directory.path(), SECRET, &mut rng).unwrap();
    let path = directory.path().join(VAULT_FILE);
    let old_v4 = std::fs::read(&path).unwrap();
    activate_owner_operations_vault(directory.path(), SECRET, &mut rng).unwrap();
    // This is intentionally a documented limitation: the old authentic wrapper alone contains
    // the SAME DEK. Host intent protects a NEW reader; old binaries do not know that intent.
    std::fs::write(path, old_v4).unwrap();
    let (same, format) = classified(directory.path(), &mut rng);
    assert_eq!(format, VaultFormat::OwnerEvidence);
    assert_eq!(same.db_key().unwrap(), key);
}
