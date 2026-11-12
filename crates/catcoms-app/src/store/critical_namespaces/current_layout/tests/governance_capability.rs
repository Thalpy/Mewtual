//! Actual encrypted catalog/opening refusal, not a fabricated Source or Saved4.
use super::*;
use crate::store::critical_namespaces::governance_namespace_codec as governance;

#[test]
fn real_current_three_opener_preserves_unsupported_four_in_either_or_both_sources() {
    for mode in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x4400 + mode);
        initialize(directory.path(), &mut rng);
        let mut opening = open(directory.path(), &shared, &mut rng);
        opening.retry(&mut rng).unwrap();
        let ready = opening.into_current_layout_store().unwrap();
        let store = ready.checked_store().unwrap();
        let current_path = directory.path().join(Slot::Current.name());
        let stage_path = directory.path().join(Slot::Stage.name());
        assert!(!stage_path.exists());
        let original = fs::read(&current_path).unwrap();
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let old_plain = Zeroizing::new(unseal(&key, &unframe(&original).unwrap()).unwrap());
        let domain = store.chat_vault_domain().unwrap();
        assert!(store.parse_configured_join_catalog(&old_plain).is_ok());
        let candidate = governance::prepare_declaration_data(&old_plain, &domain).unwrap();
        governance::validate_declaration_pair_data(&old_plain, &candidate, &domain).unwrap();
        let first = frame(&seal(&key, &candidate, &mut rng).unwrap());
        let second = frame(&seal(&key, &candidate, &mut rng).unwrap());
        assert_ne!(
            first, second,
            "equal plaintext is not exact ciphertext identity"
        );
        // These are deliberately injected NEGATIVE fixtures. No production4 writer,
        // Current constructor or effect receipt is being tested or manufactured.
        match mode {
            0 => fs::write(&current_path, &first).unwrap(),
            1 => fs::write(&stage_path, &first).unwrap(),
            2 => {
                fs::write(&current_path, &first).unwrap();
                fs::write(&stage_path, &second).unwrap();
            }
            3 => {
                fs::remove_file(&current_path).unwrap();
                fs::write(&stage_path, &first).unwrap();
            }
            _ => unreachable!(),
        }
        let before_current = fs::read(&current_path).ok();
        let before_stage = fs::read(&stage_path).ok();
        drop(ready);
        assert!(
            PreparedV2CurrentLayoutStore::open(
                directory.path(),
                SECRET,
                &mut NeverRng,
                policy(&shared),
            )
            .is_err(),
            "unsupported4 must refuse before settlement or older cleanup"
        );
        assert_eq!(fs::read(&current_path).ok(), before_current);
        assert_eq!(fs::read(&stage_path).ok(), before_stage);
        assert!(!directory.path().join(CURRENT_DECLARATION_SEED).exists());
        assert!(!directory.path().join(ARCHIVE_DECLARATION_SEED).exists());
    }
}
