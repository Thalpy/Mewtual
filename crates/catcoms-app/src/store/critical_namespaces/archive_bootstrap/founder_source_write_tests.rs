//! Genuine original source / physical pass regressions. Authored UNRUN under the user's
//! compilation pause. A source-file result is not scope Current or native activation evidence.
use super::super::super::tests::{created, fixture, NeverRng, SECRET};
use super::super::*;
use crate::store::critical_namespaces::archive_bootstrap::BOOTSTRAP_SOURCE_CUSTODY_FEE;
use crate::store::persistence::OriginalSourceWriteWork;
use catcoms_rt::MemNetwork;
use rand_chacha::ChaCha20Rng;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

type Owner = ArchiveFounderSourcePreparation<MemNetwork, ChaCha20Rng>;
fn owner(store: &ServerStore, rng: &mut ChaCha20Rng, invocation: u8) -> Owner {
    let creator = created(store, rng, invocation);
    let mut plan = creator.into_ram_plan(store).unwrap();
    plan.save_founding_intent(store, rng).unwrap();
    plan.into_source_preparation(store).unwrap()
}
fn control(store: &ServerStore, owner: &Owner) -> PathBuf {
    let name = owner.original.creator.original.data().invocation.name();
    store
        .dir
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(format!("{}.intent", std::str::from_utf8(&name).unwrap()))
}
fn target(store: &ServerStore, owner: &Owner) -> PathBuf {
    let facts = owner.original.plan.as_ref().unwrap().data().unwrap();
    let name = target_name(&facts.scope_domain, &facts.group, &facts.signer).unwrap();
    store
        .dir
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&name).unwrap())
}
fn decode(store: &ServerStore, owner: &Owner) -> codec::IntentData {
    codec::open_intent_data(
        &store.keys.db_key().unwrap(),
        owner.original.creator.original.data(),
        owner.original.plan.as_ref().unwrap().debit_id(),
        &fs::read(control(store, owner)).unwrap(),
    )
    .unwrap()
}
fn fault(store: &ServerStore, point: u8) {
    store
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}
fn prepare(
    store: &ServerStore,
    owner: &mut Owner,
    rng: &mut ChaCha20Rng,
) -> OriginalSourceWriteWork {
    owner.save_source_prepared(store, rng).unwrap();
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let SourcePhase::Transferred {
        source: bootstrap,
        context,
        id,
    } = &owner.phase
    else {
        panic!("actual transfer");
    };
    let source = ActualFounderSource {
        creator: &owner.original.creator,
        phase: ActualSourcePhase::Transferred {
            source: bootstrap,
            key: &key,
            context,
            id: *id,
        },
    };
    store
        .with_original_archive_founder(
            &source,
            owner.original.plan.as_ref().unwrap(),
            |admission| {
                let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                store.prepare_original_bootstrap_source_write(
                    &owner.original.creator.original,
                    &vacancy,
                    owner.registered.as_ref().unwrap(),
                    rng,
                )
            },
        )
        .unwrap()
}

#[test]
fn actual_large_pass_saves_exact_files_after_one_durable_charge_and_lost_reply_is_bounded() {
    let (path, store, mut rng) = fixture(0xff01);
    let mut source = owner(&store, &mut rng, 81);
    let rows = fs::read(path.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
    assert_eq!(
        source.save_source_files(&store, &mut rng).unwrap(),
        ArchiveFounderSourceProgress::SourceFilesSavedUnexposed
    );
    assert_eq!(
        (
            decode(&store, &source).sequence,
            decode(&store, &source).io_spent
        ),
        (3, 1)
    );
    let selected = fs::read(control(&store, &source)).unwrap();
    let target = target(&store, &source);
    let frame = fs::read(target.join("source.bin")).unwrap();
    assert_eq!(fs::read(target.join("source.seed")).unwrap(), frame);
    let SourcePhase::Transferred { context, id, .. } = &source.phase else {
        panic!("actual transfer");
    };
    let opened = catcoms_sync::owner_v2::open_bootstrap_source_data(
        &store.keys.db_key().unwrap(),
        context,
        *id,
        &frame,
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("records.bin")).unwrap(),
        opened.view_data().records_frame
    );
    assert_eq!(
        fs::read(target.join("manifest.bin")).unwrap(),
        opened.view_data().manifest_frame
    );
    // Lose only the caller's local return. Root retains the actual historical writer outcome;
    // its reobservation does not consume a unit or perform another complete file verification.
    source.saved_files = None;
    source.save_source_files(&store, &mut NeverRng).unwrap();
    assert_eq!(fs::read(control(&store, &source)).unwrap(), selected);
    assert_eq!(
        fs::read(path.path().join("chat-admissions.bin")).unwrap(),
        rows
    );
    assert_eq!(
        fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
}

#[test]
fn all_large_faults_keep_original_cipher_and_same_files_but_require_a_new_durable_pass() {
    for point in 160..=169 {
        let (_path, store, mut rng) = fixture(0xff10 + u64::from(point));
        let mut source = owner(&store, &mut rng, 82);
        fault(&store, point);
        assert!(
            source.save_source_files(&store, &mut rng).is_err(),
            "fault {point}"
        );
        assert_eq!(decode(&store, &source).io_spent, 1);
        let retained = store.retained_bootstrap_source_for_test([82; 32]).unwrap();
        let first_seed = fs::File::open(target(&store, &source).join("source.seed")).unwrap();
        let seed_id = catcoms_storage::kept::session_id(&first_seed).unwrap();
        fault(&store, 0);
        source.save_source_files(&store, &mut rng).unwrap();
        assert_eq!(decode(&store, &source).io_spent, 2);
        assert_eq!(
            store.retained_bootstrap_source_for_test([82; 32]).unwrap(),
            retained
        );
        assert!(matches!(
            catcoms_storage::kept::session_at(
                &target(&store, &source).join("source.seed"),
                seed_id
            ),
            catcoms_storage::kept::SessionAt::Ours
        ));
        source.save_source_files(&store, &mut NeverRng).unwrap();
        assert_eq!(decode(&store, &source).io_spent, 2);
    }
}

#[test]
fn ambiguous_small_arm_save_is_repaired_exactly_and_never_revives_its_abandoned_work() {
    for point in 150..=157 {
        let (_path, store, mut rng) = fixture(0xff30 + u64::from(point));
        let mut source = owner(&store, &mut rng, 83);
        source.save_source_prepared(&store, &mut rng).unwrap();
        let retained = store.retained_bootstrap_source_for_test([83; 32]).unwrap();
        fault(&store, point);
        assert!(
            source.save_source_files(&store, &mut rng).is_err(),
            "fault {point}"
        );
        assert!(!target(&store, &source).join("source.seed").exists());
        fault(&store, 0);
        source.save_source_files(&store, &mut rng).unwrap();
        assert_eq!(
            decode(&store, &source).io_spent,
            2,
            "ambiguous arm stays spent {point}"
        );
        assert_eq!(
            store.retained_bootstrap_source_for_test([83; 32]).unwrap(),
            retained
        );
    }
}

#[test]
fn original_three_pass_allowance_never_refills_on_retry_and_retains_complete_debt() {
    let (_path, store, mut rng) = fixture(0xff50);
    let mut source = owner(&store, &mut rng, 84);
    for spent in 1..=3 {
        fault(&store, 162);
        assert!(source.save_source_files(&store, &mut rng).is_err());
        assert_eq!(decode(&store, &source).io_spent, spent);
    }
    let retained = store.retained_bootstrap_source_for_test([84; 32]).unwrap();
    let selected = fs::read(control(&store, &source)).unwrap();
    fault(&store, 0);
    assert!(source.save_source_files(&store, &mut NeverRng).is_err());
    assert_eq!(fs::read(control(&store, &source)).unwrap(), selected);
    assert_eq!(
        store.retained_bootstrap_source_for_test([84; 32]).unwrap(),
        retained
    );
    assert!(target(&store, &source).join("source.seed").exists());
    assert!(!target(&store, &source).join("source.bin").exists());
}

#[test]
fn foreign_first_seed_never_inherits_original_file_ownership() {
    let (_path, store, mut rng) = fixture(0xff60);
    let mut source = owner(&store, &mut rng, 85);
    let work = prepare(&store, &mut source, &mut rng);
    let target = target(&store, &source);
    // No first-seed ownership follows from finding a file, including an empty prefix.
    fs::write(
        target.join("source.seed"),
        b"foreign complete or partial seed",
    )
    .unwrap();
    assert!(work.run().is_err());
    assert_eq!(
        fs::read(target.join("source.seed")).unwrap(),
        b"foreign complete or partial seed"
    );
    assert!(source.save_source_files(&store, &mut rng).is_err());
    assert_eq!(
        fs::read(target.join("source.seed")).unwrap(),
        b"foreign complete or partial seed"
    );
}

#[test]
fn byte_equal_replacement_seed_never_inherits_the_actual_created_file() {
    let (path, store, mut rng) = fixture(0xff61);
    let mut source = owner(&store, &mut rng, 90);
    fault(&store, 162);
    assert!(source.save_source_files(&store, &mut rng).is_err());
    let seed = target(&store, &source).join("source.seed");
    let bytes = fs::read(&seed).unwrap();
    fs::rename(&seed, path.path().join("held-original-large-seed")).unwrap();
    fs::write(&seed, &bytes).unwrap();
    fault(&store, 0);
    assert!(source.save_source_files(&store, &mut rng).is_err());
    assert_eq!(fs::read(seed).unwrap(), bytes);
    assert_eq!(decode(&store, &source).io_spent, 2);
}

struct BlockedIo {
    release: Option<Sender<()>>,
    result: Receiver<Result<(), String>>,
    join: Option<JoinHandle<()>>,
}
impl Drop for BlockedIo {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
impl BlockedIo {
    fn start(work: OriginalSourceWriteWork) -> Self {
        let (ready_tx, ready) = channel();
        let (release, advance) = channel();
        work.set_after_seed(Box::new(move || {
            ready_tx.send(()).unwrap();
            advance.recv_timeout(Duration::from_secs(10)).unwrap();
        }));
        let (done, result) = channel();
        let join = std::thread::spawn(move || {
            let _ = done.send(work.run().map_err(|e| e.to_string()));
        });
        let running = Self {
            release: Some(release),
            result,
            join: Some(join),
        };
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        running
    }
    fn drain(mut self) -> Result<(), String> {
        self.release.take().unwrap().send(()).unwrap();
        let result = self.result.recv_timeout(Duration::from_secs(10)).unwrap();
        self.join.take().unwrap().join().unwrap();
        result
    }
}

#[test]
fn other_group_actual_admission_progresses_while_source_a_holds_file_io_mutex() {
    let (_path, store, mut rng) = fixture(0xff70);
    let mut a = owner(&store, &mut rng, 86);
    // Key/MLS construction is unrelated to the guard-release property. Finish that
    // independent setup before parking A, rather than spending its bounded barrier wait
    // on cryptography when a full suite has many expensive fixtures running in parallel.
    let mut b = owner(&store, &mut rng, 87);
    let running = BlockedIo::start(prepare(&store, &mut a, &mut rng));
    assert!(
        target(&store, &a).join("source.seed").exists(),
        "actual file work preceded barrier"
    );
    // B still performs its real source preparation and journal admission while A holds
    // its output mutex. Those actual AD/pins/canonical/Root acquisitions would deadlock
    // if A had retained a global guard across physical I/O; no stand-in permit is used.
    b.save_source_prepared(&store, &mut rng).unwrap();
    assert_eq!(decode(&store, &b).io_spent, 0);
    running.drain().unwrap();
    a.save_source_files(&store, &mut NeverRng).unwrap();
    assert_eq!(decode(&store, &a).io_spent, 1);
}

#[test]
fn actual_worker_retains_vault_exclusion_after_store_drop_until_file_work_drains() {
    let (path, store, mut rng) = fixture(0xff80);
    let mut source = owner(&store, &mut rng, 88);
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let negative = source.original.creator.original.observer().clone();
    let running = BlockedIo::start(prepare(&store, &mut source, &mut rng));
    let seed = target(&store, &source).join("source.seed");
    let retained = fs::read(&seed).unwrap();
    drop(store);
    drop(source);
    assert!(negative.is_cancelled());
    assert_eq!(
        pool.charged_for_test(),
        BOOTSTRAP_SOURCE_CUSTODY_FEE,
        "only the actual Work's separate cipher/output invoice survives both authority owners"
    );
    assert!(matches!(
        ServerStore::open(path.path(), b"not consulted", &mut rng),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    ));
    assert!(
        running.drain().is_err(),
        "mount loss prevents further positive work"
    );
    assert_eq!(
        pool.charged_for_test(),
        0,
        "last physical Work drains its real invoice"
    );
    // The lock is now available. Legacy grammar rejects the private namespace, independently
    // of vault exclusion; it must no longer report Busy or clean up the retained source seed.
    assert!(matches!(
        ServerStore::open(path.path(), SECRET, &mut rng),
        Err(AppError::Invalid(message)) if message == "critical vault metadata is unavailable or unsupported"
    ));
    assert_eq!(fs::read(seed).unwrap(), retained);
}

#[test]
fn cancelled_original_before_io_cannot_publish_or_revive_across_a_new_generation() {
    let (_path, store, mut rng) = fixture(0xff90);
    let mut source = owner(&store, &mut rng, 89);
    source.save_source_prepared(&store, &mut rng).unwrap();
    // Cache only the pathname used for the negative filesystem assertion. Asking the
    // revoked plan for authoritative DATA must fail, so doing that AFTER cancellation
    // would make this regression panic in its helper instead of checking the real I/O.
    let original_target = target(&store, &source);
    let revoke = source.original.creator.original.revoker();
    source.before_source_io = Some(Box::new(move || revoke.revoke()));
    assert!(source.save_source_files(&store, &mut rng).is_err());
    assert_eq!(decode(&store, &source).io_spent, 1);
    assert!(!original_target.join("source.seed").exists());
    assert!(source.save_source_files(&store, &mut NeverRng).is_err());
}

#[test]
fn lost_prepared_work_stays_spent_and_an_earlier_saved_journal_cannot_become_current() {
    let (_path, store, mut rng) = fixture(0xffa0);
    let mut source = owner(&store, &mut rng, 91);
    source.save_source_prepared(&store, &mut rng).unwrap();
    let old = fs::read(control(&store, &source)).unwrap();
    drop(prepare(&store, &mut source, &mut rng));
    assert_eq!(decode(&store, &source).io_spent, 1);
    assert!(!target(&store, &source).join("source.seed").exists());
    source.save_source_files(&store, &mut rng).unwrap();
    assert_eq!(decode(&store, &source).io_spent, 2);
    source.saved_files = None;
    // The old SP is genuine and its original held File is still in the retained chain.
    // Replacing current CONTENT with that old saved frame must nevertheless refuse.
    fs::write(control(&store, &source), &old).unwrap();
    assert!(source.save_source_files(&store, &mut NeverRng).is_err());
    assert_eq!(fs::read(control(&store, &source)).unwrap(), old);
}

#[test]
fn completed_writer_outcome_refuses_replaced_named_output_without_rewriting_it() {
    let (path, store, mut rng) = fixture(0xffa1);
    let mut source = owner(&store, &mut rng, 92);
    source.save_source_files(&store, &mut rng).unwrap();
    let output = target(&store, &source).join("source.bin");
    let bytes = fs::read(&output).unwrap();
    fs::rename(&output, path.path().join("held-original-large-output")).unwrap();
    fs::write(&output, &bytes).unwrap();
    source.saved_files = None;
    assert!(source.save_source_files(&store, &mut NeverRng).is_err());
    assert_eq!(decode(&store, &source).io_spent, 1);
    assert_eq!(fs::read(output).unwrap(), bytes);
}

#[test]
fn source_open_wait_cannot_lend_stale_authority_to_the_first_page_effect() {
    for replace_target in [false, true] {
        let (path, store, mut rng) = fixture(0xffb0 + u64::from(replace_target));
        let mut source = owner(&store, &mut rng, 93);
        let work = prepare(&store, &mut source, &mut rng);
        let target = target(&store, &source);
        if replace_target {
            let actual_target = target.clone();
            let moved = path.path().join("held-original-target-after-source-open");
            work.set_before_pages(Box::new(move || {
                #[cfg(not(windows))]
                {
                    fs::rename(&actual_target, moved).unwrap();
                    fs::create_dir(&actual_target).unwrap();
                }
                #[cfg(windows)]
                {
                    // Held source Files prevent this directory move on Windows. Do
                    // not weaken the original owner to force an impossible injection.
                    let error = fs::rename(&actual_target, &moved).unwrap_err();
                    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
                    assert!(actual_target.is_dir());
                    assert!(!moved.exists());
                    // Exercise the actual final-source proof at this SAME hook with
                    // a real equal-byte replacement of the held immutable origin File.
                    let origin = actual_target.join("origin");
                    let original = fs::read(&origin).unwrap();
                    fs::rename(&origin, actual_target.join("held-origin-after-open")).unwrap();
                    fs::write(origin, original).unwrap();
                }
            }));
        } else {
            let revoker = source.original.creator.original.revoker();
            work.set_before_pages(Box::new(move || revoker.revoke()));
        }
        assert!(work.run().is_err());
        assert!(!target.join("records.bin").exists());
        assert!(!target.join("manifest.bin").exists());
        assert!(source.save_source_files(&store, &mut NeverRng).is_err());
    }
}

#[test]
fn final_result_proves_output_names_after_all_source_current_and_root_waits() {
    let (path, store, mut rng) = fixture(0xffc0);
    let mut source = owner(&store, &mut rng, 94);
    source.save_source_files(&store, &mut rng).unwrap();
    let output = target(&store, &source).join("manifest.bin");
    let bytes = fs::read(&output).unwrap();
    let changed = output.clone();
    let replacement = bytes.clone();
    let moved = path.path().join("held-original-final-manifest");
    *store.founder_before_source_files_saved.lock().unwrap() = Some(Box::new(move || {
        fs::rename(&changed, moved).unwrap();
        fs::write(&changed, replacement).unwrap();
    }));
    source.saved_files = None;
    assert!(source.save_source_files(&store, &mut NeverRng).is_err());
    assert!(store
        .founder_before_source_files_saved
        .lock()
        .unwrap()
        .is_none());
    assert_eq!(fs::read(output).unwrap(), bytes);
    assert_eq!(decode(&store, &source).io_spent, 1);
}
