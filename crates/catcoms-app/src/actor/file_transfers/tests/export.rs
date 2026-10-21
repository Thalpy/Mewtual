//! Journaled external export through the real actor command boundary, using the same sequence
//! native runs: storage-crate `ExportFile` effects outside the actor, plaintext from
//! `ReadDownloadChunk`, and only sealed journal transitions on the actor.
use super::*;
use crate::actor::{DownloadExportEdge, DownloadExportTransition};
use catcoms_storage::kept::{
    remove_partial, ExportFile, ExportHalt, ExportReopen, ExportState, Publication,
};
use std::path::PathBuf;

struct Running {
    actor: ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
    cid: Cid,
    data: Vec<u8>,
    origin: RequestCancellation,
    signal: watch::Sender<bool>,
    _vault: tempfile::TempDir,
    downloads: tempfile::TempDir,
}
impl Running {
    async fn close(self) {
        self.actor.shutdown().await;
        bounded(self.task).await.unwrap();
        bounded(self.drain).await.unwrap();
    }
}

/// A holder with the file locally and the native protected store, so acquisition is local.
async fn running() -> Running {
    let Pair {
        mut holder,
        cid,
        data,
        entered: _entered,
        ..
    } = pair().await;
    let vault = tempfile::tempdir().unwrap();
    let _store = attach_protected_kept(&mut holder, vault.path());
    let (actor, mut events, task) = spawn(holder);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (signal, watch) = watch::channel(false);
    Running {
        actor,
        task,
        drain,
        cid,
        data,
        origin: RequestCancellation::new(watch, None),
        signal,
        _vault: vault,
        downloads: tempfile::tempdir().unwrap(),
    }
}

async fn ready(run: &Running, resume: Option<catcoms_storage::kept::DownloadId>) -> DownloadReady {
    bounded(
        run.actor
            .acquire_download(run.cid.as_bytes().to_vec(), resume, run.origin.clone()),
    )
    .await
    .unwrap()
    .with_export_custody(Arc::new(()))
}

async fn begin(run: &Running, ready: &DownloadReady) -> ExportFile {
    let file = ExportFile::create(
        run.downloads.path(),
        "cat.bin",
        ready.export_plan().unwrap(),
    )
    .unwrap();
    bounded(run.actor.download_export_transition(
        ready.clone(),
        DownloadExportTransition::Begin {
            journal: file.journal().clone(),
            presence: file.presence(),
        },
        run.origin.clone(),
    ))
    .await
    .unwrap();
    file
}

async fn write_all(run: &Running, ready: &DownloadReady, file: &mut ExportFile) {
    while let Some(index) = file.next_index() {
        let chunk = bounded(run.actor.read_download_chunk(
            ready.clone(),
            index,
            run.origin.clone(),
        ))
        .await
        .unwrap()
        .into_bytes();
        file.accept(index, &chunk).unwrap();
    }
}

/// Checkpoint (with the chosen final name) before the link, then the link, then Published.
async fn publish(run: &Running, ready: &DownloadReady, file: &mut ExportFile) -> PathBuf {
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!("not yet published");
    };
    bounded(run.actor.download_export_transition(
        ready.clone(),
        DownloadExportTransition::Checkpoint {
            expected: file.journal().clone(),
            next: journal.clone(),
        },
        run.origin.clone(),
    ))
    .await
    .unwrap();
    file.checkpointed(journal);
    let path = file.link().unwrap();
    bounded(run.actor.download_export_transition(
        ready.clone(),
        DownloadExportTransition::Publish {
            expected: file.published_journal(),
        },
        run.origin.clone(),
    ))
    .await
    .unwrap();
    path
}

async fn saved_state(run: &Running) -> Option<ExportState> {
    let id = bounded(run.actor.downloads()).await.unwrap()[0].id;
    bounded(run.actor.download_export_state(id)).await.unwrap()
}

#[tokio::test]
async fn journaled_export_through_the_actor_publishes_the_verified_plaintext_once() {
    let run = running().await;
    let ready = ready(&run, None).await;
    let mut file = begin(&run, &ready).await;
    assert!(matches!(
        bounded(run.actor.downloads()).await.unwrap()[0].export,
        catcoms_storage::kept::ExportView::Writing { active: true, .. }
    ));
    write_all(&run, &ready, &mut file).await;
    let path = publish(&run, &ready, &mut file).await;
    file.finish();
    assert_eq!(std::fs::read(&path).unwrap(), run.data);
    assert_eq!(path.file_name().unwrap(), "cat.bin");
    bounded(run.actor.finish_download_export(ready.clone()))
        .await
        .unwrap();
    assert!(bounded(run.actor.downloads()).await.unwrap().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), run.data);
    run.close().await;
}

#[tokio::test]
async fn journal_writes_after_lock_are_refused_even_though_the_lease_is_live() {
    let run = running().await;
    let ready = ready(&run, None).await;
    let mut file = begin(&run, &ready).await;
    write_all(&run, &ready, &mut file).await; // a blocking write that completes...
    let before = saved_state(&run).await;
    run.signal.send_replace(true); // ...after Lock retired the original invocation
    assert!(
        ready.is_current(),
        "Lock does not retire the storage lease itself"
    );
    let checkpoint = file.checkpoint().unwrap();
    assert!(bounded(run.actor.download_export_transition(
        ready.clone(),
        DownloadExportTransition::Checkpoint {
            expected: file.journal().clone(),
            next: checkpoint,
        },
        run.origin.clone(),
    ))
    .await
    .is_err());
    let id = ready.id();
    assert!(bounded(run.actor.download_export_edge(
        id,
        file.journal().clone(),
        DownloadExportEdge::Halt(ExportHalt::Unavailable),
        run.origin.clone(),
    ))
    .await
    .is_err());
    assert_eq!(
        saved_state(&run).await,
        before,
        "no record write after Lock"
    );
    run.close().await;
}

#[tokio::test]
async fn pause_mid_export_stops_effects_and_resume_continues_the_same_partial_file() {
    let run = running().await;
    let first = ready(&run, None).await;
    let file = begin(&run, &first).await;
    let journal = file.journal().clone();
    drop(file);
    let saved = bounded(run.actor.downloads()).await.unwrap().remove(0);
    bounded(
        run.actor
            .set_download_paused(saved.id, saved.state.intent, true),
    )
    .await
    .unwrap();
    assert!(!first.is_current());
    assert!(bounded(
        run.actor
            .read_download_chunk(first.clone(), 0, run.origin.clone())
    )
    .await
    .is_err());
    assert!(bounded(run.actor.download_export_transition(
        first.clone(),
        DownloadExportTransition::Checkpoint {
            expected: journal.clone(),
            next: journal.clone(),
        },
        run.origin.clone(),
    ))
    .await
    .is_err());
    // Explicit Resume: intent CAS, then fresh re-verification and the SAME partial file.
    bounded(
        run.actor
            .set_download_paused(saved.id, saved.state.intent + 1, false),
    )
    .await
    .unwrap();
    let second = ready(&run, Some(saved.id)).await;
    let Some(ExportState::Writing(saved_journal)) = saved_state(&run).await else {
        panic!("journal survives Pause");
    };
    let ExportReopen::Ready(mut file) =
        ExportFile::reopen(&saved_journal, second.export_plan().unwrap()).unwrap()
    else {
        panic!("our partial reopens");
    };
    bounded(run.actor.download_export_transition(
        second.clone(),
        DownloadExportTransition::Resume {
            journal: saved_journal,
            presence: file.presence(),
        },
        run.origin.clone(),
    ))
    .await
    .unwrap();
    write_all(&run, &second, &mut file).await;
    let path = publish(&run, &second, &mut file).await;
    // Native finishes the session (releasing its file lock) before anything reads the file.
    file.finish();
    assert_eq!(
        path.file_name().unwrap(),
        "cat.bin",
        "no numbered duplicate"
    );
    assert_eq!(std::fs::read(path).unwrap(), run.data);
    run.close().await;
}

#[tokio::test]
async fn cancel_racing_publication_resolves_to_exactly_one_outcome() {
    for publication_first in [true, false] {
        let run = running().await;
        let ready = ready(&run, None).await;
        let mut file = begin(&run, &ready).await;
        write_all(&run, &ready, &mut file).await;
        let id = ready.id();
        if publication_first {
            let path = publish(&run, &ready, &mut file).await;
            let Some(ExportState::Published(journal)) =
                bounded(run.actor.cancel_download(id)).await.unwrap()
            else {
                panic!("Remove after publication keeps the saved journal for cleanup");
            };
            drop(file);
            remove_partial(&journal).unwrap();
            bounded(run.actor.finish_download_cleanup(id))
                .await
                .unwrap();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                run.data,
                "the saved file stays"
            );
            assert!(!run.downloads.path().join("cat.bin.mewtual-part").exists());
        } else {
            let Some(ExportState::Writing(journal)) =
                bounded(run.actor.cancel_download(id)).await.unwrap()
            else {
                panic!("Remove keeps the journal until external cleanup");
            };
            // Native checks the original invocation and lease before linking; Remove won.
            assert!(!ready.is_current());
            assert!(bounded(run.actor.download_export_transition(
                ready.clone(),
                DownloadExportTransition::Publish {
                    expected: file.published_journal(),
                },
                run.origin.clone(),
            ))
            .await
            .is_err());
            drop(file);
            remove_partial(&journal).unwrap();
            bounded(run.actor.finish_download_cleanup(id))
                .await
                .unwrap();
            assert_eq!(std::fs::read_dir(run.downloads.path()).unwrap().count(), 0);
        }
        assert!(bounded(run.actor.downloads()).await.unwrap().is_empty());
        run.close().await;
    }
}

#[tokio::test]
async fn save_new_copy_and_replacement_transfers_release_stale_export_results() {
    let run = running().await;
    let old = ready(&run, None).await;
    let file = begin(&run, &old).await;
    let journal = file.journal().clone();
    let saved = bounded(run.actor.downloads()).await.unwrap().remove(0);
    remove_partial(&journal).unwrap();
    let intent = bounded(
        run.actor
            .abandon_download_export(saved.id, saved.state.intent),
    )
    .await
    .unwrap();
    assert_eq!(intent, saved.state.intent + 1);
    assert!(
        !old.is_current(),
        "Save new copy retires the in-flight export"
    );
    assert!(bounded(run.actor.download_export_transition(
        old.clone(),
        DownloadExportTransition::Checkpoint {
            expected: journal.clone(),
            next: journal.clone(),
        },
        run.origin.clone(),
    ))
    .await
    .is_err());
    drop(file);
    // Remove (no journal now) and start a replacement Download of the same file.
    assert!(bounded(run.actor.cancel_download(saved.id))
        .await
        .unwrap()
        .is_none());
    let replacement = ready(&run, None).await;
    assert_ne!(replacement.id(), old.id());
    assert!(bounded(
        run.actor.download_export_transition(
            old.clone(),
            DownloadExportTransition::Begin {
                journal: journal.clone(),
                presence: ExportFile::create(
                    run.downloads.path(),
                    "other.bin",
                    old.export_plan().unwrap()
                )
                .unwrap()
                .presence(),
            },
            run.origin.clone(),
        )
    )
    .await
    .is_err());
    let mut file = begin(&run, &replacement).await;
    write_all(&run, &replacement, &mut file).await;
    let path = publish(&run, &replacement, &mut file).await;
    file.finish();
    assert_eq!(std::fs::read(path).unwrap(), run.data);
    run.close().await;
}

#[tokio::test]
async fn actor_freeze_and_thaw_release_stale_export_transitions() {
    let Pair {
        mut holder,
        cid,
        entered: _entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    let store = attach_protected_kept(&mut holder, dir.path());
    let vault = Arc::new(tokio::sync::Mutex::new(Some(store)));
    let (actor, mut events, task) = spawn(holder);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_signal, watch) = watch::channel(false);
    let origin = RequestCancellation::new(watch, None);
    let ready = bounded(actor.acquire_download(cid.as_bytes().to_vec(), None, origin.clone()))
        .await
        .unwrap()
        .with_export_custody(Arc::new(()));
    let downloads = tempfile::tempdir().unwrap();
    let file =
        ExportFile::create(downloads.path(), "cat.bin", ready.export_plan().unwrap()).unwrap();
    let preparing = bounded(actor.prepare_shutdown()).await.unwrap();
    let frozen = bounded(
        preparing.save_and_freeze(crate::shutdown::ShutdownLease::new(
            vault.lock_owned().await,
            78,
            (),
        )),
    )
    .await
    .unwrap();
    drop(frozen);
    assert!(bounded(actor.download_export_transition(
        ready.clone(),
        DownloadExportTransition::Begin {
            journal: file.journal().clone(),
            presence: file.presence(),
        },
        origin.clone(),
    ))
    .await
    .is_err());
    let id = ready.id();
    assert!(bounded(actor.download_export_state(id))
        .await
        .unwrap()
        .is_none());
    actor.shutdown().await;
    bounded(task).await.unwrap();
    bounded(drain).await.unwrap();
}
