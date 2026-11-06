//! Actual warm completed-run -> terminal small-record tests. Authored UNRUN during the
//! compilation pause; cold DATA reopening here is not a paid Current/SDK restore consumer.
use super::super::super::tests::{created, fixture, NeverRng, SECRET};
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
    codec::open_intent_data(
        &store.keys.db_key().unwrap(),
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
fn actual_terminal_follows_completed_run_preserves_spent_units_and_lost_handles_recover_same_event()
{
    let (path, store, mut rng) = fixture(0xe001);
    let mut source = owner(&store, &mut rng, 101);
    let rows = fs::read(path.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
    assert_eq!(
        source.save_source_terminal(&store, &mut rng).unwrap(),
        ArchiveFounderSourceProgress::SourceSavedUnexposed
    );
    let data = decode(&store, &source);
    assert_eq!((data.sequence, data.io_spent), (4, 1));
    let SourcePhase::Transferred { context, id, .. } = &source.phase else {
        panic!("actual transfer");
    };
    match data.kind {
        codec::IntentKind::SourceSaved {
            context: actual,
            source: actual_id,
            ..
        } => {
            assert_eq!(actual, **context);
            assert_eq!(actual_id, *id);
        }
        _ => panic!("actual terminal missing"),
    }
    let selected = fs::read(control(&store, &source, ".intent")).unwrap();
    // All caller event handles may be lost independently of Root's actual retained closure.
    source.prepared_terminal = None;
    source.saved_terminal = None;
    source.saved_files = None;
    source.save_source_terminal(&store, &mut NeverRng).unwrap();
    assert_eq!(
        fs::read(control(&store, &source, ".intent")).unwrap(),
        selected
    );
    assert!(
        source.save_source_files(&store, &mut NeverRng).is_err(),
        "old arm-current API stays strict"
    );
    source.save_source_terminal(&store, &mut NeverRng).unwrap();
    assert_eq!(
        fs::read(path.path().join("chat-admissions.bin")).unwrap(),
        rows
    );
    assert_eq!(
        fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
    assert!(!control(&store, &source, ".intent-stage").exists());
    assert!(!control(&store, &source, ".intent-seed").exists());
}

#[test]
fn every_terminal_control_fault_repairs_same_candidate_without_new_nonce_or_large_pass() {
    for point in 150..=157 {
        let (_path, store, mut rng) = fixture(0xe010 + u64::from(point));
        let mut source = owner(&store, &mut rng, 102);
        source.save_source_files(&store, &mut rng).unwrap();
        let before = fs::read(control(&store, &source, ".intent")).unwrap();
        let retained = store.retained_bootstrap_source_for_test([102; 32]).unwrap();
        fault(&store, point);
        assert!(
            source.save_source_terminal(&store, &mut rng).is_err(),
            "fault {point}"
        );
        assert!(source.saved_terminal.is_none());
        let interrupted = fs::read(control(&store, &source, ".intent")).unwrap();
        if point < 156 {
            assert_eq!(interrupted, before);
        }
        // Recovering through actual Root preparation must precede the now-invalid old
        // SavedFiles API, including a lost return after the terminal was already promoted.
        source.prepared_terminal = None;
        source.saved_files = None;
        fault(&store, 0);
        source.save_source_terminal(&store, &mut NeverRng).unwrap();
        let data = decode(&store, &source);
        assert_eq!((data.sequence, data.io_spent), (4, 1));
        assert!(matches!(data.kind, codec::IntentKind::SourceSaved { .. }));
        if point >= 156 {
            assert_eq!(
                fs::read(control(&store, &source, ".intent")).unwrap(),
                interrupted
            );
        }
        assert_eq!(
            store.retained_bootstrap_source_for_test([102; 32]).unwrap(),
            retained
        );
    }
}

#[test]
fn exhausted_successful_last_pass_can_close_and_terminal_data_survives_a_new_physical_mount() {
    let (path, store, mut rng) = fixture(0xe030);
    let mut source = owner(&store, &mut rng, 103);
    for spent in 1..=2 {
        fault(&store, 162);
        assert!(source.save_source_files(&store, &mut rng).is_err());
        assert_eq!(decode(&store, &source).io_spent, spent);
    }
    fault(&store, 0);
    source.save_source_files(&store, &mut rng).unwrap();
    assert_eq!(decode(&store, &source).io_spent, 3);
    source.save_source_terminal(&store, &mut rng).unwrap();
    let data = decode(&store, &source);
    assert_eq!((data.sequence, data.io_spent), (6, 3));
    let invocation = source.original.creator.original.data().clone();
    let debit = source.original.plan.as_ref().unwrap().debit_id();
    let current = control(&store, &source, ".intent");
    let bytes = fs::read(&current).unwrap();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    drop(source);
    drop(store);
    let config = ArchiveBootstrapCatalogPolicy::new(pool, 4 * MAX_SEED_FRAMED).unwrap();
    let mut opening =
        PreparedArchiveBootstrapStore::open(path.path(), SECRET, &mut rng, config).unwrap();
    opening.retry(&mut rng).unwrap();
    let reopened = match opening.into_store() {
        Ok(store) => store,
        Err(_) => panic!("actual namespace reopen"),
    };
    let decoded = codec::open_intent_data(
        &reopened.keys.db_key().unwrap(),
        &invocation,
        debit,
        &fs::read(&current).unwrap(),
    )
    .unwrap();
    assert!(decoded == data);
    assert_eq!(fs::read(current).unwrap(), bytes);
    assert!(
        reopened
            .retained_bootstrap_source_for_test([103; 32])
            .is_none(),
        "cold DATA never resurrects a warm run/source token"
    );
}

#[test]
fn another_originals_real_completed_result_cannot_close_an_unwritten_source() {
    let (_path, store, mut rng) = fixture(0xe040);
    let mut a = owner(&store, &mut rng, 104);
    let mut b = owner(&store, &mut rng, 105);
    a.save_source_prepared(&store, &mut rng).unwrap();
    b.save_source_files(&store, &mut rng).unwrap();
    let before = fs::read(control(&store, &a, ".intent")).unwrap();
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let SourcePhase::Transferred {
        source: bootstrap,
        context,
        id,
    } = &a.phase
    else {
        panic!("actual transfer");
    };
    let actual = ActualFounderSource {
        creator: &a.original.creator,
        phase: ActualSourcePhase::Transferred {
            source: bootstrap,
            key: &key,
            context,
            id: *id,
        },
    };
    let result = store.with_original_archive_founder(
        &actual,
        a.original.plan.as_ref().unwrap(),
        |admission| {
            let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
            store.prepare_original_source_terminal(
                &a.original.creator.original,
                &vacancy,
                a.registered.as_ref().unwrap(),
                b.saved_files.as_ref().unwrap(),
            )
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(control(&store, &a, ".intent")).unwrap(), before);
    assert_eq!(decode(&store, &a).io_spent, 0);
}

#[test]
fn original_close_at_final_terminal_barrier_cannot_issue_a_saved_result() {
    let (_path, store, mut rng) = fixture(0xe050);
    let mut source = owner(&store, &mut rng, 106);
    source.save_source_files(&store, &mut rng).unwrap();
    let revoker = source.original.creator.original.revoker();
    *store.founder_before_source_prepared_saved.lock().unwrap() =
        Some(Box::new(move || revoker.revoke()));
    assert!(source.save_source_terminal(&store, &mut rng).is_err());
    assert!(source.saved_terminal.is_none());
    assert_eq!(decode(&store, &source).io_spent, 1);
    assert!(source.save_source_terminal(&store, &mut NeverRng).is_err());
}

#[test]
fn equal_foreign_terminal_seed_and_unknown_stage_are_preserved() {
    for replace_seed in [false, true] {
        let (path, store, mut rng) = fixture(0xe060 + u64::from(replace_seed));
        let mut source = owner(&store, &mut rng, 107);
        source.save_source_files(&store, &mut rng).unwrap();
        fault(&store, if replace_seed { 153 } else { 150 });
        assert!(source.save_source_terminal(&store, &mut rng).is_err());
        let destination;
        let preserved;
        if replace_seed {
            destination = control(&store, &source, ".intent-seed");
            preserved = fs::read(&destination).unwrap();
            fs::rename(
                &destination,
                path.path().join("held-original-terminal-seed"),
            )
            .unwrap();
            fs::write(&destination, &preserved).unwrap();
        } else {
            destination = control(&store, &source, ".intent-stage");
            preserved = b"foreign terminal stage".to_vec();
            fs::write(&destination, &preserved).unwrap();
        }
        fault(&store, 0);
        assert!(source.save_source_terminal(&store, &mut NeverRng).is_err());
        assert_eq!(fs::read(destination).unwrap(), preserved);
        assert!(source.saved_terminal.is_none());
    }
}

#[test]
fn final_saved_publication_requires_candidate_not_the_genuine_old_pending_parent() {
    let (path, store, mut rng) = fixture(0xe070);
    let mut source = owner(&store, &mut rng, 108);
    source.save_source_files(&store, &mut rng).unwrap();
    let current = control(&store, &source, ".intent");
    let parent = fs::read(&current).unwrap();
    // Keep a link to the ACTUAL old parent object. Merely rewriting parent bytes into the
    // terminal's new inode would already fail the held-object check and miss this boundary.
    let old_object = path.path().join("actual-old-sp-parent");
    fs::hard_link(&current, &old_object).unwrap();
    let changed = current.clone();
    let displaced = path.path().join("held-unexposed-terminal-current");
    *store.founder_before_source_prepared_saved.lock().unwrap() = Some(Box::new(move || {
        fs::rename(&changed, displaced).unwrap();
        fs::rename(old_object, changed).unwrap();
    }));
    assert!(source.save_source_terminal(&store, &mut rng).is_err());
    assert!(source.saved_terminal.is_none());
    assert_eq!(fs::read(current).unwrap(), parent);
}

#[test]
fn terminal_final_boundary_rejects_byte_equal_page_replacement_after_completed_run_proofs() {
    for name in ["records.bin", "manifest.bin"] {
        let (path, store, mut rng) = fixture(if name == "records.bin" {
            0xe080
        } else {
            0xe081
        });
        let mut source = owner(&store, &mut rng, 109);
        source.save_source_files(&store, &mut rng).unwrap();
        let facts = source.original.plan.as_ref().unwrap().data().unwrap();
        let target = target_name(&facts.scope_domain, &facts.group, &facts.signer).unwrap();
        let output = store
            .dir
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(std::str::from_utf8(&target).unwrap())
            .join(name);
        let bytes = fs::read(&output).unwrap();
        let changed = output.clone();
        let replacement = bytes.clone();
        let displaced = path.path().join("held-original-terminal-page");
        *store.founder_before_source_prepared_saved.lock().unwrap() = Some(Box::new(move || {
            fs::rename(&changed, displaced).unwrap();
            fs::write(&changed, replacement).unwrap();
        }));
        assert!(source.save_source_terminal(&store, &mut rng).is_err());
        assert!(source.saved_terminal.is_none());
        assert_eq!(decode(&store, &source).io_spent, 1);
        assert_eq!(fs::read(&output).unwrap(), bytes);
        assert!(source.save_source_terminal(&store, &mut NeverRng).is_err());
        assert_eq!(fs::read(output).unwrap(), bytes);
    }
}
