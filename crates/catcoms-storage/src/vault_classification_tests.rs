//! Classification comes from the exact authenticated wrapper, not a later public header read.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::cell::Cell;

const SECRET: &[u8] = b"classified vault fixture";

#[test]
fn classified_open_retains_authenticated_format_when_later_header_is_changed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc1a5_51f1);
    let original = open_or_create_vault_classified(dir.path(), SECRET, &mut rng).unwrap();
    assert_eq!(format!("{original:?}"), "OpenedVault { .. }");
    let (keys, format) = original.into_parts();
    assert_eq!(format, VaultFormat::Legacy);
    let expected_key = keys.db_key().unwrap();
    activate_chat_scope_vault(dir.path(), SECRET, &mut rng).unwrap();
    let opened = open_classified_with_hooks(
        dir.path(),
        SECRET,
        &mut rng,
        || {},
        || {},
        |path| {
            let file = path.join(VAULT_FILE);
            let mut bytes = std::fs::read(&file).unwrap();
            assert_eq!(bytes[0], VAULT_VERSION_CHAT_SCOPE);
            bytes[0] = VAULT_VERSION_RAW_SECRET;
            std::fs::write(file, bytes).unwrap();
        },
        sync_managed_directory,
    )
    .unwrap();
    let (keys, format) = opened.into_parts();
    assert_eq!(keys.db_key().unwrap(), expected_key);
    assert_eq!(format, VaultFormat::ChatScope);
    assert!(
        !chat_scope_vault_managed(dir.path()).unwrap(),
        "a later unauthenticated metadata read would now give the WRONG classification"
    );
    assert!(
        open_or_create_vault(dir.path(), SECRET, &mut rng).is_err(),
        "the KDF domain still prevents a plain 3-to-1 header downgrade"
    );
}

#[test]
fn classified_managed_open_requires_actual_directory_barrier_before_returning_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc1a5_51f2);
    let expected = open_or_create_vault(dir.path(), SECRET, &mut rng)
        .unwrap()
        .db_key()
        .unwrap();
    activate_chat_scope_vault(dir.path(), SECRET, &mut rng).unwrap();
    let authenticated = Cell::new(false);
    let synced = Cell::new(false);
    let missing = dir.path().join("proven-missing-directory");
    assert!(!missing.exists());
    let result = open_classified_with_hooks(
        dir.path(),
        SECRET,
        &mut rng,
        || {},
        || {},
        |_| authenticated.set(true),
        |path| {
            assert_eq!(path, dir.path());
            synced.set(true);
            // Exercise the production OS callback on a verified nonexistent directory, rather
            // than fabricating an error. No locked directory rename is required on Windows.
            sync_managed_directory(&missing)
        },
    );
    assert!(authenticated.get() && synced.get());
    assert!(matches!(result, Err(StorageError::Io(_))));
    let (keys, format) = open_or_create_vault_classified(dir.path(), SECRET, &mut rng)
        .unwrap()
        .into_parts();
    assert_eq!(format, VaultFormat::ChatScope);
    assert_eq!(keys.db_key().unwrap(), expected);
    let wrong = open_classified_with_hooks(
        dir.path(),
        b"wrong secret",
        &mut rng,
        || {},
        || {},
        |_| panic!("wrong secret must never reach authenticated classification"),
        |_| panic!("wrong secret must never reach a managed positive barrier"),
    );
    assert!(wrong.is_err());
}

#[test]
fn classified_legacy_long_wrapper_migrates_to_v2_without_becoming_managed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc1a5_51f3);
    let legacy_secret = vec![b'x'; MAX_VAULT_SECRET_BYTES + 1];
    let dek = Dek::generate(&mut rng);
    let bytes =
        seal_and_encode_version(&dek, &legacy_secret, &mut rng, VAULT_VERSION_RAW_SECRET).unwrap();
    let expected = KeyHierarchy::new(dek).db_key().unwrap();
    std::fs::write(dir.path().join(VAULT_FILE), bytes).unwrap();
    let (keys, format) = open_or_create_vault_classified(dir.path(), &legacy_secret, &mut rng)
        .unwrap()
        .into_parts();
    assert_eq!(format, VaultFormat::Legacy);
    assert_eq!(keys.db_key().unwrap(), expected);
    assert_eq!(
        std::fs::read(dir.path().join(VAULT_FILE)).unwrap()[0],
        VAULT_VERSION_PREHASHED_SECRET
    );
}
