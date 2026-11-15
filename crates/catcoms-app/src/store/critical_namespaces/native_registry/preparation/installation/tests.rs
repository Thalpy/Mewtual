use super::super::tests::{current_policy, initialize, records, NeverRng, SECRET};
use super::*;
use crate::store::persistence::native_registry::{
    arm_native_install_fault, take_native_install_fault,
};
use crate::store::ServerStore;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::fs;

fn policy() -> NativeRegistryPolicy {
    NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap()
}
fn installation(path: &Path, rng: &mut ChaCha20Rng) -> NativeRegistryInstallation {
    NativeRegistryInstallation::prepare(path, SECRET, rng, current_policy(), policy()).unwrap()
}
fn reopen(path: &Path) -> Result<NativeRegistryStore, AppError> {
    NativeRegistryStore::open(path, SECRET, &mut NeverRng, current_policy(), policy())
}
fn file(path: &Path, name: &str) -> std::path::PathBuf {
    if name == SEED_NAME || name == Slot::Stage.name() || name == Slot::Current.name() {
        path.join(name)
    } else {
        path.join(super::super::super::super::ARCHIVE_BOOTSTRAP_ROOT)
            .join(name)
    }
}

#[test]
fn native_install_actual_legacy_rows_survive_install_and_cold_reopen_without_raw_store_escape() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7710);
    initialize(dir.path(), Some(&records()), &mut rng);
    let legacy = fs::read(dir.path().join("registry.bin")).unwrap();
    let mut owner = installation(dir.path(), &mut rng);
    let seed = owner.prepared.output.seed.clone();
    let candidate = owner.prepared.output.candidate.clone();
    owner.retry().unwrap();
    owner.retry().unwrap();
    assert_eq!(owner.prepared.output.seed, seed);
    assert_eq!(
        fs::read(file(dir.path(), Slot::Current.name())).unwrap(),
        candidate.as_slice()
    );
    let mounted = owner.into_native_registry().unwrap();
    assert_eq!(mounted.row_count_data().unwrap(), 1);
    drop(mounted);
    assert_eq!(fs::read(dir.path().join("registry.bin")).unwrap(), legacy);
    assert!(ServerStore::open(dir.path(), SECRET, &mut NeverRng).is_err());
    assert!(PreparedV2CurrentLayoutStore::open(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy()
    )
    .is_err());
    assert_eq!(reopen(dir.path()).unwrap().row_count_data().unwrap(), 1);
}

#[test]
fn native_install_payment_and_persistent_limits_precede_prerequisite_opening() {
    assert!(NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity() - 1).unwrap(),
        PERSISTENT_MINIMUM
    )
    .is_err());
    assert!(NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        PERSISTENT_MINIMUM - 1
    )
    .is_err());
    let native = policy();
    let held = native.preparation.pool.try_reserve(1).unwrap();
    let dir = tempfile::tempdir().unwrap();
    assert!(NativeRegistryInstallation::prepare(
        dir.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        native
    )
    .is_err());
    assert!(!dir.path().join("vault.bin").exists());
    drop(held);
}

#[test]
fn native_install_every_warm_cut_retries_the_same_owned_ciphertexts() {
    for point in 1..=13 {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7720 + u64::from(point));
        initialize(dir.path(), Some(&records()), &mut rng);
        let mut owner = installation(dir.path(), &mut rng);
        let expected = owner.prepared.output.index.clone();
        arm_native_install_fault(point);
        assert!(owner.retry().is_err(), "cut {point}");
        assert!(take_native_install_fault(), "dead cut {point}");
        owner
            .retry()
            .unwrap_or_else(|error| panic!("retry cut {point}: {error:?}"));
        assert_eq!(
            fs::read(file(dir.path(), INDEX_NAME)).unwrap(),
            expected.as_slice()
        );
        assert_eq!(
            owner
                .into_native_registry()
                .unwrap()
                .row_count_data()
                .unwrap(),
            1
        );
    }
}

#[test]
fn native_install_complete_seed_cold_repairs_each_output_and_cleanup_cut_without_rng() {
    for point in 4..=13 {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7740 + u64::from(point));
        initialize(dir.path(), Some(&records()), &mut rng);
        let mut owner = installation(dir.path(), &mut rng);
        let expected = owner.prepared.output.index.clone();
        arm_native_install_fault(point);
        assert!(owner.retry().is_err());
        assert!(take_native_install_fault());
        drop(owner);
        let mounted =
            reopen(dir.path()).unwrap_or_else(|error| panic!("cold cut {point}: {error:?}"));
        assert_eq!(mounted.row_count_data().unwrap(), 1);
        assert_eq!(
            fs::read(file(dir.path(), INDEX_NAME)).unwrap(),
            expected.as_slice()
        );
        assert!(!file(dir.path(), SEED_NAME).exists());
        assert!(!file(dir.path(), Slot::Stage.name()).exists());
    }
}

#[test]
fn native_install_first_torn_seed_abandons_only_its_empty_fence_then_allows_fresh_declaration() {
    for point in 1..=3 {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7760 + u64::from(point));
        initialize(dir.path(), Some(&records()), &mut rng);
        let legacy = fs::read(dir.path().join("registry.bin")).unwrap();
        let mut owner = installation(dir.path(), &mut rng);
        arm_native_install_fault(point);
        assert!(owner.retry().is_err());
        assert!(take_native_install_fault());
        drop(owner);
        // No complete index was ever installed, so this cannot yield a registry owner.
        assert!(reopen(dir.path()).is_err());
        assert!(!file(dir.path(), SEED_NAME).exists());
        assert!(!file(dir.path(), Slot::Stage.name()).exists());
        assert_eq!(fs::read(dir.path().join("registry.bin")).unwrap(), legacy);
        let mut next = installation(dir.path(), &mut rng);
        next.retry().unwrap();
        assert_eq!(
            next.into_native_registry()
                .unwrap()
                .row_count_data()
                .unwrap(),
            1
        );
    }
}

#[test]
fn native_install_warm_retry_never_adopts_an_equal_ciphertext_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7780);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    arm_native_install_fault(6);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    let path = file(dir.path(), INDEX_NAME);
    let data = fs::read(&path).unwrap();
    fs::rename(&path, path.with_extension("retained-original")).unwrap();
    fs::write(&path, &data).unwrap();
    assert!(owner.retry().is_err());
    assert!(file(dir.path(), SEED_NAME).exists());
    assert_eq!(fs::read(&path).unwrap(), data);
}

#[test]
fn native_install_new_legacy_input_cannot_replace_an_absence_association() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7781);
    initialize(dir.path(), None, &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    arm_native_install_fault(4);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    fs::write(dir.path().join("registry.bin"), b"late foreign input").unwrap();
    assert!(owner.retry().is_err());
    drop(owner);
    assert!(reopen(dir.path()).is_err());
    assert!(file(dir.path(), SEED_NAME).exists());
}

#[test]
fn native_install_authenticated_future_seed_is_preserved_without_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7782);
    initialize(dir.path(), None, &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    arm_native_install_fault(4);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    let key = Zeroizing::new(
        owner
            .prepared
            .current
            .checked_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    let future = seal_kind(
        &key,
        SEED_KEY,
        b"authenticated unknown declaration",
        MAX_SEED_PLAIN,
        &mut rng,
    )
    .unwrap();
    drop(owner);
    fs::write(file(dir.path(), SEED_NAME), &future).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(
        fs::read(file(dir.path(), SEED_NAME)).unwrap(),
        future.as_slice()
    );
    assert!(!file(dir.path(), INDEX_NAME).exists());
}

#[test]
fn native_install_promoted_catalog_never_recreates_a_missing_committed_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7783);
    initialize(dir.path(), None, &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    arm_native_install_fault(11);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    drop(owner);
    fs::remove_file(file(dir.path(), MARKER_NAME)).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert!(!file(dir.path(), MARKER_NAME).exists());
    assert!(file(dir.path(), SEED_NAME).exists());
}

#[test]
fn native_install_a_stale_seed_never_replaces_a_later_authenticated_index() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7784);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    let key = Zeroizing::new(
        owner
            .prepared
            .current
            .checked_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    let seed = owner.prepared.output.seed.clone();
    let old_index = owner.prepared.output.index.clone();
    let cap = owner.prepared.output.capability;
    owner.retry().unwrap();
    drop(owner.into_native_registry().unwrap());
    let mut data = IndexData::decode_data(
        &open_kind(&key, INDEX_KEY, &old_index, MAX_PLAIN).unwrap(),
        cap,
    )
    .unwrap();
    data.generation = 2;
    data.parent = Some(*blake3::hash(&old_index).as_bytes());
    let later = seal_kind(
        &key,
        INDEX_KEY,
        &data.encode_data().unwrap(),
        MAX_PLAIN,
        &mut rng,
    )
    .unwrap();
    fs::write(file(dir.path(), INDEX_NAME), &later).unwrap();
    fs::write(file(dir.path(), SEED_NAME), &seed).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(
        fs::read(file(dir.path(), INDEX_NAME)).unwrap(),
        later.as_slice()
    );
    assert_eq!(
        fs::read(file(dir.path(), SEED_NAME)).unwrap(),
        seed.as_slice()
    );
}

#[test]
fn native_install_revoked_original_cannot_transfer_an_owner_after_durable_completion() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7785);
    initialize(dir.path(), None, &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    owner.retry().unwrap();
    owner
        .prepared
        .current
        .cached_store()
        .unwrap()
        .chat_mount
        .revoke();
    assert!(owner.into_native_registry().is_err());
}

#[test]
fn native_install_authenticated_empty_seed_is_not_first_torn_scratch() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x7786);
    initialize(dir.path(), None, &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    let key = Zeroizing::new(
        owner
            .prepared
            .current
            .checked_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    arm_native_install_fault(4);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    drop(owner);
    let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, &key[..]));
    let empty = frame(&seal(&derived, &[], &mut rng).unwrap());
    assert!(authenticate_seed(&key, &empty).unwrap().is_empty());
    fs::write(file(dir.path(), SEED_NAME), &empty).unwrap();
    let stage = fs::read(file(dir.path(), Slot::Stage.name())).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(fs::read(file(dir.path(), SEED_NAME)).unwrap(), empty);
    assert_eq!(
        fs::read(file(dir.path(), Slot::Stage.name())).unwrap(),
        stage
    );
}

#[test]
fn native_install_first_torn_disposal_rejects_replacement_of_its_classified_seed_or_stage() {
    for replace_seed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7787 + u64::from(replace_seed));
        initialize(dir.path(), None, &mut rng);
        let mut owner = installation(dir.path(), &mut rng);
        let key = Zeroizing::new(
            owner
                .prepared
                .current
                .checked_store()
                .unwrap()
                .keys
                .db_key()
                .unwrap(),
        );
        let future = seal_kind(
            &key,
            SEED_KEY,
            b"authenticated replacement",
            MAX_SEED_PLAIN,
            &mut rng,
        )
        .unwrap();
        arm_native_install_fault(2);
        assert!(owner.retry().is_err());
        assert!(take_native_install_fault());
        drop(owner);
        let expected = if replace_seed {
            future.to_vec()
        } else {
            Vec::new()
        };
        let replacement = expected.clone();
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = fired.clone();
        AFTER_CLASSIFICATION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                seen.set(true);
                let selected = file(
                    path,
                    if replace_seed {
                        SEED_NAME
                    } else {
                        Slot::Stage.name()
                    },
                );
                fs::rename(&selected, selected.with_extension("classified-original")).unwrap();
                fs::write(&selected, &replacement).unwrap();
            }))
        });
        assert!(reopen(dir.path()).is_err());
        assert!(fired.get(), "classification hook must execute");
        assert_eq!(
            fs::read(file(
                dir.path(),
                if replace_seed {
                    SEED_NAME
                } else {
                    Slot::Stage.name()
                }
            ))
            .unwrap(),
            expected
        );
        assert!(file(dir.path(), SEED_NAME).exists());
        assert!(file(dir.path(), Slot::Stage.name()).exists());
    }
}

#[test]
fn native_install_first_torn_cleanup_survives_every_unlink_and_parent_barrier_cut() {
    for point in 14..=17 {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7790 + u64::from(point));
        initialize(dir.path(), None, &mut rng);
        let mut owner = installation(dir.path(), &mut rng);
        arm_native_install_fault(2);
        assert!(owner.retry().is_err());
        assert!(take_native_install_fault());
        drop(owner);
        arm_native_install_fault(point);
        assert!(reopen(dir.path()).is_err());
        assert!(take_native_install_fault(), "dead disposal cut {point}");
        assert!(reopen(dir.path()).is_err());
        assert!(!file(dir.path(), SEED_NAME).exists());
        assert!(!file(dir.path(), Slot::Stage.name()).exists());
        let mut next = installation(dir.path(), &mut rng);
        next.retry().unwrap();
        assert_eq!(
            next.into_native_registry()
                .unwrap()
                .row_count_data()
                .unwrap(),
            0
        );
    }
}

#[test]
fn native_install_older_authenticated_wrapper_refuses_before_repair_effects() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x779f);
    let old = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    let wrapper = fs::read(dir.path().join("vault.bin")).unwrap();
    drop(old.into_critical_namespace_store(SECRET, &mut rng).unwrap());
    let mut owner = installation(dir.path(), &mut rng);
    arm_native_install_fault(4);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    let seed = fs::read(file(dir.path(), SEED_NAME)).unwrap();
    drop(owner);
    fs::write(dir.path().join("vault.bin"), wrapper).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(fs::read(file(dir.path(), SEED_NAME)).unwrap(), seed);
    assert!(!file(dir.path(), INDEX_NAME).exists());
    assert!(!file(dir.path(), MARKER_NAME).exists());
}

#[test]
fn native_install_conflicting_declaration_debt_refuses_without_writing_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77a0);
    initialize(dir.path(), None, &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    arm_native_install_fault(4);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    let seed = fs::read(file(dir.path(), SEED_NAME)).unwrap();
    drop(owner);
    let other = dir
        .path()
        .join(super::super::super::super::member_outbox_layout::MEMBER_OUTBOX_SEED);
    fs::write(&other, b"other declaration debt").unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(fs::read(other).unwrap(), b"other declaration debt");
    assert_eq!(fs::read(file(dir.path(), SEED_NAME)).unwrap(), seed);
    assert!(!file(dir.path(), INDEX_NAME).exists());
}

#[test]
fn native_install_late_catalog_replacement_keeps_the_seed_and_remains_cold_repairable() {
    use crate::store::persistence::native_registry::before_native_seed_removal_for_test;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77a1);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    let original = owner.prepared.inputs.catalog().to_vec();
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = fired.clone();
    before_native_seed_removal_for_test(Box::new(move |path| {
        seen.set(true);
        let selected = file(path, Slot::Current.name());
        fs::rename(&selected, selected.with_extension("late-original-selected")).unwrap();
        fs::write(&selected, &original).unwrap();
    }));
    assert!(owner.retry().is_err());
    assert!(fired.get());
    assert!(file(dir.path(), SEED_NAME).exists());
    drop(owner);
    assert_eq!(reopen(dir.path()).unwrap().row_count_data().unwrap(), 1);
}

#[test]
fn native_install_shared_readers_do_not_multiply_the_single_paid_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77a2);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    owner.retry().unwrap();
    let mounted = std::sync::Arc::new(owner.into_native_registry().unwrap());
    let (entered, saw) = std::sync::mpsc::sync_channel(1);
    let (release, finish) = std::sync::mpsc::sync_channel(1);
    let (done, first_result) = std::sync::mpsc::sync_channel(1);
    let bound = std::time::Duration::from_secs(10);
    *mounted.read_hook.lock().unwrap() = Some(Box::new(move || {
        entered.send(()).unwrap();
        finish
            .recv_timeout(bound)
            .expect("first read must be released");
    }));
    let first = mounted.clone();
    let reader = std::thread::spawn(move || done.send(first.row_count_data()).unwrap());
    let entered_result = saw.recv_timeout(bound);
    let (done, second_result) = std::sync::mpsc::sync_channel(1);
    let second = mounted.clone();
    let contender = std::thread::spawn(move || done.send(second.row_count_data()).unwrap());
    let refusal = second_result.recv_timeout(std::time::Duration::from_secs(1));
    // ALWAYS unblock the first read before asserting. A blocking regression produces a
    // bounded failure here, not a hung cohort or a second hook-mutex explanation.
    release.send(()).unwrap();
    let completion = first_result.recv_timeout(bound);
    assert!(entered_result.is_ok());
    assert!(
        matches!(refusal, Ok(Err(_))),
        "second reader must refuse before copying an index"
    );
    assert_eq!(completion.unwrap().unwrap(), 1);
    reader.join().unwrap();
    contender.join().unwrap();
    assert_eq!(mounted.row_count_data().unwrap(), 1);
}

#[test]
fn native_install_late_in_place_index_change_preserves_both_the_new_frame_and_seed() {
    use crate::store::persistence::native_registry::before_native_seed_removal_for_test;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77a3);
    initialize(dir.path(), Some(&records()), &mut rng);
    let mut owner = installation(dir.path(), &mut rng);
    let key = Zeroizing::new(
        owner
            .prepared
            .current
            .checked_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    let cap = owner.prepared.output.capability;
    let old = owner.prepared.output.index.clone();
    let mut data =
        IndexData::decode_data(&open_kind(&key, INDEX_KEY, &old, MAX_PLAIN).unwrap(), cap).unwrap();
    data.generation = 2;
    data.parent = Some(*blake3::hash(&old).as_bytes());
    let later = seal_kind(
        &key,
        INDEX_KEY,
        &data.encode_data().unwrap(),
        MAX_PLAIN,
        &mut rng,
    )
    .unwrap();
    let replacement = later.to_vec();
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = fired.clone();
    before_native_seed_removal_for_test(Box::new(move |path| {
        seen.set(true);
        fs::write(file(path, INDEX_NAME), &replacement).unwrap();
    }));
    assert!(owner.retry().is_err());
    assert!(fired.get());
    assert!(file(dir.path(), SEED_NAME).exists());
    assert_eq!(
        fs::read(file(dir.path(), INDEX_NAME)).unwrap(),
        later.as_slice()
    );
    drop(owner);
    assert!(reopen(dir.path()).is_err());
    assert_eq!(
        fs::read(file(dir.path(), INDEX_NAME)).unwrap(),
        later.as_slice()
    );
}

/// Arrange genuine governance progress while the native claim is temporarily outside the
/// tree, emulating an older reader that does not know the claim's name. No production gate
/// is disabled. The separate a607 binary gate proves that reader can really take this path.
fn governance_during_claim(path: &Path, rng: &mut ChaCha20Rng, point: u8) -> Zeroizing<[u8; 32]> {
    let mut owner = installation(path, rng);
    let key = Zeroizing::new(
        owner
            .prepared
            .current
            .checked_store()
            .unwrap()
            .keys
            .db_key()
            .unwrap(),
    );
    arm_native_install_fault(1);
    assert!(owner.retry().is_err());
    assert!(take_native_install_fault());
    drop(owner);
    let claim = file(path, SEED_NAME);
    let parked = path.join("parked-native7-claim");
    fs::rename(&claim, &parked).unwrap();
    let policy = crate::store::V2GovernanceLayoutPolicy::new(current_policy(), 1 << 30).unwrap();
    let mut governance =
        crate::store::PreparedV2GovernanceLayoutStore::open(path, SECRET, rng, policy).unwrap();
    if point != 0 {
        governance.native_claim_fault_for_test(point);
        assert!(governance.retry(rng).is_err());
    } else {
        governance.retry(rng).unwrap();
    }
    drop(governance);
    fs::rename(&parked, &claim).unwrap();
    key
}

#[test]
fn native_install_first_claim_beside_settled_or_interrupted_governance_leaves_its_stage_untouched()
{
    for point in [0, 232, 235, 236] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x77c0 + u64::from(point));
        initialize(dir.path(), None, &mut rng);
        governance_during_claim(dir.path(), &mut rng, point);
        let fixed = [
            file(dir.path(), Slot::Current.name()),
            file(dir.path(), Slot::Stage.name()),
            dir.path()
                .join(super::super::super::super::governance_layout::GOVERNANCE_SEED),
            file(
                dir.path(),
                super::super::super::super::governance_layout::GOVERNANCE_MARKER,
            ),
        ];
        let before = fixed
            .iter()
            .map(|path| fs::read(path).ok())
            .collect::<Vec<_>>();
        assert!(
            reopen(dir.path()).is_err(),
            "native Current-only owner must still refuse4"
        );
        assert!(!file(dir.path(), SEED_NAME).exists());
        assert_eq!(
            fixed
                .iter()
                .map(|path| fs::read(path).ok())
                .collect::<Vec<_>>(),
            before
        );
        let policy =
            crate::store::V2GovernanceLayoutPolicy::new(current_policy(), 1 << 30).unwrap();
        let mut actual = crate::store::PreparedV2GovernanceLayoutStore::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            policy,
        )
        .unwrap();
        actual.retry(&mut NeverRng).unwrap();
        assert!(actual.into_governance_layout_store().is_ok());
    }
}

#[test]
fn native_install_first_claim_preserves_malformed_or_future_governance_debt() {
    for authenticated in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x77d0 + u64::from(authenticated));
        initialize(dir.path(), None, &mut rng);
        let key = governance_during_claim(dir.path(), &mut rng, 232);
        let gov = dir
            .path()
            .join(super::super::super::super::governance_layout::GOVERNANCE_SEED);
        let replacement = if authenticated {
            let derived = Zeroizing::new(blake3::derive_key(
                "catcoms/private-chat-v2-governance-declaration-key/v1",
                &key[..],
            ));
            frame(
                &seal(
                    &derived,
                    b"authenticated unknown governance declaration",
                    &mut rng,
                )
                .unwrap(),
            )
        } else {
            b"partial or foreign governance seed".to_vec()
        };
        fs::write(&gov, &replacement).unwrap();
        assert!(reopen(dir.path()).is_err());
        assert_eq!(fs::read(&gov).unwrap(), replacement);
        assert_eq!(fs::read(file(dir.path(), SEED_NAME)).unwrap(), b"");
    }
}

#[test]
fn native_install_authenticated_native_seed_beside_governance_is_never_disposable() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77d2);
    initialize(dir.path(), None, &mut rng);
    let key = governance_during_claim(dir.path(), &mut rng, 0);
    let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, &key[..]));
    let empty = frame(&seal(&derived, &[], &mut rng).unwrap());
    fs::write(file(dir.path(), SEED_NAME), &empty).unwrap();
    assert!(reopen(dir.path()).is_err());
    assert_eq!(fs::read(file(dir.path(), SEED_NAME)).unwrap(), empty);
}

#[test]
fn native_fresh_factory_keeps_original_keys_mount_and_session_at_every_boundary() {
    use crate::store::critical_namespaces::current_layout::{NativeFreshEdge, NATIVE_FRESH_HOOK};
    use std::sync::{Arc, Mutex};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fresh");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observations = seen.clone();
    let target = path.clone();
    NATIVE_FRESH_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |edge, store| {
            // This is a genuine competing opener, not a stand-in boolean lock.
            assert!(matches!(
                ServerStore::open(&target, SECRET, &mut NeverRng),
                Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
            ));
            observations.lock().unwrap().push((
                edge,
                store.registry_mount(),
                *blake3::hash(&store.keys.db_key().unwrap()).as_bytes(),
                store.chat_mount.observer(),
            ));
        }))
    });
    let mut rng = ChaCha20Rng::seed_from_u64(0x77f0);
    let mut original = NativeRegistryInstallation::prepare_fresh(
        &path,
        SECRET,
        &mut rng,
        current_policy(),
        policy(),
    )
    .unwrap();
    NATIVE_FRESH_HOOK.with(|slot| drop(slot.borrow_mut().take()));
    {
        let observations = seen.lock().unwrap();
        assert_eq!(observations.len(), 3);
        assert_eq!(
            observations.iter().map(|x| x.0).collect::<Vec<_>>(),
            [
                NativeFreshEdge::LowerOpened,
                NativeFreshEdge::CriticalActivated,
                NativeFreshEdge::NativePrepared
            ]
        );
        let actual = original.prepared.current.cached_store().unwrap();
        for (_, mount, key, negative) in observations.iter() {
            assert!(Arc::ptr_eq(mount, &actual.registry_mount()));
            assert_eq!(
                *key,
                *blake3::hash(&actual.keys.db_key().unwrap()).as_bytes()
            );
            assert!(!negative.is_cancelled());
        }
        assert_eq!(
            actual.chat_vault_format().unwrap(),
            catcoms_storage::VaultFormat::CriticalNamespaces
        );
    }
    let frame = original.prepared.output.index.clone();
    arm_native_install_fault(4);
    assert!(original.retry().is_err());
    assert!(take_native_install_fault());
    original.retry().unwrap(); // the retry API cannot consume RNG at all
    assert_eq!(original.prepared.output.index, frame);
    let owner = original.into_native_registry().unwrap();
    assert_eq!(owner.row_count_data().unwrap(), 0);
    assert!(matches!(
        ServerStore::open(&path, SECRET, &mut NeverRng),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    ));
    drop(owner);
    assert!(seen.lock().unwrap().iter().all(|x| x.3.is_cancelled()));
    assert_eq!(reopen(&path).unwrap().row_count_data().unwrap(), 0);
}

#[test]
fn native_fresh_factory_refuses_all_preexisting_wrapper_versions_without_rng_or_rewrite() {
    for kind in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x77f1 + kind);
        let owner = ServerStore::open(root.path(), SECRET, &mut rng).unwrap();
        owner.save_registry(&records(), &mut rng).unwrap();
        if kind == 1 {
            drop(
                owner
                    .into_critical_namespace_store(SECRET, &mut rng)
                    .unwrap(),
            );
        } else {
            drop(owner);
        }
        if kind >= 2 {
            let wrapper = if kind == 2 { vec![255; 89] } else { Vec::new() };
            fs::write(root.path().join("vault.bin"), wrapper).unwrap();
        }
        let wrapper = fs::read(root.path().join("vault.bin")).unwrap();
        let registry = fs::read(root.path().join("registry.bin")).unwrap();
        assert!(NativeRegistryInstallation::prepare_fresh(
            root.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
            policy(),
        )
        .is_err());
        assert_eq!(fs::read(root.path().join("vault.bin")).unwrap(), wrapper);
        assert_eq!(
            fs::read(root.path().join("registry.bin")).unwrap(),
            registry
        );
        assert!(!root.path().join(SEED_NAME).exists());
    }
    // A nonregular wrapper is presence too, never permission to create keys.
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("vault.bin")).unwrap();
    assert!(NativeRegistryInstallation::prepare_fresh(
        root.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy()
    )
    .is_err());
    assert!(root.path().join("vault.bin").is_dir());
    // Unknown native artifacts without a vault survive the first orphan fence.
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(SEED_NAME), b"unknown original").unwrap();
    assert!(NativeRegistryInstallation::prepare_fresh(
        root.path(),
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy()
    )
    .is_err());
    assert_eq!(
        fs::read(root.path().join(SEED_NAME)).unwrap(),
        b"unknown original"
    );
    assert!(!root.path().join("vault.bin").exists());
}

#[test]
fn native_fresh_factory_payment_refusal_precedes_even_session_directory_creation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("not-created");
    let policy = policy();
    let held = policy.preparation.pool.try_reserve(1).unwrap();
    assert!(NativeRegistryInstallation::prepare_fresh(
        &path,
        SECRET,
        &mut NeverRng,
        current_policy(),
        policy
    )
    .is_err());
    assert!(!path.exists());
    drop(held);
}

#[test]
fn native_fresh_factory_missing_wrapper_never_creates_new_keys_beside_old_ciphertext() {
    for with_registry in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x77ff);
        let store = ServerStore::open(root.path(), SECRET, &mut rng).unwrap();
        if with_registry {
            store.save_registry(&records(), &mut rng).unwrap();
        }
        // Actual sealed server data, not just a fabricated orphan basename.
        store
            .save_server(9, b"original encrypted server", &mut rng)
            .unwrap();
        drop(store);
        let original = fs::read(root.path().join("servers/9.bin")).unwrap();
        fs::remove_file(root.path().join("vault.bin")).unwrap();
        assert!(NativeRegistryInstallation::prepare_fresh(
            root.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
            policy()
        )
        .is_err());
        assert!(!root.path().join("vault.bin").exists());
        assert_eq!(
            fs::read(root.path().join("servers/9.bin")).unwrap(),
            original
        );
        if with_registry {
            assert!(root.path().join("registry.bin").is_file());
        }
    }
}
