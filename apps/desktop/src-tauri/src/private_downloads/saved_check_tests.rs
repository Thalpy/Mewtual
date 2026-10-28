//! The saved-file check (user decision, 23 October) through [`SavedCheckPorts`]: each outcome,
//! the bounded wait, and the fences that keep a stale answer from relabelling a newer row.
use super::*;
use catcoms_storage::kept::ExportPlan;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

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
    fn reveal(&self, path: &Path) -> Result<(), String> {
        if self.reveal_fails.load(AtomicOrdering::SeqCst) {
            return Err(format!(
                "could not start a file manager for {}",
                path.display()
            ));
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
