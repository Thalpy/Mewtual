//! Critical owner format is sticky and cryptographically distinct from scope-only readers.
//! The low-level wrapper does not create the host's required immutable pin inventory.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"owner evidence critical vault";
fn classified(dir: &Path, rng: &mut ChaCha20Rng) -> (KeyHierarchy, VaultFormat) {
    open_or_create_vault_classified(dir, SECRET, rng)
        .unwrap()
        .into_parts()
}

#[test]
fn owner_format_upgrade_keeps_dek_scope_activation_and_secret_change_never_downgrade() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x04ca_1101);
    let (original, format) = classified(dir.path(), &mut rng);
    assert_eq!(format, VaultFormat::Legacy);
    let key = original.db_key().unwrap();
    activate_chat_scope_vault(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(classified(dir.path(), &mut rng).1, VaultFormat::ChatScope);
    activate_owner_evidence_vault(dir.path(), SECRET, &mut rng).unwrap();
    let (opened, format) = classified(dir.path(), &mut rng);
    assert_eq!(format, VaultFormat::OwnerEvidence);
    assert_eq!(opened.db_key().unwrap(), key);
    assert!(chat_scope_vault_managed(dir.path()).unwrap());
    activate_chat_scope_vault(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        classified(dir.path(), &mut rng).1,
        VaultFormat::OwnerEvidence
    );
    change_vault_passphrase(dir.path(), SECRET, b"changed owner vault secret", &mut rng).unwrap();
    let (changed, format) =
        open_or_create_vault_classified(dir.path(), b"changed owner vault secret", &mut rng)
            .unwrap()
            .into_parts();
    assert_eq!(format, VaultFormat::OwnerEvidence);
    assert_eq!(changed.db_key().unwrap(), key);
    assert!(open_or_create_vault(dir.path(), SECRET, &mut rng).is_err());
    assert_eq!(
        std::fs::read(dir.path().join(VAULT_FILE)).unwrap()[0],
        VAULT_VERSION_OWNER_EVIDENCE
    );
}

#[test]
fn owner_format_cannot_be_opened_by_flipping_header_to_any_old_kdf_domain() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x04ca_1102);
    let (original, _) = classified(dir.path(), &mut rng);
    let key = original.db_key().unwrap();
    activate_owner_evidence_vault(dir.path(), SECRET, &mut rng).unwrap();
    let file = dir.path().join(VAULT_FILE);
    let original = std::fs::read(&file).unwrap();
    for version in [
        VAULT_VERSION_RAW_SECRET,
        VAULT_VERSION_PREHASHED_SECRET,
        VAULT_VERSION_CHAT_SCOPE,
    ] {
        let mut forged = original.clone();
        forged[0] = version;
        std::fs::write(&file, &forged).unwrap();
        assert!(open_or_create_vault_classified(dir.path(), SECRET, &mut rng).is_err());
        assert_eq!(
            std::fs::read(&file).unwrap(),
            forged,
            "failed opens cannot rewrite the only wrapper"
        );
    }
    std::fs::write(&file, &original).unwrap();
    assert_eq!(classified(dir.path(), &mut rng).0.db_key().unwrap(), key);
}

#[test]
fn owner_format_actual_rename_failure_reopen_barrier_remains_required() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x04ca_1103);
    let _ = classified(dir.path(), &mut rng);
    activate_chat_scope_vault(dir.path(), SECRET, &mut rng).unwrap();
    let old = std::fs::read(dir.path().join(VAULT_FILE)).unwrap();
    let failed = activate_managed_vault_with_hooks(
        dir.path(),
        SECRET,
        &mut rng,
        VAULT_VERSION_OWNER_EVIDENCE,
        |phase, staged| {
            if phase == AtomicWritePhase::TempSynced {
                std::fs::remove_file(staged).unwrap();
            }
        },
        sync_managed_directory,
    );
    assert!(failed.is_err());
    assert_eq!(std::fs::read(dir.path().join(VAULT_FILE)).unwrap(), old);
    let missing = dir.path().join("proven-missing-parent");
    assert!(!missing.exists());
    let failed = activate_managed_vault_with_hooks(
        dir.path(),
        SECRET,
        &mut rng,
        VAULT_VERSION_OWNER_EVIDENCE,
        |_, _| {},
        |_| sync_managed_directory(&missing),
    );
    assert!(matches!(
        failed,
        Err(StorageError::CommittedButNotDurable(_))
    ));
    assert_eq!(
        std::fs::read(dir.path().join(VAULT_FILE)).unwrap()[0],
        VAULT_VERSION_OWNER_EVIDENCE
    );
    let reopened = open_classified_with_hooks(
        dir.path(),
        SECRET,
        &mut rng,
        || {},
        || {},
        |_| {},
        |_| sync_managed_directory(&missing),
    );
    assert!(
        matches!(reopened, Err(StorageError::Io(_))),
        "visible v4 bytes are not a sync proof"
    );
    assert_eq!(
        classified(dir.path(), &mut rng).1,
        VaultFormat::OwnerEvidence
    );
}
