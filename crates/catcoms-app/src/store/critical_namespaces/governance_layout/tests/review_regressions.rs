//! Independent-review recovery and predecessor-refusal regression fixtures.
use super::*;

/// Fixture-only enumeration, including vault/key files and directory names.
/// Hashes keep failure diagnostics compact and avoid printing encrypted frames.
fn complete_fixture_inventory(base: &Path) -> BTreeMap<PathBuf, Option<(u64, blake3::Hash)>> {
    fn visit(
        base: &Path,
        path: &Path,
        result: &mut BTreeMap<PathBuf, Option<(u64, blake3::Hash)>>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            let key = path.strip_prefix(base).unwrap().to_path_buf();
            if metadata.is_dir() {
                result.insert(key, None);
                visit(base, &path, result);
            } else {
                assert!(metadata.is_file());
                result.insert(
                    key,
                    Some((metadata.len(), blake3::hash(&fs::read(path).unwrap()))),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(base, base, &mut result);
    result
}

fn promoted(
    directory: &Path,
    shared: &CriticalCatalogReadPool,
    rng: &mut ChaCha20Rng,
) -> PreparedV2GovernanceLayoutStore {
    settled_three(directory, shared, rng);
    let mut owner = open(directory, shared, rng);
    fault(&owner, 237);
    assert!(owner.retry(rng).is_err());
    assert!(!directory.join(Slot::Stage.name()).exists());
    owner
}

#[test]
fn promoted_retry_preserves_equal_bytes_foreign_stage() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44f2);
    let mut owner = promoted(directory.path(), &shared, &mut rng);
    let candidate = owner.declaration.as_ref().unwrap().candidate.to_vec();
    fs::write(directory.path().join(Slot::Stage.name()), &candidate).unwrap();
    let before = files(directory.path());
    fault(&owner, 0);
    assert!(owner.retry(&mut NeverRng).is_err());
    assert_eq!(files(directory.path()), before);
    // Refusal must not release the live original physical session.
    assert!(catcoms_storage::acquire_vault_session(directory.path()).is_err());
    drop(owner);
    drop(catcoms_storage::acquire_vault_session(directory.path()).unwrap());
}

#[test]
fn cold_promoted_cleanup_refuses_replaced_captured_stage() {
    let _reset = ResetHooks(std::marker::PhantomData);
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44f3);
    let owner = promoted(directory.path(), &shared, &mut rng);
    let candidate = owner.declaration.as_ref().unwrap().candidate.to_vec();
    drop(owner);
    let stage = directory.path().join(Slot::Stage.name());
    fs::write(&stage, &candidate).unwrap();
    let before = files(directory.path());
    let retained = directory.path().join("captured-stage-test-only");
    AFTER_DECLARATION_CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            // Keep the originally captured native object alive at another name,
            // while replacing the fixed name with exactly matching ciphertext.
            fs::rename(&stage, &retained).unwrap();
            fs::write(&stage, &candidate).unwrap();
        }));
    });
    assert!(PreparedV2GovernanceLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        policy(&shared),
    )
    .is_err());
    assert_eq!(files(directory.path()), before);
    drop(catcoms_storage::acquire_vault_session(directory.path()).unwrap());
}

#[test]
fn cold_promoted_cleanup_retires_only_its_actual_captured_duplicate() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44f4);
    let owner = promoted(directory.path(), &shared, &mut rng);
    let candidate = owner.declaration.as_ref().unwrap().candidate.to_vec();
    drop(owner);
    fs::write(directory.path().join(Slot::Stage.name()), &candidate).unwrap();
    let mut cold = open(directory.path(), &shared, &mut NeverRng);
    cold.retry(&mut NeverRng).unwrap();
    assert!(!directory.path().join(Slot::Stage.name()).exists());
    assert!(!directory.path().join(GOVERNANCE_SEED).exists());
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        candidate
    );
}

#[test]
fn durable_legacy_stage_fence_precedes_governance_outputs_and_is_owned_on_retry() {
    for cold in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x44f5);
        settled_three(directory.path(), &shared, &mut rng);
        let mut owner = open(directory.path(), &shared, &mut rng);
        fault(&owner, 229);
        assert!(owner.retry(&mut rng).is_err());
        let before = files(directory.path());
        assert_eq!(before[Slot::Stage.name()].as_deref(), Some(&[][..]));
        assert!(before[GOVERNANCE_SEED].is_none());
        assert!(before["chat-owner-v2/governance-layout.bin"].is_none());
        if cold {
            drop(owner);
            assert!(PreparedV2GovernanceLayoutStore::open(
                directory.path(),
                SECRET,
                &mut NeverRng,
                policy(&shared),
            )
            .is_err());
            assert_eq!(files(directory.path()), before);
        } else {
            fault(&owner, 0);
            owner.retry(&mut NeverRng).unwrap();
            assert!(!directory.path().join(Slot::Stage.name()).exists());
        }
    }
}

#[test]
fn warm_legacy_fence_refuses_a_different_empty_native_file() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x44f6);
    settled_three(directory.path(), &shared, &mut rng);
    let mut owner = open(directory.path(), &shared, &mut rng);
    fault(&owner, 229);
    assert!(owner.retry(&mut rng).is_err());
    let stage = directory.path().join(Slot::Stage.name());
    fs::rename(
        &stage,
        directory.path().join("original-empty-stage-test-only"),
    )
    .unwrap();
    fs::write(&stage, []).unwrap();
    let before = files(directory.path());
    fault(&owner, 0);
    assert!(owner.retry(&mut NeverRng).is_err());
    assert_eq!(files(directory.path()), before);
    assert!(before[GOVERNANCE_SEED].is_none());
    assert!(before["chat-owner-v2/governance-layout.bin"].is_none());
}

#[test]
fn legacy_stage_fence_is_checked_by_actual_predecessor() {
    // Ordinary runs pin every new-writer cut and recovery. The separately run
    // check-chat-v2-predecessor.py gate additionally provides a binary compiled
    // from the complete actual predecessor production sources, with no modern
    // fence backported. Its clean positive control prevents a broken child
    // launcher or unsupported fixture from masquerading as compatibility.
    let predecessor = std::env::var_os("MEWTUAL_CHAT_PREDECESSOR_READER");
    for point in [0, 229, 232, 234] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x44f7 + u64::from(point));
        settled_three(directory.path(), &shared, &mut rng);
        if point != 0 {
            let mut owner = open(directory.path(), &shared, &mut rng);
            fault(&owner, point);
            assert!(owner.retry(&mut rng).is_err());
            drop(owner);
            assert_eq!(
                fs::metadata(directory.path().join(Slot::Stage.name()))
                    .unwrap()
                    .len(),
                0
            );
        }
        let before = files(directory.path());
        let full_inventory = complete_fixture_inventory(directory.path());
        if let Some(predecessor) = &predecessor {
            let result = std::process::Command::new(predecessor)
                .arg(directory.path())
                .arg(if point == 0 { "open" } else { "refuse" })
                .output()
                .expect("run actual predecessor public opener");
            assert!(
                result.status.success(),
                "cut {point}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                String::from_utf8_lossy(&result.stdout).contains(if point == 0 {
                    "CURRENT3_OPENED"
                } else {
                    "CURRENT3_REFUSED"
                })
            );
            println!(
                "Actual predecessor public open cut {point}: {}",
                String::from_utf8_lossy(&result.stdout).trim()
            );
        }
        assert_eq!(files(directory.path()), before);
        assert_eq!(complete_fixture_inventory(directory.path()), full_inventory);
        let recovered = PreparedV2GovernanceLayoutStore::open(
            directory.path(),
            SECRET,
            &mut rng,
            policy(&shared),
        );
        if point == 229 {
            assert!(recovered.is_err(), "no authenticated seed to continue cold");
            assert_eq!(files(directory.path()), before);
        } else {
            let mut recovered = recovered.unwrap();
            recovered.retry(&mut rng).unwrap();
            drop(recovered.into_governance_layout_store().unwrap());
        }
    }
}
