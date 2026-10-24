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
