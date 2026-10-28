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

#[derive(Clone, Copy, Debug)]
enum SessionMutation {
    Forget,
    Reset,
    Limit,
}

impl SessionMutation {
    /// The production paths behind the three Tauri commands, including their entry checks.
    async fn apply(self, state: &AppState) -> Result<(), String> {
        match self {
            Self::Forget => forget_saved_file_in_session(state, 1, hex::encode([1u8; 16])).await,
            Self::Reset => reset_saved_files_in_session(state, 1).await,
            Self::Limit => set_saved_files_limit_in_session(state, 1, 16, true, Some(4))
                .await
                .map(|_| ()),
        }
    }
}

struct SessionWriteCase {
    root: tempfile::TempDir,
    state: AppState,
    actor: ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
}

impl SessionWriteCase {
    async fn new(action: SessionMutation) -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let store = store_in(root.path());
        let (server, task, drain) = crate::leaving::tests::running(56_001);
        let actor = server.actor.clone();
        let owner = [server.group_id.as_slice(), server.device_id.as_bytes()].concat();
        let mut list = SavedFiles::new(&owner);
        for id in 1..=20 {
            list.record_default(entry(
                id,
                &[id],
                root.path(),
                &format!("file-{id}"),
                id as u64,
            ));
        }
        store
            .save_saved_files(1, &owner, &list, &mut OsCryptoRng)
            .unwrap();
        store
            .save_saved_files_limit(1, &owner, SavedLimit::DEFAULT, &mut OsCryptoRng)
            .unwrap();
        if matches!(action, SessionMutation::Reset) {
            std::fs::write(root.path().join("servers/1.saved"), b"damaged list").unwrap();
        }
        *state.store.lock().await = Some(store);
        *state.session_resumable.lock().await = true;
        state.servers.lock().await.insert(1, server);
        Self {
            root,
            state,
            actor,
            task,
            drain,
        }
    }

    fn sealed(&self) -> [Vec<u8>; 2] {
        ["1.saved", "1.saved-limit"]
            .map(|name| std::fs::read(self.root.path().join("servers").join(name)).unwrap())
    }

    async fn finish(self) {
        self.state.servers.lock().await.clear();
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.await.unwrap();
    }
}

/// Poll once, without moving time or relying on a scheduler delay.
async fn still_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) -> bool {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx).is_pending())).await
}

async fn mutation_rechecks_lock_at_the_actual_sealed_write(action: SessionMutation) {
    let case = SessionWriteCase::new(action).await;
    let before = case.sealed();
    let servers = case.state.servers.lock().await;
    let mut write = Box::pin(action.apply(&case.state));
    // Entry authorization has passed; actor_instance_of is waiting on Servers.
    assert!(still_pending(write.as_mut()).await, "{action:?}");
    let store = case.state.store.lock().await;
    drop(servers);
    // Final UI authorization now owns commit but is waiting on Store.
    assert!(still_pending(write.as_mut()).await, "{action:?}");
    let mut next_store = Box::pin(case.state.store.lock());
    assert!(still_pending(next_store.as_mut()).await);
    drop(store);
    // Tokio's FIFO mutex admits authorization first, then next_store, then the
    // command's mutation. Thus Lock races the real mutation, not its entry guard.
    assert!(still_pending(write.as_mut()).await, "{action:?}");
    let store = next_store.await;
    assert!(
        case.state.ui_session_commit.try_lock().is_err(),
        "{action:?}: mutation must still own its final authorization guard"
    );
    let mut lock = Box::pin(lock_session_inner(&case.state, None));
    assert!(
        still_pending(lock.as_mut()).await,
        "{action:?}: Lock must wait for the command's retained commit fence"
    );
    drop(store);
    assert!(
        write.await.is_err(),
        "{action:?}: Lock registered before final admission"
    );
    lock.await.unwrap();
    assert_eq!(
        case.sealed(),
        before,
        "{action:?}: sealed records stay unchanged"
    );
    case.finish().await;
}

#[tokio::test]
async fn forget_saved_file_rechecks_lock_at_the_actual_sealed_write() {
    mutation_rechecks_lock_at_the_actual_sealed_write(SessionMutation::Forget).await;
}

#[tokio::test]
async fn reset_saved_files_rechecks_lock_at_the_actual_sealed_write() {
    mutation_rechecks_lock_at_the_actual_sealed_write(SessionMutation::Reset).await;
}

#[tokio::test]
async fn saved_files_limit_rechecks_lock_at_the_actual_sealed_write() {
    mutation_rechecks_lock_at_the_actual_sealed_write(SessionMutation::Limit).await;
}

#[tokio::test]
async fn saved_file_mutations_apply_while_the_original_session_is_open() {
    for action in [
        SessionMutation::Forget,
        SessionMutation::Reset,
        SessionMutation::Limit,
    ] {
        let case = SessionWriteCase::new(action).await;
        action.apply(&case.state).await.unwrap();
        let SavedFilesLoad::Found(list) = stored(&case.state).await else {
            panic!("{action:?}: the sealed list must read back");
        };
        let expected = match action {
            SessionMutation::Forget => 19,
            SessionMutation::Reset => 0,
            SessionMutation::Limit => 16,
        };
        assert_eq!(list.entries().len(), expected, "{action:?}");
        if matches!(action, SessionMutation::Forget) {
            assert!(list
                .entries()
                .iter()
                .all(|e| e.id != SavedFileId::from_bytes([1; 16])));
        }
        if matches!(action, SessionMutation::Limit) {
            let servers = case.state.servers.lock().await;
            let server = servers.get(&1).unwrap();
            let owner = [server.group_id.as_slice(), server.device_id.as_bytes()].concat();
            drop(servers);
            let store = case.state.store.lock().await;
            assert_eq!(
                store.as_ref().unwrap().load_saved_files_limit(1, &owner),
                SavedLimitLoad::Found(SavedLimit::clamped(16))
            );
        }
        case.finish().await;
    }
}

#[tokio::test]
async fn saved_entry_reveal_rejects_lock_registered_during_the_final_registry_wait() {
    let case = SessionWriteCase::new(SessionMutation::Forget).await;
    let SavedFilesLoad::Found(list) = stored(&case.state).await else {
        panic!("stored list");
    };
    let expected = list.entries()[0].clone();
    let request = ActorSavedList {
        state: &case.state,
        server: 1,
        instance: 56_001,
        generation: 0,
    };
    let path = case.root.path().join("file-1");
    let called = AtomicBool::new(false);
    let servers = case.state.servers.lock().await;
    let mut reveal = Box::pin(reveal_entry_in_session(&request, &path, &expected, |_| {
        called.store(true, Ordering::Release);
        Ok(())
    }));
    assert!(still_pending(reveal.as_mut()).await);
    assert!(case.state.ui_session_commit.try_lock().is_err());
    let mut lock = Box::pin(lock_session_inner(&case.state, None));
    assert!(still_pending(lock.as_mut()).await);
    drop(servers);
    assert_eq!(reveal.await, Err(private_downloads::RevealError::Stale));
    lock.await.unwrap();
    assert!(!called.load(Ordering::Acquire));
    drop(request);
    case.finish().await;
}

#[tokio::test]
async fn saved_entry_reveal_holds_commit_and_an_unlock_cannot_revive_it() {
    let case = SessionWriteCase::new(SessionMutation::Forget).await;
    let SavedFilesLoad::Found(list) = stored(&case.state).await else {
        panic!("stored list");
    };
    let expected = list.entries()[0].clone();
    let request = ActorSavedList {
        state: &case.state,
        server: 1,
        instance: 56_001,
        generation: 0,
    };
    let path = case.root.path().join("file-1");
    reveal_entry_in_session(&request, &path, &expected, |_| {
        assert!(case.state.ui_session_commit.try_lock().is_err());
        assert!(case.state.store.try_lock().is_err());
        assert!(case.state.servers.try_lock().is_err());
        Ok(())
    })
    .await
    .unwrap();
    lock_session_inner(&case.state, None).await.unwrap();
    let generation = case.state.ui_session_generation.load(Ordering::Acquire);
    finalize_unlock_session(&case.state, generation, false)
        .await
        .unwrap();
    assert!(!ui_session_closed(&case.state));
    assert_eq!(
        reveal_entry_in_session(&request, &path, &expected, |_| panic!(
            "unlock cannot authorize an old reveal"
        ))
        .await,
        Err(private_downloads::RevealError::Stale)
    );
    drop(request);
    case.finish().await;
}

#[tokio::test]
async fn saved_entry_reveal_rechecks_the_actual_entry_after_waiting_for_commit() {
    let case = SessionWriteCase::new(SessionMutation::Forget).await;
    let SavedFilesLoad::Found(list) = stored(&case.state).await else {
        panic!("stored list");
    };
    let expected = list.entries()[0].clone();
    let request = ActorSavedList {
        state: &case.state,
        server: 1,
        instance: 56_001,
        generation: 0,
    };
    let path = case.root.path().join("file-1");
    let commit = require_ui_session_generation(&case.state, 0).await.unwrap();
    let mut reveal = Box::pin(reveal_entry_in_session(&request, &path, &expected, |_| {
        panic!("a cleared entry must never launch")
    }));
    assert!(still_pending(reveal.as_mut()).await);
    assert!(forget_entry(&case.state, 1, 56_001, expected.id).await);
    drop(commit);
    assert_eq!(reveal.await, Err(private_downloads::RevealError::Stale));
    drop(request);
    case.finish().await;
}

/// Recording at the default setting, which most rules here are about.
trait RecordDefault {
    fn record_default(&mut self, entry: SavedFile) -> bool;
}
impl RecordDefault for SavedFiles {
    fn record_default(&mut self, entry: SavedFile) -> bool {
        self.record(entry, SavedLimit::DEFAULT)
    }
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
    list.record_default(entry(1, b"a", &downloads, "a.txt", 10));
    list.record_default(entry(2, b"b", &other, "b (1).txt", 20));
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

/// Adversarial review M1: a replace whose `forget(old)` write failed still renames the new entry
/// onto the old copy's name. That rename must leave a list the next load can read, not two
/// entries for one place (which reads as damaged and would cost every entry on "Start a new
/// list").
#[tokio::test]
async fn a_rename_after_a_failed_forget_keeps_the_list_readable() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    let state = AppState::default();
    *state.store.lock().await = Some(store_in(dir.path()));
    let (entry, task, drain) = crate::leaving::tests::running(564);
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);
    for leaf in ["report.pdf", "report (1).pdf"] {
        let save = plain(b"file", leaf, &folder);
        assert!(matches!(
            record_save(&state, 1, 564, save, None).await,
            Recorded::Yes { .. }
        ));
    }
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("list");
    };
    let new = list.entries()[1].id;

    // The old entry was never forgotten; the rename still succeeds and replaces it.
    assert!(rename_entry(&state, 1, 564, new, "report (1).pdf", "report.pdf").await);
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("the list must still read after the rename");
    };
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].id, new);
    assert_eq!(list.entries()[0].location.leaf(), "report.pdf");
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

/// Server 1's owner binding in `state` (the group, then this device's member id).
async fn owner_of(state: &AppState) -> Vec<u8> {
    let servers = state.servers.lock().await;
    let entry = servers.get(&1).expect("server 1");
    [entry.group_id.as_slice(), entry.device_id.as_bytes()].concat()
}

/// "Remember saved files": a save is recorded within the stored setting, and with a setting that
/// cannot be read nothing is recorded (guessing it could forget entries the user chose to keep).
#[tokio::test]
async fn a_save_is_recorded_within_the_stored_setting() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    let state = AppState::default();
    *state.store.lock().await = Some(store_in(dir.path()));
    let (entry, task, drain) = crate::leaving::tests::running(565);
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);
    let owner = owner_of(&state).await;
    state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .save_saved_files_limit(1, &owner, SavedLimit::clamped(16), &mut OsCryptoRng)
        .unwrap();
    for n in 0..20u8 {
        let save = plain(&[n], &format!("f{n}.txt"), &folder);
        assert!(matches!(
            record_save(&state, 1, 565, save, None).await,
            Recorded::Yes { .. }
        ));
    }
    let SavedFilesLoad::Found(list) = stored(&state).await else {
        panic!("list");
    };
    let names: Vec<_> = list.entries().iter().map(|e| e.location.leaf()).collect();
    assert_eq!(names.len(), 16);
    assert_eq!(names[0], "f4.txt", "the four oldest were forgotten");
    assert_eq!(names[15], "f19.txt");

    // A damaged setting: nothing is recorded, the user hears so, and the list stays as it was.
    let setting = dir.path().join("servers").join("1.saved-limit");
    std::fs::write(&setting, b"damaged").unwrap();
    let refused = record_save(&state, 1, 565, plain(b"x", "x.txt", &folder), None).await;
    assert!(
        matches!(refused, Recorded::No { tell: true }),
        "{refused:?}"
    );
    assert_eq!(stored(&state).await, SavedFilesLoad::Found(list));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[test]
fn choosing_the_setting_previews_clamps_and_forgets_only_the_oldest() {
    const OWNER: &[u8] = b"group+member";
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    let store = store_in(dir.path());
    let view = limit_view(&store, 1, OWNER);
    assert_eq!(
        view,
        UiSavedFilesLimit {
            state: "ok",
            limit: Some(256),
            listed: Some(0),
            min: 16,
            max: 4096,
            default: 256,
        }
    );
    let mut list = SavedFiles::new(OWNER);
    for n in 0..40u8 {
        assert!(list.record_default(entry(n, &[n], &folder, &format!("f{n}.txt"), u64::from(n))));
    }
    store
        .save_saved_files(1, OWNER, &list, &mut OsCryptoRng)
        .unwrap();

    // A preview changes nothing; a request below the range is clamped to 16.
    let preview = change_limit(&store, 1, OWNER, 1, None).unwrap();
    assert_eq!(
        preview,
        UiSavedLimitChange {
            limit: 16,
            forgets: 24,
            applied: false,
        }
    );
    assert_eq!(
        store.load_saved_files(1, OWNER),
        SavedFilesLoad::Found(list.clone())
    );
    assert_eq!(
        store.load_saved_files_limit(1, OWNER),
        SavedLimitLoad::Missing
    );

    // A save meanwhile changed what the confirmed count would forget: nothing changes (review
    // LOW-1), and the page asks again.
    assert_eq!(
        change_limit(&store, 1, OWNER, 1, Some(23)),
        Err(LIMIT_LIST_CHANGED.to_string())
    );
    assert_eq!(
        store.load_saved_files(1, OWNER),
        SavedFilesLoad::Found(list.clone())
    );
    assert_eq!(
        store.load_saved_files_limit(1, OWNER),
        SavedLimitLoad::Missing
    );

    // Applied as confirmed: the 24 oldest entries are forgotten, the 16 newest stay, the setting
    // is stored.
    let applied = change_limit(&store, 1, OWNER, 1, Some(24)).unwrap();
    assert_eq!(
        (applied.limit, applied.forgets, applied.applied),
        (16, 24, true)
    );
    let SavedFilesLoad::Found(trimmed) = store.load_saved_files(1, OWNER) else {
        panic!("list");
    };
    assert_eq!(trimmed.entries(), &list.entries()[24..]);
    assert_eq!(
        store.load_saved_files_limit(1, OWNER),
        SavedLimitLoad::Found(SavedLimit::clamped(16))
    );

    // Raising (above the range: clamped to 4096) keeps every entry.
    let raised = change_limit(&store, 1, OWNER, 100_000, Some(0)).unwrap();
    assert_eq!((raised.limit, raised.forgets), (4096, 0));
    assert_eq!(
        store.load_saved_files(1, OWNER),
        SavedFilesLoad::Found(trimmed.clone())
    );

    // Across a restart (a new mount), the setting and the list read back.
    drop(store);
    let store = store_in(dir.path());
    let view = limit_view(&store, 1, OWNER);
    assert_eq!((view.limit, view.listed), (Some(4096), Some(16)));
}

#[test]
fn an_unreadable_setting_or_list_is_never_guessed_at() {
    const OWNER: &[u8] = b"group+member";
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let servers = dir.path().join("servers");
    // A damaged setting reads as unreadable; choosing one again writes over it.
    std::fs::write(servers.join("1.saved-limit"), b"damaged").unwrap();
    let view = limit_view(&store, 1, OWNER);
    assert_eq!((view.state, view.limit), ("unreadable", None));
    change_limit(&store, 1, OWNER, 300, Some(0)).unwrap();
    assert_eq!(
        store.load_saved_files_limit(1, OWNER),
        SavedLimitLoad::Found(SavedLimit::clamped(300))
    );
    // A damaged list is never trimmed or written; only its setting changes.
    std::fs::write(servers.join("1.saved"), b"damaged list").unwrap();
    let view = limit_view(&store, 1, OWNER);
    assert_eq!(view.listed, None);
    let lowered = change_limit(&store, 1, OWNER, 16, Some(0)).unwrap();
    assert_eq!((lowered.limit, lowered.forgets), (16, 0));
    assert_eq!(
        std::fs::read(servers.join("1.saved")).unwrap(),
        b"damaged list"
    );
}

/// Adversarial review of the setting, MEDIUM-1: while the list cannot be read (a newer build's
/// record, or an I/O error right now) its length is unknown, so nothing could be shown before
/// confirming. Only a choice that keeps every entry is accepted; a lower one would let the next
/// save forget entries without warning once the list reads again.
#[test]
fn a_list_of_unknown_length_accepts_only_a_setting_that_keeps_every_entry() {
    let at = SavedLimit::clamped;
    let cases = [
        (SavedLimitLoad::Found(at(256)), [false, true, true]),
        (SavedLimitLoad::Found(at(1024)), [false, false, true]),
        // No setting stored: the default 256 is in force.
        (SavedLimitLoad::Missing, [false, true, true]),
        // The setting itself cannot be read: only the largest keeps every entry for sure.
        (SavedLimitLoad::Unreadable, [false, false, true]),
    ];
    for (current, allowed) in cases {
        for (limit, allowed) in [at(16), at(256), at(4096)].into_iter().zip(allowed) {
            assert_eq!(
                unknown_list_allows(current, limit),
                allowed,
                "{current:?} -> {limit:?}"
            );
        }
    }
}

/// Review LOW-2: when the new setting cannot be stored after the list was shortened, the error
/// says how many entries went, so the page does not claim nothing changed.
#[test]
fn a_setting_that_cannot_be_stored_after_the_trim_says_what_was_forgotten() {
    const OWNER: &[u8] = b"group+member";
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Downloads");
    let store = store_in(dir.path());
    let mut list = SavedFiles::new(OWNER);
    for n in 0..20u8 {
        assert!(list.record_default(entry(n, &[n], &folder, &format!("f{n}.txt"), u64::from(n))));
    }
    store
        .save_saved_files(1, OWNER, &list, &mut OsCryptoRng)
        .unwrap();
    // A folder where the setting's record goes: it reads as unreadable, and writing over it fails.
    std::fs::create_dir(dir.path().join("servers").join("1.saved-limit")).unwrap();
    let error = change_limit(&store, 1, OWNER, 16, Some(4)).unwrap_err();
    assert!(
        error.starts_with(
            "Mewtual forgot the 4 oldest saved files from the list, but could not store the new setting."
        ),
        "{error}"
    );
    let SavedFilesLoad::Found(trimmed) = store.load_saved_files(1, OWNER) else {
        panic!("the shortened list reads");
    };
    assert_eq!(trimmed.entries(), &list.entries()[4..]);
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
    list.record_default(SavedFile {
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
    drop: FirstNameDrop,
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
            drop: FirstNameDrop::Removed,
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
    async fn drop_first(&self, _new: SavedLocation, first: String, old: String) -> FirstNameDrop {
        self.dropped.lock().unwrap().push((first, old));
        self.drop
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
    // Re-review MEDIUM: a first name that was not dropped is reported by what step 6 saw
    // afterwards, so the result never sends the user to a name that went.
    for (drop, renamed_to, note, moved_back) in [
        // Both names hold the copy: the second name is named, deleting either keeps the other.
        (
            FirstNameDrop::BothNames,
            Some("report.pdf"),
            Some(second_name_left("report (1).pdf")),
            false,
        ),
        // The old name went (or holds another file): the copy is only at its first name, the
        // entry moves back and nothing says "both names".
        (
            FirstNameDrop::OnlyFirst,
            None,
            Some(kept_own_name("report (1).pdf")),
            true,
        ),
        // The first name went: the copy has the old name only, nothing more to say.
        (FirstNameDrop::OnlyOld, Some("report.pdf"), None, false),
        // Neither could be shown: no name is claimed.
        (
            FirstNameDrop::Unknown,
            Some("report.pdf"),
            Some(NAME_UNCONFIRMED.to_owned()),
            false,
        ),
    ] {
        let mut ports = FakeReplace::new(ReplaceOutcome::Linked {
            old_leaf: "report.pdf".into(),
        });
        ports.drop = drop;
        let done = replace_sequence(&ports, plan_for_test(&folder)).await;
        assert_eq!(done.renamed_to.as_deref(), renamed_to, "{drop:?}");
        assert_eq!(done.notes[0], REPLACED, "{drop:?}");
        assert_eq!(done.notes.get(1).cloned(), note, "{drop:?}");
        let mut expected = vec![("report (1).pdf".to_owned(), "report.pdf".to_owned())];
        if moved_back {
            expected.push(("report.pdf".into(), "report (1).pdf".into()));
        }
        assert_eq!(*ports.renamed.lock().unwrap(), expected, "{drop:?}");
        if drop != FirstNameDrop::BothNames {
            assert!(
                done.notes.iter().all(|n| !n.contains("Both names")),
                "{drop:?}: {:?}",
                done.notes
            );
        }
    }
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
            ReplaceOutcome::RemovedNotMoved {
                still_named: Some(true),
            },
            true,
            Some(kept_own_name("report (1).pdf")),
        ),
        // A name that is gone is never named (adversarial review H1), nor one that could not be
        // read (re-review LOW 1).
        (
            ReplaceOutcome::RemovedNotMoved {
                still_named: Some(false),
            },
            true,
            Some(REMOVED_NEW_GONE.into()),
        ),
        (
            ReplaceOutcome::RemovedNotMoved { still_named: None },
            true,
            Some(REMOVED_NOT_MOVED.into()),
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
    async fn reveal(
        &self,
        path: &Path,
        _expected: &SavedFile,
    ) -> Result<(), private_downloads::RevealError> {
        self.revealed.lock().unwrap().push(path.to_owned());
        if self.reveal_fails {
            Err(private_downloads::RevealError::Launch)
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
        assert!(list.record_default(SavedFile {
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
