//! Mandatory inventory validation happens before an actual managed Store can escape open.
//! Runtime no-tail restoration/metadata initialization are separate host gates, still unwired.
use super::*;
use catcoms_storage::VaultFormat;

#[test]
fn authenticated_managed_reopen_resolves_original_current_and_retired_lineage_without_reenable() {
    let (dir, store, mut rng) = mounted();
    let current_key = key(61);
    let current = issue(&store, current_key.clone(), &mut rng);
    let retired_key = key(62);
    let retired = issue(&store, retired_key.clone(), &mut rng);
    store.retire_chat_admission(&retired, &mut rng).unwrap();
    drop(store);
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::ChatScope
    );
    assert!(current.publication_authority().is_cancelled());
    let restored = match reopened
        .resolve_chat_admission(&current_key, current.nonce())
        .unwrap()
    {
        ChatAdmissionResolution::Current(binding) => binding,
        _ => panic!("the same saved admission remains Current"),
    };
    assert!(restored.is_live());
    assert!(!Arc::ptr_eq(
        &restored.owner_identity,
        &current.owner_identity
    ));
    assert!(
        reopened
            .open_chat_admission_inventory(VaultFormat::Legacy)
            .is_err(),
        "constructor-only load cannot reset a live owner's custody from disk"
    );
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::ChatScope
    );
    assert!(restored.is_live());
    assert!(matches!(
        reopened
            .resolve_chat_admission(&retired_key, retired.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
    let capture = reopened.capture_chat_admission(retired_key).unwrap();
    assert!(reopened
        .prepare_legacy_chat_admission(&capture, b"captured old no-tail snapshot", &mut rng)
        .is_err());
}

#[test]
fn legacy_empty_staging_is_inert_and_never_exposes_managed_issuance_before_activation() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x1e6a_c163);
    let store = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(store.chat_vault_format().unwrap(), VaultFormat::Legacy);
    let legacy_wrapper = fs::read(dir.path().join("vault.bin")).unwrap();
    store
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    drop(store);
    // This is exactly the staged empty inventory left before a first wrapper activation. It
    // contains no issued identity, unlike the nonempty legacy-rollback rejection regression.
    fs::write(dir.path().join("vault.bin"), legacy_wrapper).unwrap();
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(reopened.chat_vault_format().unwrap(), VaultFormat::Legacy);
    assert!(reopened.capture_chat_admission(key(63)).is_err());
    reopened
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    assert_eq!(
        reopened.chat_vault_format().unwrap(),
        VaultFormat::ChatScope
    );
    assert!(reopened.capture_chat_admission(key(63)).is_ok());
}

#[test]
fn managed_corrupt_inventory_refuses_before_any_runtime_resolution_or_context() {
    let (dir, store, mut rng) = mounted();
    let _current = issue(&store, key(64), &mut rng);
    drop(store);
    fs::write(
        dir.path().join(FILE),
        b"malformed retained sealed inventory",
    )
    .unwrap();
    assert!(ServerStore::open(dir.path(), SECRET, &mut rng).is_err());
}
