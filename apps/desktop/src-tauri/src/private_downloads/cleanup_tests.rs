//! The real Remove, Keep and Save new copy sequences across awaits (review U-H1), driven through
//! [`CleanupPorts`] with real partial files and a deterministic barrier inside the deleting
//! worker. The actor-side transitions are covered by the storage and app crates; what these pin
//! is the native interleaving: no preserve decision while a delete may still happen.
use super::*;
use catcoms_storage::kept::ExportPlan;
use std::sync::mpsc;

/// Upper bound on any barrier wait; a correct run releases each barrier within milliseconds.
const BARRIER: Duration = Duration::from_secs(20);

/// One real `.mewtual-part` with its real journal, in a temporary Downloads folder.
fn partial() -> (tempfile::TempDir, PathBuf, ExportJournal) {
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    let chunk = catcoms_storage::FileRef {
        plaintext_cid: catcoms_storage::Cid::of(b"x"),
        ciphertext_cid: catcoms_storage::Cid::of(b"y"),
        wrapped_key: catcoms_app::SealedBlob {
            nonce: [0; 24],
            ciphertext: vec![0; 48],
        },
        size: 1,
        mime: "application/octet-stream".into(),
    };
    let manifest = catcoms_storage::FileManifest {
        plaintext_cid: catcoms_storage::Cid::of(b"x"),
        total_size: 1,
        mime: "application/octet-stream".into(),
        chunks: vec![chunk],
    };
    let plan = ExportPlan::from_manifest(&manifest).unwrap();
    let file = ExportFile::create(&downloads, "notes.txt", plan, &mut OsCryptoRng).unwrap();
    let journal = file.journal().clone();
    drop(file); // the session ended (Pause, Lock or a crash); the partial stays
    let staging = downloads.join("notes.txt.mewtual-part");
    assert!(staging.exists());
    (root, staging, journal)
}

#[derive(Default)]
struct Record {
    exists: bool,
    deleting: bool,
    intent: u64,
    export: Option<ExportState>,
}

/// In-memory actor state plus a barrier the first delete waits on inside its blocking worker.
struct Fake {
    record: StdMutex<Record>,
    gate: StdMutex<Option<mpsc::Receiver<()>>>,
    entered: StdMutex<Option<mpsc::Sender<()>>>,
    deletes: std::sync::atomic::AtomicUsize,
    /// The presence check never returns (a dead share; folder identity blocks forever).
    presence_hangs: std::sync::atomic::AtomicBool,
}
impl Fake {
    fn new(journal: &ExportJournal) -> (Arc<Self>, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (release, gate) = mpsc::channel();
        let (entered, inside) = mpsc::channel();
        let fake = Arc::new(Self {
            record: StdMutex::new(Record {
                exists: true,
                deleting: false,
                intent: 3,
                export: Some(ExportState::Writing(journal.clone())),
            }),
            gate: StdMutex::new(Some(gate)),
            entered: StdMutex::new(Some(entered)),
            deletes: Default::default(),
            presence_hangs: Default::default(),
        });
        (fake, release, inside)
    }
    fn exists(&self) -> bool {
        self.record.lock().unwrap().exists
    }
}
#[async_trait::async_trait]
impl CleanupPorts for Arc<Fake> {
    async fn cancel(&self) -> Result<Option<ExportState>, String> {
        let mut record = self.record.lock().unwrap();
        if !record.exists {
            return Ok(None);
        }
        if record.export.is_none() {
            record.exists = false;
            return Ok(None);
        }
        record.deleting = true;
        Ok(record.export.clone())
    }
    async fn export_state(&self) -> Result<Option<ExportState>, String> {
        let record = self.record.lock().unwrap();
        Ok(if record.exists && !record.deleting {
            record.export.clone()
        } else {
            None
        })
    }
    fn remover(&self, journal: ExportJournal, published: bool) -> Result<Remover, String> {
        // The first delete waits at the barrier inside the blocking worker, after the claim moved
        // there and before the real `remove_partial`.
        let gate = self.gate.lock().unwrap().take();
        let entered = self.entered.lock().unwrap().take();
        let fake = self.clone();
        Ok(Box::new(move || {
            if let (Some(gate), Some(entered)) = (gate, entered) {
                entered.send(()).unwrap();
                // Bounded so a failing assertion elsewhere can never hang the suite.
                let _ = gate.recv_timeout(BARRIER);
            }
            fake.deletes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            remove_partial(&journal, published, &mut OsCryptoRng)
        }))
    }
    async fn presence(&self, journal: ExportJournal) -> Result<PartialPresence, String> {
        if self
            .presence_hangs
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            std::future::pending::<()>().await;
        }
        Ok(partial_presence(&journal))
    }
    async fn wait(&self, _delay: Duration) {
        // The bound elapses at once; a presence answer that is ready still wins (biased select).
    }
    async fn finish_cleanup(&self) -> Result<(), String> {
        let mut record = self.record.lock().unwrap();
        if !record.exists || !record.deleting {
            return Err("malformed data".into());
        }
        record.exists = false;
        Ok(())
    }
    async fn retire(&self, expected: u64) -> Result<u64, String> {
        let mut record = self.record.lock().unwrap();
        if record.intent != expected || record.deleting || record.export.is_none() {
            return Err(STALE_DOWNLOAD.into());
        }
        record.intent += 1;
        Ok(record.intent)
    }
    async fn abandon(&self, expected: u64) -> Result<u64, String> {
        let mut record = self.record.lock().unwrap();
        if record.intent != expected || record.export.is_none() {
            return Err(STALE_DOWNLOAD.into());
        }
        record.export = None;
        record.intent += 1;
        Ok(record.intent)
    }
}

fn id(byte: u8) -> DownloadId {
    DownloadId::from_bytes([byte; 32])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_partial_cannot_promise_a_file_while_an_older_remove_may_still_delete_it() {
    let (_root, staging, journal) = partial();
    let (fake, release, inside) = Fake::new(&journal);
    let key = id(0xA1);
    // Remove(keepPartial=false) reaches the barrier inside its deleting worker.
    let older = tokio::spawn({
        let fake = fake.clone();
        async move {
            let claim = CleanupClaim::acquire(7, key).unwrap();
            remove_sequence(&fake, claim, false).await
        }
    });
    inside.recv_timeout(BARRIER).unwrap();
    // Keep partly saved file: refused, nothing released, the file and record untouched.
    let keep = CleanupClaim::acquire(7, key);
    assert_eq!(keep.err().as_deref(), Some(CLEANUP_BUSY));
    assert!(staging.exists() && fake.exists());
    release.send(()).unwrap();
    assert_eq!(older.await.unwrap(), Ok(None));
    assert!(!staging.exists() && !fake.exists());
    // Once the delete is over a later Keep reassesses instead of promising anything.
    let claim = CleanupClaim::acquire(7, key).expect("the claim was released");
    assert_eq!(remove_sequence(&fake, claim, true).await, Ok(None));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_remove_still_owns_its_delete_until_the_worker_ends() {
    // Lock, a closed window or a dropped invoke abandons the invocation, not its worker.
    let (_root, staging, journal) = partial();
    let (fake, release, inside) = Fake::new(&journal);
    let key = id(0xA2);
    let older = tokio::spawn({
        let fake = fake.clone();
        async move {
            let claim = CleanupClaim::acquire(7, key).unwrap();
            remove_sequence(&fake, claim, false).await
        }
    });
    inside.recv_timeout(BARRIER).unwrap();
    older.abort();
    let _ = older.await;
    assert_eq!(
        CleanupClaim::acquire(7, key).err().as_deref(),
        Some(CLEANUP_BUSY),
        "the deleting worker still holds the claim after its invocation is gone"
    );
    release.send(()).unwrap();
    // The worker finishes its delete; the record stays Deleting because nothing finished it.
    let mut spins = 0u32;
    let claim = loop {
        if let Ok(claim) = CleanupClaim::acquire(7, key) {
            break claim;
        }
        spins += 1;
        assert!(spins < 5_000_000, "the worker never released its claim");
        tokio::task::yield_now().await;
    };
    assert!(!staging.exists());
    assert!(
        fake.exists(),
        "the encrypted record waits for a finished cleanup"
    );
    // Keep now: the file is already gone, and Keep says so instead of reporting it kept.
    assert_eq!(
        remove_sequence(&fake, claim, true).await,
        Ok(Some(ALREADY_REMOVED))
    );
    assert!(!fake.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_partial_releases_without_deleting_when_nothing_else_owns_the_cleanup() {
    let (_root, staging, journal) = partial();
    let (fake, _release, _inside) = Fake::new(&journal);
    let claim = CleanupClaim::acquire(7, id(0xA3)).unwrap();
    assert_eq!(remove_sequence(&fake, claim, true).await, Ok(None));
    assert!(staging.exists(), "the kept partial is untouched");
    assert!(!fake.exists());
    assert_eq!(fake.deletes.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_partial_still_releases_when_its_presence_check_hangs() {
    // Re-review 2 MEDIUM: Keep is the escape from a stuck removal. A presence check that never
    // returns must not hold the claim (and the Removing record's quota) forever.
    let (_root, staging, journal) = partial();
    let (fake, _release, _inside) = Fake::new(&journal);
    fake.presence_hangs
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let claim = CleanupClaim::acquire(7, id(0xA6)).unwrap();
    let kept = tokio::time::timeout(BARRIER, remove_sequence(&fake, claim, true))
        .await
        .expect("Keep finishes although the check hangs");
    assert_eq!(kept, Ok(None), "no notice promises anything");
    assert!(!fake.exists(), "the record is released");
    assert!(staging.exists(), "the partial is left for the user");
    assert_eq!(fake.deletes.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        CleanupClaim::acquire(7, id(0xA6)).is_ok(),
        "the claim is released"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_presence_is_unknown_without_touching_the_disk_when_every_slot_is_busy() {
    // Re-review 2 MEDIUM: four stuck Removes, of any downloads, hold every cleanup slot. Keep's
    // check answers Unknown at once (no promise) instead of Busy, and reads nothing.
    // The test owns its pool (re-review 3 nit), so other tests using the process-wide one can
    // never make it flaky.
    let (_root, staging, journal) = partial();
    let pool = Arc::new(tokio::sync::Semaphore::new(1));
    let held = pool.clone().try_acquire_owned().unwrap();
    assert_eq!(
        presence_in(pool.clone(), journal.clone()).await,
        Ok(PartialPresence::Unknown)
    );
    assert!(staging.exists());
    drop(held);
    assert_eq!(
        presence_in(pool, journal).await,
        Ok(PartialPresence::Present)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_new_copy_retires_the_old_journal_before_deleting_and_excludes_keep() {
    let (_root, staging, journal) = partial();
    let (fake, release, inside) = Fake::new(&journal);
    let key = id(0xA4);
    let older = tokio::spawn({
        let fake = fake.clone();
        async move {
            let claim = CleanupClaim::acquire(7, key).unwrap();
            abandon_sequence(&fake, claim, 3).await
        }
    });
    inside.recv_timeout(BARRIER).unwrap();
    {
        let record = fake.record.lock().unwrap();
        assert_eq!(
            record.intent, 4,
            "the intent moved before any external delete"
        );
        assert!(
            record.export.is_some(),
            "the journal stays durable until the delete is settled"
        );
    }
    // A delayed Resume click carries the old intent and can no longer act.
    assert!(fake.retire(3).await.is_err());
    assert_eq!(
        CleanupClaim::acquire(7, key).err().as_deref(),
        Some(CLEANUP_BUSY),
        "Keep partly saved file cannot interleave"
    );
    release.send(()).unwrap();
    assert_eq!(older.await.unwrap(), Ok((5, None)));
    assert!(!staging.exists());
    assert!(fake.record.lock().unwrap().export.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_save_new_copy_deletes_nothing() {
    let (_root, staging, journal) = partial();
    let (fake, _release, _inside) = Fake::new(&journal);
    fake.record.lock().unwrap().intent = 9; // paused or resumed since the click
    let claim = CleanupClaim::acquire(7, id(0xA5)).unwrap();
    assert_eq!(
        abandon_sequence(&fake, claim, 3).await,
        Err(STALE_DOWNLOAD.into())
    );
    assert!(staging.exists());
    assert_eq!(fake.deletes.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_takes_the_cleanup_claim_only_around_its_intent_change() {
    // Re-review 2 LOW 2, narrowed by re-review 3 MEDIUM (an intentional contract change): while
    // Remove, Keep or Save new copy holds the claim, a Resume must not commit its intent change
    // (and so cannot go on to reopen and lock a partial a delete worker is about to remove). But
    // its destination preflight, whose reads can hang on a dead share, runs without the claim, so
    // a hung Resume can never make the cleanups report "still being removed".
    let key = id(0xA7);
    let committed = std::sync::atomic::AtomicBool::new(false);
    let commit = || async {
        committed.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    };
    let held = CleanupClaim::acquire(1, key).unwrap();
    assert_eq!(
        plain_resume_commit(1, key, commit()).await.err().as_deref(),
        Some(CLEANUP_BUSY)
    );
    assert!(
        !committed.load(std::sync::atomic::Ordering::SeqCst),
        "no intent change"
    );
    drop(held);
    assert_eq!(plain_resume_commit(1, key, commit()).await, Ok(()));
    assert!(committed.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        CleanupClaim::acquire(1, key).is_ok(),
        "Resume does not keep the claim"
    );

    // The preflight is not excluded: with a cleanup holding the claim, prepare_resume still
    // reaches the actor (this empty one then refuses it for its own reason, not Busy).
    let state = AppState::default();
    let (row, task, drain) = crate::leaving::tests::running(943);
    let actor = row.actor.clone();
    let (_cancel, watch) = tokio::sync::watch::channel(false);
    let origin = RequestCancellation::new(watch, None);
    let held = CleanupClaim::acquire(1, key).unwrap();
    let reached = prepare_resume(&state, 1, 0, 0, &actor, &origin, key, 4, false, false).await;
    assert!(reached.is_err());
    assert_ne!(reached.err().as_deref(), Some(CLEANUP_BUSY));
    drop(held);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_real_resume_commits_its_intent_change_only_without_a_cleanup_claim() {
    // Final review L2: a real Download record whose preflight passes (Completed, no journal, no
    // remembered place), so prepare_resume reaches its intent change. With a cleanup holding the
    // claim it must be Busy and leave the intent alone; without one it commits.
    let state = AppState::default();
    let vault = tempfile::tempdir().unwrap();
    *state.store.lock().await = Some(
        catcoms_app::store::ServerStore::open(vault.path(), b"resume claim", &mut OsCryptoRng)
            .unwrap(),
    );
    *state.session_resumable.lock().await = true;
    let (row, task, drain, cid) =
        crate::leaving::tests::running_with_file(944, &state, b"a resumable private copy").await;
    let actor = row.actor.clone();
    state.servers.lock().await.insert(1, row);
    let (_cancel, watch) = tokio::sync::watch::channel(false);
    let origin = RequestCancellation::new(watch, None);
    let ready = actor
        .acquire_download(cid, None, origin.clone())
        .await
        .expect("the file is local, so the private copy completes at once");
    let key = ready.id();
    drop(ready);
    let intent = || async {
        actor
            .downloads()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == key)
            .unwrap()
            .state
            .intent
    };
    let before = intent().await;
    let held = CleanupClaim::acquire(1, key).unwrap();
    let refused = prepare_resume(
        &state, 1, 944, 0, &actor, &origin, key, before, false, false,
    )
    .await;
    assert_eq!(refused.err().as_deref(), Some(CLEANUP_BUSY));
    assert_eq!(intent().await, before, "no intent change under a cleanup");
    drop(held);
    let resumed = prepare_resume(
        &state, 1, 944, 0, &actor, &origin, key, before, false, false,
    )
    .await
    .expect("no cleanup in flight: Resume commits");
    assert!(resumed.saved.is_none());
    assert_eq!(intent().await, before + 1);
    assert!(
        CleanupClaim::acquire(1, key).is_ok(),
        "Resume released the claim"
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recovered_save_clears_only_the_record_it_recovered() {
    // Final review L1: the Resume preflight recovered a published save; before its cleanup ran, a
    // Save new copy from another window retired and forgot that journal and is now re-saving the
    // same record (or still holds the claim). The late cleanup must leave that record and its run
    // alone and report no failure (which would become a false "could not be removed" warning).
    let (_root, staging, journal) = partial();
    // A real published save: the final name is a second name of the same file. (PR51 M2: a
    // Published record whose final name is missing keeps its staging name as the only copy, so
    // this fixture must link the final name for "our staging name goes" to hold.)
    let saved = staging.with_file_name("notes.txt");
    std::fs::hard_link(&staging, &saved).unwrap();
    let (fake, release, _inside) = Fake::new(&journal);
    drop(release); // no barrier: the one delete below runs straight through
    fake.record.lock().unwrap().export = Some(ExportState::Published(journal.clone()));
    let key = id(0xA8);
    // The record moved on (its observation was retired by the Save new copy).
    let claim = CleanupClaim::acquire(7, key);
    assert_eq!(recovered_cleanup(&fake, claim, || false).await, Ok(()));
    assert!(
        fake.exists() && !fake.record.lock().unwrap().deleting,
        "the re-saving record survives"
    );
    // Another cleanup holds the claim right now: leave it to that cleanup.
    let other = CleanupClaim::acquire(7, key).unwrap();
    assert_eq!(
        recovered_cleanup(&fake, CleanupClaim::acquire(7, key), || true).await,
        Ok(())
    );
    assert!(fake.exists() && !fake.record.lock().unwrap().deleting);
    drop(other);
    assert_eq!(fake.deletes.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Still the recovered record: it is cleared as before (our staging name goes, the record too).
    assert_eq!(
        recovered_cleanup(&fake, CleanupClaim::acquire(7, key), || true).await,
        Ok(())
    );
    assert!(!fake.exists());
    assert!(!staging.exists());
    assert!(saved.exists(), "the saved file is never deleted");
}

/// PR51 M2: clearing a saved row whose final name is gone (lost to a power failure that a folder
/// flush could not prevent, or moved) must never delete the staging name, which is then the only
/// copy of the saved file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_a_saved_row_keeps_the_only_copy_when_its_final_name_is_gone() {
    let (_root, staging, journal) = partial();
    let (fake, release, _inside) = Fake::new(&journal);
    drop(release);
    fake.record.lock().unwrap().export = Some(ExportState::Published(journal.clone()));
    let claim = CleanupClaim::acquire(7, id(0xA9)).unwrap();
    let notice = remove_sequence(&fake, claim, false).await.unwrap();
    assert_eq!(notice, PartialRemoval::KeptOnlyCopy.notice(true));
    assert!(
        notice.unwrap().contains(".mewtual-part"),
        "says where it is"
    );
    assert!(staging.exists(), "the only copy is kept");
    assert!(
        !fake.exists(),
        "the row is cleared, never shown as saved again"
    );
}

#[test]
fn claims_are_per_download_and_released_by_the_last_holder() {
    let first = CleanupClaim::acquire(7, id(0xB1)).unwrap();
    let other = CleanupClaim::acquire(7, id(0xB2)).expect("another download is independent");
    let elsewhere = CleanupClaim::acquire(8, id(0xB1)).expect("another server is independent");
    let worker = first.clone();
    drop(first);
    assert!(
        CleanupClaim::acquire(7, id(0xB1)).is_err(),
        "a clone still holds it"
    );
    drop(worker);
    assert!(CleanupClaim::acquire(7, id(0xB1)).is_ok());
    drop((other, elsewhere));
}
