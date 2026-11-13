//! Real private mounts and bounded DATA fixtures. No native authorization claimed.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, ServerStore,
    V2MemberOutboxPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::fs;

pub(in crate::store::critical_namespaces::native_registry) const SECRET: &[u8] =
    b"private native registry preparation";
pub(in crate::store::critical_namespaces::native_registry) struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("refusal reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("refusal reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("refusal reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("refusal reached RNG")
    }
}
impl CryptoRng for NeverRng {}
pub(in crate::store::critical_namespaces::native_registry) fn current_policy(
) -> V2CurrentLayoutPolicy {
    let shared = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(
            shared,
            4 * super::super::super::archive_bootstrap::MAX_SEED_FRAMED,
        )
        .unwrap()
        .with_bootstrap_source_data_read(),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * super::super::super::archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap()
    .with_member_outbox(
        V2MemberOutboxPolicy::new(
            CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap(),
            256,
            256 * 1024 * 1024,
            1 << 20,
            1,
            super::super::super::member_outbox_layout::codec::PERSISTENT_MINIMUM,
        )
        .unwrap(),
    )
    .unwrap()
}
fn native_policy() -> NativeRegistryPreparationPolicy {
    NativeRegistryPreparationPolicy::new(CriticalCatalogReadPool::new(PREPARATION_FEE).unwrap())
        .unwrap()
}
pub(in crate::store::critical_namespaces::native_registry) fn records() -> Vec<ServerRecord> {
    vec![ServerRecord {
        id: 9,
        display_name: "original legacy".into(),
        invite: "retained invite".into(),
        is_dm: true,
    }]
}
pub(in crate::store::critical_namespaces::native_registry) fn initialize(
    path: &Path,
    records: Option<&[ServerRecord]>,
    rng: &mut ChaCha20Rng,
) {
    let store = ServerStore::open(path, SECRET, rng).unwrap();
    if let Some(records) = records {
        store.save_registry(records, rng).unwrap();
    }
    drop(store.into_critical_namespace_store(SECRET, rng).unwrap());
}
fn prepared(path: &Path, rng: &mut ChaCha20Rng) -> PreparedNativeRegistryMigration {
    PreparedNativeRegistryMigration::open(path, SECRET, rng, current_policy(), native_policy())
        .unwrap()
}

#[test]
fn native_preparation_requires_complete_independent_payment_before_open_or_rng() {
    assert!(NativeRegistryPreparationPolicy::new(
        CriticalCatalogReadPool::new(PREPARATION_FEE - 1).unwrap()
    )
    .is_err());
    let policy = native_policy();
    let held = policy.pool.try_reserve(1).unwrap();
    let absent = tempfile::tempdir().unwrap();
    assert!(PreparedNativeRegistryMigration::open(
        absent.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy
    )
    .is_err());
    assert!(!absent.path().join("vault.bin").exists());
    drop(held);
    let current = current_policy();
    let alias = NativeRegistryPreparationPolicy {
        pool: current.archive.pending.pool.clone(),
    };
    assert!(PreparedNativeRegistryMigration::open(
        absent.path(),
        SECRET,
        &mut NeverRng,
        current,
        alias
    )
    .is_err());
    let mut current = current_policy();
    let member_pool = CriticalCatalogReadPool::new(PREPARATION_FEE).unwrap();
    current.member_outbox.as_mut().unwrap().pool = member_pool.clone();
    let alias = NativeRegistryPreparationPolicy::new(member_pool).unwrap();
    assert!(PreparedNativeRegistryMigration::open(
        absent.path(),
        SECRET,
        &mut NeverRng,
        current,
        alias
    )
    .is_err());
}

#[test]
fn native_preparation_real_mount_preserves_legacy_and_caches_without_declaring_seven() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7701);
    initialize(dir.path(), Some(&records()), &mut rng);
    let registry = fs::read(dir.path().join("registry.bin")).unwrap();
    let original = prepared(dir.path(), &mut rng);
    assert_eq!(original.legacy_row_count_data(), 1);
    assert_eq!(fs::read(dir.path().join("registry.bin")).unwrap(), registry);
    let catalog = fs::read(dir.path().join(super::super::super::Slot::Current.name())).unwrap();
    let candidates = [
        &original.output.candidate,
        &original.output.index,
        &original.output.marker,
        &original.output.seed,
    ]
    .map(|bytes| *blake3::hash(bytes).as_bytes());
    for _ in 0..3 {
        original.revalidate_inputs().unwrap();
    }
    assert_eq!(
        candidates,
        [
            &original.output.candidate,
            &original.output.index,
            &original.output.marker,
            &original.output.seed
        ]
        .map(|bytes| *blake3::hash(bytes).as_bytes())
    );
    assert_eq!(
        fs::read(dir.path().join(super::super::super::Slot::Current.name())).unwrap(),
        catalog
    );
    assert!(!dir.path().join(SEED_NAME).exists());
    for name in [MARKER_NAME, INDEX_NAME, INDEX_STAGE_NAME] {
        assert!(!dir
            .path()
            .join(super::super::super::ARCHIVE_BOOTSTRAP_ROOT)
            .join(name)
            .exists());
    }
    assert!(!format!("{original:?}").contains("original legacy"));
}

#[test]
fn native_preparation_strict_absence_does_not_adopt_a_later_registry() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7702);
    initialize(dir.path(), None, &mut rng);
    let original = prepared(dir.path(), &mut rng);
    assert_eq!(original.legacy_row_count_data(), 0);
    original
        .current
        .checked_store()
        .unwrap()
        .save_registry(&records(), &mut rng)
        .unwrap();
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_equal_bytes_replacement_is_not_the_original_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7703);
    initialize(dir.path(), Some(&records()), &mut rng);
    let original = prepared(dir.path(), &mut rng);
    let path = dir.path().join("registry.bin");
    let bytes = fs::read(&path).unwrap();
    fs::rename(&path, dir.path().join("held-original-registry.bin")).unwrap();
    fs::write(&path, bytes).unwrap();
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_equal_catalog_replacement_or_changed_input_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7704);
    initialize(dir.path(), Some(&records()), &mut rng);
    let original = prepared(dir.path(), &mut rng);
    let catalog = dir.path().join(super::super::super::Slot::Current.name());
    let bytes = fs::read(&catalog).unwrap();
    fs::rename(&catalog, dir.path().join("held-original-catalog.bin")).unwrap();
    fs::write(&catalog, bytes).unwrap();
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_revalidation_honors_original_mount_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7705);
    initialize(dir.path(), None, &mut rng);
    let original = prepared(dir.path(), &mut rng);
    original
        .current
        .checked_store()
        .unwrap()
        .chat_mount
        .revoke();
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_legacy_preflight_bounds_complete_input_before_copy() {
    let valid = crate::store::encode_registry(&records());
    assert_eq!(decode_bounded_legacy(&valid).unwrap(), records());
    let v1 = &valid[..valid.len() - 1];
    assert!(!decode_bounded_legacy(v1).unwrap()[0].is_dm);
    let mut old_noncanonical_flag = valid.clone();
    *old_noncanonical_flag.last_mut().unwrap() = 7;
    assert!(decode_bounded_legacy(&old_noncanonical_flag).unwrap()[0].is_dm);
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(decode_bounded_legacy(&trailing).is_err());
    assert!(decode_bounded_legacy(&(u32::MAX).to_be_bytes()).is_err());
    let mut duplicate = records();
    duplicate.push(duplicate[0].clone());
    assert!(decode_bounded_legacy(&crate::store::encode_registry(&duplicate)).is_err());
    let mut excess = records();
    excess[0].display_name = "x".repeat(MAX_NAME + 1);
    assert!(decode_bounded_legacy(&crate::store::encode_registry(&excess)).is_err());
}

#[test]
fn native_preparation_existing_native_artifacts_refuse_before_older_cleanup() {
    for name in [SEED_NAME, MARKER_NAME, INDEX_NAME, INDEX_STAGE_NAME] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7706);
        initialize(dir.path(), None, &mut rng);
        let path = if name == SEED_NAME {
            dir.path().join(name)
        } else {
            let root = dir.path().join(super::super::super::ARCHIVE_BOOTSTRAP_ROOT);
            fs::create_dir(&root).unwrap();
            root.join(name)
        };
        fs::write(&path, b"future format remains untouched").unwrap();
        assert!(ServerStore::open(dir.path(), SECRET, &mut rng).is_err());
        assert!(PreparedNativeRegistryMigration::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
            native_policy()
        )
        .is_err());
        assert_eq!(fs::read(path).unwrap(), b"future format remains untouched");
    }
}

#[test]
fn native_preparation_sealed_candidates_reject_role_swap_and_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7707);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut original = prepared(dir.path(), &mut rng);
    std::mem::swap(&mut original.output.marker, &mut original.output.index);
    assert!(original.revalidate_inputs().is_err());
    std::mem::swap(&mut original.output.marker, &mut original.output.index);
    original.output.seed[40] ^= 1;
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_in_place_legacy_byte_change_is_not_a_matching_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7708);
    initialize(dir.path(), Some(&records()), &mut rng);
    let original = prepared(dir.path(), &mut rng);
    let path = dir.path().join("registry.bin");
    let mut bytes = fs::read(&path).unwrap();
    bytes[40] ^= 1;
    // This deliberately preserves the inode/SessionId: byte comparison is also
    // required, independently of the replacement-file regressions above.
    fs::write(path, bytes).unwrap();
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_authenticated_seed_cannot_substitute_another_initial_index() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7709);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut original = prepared(dir.path(), &mut rng);
    let key = Zeroizing::new(
        original
            .current
            .checked_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    let plain = open_kind(&key, INDEX_KEY, &original.output.index, MAX_PLAIN).unwrap();
    let mut index = IndexData::decode_data(&plain, original.output.capability).unwrap();
    index.rows[0].name = "authenticated but wrong".into();
    original.output.index = seal_kind(
        &key,
        INDEX_KEY,
        &index.encode_data().unwrap(),
        MAX_PLAIN,
        &mut rng,
    )
    .unwrap();
    let seed = seed_plain(
        original.output.capability,
        original.inputs.catalog(),
        &original.output.candidate,
        &original.output.index,
        &original.output.marker,
        original.inputs.legacy(),
    )
    .unwrap();
    original.output.seed = seal_kind(&key, SEED_KEY, &seed, MAX_SEED_PLAIN, &mut rng).unwrap();
    assert!(original.revalidate_inputs().is_err());
}

#[test]
fn native_preparation_final_negative_after_sealing_refuses_and_releases_payment() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7710);
    initialize(dir.path(), Some(&records()), &mut rng);
    let registry = fs::read(dir.path().join("registry.bin")).unwrap();
    let policy = native_policy();
    let account = policy.pool.clone();
    CANCEL_BEFORE_RETURN.with(|cut| cut.set(true));
    assert!(PreparedNativeRegistryMigration::open(
        dir.path(),
        SECRET,
        &mut rng,
        current_policy(),
        policy
    )
    .is_err());
    assert!(
        !CANCEL_BEFORE_RETURN.with(std::cell::Cell::get),
        "the real final cut must fire"
    );
    assert_eq!(account.charged_for_test(), 0);
    assert_eq!(fs::read(dir.path().join("registry.bin")).unwrap(), registry);
    assert!(!dir.path().join(SEED_NAME).exists());
    let original = prepared(dir.path(), &mut rng);
    original.revalidate_inputs().unwrap();
}

#[test]
fn native_preparation_retains_full_original_payment_until_files_and_mount_drop() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7711);
    initialize(dir.path(), None, &mut rng);
    let policy = native_policy();
    let account = policy.pool.clone();
    let original = PreparedNativeRegistryMigration::open(
        dir.path(),
        SECRET,
        &mut rng,
        current_policy(),
        policy,
    )
    .unwrap();
    assert_eq!(account.charged_for_test(), PREPARATION_FEE);
    original.revalidate_inputs().unwrap();
    assert_eq!(account.charged_for_test(), PREPARATION_FEE);
    drop(original);
    assert_eq!(account.charged_for_test(), 0);
}
