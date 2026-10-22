//! The real native export sequence ([`export_sequence`]) across its awaits, with real files and a
//! fake [`ExportPorts`] that keeps the journal in memory and records every transition. Both
//! reviews asked for this coverage: where the commit guard is held, a Publish failure after the
//! link, RecoverPublished plus staging removal, the prepare/link retry loop, and what `stop()`
//! checkpoints. Actor-side fencing is covered in the app crate.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};

const DATA: [&[u8]; 2] = [b"first chunk of the file", b"second, shorter"];

fn plan(cid: catcoms_storage::Cid) -> ExportPlan {
    let chunk = |size: usize| catcoms_storage::FileRef {
        plaintext_cid: catcoms_storage::Cid::of(b"p"),
        ciphertext_cid: catcoms_storage::Cid::of(b"c"),
        wrapped_key: catcoms_app::SealedBlob {
            nonce: [0; 24],
            ciphertext: vec![0; 48],
        },
        size: size as u64,
        mime: "application/octet-stream".into(),
    };
    ExportPlan::from_manifest(&catcoms_storage::FileManifest {
        plaintext_cid: cid,
        total_size: DATA.iter().map(|chunk| chunk.len() as u64).sum(),
        mime: "application/octet-stream".into(),
        chunks: DATA.iter().map(|data| chunk(data.len())).collect(),
    })
    .unwrap()
}
fn good_plan() -> ExportPlan {
    plan(catcoms_storage::Cid::of(&DATA.concat()))
}

/// What the fake actor saw, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Begin,
    Resume,
    Checkpoint(usize),
    Publish,
    Halt,
    Recover,
}

struct Fake {
    record: StdMutex<Option<ExportState>>,
    events: StdMutex<Vec<Event>>,
    guard: Arc<AtomicBool>,
    live: AtomicBool,
    refuse_publish: AtomicBool,
    /// When Published is recorded, another program opens the staging name without delete
    /// sharing, so the sequence cannot remove it afterwards (Windows).
    hold_staging: AtomicBool,
    held: StdMutex<Option<std::fs::File>>,
    /// Lock lands right after the every-chunk checkpoint, before the commit guard.
    lock_at_publication: AtomicBool,
    /// The session is retired after a step was dispatched but before its worker starts.
    retired_before_worker_start: AtomicBool,
    /// Another program takes the chosen final name right after each publication checkpoint.
    take_final_name: AtomicUsize,
    folder: PathBuf,
    backoffs: AtomicUsize,
    /// The actor refuses Begin (for example the record's remembered choice is not this journal's
    /// place, or the record was replaced meanwhile).
    refuse_begin: AtomicBool,
}
impl Fake {
    fn new(folder: &Path) -> Arc<Self> {
        Arc::new(Self {
            record: StdMutex::new(None),
            events: StdMutex::new(Vec::new()),
            guard: Arc::new(AtomicBool::new(false)),
            live: AtomicBool::new(true),
            refuse_begin: AtomicBool::new(false),
            refuse_publish: AtomicBool::new(false),
            hold_staging: AtomicBool::new(false),
            held: StdMutex::new(None),
            lock_at_publication: AtomicBool::new(false),
            retired_before_worker_start: AtomicBool::new(false),
            take_final_name: AtomicUsize::new(0),
            folder: folder.to_owned(),
            backoffs: AtomicUsize::new(0),
        })
    }
    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }
    fn written_chunks(&self) -> Option<usize> {
        match &*self.record.lock().unwrap() {
            Some(ExportState::Writing(journal)) => Some(journal.written_chunks()),
            _ => None,
        }
    }
    fn published(&self) -> bool {
        matches!(
            &*self.record.lock().unwrap(),
            Some(ExportState::Published(_))
        )
    }
}
struct Guard(Arc<AtomicBool>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(false, AtomicOrdering::SeqCst);
    }
}
#[async_trait::async_trait]
impl ExportPorts for Arc<Fake> {
    async fn export_state(&self) -> Result<Option<ExportState>, String> {
        Ok(self.record.lock().unwrap().clone())
    }
    async fn transition(&self, transition: DownloadExportTransition) -> Result<(), String> {
        let mut record = self.record.lock().unwrap();
        let event = match transition {
            DownloadExportTransition::Begin { journal, .. } => {
                if self.refuse_begin.load(AtomicOrdering::SeqCst) {
                    return Err("Saved download changed.".into());
                }
                assert!(record.is_none());
                *record = Some(ExportState::Writing(journal));
                Event::Begin
            }
            DownloadExportTransition::Resume { journal, .. } => {
                assert_eq!(*record, Some(ExportState::Writing(journal)));
                Event::Resume
            }
            DownloadExportTransition::Checkpoint { expected, next } => {
                assert_eq!(*record, Some(ExportState::Writing(expected)));
                let chunks = next.written_chunks();
                if chunks == DATA.len() && self.lock_at_publication.load(AtomicOrdering::SeqCst) {
                    self.live.store(false, AtomicOrdering::SeqCst);
                }
                if chunks == DATA.len() && self.take_final_name.load(AtomicOrdering::SeqCst) > 0 {
                    self.take_final_name.fetch_sub(1, AtomicOrdering::SeqCst);
                    // The name this checkpoint just persisted is taken before the link.
                    let taken = self.folder.join(next.final_name());
                    std::fs::write(taken, b"someone else's file").unwrap();
                }
                *record = Some(ExportState::Writing(next));
                Event::Checkpoint(chunks)
            }
            DownloadExportTransition::Publish { expected } => {
                assert!(
                    self.guard.load(AtomicOrdering::SeqCst),
                    "Published is recorded only under the session commit guard"
                );
                if self.refuse_publish.swap(false, AtomicOrdering::SeqCst) {
                    return Err("the download is no longer active".into());
                }
                #[cfg(windows)]
                if self.hold_staging.load(AtomicOrdering::SeqCst) {
                    use std::os::windows::fs::OpenOptionsExt;
                    let holder = std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0x1 | 0x2) // read and write sharing, never delete
                        .open(self.folder.join(expected.staging_name()))
                        .unwrap();
                    *self.held.lock().unwrap() = Some(holder);
                }
                *record = Some(ExportState::Published(expected));
                Event::Publish
            }
            DownloadExportTransition::Halt { .. } => Event::Halt,
            DownloadExportTransition::RecoverPublished { expected } => {
                *record = Some(ExportState::Published(expected));
                Event::Recover
            }
        };
        self.events.lock().unwrap().push(event);
        Ok(())
    }
    async fn chunk(&self, index: usize) -> Result<Vec<u8>, String> {
        Ok(DATA[index].to_vec())
    }
    fn live(&self) -> bool {
        self.live.load(AtomicOrdering::SeqCst)
    }
    fn liveness(&self) -> Liveness {
        let fake = self.clone();
        Arc::new(move || {
            fake.live.load(AtomicOrdering::SeqCst)
                && !fake
                    .retired_before_worker_start
                    .load(AtomicOrdering::SeqCst)
        })
    }
    async fn commit(&self) -> Result<Box<dyn Send + '_>, String> {
        if !self.live() {
            return Err("The download is no longer active.".into());
        }
        self.guard.store(true, AtomicOrdering::SeqCst);
        Ok(Box::new(Guard(self.guard.clone())))
    }
    fn progress(&self, _progress: ExportProgress) {}
    async fn backoff(&self, _delay: Duration) {
        self.backoffs.fetch_add(1, AtomicOrdering::SeqCst);
    }
}

fn downloads() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    std::fs::create_dir(&folder).unwrap();
    (root, folder)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_save_checkpoints_before_linking_and_records_published_under_the_guard() {
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    let destination = Destination::Downloads(folder.clone());
    let exported = export_sequence(&fake, good_plan(), &destination, "notes.txt")
        .await
        .unwrap()
        .expect("this folder can carry a journal");
    assert_eq!(exported.path, folder.join("notes.txt"));
    assert!(exported.staging_cleared);
    assert_eq!(std::fs::read(&exported.path).unwrap(), DATA.concat());
    assert!(!folder.join("notes.txt.mewtual-part").exists());
    assert_eq!(
        fake.events(),
        [Event::Begin, Event::Checkpoint(2), Event::Publish],
        "the every-chunk checkpoint is durable before the link"
    );
    assert!(fake.published());
    assert!(
        !fake.guard.load(AtomicOrdering::SeqCst),
        "the guard is released"
    );
}

/// A folder the user picked in Save as, beside (not inside) the Downloads folder.
fn chosen_place(root: &tempfile::TempDir, name: &str) -> (PathBuf, Destination) {
    let folder = root.path().join("Chosen place");
    std::fs::create_dir_all(&folder).unwrap();
    let choice = ChosenDestination::validate(&folder.join(name)).unwrap();
    (folder, Destination::Chosen(Arc::new(choice)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_save_as_choice_saves_at_the_exact_chosen_name_through_the_same_sequence() {
    // Picker review coverage: the chosen place goes through the same journaled sequence
    // (Begin, every-chunk checkpoint, Published under the guard) at the exact name typed.
    let (root, downloads) = downloads();
    let (folder, destination) = chosen_place(&root, "Renamed report.bin");
    let fake = Fake::new(&folder);
    let exported = export_sequence(&fake, good_plan(), &destination, "listed name.bin")
        .await
        .unwrap()
        .expect("an identified chosen folder carries a journal");
    assert_eq!(exported.path, folder.join("Renamed report.bin"));
    assert!(exported.staging_cleared);
    assert_eq!(std::fs::read(&exported.path).unwrap(), DATA.concat());
    assert_eq!(
        fake.events(),
        [Event::Begin, Event::Checkpoint(2), Event::Publish]
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_dir(&downloads).unwrap().count(),
        0,
        "nothing in Downloads"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_begin_at_a_chosen_place_discards_the_new_partial() {
    // The actor refuses Begin (the record's choice moved, or the record was replaced): the empty
    // partial this save just created at the chosen place is removed, nothing is journaled.
    let (root, _downloads) = downloads();
    let (folder, destination) = chosen_place(&root, "report.bin");
    let fake = Fake::new(&folder);
    fake.refuse_begin.store(true, AtomicOrdering::SeqCst);
    assert!(
        export_sequence(&fake, good_plan(), &destination, "report.bin")
            .await
            .is_err()
    );
    assert!(fake.events().is_empty());
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        0,
        "no partial left at the chosen place"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chosen_place_never_redirects_an_existing_journal() {
    // A Download whose record already holds a Writing journal (here in Downloads) is never
    // continued or restarted at a chosen place; nothing is created there.
    let (root, downloads) = downloads();
    let fake = Fake::new(&downloads);
    fake.lock_at_publication.store(true, AtomicOrdering::SeqCst);
    let first = Destination::Downloads(downloads.clone());
    assert!(export_sequence(&fake, good_plan(), &first, "notes.txt")
        .await
        .is_err());
    assert_eq!(fake.written_chunks(), Some(DATA.len()));
    fake.live.store(true, AtomicOrdering::SeqCst);
    let (folder, chosen) = chosen_place(&root, "elsewhere.bin");
    assert_eq!(
        export_sequence(&fake, good_plan(), &chosen, "notes.txt")
            .await
            .err()
            .as_deref(),
        Some(JOURNAL_EXISTS)
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    assert!(
        downloads.join("notes.txt.mewtual-part").exists(),
        "the tracked partial is untouched"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_commit_links_nothing() {
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    // Lock lands after every chunk is written and checkpointed, before the commit guard.
    fake.lock_at_publication.store(true, AtomicOrdering::SeqCst);
    let destination = Destination::Downloads(folder.clone());
    let refused = export_sequence(&fake, good_plan(), &destination, "notes.txt").await;
    assert!(refused.is_err());
    assert!(
        !folder.join("notes.txt").exists(),
        "no final name after Lock"
    );
    assert!(!fake.events().contains(&Event::Publish));
    assert!(
        folder.join("notes.txt.mewtual-part").exists(),
        "the partial stays tracked"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_publish_failure_after_the_link_is_recovered_by_the_next_resume_without_a_copy() {
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    fake.refuse_publish.store(true, AtomicOrdering::SeqCst);
    let destination = Destination::Downloads(folder.clone());
    assert!(
        export_sequence(&fake, good_plan(), &destination, "notes.txt")
            .await
            .is_err()
    );
    // The link happened under the guard; Published was refused (for example Lock won).
    assert!(folder.join("notes.txt").exists());
    assert_eq!(fake.written_chunks(), Some(DATA.len()));
    // The next explicit Resume: reopen finds our final name, verifies it, recovers it and
    // removes only the staging name.
    let exported = export_sequence(&fake, good_plan(), &destination, "notes.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exported.path, folder.join("notes.txt"));
    assert!(exported.staging_cleared);
    assert!(fake.events().ends_with(&[Event::Recover]));
    assert!(fake.published());
    assert!(!folder.join("notes.txt.mewtual-part").exists());
    assert!(!folder.join("notes (1).txt").exists(), "no second copy");
    assert_eq!(std::fs::read(&exported.path).unwrap(), DATA.concat());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_name_taken_before_the_link_retries_with_backoff_and_a_numbered_name() {
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    fake.take_final_name.store(1, AtomicOrdering::SeqCst);
    let destination = Destination::Downloads(folder.clone());
    let exported = export_sequence(&fake, good_plan(), &destination, "notes.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exported.path, folder.join("notes (1).txt"));
    assert_eq!(
        fake.backoffs.load(AtomicOrdering::SeqCst),
        1,
        "one paced retry"
    );
    assert_eq!(
        std::fs::read(folder.join("notes.txt")).unwrap(),
        b"someone else's file",
        "never replaced"
    );
    assert_eq!(std::fs::read(&exported.path).unwrap(), DATA.concat());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publication_retries_are_bounded_and_the_stop_checkpoints_what_is_durable() {
    // Review U-L1: every attempt loses its name; retries stop at the per-chunk bound and the
    // transient failure becomes a stop that checkpoints the (every-chunk) durable progress.
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    fake.take_final_name
        .store(usize::MAX, AtomicOrdering::SeqCst);
    let destination = Destination::Downloads(folder.clone());
    assert!(
        export_sequence(&fake, good_plan(), &destination, "notes.txt")
            .await
            .is_err()
    );
    assert_eq!(
        fake.backoffs.load(AtomicOrdering::SeqCst) as u32,
        LOCAL_RETRIES_PER_CHUNK
    );
    assert!(!fake.events().contains(&Event::Publish));
    assert_eq!(fake.written_chunks(), Some(DATA.len()));
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_staging_name_that_cannot_be_removed_keeps_the_saved_record() {
    // Review L3: the save succeeded, but the second (staging) plaintext name stays because
    // another program holds it. The sequence must say so, so the caller keeps the Published
    // record (and its Clear) instead of releasing an untracked second name.
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    fake.hold_staging.store(true, AtomicOrdering::SeqCst);
    let destination = Destination::Downloads(folder.clone());
    let exported = export_sequence(&fake, good_plan(), &destination, "notes.txt")
        .await
        .unwrap()
        .unwrap();
    assert!(!exported.staging_cleared);
    assert!(fake.published());
    assert!(folder.join("notes.txt.mewtual-part").exists());
    drop(fake.held.lock().unwrap().take());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_queued_before_retirement_writes_nothing_when_its_worker_starts() {
    // The dispatch-time check passes; Lock retires the session while the blocking step waits
    // for a worker. When it starts it must refuse, write nothing and checkpoint nothing.
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    fake.retired_before_worker_start
        .store(true, AtomicOrdering::SeqCst);
    let destination = Destination::Downloads(folder.clone());
    assert!(
        export_sequence(&fake, good_plan(), &destination, "notes.txt")
            .await
            .is_err()
    );
    assert_eq!(fake.events(), [Event::Begin]);
    assert_eq!(
        std::fs::metadata(folder.join("notes.txt.mewtual-part"))
            .unwrap()
            .len(),
        0,
        "no plaintext was written"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_that_fails_verification_stops_without_a_checkpoint() {
    // Review L4: the whole stream was written but does not hash to the manifest's CID.
    let (_root, folder) = downloads();
    let fake = Fake::new(&folder);
    let destination = Destination::Downloads(folder.clone());
    let wrong = plan(catcoms_storage::Cid::of(b"another file"));
    assert!(export_sequence(&fake, wrong, &destination, "notes.txt")
        .await
        .is_err());
    assert_eq!(
        fake.events(),
        [Event::Begin],
        "no checkpoint, no publication"
    );
    assert_eq!(fake.written_chunks(), Some(0));
    assert!(!folder.join("notes.txt").exists());
}
