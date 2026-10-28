//! The saved-file check (user decision, 23 October) through [`SavedCheckPorts`]: each outcome,
//! the bounded wait, and the fences that keep a stale answer from relabelling a newer row.
use super::*;
use catcoms_storage::kept::ExportPlan;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

/// A genuine Published download and its real UI/server state. Only the final
/// platform launch is injected; journal publication and authorization are real.
struct RevealCase {
    root: tempfile::TempDir,
    state: AppState,
    actor: ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
    id: DownloadId,
    intent: u64,
    journal: ExportJournal,
    path: PathBuf,
    _ready: DownloadReady,
    _signal: tokio::sync::watch::Sender<bool>,
}
impl RevealCase {
    async fn new() -> Self {
        use rand_core::SeedableRng;
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await = Some(
            ServerStore::open(
                root.path(),
                b"reveal",
                &mut rand_chacha::ChaCha20Rng::seed_from_u64(51_077),
            )
            .unwrap(),
        );
        *state.session_resumable.lock().await = true;
        let (entry, task, drain, cid) =
            crate::leaving::tests::running_with_file(51_077, &state, b"published private file")
                .await;
        let actor = entry.actor.clone();
        state.servers.lock().await.insert(1, entry);
        let (signal, watch) = tokio::sync::watch::channel(false);
        let origin = RequestCancellation::new(watch, None);
        let ready = actor
            .acquire_download(cid, None, origin.clone())
            .await
            .unwrap()
            .with_export_custody(Arc::new(()));
        let folder = root.path().join("Downloads");
        catcoms_storage::kept::create_folder(&folder).unwrap();
        let mut file = ExportFile::create(
            &folder,
            "private.txt",
            ready.export_plan().unwrap(),
            &mut OsCryptoRng,
        )
        .unwrap();
        actor
            .download_export_transition(
                ready.clone(),
                DownloadExportTransition::Begin {
                    journal: file.journal().clone(),
                    presence: file.presence(),
                },
                origin.clone(),
            )
            .await
            .unwrap();
        while let Some(index) = file.next_index() {
            let bytes = actor
                .read_download_chunk(ready.clone(), index, origin.clone())
                .await
                .unwrap()
                .into_bytes();
            file.accept(index, &bytes).unwrap();
        }
        let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
            panic!("verified plaintext must be ready");
        };
        actor
            .download_export_transition(
                ready.clone(),
                DownloadExportTransition::Checkpoint {
                    expected: file.journal().clone(),
                    next: journal.clone(),
                },
                origin.clone(),
            )
            .await
            .unwrap();
        file.checkpointed(journal);
        let path = file.link().unwrap();
        let journal = file.published_journal();
        actor
            .download_export_transition(
                ready.clone(),
                DownloadExportTransition::Publish {
                    expected: journal.clone(),
                },
                origin,
            )
            .await
            .unwrap();
        file.finish(&mut OsCryptoRng).unwrap();
        let status = actor
            .downloads()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == ready.id())
            .unwrap();
        Self {
            root,
            state,
            actor,
            task,
            drain,
            id: status.id,
            intent: status.state.intent,
            journal,
            path,
            _ready: ready,
            _signal: signal,
        }
    }
    fn request(&self, generation: u64) -> ActorSavedCheck<'_> {
        ActorSavedCheck {
            state: &self.state,
            server: 1,
            instance: 51_077,
            generation,
            actor: &self.actor,
            id: self.id,
        }
    }
    async fn finish(self) {
        self.state.servers.lock().await.clear();
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.await.unwrap();
        assert!(self.path.starts_with(self.root.path()));
    }
}

async fn poll_pending<F: Future>(mut future: std::pin::Pin<&mut F>) -> bool {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx).is_pending())).await
}

#[tokio::test]
async fn download_reveal_rejects_lock_registered_during_the_final_registry_wait() {
    let case = RevealCase::new().await;
    let servers = case.state.servers.lock().await;
    let request = case.request(0);
    let called = AtomicBool::new(false);
    let mut reveal = Box::pin(reveal_download_in_session(
        &request,
        &case.path,
        case.intent,
        &case.journal,
        |_| {
            called.store(true, AtomicOrdering::Release);
            Ok(())
        },
    ));
    // Real actor reads may first suspend; a queued observation runs behind them.
    assert!(poll_pending(reveal.as_mut()).await);
    case.actor.downloads().await.unwrap();
    assert!(poll_pending(reveal.as_mut()).await);
    case.actor.download_export_state(case.id).await.unwrap();
    assert!(poll_pending(reveal.as_mut()).await);
    assert!(case.state.ui_session_commit.try_lock().is_err());
    let mut lock = Box::pin(lock_session_inner(&case.state, None));
    assert!(poll_pending(lock.as_mut()).await);
    drop(servers);
    assert_eq!(reveal.await, Err(RevealError::Stale));
    lock.await.unwrap();
    assert!(!called.load(AtomicOrdering::Acquire));
    drop(request);
    case.finish().await;
}

#[tokio::test]
async fn download_reveal_holds_commit_and_masks_platform_errors() {
    let case = RevealCase::new().await;
    let request = case.request(0);
    let called = AtomicBool::new(false);
    let result =
        reveal_download_in_session(&request, &case.path, case.intent, &case.journal, |path| {
            assert!(case.state.ui_session_commit.try_lock().is_err());
            assert!(case.state.servers.try_lock().is_err());
            called.store(true, AtomicOrdering::Release);
            Err(format!("platform error at {}", path.display()))
        })
        .await;
    assert_eq!(result, Err(RevealError::Launch));
    assert_eq!(result.unwrap_err().message(), REVEAL_FAILED);
    assert!(called.load(AtomicOrdering::Acquire));
    // A row removed after the outer sequence read is not final reveal authority.
    case.actor
        .finish_download_export(case._ready.clone())
        .await
        .unwrap();
    assert_eq!(
        reveal_download_in_session(
            &request,
            &case.path,
            case.intent,
            &case.journal,
            |_| panic!("a cleared download must never launch")
        )
        .await,
        Err(RevealError::Stale)
    );
    drop(request);
    case.finish().await;
}

/// Two distinct real journals (two partials in one temporary Downloads folder).
fn journals() -> (tempfile::TempDir, ExportJournal, ExportJournal) {
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
    let plan = || ExportPlan::from_manifest(&manifest).unwrap();
    let first = ExportFile::create(&downloads, "notes.txt", plan(), &mut OsCryptoRng).unwrap();
    let second = ExportFile::create(&downloads, "notes.txt", plan(), &mut OsCryptoRng).unwrap();
    let journals = (first.journal().clone(), second.journal().clone());
    drop((first, second));
    (root, journals.0, journals.1)
}

/// Scripted record reads, a scripted (or never-answering) presence check and a scripted session.
struct Fake {
    /// One answer per `saved()` call, in order.
    saved: StdMutex<VecDeque<Option<(u64, ExportJournal)>>>,
    /// `None`: the check never answers (a dead share).
    presence: StdMutex<Option<Result<SavedPresence, String>>>,
    current: AtomicBool,
    waited: AtomicBool,
}
impl Fake {
    fn new(
        saved: Vec<Option<(u64, ExportJournal)>>,
        presence: Option<Result<SavedPresence, String>>,
    ) -> Self {
        Self {
            saved: StdMutex::new(saved.into()),
            presence: StdMutex::new(presence),
            current: AtomicBool::new(true),
            waited: AtomicBool::new(false),
        }
    }
}
#[async_trait::async_trait]
impl SavedCheckPorts for Fake {
    async fn saved(&self) -> Result<Option<(u64, ExportJournal)>, String> {
        Ok(self
            .saved
            .lock()
            .unwrap()
            .pop_front()
            .expect("an answer for each read"))
    }
    async fn presence(&self, _journal: ExportJournal) -> Result<SavedPresence, String> {
        let answer = self.presence.lock().unwrap().clone();
        match answer {
            Some(answer) => answer,
            None => std::future::pending().await,
        }
    }
    async fn wait(&self, _delay: Duration) {
        // The bound elapses at once; a presence answer that is ready still wins (biased select).
        self.waited.store(true, AtomicOrdering::SeqCst);
    }
    async fn current(&self) -> bool {
        self.current.load(AtomicOrdering::SeqCst)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_outcome_of_a_saved_file_check_is_reported_as_seen() {
    let (_root, journal, _) = journals();
    let saved = || Some((7, journal.clone()));
    for (presence, expected) in [
        (Ok(SavedPresence::Ours), SavedCheck::Present),
        (Ok(SavedPresence::Missing), SavedCheck::Missing),
        (Ok(SavedPresence::Replaced), SavedCheck::Replaced),
        (Ok(SavedPresence::Unknown), SavedCheck::Unknown),
        (Err("worker stopped".into()), SavedCheck::Unknown),
    ] {
        let fake = Fake::new(vec![saved(), saved()], Some(presence.clone()));
        assert_eq!(
            saved_check_sequence(&fake, 7).await,
            Ok(expected),
            "{presence:?}"
        );
    }
    // The renderer sees one of four words, never a path.
    for (check, word) in [
        (SavedCheck::Present, "\"present\""),
        (SavedCheck::Missing, "\"missing\""),
        (SavedCheck::Replaced, "\"replaced\""),
        (SavedCheck::Unknown, "\"unknown\""),
    ] {
        assert_eq!(serde_json::to_string(&check).unwrap(), word);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_check_that_does_not_answer_in_time_is_unknown_not_saved_or_missing() {
    let (_root, journal, _) = journals();
    let fake = Fake::new(
        vec![Some((7, journal.clone())), Some((7, journal.clone()))],
        None,
    );
    let answered = tokio::time::timeout(Duration::from_secs(20), saved_check_sequence(&fake, 7))
        .await
        .expect("the bounded wait ends the check");
    assert_eq!(answered, Ok(SavedCheck::Unknown));
    assert!(fake.waited.load(AtomicOrdering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_check_never_relabels_a_newer_row() {
    let (_root, journal, other) = journals();
    let missing = || Some(Ok(SavedPresence::Missing));
    // The row changed before the check started: a newer revision, no longer Saved, or gone.
    for before in [Some((8, journal.clone())), None] {
        let fake = Fake::new(vec![before], missing());
        assert_eq!(
            saved_check_sequence(&fake, 7).await,
            Err(STALE_DOWNLOAD.into())
        );
    }
    // The row changed while the check ran: Save another copy (new revision), a different saved
    // journal, or Clear (gone). The answer about the old file is dropped.
    for after in [Some((8, journal.clone())), Some((7, other.clone())), None] {
        let fake = Fake::new(vec![Some((7, journal.clone())), after], missing());
        assert_eq!(
            saved_check_sequence(&fake, 7).await,
            Err(STALE_DOWNLOAD.into())
        );
    }
    // Lock, unlock or a server replacement while it ran.
    let fake = Fake::new(
        vec![Some((7, journal.clone())), Some((7, journal.clone()))],
        missing(),
    );
    fake.current.store(false, AtomicOrdering::SeqCst);
    assert_eq!(
        saved_check_sequence(&fake, 7).await,
        Err("The download is no longer active.".into())
    );
}

/// "Show in folder" (Transfers UX): the saved-check fake plus a scripted locate and a recorded
/// reveal.
struct RevealFake {
    check: Fake,
    /// `None`: the check never answers.
    located: StdMutex<Option<Result<PathBuf, SavedPresence>>>,
    revealed: StdMutex<Vec<PathBuf>>,
    reveal_fails: AtomicBool,
}
impl RevealFake {
    fn new(
        saved: Vec<Option<(u64, ExportJournal)>>,
        located: Option<Result<PathBuf, SavedPresence>>,
    ) -> Self {
        Self {
            check: Fake::new(saved, None),
            located: StdMutex::new(located),
            revealed: StdMutex::new(Vec::new()),
            reveal_fails: AtomicBool::new(false),
        }
    }
}
#[async_trait::async_trait]
impl SavedCheckPorts for RevealFake {
    async fn saved(&self) -> Result<Option<(u64, ExportJournal)>, String> {
        self.check.saved().await
    }
    async fn presence(&self, journal: ExportJournal) -> Result<SavedPresence, String> {
        self.check.presence(journal).await
    }
    async fn wait(&self, delay: Duration) {
        self.check.wait(delay).await;
    }
    async fn current(&self) -> bool {
        self.check.current().await
    }
}
#[async_trait::async_trait]
impl SavedRevealPorts for RevealFake {
    async fn locate(
        &self,
        _journal: ExportJournal,
    ) -> Result<Result<PathBuf, SavedPresence>, String> {
        let answer = self.located.lock().unwrap().clone();
        match answer {
            Some(answer) => Ok(answer),
            None => std::future::pending().await,
        }
    }
    async fn reveal(
        &self,
        path: &Path,
        _intent: u64,
        _journal: &ExportJournal,
    ) -> Result<(), RevealError> {
        if self.reveal_fails.load(AtomicOrdering::SeqCst) {
            return Err(RevealError::Launch);
        }
        self.revealed.lock().unwrap().push(path.to_owned());
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_in_folder_reveals_only_a_file_proven_ours_and_never_returns_its_path() {
    let (root, journal, _) = journals();
    let path = root.path().join("Downloads").join("notes.txt");
    let saved = || Some((7, journal.clone()));
    let fake = RevealFake::new(vec![saved(), saved()], Some(Ok(path.clone())));
    assert_eq!(reveal_saved_sequence(&fake, 7).await, Ok(()));
    assert_eq!(*fake.revealed.lock().unwrap(), vec![path.clone()]);
    // Each refusal is a fixed sentence with no path, and nothing is revealed.
    for (located, message) in [
        (Err(SavedPresence::Missing), REVEAL_MISSING),
        (Err(SavedPresence::Replaced), REVEAL_REPLACED),
        (Err(SavedPresence::Unknown), REVEAL_UNCHECKED),
    ] {
        let fake = RevealFake::new(vec![saved(), saved()], Some(located));
        assert_eq!(reveal_saved_sequence(&fake, 7).await, Err(message.into()));
        assert!(fake.revealed.lock().unwrap().is_empty());
        assert!(!message.contains("Downloads") && !message.contains("notes"));
    }
    // A file manager that cannot start reports the fixed sentence, not the OS error with a path.
    let fake = RevealFake::new(vec![saved(), saved()], Some(Ok(path)));
    fake.reveal_fails.store(true, AtomicOrdering::SeqCst);
    assert_eq!(
        reveal_saved_sequence(&fake, 7).await,
        Err(REVEAL_FAILED.into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_in_folder_is_bounded_and_fenced_like_the_saved_check() {
    let (root, journal, other) = journals();
    let path = root.path().join("Downloads").join("notes.txt");
    // A check that never answers ends with the bounded wait and reveals nothing.
    let fake = RevealFake::new(
        vec![Some((7, journal.clone())), Some((7, journal.clone()))],
        None,
    );
    let answered = tokio::time::timeout(Duration::from_secs(20), reveal_saved_sequence(&fake, 7))
        .await
        .expect("the bounded wait ends it");
    assert_eq!(answered, Err(REVEAL_UNCHECKED.into()));
    assert!(fake.revealed.lock().unwrap().is_empty());
    // A row that changed before or while it ran, or Lock: nothing is revealed.
    for reads in [
        vec![Some((8, journal.clone()))],
        vec![None],
        vec![Some((7, journal.clone())), Some((8, journal.clone()))],
        vec![Some((7, journal.clone())), Some((7, other.clone()))],
        vec![Some((7, journal.clone())), None],
    ] {
        let fake = RevealFake::new(reads, Some(Ok(path.clone())));
        assert_eq!(
            reveal_saved_sequence(&fake, 7).await,
            Err(STALE_DOWNLOAD.into())
        );
        assert!(fake.revealed.lock().unwrap().is_empty());
    }
    let fake = RevealFake::new(
        vec![Some((7, journal.clone())), Some((7, journal))],
        Some(Ok(path)),
    );
    fake.check.current.store(false, AtomicOrdering::SeqCst);
    assert_eq!(
        reveal_saved_sequence(&fake, 7).await,
        Err("The download is no longer active.".into())
    );
    assert!(fake.revealed.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_production_check_is_bounded_and_read_only() {
    let (root, journal, _) = journals();
    let downloads = root.path().join("Downloads");
    let before = std::fs::read_dir(&downloads).unwrap().count();
    // Decisions review M1: with the only slot held by an earlier check, a later check waits for
    // it (holding no thread) and then gives a real answer instead of an immediate "unknown".
    let one = Arc::new(tokio::sync::Semaphore::new(1));
    let held = one.clone().try_acquire_owned().unwrap();
    let waiting = tokio::spawn(saved_presence_in(one.clone(), journal.clone()));
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished(), "it waits for the slot");
    drop(held);
    // The real read-only check (this journal never got its final name).
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(20), waiting)
            .await
            .expect("answers once the slot is free")
            .unwrap(),
        Ok(SavedPresence::Missing)
    );
    assert_eq!(one.available_permits(), 1, "the slot is returned");
    // The sequence bounds that wait like the check: a pool that never frees answers "unknown".
    let never = Arc::new(tokio::sync::Semaphore::new(0));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), saved_presence_in(never, journal))
            .await
            .is_err(),
        "without the sequence's bound it would wait"
    );
    assert_eq!(
        std::fs::read_dir(&downloads).unwrap().count(),
        before,
        "nothing created or removed"
    );
}
