//! Actual authenticated-row→paid-index construction. No returned boolean/synthetic table is
//! used as a group/Current capability. Compiled checks run only on the primary serial pipeline.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    OwnerEvidenceBudget, PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
const SECRET: &[u8] = b"paid negative filename index";
fn key(group: &[u8], signer: u8) -> ChatAdmissionKey {
    ChatAdmissionKey::new(group, [signer; 32]).unwrap()
}
fn old(path: &Path, rng: &mut ChaCha20Rng) -> ServerStore {
    ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, rng)
        .unwrap()
}
fn policy(pool: &CriticalCatalogReadPool, files: usize, file_bytes: u64) -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(ArchiveBootstrapCatalogPolicy::new(pool.clone(),4*crate::store::critical_namespaces::archive_bootstrap::MAX_SEED_FRAMED)
        .unwrap().with_bootstrap_source_data_read(),CanonicalFilenameInventoryPolicy::new(files,file_bytes).unwrap(),
        4*crate::store::critical_namespaces::archive_current_namespace_codec::MAX_DECLARATION_FRAME).unwrap()
}
fn ready(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    rng: &mut ChaCha20Rng,
    files: usize,
    bytes: u64,
) -> crate::store::V2CurrentLayoutStore {
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, rng, policy(pool, files, bytes)).unwrap();
    owner.retry(rng).unwrap();
    owner.into_current_layout_store().unwrap()
}

#[test]
fn known_unrelated_current_and_stage_names_are_attributed_without_head_cells_or_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3a01);
    let old = old(directory.path(), &mut rng);
    let a = old.capture_chat_admission(key(b"a", 1)).unwrap();
    let a = old
        .prepare_legacy_chat_admission(&a, b"real a", &mut rng)
        .unwrap();
    let a = old
        .finish_chat_admission(&a, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    let b = old.capture_chat_admission(key(b"b", 2)).unwrap();
    let b = old
        .prepare_legacy_chat_admission(&b, b"real b", &mut rng)
        .unwrap();
    let parent = directory.path().join("owner-state");
    fs::create_dir(&parent).unwrap();
    let name = crate::store::owner_evidence::canonical::key(b"b", &[2; 32]);
    let stage = parent.join(format!("{}.replace.tmp", hex::encode(name)));
    fs::write(&stage, b"retained original b source").unwrap();
    fs::write(
        parent.join(format!("{}.bin", hex::encode(name))),
        b"retained current b",
    )
    .unwrap();
    drop(old);
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let ready = ready(directory.path(), &pool, &mut rng, 8, 1024);
    let store = ready.checked_store().unwrap();
    let runtime = store.chat_mount.locked().unwrap();
    let index = runtime.current_lineage.as_ref().unwrap();
    assert_eq!(index.keys.len(), 2);
    assert!(index.contains_key_data(&name));
    index.require_complete(store).unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::head_count_for_test(store),
        0,
        "readonly proof must not configure checkpoint inventory"
    );
    drop(runtime);
    assert_eq!(fs::read(stage).unwrap(), b"retained original b source");
    assert!(matches!(
        store
            .resolve_chat_admission(&key(b"a", 1), a.nonce())
            .unwrap(),
        crate::store::ChatAdmissionResolution::Current(_)
    ));
    assert!(matches!(
        store
            .resolve_chat_admission(&key(b"b", 2), b.nonce())
            .unwrap(),
        crate::store::ChatAdmissionResolution::Initializing(_)
    ));
}

#[test]
fn unknown_or_over_cap_filename_closes_only_new_vacancy_not_existing_other_group_control() {
    for shape in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x3a10 + shape);
        let old = old(directory.path(), &mut rng);
        let capture = old.capture_chat_admission(key(b"existing", 3)).unwrap();
        let scope = old
            .prepare_legacy_chat_admission(&capture, b"existing", &mut rng)
            .unwrap();
        let binding = old
            .finish_chat_admission(&scope, &mut rng, |_, _| Ok(()))
            .unwrap()
            .into_binding();
        let parent = directory.path().join("owner-state");
        fs::create_dir(&parent).unwrap();
        let name = match shape {
            0 => format!("{}.bin", hex::encode([0x55; 32])),
            1 => "unsupported-stage".into(),
            _ => format!(
                "{}.replace.tmp",
                hex::encode(crate::store::owner_evidence::canonical::key(
                    b"existing",
                    &[3; 32]
                ))
            ),
        };
        let path = parent.join(name);
        fs::write(&path, [1; 17]).unwrap();
        drop(old);
        let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
        let ready = ready(directory.path(), &pool, &mut rng, 8, 16);
        let store = ready.checked_store().unwrap();
        let runtime = store.chat_mount.locked().unwrap();
        assert!(runtime
            .current_lineage
            .as_ref()
            .unwrap()
            .require_complete(store)
            .is_err());
        drop(runtime);
        let crate::store::ChatAdmissionResolution::Current(current) = store
            .resolve_chat_admission(&key(b"existing", 3), binding.nonce())
            .unwrap()
        else {
            panic!("same old current retained");
        };
        store
            .with_chat_admission_storage(&current, |_| Ok(()))
            .unwrap();
        store.retire_chat_admission(&current, &mut rng).unwrap();
        assert_eq!(fs::read(path).unwrap(), [1; 17]);
    }
}

#[test]
fn new_actual_row_is_indexed_before_its_real_canonical_stage_and_known_directory_effect() {
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3a21);
    drop(old(directory.path(), &mut rng));
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let ready = ready(
        directory.path(),
        &pool,
        &mut rng,
        8,
        144 * 1024 * 1024 + 1024 + 40,
    );
    let store = ready.checked_store().unwrap();
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
    let mut attempt = store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(0x3a21)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x3a21),
            Box::new(ManualClock::new(1)),
            "actual tracked v1 source",
            9,
        )
        .unwrap();
    let name = crate::store::owner_evidence::canonical::key(
        attempt.group().unwrap(),
        attempt.founder().unwrap(),
    );
    store
        .owner_evidence
        .canonical_fault
        .store(3, std::sync::atomic::Ordering::SeqCst);
    assert!(
        attempt.retry(store, &mut rng).is_err(),
        "actual TempSynced stop retains known stage"
    );
    assert!(directory
        .path()
        .join("owner-state")
        .join(format!("{}.replace.tmp", hex::encode(name)))
        .is_file());
    let runtime = store.chat_mount.locked().unwrap();
    let index = runtime.current_lineage.as_ref().unwrap();
    assert!(index.contains_key_data(&name));
    assert!(index.generation_data() > 0);
    index.require_complete(store).unwrap();
}

#[test]
fn loaded_table_replacement_and_overflow_never_refresh_the_same_index_to_empty() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3a31);
    drop(old(directory.path(), &mut rng));
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let ready = ready(directory.path(), &pool, &mut rng, 8, 1024);
    let store = ready.checked_store().unwrap();
    let mut runtime = store.chat_mount.locked().unwrap();
    let index = runtime.current_lineage.as_mut().unwrap();
    let identity = index.identity_data().clone();
    index.generation.set(u64::MAX);
    index.observe_new_key(&key(b"bounded new key", 4));
    assert!(index.require_complete(store).is_err());
    assert!(Arc::ptr_eq(&identity, index.identity_data()));
    runtime.invalidate_current_lineage();
    runtime.table = Some(BTreeMap::new());
    assert!(runtime
        .current_lineage
        .as_ref()
        .unwrap()
        .require_complete(store)
        .is_err());
}

#[test]
fn borrowed_attribution_rechecks_original_directory_and_never_adopts_replacement_or_late_presence()
{
    for initially_present in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x3a41 + u64::from(initially_present));
        drop(old(directory.path(), &mut rng));
        let parent = directory.path().join("owner-state");
        if initially_present {
            fs::create_dir(&parent).unwrap();
        }
        let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
        let ready = ready(directory.path(), &pool, &mut rng, 8, 1024);
        let store = ready.checked_store().unwrap();
        let runtime = store.chat_mount.locked().unwrap();
        let association = runtime.current_filename_association(store).unwrap();
        association.validate_original_directory().unwrap();
        // These are intentionally negative external mutations. Real app writers must use
        // actual known-key hooks; a cached generation cannot attest a new directory object.
        if initially_present {
            fs::rename(&parent, directory.path().join("old-owner-state-held")).unwrap();
        }
        fs::create_dir(&parent).unwrap();
        association.require_original_data().unwrap();
        assert!(association.validate_original_directory().is_err());
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    }
}

#[test]
fn borrowed_attribution_allows_same_dirty_table_negatively_but_refuses_untracked_rows() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3a43);
    let old = old(directory.path(), &mut rng);
    let captured = old.capture_chat_admission(key(b"original", 1)).unwrap();
    old.prepare_legacy_chat_admission(&captured, b"same original", &mut rng)
        .unwrap();
    drop(old);
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let ready = ready(directory.path(), &pool, &mut rng, 8, 1024);
    let store = ready.checked_store().unwrap();
    let mut runtime = store.chat_mount.locked().unwrap();
    runtime.dirty = true; // negative comparison of SAME table; no durability/Current granted
    runtime
        .current_filename_association(store)
        .unwrap()
        .validate_original_directory()
        .unwrap();
    let row = runtime
        .table
        .as_ref()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    runtime
        .table
        .as_mut()
        .unwrap()
        .insert(key(b"untracked", 2), row);
    assert!(runtime.current_filename_association(store).is_err());
}
