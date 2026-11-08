//! Genuine fixed-file Current3 -> private Current4 migration. These tests issue no
//! Source4, saved request, SDK operation or member/signing authority.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

mod review_regressions;

const SECRET: &[u8] = b"actual private governance metadata mount";
fn pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap()
}
fn current_policy(pool: &CriticalCatalogReadPool) -> current_layout::V2CurrentLayoutPolicy {
    current_layout::V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * archive_bootstrap::MAX_SEED_FRAMED)
            .unwrap()
            .with_bootstrap_source_data_read(),
        CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap()
}
fn policy(pool: &CriticalCatalogReadPool) -> V2GovernanceLayoutPolicy {
    V2GovernanceLayoutPolicy::new(current_policy(pool), codec::PERSISTENT_MINIMUM).unwrap()
}
fn settled_three(path: &Path, pool: &CriticalCatalogReadPool, rng: &mut ChaCha20Rng) {
    drop(
        ServerStore::open(path, SECRET, rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, rng)
            .unwrap(),
    );
    let mut opening =
        current_layout::PreparedV2CurrentLayoutStore::open(path, SECRET, rng, current_policy(pool))
            .unwrap();
    opening.retry(rng).unwrap();
    // The old private mount and ALL its writers are gone before the new mount.
    drop(opening.into_current_layout_store().unwrap());
}
fn open(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    rng: &mut impl CryptoRngCore,
) -> PreparedV2GovernanceLayoutStore {
    PreparedV2GovernanceLayoutStore::open(path, SECRET, rng, policy(pool)).unwrap()
}
fn private(owner: &PreparedV2GovernanceLayoutStore) -> &ServerStore {
    // Child-test access only; production intentionally has no Store conversion.
    owner.store.as_deref().unwrap()
}
fn fault(owner: &PreparedV2GovernanceLayoutStore, point: u8) {
    private(owner)
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}
fn files(path: &Path) -> BTreeMap<&'static str, Option<Vec<u8>>> {
    [
        Slot::Current.name(),
        Slot::Stage.name(),
        ARCHIVE_DECLARATION_SEED,
        current_group::DECLARATION_SEED,
        GOVERNANCE_SEED,
        "chat-owner-v2/governance-layout.bin",
        "chat-owner-v2/current-layout.bin",
        "chat-owner-v2/root.bin",
    ]
    .into_iter()
    .map(|name| (name, fs::read(path.join(name)).ok()))
    .collect()
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("unexpected RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("unexpected RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("unexpected RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("unexpected RNG")
    }
}
impl CryptoRng for NeverRng {}

struct ResetHooks(std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for ResetHooks {
    fn drop(&mut self) {
        AFTER_MARKER_VERIFICATION.with(|slot| {
            slot.borrow_mut().take();
        });
        BEFORE_STAGE_SELECTION.with(|slot| {
            slot.borrow_mut().take();
        });
        BEFORE_MARKER_OPEN.with(|slot| {
            slot.borrow_mut().take();
        });
        AT_DECLARATION_TAIL_DROP.with(|slot| {
            slot.borrow_mut().take();
        });
        AFTER_DECLARATION_CAPTURE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[test]
fn private_governance_mount_declares_once_and_old_openers_refuse_before_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x4401);
    settled_three(directory.path(), &shared, &mut rng);
    let before = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let mut owner = open(directory.path(), &shared, &mut rng);
    assert!(!owner.complete);
    owner = owner.into_governance_layout_store().unwrap_err();
    owner.retry(&mut rng).unwrap();
    let declaration_cap = private(&owner)
        .governance_layout
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .capability;
    let current = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    assert_ne!(before, current);
    let key = private(&owner).keys.db_key().unwrap();
    let old_plain = unseal(&key, &unframe(&before).unwrap()).unwrap();
    let new_plain = unseal(&key, &unframe(&current).unwrap()).unwrap();
    governance_namespace_codec::validate_declaration_pair_data(
        &old_plain,
        &new_plain,
        &private(&owner).chat_vault_domain().unwrap(),
    )
    .unwrap();
    assert_eq!(
        private(&owner)
            .parse_configured_join_catalog(&new_plain)
            .unwrap()
            .governance,
        Some(declaration_cap),
    );
    let snapshot = files(directory.path());
    owner.retry(&mut NeverRng).unwrap();
    assert_eq!(files(directory.path()), snapshot);
    let ready = owner.into_governance_layout_store().unwrap();
    assert!(ready
        .read_saved_bootstrap_source_data(b"not an actual source", [1; 32])
        .is_err());
    drop(ready);
    assert!(current_layout::PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        current_policy(&shared),
    )
    .is_err());
    assert_eq!(files(directory.path()), snapshot);
    assert!(ServerStore::open(directory.path(), SECRET, &mut NeverRng).is_err());
    assert_eq!(files(directory.path()), snapshot);
    let mut reopened = open(directory.path(), &shared, &mut NeverRng);
    reopened.retry(&mut NeverRng).unwrap();
    drop(reopened.into_governance_layout_store().unwrap());
    assert_eq!(files(directory.path()), snapshot);
}

#[test]
fn same_warm_owner_recovers_its_exact_first_seed_prefix_without_resealing() {
    for point in [230, 231, 233, 235, 237, 239] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x4410 + u64::from(point));
        settled_three(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err(), "cut {point}");
        let declaration = owner.declaration.as_ref().unwrap();
        let candidate = declaration.candidate.to_vec();
        let marker = declaration.marker.to_vec();
        let seed = declaration.seed.to_vec();
        if let Ok(raw) = fs::read(directory.path().join(GOVERNANCE_SEED)) {
            assert_eq!(raw, seed[..raw.len()], "cut {point}");
        }
        fault(&owner, 0);
        owner.retry(&mut NeverRng).unwrap();
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            candidate
        );
        assert_eq!(
            fs::read(
                directory
                    .path()
                    .join(ARCHIVE_BOOTSTRAP_ROOT)
                    .join(GOVERNANCE_MARKER)
            )
            .unwrap(),
            marker
        );
        assert!(!directory.path().join(GOVERNANCE_SEED).exists());
        assert!(!directory.path().join(Slot::Stage.name()).exists());
    }
}

#[test]
fn cold_complete_seed_retains_both_actual_catalogs_and_repairs_every_later_cut() {
    for point in [232, 233, 234, 235, 236, 237, 238, 239] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x4430 + u64::from(point));
        settled_three(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err(), "cut {point}");
        let d = owner.declaration.as_ref().unwrap();
        let candidate = d.candidate.to_vec();
        let marker = d.marker.to_vec();
        let key = private(&owner).keys.db_key().unwrap();
        let seed_plain = codec::open_seed(&key, &d.seed).unwrap();
        let saved = codec::parse_seed(
            &key,
            &private(&owner).chat_vault_domain().unwrap(),
            &seed_plain,
        )
        .unwrap();
        assert_eq!(saved.original, d.original.as_slice());
        assert_eq!(saved.candidate, candidate);
        drop(owner);
        let mut cold = open(directory.path(), &shared, &mut NeverRng);
        cold.retry(&mut NeverRng).unwrap();
        assert_eq!(
            fs::read(directory.path().join(Slot::Current.name())).unwrap(),
            candidate
        );
        assert_eq!(
            fs::read(
                directory
                    .path()
                    .join(ARCHIVE_BOOTSTRAP_ROOT)
                    .join(GOVERNANCE_MARKER)
            )
            .unwrap(),
            marker
        );
        assert!(!directory.path().join(GOVERNANCE_SEED).exists());
    }
}

#[test]
fn cold_torn_first_seed_is_held_preserved_until_a_future_birth_recovery_role_exists() {
    // This deliberately pins REMAINING automatic-recovery work. Unknown partial
    // evidence cannot be silently deleted/resealed merely to make startup proceed.
    for point in [230, 231] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x4460 + u64::from(point));
        settled_three(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, point);
        assert!(owner.retry(&mut rng).is_err());
        drop(owner);
        let before = files(directory.path());
        assert!(PreparedV2GovernanceLayoutStore::open(
            directory.path(),
            SECRET,
            &mut NeverRng,
            policy(&shared),
        )
        .is_err());
        assert_eq!(files(directory.path()), before);
        assert!(current_layout::PreparedV2CurrentLayoutStore::open(
            directory.path(),
            SECRET,
            &mut NeverRng,
            current_policy(&shared),
        )
        .is_err());
        assert_eq!(files(directory.path()), before);
    }
}

#[test]
fn promoted_catalog_never_initializes_missing_or_partial_governance_marker() {
    for partial in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x4480 + u64::from(partial));
        settled_three(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, 237);
        assert!(owner.retry(&mut rng).is_err());
        let marker_path = directory
            .path()
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(GOVERNANCE_MARKER);
        let marker = fs::read(&marker_path).unwrap();
        drop(owner);
        fs::remove_file(&marker_path).unwrap();
        if partial {
            fs::write(&marker_path, &marker[..marker.len() / 2]).unwrap();
        }
        let before = files(directory.path());
        assert!(PreparedV2GovernanceLayoutStore::open(
            directory.path(),
            SECRET,
            &mut NeverRng,
            policy(&shared),
        )
        .is_err());
        assert_eq!(files(directory.path()), before);
        assert!(directory.path().join(GOVERNANCE_SEED).exists());
    }
}

#[test]
fn conflicting_old_declaration_debts_are_not_normalized_before_governance_inventory() {
    for old in [ARCHIVE_DECLARATION_SEED, current_group::DECLARATION_SEED] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x4490);
        settled_three(directory.path(), &shared, &mut rng);
        fs::write(
            directory.path().join(old),
            b"foreign old declaration evidence",
        )
        .unwrap();
        fs::write(
            directory.path().join(GOVERNANCE_SEED),
            b"foreign new declaration evidence",
        )
        .unwrap();
        let before = files(directory.path());
        assert!(PreparedV2GovernanceLayoutStore::open(
            directory.path(),
            SECRET,
            &mut NeverRng,
            policy(&shared),
        )
        .is_err());
        assert_eq!(files(directory.path()), before);
    }
}

#[test]
fn equal_bytes_in_replaced_original_current_or_marker_are_not_adopted() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44a0);
    settled_three(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    fault(&owner, 232);
    assert!(owner.retry(&mut rng).is_err());
    let current = directory.path().join(Slot::Current.name());
    let bytes = fs::read(&current).unwrap();
    fs::rename(&current, current.with_extension("original")).unwrap();
    fs::write(&current, bytes).unwrap();
    fault(&owner, 0);
    let before = files(directory.path());
    assert!(owner.retry(&mut NeverRng).is_err());
    assert_eq!(files(directory.path()), before);
    drop(owner);

    // A separate genuine completed mount exercises replacement AFTER the first
    // probe and BEFORE older inspectors/account waits, rather than merely tamper
    // before opening. The callback provides no positive fixture authority.
    let second = tempfile::tempdir().unwrap();
    settled_three(second.path(), &shared, &mut rng);
    let mut owner = open(second.path(), &shared, &mut rng);
    owner.retry(&mut rng).unwrap();
    drop(owner);
    let marker = second
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(GOVERNANCE_MARKER);
    let bytes = fs::read(&marker).unwrap();
    struct Reset(std::marker::PhantomData<std::rc::Rc<()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_MARKER_CAPTURE.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    let _reset = Reset(std::marker::PhantomData);
    AFTER_MARKER_CAPTURE.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            fs::rename(&marker, marker.with_extension("original")).unwrap();
            fs::write(&marker, bytes).unwrap();
        }));
    });
    assert!(PreparedV2GovernanceLayoutStore::open(
        second.path(),
        SECRET,
        &mut NeverRng,
        policy(&shared),
    )
    .is_err());
    assert!(AFTER_MARKER_CAPTURE.with(|slot| slot.borrow().is_none()));
}

#[test]
fn governance_persistent_capacity_and_new_phase_payment_precede_any_declaration_output() {
    let shared = pool();
    assert!(
        V2GovernanceLayoutPolicy::new(current_policy(&shared), codec::PERSISTENT_MINIMUM - 1)
            .is_err()
    );
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44b0);
    settled_three(directory.path(), &shared, &mut rng);
    let small = CriticalCatalogReadPool::new(codec::DECLARATION_FEE - 1).unwrap();
    let before = files(directory.path());
    assert!(PreparedV2GovernanceLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        policy(&small),
    )
    .is_err());
    assert_eq!(files(directory.path()), before);
}

#[test]
fn same_native_same_length_marker_change_during_old_proof_is_refused_at_final_tail() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44d0);
    settled_three(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    owner.retry(&mut rng).unwrap();
    let marker = directory
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(GOVERNANCE_MARKER);
    let original = fs::read(&marker).unwrap();
    let id = catcoms_storage::kept::session_id(&File::open(&marker).unwrap()).unwrap();
    let path = marker.clone();
    let mut bad = original.clone();
    bad[24] ^= 1;
    let _reset = ResetHooks(std::marker::PhantomData);
    AFTER_MARKER_VERIFICATION.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || fs::write(path, bad).unwrap()));
    });
    assert!(private(&owner)
        .validate_mounted_governance_layout()
        .is_err());
    assert_eq!(
        catcoms_storage::kept::session_id(&File::open(&marker).unwrap()).unwrap(),
        id
    );
    assert_eq!(fs::metadata(&marker).unwrap().len(), original.len() as u64);
    fs::write(&marker, original).unwrap();
    private(&owner)
        .validate_mounted_governance_layout()
        .unwrap();
}

#[test]
fn same_native_late_stage_change_is_preserved_and_never_selected() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44d1);
    settled_three(directory.path(), &shared, &mut rng);
    let old_current = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let mut owner = open(directory.path(), &shared, &mut rng);
    let stage = directory.path().join(Slot::Stage.name());
    let path = stage.clone();
    let _reset = ResetHooks(std::marker::PhantomData);
    BEFORE_STAGE_SELECTION.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            let original_id =
                catcoms_storage::kept::session_id(&File::open(&path).unwrap()).unwrap();
            let mut bad = fs::read(&path).unwrap();
            bad[24] ^= 1;
            fs::write(&path, &bad).unwrap();
            assert_eq!(
                catcoms_storage::kept::session_id(&File::open(&path).unwrap()).unwrap(),
                original_id
            );
        }));
    });
    assert!(owner.retry(&mut rng).is_err());
    let declaration = owner.declaration.as_ref().unwrap();
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        old_current
    );
    assert_eq!(
        fs::read(directory.path().join(GOVERNANCE_SEED)).unwrap(),
        declaration.seed.as_slice()
    );
    assert_ne!(fs::read(&stage).unwrap(), declaration.candidate.as_slice());
    fs::write(&stage, declaration.candidate.as_slice()).unwrap();
    owner.retry(&mut NeverRng).unwrap();
    assert!(!directory.path().join(GOVERNANCE_SEED).exists());
}

#[cfg(any(windows, unix))]
#[test]
fn intermediate_root_link_refuses_before_external_governance_marker_is_opened() {
    let directory = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44d2);
    settled_three(directory.path(), &shared, &mut rng);
    let root = directory.path().join(ARCHIVE_BOOTSTRAP_ROOT);
    fs::rename(&root, root.with_extension("original")).unwrap();
    // This sentinel remains outside the original vault. The actual leaf-open
    // seam also proves the earlier intermediate real/no-reparse refusal.
    fs::write(
        external.path().join(GOVERNANCE_MARKER),
        b"outside marker must remain untouched",
    )
    .unwrap();
    let _reset = ResetHooks(std::marker::PhantomData);
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = Arc::clone(&reached);
    BEFORE_MARKER_OPEN.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            observed.store(true, Ordering::SeqCst);
        }));
    });
    #[cfg(unix)]
    std::os::unix::fs::symlink(external.path(), &root).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if std::os::windows::fs::symlink_dir(external.path(), &root).is_err() {
            assert!(std::process::Command::new("cmd")
                .creation_flags(0x08000000)
                .args(["/C", "mklink", "/J"])
                .arg(&root)
                .arg(external.path())
                .output()
                .unwrap()
                .status
                .success());
        }
    }
    assert!(PreparedV2GovernanceLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        policy(&shared),
    )
    .is_err());
    assert!(!reached.load(Ordering::SeqCst));
    assert_eq!(
        fs::read(external.path().join(GOVERNANCE_MARKER)).unwrap(),
        b"outside marker must remain untouched"
    );
    assert!(fs::symlink_metadata(&root).is_ok());
    // Remove only the link itself, before either real directory's TempDir drops.
    #[cfg(unix)]
    fs::remove_file(&root).unwrap();
    #[cfg(windows)]
    fs::remove_dir(&root).unwrap();
}

#[test]
fn partial_declaration_files_and_data_drain_before_the_actual_vault_session_releases() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44d3);
    settled_three(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    fault(&owner, 232);
    assert!(owner.retry(&mut rng).is_err());
    assert!(owner.declaration.is_some());
    let path = directory.path().to_path_buf();
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached = Arc::clone(&observed);
    let _reset = ResetHooks(std::marker::PhantomData);
    AT_DECLARATION_TAIL_DROP.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            assert!(matches!(
                ServerStore::open(path, SECRET, &mut NeverRng),
                Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
            ));
            reached.store(true, Ordering::SeqCst);
        }));
    });
    drop(owner);
    assert!(observed.load(Ordering::SeqCst));
    // After the whole original owner is gone, the genuine complete source can
    // open and recover under a new actual session, without a synthetic receipt.
    let mut cold = open(directory.path(), &shared, &mut NeverRng);
    cold.retry(&mut NeverRng).unwrap();
}

mod join;
mod saved_source;
