//! Actual original factory -> pre-row plan regressions. None supplies a Server, saved boolean,
//! fake scope sender or decoded Fresh. All authored UNRUN during the compilation pause.
use super::*;
use crate::store::{ChatAdmissionKey, CriticalCatalogReadPool};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

const SECRET: &[u8] = b"actual original archive plan";
type Creator = ArchiveFounderCreation<MemNetwork, ChaCha20Rng>;
pub(super) fn fixture(seed: u64) -> (tempfile::TempDir, ServerStore, ChaCha20Rng) {
    let path = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    drop(
        ServerStore::open(path.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let config = ArchiveBootstrapCatalogPolicy::new(pool, 4 * MAX_SEED_FRAMED).unwrap();
    let mut opening =
        PreparedArchiveBootstrapStore::open(path.path(), SECRET, &mut rng, config).unwrap();
    opening.retry(&mut rng).unwrap();
    let store = match opening.into_store() {
        Ok(store) => store,
        Err(_) => panic!("namespace barriers incomplete"),
    };
    (path, store, rng)
}
pub(super) fn begin(store: &ServerStore, seed: u8) -> Creator {
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let policy = ArchiveFounderPolicy::new(limits, 3, codec::PERSISTENT_CONTENT_BYTES).unwrap();
    let input = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(seed as u64)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(seed as u64),
        Box::new(ManualClock::new(10)),
    );
    match store
        .prepare_archive_founder([seed; 32], "private plan", policy, input)
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        ArchiveFounderBegin::Existing { .. } => panic!("fresh original expected"),
    }
}
pub(super) fn created(store: &ServerStore, rng: &mut ChaCha20Rng, seed: u8) -> Creator {
    let mut creator = begin(store, seed);
    assert_eq!(
        creator.advance(store, rng).unwrap(),
        ArchiveFounderProgress::GroupCreatedUnexposed
    );
    creator
}
pub(super) fn actual_key(creator: &Creator) -> ChatAdmissionKey {
    ChatAdmissionKey::new(
        creator.group_data().unwrap(),
        creator.original.data().signer,
    )
    .unwrap()
}
struct CountingRng {
    rng: ChaCha20Rng,
    draws: usize,
    bytes: usize,
}
impl CountingRng {
    fn new() -> Self {
        Self {
            rng: ChaCha20Rng::seed_from_u64(991),
            draws: 0,
            bytes: 0,
        }
    }
}
impl RngCore for CountingRng {
    fn next_u32(&mut self) -> u32 {
        panic!("plan only draws its fixed nonce")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("plan only draws its fixed nonce")
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        self.draws += 1;
        self.bytes += out.len();
        self.rng.fill_bytes(out);
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(out);
        Ok(())
    }
}
impl CryptoRng for CountingRng {}
pub(super) struct NeverRng;
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

#[test]
fn early_move_returns_same_creator_then_actual_plan_keeps_nonce_without_rows_or_sdk_retry() {
    let (path, store, mut rng) = fixture(0xfa01);
    let creator = begin(&store, 1);
    let (mut creator, _) = creator.into_ram_plan(&store).unwrap_err().into_parts();
    creator.advance(&store, &mut rng).unwrap();
    let key = actual_key(&creator);
    let admissions = fs::read(path.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
    let mut plan = creator.into_ram_plan(&store).unwrap();
    let mut nonce_rng = CountingRng::new();
    assert_eq!(
        plan.prepare_ram_plan(&store, &mut nonce_rng).unwrap(),
        ArchiveFounderPlanningProgress::RamPlanRegisteredUnexposed
    );
    let first = plan.plan.as_ref().unwrap().data().unwrap().clone();
    assert_eq!(first.revision, 1);
    assert_eq!(nonce_rng.draws, 1);
    assert_eq!(nonce_rng.bytes, 32);
    plan.prepare_ram_plan(&store, &mut NeverRng).unwrap();
    assert_eq!(plan.plan.as_ref().unwrap().data().unwrap(), &first);
    assert_eq!(
        fs::read(path.path().join("chat-admissions.bin")).unwrap(),
        admissions
    );
    assert_eq!(
        fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
    assert!(
        !path.path().join("owner-state").exists(),
        "negative checking never mkdirs"
    );
    let old = store.capture_chat_admission(key).unwrap();
    assert!(store
        .prepare_legacy_chat_admission(&old, b"no old adopter", &mut NeverRng)
        .is_err());
    let negative = plan.observer();
    drop(plan);
    assert!(negative.is_cancelled());
    assert!(store
        .prepare_legacy_chat_admission(&old, b"weak expiry is not source absence", &mut NeverRng)
        .is_err());
}

#[test]
fn real_old_issued_or_dirty_row_and_revoked_actual_creator_refuse_before_nonce() {
    for dirty in [false, true] {
        let (_path, store, mut rng) = fixture(0xfa10 + dirty as u64);
        let creator = created(&store, &mut rng, 2);
        let key = actual_key(&creator);
        let old = store.capture_chat_admission(key).unwrap();
        store
            .chat_fence_pre_rename_failure
            .store(dirty, std::sync::atomic::Ordering::SeqCst);
        let issued = store.prepare_legacy_chat_admission(&old, b"real old source", &mut rng);
        assert_eq!(issued.is_err(), dirty);
        let mut plan = creator.into_ram_plan(&store).unwrap();
        assert!(plan.prepare_ram_plan(&store, &mut NeverRng).is_err());
    }
    let (_path, store, mut rng) = fixture(0xfa12);
    let creator = created(&store, &mut rng, 3);
    let mut plan = creator.into_ram_plan(&store).unwrap();
    plan.close();
    assert!(plan.prepare_ram_plan(&store, &mut NeverRng).is_err());
}

#[test]
fn retained_fixed_current_stage_and_unknown_parent_refuse_without_checkpoint_inventory() {
    for object in [0u8, 1, 2] {
        let (path, store, mut rng) = fixture(0xfa20 + object as u64);
        let creator = created(&store, &mut rng, 4);
        let key = actual_key(&creator);
        let parent = path.path().join("owner-state");
        if object == 2 {
            fs::write(&parent, b"unknown parent").unwrap();
        } else {
            fs::create_dir(&parent).unwrap();
            let digest = crate::store::owner_evidence::canonical::archive_vacancy::old_key_for_test(
                key.group(),
                key.signer(),
            );
            let suffix = if object == 0 { ".bin" } else { ".replace.tmp" };
            fs::write(
                parent.join(format!("{}{suffix}", hex::encode(digest))),
                b"retained unknown source",
            )
            .unwrap();
        }
        let mut plan = creator.into_ram_plan(&store).unwrap();
        assert!(plan.prepare_ram_plan(&store, &mut NeverRng).is_err());
        if object == 2 {
            assert_eq!(fs::read(&parent).unwrap(), b"unknown parent");
        } else {
            assert_eq!(
                fs::read_dir(parent).unwrap().count(),
                1,
                "no stage cleaner reached"
            );
        }
    }
}

#[test]
fn existing_registered_owner_head_or_pin_is_not_vacant_even_without_disk_bytes() {
    use crate::store::owner_evidence::canonical::archive_vacancy::{
        install_conflicting_metadata_for_test, MetadataConflict,
    };
    for conflict in [MetadataConflict::Head, MetadataConflict::Pin] {
        let (_path, store, mut rng) =
            fixture(0xfa28 + matches!(conflict, MetadataConflict::Pin) as u64);
        let creator = created(&store, &mut rng, 9);
        let key = actual_key(&creator);
        // Deliberately conflicting old metadata is a NEGATIVE fixture. It does not claim a
        // valid owner/profile; ANY registered reservation/pin must block this new namespace.
        install_conflicting_metadata_for_test(&store, key.group(), key.signer(), conflict);
        let mut plan = creator.into_ram_plan(&store).unwrap();
        assert!(plan.prepare_ram_plan(&store, &mut NeverRng).is_err());
    }
}

#[test]
fn target_presence_other_mount_or_changed_scope_refuse_original_plan() {
    let (path, store, mut rng) = fixture(0xfa30);
    let creator = created(&store, &mut rng, 5);
    let key = actual_key(&creator);
    let target = target_name(
        &creator.original.data().invocation.vault,
        key.group(),
        key.signer(),
    )
    .unwrap();
    let target = path
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&target).unwrap());
    fs::create_dir(&target).unwrap();
    let mut plan = creator.into_ram_plan(&store).unwrap();
    assert!(plan.prepare_ram_plan(&store, &mut NeverRng).is_err());
    assert!(target.is_dir());
    let (_other, foreign, _other_rng) = fixture(0xfa31);
    assert!(plan.prepare_ram_plan(&foreign, &mut NeverRng).is_err());

    let (_path, store, mut rng) = fixture(0xfa32);
    let mut creator = created(&store, &mut rng, 6);
    creator
        .server
        .as_mut()
        .unwrap()
        .sync
        .stage_managed_chat_scope([1; 32], [2; 32])
        .unwrap();
    assert!(creator.into_ram_plan(&store).is_err());
}

#[test]
fn root_held_source_check_uses_same_creator_and_cannot_escape_original_writer() {
    // A recursion regression must fail with a bounded diagnostic instead of hanging the
    // entire Rust suite. The worker owns its fixture and locks; timeout drops its JoinHandle
    // without blocking, and no other test needs those private objects to make progress.
    let (finished_tx, finished_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let (_path, store, mut rng) = fixture(0xfa40);
        let creator = created(&store, &mut rng, 7);
        let other = created(&store, &mut rng, 8);
        let mut planning = creator.into_ram_plan(&store).unwrap();
        planning.prepare_ram_plan(&store, &mut rng).unwrap();
        let source = ActualFounderSource::founding(&planning.creator);
        let plan = planning.plan.as_ref().unwrap();
        store
            .with_original_archive_founder(&source, plan, |permit| {
                let vacancy = store.lock_archive_vacant_owner_namespace(permit)?;
                vacancy.validate_original()?;
                // Holding Root here deterministically exposes accidental recursive locking. The
                // source-only method checks originals without ever entering that same mutex again.
                let _root = store.archive_bootstrap_root.lock().unwrap();
                vacancy.validate_source_only(&planning.creator.original)?;
                assert!(vacancy.validate_source_only(&other.original).is_err());
                assert_eq!(vacancy.debit_id(), plan.debit_id());
                assert_eq!(vacancy.plan_data(), plan.data()?);
                planning.close();
                assert!(vacancy
                    .validate_source_only(&planning.creator.original)
                    .is_err());
                Ok(())
            })
            .unwrap();
        finished_tx.send(()).unwrap();
    });
    finished_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("original source-only check must complete while Root is already held");
    worker.join().unwrap();
}

#[test]
fn actual_original_cancellation_during_target_lookup_or_nonce_draw_never_renews_plan() {
    let (_path, store, mut rng) = fixture(0xfa50);
    let creator = created(&store, &mut rng, 10);
    let mut planning = creator.into_ram_plan(&store).unwrap();
    let revoke = planning.creator.original.revoker();
    *store.archive_after_target_lookup.lock().unwrap() = Some(Box::new(move || revoke.revoke()));
    assert!(planning.prepare_ram_plan(&store, &mut NeverRng).is_err());
    assert!(planning
        .plan
        .as_ref()
        .unwrap()
        .sampled_nonce_data_for_test()
        .is_none());

    struct CancelOnDraw {
        rng: ChaCha20Rng,
        revoke: crate::store::persistence::FounderCreationRevoker,
        draws: usize,
    }
    impl RngCore for CancelOnDraw {
        fn next_u32(&mut self) -> u32 {
            panic!("unexpected nonce shape")
        }
        fn next_u64(&mut self) -> u64 {
            panic!("unexpected nonce shape")
        }
        fn fill_bytes(&mut self, out: &mut [u8]) {
            self.rng.fill_bytes(out);
            self.draws += 1;
            self.revoke.revoke();
        }
        fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(out);
            Ok(())
        }
    }
    impl CryptoRng for CancelOnDraw {}
    let (_path, store, mut rng) = fixture(0xfa51);
    let creator = created(&store, &mut rng, 11);
    let mut planning = creator.into_ram_plan(&store).unwrap();
    let mut nonce_rng = CancelOnDraw {
        rng: ChaCha20Rng::seed_from_u64(91),
        revoke: planning.creator.original.revoker(),
        draws: 0,
    };
    assert!(planning.prepare_ram_plan(&store, &mut nonce_rng).is_err());
    assert_eq!(nonce_rng.draws, 1);
    let original = planning
        .plan
        .as_ref()
        .unwrap()
        .sampled_nonce_data_for_test()
        .unwrap();
    assert!(planning.prepare_ram_plan(&store, &mut NeverRng).is_err());
    assert_eq!(
        planning
            .plan
            .as_ref()
            .unwrap()
            .sampled_nonce_data_for_test(),
        Some(original)
    );
}

#[test]
fn actual_founding_intent_barrier_faults_keep_exact_transaction_and_never_create_scope_row() {
    for point in 131..=143u8 {
        let (path, store, mut rng) = fixture(0xfb00 + point as u64);
        let creator = created(&store, &mut rng, 12);
        let admissions = fs::read(path.path().join("chat-admissions.bin")).unwrap();
        let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
        let mut planning = creator.into_ram_plan(&store).unwrap();
        store
            .critical_namespace_fault
            .store(point, std::sync::atomic::Ordering::SeqCst);
        assert!(
            planning.save_founding_intent(&store, &mut rng).is_err(),
            "barrier {point}"
        );
        let (marker, intent, seed) = planning.retained_founding_frames_for_test();
        let marker = marker.to_vec();
        let intent = intent.to_vec();
        let seed = seed.to_vec();
        let facts = planning.plan.as_ref().unwrap().data().unwrap().clone();
        let target = target_name(
            &planning.creator.original.data().invocation.vault,
            &facts.group,
            &facts.signer,
        )
        .unwrap();
        let target = path
            .path()
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(std::str::from_utf8(&target).unwrap());
        if point == 131 {
            assert!(!target.exists(), "pre-mkdir refusal has no target effect");
        } else {
            assert!(
                target.is_dir(),
                "negative target precedes any invocation seed"
            );
        }
        store
            .critical_namespace_fault
            .store(0, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            planning
                .save_founding_intent(&store, &mut NeverRng)
                .unwrap(),
            ArchiveFounderPlanningProgress::FoundingIntentSavedUnexposed
        );
        assert!(planning.saved_founding.is_some());
        assert_eq!(fs::read(target.join("origin")).unwrap(), marker);
        let invocation = planning.creator.original.data().invocation.name();
        let base = path.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(format!(
            "{}.intent",
            std::str::from_utf8(&invocation).unwrap()
        ));
        assert_eq!(fs::read(&base).unwrap(), intent);
        assert_eq!(planning.retained_founding_frames_for_test().2, seed);
        assert_eq!(planning.plan.as_ref().unwrap().data().unwrap(), &facts);
        assert_eq!(
            fs::read(path.path().join("chat-admissions.bin")).unwrap(),
            admissions
        );
        assert_eq!(
            fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
            pins
        );
        assert!(planning
            .creator
            .server
            .as_ref()
            .unwrap()
            .sync
            .chat_scope_label()
            .is_none());
        planning
            .save_founding_intent(&store, &mut NeverRng)
            .unwrap();
        // Saved retry checks actual immutable original files instead of its cached success.
        fs::write(&base, b"foreign replacement").unwrap();
        assert!(planning
            .save_founding_intent(&store, &mut NeverRng)
            .is_err());
        assert_eq!(fs::read(base).unwrap(), b"foreign replacement");
    }
}

#[test]
fn creator_drop_after_seed_failure_preserves_target_and_blocks_old_same_key_while_b_progresses() {
    let (path, store, mut rng) = fixture(0xfb90);
    let creator = created(&store, &mut rng, 13);
    let key = actual_key(&creator);
    let mut planning = creator.into_ram_plan(&store).unwrap();
    store
        .critical_namespace_fault
        .store(138, std::sync::atomic::Ordering::SeqCst);
    assert!(planning.save_founding_intent(&store, &mut rng).is_err());
    let facts = planning.plan.as_ref().unwrap().data().unwrap().clone();
    let target = target_name(
        &planning.creator.original.data().invocation.vault,
        &facts.group,
        &facts.signer,
    )
    .unwrap();
    let target = path
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&target).unwrap());
    let marker = fs::read(target.join("origin")).unwrap();
    let negative = planning.observer();
    drop(planning);
    assert!(negative.is_cancelled());
    assert_eq!(fs::read(target.join("origin")).unwrap(), marker);
    let old = store.capture_chat_admission(key).unwrap();
    assert!(store
        .prepare_legacy_chat_admission(&old, b"creator drop is not source absence", &mut NeverRng)
        .is_err());
    store
        .critical_namespace_fault
        .store(0, std::sync::atomic::Ordering::SeqCst);
    let other = created(&store, &mut rng, 14);
    let mut other = other.into_ram_plan(&store).unwrap();
    assert_eq!(
        other.save_founding_intent(&store, &mut rng).unwrap(),
        ArchiveFounderPlanningProgress::FoundingIntentSavedUnexposed
    );
}

#[test]
fn actual_saved_intents_reclaim_transient_slots_without_changing_original_plan_life() {
    let (_path, store, mut rng) = fixture(0xfc00);
    let mut retained = Vec::new();
    for invocation in 20..27u8 {
        let creator = created(&store, &mut rng, invocation);
        let key = actual_key(&creator);
        let mut planning = creator.into_ram_plan(&store).unwrap();
        assert_eq!(
            planning.save_founding_intent(&store, &mut rng).unwrap(),
            ArchiveFounderPlanningProgress::FoundingIntentSavedUnexposed
        );
        assert!(
            !planning.observer().is_cancelled(),
            "releasing metadata must not revoke same actual task"
        );
        planning
            .save_founding_intent(&store, &mut NeverRng)
            .unwrap();
        assert_eq!(
            planning.prepare_ram_plan(&store, &mut NeverRng).unwrap(),
            ArchiveFounderPlanningProgress::FoundingIntentSavedUnexposed
        );
        let old = store.capture_chat_admission(key).unwrap();
        assert!(store
            .prepare_legacy_chat_admission(&old, b"saved target remains the fence", &mut NeverRng)
            .is_err());
        retained.push(planning);
    }
    let originals: Vec<_> = retained
        .iter()
        .map(|planning| planning.observer())
        .collect();
    drop(retained);
    assert!(originals.iter().all(RequestCancellation::is_cancelled));
}

#[test]
fn unowned_directory_open_gap_stays_held_without_adoption_or_sdk_retry() {
    let (path, store, mut rng) = fixture(0xfc90);
    let creator = created(&store, &mut rng, 31);
    let key = actual_key(&creator);
    let mut planning = creator.into_ram_plan(&store).unwrap();
    store
        .critical_namespace_fault
        .store(144, std::sync::atomic::Ordering::SeqCst);
    assert!(planning.save_founding_intent(&store, &mut rng).is_err());
    let (marker, intent, seed) = planning.retained_founding_frames_for_test();
    let retained = (marker.to_vec(), intent.to_vec(), seed.to_vec());
    let facts = planning.plan.as_ref().unwrap().data().unwrap();
    let name = target_name(
        &planning.creator.original.data().invocation.vault,
        &facts.group,
        &facts.signer,
    )
    .unwrap();
    let target = path
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&name).unwrap());
    assert!(target.is_dir());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    store
        .critical_namespace_fault
        .store(0, std::sync::atomic::Ordering::SeqCst);
    assert!(
        planning
            .save_founding_intent(&store, &mut NeverRng)
            .is_err(),
        "unknown created/open gap never adopts its path"
    );
    assert!(planning.saved_founding.is_none());
    assert_eq!(planning.retained_founding_frames_for_test().0, retained.0);
    assert_eq!(planning.retained_founding_frames_for_test().1, retained.1);
    assert_eq!(planning.retained_founding_frames_for_test().2, retained.2);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    let negative = planning.observer();
    drop(planning);
    assert!(negative.is_cancelled());
    let old = store.capture_chat_admission(key).unwrap();
    assert!(store
        .prepare_legacy_chat_admission(&old, b"unknown empty is not absent", &mut NeverRng)
        .is_err());
    assert!(target.is_dir());
}
