//! Journaled external export: storage transitions plus real-filesystem `ExportFile` behavior.
//! Each crash-table row from EXTERNAL-DOWNLOAD-RESUME.md is modelled by stopping at that step,
//! dropping the store (process restart) and reopening the same vault directory.
use super::super::external::{self, Effect, INJECT};
use super::*;
use crate::MemoryBlobStore;
use catcoms_crypto::DeviceId;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[path = "export_chosen_tests.rs"]
mod chosen;

const CHUNKS: [&[u8]; 2] = [b"alpha chunk one!", b"bravo chunk two"];

fn scope() -> KeepScope {
    KeepScope {
        group: vec![3; 32],
        device: DeviceId::from_bytes([4; 32]),
    }
}
fn open(path: &Path) -> KeptBlobStore<ChaCha20Rng> {
    let mut store = KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        path.to_owned(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    store.configure_keep_scope(scope()).unwrap();
    store
}
fn plan() -> KeepPlan {
    KeepPlan {
        cid: Cid::of(b"firstsecond"),
        version: [1; 32],
        manifest: b"opaque manifest".to_vec(),
        chunks: vec![(Cid::of(b"first"), 5), (Cid::of(b"second"), 6)],
    }
}
fn export_plan() -> ExportPlan {
    ExportPlan::for_test(&CHUNKS)
}
fn verify_private(store: &mut KeptBlobStore<ChaCha20Rng>, run: DownloadRun) -> DownloadExportLease {
    store.put_download(run, b"first").unwrap();
    store.put_download(run, b"second").unwrap();
    store.finish_download(run).unwrap()
}
/// A completed, whole-file-verified private copy and its export lease.
fn completed(store: &mut KeptBlobStore<ChaCha20Rng>) -> (DownloadId, DownloadExportLease) {
    let run = store.begin_download(vec![plan()], 0, [9; 32]).unwrap();
    (run.id(), verify_private(store, run))
}
/// Fresh mount: explicit Resume re-verifies the private copy and issues a new lease.
fn reverified(store: &mut KeptBlobStore<ChaCha20Rng>, id: DownloadId) -> DownloadExportLease {
    let run = store.resume_download(id).unwrap();
    verify_private(store, run)
}
struct Fixture {
    _root: tempfile::TempDir,
    vault: PathBuf,
    downloads: PathBuf,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let vault = root.path().join("vault");
    let downloads = root.path().join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    Fixture {
        _root: root,
        vault,
        downloads,
    }
}
fn started(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    lease: &DownloadExportLease,
    downloads: &Path,
) -> ExportFile {
    let file = ExportFile::create(downloads, "notes.txt", export_plan()).unwrap();
    store
        .begin_download_export(lease, file.journal(), file.presence())
        .unwrap();
    file
}
fn write_all(file: &mut ExportFile) {
    for (index, chunk) in CHUNKS.iter().enumerate() {
        file.accept(index, chunk).unwrap();
    }
}
/// The native publication sequence: checkpoint (with any new final name) BEFORE the link, the
/// link, then the Published transition.
fn publish(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    lease: &DownloadExportLease,
    file: &mut ExportFile,
) -> PathBuf {
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!("not yet published");
    };
    store
        .checkpoint_download_export(lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal);
    let path = file.link().unwrap();
    store
        .publish_download_export(lease, &file.published_journal())
        .unwrap();
    path
}
fn state(store: &KeptBlobStore<ChaCha20Rng>, id: DownloadId) -> ExportState {
    store.download_export_state(id).unwrap().unwrap()
}
fn view(store: &KeptBlobStore<ChaCha20Rng>) -> ExportView {
    store.downloads().unwrap()[0].export
}
fn staging(downloads: &Path) -> PathBuf {
    downloads.join("notes.txt.mewtual-part")
}
fn injected<T>(effect: Effect, kind: io::ErrorKind, run: impl FnOnce() -> T) -> T {
    INJECT.with(|slot| slot.set(Some((effect, kind, 1))));
    let result = run();
    INJECT.with(|slot| slot.set(None));
    result
}

#[test]
fn fresh_export_publishes_without_replacement_and_releases_only_private_progress() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    assert!(matches!(
        view(&store),
        ExportView::Writing {
            saved_chunks: 0,
            halt: None,
            active: true
        }
    ));
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    file.finish();
    assert_eq!(path, fx.downloads.join("notes.txt"));
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
    assert!(
        !staging(&fx.downloads).exists(),
        "the staging name is removed after publication"
    );
    assert_eq!(view(&store), ExportView::Published);
    store.finish_download_export(&lease).unwrap();
    assert!(store.downloads().unwrap().is_empty());
    assert_eq!(
        fs::read(&path).unwrap(),
        CHUNKS.concat(),
        "the saved file is never removed"
    );
    store
        .finish_download_export(&lease)
        .expect("finishing an already released record is a no-op");
    let _ = id;
}

#[test]
fn journaled_records_stay_private_and_survive_remount_as_download_v2() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let _file = started(&mut store, &lease, &fx.downloads);
    let index = store.download_index(id).unwrap();
    assert_eq!(
        store.transfers[index].record.encoding,
        record::RecordEncoding::DownloadV2
    );
    let assert_private = |store: &KeptBlobStore<ChaCha20Rng>| {
        assert!(store.kept_files().files.is_empty() && store.kept_files().transfers.is_empty());
        assert!(store.kept_saved_plans(&plan().cid).is_none());
        assert!(store.automatic_keep_candidate(None).is_none());
        for (cid, bound) in plan().chunks {
            assert!(store.get(&cid).unwrap().is_none());
            assert!(!store.has(&cid));
            assert!(store
                .prepare_read_bounded(&cid, bound as usize)
                .unwrap()
                .run()
                .unwrap()
                .is_none());
        }
        assert!(store.cids().is_empty());
    };
    assert_private(&store);
    drop(store);
    let store = open(&fx.vault);
    assert_private(&store);
    assert!(
        matches!(
            view(&store),
            ExportView::Writing {
                active: false,
                halt: None,
                ..
            }
        ),
        "presence is runtime-only and ends with the process"
    );
}

#[test]
fn a_second_writer_is_refused_by_the_file_lock_even_without_store_presence() {
    // Review M1: the store's presence token is per mount. A session stalled on a hung share can
    // outlive Lock and remount; the OS lock on its handle must still refuse a second writer.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let mut first = started(&mut store, &lease, &fx.downloads);
    first.accept(0, CHUNKS[0]).unwrap();
    let journal = first.journal().clone();
    assert!(matches!(
        ExportFile::reopen(&journal, export_plan()),
        Err(ExportError::Busy)
    ));
    drop(first);
    assert_eq!(
        fs::metadata(staging(&fx.downloads)).unwrap().len(),
        CHUNKS[0].len() as u64,
        "the refused reopen touched nothing"
    );
    let ExportReopen::Ready(second) = ExportFile::reopen(&journal, export_plan()).unwrap() else {
        panic!("the partial reopens once the first session is gone");
    };
    // And the second session now excludes any third.
    assert!(matches!(
        ExportFile::reopen(&journal, export_plan()),
        Err(ExportError::Busy)
    ));
    drop(second);
}

#[test]
fn crash_after_exclusive_create_leaves_an_unadopted_orphan_and_a_fresh_name() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, _lease) = completed(&mut store);
    let orphan = ExportFile::create(&fx.downloads, "notes.txt", export_plan()).unwrap();
    drop(orphan); // crash before Writing(0)
    drop(store);
    let mut store = open(&fx.vault);
    assert!(store.download_export_state(id).unwrap().is_none());
    let lease = reverified(&mut store, id);
    let file = started(&mut store, &lease, &fx.downloads);
    assert_eq!(file.journal().staging, "notes (1).txt.mewtual-part");
    assert!(
        staging(&fx.downloads).exists(),
        "the orphan is neither adopted nor deleted"
    );
    let _ = lease;
}

#[test]
fn torn_or_extra_tail_after_the_checkpoint_is_verified_then_rewritten() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file); // crash before any checkpoint: chunk 0 is beyond the durable range
    let mut torn = fs::OpenOptions::new()
        .append(true)
        .open(staging(&fx.downloads))
        .unwrap();
    std::io::Write::write_all(&mut torn, &[0; 4]).unwrap(); // a torn start of chunk 1
    drop(torn);
    drop(store);
    let mut store = open(&fx.vault);
    let ExportState::Writing(journal) = state(&store, id) else {
        panic!("still writing");
    };
    assert_eq!(journal.written_chunks(), 0);
    let lease = reverified(&mut store, id);
    let ExportReopen::Ready(mut file) = ExportFile::reopen(&journal, export_plan()).unwrap() else {
        panic!("our partial reopens");
    };
    store
        .resume_download_export(&lease, &journal, file.presence())
        .unwrap();
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    file.finish(); // releases the session lock, which on Windows also blocks other readers
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn checkpoint_claiming_more_than_the_file_rewinds_but_a_changed_byte_inside_it_halts() {
    for change in [false, true] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        write_all(&mut file);
        let checkpoint = file.checkpoint().unwrap();
        store
            .checkpoint_download_export(&lease, file.journal(), &checkpoint)
            .unwrap();
        drop(file);
        let partial = staging(&fx.downloads);
        let mut bytes = fs::read(&partial).unwrap();
        if change {
            bytes[1] ^= 0xff; // a modification inside the durable checkpoint
        } else {
            bytes.truncate(CHUNKS[0].len() + 3); // a lost tail (torn write or lying flush)
        }
        let modified = bytes.clone();
        // Rewrite in place so the file keeps its identity.
        let mut handle = fs::OpenOptions::new().write(true).open(&partial).unwrap();
        handle.set_len(0).unwrap();
        std::io::Write::write_all(&mut handle, &bytes).unwrap();
        drop(handle);
        drop(store);
        let mut store = open(&fx.vault);
        let lease = reverified(&mut store, id);
        let ExportReopen::Ready(mut file) = ExportFile::reopen(&checkpoint, export_plan()).unwrap()
        else {
            panic!("identity still matches");
        };
        if change {
            assert!(matches!(
                file.accept(0, CHUNKS[0]),
                Err(ExportError::Halted(ExportHalt::Changed))
            ));
            drop(file);
            assert_eq!(fs::read(&partial).unwrap(), modified, "nothing is repaired");
            store
                .halt_download_export(id, &checkpoint, ExportHalt::Changed)
                .unwrap();
            assert!(matches!(
                view(&store),
                ExportView::Writing {
                    halt: Some(ExportHalt::Changed),
                    ..
                }
            ));
        } else {
            write_all(&mut file);
            let path = publish(&mut store, &lease, &mut file);
            file.finish();
            assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
        }
    }
}

#[test]
fn verification_never_lowers_a_checkpoint_but_a_rewind_does() {
    for truncated in [false, true] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        write_all(&mut file);
        let full = file.checkpoint().unwrap();
        store
            .checkpoint_download_export(&lease, file.journal(), &full)
            .unwrap();
        drop(file);
        if truncated {
            // Lost tail shorter than the first chunk: the session must rewrite from zero.
            let handle = fs::OpenOptions::new()
                .write(true)
                .open(staging(&fx.downloads))
                .unwrap();
            handle.set_len(3).unwrap();
        }
        let ExportReopen::Ready(mut file) = ExportFile::reopen(&full, export_plan()).unwrap()
        else {
            panic!();
        };
        file.accept(0, CHUNKS[0]).unwrap();
        let mid = file.checkpoint().unwrap();
        if truncated {
            assert_eq!(
                mid.written_chunks(),
                1,
                "after a rewind the lower checkpoint is the truth"
            );
        } else {
            assert_eq!(
                mid, full,
                "verifying chunk 0 must not lower the durable checkpoint of chunks 0 and 1"
            );
        }
        let _ = (id, lease);
    }
}

#[test]
fn identity_collision_over_foreign_content_halts_without_writing() {
    // Birth time makes an accidental (dev, ino) reuse infeasible; this forges the collision to
    // pin the content guard: a mismatch inside the checkpoint halts and writes nothing. Identity
    // is the only ownership proof for deletion, so a real collision WOULD let Remove delete this
    // file; that residual is documented in THREAT-MODEL rather than claimed away here.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let file = started(&mut store, &lease, &fx.downloads);
    let mut journal = file.journal().clone();
    drop(file);
    fs::remove_file(staging(&fx.downloads)).unwrap();
    let foreign = b"another program's download, same length".to_vec();
    fs::write(staging(&fx.downloads), &foreign).unwrap();
    let Leaf::Opened(_, observed) = external::open_leaf(&staging(&fx.downloads), false).unwrap()
    else {
        panic!("foreign file opens");
    };
    journal.file_id = observed.identity;
    journal.written_chunks = 1;
    journal.written_bytes = CHUNKS[0].len() as u64;
    let ExportReopen::Ready(mut forged) = ExportFile::reopen(&journal, export_plan()).unwrap()
    else {
        panic!("forged identity matches");
    };
    assert!(matches!(
        forged.accept(0, CHUNKS[0]),
        Err(ExportError::Halted(ExportHalt::Changed))
    ));
    drop(forged);
    assert_eq!(fs::read(staging(&fx.downloads)).unwrap(), foreign);
    let _ = id;
}

#[test]
fn destination_changes_while_paused_become_typed_halts_without_touching_other_files() {
    #[derive(Debug, Clone, Copy)]
    enum Change {
        Deleted,
        Moved,
        Replaced,
        Linked,
        FolderGone,
        FolderReplaced,
        PermissionRevoked,
    }
    for change in [
        Change::Deleted,
        Change::Moved,
        Change::Replaced,
        Change::Linked,
        Change::FolderGone,
        Change::FolderReplaced,
        Change::PermissionRevoked,
    ] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        file.accept(0, CHUNKS[0]).unwrap();
        let journal = file.journal().clone();
        drop(file);
        let partial = staging(&fx.downloads);
        let mut keep = None;
        let expected = match change {
            Change::Deleted => {
                fs::remove_file(&partial).unwrap();
                ExportHalt::Unavailable
            }
            Change::Moved => {
                fs::rename(&partial, fx.downloads.join("moved.bin")).unwrap();
                ExportHalt::Unavailable
            }
            Change::Replaced => {
                fs::remove_file(&partial).unwrap();
                fs::write(&partial, b"user file").unwrap();
                keep = Some((partial.clone(), b"user file".to_vec()));
                ExportHalt::Changed
            }
            Change::Linked => {
                fs::remove_file(&partial).unwrap();
                let target = fx.downloads.join("target");
                fs::create_dir(&target).unwrap();
                if !external::tests::link_directory(&target, &partial) {
                    eprintln!("skipping link replacement: no link or junction support");
                    continue;
                }
                ExportHalt::Changed
            }
            Change::FolderGone => {
                fs::rename(&fx.downloads, fx.downloads.with_file_name("Elsewhere")).unwrap();
                ExportHalt::Unavailable
            }
            Change::FolderReplaced => {
                fs::rename(&fx.downloads, fx.downloads.with_file_name("Old")).unwrap();
                fs::create_dir(&fx.downloads).unwrap();
                ExportHalt::Changed
            }
            Change::PermissionRevoked => {
                let mut permissions = fs::metadata(&partial).unwrap().permissions();
                permissions.set_readonly(true);
                fs::set_permissions(&partial, permissions).unwrap();
                ExportHalt::Unavailable
            }
        };
        assert!(
            matches!(ExportFile::probe(&journal), ExportProbe::Halt(halt) if halt == expected),
            "{change:?}"
        );
        assert!(
            matches!(
                ExportFile::reopen(&journal, export_plan()).unwrap(),
                ExportReopen::Halt(halt) if halt == expected
            ),
            "{change:?}"
        );
        store.halt_download_export(id, &journal, expected).unwrap();
        // Removal of a halted record never deletes anything it cannot prove is ours, and says
        // which: FolderUnreachable keeps Removing; the others finish, with a notice whenever a
        // file stays on disk.
        let ExportState::Writing(halted) = store.cancel_download(id).unwrap().unwrap() else {
            panic!("journal is kept for cleanup");
        };
        let outcome = remove_partial(&halted).unwrap();
        let expected_outcome = match change {
            Change::Deleted | Change::Moved | Change::PermissionRevoked => PartialRemoval::Removed,
            Change::Replaced | Change::Linked => PartialRemoval::LeftInPlace,
            Change::FolderReplaced => PartialRemoval::FolderChanged,
            Change::FolderGone => PartialRemoval::FolderUnreachable,
        };
        assert_eq!(outcome, expected_outcome, "{change:?}");
        // A file left on disk is never reported as removed.
        assert_eq!(
            outcome.notice(false).is_some(),
            matches!(
                change,
                Change::Replaced | Change::Linked | Change::FolderReplaced
            ),
            "{change:?}"
        );
        store.finish_download_cleanup(id).unwrap();
        if let Some((path, bytes)) = keep {
            assert_eq!(fs::read(path).unwrap(), bytes, "{change:?}");
        }
        if matches!(change, Change::PermissionRevoked) {
            assert!(
                !partial.exists(),
                "a read-only partial that is provably ours is still removed"
            );
        }
        let _ = lease;
    }
}

#[test]
fn concurrent_truncation_is_a_conflict_never_a_zero_filled_hole() {
    for at_publication in [false, true] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (_id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        file.accept(0, CHUNKS[0]).unwrap();
        if at_publication {
            file.accept(1, CHUNKS[1]).unwrap();
        }
        // Another writer (for example a second session after Lock and Resume) shrinks the
        // partial while this session's handle is still open.
        let other = fs::OpenOptions::new()
            .write(true)
            .open(staging(&fx.downloads))
            .unwrap();
        other.set_len(5).unwrap();
        drop(other);
        let outcome = if at_publication {
            file.prepare_publication().map(|_| ())
        } else {
            file.accept(1, CHUNKS[1]).map(|_| ())
        };
        assert!(
            matches!(outcome, Err(ExportError::Conflict)),
            "at_publication={at_publication}"
        );
        assert_eq!(
            fs::metadata(staging(&fx.downloads)).unwrap().len(),
            5,
            "the file is never zero-extended over the truncated range"
        );
    }
}

#[test]
fn a_live_older_session_blocks_a_second_writer_until_it_is_dropped() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let older = started(&mut store, &lease, &fx.downloads);
    let journal = older.journal().clone();
    let intent = store.downloads().unwrap()[0].state.intent;
    store.set_download_paused(id, intent, true).unwrap();
    assert!(matches!(
        view(&store),
        ExportView::Writing { active: false, .. }
    ));
    store.set_download_paused(id, intent + 1, false).unwrap();
    let lease = reverified(&mut store, id);
    // Two layers: the OS lock refuses a second handle onto the partial outright, and the store's
    // presence refuses any other session's registration while the older one is alive.
    assert!(matches!(
        ExportFile::reopen(&journal, export_plan()),
        Err(ExportError::Busy)
    ));
    let elsewhere = fx.downloads.with_file_name("Elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let unrelated = ExportFile::create(&elsewhere, "other.txt", export_plan()).unwrap();
    assert!(
        store
            .resume_download_export(&lease, &journal, unrelated.presence())
            .is_err(),
        "the paused session still owns a live handle onto the partial"
    );
    drop(older);
    let ExportReopen::Ready(newer) = ExportFile::reopen(&journal, export_plan()).unwrap() else {
        panic!();
    };
    store
        .resume_download_export(&lease, &journal, newer.presence())
        .unwrap();
    assert!(matches!(
        view(&store),
        ExportView::Writing { active: true, .. }
    ));
}

#[cfg(windows)]
#[test]
fn removal_reports_a_partial_it_cannot_open_instead_of_forgetting_it() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    // Another program holds the partial without sharing.
    let holder = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(staging(&fx.downloads))
        .unwrap();
    let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    assert_eq!(remove_partial(&journal).unwrap(), PartialRemoval::Unchecked);
    drop(holder);
    assert!(
        staging(&fx.downloads).exists(),
        "nothing was deleted or forgotten"
    );
    assert_eq!(remove_partial(&journal).unwrap(), PartialRemoval::Removed);
    store.finish_download_cleanup(id).unwrap();
}

#[cfg(windows)]
#[test]
fn a_compressed_or_cloud_tagged_partial_probes_ready_and_is_removed() {
    // Review M2: a non-surrogate reparse point (WOF here; OneDrive placeholders and dedup use the
    // same mechanism) must neither halt as Changed nor let Remove leave the plaintext behind.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let file = started(&mut store, &lease, &fx.downloads);
    let journal = file.journal().clone();
    drop(file);
    // Grow it in place (same identity) so there is something to compress.
    let mut handle = fs::OpenOptions::new()
        .append(true)
        .open(staging(&fx.downloads))
        .unwrap();
    std::io::Write::write_all(&mut handle, &vec![b'a'; 256 * 1024]).unwrap();
    drop(handle);
    if !external::tests::compact_wof(&staging(&fx.downloads)) {
        eprintln!("skipping: compact /exe did not add a reparse point on this volume");
        return;
    }
    assert!(
        matches!(ExportFile::probe(&journal), ExportProbe::Ready),
        "our own partial is not reported as changed"
    );
    let ExportState::Writing(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    assert_eq!(remove_partial(&saved).unwrap(), PartialRemoval::Removed);
    assert!(!staging(&fx.downloads).exists(), "the plaintext is gone");
    store.finish_download_cleanup(id).unwrap();
}

#[cfg(windows)]
#[test]
fn publication_pins_the_proven_staging_file_across_the_link() {
    // Review M3: between the identity check and the path-based link, another program tries to
    // move our staging file away (and could put its own file there). The pin refuses that.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal);
    let swapped = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = swapped.clone();
    let elsewhere = fx.downloads.join("moved away");
    external::BEFORE_LINK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |from: &Path| {
            seen.set(Some(fs::rename(from, &elsewhere).is_ok()));
        }))
    });
    let linked = file.link();
    external::BEFORE_LINK.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(
        swapped.get(),
        Some(false),
        "the swap was attempted and refused"
    );
    let path = linked.unwrap();
    file.finish();
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn clearing_a_saved_row_never_changes_the_saved_files_attributes() {
    // Review U-L2: hard links share attributes. A read-only saved file must not lose that
    // attribute just so the staging name can be removed.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    drop(file); // both names still present (no finish)
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let ExportState::Published(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    // Model a filesystem that refuses to delete a read-only name (older Windows std did): the
    // read-only retry must not run for a multiply-linked file, so removal stays in Removing.
    let refused = injected(Effect::Remove, io::ErrorKind::PermissionDenied, || {
        remove_partial(&saved)
    });
    assert!(refused.is_err(), "removal stays in Removing instead");
    assert!(staging(&fx.downloads).exists());
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    // Where deletion of the name is allowed, it leaves the shared attributes alone.
    assert_eq!(remove_partial(&saved).unwrap(), PartialRemoval::KeptSaved);
    assert!(!staging(&fx.downloads).exists());
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
    // Let the temporary directory clean up.
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
}

#[test]
fn removal_reports_every_name_it_leaves_behind() {
    // Review L2. (a) Remove racing publication: the link exists but Published was never
    // recorded. Only the staging name goes; the saved file stays and the user is told.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal);
    let path = file.link().unwrap();
    drop(file);
    let ExportState::Writing(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    let outcome = remove_partial(&saved).unwrap();
    assert_eq!(outcome, PartialRemoval::KeptSaved);
    assert!(outcome.notice(false).is_some() && outcome.notice(true).is_none());
    assert!(!staging(&fx.downloads).exists());
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
    store.finish_download_cleanup(id).unwrap();
    // (b) Someone else hard-linked our partial elsewhere: our name goes, theirs stays, and the
    // user is told a second name still holds the plaintext.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    let other = fx.downloads.join("copied elsewhere.bin");
    fs::hard_link(staging(&fx.downloads), &other).unwrap();
    let ExportState::Writing(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    let outcome = remove_partial(&saved).unwrap();
    assert_eq!(outcome, PartialRemoval::OtherNameRemains);
    assert!(outcome.notice(true).is_some());
    assert!(!staging(&fx.downloads).exists());
    assert_eq!(fs::read(&other).unwrap(), CHUNKS[0]);
    store.finish_download_cleanup(id).unwrap();
    let _ = lease;
}

#[test]
fn a_failed_check_after_the_link_retries_into_saved_and_recovery_clears_halt_and_pause() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal.clone());
    assert!(matches!(
        injected(Effect::Check, io::ErrorKind::Other, || file.link()),
        Err(ExportError::Local {
            retryable: true,
            ..
        })
    ));
    // The retry recognizes our own final name instead of linking again.
    let Publication::AlreadyPublished(again, path) = file.prepare_publication().unwrap() else {
        panic!("our link is recognized");
    };
    assert_eq!(again, journal);
    drop(file);
    // Even an older build's halt plus a Pause cannot strand the saved file.
    store
        .halt_download_export(id, &journal, ExportHalt::Changed)
        .unwrap();
    let ExportState::Writing(halted) = state(&store, id) else {
        panic!();
    };
    let intent = store.downloads().unwrap()[0].state.intent;
    store.set_download_paused(id, intent, true).unwrap();
    store.recover_download_export(id, &halted).unwrap();
    let saved = store.downloads().unwrap().remove(0);
    assert_eq!(saved.export, ExportView::Published);
    assert!(!saved.state.user_paused);
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
    assert!(!fx.downloads.join("notes (1).txt").exists());
}

#[test]
fn a_retry_after_an_unconfirmed_link_never_trims_or_claims_a_changed_final_file() {
    // Review U-M1: the link succeeded, its confirmation failed, and another program changed the
    // now-visible final file before the retry. The retry must not trim it and must not report
    // success for content that is not ours.
    for append in [true, false] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (_id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        write_all(&mut file);
        let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
            panic!();
        };
        store
            .checkpoint_download_export(&lease, file.journal(), &journal)
            .unwrap();
        file.checkpointed(journal);
        assert!(injected(Effect::Check, io::ErrorKind::Other, || file.link()).is_err());
        let path = fx.downloads.join("notes.txt");
        file.release_lock_for_test();
        let mut handle = fs::OpenOptions::new().write(true).open(&path).unwrap();
        let expected = if append {
            std::io::Seek::seek(&mut handle, SeekFrom::End(0)).unwrap();
            std::io::Write::write_all(&mut handle, b" appended by the user").unwrap();
            [CHUNKS.concat(), b" appended by the user".to_vec()].concat()
        } else {
            std::io::Write::write_all(&mut handle, b"X").unwrap(); // same length, one byte changed
            let mut changed = CHUNKS.concat();
            changed[0] = b'X';
            changed
        };
        drop(handle);
        assert!(
            matches!(
                file.prepare_publication(),
                Err(ExportError::Halted(ExportHalt::Changed))
            ),
            "append={append}"
        );
        // A write is refused too: the file may be visible under its final name.
        drop(file);
        assert_eq!(
            fs::read(&path).unwrap(),
            expected,
            "every final byte survives"
        );
    }
}

#[test]
fn a_partial_that_gained_another_name_is_never_written_again() {
    // Review U-M1: someone hard-linked our partial elsewhere mid-save; those bytes are visible
    // there now, so the session halts instead of truncating or extending them.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let other = fx.downloads.join("someone else's name.bin");
    fs::hard_link(staging(&fx.downloads), &other).unwrap();
    assert!(matches!(
        file.accept(1, CHUNKS[1]),
        Err(ExportError::Halted(ExportHalt::Changed))
    ));
    drop(file);
    assert_eq!(fs::read(&other).unwrap(), CHUNKS[0], "nothing was written");
}

#[test]
fn a_matching_final_identity_is_not_saved_until_its_content_is_verified() {
    // Review U-M2: the full checkpoint is durable and the app stops before the link. A local
    // program rewrites or truncates the partial in place (same identity), then links or renames it
    // to the final name. Neither the probe nor a reopen may report it saved.
    #[derive(Debug, Clone, Copy)]
    enum Tamper {
        SameLength,
        Shortened,
    }
    for (tamper, rename) in [
        (Tamper::SameLength, false),
        (Tamper::Shortened, false),
        (Tamper::SameLength, true),
    ] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        write_all(&mut file);
        let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
            panic!();
        };
        store
            .checkpoint_download_export(&lease, file.journal(), &journal)
            .unwrap();
        drop(file); // stop before the link
        let partial = staging(&fx.downloads);
        let mut handle = fs::OpenOptions::new().write(true).open(&partial).unwrap();
        match tamper {
            Tamper::SameLength => std::io::Write::write_all(&mut handle, b"X").unwrap(),
            Tamper::Shortened => handle.set_len(3).unwrap(),
        }
        drop(handle);
        let final_path = fx.downloads.join("notes.txt");
        if rename {
            fs::rename(&partial, &final_path).unwrap();
        } else {
            fs::hard_link(&partial, &final_path).unwrap();
        }
        let tampered = fs::read(&final_path).unwrap();
        drop(store);
        let mut store = open(&fx.vault);
        let ExportProbe::Published(candidate) = ExportFile::probe(&journal) else {
            panic!("the identity matches, so this is only a candidate ({tamper:?})");
        };
        assert!(matches!(
            candidate.verify(&export_plan()),
            Err(ExportError::Halted(ExportHalt::Changed))
        ));
        let lease = reverified(&mut store, id);
        assert!(matches!(
            ExportFile::reopen(&journal, export_plan()).unwrap(),
            ExportReopen::Halt(ExportHalt::Changed)
        ));
        assert_eq!(fs::read(&final_path).unwrap(), tampered, "never repaired");
        assert!(matches!(state(&store, id), ExportState::Writing(_)));
        let _ = lease;
    }
}

#[test]
fn a_straddling_chunk_inside_the_checkpoint_must_match_before_any_rewrite() {
    // Review L1: the checkpoint covers chunk 0, but the file is now 3 bytes that do not match.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let checkpoint = file.checkpoint().unwrap();
    store
        .checkpoint_download_export(&lease, file.journal(), &checkpoint)
        .unwrap();
    drop(file);
    let mut handle = fs::OpenOptions::new()
        .write(true)
        .open(staging(&fx.downloads))
        .unwrap();
    handle.set_len(0).unwrap();
    std::io::Write::write_all(&mut handle, b"zzz").unwrap();
    drop(handle);
    drop(store);
    let mut store = open(&fx.vault);
    let lease = reverified(&mut store, id);
    let ExportReopen::Ready(mut file) = ExportFile::reopen(&checkpoint, export_plan()).unwrap()
    else {
        panic!();
    };
    assert!(matches!(
        file.accept(0, CHUNKS[0]),
        Err(ExportError::Halted(ExportHalt::Changed))
    ));
    drop(file);
    assert_eq!(
        fs::read(staging(&fx.downloads)).unwrap(),
        b"zzz",
        "bytes intact"
    );
    let _ = lease;
}

#[cfg(windows)]
#[test]
fn an_occupied_final_name_that_cannot_be_opened_is_numbered_around() {
    // Review L8: a file another program holds without sharing occupies the final name.
    use std::os::windows::fs::OpenOptionsExt;
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    fs::write(fx.downloads.join("notes.txt"), b"held elsewhere").unwrap();
    let holder = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(fx.downloads.join("notes.txt"))
        .unwrap();
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    assert_eq!(journal.final_leaf, "notes (1).txt");
    drop(holder);
    drop(file);
}

#[test]
fn disk_full_while_writing_persists_nothing_and_a_later_attempt_completes() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    let before = state(&store, id);
    assert!(matches!(
        injected(Effect::Write, io::ErrorKind::StorageFull, || file
            .accept(0, CHUNKS[0])),
        Err(ExportError::Full)
    ));
    assert_eq!(
        state(&store, id),
        before,
        "no record write on a full volume"
    );
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    file.finish();
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn a_folder_without_links_is_refused_at_creation_and_halts_at_publication() {
    // FAT and exFAT report identities but cannot link, and every publication (journaled or the
    // non-resumable writer) is a no-replace link: refuse before any plaintext exists. Linux vfat
    // reports the failure as a permission error, so the kind does not matter.
    for kind in [io::ErrorKind::Unsupported, io::ErrorKind::PermissionDenied] {
        let fx = fixture();
        assert!(matches!(
            injected(Effect::Link, kind, || {
                ExportFile::create(&fx.downloads, "notes.txt", export_plan())
            }),
            Err(ExportError::NoLinks)
        ));
        assert_eq!(
            fs::read_dir(&fx.downloads).unwrap().count(),
            0,
            "a refused destination keeps nothing behind"
        );
    }
    // A folder without stable identities still selects the non-resumable writer.
    let fx = fixture();
    assert!(matches!(
        injected(Effect::Identity, io::ErrorKind::Unsupported, || {
            ExportFile::create(&fx.downloads, "notes.txt", export_plan())
        }),
        Err(ExportError::Unsupported)
    ));
    assert_eq!(fs::read_dir(&fx.downloads).unwrap().count(), 0);
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal);
    assert!(matches!(
        injected(Effect::Link, io::ErrorKind::Unsupported, || file.link()),
        Err(ExportError::Halted(ExportHalt::Unavailable))
    ));
}

#[test]
fn occupied_final_name_is_renamed_durably_before_linking_and_never_replaced() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    fs::write(
        fx.downloads.join("notes.txt"),
        b"made by the user meanwhile",
    )
    .unwrap();
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    assert_eq!(journal.final_leaf, "notes (1).txt");
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    drop(file); // crash after the new name is durable, before the link
    drop(store);
    let mut store = open(&fx.vault);
    let lease = reverified(&mut store, id);
    let ExportReopen::Ready(mut file) = ExportFile::reopen(&journal, export_plan()).unwrap() else {
        panic!();
    };
    store
        .resume_download_export(&lease, &journal, file.presence())
        .unwrap();
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    assert_eq!(path, fx.downloads.join("notes (1).txt"));
    assert_eq!(
        fs::read(fx.downloads.join("notes.txt")).unwrap(),
        b"made by the user meanwhile"
    );
}

#[test]
fn crash_between_link_and_published_recovers_to_saved_without_a_second_copy() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!();
    };
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal.clone());
    file.link().unwrap();
    // A retried publication in the same session sees its own earlier link and never links again.
    let Publication::AlreadyPublished(again, linked) = file.prepare_publication().unwrap() else {
        panic!("our final name is recognized");
    };
    assert_eq!(again, journal);
    assert_eq!(linked, fx.downloads.join("notes.txt"));
    drop(file); // crash before Published
    drop(store);
    let mut store = open(&fx.vault);
    let ExportProbe::Published(candidate) = ExportFile::probe(&journal) else {
        panic!("the published final name is found before any re-verification");
    };
    let path = candidate
        .verify(&export_plan())
        .expect("the genuine saved file passes read-only verification");
    store.recover_download_export(id, &journal).unwrap();
    assert_eq!(view(&store), ExportView::Published);
    assert!(
        !fx.downloads.join("notes (1).txt").exists(),
        "no second copy"
    );
    assert!(
        store.resume_download(id).is_err(),
        "a saved file is not re-verified"
    );
    // Crash row 7: Clear removes only the staging name, never the saved file.
    let ExportState::Published(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    remove_partial(&saved).unwrap();
    store.finish_download_cleanup(id).unwrap();
    assert!(!staging(&fx.downloads).exists());
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn removal_survives_restart_without_auto_cleanup_and_never_resumes() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    assert!(store.cancel_download(id).unwrap().is_some());
    assert!(
        !lease.is_current(),
        "Remove revokes the export lease at once"
    );
    assert!(
        store
            .checkpoint_download_export(&lease, file.journal(), file.journal())
            .is_err(),
        "a late worker cannot checkpoint into a removal"
    );
    drop(file);
    drop(store); // crash before external cleanup
    let mut store = open(&fx.vault);
    let listed = store.downloads().unwrap().remove(0);
    assert_eq!(listed.state.phase, KeepPhase::Deleting);
    assert!(
        staging(&fx.downloads).exists(),
        "mount never touches the Downloads folder"
    );
    assert!(store.resume_download(id).is_err());
    assert!(store.begin_download(vec![plan()], 0, [9; 32]).is_err());
    // Proven-ours deletion failure keeps the record and its charge, without a store fault.
    let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    assert!(matches!(
        injected(Effect::Remove, io::ErrorKind::PermissionDenied, || {
            remove_partial(&journal)
        }),
        Err(ExportError::Local { .. })
    ));
    assert!(store.kept_files().error.is_none());
    remove_partial(&journal).unwrap();
    store.finish_download_cleanup(id).unwrap();
    assert!(!staging(&fx.downloads).exists());
    assert!(store.downloads().unwrap().is_empty());
}

#[test]
fn save_new_copy_retires_the_old_session_and_keep_partly_saved_file_releases_without_deleting() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let ExportState::Writing(journal) = state(&store, id) else {
        panic!();
    };
    let intent = store.downloads().unwrap()[0].state.intent;
    remove_partial(&journal).unwrap();
    assert!(
        store.abandon_download_export(id, intent + 7).is_err(),
        "intent CAS"
    );
    let next = store.abandon_download_export(id, intent).unwrap();
    assert_eq!(next, intent + 1);
    assert!(!lease.is_current());
    assert!(store.download_export_state(id).unwrap().is_none());
    assert!(store
        .checkpoint_download_export(&lease, file.journal(), file.journal())
        .is_err());
    drop(file);
    // A fresh export after Save new copy uses a new name and new identity.
    let lease = reverified(&mut store, id);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    // "Keep partly saved file": release the record, leave the partial for the user.
    assert!(store.cancel_download(id).unwrap().is_some());
    store.finish_download_cleanup(id).unwrap();
    assert!(staging(&fx.downloads).exists());
    assert!(store.downloads().unwrap().is_empty());
}

#[test]
fn save_new_copy_retires_every_observer_before_the_delete_and_keeps_the_journal() {
    // Review U-H1: the intent CAS and the observer retirement come first, the journal stays until
    // native's delete is settled, and only the retired intent can then forget it.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let before = state(&store, id);
    let intent = store.downloads().unwrap()[0].state.intent;
    assert!(
        store.retire_download_export(id, intent + 7).is_err(),
        "intent CAS"
    );
    let retired = store.retire_download_export(id, intent).unwrap();
    assert_eq!(retired, intent + 1);
    assert_eq!(state(&store, id), before, "the journal is kept");
    assert!(!lease.is_current(), "every export observer is retired");
    assert!(store
        .checkpoint_download_export(&lease, file.journal(), file.journal())
        .is_err());
    // The old click can neither retire again nor forget the journal.
    assert!(store.retire_download_export(id, intent).is_err());
    assert!(store.abandon_download_export(id, intent).is_err());
    drop(file);
    drop(store); // a crash between the retirement and the delete keeps everything tracked
    let mut store = open(&fx.vault);
    assert_eq!(state(&store, id), before);
    assert_eq!(
        remove_partial(before.journal()).unwrap(),
        PartialRemoval::Removed
    );
    assert_eq!(
        store.abandon_download_export(id, retired).unwrap(),
        retired + 1
    );
    assert!(store.download_export_state(id).unwrap().is_none());
}

#[test]
fn partial_presence_reports_only_what_is_provably_there() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let journal = file.journal().clone();
    drop(file);
    assert_eq!(partial_presence(&journal), PartialPresence::Present);
    fs::remove_file(staging(&fx.downloads)).unwrap();
    assert_eq!(partial_presence(&journal), PartialPresence::Gone);
    fs::write(staging(&fx.downloads), b"someone else's").unwrap();
    assert_eq!(
        partial_presence(&journal),
        PartialPresence::Gone,
        "another file at our name is not our partial"
    );
    fs::rename(&fx.downloads, fx.downloads.with_file_name("Moved")).unwrap();
    assert_eq!(partial_presence(&journal), PartialPresence::Unknown);
}

#[test]
fn finish_cannot_release_a_writing_journal_through_the_fallback_path() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (_id, lease) = completed(&mut store);
    let _file = started(&mut store, &lease, &fx.downloads);
    assert!(store.finish_download_export(&lease).is_err());
    assert_eq!(store.downloads().unwrap().len(), 1);
}

#[test]
fn pause_retires_presence_and_lease_and_resume_clears_a_halt_by_intent_change() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let file = started(&mut store, &lease, &fx.downloads);
    let journal = file.journal().clone();
    store
        .halt_download_export(id, &journal, ExportHalt::Unavailable)
        .unwrap();
    assert!(matches!(
        view(&store),
        ExportView::Writing {
            halt: Some(ExportHalt::Unavailable),
            active: false,
            ..
        }
    ));
    let intent = store.downloads().unwrap()[0].state.intent;
    store.set_download_paused(id, intent, true).unwrap();
    assert!(!lease.is_current());
    store.set_download_paused(id, intent + 1, false).unwrap();
    let ExportState::Writing(resumed) = state(&store, id) else {
        panic!();
    };
    assert_eq!(resumed.halt(), None, "explicit Resume clears the halt");
    assert!(resumed.same_destination(&journal));
}

#[test]
fn exhausted_reverification_survives_restart_and_retry_preserves_the_journal() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let file = started(&mut store, &lease, &fx.downloads);
    let journal = file.journal().clone();
    drop(file);
    let run = store.resume_download(id).unwrap();
    for slice in 0..120 {
        store
            .download_work(run, None, slice * KEEP_EXECUTION_SLICE_MS)
            .unwrap();
    }
    assert!(matches!(
        store.download_work(run, None, 120 * KEEP_EXECUTION_SLICE_MS),
        Err(StorageError::KeepExhausted)
    ));
    store.pause_download(run).unwrap();
    drop(store);
    let mut store = open(&fx.vault);
    assert!(matches!(
        store.resume_download(id),
        Err(StorageError::KeepExhausted)
    ));
    let receipt = store
        .retry_download(
            id,
            KeepRetry {
                transfer: *id.as_bytes(),
                expected_grant: 1,
                request: [5; 16],
            },
        )
        .unwrap();
    assert_eq!(receipt.granted, 2);
    assert_eq!(state(&store, id), ExportState::Writing(journal));
}

#[test]
fn record_successor_rules_close_every_unlisted_journal_edge() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let index = store.download_index(id).unwrap();
    let v1 = store.transfers[index].record.clone();
    assert_eq!(v1.encoding, record::RecordEncoding::DownloadV1);
    let file = started(&mut store, &lease, &fx.downloads);
    let v2 = store.transfers[index].record.clone();
    assert!(v1.validate_successor(&v2).is_ok());
    let journal = file.journal().clone();
    let mut bad = Vec::new();
    // V2 back to V1 without a journal.
    let mut next = v2.successor().unwrap();
    next.encoding = record::RecordEncoding::DownloadV1;
    next.export = Export::None;
    next.intent += 1;
    bad.push(("downgrade", next));
    // Clearing a halt without an intent change.
    let mut halted = v2.successor().unwrap();
    let mut with_halt = journal.clone();
    with_halt.halt = Some(ExportHalt::Changed);
    halted.export = Export::Writing(with_halt);
    assert!(v2.validate_successor(&halted).is_ok());
    let mut cleared = halted.successor().unwrap();
    cleared.export = Export::Writing(journal.clone());
    bad.push(("silent halt clear", cleared.clone()));
    // A different destination object.
    let mut moved = v2.successor().unwrap();
    let mut other = journal.clone();
    other.file_id = external::FileIdentity::for_test(external::CURRENT_PLATFORM.max(1), 9, 9);
    moved.export = Export::Writing(other);
    bad.push(("destination swap", moved));
    // Published before every chunk.
    let mut early = v2.successor().unwrap();
    early.export = Export::Published(journal.clone());
    bad.push(("early publish", early));
    // Forgetting without an intent change.
    let mut forgot = v2.successor().unwrap();
    forgot.export = Export::None;
    bad.push(("silent forget", forgot));
    // A network Retry grant that also moves the journal.
    let mut exhausted = v2.successor().unwrap();
    exhausted.exhausted = true;
    exhausted.remaining.slices = 0;
    assert!(v2.validate_successor(&exhausted).is_ok());
    let (mut granted, _) = exhausted
        .retry(KeepRetry {
            transfer: exhausted.transfer,
            expected_grant: 1,
            request: [4; 16],
        })
        .unwrap();
    assert!(exhausted.validate_successor(&granted).is_ok());
    let mut progressed = journal.clone();
    progressed.written_chunks = 1;
    progressed.written_bytes = CHUNKS[0].len() as u64;
    granted.export = Export::Writing(progressed);
    assert!(
        exhausted.validate_successor(&granted).is_err(),
        "Retry grant moved the journal"
    );
    // Deleting edits the journal.
    let mut deleting = v2.successor().unwrap();
    deleting.phase = KeepPhase::Deleting;
    assert!(v2.validate_successor(&deleting).is_ok());
    let mut edited = deleting.successor().unwrap();
    edited.export = Export::None;
    for (name, next) in bad {
        let base = if name == "silent halt clear" {
            &halted
        } else {
            &v2
        };
        assert!(base.validate_successor(&next).is_err(), "{name}");
    }
    assert!(deleting.validate_successor(&edited).is_err(), "Deleting");
    // A Keep record can never carry a journal, and no revision-1 record has one.
    let mut keep =
        record::Record::initial(scope(), plan(), [9; 32], &mut ChaCha20Rng::seed_from_u64(1))
            .unwrap();
    keep.export = Export::Writing(journal.clone());
    assert!(keep.validate().is_err());
    let mut genesis = v1.clone();
    genesis.revision = 1;
    genesis.predecessor = [0; 32];
    genesis.export = Export::Writing(journal);
    assert!(genesis.validate().is_err());
}

#[test]
fn v1_records_stay_canonical_until_an_export_starts_and_unknown_domains_are_left_intact() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, _lease) = completed(&mut store);
    let index = store.download_index(id).unwrap();
    let intent = store.transfers[index].record.intent;
    store.set_download_paused(id, intent, true).unwrap();
    store.set_download_paused(id, intent + 1, false).unwrap();
    assert_eq!(
        store.transfers[index].record.encoding,
        record::RecordEncoding::DownloadV1,
        "ordinary successors keep the released reader's format"
    );
    let v1 = store.transfers[index].record.encode();
    assert_eq!(record::Record::decode(&v1).unwrap().encode(), v1);
    let lease = reverified(&mut store, id);
    let _file = started(&mut store, &lease, &fx.downloads);
    let path = store.transfers[index].path.clone();
    let mut future = store.transfers[index].record.encode();
    drop(store);
    // Same record under an unknown future domain, correctly sealed: this reader must refuse it
    // without deleting the transfer directory or the external partial.
    let domain = b"mewtual/download-transfer/v2";
    let at = future
        .windows(domain.len())
        .position(|window| window == domain)
        .unwrap();
    future[at + domain.len() - 1] = b'9';
    let sealed = crate::blob::encode_sealed(
        &catcoms_crypto::seal(&[8; 32], &future, &mut ChaCha20Rng::seed_from_u64(3)).unwrap(),
    );
    fs::write(path.join("current"), &sealed).unwrap();
    let before: Vec<_> = fs::read_dir(&path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let mut store = KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        fx.vault.clone(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    assert!(store.configure_keep_scope(scope()).is_err());
    assert_eq!(fs::read(path.join("current")).unwrap(), sealed);
    for leaf in before {
        assert!(leaf.exists());
    }
    assert!(staging(&fx.downloads).exists());
}

#[test]
fn maximum_journal_fits_the_existing_download_slot_with_four_maximum_plans() {
    let plans: Vec<KeepPlan> = (0..record::MAX_VARIANTS)
        .map(|index| KeepPlan {
            cid: Cid::of(b"same plaintext"),
            version: [index as u8 + 1; 32],
            manifest: vec![index as u8; 64 * 1024],
            chunks: (0..CHUNK_LIMIT)
                .map(|chunk| {
                    (
                        Cid::of(&[index as u8, chunk as u8, (chunk >> 8) as u8]),
                        CHUNK_LIMIT_BYTES,
                    )
                })
                .collect(),
        })
        .collect();
    let mut record = record::Record::initial_download(
        scope(),
        plans,
        0,
        [9; 32],
        &mut ChaCha20Rng::seed_from_u64(2),
    )
    .unwrap();
    record.revision = 2;
    record.predecessor = [1; 32];
    record.phase = KeepPhase::Completed;
    record.verified = vec![true; CHUNK_LIMIT];
    if let Some(variants) = record.variants.as_mut() {
        variants.candidates[0].verified = vec![true; CHUNK_LIMIT];
        variants.committed = Some(0);
    }
    record.encoding = record::RecordEncoding::DownloadV2;
    let identity = external::FileIdentity::for_test(external::CURRENT_PLATFORM.max(1), 1, 1);
    record.export = Export::Writing(ExportJournal {
        folder: vec![b'a'; external::MAX_FOLDER_BYTES],
        folder_id: identity,
        staging: format!("{}{}", "s".repeat(1000), external::STAGING_SUFFIX),
        final_leaf: "f".repeat(external::MAX_LEAF_BYTES),
        file_id: identity,
        written_chunks: CHUNK_LIMIT as u32,
        written_bytes: EXPORT_MAX_BYTES,
        halt: Some(ExportHalt::Changed),
    });
    record.validate().unwrap();
    let encoded = record.encode();
    assert!(
        encoded.len() + 40 <= record::VARIANT_RECORD_LIMIT,
        "{} bytes",
        encoded.len()
    );
    assert_eq!(record::Record::decode(&encoded).unwrap(), record);
}

#[test]
fn disk_full_record_writes_leave_no_torn_successor_and_no_store_fault() {
    let fx = fixture();
    let mut store = open(&fx.vault);
    // First-ever record of a new transfer: refused cleanly, nothing left, no fault.
    let refused = injected(Effect::RecordWrite, io::ErrorKind::StorageFull, || {
        store.begin_download(vec![plan()], 0, [9; 32])
    });
    assert!(matches!(refused, Err(ref error) if persistence::is_clean_disk_full(error)));
    assert!(store.kept_files().error.is_none());
    assert_eq!(
        fs::read_dir(fx.vault.join(TRANSFERS)).unwrap().count(),
        0,
        "no empty transfer directory remains to refuse later mounts"
    );
    drop(store);
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let checkpoint = file.checkpoint().unwrap();
    let before = state(&store, id);
    let refused = injected(Effect::RecordWrite, io::ErrorKind::StorageFull, || {
        store.checkpoint_download_export(&lease, file.journal(), &checkpoint)
    });
    assert!(matches!(refused, Err(ref error) if persistence::is_clean_disk_full(error)));
    assert_eq!(state(&store, id), before);
    let index = store.download_index(id).unwrap();
    assert!(!store.transfers[index].poisoned && store.kept_files().error.is_none());
    assert!(!store.transfers[index].path.join("next").exists());
    drop(file);
    drop(store);
    let store = open(&fx.vault);
    assert_eq!(
        state(&store, id),
        before,
        "remount after the refusal is clean"
    );
}

#[test]
fn export_errors_never_carry_paths_or_names() {
    let fx = fixture();
    let secret = fx.downloads.to_string_lossy().into_owned();
    let errors = [
        ExportError::Halted(ExportHalt::Unavailable),
        ExportError::Halted(ExportHalt::Changed),
        ExportError::Full,
        ExportError::Unsupported,
        ExportError::NoLinks,
        ExportError::Inactive,
        ExportError::Refused(ChoiceRefusal::Exists),
        ExportError::Refused(ChoiceRefusal::NotWritable),
        chosen_error(io::Error::from(io::ErrorKind::PermissionDenied)),
        local(fs::File::open(fx.downloads.join("notes.txt.mewtual-part")).unwrap_err()),
        local(io::Error::from(io::ErrorKind::TimedOut)),
    ];
    for error in errors {
        let text = error.to_string();
        assert!(!text.contains(&secret) && !text.contains("notes"), "{text}");
    }
    let journal = {
        let mut store = open(&fx.vault);
        let (_id, lease) = completed(&mut store);
        started(&mut store, &lease, &fx.downloads).journal().clone()
    };
    let debug = format!("{journal:?}");
    assert!(
        !debug.contains("notes") && !debug.contains("Downloads"),
        "{debug}"
    );
}
