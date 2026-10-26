//! The saved-files list on the desktop side (PR #56): the renderer projection, recording fenced on
//! the server incarnation and owner, forgetting and starting anew, the bounded whole-list check and
//! Show in folder over their port seam, and the "Replace my older saved copy" orchestration.
use super::*;
use catcoms_storage::kept::{create_folder, ExportFile, ExportPlan, Publication};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

fn store_in(dir: &Path) -> ServerStore {
    ServerStore::open(dir, b"saved files", &mut ChaCha20Rng::seed_from_u64(56)).unwrap()
}

/// A real journaled save of `name` (its content is the name itself) in `dir`, through the
/// storage crate's own export primitives: what a finished one-click save leaves behind.
pub(crate) fn published_journal(dir: &Path, name: &str) -> (PathBuf, ExportJournal) {
    let data = name.as_bytes();
    let plan = ExportPlan::from_manifest(&catcoms_storage::FileManifest {
        plaintext_cid: catcoms_storage::Cid::of(data),
        total_size: data.len() as u64,
        mime: "application/octet-stream".into(),
        chunks: vec![catcoms_storage::FileRef {
            plaintext_cid: catcoms_storage::Cid::of(b"p"),
            ciphertext_cid: catcoms_storage::Cid::of(b"c"),
            wrapped_key: catcoms_app::SealedBlob {
                nonce: [0; 24],
                ciphertext: vec![0; 48],
            },
            size: data.len() as u64,
            mime: "application/octet-stream".into(),
        }],
    })
    .unwrap();
    let mut file = ExportFile::create(dir, name, plan, &mut OsCryptoRng).unwrap();
    file.accept(0, data).unwrap();
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!("not yet published");
    };
    file.checkpointed(journal);
    let path = file.link().unwrap();
    let journal = file.published_journal();
    file.finish(&mut OsCryptoRng).unwrap();
    (path, journal)
}

fn plain(cid: &[u8], leaf: &str, folder: &Path) -> PublishedSave {
    PublishedSave {
        cid: catcoms_storage::Cid::of(cid),
        size: 3,
        location: SavedLocation::unidentified(&folder.join(leaf)).unwrap(),
    }
}

fn entry(id: u8, cid: &[u8], folder: &Path, leaf: &str, at: u64) -> SavedFile {
    SavedFile {
        id: SavedFileId::from_bytes([id; 16]),
        cid: catcoms_storage::Cid::of(cid),
        size: 3,
        saved_at_ms: at,
        location: SavedLocation::unidentified(&folder.join(leaf)).unwrap(),
    }
}

/// The list as stored for server 1 of `state` (its current owner binding).
async fn stored(state: &AppState) -> SavedFilesLoad {
    let servers = state.servers.lock().await;
    let entry = servers.get(&1).expect("server 1");
    let owner = [entry.group_id.as_slice(), entry.device_id.as_bytes()].concat();
    drop(servers);
    state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_saved_files(1, &owner)
}

#[test]
fn the_projection_is_display_data_only_newest_first() {
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    let other = root.path().join("Secret Projects");
    let mut list = SavedFiles::new(b"owner");
    list.record(entry(1, b"a", &downloads, "a.txt", 10));
    list.record(entry(2, b"b", &other, "b (1).txt", 20));
    let ui = project(&list, Some(&downloads));
    assert_eq!(ui.state, "ok");
    assert_eq!(
        ui.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["b (1).txt", "a.txt"],
        "newest first"
    );
    assert_eq!(ui.files[0].saved_to, Some("other"));
    assert_eq!(ui.files[1].saved_to, Some("downloads"));
    assert!(
        !ui.files[0].checkable,
        "a save without identity cannot be checked"
    );
    assert_eq!(ui.files[0].id, hex::encode([2u8; 16]));
    assert!(project(&list, None)
        .files
        .iter()
        .all(|f| f.saved_to.is_none()));
    let json = serde_json::to_string(&ui).unwrap();
    assert!(!json.contains("Secret Projects"), "{json}");
    assert!(!json.contains("Downloads"), "{json}");
    assert!(
        json.contains("\"savedAt\":20") && json.contains("\"savedTo\":\"other\""),
        "{json}"
    );
}

#[tokio::test]
async fn a_save_is_recorded_only_for_the_incarnation_that_saved() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    let state = AppState::default();
    *state.store.lock().await = Some(store_in(dir.path()));
    let (entry, task, drain) = crate::leaving::tests::running(561);
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);

    let first = record_save(&state, 1, 561, plain(b"file", "report.pdf", &folder), None).await;
    assert!(
        matches!(
            first,
            Recorded::Yes {
                replace: ReplaceChoice::NotAsked,
                ..
            }
        ),
        "{first:?}"
    );
    // A second save of the same file is a second copy, and a second entry (review M1).
    let second = record_save(
        &state,
        1,
        561,
        plain(b"file", "report (1).pdf", &folder),
        None,
    )
    .await;
    assert!(matches!(second, Recorded::Yes { .. }), "{second:?}");
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("a list was written");
    };
    let names: Vec<_> = list.entries().iter().map(|e| e.location.leaf()).collect();
    assert_eq!(names, ["report.pdf", "report (1).pdf"]);

    // Another incarnation of the same number (a leave and a rejoin, or an account switch) gets
    // nothing written, and neither does a server that is gone; neither is reported as unlisted.
    let stale = record_save(&state, 1, 999, plain(b"other", "x.txt", &folder), None).await;
    assert!(matches!(stale, Recorded::No { tell: false }), "{stale:?}");
    assert_eq!(stale.note(), None);
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("still the earlier list");
    };
    assert_eq!(
        list.entries().len(),
        2,
        "nothing was added for a stale incarnation"
    );
    state.servers.lock().await.remove(&1);
    let gone = record_save(&state, 1, 561, plain(b"other", "x.txt", &folder), None).await;
    assert!(matches!(gone, Recorded::No { tell: false }));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn a_list_this_build_cannot_read_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    let state = AppState::default();
    let store = store_in(dir.path());
    let record = store.backup_source_dir().join("servers").join("1.saved");
    std::fs::write(&record, b"written by a newer Mewtual, or damaged").unwrap();
    *state.store.lock().await = Some(store);
    let (entry, task, drain) = crate::leaving::tests::running(562);
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);

    let recorded = record_save(&state, 1, 562, plain(b"file", "report.pdf", &folder), None).await;
    assert!(
        matches!(recorded, Recorded::No { tell: true }),
        "{recorded:?}"
    );
    assert_eq!(recorded.note(), Some(NOT_RECORDED));
    assert!(!forget_entry(&state, 1, 562, SavedFileId::from_bytes([1; 16])).await);
    assert!(!rename_entry(&state, 1, 562, SavedFileId::from_bytes([1; 16]), "a", "b").await);
    assert_eq!(
        std::fs::read(&record).unwrap(),
        b"written by a newer Mewtual, or damaged"
    );
    assert_eq!(read_list(&state, 1, 562).await.unwrap(), Err("damaged"));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn clearing_a_saved_private_row_forgets_only_its_own_entry() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("Downloads");
    create_folder(&downloads).unwrap();
    let state = AppState::default();
    *state.store.lock().await = Some(store_in(dir.path()));
    let (entry, task, drain) = crate::leaving::tests::running(563);
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);
    let (_, journal) = published_journal(&downloads, "a.txt");
    let (_, other) = published_journal(&downloads, "b.txt");
    for (cid, journal) in [(&b"a"[..], &journal), (&b"b"[..], &other)] {
        let save = PublishedSave {
            cid: catcoms_storage::Cid::of(cid),
            size: 5,
            location: SavedLocation::from_journal(journal),
        };
        assert!(matches!(
            record_save(&state, 1, 563, save, None).await,
            Recorded::Yes { .. }
        ));
    }
    // The Saved private row of `journal` stands for exactly its own entry.
    let entries = entries_for_journals(&state, 1, 563, &[journal.clone(), other.clone()]).await;
    assert!(entries.iter().all(Option::is_some));
    assert_ne!(entries[0], entries[1]);
    forget_journal(&state, 1, 563, &journal).await;
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("list");
    };
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].location.leaf(), "b.txt");
    assert_eq!(
        entries_for_journals(&state, 1, 563, &[journal]).await,
        [None]
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn a_replace_plan_comes_from_the_same_locked_read_that_records_the_save() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("Downloads");
    let elsewhere = dir.path().join("Elsewhere");
    create_folder(&downloads).unwrap();
    create_folder(&elsewhere).unwrap();
    let state = AppState::default();
    *state.store.lock().await = Some(store_in(dir.path()));
    let (entry, task, drain) = crate::leaving::tests::running(564);
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);
    let cid = catcoms_storage::Cid::of(b"report.pdf");
    let save = |journal: &ExportJournal| PublishedSave {
        cid,
        size: 10,
        location: SavedLocation::from_journal(journal),
    };
    let ask = |claim| {
        Some(ReplaceAsk {
            claim,
            started_from: Some("report.pdf".into()),
        })
    };
    // Nothing saved before: nothing to say.
    let (_, first) = published_journal(&downloads, "report.pdf");
    let recorded = record_save(
        &state,
        1,
        564,
        save(&first),
        ask(ReplaceClaim::try_acquire(1, *cid.as_bytes())),
    )
    .await;
    assert!(
        matches!(
            recorded,
            Recorded::Yes {
                replace: ReplaceChoice::Skip(None),
                ..
            }
        ),
        "{recorded:?}"
    );
    // A second save in the same folder: a plan naming the first copy.
    let (_, second) = published_journal(&downloads, "report.pdf");
    let claim = ReplaceClaim::try_acquire(1, *cid.as_bytes());
    assert!(claim.is_some());
    assert!(
        ReplaceClaim::try_acquire(1, *cid.as_bytes()).is_none(),
        "one replace per file at a time"
    );
    let Recorded::Yes {
        replace: ReplaceChoice::Run(plan),
    } = record_save(&state, 1, 564, save(&second), ask(claim)).await
    else {
        panic!("a plan");
    };
    assert_eq!(plan.request.old.location.leaf(), "report.pdf");
    assert_eq!(plan.request.new.leaf(), "report (1).pdf");
    // The plan's entry is this save's own, recorded in the same locked write.
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("list");
    };
    assert_eq!(list.entries().last().map(|e| e.id), Some(plan.entry));
    drop(plan);
    assert!(
        ReplaceClaim::try_acquire(1, *cid.as_bytes()).is_some(),
        "released with the plan"
    );
    // Busy (another save of the file replacing) and another folder each say why not.
    let (_, third) = published_journal(&downloads, "report.pdf");
    let busy = record_save(&state, 1, 564, save(&third), ask(None)).await;
    assert!(
        matches!(
            busy,
            Recorded::Yes {
                replace: ReplaceChoice::Skip(Some(NOT_REPLACED_UNCHECKED)),
                ..
            }
        ),
        "{busy:?}"
    );
    let (_, there) = published_journal(&elsewhere, "report.pdf");
    let mut list = SavedFiles::new(b"o");
    list.record(SavedFile {
        id: SavedFileId::from_bytes([7; 16]),
        cid,
        size: 10,
        saved_at_ms: 1,
        location: SavedLocation::from_journal(&there),
    });
    let new = SavedFile {
        id: SavedFileId::from_bytes([8; 16]),
        cid,
        size: 10,
        saved_at_ms: 2,
        location: SavedLocation::from_journal(&third),
    };
    assert_eq!(
        list.replace_candidate(&new).err(),
        Some(NoCandidate::Elsewhere)
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

/// Scripted replace ports.
struct FakeReplace {
    outcome: Mutex<Option<ReplaceOutcome>>,
    forgot: Mutex<Vec<SavedFileId>>,
    renamed: Mutex<Vec<(String, String)>>,
    rename_ok: bool,
    dropped: Mutex<Vec<(String, String)>>,
    drop_ok: bool,
    live: AtomicBool,
}
impl FakeReplace {
    fn new(outcome: ReplaceOutcome) -> Self {
        Self {
            outcome: Mutex::new(Some(outcome)),
            forgot: Mutex::new(Vec::new()),
            renamed: Mutex::new(Vec::new()),
            rename_ok: true,
            dropped: Mutex::new(Vec::new()),
            drop_ok: true,
            live: AtomicBool::new(true),
        }
    }
}
#[async_trait::async_trait]
impl ReplacePorts for FakeReplace {
    async fn replace(&self, _request: ReplaceRequest) -> ReplaceOutcome {
        self.outcome.lock().unwrap().take().expect("one replace")
    }
    async fn forget(&self, id: SavedFileId) -> bool {
        self.forgot.lock().unwrap().push(id);
        true
    }
    async fn rename(&self, _id: SavedFileId, from: String, to: String) -> bool {
        self.renamed.lock().unwrap().push((from, to));
        self.rename_ok
    }
    async fn drop_first(&self, _new: SavedLocation, first: String, old: String) -> bool {
        self.dropped.lock().unwrap().push((first, old));
        self.drop_ok
    }
    fn live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }
}

/// A plan for a second save of "report.pdf" (numbered "report (1).pdf"), each in its own folder
/// under `root` so repeated plans in one test see the same names.
fn plan_for_test(root: &Path) -> ReplacePlan {
    static FOLDERS: AtomicUsize = AtomicUsize::new(0);
    let folder = root.join(format!("plan-{}", FOLDERS.fetch_add(1, Ordering::SeqCst)));
    create_folder(&folder).unwrap();
    let (_, old) = published_journal(&folder, "report.pdf");
    let (_, new) = published_journal(&folder, "report.pdf");
    ReplacePlan {
        request: ReplaceRequest {
            old: SavedFile {
                id: SavedFileId::from_bytes([1; 16]),
                cid: catcoms_storage::Cid::of(b"report.pdf"),
                size: 10,
                saved_at_ms: 1,
                location: SavedLocation::from_journal(&old),
            },
            new: SavedLocation::from_journal(&new),
            started_from: Some("report.pdf".into()),
        },
        entry: SavedFileId::from_bytes([2; 16]),
        // Claims are process-wide and tests run in parallel: each plan claims its own key.
        _claim: {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::SeqCst) as u64;
            ReplaceClaim::try_acquire(u64::MAX - n, [9; 32]).expect("a test claim")
        },
    }
}

#[tokio::test]
async fn the_replace_sequence_changes_the_list_only_after_each_effect() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    create_folder(&folder).unwrap();
    let old = SavedFileId::from_bytes([1; 16]);
    // Linked: forget the old entry, rename the new one, then drop the first name.
    let ports = FakeReplace::new(ReplaceOutcome::Linked {
        old_leaf: "report.pdf".into(),
    });
    let done = replace_sequence(&ports, plan_for_test(&folder)).await;
    assert_eq!(done.renamed_to.as_deref(), Some("report.pdf"));
    assert_eq!(done.notes, [REPLACED]);
    assert_eq!(*ports.forgot.lock().unwrap(), [old]);
    assert_eq!(
        *ports.renamed.lock().unwrap(),
        [("report (1).pdf".into(), "report.pdf".into())]
    );
    assert_eq!(
        *ports.dropped.lock().unwrap(),
        [("report (1).pdf".into(), "report.pdf".into())]
    );
    // Review M4: a rename that did not happen never lets the first name be removed.
    let mut ports = FakeReplace::new(ReplaceOutcome::Linked {
        old_leaf: "report.pdf".into(),
    });
    ports.rename_ok = false;
    let done = replace_sequence(&ports, plan_for_test(&folder)).await;
    assert_eq!(done.renamed_to, None);
    assert!(ports.dropped.lock().unwrap().is_empty());
    assert!(done.notes[1].contains("also kept the name report (1).pdf"));
    // Lock after the link: nothing more happens; both names stay and the note says so.
    let ports = FakeReplace::new(ReplaceOutcome::Linked {
        old_leaf: "report.pdf".into(),
    });
    ports.live.store(false, Ordering::SeqCst);
    let done = replace_sequence(&ports, plan_for_test(&folder)).await;
    assert!(ports.renamed.lock().unwrap().is_empty() && ports.dropped.lock().unwrap().is_empty());
    assert!(done
        .notes
        .iter()
        .any(|note| note.contains("also kept the name")));
    // A first name that could not be dropped (the old name went meanwhile, review H1).
    let mut ports = FakeReplace::new(ReplaceOutcome::Linked {
        old_leaf: "report.pdf".into(),
    });
    ports.drop_ok = false;
    let done = replace_sequence(&ports, plan_for_test(&folder)).await;
    assert_eq!(done.renamed_to.as_deref(), Some("report.pdf"));
    assert!(done.notes[1].contains("also kept the name report (1).pdf"));
}

#[tokio::test]
async fn every_replace_outcome_has_a_truthful_note() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    create_folder(&folder).unwrap();
    let old = SavedFileId::from_bytes([1; 16]);
    for (outcome, forgets, note) in [
        (ReplaceOutcome::Missing, true, None),
        (ReplaceOutcome::AlreadyNew, true, None),
        (ReplaceOutcome::Removed, true, Some(REPLACED.to_owned())),
        (
            ReplaceOutcome::RemovedNotMoved { still_named: true },
            true,
            Some(kept_own_name("report (1).pdf")),
        ),
        // A name that is gone is never named (adversarial review H1).
        (
            ReplaceOutcome::RemovedNotMoved { still_named: false },
            true,
            Some(REMOVED_NEW_GONE.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::NewGone),
            false,
            Some(NOT_REPLACED_NEW_GONE.into()),
        ),
        // A folder that moved says nothing about the copy (adversarial review L2).
        (
            ReplaceOutcome::Refused(ReplaceRefusal::FolderChanged),
            false,
            Some(NOT_REPLACED_UNCHECKED.into()),
        ),
        (
            ReplaceOutcome::Stranded,
            false,
            Some(REPLACE_STRANDED.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::Changed),
            false,
            Some(NOT_REPLACED_CHANGED.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::MoreNames),
            false,
            Some(NOT_REPLACED_NAMES.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::ReadOnly),
            false,
            Some(NOT_REPLACED_READ_ONLY.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::Shared),
            false,
            Some(NOT_REPLACED_SHARED.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::Reparse),
            false,
            Some(NOT_REPLACED_CANNOT_CHECK.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::Stopped),
            false,
            Some(NOT_REPLACED_UNCHECKED.into()),
        ),
        (
            ReplaceOutcome::Refused(ReplaceRefusal::OtherFolder),
            false,
            Some(NOT_REPLACED_ELSEWHERE.into()),
        ),
    ] {
        let ports = FakeReplace::new(outcome.clone());
        let done = replace_sequence(&ports, plan_for_test(&folder)).await;
        assert_eq!(done.notes.first().cloned(), note, "{outcome:?}");
        assert_eq!(
            *ports.forgot.lock().unwrap() == [old],
            forgets,
            "{outcome:?}: the old entry goes only when its copy is gone"
        );
        assert!(ports.renamed.lock().unwrap().is_empty());
        for text in &done.notes {
            assert!(!text.contains('·') && !text.contains('—'), "{text}");
        }
    }
}

/// Scripted list ports.
struct FakeList {
    lists: Mutex<Vec<Option<SavedFiles>>>,
    reads: AtomicUsize,
    answers: Vec<(SavedFileId, SavedPresence)>,
    /// The checker parks forever after its answers (a dead share).
    park: bool,
    located: Mutex<Option<Result<PathBuf, SavedPresence>>>,
    current: AtomicBool,
    revealed: Mutex<Vec<PathBuf>>,
    reveal_fails: bool,
}
impl FakeList {
    fn new(lists: Vec<Option<SavedFiles>>) -> Self {
        Self {
            lists: Mutex::new(lists),
            reads: AtomicUsize::new(0),
            answers: Vec::new(),
            park: false,
            located: Mutex::new(None),
            current: AtomicBool::new(true),
            revealed: Mutex::new(Vec::new()),
            reveal_fails: false,
        }
    }
}
#[async_trait::async_trait]
impl SavedListPorts for FakeList {
    async fn list(&self) -> Result<Option<SavedFiles>, String> {
        let at = self.reads.fetch_add(1, Ordering::SeqCst);
        let lists = self.lists.lock().unwrap();
        Ok(lists[at.min(lists.len() - 1)].clone())
    }
    async fn check_all(
        &self,
        entries: Vec<SavedFile>,
        _stop: Arc<AtomicBool>,
        found: Arc<std::sync::Mutex<Vec<(SavedFileId, SavedPresence)>>>,
    ) {
        for answer in &self.answers {
            assert!(entries.iter().any(|entry| entry.id == answer.0));
            found.lock().unwrap().push(*answer);
        }
        if self.park {
            std::future::pending::<()>().await;
        }
    }
    async fn locate(&self, _entry: SavedFile) -> Result<PathBuf, SavedPresence> {
        let answer = self.located.lock().unwrap().clone();
        match answer {
            Some(answer) => answer,
            None => std::future::pending().await,
        }
    }
    async fn wait(&self, _delay: Duration) {
        // The bound is reached at once; a check that is not parked has already finished.
        tokio::task::yield_now().await;
    }
    async fn current(&self) -> bool {
        self.current.load(Ordering::SeqCst)
    }
    fn reveal(&self, path: &Path) -> Result<(), String> {
        self.revealed.lock().unwrap().push(path.to_owned());
        if self.reveal_fails {
            Err("no file manager".into())
        } else {
            Ok(())
        }
    }
}

fn identified_list(folder: &Path) -> (SavedFiles, SavedFileId, SavedFileId) {
    let (_, a) = published_journal(folder, "a.txt");
    let (_, b) = published_journal(folder, "b.txt");
    let mut list = SavedFiles::new(b"owner");
    let ids = [
        SavedFileId::from_bytes([1; 16]),
        SavedFileId::from_bytes([2; 16]),
    ];
    for (id, cid, journal) in [(ids[0], &b"a"[..], &a), (ids[1], &b"b"[..], &b)] {
        assert!(list.record(SavedFile {
            id,
            cid: catcoms_storage::Cid::of(cid),
            size: 5,
            saved_at_ms: 1,
            location: SavedLocation::from_journal(journal),
        }));
    }
    (list, ids[0], ids[1])
}

#[tokio::test]
async fn a_whole_list_check_answers_only_for_entries_unchanged_since() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    create_folder(&folder).unwrap();
    let (list, a, b) = identified_list(&folder);
    let mut after = list.clone();
    after.forget(b);
    let mut ports = FakeList::new(vec![Some(list.clone()), Some(after)]);
    ports.answers = vec![(b, SavedPresence::Ours), (a, SavedPresence::Missing)];
    let answers = check_list_sequence(&ports).await.unwrap();
    assert_eq!(
        answers,
        [UiSavedFileCheck {
            id: a.to_hex(),
            name: "a.txt".into(),
            check: SavedCheck::Missing,
        }]
    );
    // An entry renamed meanwhile (a replace moved it) gets no answer for its old name.
    let mut renamed = list.clone();
    assert!(renamed.rename(a, "a.txt", "a (2).txt"));
    let mut ports = FakeList::new(vec![Some(list.clone()), Some(renamed)]);
    ports.answers = vec![(a, SavedPresence::Ours)];
    assert!(check_list_sequence(&ports).await.unwrap().is_empty());
    // The session changed: nothing is answered.
    let mut ports = FakeList::new(vec![Some(list.clone())]);
    ports.answers = vec![(a, SavedPresence::Ours)];
    ports.current.store(false, Ordering::SeqCst);
    assert!(check_list_sequence(&ports).await.is_err());
    // Unreadable: refused, before any check.
    let ports = FakeList::new(vec![None]);
    assert_eq!(
        check_list_sequence(&ports).await.unwrap_err(),
        LIST_UNREADABLE
    );
}

#[tokio::test]
async fn a_whole_list_check_is_bounded_and_returns_what_it_reached() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    create_folder(&folder).unwrap();
    let (list, a, _) = identified_list(&folder);
    let mut ports = FakeList::new(vec![Some(list)]);
    ports.answers = vec![(a, SavedPresence::Replaced)];
    ports.park = true;
    let answers = check_list_sequence(&ports).await.unwrap();
    assert_eq!(
        answers.len(),
        1,
        "the unreached entry is left out, not 'could not check'"
    );
    assert_eq!(answers[0].check, SavedCheck::Replaced);
}

#[test]
fn the_production_checker_stops_between_entries_and_never_changes_a_file() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    create_folder(&folder).unwrap();
    let (list, a, b) = identified_list(&folder);
    let entries: Vec<_> = list.entries().iter().rev().cloned().collect();
    let found = Mutex::new(Vec::new());
    check_entries(entries.clone(), &AtomicBool::new(false), &found);
    assert_eq!(
        found.lock().unwrap().clone(),
        [(b, SavedPresence::Ours), (a, SavedPresence::Ours)]
    );
    std::fs::remove_file(folder.join("a.txt")).unwrap();
    let found = Mutex::new(Vec::new());
    check_entries(entries.clone(), &AtomicBool::new(false), &found);
    assert_eq!(found.lock().unwrap()[1], (a, SavedPresence::Missing));
    let found = Mutex::new(Vec::new());
    check_entries(entries, &AtomicBool::new(true), &found);
    assert!(
        found.lock().unwrap().is_empty(),
        "a stop before the worker checks nothing"
    );
    assert!(folder.join("b.txt").exists() && !folder.join("a.txt").exists());
}

#[tokio::test]
async fn show_in_folder_by_entry_checks_first_and_says_why_not() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("Downloads");
    create_folder(&folder).unwrap();
    let (list, a, _) = identified_list(&folder);
    let path = folder.join("a.txt");
    let ports = FakeList::new(vec![Some(list.clone())]);
    *ports.located.lock().unwrap() = Some(Ok(path.clone()));
    reveal_entry_sequence(&ports, a).await.unwrap();
    assert_eq!(*ports.revealed.lock().unwrap(), std::slice::from_ref(&path));
    for (answer, message) in [
        (
            Err(SavedPresence::Missing),
            private_downloads::REVEAL_MISSING,
        ),
        (
            Err(SavedPresence::Replaced),
            private_downloads::REVEAL_REPLACED,
        ),
        (
            Err(SavedPresence::Unknown),
            private_downloads::REVEAL_UNCHECKED,
        ),
    ] {
        let ports = FakeList::new(vec![Some(list.clone())]);
        *ports.located.lock().unwrap() = Some(answer);
        assert_eq!(reveal_entry_sequence(&ports, a).await.unwrap_err(), message);
        assert!(ports.revealed.lock().unwrap().is_empty());
    }
    let ports = FakeList::new(vec![Some(list.clone())]);
    assert_eq!(
        reveal_entry_sequence(&ports, a).await.unwrap_err(),
        private_downloads::REVEAL_UNCHECKED,
        "a check that does not answer in time"
    );
    let mut ports = FakeList::new(vec![Some(list.clone())]);
    ports.reveal_fails = true;
    *ports.located.lock().unwrap() = Some(Ok(path.clone()));
    assert_eq!(
        reveal_entry_sequence(&ports, a).await.unwrap_err(),
        private_downloads::REVEAL_FAILED
    );
    let mut renamed = list.clone();
    renamed.rename(a, "a.txt", "a (2).txt");
    let mut cleared = list.clone();
    cleared.forget(a);
    for after in [renamed, cleared] {
        let ports = FakeList::new(vec![Some(list.clone()), Some(after)]);
        *ports.located.lock().unwrap() = Some(Ok(path.clone()));
        assert_eq!(
            reveal_entry_sequence(&ports, a).await.unwrap_err(),
            ENTRY_GONE
        );
        assert!(ports.revealed.lock().unwrap().is_empty());
    }
    let ports = FakeList::new(vec![Some(list.clone())]);
    *ports.located.lock().unwrap() = Some(Ok(path.clone()));
    ports.current.store(false, Ordering::SeqCst);
    assert!(reveal_entry_sequence(&ports, a).await.is_err());
    assert!(ports.revealed.lock().unwrap().is_empty());
    let ports = FakeList::new(vec![Some(list.clone())]);
    assert_eq!(
        reveal_entry_sequence(&ports, SavedFileId::from_bytes([9; 16]))
            .await
            .unwrap_err(),
        ENTRY_GONE
    );
    let ports = FakeList::new(vec![None]);
    assert_eq!(
        reveal_entry_sequence(&ports, a).await.unwrap_err(),
        LIST_UNREADABLE
    );
}

/// `save_group_file` needs a live AppHandle, so its order is pinned on its text (as the leave
/// order is in `cache_housekeeping`): the publication is listed before anything else can stop the
/// invocation (design review L10), a replace runs only for a durably recorded plan and only after
/// the private record was released, and the replace branch reveals the final name fenced by the
/// original invocation.
#[test]
fn save_group_file_lists_the_save_first_and_replaces_only_after_release() {
    let source = include_str!("../lib.rs");
    let body = &source[source
        .find("async fn save_group_file(")
        .expect("save_group_file")..];
    let body = &body[..body
        .find("\nasync fn finish_download_response")
        .expect("end")];
    let at = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("{needle} is in save_group_file"))
    };
    let record = at("saved_files::record_save(");
    assert!(record < at("let mut saved = finish_saved_read_if("));
    assert!(record < at("finish_download_response("));
    // The replace branch: a Run plan only from `Recorded::Yes`, never for a kept record.
    assert!(body.contains("replace: saved_files::ReplaceChoice::Run(plan),\n                ..\n            } if !keep_record => Some(plan),"));
    let release = at("let released = actor.finish_download_export(ready.clone()).await.is_ok();");
    let replace = at("saved_files::run_replace(");
    assert!(
        release < replace,
        "the record is released before the replace"
    );
    let branch = &body[replace..];
    assert!(branch.contains("|| ready.invocation_current(),"));
    // A recovered publication is listed but never replaces.
    assert!(body.contains("let ask = (asked && !recovered).then("));
}

#[test]
fn a_busy_replace_claim_refuses_without_releasing_the_holder() {
    // Regression: a refused acquire once built (and dropped) a claim for the busy key, which
    // deadlocked on the claims lock and would have released the holder's key.
    let key = (u64::MAX / 2, [7u8; 32]);
    let held = ReplaceClaim::try_acquire(key.0, key.1).expect("free");
    for _ in 0..3 {
        assert!(ReplaceClaim::try_acquire(key.0, key.1).is_none());
    }
    assert!(
        ReplaceClaim::try_acquire(key.0, key.1).is_none(),
        "still held after refusals"
    );
    drop(held);
    assert!(ReplaceClaim::try_acquire(key.0, key.1).is_some());
}

#[test]
fn entry_ids_from_the_renderer_are_parsed_strictly() {
    assert!(entry_id(&"ab".repeat(16)).is_ok());
    for bad in ["", "..\\x", &"AB".repeat(16), &"ab".repeat(17)] {
        assert!(entry_id(bad).is_err(), "{bad}");
    }
}
