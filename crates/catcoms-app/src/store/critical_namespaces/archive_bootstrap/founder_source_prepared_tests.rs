//! Actual original source -> small SP journal regressions. Authored UNRUN during the user's
//! compilation pause; these are not source-file, arm, Current or native acceptance evidence.
use super::super::super::tests::{created, fixture, NeverRng};
use super::super::*;
use catcoms_rt::MemNetwork;
use rand_chacha::ChaCha20Rng;
use std::sync::atomic::Ordering;

type Owner = ArchiveFounderSourcePreparation<MemNetwork, ChaCha20Rng>;
fn owner(store: &ServerStore, rng: &mut ChaCha20Rng, invocation: u8) -> Owner {
    let creator = created(store, rng, invocation);
    let mut plan = creator.into_ram_plan(store).unwrap();
    plan.save_founding_intent(store, rng).unwrap();
    plan.into_source_preparation(store).unwrap()
}
fn control(store: &ServerStore, owner: &Owner, suffix: &str) -> PathBuf {
    let name = owner.original.creator.original.data().invocation.name();
    store.dir.join(ARCHIVE_BOOTSTRAP_ROOT).join(format!(
        "{}{}",
        std::str::from_utf8(&name).unwrap(),
        suffix
    ))
}
fn decode(store: &ServerStore, owner: &Owner) -> codec::IntentData {
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    codec::open_intent_data(
        &key,
        owner.original.creator.original.data(),
        owner.original.plan.as_ref().unwrap().debit_id(),
        &fs::read(control(store, owner, ".intent")).unwrap(),
    )
    .unwrap()
}
fn fault(store: &ServerStore, point: u8) {
    store
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}

#[test]
fn real_source_prepared_is_exact_saved_successor_without_large_files_rows_or_arms() {
    let (path, store, mut rng) = fixture(0xfe01);
    let mut source = owner(&store, &mut rng, 61);
    let rows = fs::read(path.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
    assert_eq!(
        source.save_source_prepared(&store, &mut rng).unwrap(),
        ArchiveFounderSourceProgress::SourcePreparedJournalSavedUnexposed
    );
    let SourcePhase::Transferred { context, id, .. } = &source.phase else {
        panic!("actual transfer required")
    };
    let data = decode(&store, &source);
    assert_eq!((data.sequence, data.io_spent), (2, 0));
    assert!(data.parent.is_some());
    match &data.kind {
        codec::IntentKind::SourcePrepared {
            context: actual,
            source: actual_id,
            ..
        } => {
            assert_eq!(actual, context.as_ref());
            assert_eq!(actual_id, id);
        }
        _ => panic!("actual source successor missing"),
    }
    let facts = source.original.plan.as_ref().unwrap().data().unwrap();
    let target = target_name(&facts.scope_domain, &facts.group, &facts.signer).unwrap();
    let target = path
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&target).unwrap());
    let names: Vec<_> = fs::read_dir(target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("origin")],
        "SP admits no large source/page write"
    );
    assert!(!control(&store, &source, ".intent-seed").exists());
    assert!(!control(&store, &source, ".intent-stage").exists());
    assert_eq!(
        fs::read(path.path().join("chat-admissions.bin")).unwrap(),
        rows
    );
    assert_eq!(
        fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
    let current = fs::read(control(&store, &source, ".intent")).unwrap();
    source.save_source_prepared(&store, &mut NeverRng).unwrap();
    assert_eq!(
        fs::read(control(&store, &source, ".intent")).unwrap(),
        current
    );
}

#[test]
fn all_small_source_prepared_faults_repair_same_cipher_and_metadata_without_nonce_refresh() {
    for point in 150..=157u8 {
        let (_path, store, mut rng) = fixture(0xfe10 + u64::from(point));
        let mut source = owner(&store, &mut rng, 62);
        let original = fs::read(control(&store, &source, ".intent")).unwrap();
        fault(&store, point);
        assert!(
            source.save_source_prepared(&store, &mut rng).is_err(),
            "fault {point}"
        );
        assert!(source.prepared_journal.is_none());
        let retained = store.retained_bootstrap_source_for_test([62; 32]).unwrap();
        let interrupted = fs::read(control(&store, &source, ".intent")).unwrap();
        if point < 156 {
            assert_eq!(interrupted, original);
        }
        fault(&store, 0);
        assert_eq!(
            source.save_source_prepared(&store, &mut NeverRng).unwrap(),
            ArchiveFounderSourceProgress::SourcePreparedJournalSavedUnexposed
        );
        assert_eq!(
            store.retained_bootstrap_source_for_test([62; 32]).unwrap(),
            retained
        );
        let saved = fs::read(control(&store, &source, ".intent")).unwrap();
        if point >= 156 {
            assert_eq!(saved, interrupted);
        }
        assert_eq!(decode(&store, &source).io_spent, 0);
        assert!(!control(&store, &source, ".intent-seed").exists());
        source.save_source_prepared(&store, &mut NeverRng).unwrap();
        assert_eq!(
            fs::read(control(&store, &source, ".intent")).unwrap(),
            saved
        );
    }
}

#[test]
fn equal_replacement_seed_or_foreign_stage_never_inherits_original_held_file_custody() {
    for replace_seed in [false, true] {
        let (path, store, mut rng) = fixture(0xfe30 + u64::from(replace_seed));
        let mut source = owner(&store, &mut rng, 63);
        fault(&store, 153);
        assert!(source.save_source_prepared(&store, &mut rng).is_err());
        let seed_path = control(&store, &source, ".intent-seed");
        let seed = fs::read(&seed_path).unwrap();
        if replace_seed {
            fs::rename(&seed_path, path.path().join("retained-original-seed")).unwrap();
            fs::write(&seed_path, &seed).unwrap();
        } else {
            fs::write(control(&store, &source, ".intent-stage"), b"foreign stage").unwrap();
        }
        fault(&store, 0);
        assert!(source.save_source_prepared(&store, &mut NeverRng).is_err());
        assert!(source.prepared_journal.is_none());
        assert_eq!(fs::read(seed_path).unwrap(), seed);
        if !replace_seed {
            assert_eq!(
                fs::read(control(&store, &source, ".intent-stage")).unwrap(),
                b"foreign stage"
            );
        }
    }
}

#[test]
fn saved_source_journal_forbids_parent_rollback_and_preserves_corrupt_current() {
    let (_path, store, mut rng) = fixture(0xfe40);
    let mut source = owner(&store, &mut rng, 64);
    let parent = fs::read(control(&store, &source, ".intent")).unwrap();
    source.save_source_prepared(&store, &mut rng).unwrap();
    let current = control(&store, &source, ".intent");
    // Same-object mutation deliberately preserves the current SessionId. Saved state must
    // reject rollback to an old valid parent instead of treating it as a pending first write.
    fs::write(&current, &parent).unwrap();
    assert!(source.save_source_prepared(&store, &mut NeverRng).is_err());
    assert_eq!(fs::read(current).unwrap(), parent);
    assert!(
        store
            .retained_bootstrap_source_for_test([64; 32])
            .unwrap()
            .0
            > 0
    );
}

#[test]
fn saved_source_current_mutation_after_full_comparers_is_rejected_before_return() {
    let (_path, store, mut rng) = fixture(0xfe50);
    let mut source = owner(&store, &mut rng, 65);
    source.save_source_prepared(&store, &mut rng).unwrap();
    let current = control(&store, &source, ".intent");
    let original = fs::read(&current).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    let modified = changed.clone();
    let target = current.clone();
    *store.founder_before_source_admission.lock().unwrap() =
        Some(Box::new(move || fs::write(target, modified).unwrap()));
    assert!(source.save_source_prepared(&store, &mut NeverRng).is_err());
    assert!(store
        .founder_before_source_admission
        .lock()
        .unwrap()
        .is_none());
    assert_eq!(fs::read(current).unwrap(), changed);
}

#[test]
fn first_saved_sp_never_issues_after_current_changes_during_final_comparer_or_creator_closes() {
    for close in [false, true] {
        let (_path, store, mut rng) = fixture(0xfe58 + u64::from(close));
        let mut source = owner(&store, &mut rng, 68);
        let current = control(&store, &source, ".intent");
        if close {
            let revoker = source.original.creator.original.revoker();
            *store.founder_before_source_prepared_saved.lock().unwrap() =
                Some(Box::new(move || revoker.revoke()));
        } else {
            let path = current.clone();
            *store.founder_before_source_prepared_saved.lock().unwrap() =
                Some(Box::new(move || {
                    let mut bytes = fs::read(&path).unwrap();
                    bytes[0] ^= 1;
                    fs::write(path, bytes).unwrap();
                }));
        }
        assert!(source.save_source_prepared(&store, &mut rng).is_err());
        assert!(
            store
                .founder_before_source_prepared_saved
                .lock()
                .unwrap()
                .is_none(),
            "actual final writer seam ran"
        );
        assert!(source.prepared_journal.is_none());
        assert!(
            store
                .retained_bootstrap_source_for_test([68; 32])
                .unwrap()
                .0
                > 0
        );
        assert!(source.save_source_prepared(&store, &mut NeverRng).is_err());
        assert!(
            current.exists(),
            "failed admission must preserve the actual pending current"
        );
    }
}

#[test]
fn lost_local_source_and_saved_handles_lookup_same_sp_without_reviving_old_fi() {
    let (_path, store, mut rng) = fixture(0xfe60);
    let mut source = owner(&store, &mut rng, 66);
    source.save_source_prepared(&store, &mut rng).unwrap();
    let current = fs::read(control(&store, &source, ".intent")).unwrap();
    let retained = store.retained_bootstrap_source_for_test([66; 32]).unwrap();
    source.prepared_journal = None;
    source.registered = None;
    source.save_source_prepared(&store, &mut NeverRng).unwrap();
    assert_eq!(
        fs::read(control(&store, &source, ".intent")).unwrap(),
        current
    );
    assert_eq!(
        store.retained_bootstrap_source_for_test([66; 32]).unwrap(),
        retained
    );
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let SourcePhase::Transferred {
        source: bootstrap,
        context,
        id,
    } = &source.phase
    else {
        panic!("actual transfer")
    };
    let actual = ActualFounderSource {
        creator: &source.original.creator,
        phase: ActualSourcePhase::Transferred {
            source: bootstrap,
            key: &key,
            context,
            id: *id,
        },
    };
    let plan = source.original.plan.as_ref().unwrap();
    store
        .with_original_archive_founder(&actual, plan, |admission| {
            let guard = store.lock_archive_vacant_owner_namespace(admission)?;
            assert!(
                source
                    .original
                    .saved_founding
                    .as_ref()
                    .unwrap()
                    .validate_original(&store, &source.original.creator.original, &guard)
                    .is_err(),
                "old FI saved result still requires the exact FI current"
            );
            source.prepared_journal.as_ref().unwrap().validate_original(
                &store,
                &source.original.creator.original,
                &guard,
                source.registered.as_ref().unwrap(),
            )
        })
        .unwrap();
}

#[test]
fn close_after_cached_or_admitted_sp_preserves_cipher_without_new_saved_authority() {
    for point in [150, 151] {
        let (_path, store, mut rng) = fixture(0xfe70 + u64::from(point));
        let mut source = owner(&store, &mut rng, 67);
        let parent = fs::read(control(&store, &source, ".intent")).unwrap();
        fault(&store, point);
        assert!(source.save_source_prepared(&store, &mut rng).is_err());
        assert_eq!(
            control(&store, &source, ".intent-seed").exists(),
            point == 151
        );
        let retained = store.retained_bootstrap_source_for_test([67; 32]).unwrap();
        source.close();
        fault(&store, 0);
        assert!(source.save_source_prepared(&store, &mut NeverRng).is_err());
        assert!(source.prepared_journal.is_none());
        assert_eq!(
            fs::read(control(&store, &source, ".intent")).unwrap(),
            parent
        );
        assert_eq!(
            store.retained_bootstrap_source_for_test([67; 32]).unwrap(),
            retained
        );
    }
}
