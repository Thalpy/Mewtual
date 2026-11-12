//! Actual low-level v6 wrapper/KDF barriers. The mandatory EMPTY catalog is enforced by App,
//! not this wrapper primitive. Authored tests remain UNRUN under the compilation pause.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"critical catalog wrapper fixture";
fn classified(dir: &Path, secret: &[u8], rng: &mut ChaCha20Rng) -> (KeyHierarchy, VaultFormat) {
    open_or_create_vault_classified(dir, secret, rng)
        .unwrap()
        .into_parts()
}

#[test]
fn critical_namespace_wrapper_preserves_dek_and_is_sticky_through_old_activations_and_secret_change(
) {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_1001);
    let (original, _) = classified(directory.path(), SECRET, &mut rng);
    let key = original.db_key().unwrap();
    activate_critical_namespace_vault(directory.path(), SECRET, &mut rng).unwrap();
    activate_chat_scope_vault(directory.path(), SECRET, &mut rng).unwrap();
    activate_owner_evidence_vault(directory.path(), SECRET, &mut rng).unwrap();
    activate_owner_operations_vault(directory.path(), SECRET, &mut rng).unwrap();
    let (same, format) = classified(directory.path(), SECRET, &mut rng);
    assert_eq!(format, VaultFormat::CriticalNamespaces);
    assert!(format.requires_owner_evidence());
    assert!(format.requires_owner_operations());
    assert!(chat_scope_vault_managed(directory.path()).unwrap());
    assert_eq!(same.db_key().unwrap(), key);
    change_vault_passphrase(directory.path(), SECRET, b"new critical secret", &mut rng).unwrap();
    let (same, format) = classified(directory.path(), b"new critical secret", &mut rng);
    assert_eq!(format, VaultFormat::CriticalNamespaces);
    assert_eq!(same.db_key().unwrap(), key);
    assert!(open_or_create_vault_classified(directory.path(), SECRET, &mut rng).is_err());
    assert_eq!(
        std::fs::read(directory.path().join(VAULT_FILE)).unwrap()[0],
        VAULT_VERSION_CRITICAL_NAMESPACES
    );
}

#[test]
fn critical_namespace_wrapper_public_header_cannot_be_changed_to_an_old_authenticated_version() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_1002);
    let _ = classified(directory.path(), SECRET, &mut rng);
    activate_critical_namespace_vault(directory.path(), SECRET, &mut rng).unwrap();
    let path = directory.path().join(VAULT_FILE);
    let original = std::fs::read(&path).unwrap();
    for version in [
        VAULT_VERSION_RAW_SECRET,
        VAULT_VERSION_PREHASHED_SECRET,
        VAULT_VERSION_CHAT_SCOPE,
        VAULT_VERSION_OWNER_EVIDENCE,
        VAULT_VERSION_OWNER_OPERATIONS,
    ] {
        let mut forged = original.clone();
        forged[0] = version;
        std::fs::write(&path, &forged).unwrap();
        assert!(open_or_create_vault_classified(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), forged);
    }
    std::fs::write(&path, &original).unwrap();
    assert_eq!(
        classified(directory.path(), SECRET, &mut rng).1,
        VaultFormat::CriticalNamespaces
    );
}

#[test]
fn critical_namespace_wrapper_actual_rename_and_parent_failures_require_real_reopen_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_1003);
    let key = classified(directory.path(), SECRET, &mut rng)
        .0
        .db_key()
        .unwrap();
    activate_owner_operations_vault(directory.path(), SECRET, &mut rng).unwrap();
    let path = directory.path().join(VAULT_FILE);
    let original = std::fs::read(&path).unwrap();
    let result = activate_managed_vault_with_hooks(
        directory.path(),
        SECRET,
        &mut rng,
        VAULT_VERSION_CRITICAL_NAMESPACES,
        |phase, staged| {
            if phase == AtomicWritePhase::TempSynced {
                std::fs::remove_file(staged).unwrap();
            }
        },
        sync_managed_directory,
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let missing = directory.path().join("missing-critical-wrapper-parent");
    let result = activate_managed_vault_with_hooks(
        directory.path(),
        SECRET,
        &mut rng,
        VAULT_VERSION_CRITICAL_NAMESPACES,
        |_, _| {},
        |_| sync_managed_directory(&missing),
    );
    assert!(matches!(
        result,
        Err(StorageError::CommittedButNotDurable(_))
    ));
    assert_eq!(
        std::fs::read(&path).unwrap()[0],
        VAULT_VERSION_CRITICAL_NAMESPACES
    );
    assert!(open_classified_with_hooks(
        directory.path(),
        SECRET,
        &mut rng,
        || {},
        || {},
        |_| {},
        |_| sync_managed_directory(&missing)
    )
    .is_err());
    assert_eq!(
        classified(directory.path(), SECRET, &mut rng)
            .0
            .db_key()
            .unwrap(),
        key
    );
}

#[test]
fn critical_namespace_version_is_not_an_authentic_old_wrapper_component_rollback_anchor() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca1_1004);
    let key = classified(directory.path(), SECRET, &mut rng)
        .0
        .db_key()
        .unwrap();
    activate_owner_operations_vault(directory.path(), SECRET, &mut rng).unwrap();
    let path = directory.path().join(VAULT_FILE);
    let authentic_old = std::fs::read(&path).unwrap();
    activate_critical_namespace_vault(directory.path(), SECRET, &mut rng).unwrap();
    // The new APP constructor also inspects catalog intent. An old binary ignores it and an
    // authentic old wrapper alone still contains the SAME DEK. Do not advertise antirollback.
    std::fs::write(&path, authentic_old).unwrap();
    let (same, format) = classified(directory.path(), SECRET, &mut rng);
    assert_eq!(format, VaultFormat::OwnerOperations);
    assert_eq!(same.db_key().unwrap(), key);
}
