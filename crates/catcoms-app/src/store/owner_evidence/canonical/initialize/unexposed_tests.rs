//! Actual factory -> durable Initializing row -> crash/reopen -> negative recovery. No caller
//! boolean, fabricated fresh admission or dummy successful snapshot callback authorizes closure.
use super::*;
use crate::store::{ChatAdmissionKey, ChatAdmissionResolution};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"unexposed canonical recovery fixture";
type Attempt = OwnerFounderInitialization<MemNetwork, ChaCha20Rng>;

fn open(directory: &std::path::Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(directory, SECRET, rng).unwrap();
    let store = if store.chat_vault_format().unwrap() == catcoms_storage::VaultFormat::OwnerEvidence
    {
        store
    } else {
        store.into_owner_evidence_store(SECRET, rng).unwrap()
    };
    store
        .configure_owner_evidence_budget(
            OwnerEvidenceBudget::checkpoint(
                4,
                16 * 1024 * 1024,
                8 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                512 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
    store
}

fn begin(store: &ServerStore, number: u64) -> Attempt {
    store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(number)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(number),
            Box::new(ManualClock::new(1_000)),
            "never exposed founder",
            9,
        )
        .unwrap()
}

fn unexposed(store: &ServerStore, identity: &ChatAdmissionKey) -> OwnerUnexposedInitialization {
    let OwnerRestoreClassification::UnexposedInitialization(prepared) = store
        .prepare_owner_evidence_restore(identity.group(), *identity.signer())
        .unwrap()
    else {
        panic!("only exact first no-source reservation can close negatively");
    };
    prepared
}

#[test]
fn actual_pre_first_save_crash_reopen_has_automatic_negative_closure_without_a_pin_or_transport() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe001);
    let store = open(directory.path(), &mut rng);
    let mut initial = begin(&store, 0xe001);
    let scope = initial.private.reserve_scope(&store, &mut rng).unwrap();
    let identity =
        ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
    let nonce = scope.nonce();
    assert!(store
        .capture_mounted_owner_profile(identity.group())
        .is_err());
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Initializing(_)
    ));
    drop(initial); // actual lost private creation/MLS material; no canonical body was ever saved
    drop(store);

    let store = open(directory.path(), &mut rng);
    let mut recovery = unexposed(&store, &identity);
    assert_eq!(
        recovery.retry(&store, &mut rng).unwrap(),
        OwnerUnexposedRetirement::Retired
    );
    assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
    assert!(store
        .owner_evidence
        .canonical
        .lock()
        .unwrap()
        .heads
        .is_empty());
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Retired
    ));
    assert!(store
        .owner_canonical_directory()
        .unwrap()
        .read_dir()
        .unwrap()
        .next()
        .is_none());
    drop(recovery);
    drop(store);

    let store = open(directory.path(), &mut rng);
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Retired
    ));
    // No bare missing-source restore can mint another admission or erase the identity record.
    assert!(store
        .prepare_owner_evidence_restore(identity.group(), *identity.signer())
        .is_err());
}

#[test]
fn actual_canonical_current_promotion_after_capture_cannot_be_undone_by_unpolled_negative_work() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe002);
    let store = open(directory.path(), &mut rng);
    let mut initial = begin(&store, 0xe002);
    let scope = initial.private.reserve_scope(&store, &mut rng).unwrap();
    let identity =
        ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
    // There is no canonical writer yet. Capture alone must not retire or consume that creator.
    let mut recovery = unexposed(&store, &identity);
    let owned = initial.retry(&store, &mut rng).unwrap();
    let path = directory.path().join("owner-state").join(format!(
        "{}.bin",
        hex::encode(key(identity.group(), identity.signer()))
    ));
    let before = fs::read(&path).unwrap();
    assert!(recovery.retry(&store, &mut rng).is_err());
    assert!(owned.is_live());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, scope.nonce())
            .unwrap(),
        ChatAdmissionResolution::Current(_)
    ));
}

#[test]
fn real_pre_rename_pending_creator_and_post_rename_crash_source_are_not_absence() {
    for after_rename in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe003);
        let store = open(directory.path(), &mut rng);
        let mut initial = begin(&store, 0xe003);
        store
            .owner_evidence
            .canonical_fault
            .store(if after_rename { 2 } else { 1 }, Ordering::SeqCst);
        assert!(initial.retry(&store, &mut rng).is_err());
        let identity =
            ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
        assert!(
            store
                .try_prepare_unexposed_owner_initialization(&identity)
                .unwrap()
                .is_none(),
            "registered live/pending original writer precedes disk absence"
        );
        if after_rename {
            let path = directory.path().join("owner-state").join(format!(
                "{}.bin",
                hex::encode(key(identity.group(), identity.signer()))
            ));
            let before = fs::read(&path).unwrap();
            drop(initial);
            drop(store);
            let store = open(directory.path(), &mut rng);
            assert!(matches!(
                store
                    .prepare_owner_evidence_restore(identity.group(), *identity.signer())
                    .unwrap(),
                OwnerRestoreClassification::Initializing(_)
            ));
            assert!(store
                .try_prepare_unexposed_owner_initialization(&identity)
                .unwrap()
                .is_none());
            assert_eq!(fs::read(path).unwrap(), before);
        }
    }
}

#[test]
fn real_startup_preserves_stage_only_original_and_charges_its_identity_before_absence_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe004);
    let store = open(directory.path(), &mut rng);
    let mut initial = begin(&store, 0xe004);
    initial.private.reserve_scope(&store, &mut rng).unwrap();
    let identity =
        ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
    let stage = store.owner_canonical_directory().unwrap().join(format!(
        "{}.replace.tmp",
        hex::encode(key(identity.group(), identity.signer()))
    ));
    let staged_bytes = b"retained interrupted candidate; not authenticated by this test";
    fs::write(&stage, staged_bytes).unwrap();
    drop(initial);
    drop(store);

    // This exercises ACTUAL configure/inventory order. A stage planted after configure would
    // miss the old startup cleanup that erased it before the negative absence check.
    let store = open(directory.path(), &mut rng);
    assert_eq!(fs::read(&stage).unwrap(), staged_bytes);
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().heads.len(),
        1
    );
    assert!(store
        .try_prepare_unexposed_owner_initialization(&identity)
        .unwrap()
        .is_none());
    assert!(store
        .prepare_owner_evidence_restore(identity.group(), *identity.signer())
        .is_err());
    assert_eq!(fs::read(stage).unwrap(), staged_bytes);
}

#[test]
fn original_negative_retry_preserves_exact_retirement_through_directory_and_both_fence_write_faults(
) {
    for after_rename in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe005);
        let store = open(directory.path(), &mut rng);
        let mut initial = begin(&store, 0xe005);
        let scope = initial.private.reserve_scope(&store, &mut rng).unwrap();
        let identity =
            ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
        drop(initial);
        drop(store);
        let store = open(directory.path(), &mut rng);
        let mut recovery = unexposed(&store, &identity);
        store
            .owner_evidence
            .directory_sync_failure
            .store(true, Ordering::SeqCst);
        assert!(recovery.retry(&store, &mut rng).is_err());
        store
            .owner_evidence
            .directory_sync_failure
            .store(false, Ordering::SeqCst);
        assert!(
            matches!(
                store
                    .resolve_chat_admission(&identity, scope.nonce())
                    .unwrap(),
                ChatAdmissionResolution::Initializing(_)
            ),
            "failed absence barrier cannot retire"
        );
        if after_rename {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        let error = recovery.retry(&store, &mut rng).unwrap_err();
        assert_eq!(
            matches!(error, AppError::CommittedButNotDurable(_)),
            after_rename
        );
        assert_eq!(
            recovery.retry(&store, &mut rng).unwrap(),
            OwnerUnexposedRetirement::Retired
        );
        assert!(matches!(
            store
                .resolve_chat_admission(&identity, scope.nonce())
                .unwrap(),
            ChatAdmissionResolution::Retired
        ));
    }
}

#[test]
fn negative_winner_cancels_creator_that_passed_early_validation_before_canonical_pool_registration()
{
    use std::sync::mpsc::{channel, Sender};
    use std::time::Duration;

    // Harness timeout only: production state/time still uses its injected Clock. An unwind
    // must release the held method, and a failed regression must never trap a scoped join.
    struct ReleaseOnDrop(Option<Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe006);
    let store = Arc::new(open(directory.path(), &mut rng));
    let mut initial = begin(&store, 0xe006);
    let scope = initial.private.reserve_scope(&store, &mut rng).unwrap();
    let identity =
        ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
    let attempt = initial
        .private
        .server()
        .unwrap()
        .sync
        .fresh_chat_admission()
        .unwrap()
        .attempt_id();
    let mut recovery = unexposed(&store, &identity);
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (done_tx, done_rx) = channel();
    let release = ReleaseOnDrop(Some(release_tx));
    *store
        .owner_evidence
        .before_owner_initialization_pool
        .lock()
        .unwrap() = Some(Box::new(move || {
        reached_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }));
    let OwnerFounderInitialization {
        mut private,
        mut creation,
        ..
    } = initial;
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || {
        let result = worker_store.reserve_owner_initialization(&scope, attempt, &mut creation);
        let refused = result.is_err();
        drop(result);
        drop(creation);
        let _ = done_tx.send(refused);
    });
    reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        recovery.retry(&store, &mut rng).unwrap(),
        OwnerUnexposedRetirement::Retired
    );
    drop(release); // actual method passed early validation; release AFTER real retirement
    assert!(
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
        "post-pool original observer must refuse before head insertion"
    );
    worker.join().unwrap(); // completion already observed; no unbounded coordination wait
    assert!(store
        .owner_evidence
        .canonical
        .lock()
        .unwrap()
        .heads
        .is_empty());
    assert!(private.reserve_scope(&store, &mut rng).is_err());
}

#[test]
fn actual_post_rename_negative_save_failure_reopens_as_exact_retired_lineage_without_positive_bootstrap(
) {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe007);
    let store = open(directory.path(), &mut rng);
    let mut initial = begin(&store, 0xe007);
    let scope = initial.private.reserve_scope(&store, &mut rng).unwrap();
    let identity =
        ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
    drop(initial);
    drop(store);
    let store = open(directory.path(), &mut rng);
    let mut recovery = unexposed(&store, &identity);
    store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
    assert!(matches!(
        recovery.retry(&store, &mut rng),
        Err(AppError::CommittedButNotDurable(_))
    ));
    drop(recovery); // no in-memory completion or private initial material survives
    drop(store);

    let store = open(directory.path(), &mut rng);
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, scope.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
    assert!(store
        .try_prepare_unexposed_owner_initialization(&identity)
        .unwrap()
        .is_none());
    assert!(store
        .prepare_owner_evidence_restore(identity.group(), *identity.signer())
        .is_err());
    // A generic Retired record cannot prove that it was NEVER Current. Do not manufacture a
    // stronger "unexposed" history/completeness result from missing files after restart. The
    // future native registry/recovery adapter must handle exact closed lineage separately.
}

#[test]
fn negative_no_source_closure_remains_available_when_retained_stages_fill_all_canonical_slots() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe008);
    let store = open(directory.path(), &mut rng);
    let mut initial = begin(&store, 0xe008);
    let scope = initial.private.reserve_scope(&store, &mut rng).unwrap();
    let identity =
        ChatAdmissionKey::new(initial.group().unwrap(), *initial.founder().unwrap()).unwrap();
    let parent = store.owner_canonical_directory().unwrap();
    for byte in 1u8..=4 {
        fs::write(
            parent.join(format!("{}.replace.tmp", hex::encode([byte; 32]))),
            [byte],
        )
        .unwrap();
    }
    drop(initial);
    drop(store);
    let store = open(directory.path(), &mut rng);
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().heads.len(),
        4
    );
    let mut recovery = unexposed(&store, &identity);
    assert_eq!(
        recovery.retry(&store, &mut rng).unwrap(),
        OwnerUnexposedRetirement::Retired
    );
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().heads.len(),
        4
    );
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, scope.nonce())
            .unwrap(),
        ChatAdmissionResolution::Retired
    ));
    for byte in 1u8..=4 {
        assert_eq!(
            fs::read(parent.join(format!("{}.replace.tmp", hex::encode([byte; 32])))).unwrap(),
            [byte]
        );
    }
}
