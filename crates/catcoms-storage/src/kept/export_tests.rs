//! Journaled external export: storage transitions plus real-filesystem `ExportFile` behavior.
//! Each crash-table row from EXTERNAL-DOWNLOAD-RESUME.md is modelled by stopping at that step,
//! dropping the store (process restart) and reopening the same vault directory.
use super::super::external::{self, Effect, INJECT, INJECT_ALSO};
use super::*;
use crate::MemoryBlobStore;
use catcoms_crypto::DeviceId;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[path = "export_chosen_tests.rs"]
mod chosen;

const CHUNKS: [&[u8]; 2] = [b"alpha chunk one!", b"bravo chunk two"];

/// The injected RNG a removal draws its Unix "removing" name from (PR51 H1).
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(51)
}

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
    // Review M-6: 0755 whatever the developer's umask, so the Unix shared-folder rule (which
    // treats a group-writable folder as shared) does not change what these tests expect.
    create_folder(&downloads).unwrap();
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
    let file = ExportFile::create(downloads, "notes.txt", export_plan(), &mut rng()).unwrap();
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
/// A preflight halt, fenced by a fresh observation as native's preflight would take.
fn halt(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    id: DownloadId,
    journal: &ExportJournal,
    halt: ExportHalt,
) -> Result<(), StorageError> {
    let (observation, _) = store.download_observation(id)?;
    store.halt_download_export(ExportFence::Observation(&observation), journal, halt)
}
/// A preflight recovery of crash row 6, fenced like [`halt`].
fn recover(
    store: &mut KeptBlobStore<ChaCha20Rng>,
    id: DownloadId,
    journal: &ExportJournal,
) -> Result<(), StorageError> {
    let (observation, _) = store.download_observation(id)?;
    store.recover_download_export(ExportFence::Observation(&observation), journal)
}
fn injected<T>(effect: Effect, kind: io::ErrorKind, run: impl FnOnce() -> T) -> T {
    INJECT.with(|slot| slot.set(Some((effect, kind, 1))));
    let result = run();
    INJECT.with(|slot| slot.set(None));
    result
}

/// Run `run` against a folder that cannot make hard links (FAT, exFAT), modelled where each
/// platform decides that at creation: Windows links a probe name, so its link fails with `kind`
/// (Linux vfat reports a permission error, others "unsupported"); Unix makes no link at creation
/// (PR51 M3) and reads the filesystem type instead, so `fstatfs` reports a no-links filesystem
/// (the class seam's `Unsupported` means vfat or exFAT). Linux CI: before this, an injected link
/// failure had no link to fail on Unix and these tests could not pass there.
pub(super) fn without_links<T>(kind: io::ErrorKind, run: impl FnOnce() -> T) -> T {
    #[cfg(unix)]
    {
        let _ = kind;
        injected(Effect::FsClass, io::ErrorKind::Unsupported, run)
    }
    #[cfg(not(unix))]
    {
        injected(Effect::Link, kind, run)
    }
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
    // Review M-5: finish says what happened to the staging name; here it went, the saved file
    // stays.
    assert_eq!(file.finish(&mut rng()).unwrap(), PartialRemoval::KeptSaved);
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
    let orphan = ExportFile::create(&fx.downloads, "notes.txt", export_plan(), &mut rng()).unwrap();
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
    file.finish(&mut rng()).unwrap(); // releases the session lock, which on Windows also blocks other readers
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
            halt(&mut store, id, &checkpoint, ExportHalt::Changed).unwrap();
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
            file.finish(&mut rng()).unwrap();
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
        halt(&mut store, id, &journal, expected).unwrap();
        // Removal of a halted record never deletes anything it cannot prove is ours, and says
        // which: FolderUnreachable keeps Removing; the others finish, with a notice whenever a
        // file stays on disk.
        let ExportState::Writing(halted) = store.cancel_download(id).unwrap().unwrap() else {
            panic!("journal is kept for cleanup");
        };
        let outcome = remove_partial(&halted, false, &mut rng()).unwrap();
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
    let unrelated = ExportFile::create(&elsewhere, "other.txt", export_plan(), &mut rng()).unwrap();
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
    assert_eq!(
        remove_partial(&journal, false, &mut rng()).unwrap(),
        PartialRemoval::Unchecked
    );
    drop(holder);
    assert!(
        staging(&fx.downloads).exists(),
        "nothing was deleted or forgotten"
    );
    assert_eq!(
        remove_partial(&journal, false, &mut rng()).unwrap(),
        PartialRemoval::Removed
    );
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
    assert_eq!(
        remove_partial(&saved, false, &mut rng()).unwrap(),
        PartialRemoval::Removed
    );
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
    file.finish(&mut rng()).unwrap();
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

/// Arm `BEFORE_REMOVE` to replace the name with someone else's file at exactly the moment between
/// the identity check and the delete: move whatever is at the name away (if allowed) and, if the
/// name is then free, write `foreign` there (read-only when asked). Returns whether the move away
/// succeeded.
fn swap_before_remove(
    elsewhere: PathBuf,
    foreign: &'static [u8],
    read_only: bool,
) -> std::rc::Rc<std::cell::Cell<Option<bool>>> {
    let moved = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = moved.clone();
    external::BEFORE_REMOVE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path: &Path| {
            seen.set(Some(fs::rename(path, &elsewhere).is_ok()));
            if !path.exists() {
                fs::write(path, foreign).unwrap();
                if read_only {
                    let mut permissions = fs::metadata(path).unwrap().permissions();
                    permissions.set_readonly(true);
                    fs::set_permissions(path, permissions).unwrap();
                }
            }
        }))
    });
    moved
}

fn make_writable(path: &Path) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[test]
fn a_name_swapped_after_verification_is_never_what_gets_deleted() {
    // PR51 H1: Remove used to verify the partial's identity, close the handle and delete by path,
    // so a file swapped in at the name in between was deleted although Mewtual never created it.
    // Now the deleted object is exactly the verified one: on Windows the delete goes through the
    // verified handle, which also refuses the swap; on Unix ours was renamed aside first, so the
    // file put at the name survives. Also for a read-only partial (the old read-only retry
    // cleared the attribute through a re-resolved path).
    for read_only in [false, true] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        file.accept(0, CHUNKS[0]).unwrap();
        drop(file);
        let partial = staging(&fx.downloads);
        if read_only {
            let mut permissions = fs::metadata(&partial).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&partial, permissions).unwrap();
        }
        let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
            panic!("journal kept for cleanup");
        };
        let elsewhere = fx.downloads.join("moved by someone");
        let moved = swap_before_remove(elsewhere.clone(), b"someone else's file", read_only);
        let outcome = remove_partial(&journal, false, &mut rng());
        external::BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(
            moved.get(),
            Some(false),
            "the swap ran at the boundary and could not move our file (read_only={read_only})"
        );
        assert!(!elsewhere.exists(), "read_only={read_only}");
        assert_eq!(
            outcome.unwrap(),
            PartialRemoval::Removed,
            "read_only={read_only}"
        );
        if partial.exists() {
            // Unix: the name was free (ours was aside) and someone put a file there; it survives
            // byte-identical, with its own attributes.
            assert_eq!(fs::read(&partial).unwrap(), b"someone else's file");
            assert_eq!(
                fs::metadata(&partial).unwrap().permissions().readonly(),
                read_only
            );
            make_writable(&partial);
        }
        let leftovers: Vec<_> = fs::read_dir(&fx.downloads)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with("mewtual-removing-"))
            .collect();
        assert!(leftovers.is_empty(), "nothing stays at a removing name");
    }
}

#[cfg(unix)]
#[test]
fn unix_never_deletes_automatically_in_a_folder_other_accounts_can_modify() {
    // PR51 H1: a folder others can write to (without the sticky bit) lets them swap names, so
    // Remove leaves the partial for the user; a private or sticky folder still removes it.
    use std::os::unix::fs::PermissionsExt;
    for (mode, expected) in [
        (0o777, PartialRemoval::SharedFolder),
        (0o775, PartialRemoval::SharedFolder),
        (0o1777, PartialRemoval::Removed),
        (0o755, PartialRemoval::Removed),
    ] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        file.accept(0, CHUNKS[0]).unwrap();
        drop(file);
        fs::set_permissions(&fx.downloads, fs::Permissions::from_mode(mode)).unwrap();
        let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
            panic!();
        };
        let outcome = remove_partial(&journal, false, &mut rng()).unwrap();
        assert_eq!(outcome, expected, "mode {mode:o}");
        assert_eq!(
            staging(&fx.downloads).exists(),
            expected == PartialRemoval::SharedFolder,
            "mode {mode:o}"
        );
        if expected == PartialRemoval::SharedFolder {
            assert!(outcome.settled() && outcome.notice(false).is_some());
        }
        fs::set_permissions(&fx.downloads, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn unix_removes_in_a_group_writable_folder_only_for_a_user_private_group() {
    // User decision, 23 October: a 0775 folder whose group is our own user-private group (proven
    // from the account files, injected here) is private again, so Remove deletes; a group with
    // another member stays shared and nothing is deleted.
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let me = rustix::process::geteuid().as_raw();
    let my_gid = rustix::process::getegid().as_raw();
    // Decisions review HIGH: an extended ACL granting another account write makes the folder
    // shared even with a proven private group.
    for (members, acl, expected) in [
        ("", false, PartialRemoval::Removed),
        ("bob", false, PartialRemoval::SharedFolder),
        ("", true, PartialRemoval::SharedFolder),
    ] {
        let fx = fixture();
        if fs::metadata(&fx.downloads).unwrap().gid() != my_gid {
            // A set-group-id parent gave the folder another group; give it ours (decisions
            // review L5: never skip silently).
            std::os::unix::fs::chown(&fx.downloads, None, Some(my_gid)).unwrap();
        }
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        file.accept(0, CHUNKS[0]).unwrap();
        drop(file);
        let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
            panic!();
        };
        fs::set_permissions(&fx.downloads, fs::Permissions::from_mode(0o775)).unwrap();
        if acl {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                // user::rwx, user:4242:rwx, group::rwx, mask::rwx, other::r-x (version 2 format).
                let mut blob = 2u32.to_le_bytes().to_vec();
                for (tag, perm, id) in [
                    (0x01u16, 7u16, u32::MAX),
                    (0x02, 7, 4242),
                    (0x04, 7, u32::MAX),
                    (0x10, 7, u32::MAX),
                    (0x20, 5, u32::MAX),
                ] {
                    blob.extend(tag.to_le_bytes());
                    blob.extend(perm.to_le_bytes());
                    blob.extend(id.to_le_bytes());
                }
                match rustix::fs::setxattr(
                    &fx.downloads,
                    "system.posix_acl_access",
                    &blob,
                    rustix::fs::XattrFlags::empty(),
                ) {
                    Ok(()) => {}
                    Err(rustix::io::Errno::OPNOTSUPP) => {
                        eprintln!("this filesystem has no POSIX ACLs; the ACL case does not apply");
                        continue;
                    }
                    Err(error) => panic!("setting the ACL failed: {error}"),
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            continue;
        }
        external::ACCOUNT_FILES.with(|files| {
            *files.borrow_mut() = Some(external::AccountFiles {
                passwd: Some(format!(
                    "me:x:{me}:{my_gid}:Me:/home/me:/bin/sh\nbob:x:4242:4242:Bob:/home/bob:/bin/sh\n"
                )),
                group: Some(format!("me:x:{my_gid}:{members}\nbob:x:4242:\n")),
                nsswitch: external::NsSwitch::Absent,
                userdb_records: Some(false),
                host_namespace: true,
            })
        });
        let outcome = remove_partial(&journal, false, &mut rng());
        external::ACCOUNT_FILES.with(|files| *files.borrow_mut() = None);
        assert_eq!(outcome.unwrap(), expected, "members {members:?}, acl {acl}");
        assert_eq!(
            staging(&fx.downloads).exists(),
            expected == PartialRemoval::SharedFolder
        );
        fs::set_permissions(&fx.downloads, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn unix_a_shared_folder_tells_the_user_to_delete_only_its_own_file() {
    // Combined review M-A: in a folder other accounts can modify, the user is told to delete the
    // leftover only when the name still holds Mewtual's own file. A name the user already deleted,
    // or one now holding someone else's file, never produces "Delete it yourself".
    use std::os::unix::fs::PermissionsExt;
    // Root reads a mode-0 file anyway, so the unreadable case means nothing there.
    let root = rustix::process::geteuid().is_root();
    for case in ["missing", "foreign", "foreign-unreadable", "saved-missing"] {
        if root && case == "foreign-unreadable" {
            continue;
        }
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        let published = case == "saved-missing";
        if published {
            write_all(&mut file);
            publish(&mut store, &lease, &mut file);
        } else {
            file.accept(0, CHUNKS[0]).unwrap();
        }
        drop(file);
        let journal = store
            .cancel_download(id)
            .unwrap()
            .unwrap()
            .journal()
            .clone();
        fs::remove_file(staging(&fx.downloads)).unwrap();
        if case.starts_with("foreign") {
            fs::write(staging(&fx.downloads), b"someone else's file").unwrap();
        }
        if case == "foreign-unreadable" {
            // Re-review LOW 1: a file we may not even open was put there by another account.
            fs::set_permissions(staging(&fx.downloads), fs::Permissions::from_mode(0o000)).unwrap();
        }
        fs::set_permissions(&fx.downloads, fs::Permissions::from_mode(0o777)).unwrap();
        let outcome = remove_partial(&journal, published, &mut rng()).unwrap();
        let expected = match case {
            "missing" => PartialRemoval::Removed,
            "foreign" | "foreign-unreadable" => PartialRemoval::LeftInPlace,
            _ => PartialRemoval::KeptSaved,
        };
        assert_eq!(outcome, expected, "{case}");
        // Neither the unpublished ("Delete it yourself") nor the saved ("Delete that copy
        // yourself") shared-folder instruction appears.
        assert!(
            !outcome
                .notice(published)
                .is_some_and(|notice| notice.contains("yourself")),
            "{case}"
        );
        if case.starts_with("foreign") {
            fs::set_permissions(staging(&fx.downloads), fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                fs::read(staging(&fx.downloads)).unwrap(),
                b"someone else's file"
            );
        }
        fs::set_permissions(&fx.downloads, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn unix_finds_a_partial_a_crash_left_at_a_removing_name() {
    // PR51 H1: a crash between the rename aside and the unlink leaves our plaintext at a random
    // "removing" name. Keep must still see it, and the next Remove must find it by identity.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    let leftover = fx
        .downloads
        .join("mewtual-removing-00000000000000000000000000000001");
    fs::rename(staging(&fx.downloads), &leftover).unwrap();
    // Someone else's file under a similar name is never touched.
    let unrelated = fx
        .downloads
        .join("mewtual-removing-ffffffffffffffffffffffffffffffff");
    fs::write(&unrelated, b"not ours").unwrap();
    let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    assert_eq!(partial_presence(&journal), PartialPresence::Present);
    assert_eq!(
        remove_partial(&journal, false, &mut rng()).unwrap(),
        PartialRemoval::Removed
    );
    assert!(!leftover.exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"not ours");
    assert_eq!(partial_presence(&journal), PartialPresence::Gone);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn linux_links_the_open_download_even_if_its_name_was_swapped() {
    // PR51 M1: publication links the session's own descriptor through /proc, so a staging name
    // swapped after the pin check can never become the final name; the swapped-in file is left
    // exactly where it was.
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
    let aside = fx.downloads.join("ours moved");
    external::BEFORE_LINK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |staging: &Path| {
            fs::rename(staging, &aside).unwrap();
            fs::write(staging, b"someone else's file").unwrap();
        }))
    });
    let path = file.link().unwrap();
    external::BEFORE_LINK.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(
        fs::read(&path).unwrap(),
        CHUNKS.concat(),
        "our object, not theirs"
    );
    assert_eq!(
        fs::read(staging(&fx.downloads)).unwrap(),
        b"someone else's file",
        "the swapped-in file is untouched"
    );
}

/// Arm `AFTER_LINK` to rename `folder` aside to `aside` and put an empty folder at its path, right
/// after a publication link succeeded and before the post-link check.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn swap_folder_after_link(folder: &Path, aside: &Path) {
    let (path, moved) = (folder.to_owned(), aside.to_owned());
    external::AFTER_LINK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            fs::rename(&path, &moved).unwrap();
            fs::create_dir(&path).unwrap();
        }))
    });
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn a_folder_swapped_right_after_the_link_does_not_unsay_the_publication() {
    // The coordinator's review L2: the post-link check reads the final name through the folder
    // descriptor the link went into. Reading it by path again made a real publication read as
    // Changed (journaled) or as a different file (without a journal) when the folder was swapped
    // right after the link, while the complete copy sat in the moved folder.
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
    let aside = fx.downloads.with_file_name("Downloads moved");
    swap_folder_after_link(&fx.downloads, &aside);
    let linked = file.link();
    external::AFTER_LINK.with(|hook| *hook.borrow_mut() = None);
    linked.expect("the link was made and checked where it was made");
    assert_eq!(fs::read(aside.join("notes.txt")).unwrap(), CHUNKS.concat());
    assert_eq!(fs::read_dir(&fx.downloads).unwrap().count(), 0);

    // The writers without a journal.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Chosen place");
    fs::create_dir(&folder).unwrap();
    let (file, staging, final_path) = unjournaled_staging(&folder, b"verified bytes");
    let aside = dir.path().join("Moved away");
    swap_folder_after_link(&folder, &aside);
    let published = publish_unjournaled(&file, &staging, &final_path);
    external::AFTER_LINK.with(|hook| *hook.borrow_mut() = None);
    published.expect("the link was made and checked where it was made");
    assert_eq!(
        fs::read(aside.join("notes.txt")).unwrap(),
        b"verified bytes"
    );
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    drop(file);
}

/// A native writer's staging file without a journal: created exclusively and held open, as the
/// non-resumable writer and the GIF export do.
fn unjournaled_staging(folder: &Path, bytes: &[u8]) -> (File, PathBuf, PathBuf) {
    let staging = folder.join("notes.txt.part");
    let mut file = external::create_leaf(&staging).unwrap();
    std::io::Write::write_all(&mut file, bytes).unwrap();
    (file, staging, folder.join("notes.txt"))
}

#[test]
fn an_unjournaled_save_publishes_only_its_own_open_file() {
    // PR51 M1 for the non-resumable writer: another program tries to swap the staging name between
    // verification and the link. Windows: the pin refuses the swap. Linux and Android: the swap
    // happens, but the link is made from our open descriptor, so the final name is still ours and
    // the swapped-in file stays exactly where it was. macOS (no link by descriptor): the post-link
    // check finds a foreign object at the final name, fails the save, and leaves both names.
    let dir = tempfile::tempdir().unwrap();
    let (file, staging, final_path) = unjournaled_staging(dir.path(), b"verified bytes");
    let aside = dir.path().join("ours moved");
    let moved = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = moved.clone();
    external::BEFORE_LINK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |from: &Path| {
            let swapped = fs::rename(from, &aside).is_ok();
            seen.set(Some(swapped));
            if swapped {
                fs::write(from, b"someone else's file").unwrap();
            }
        }))
    });
    let published = publish_unjournaled(&file, &staging, &final_path);
    external::BEFORE_LINK.with(|hook| *hook.borrow_mut() = None);
    #[cfg(windows)]
    {
        assert_eq!(moved.get(), Some(false), "the pin refused the swap");
        published.unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"verified bytes");
        assert_eq!(fs::read(&staging).unwrap(), b"verified bytes");
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        assert_eq!(moved.get(), Some(true), "the swap happened");
        published.unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"verified bytes", "ours");
        assert_eq!(fs::read(&staging).unwrap(), b"someone else's file");
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
    {
        assert_eq!(moved.get(), Some(true), "the swap happened");
        assert_eq!(
            published.unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "never reported as saved"
        );
        assert_eq!(fs::read(&final_path).unwrap(), b"someone else's file");
        assert_eq!(fs::read(&staging).unwrap(), b"someone else's file");
    }
    drop(file);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn an_unjournaled_save_is_published_only_beside_its_partial() {
    // Review of the Linux CI fix, MEDIUM 1(b): the folder is renamed aside and another one put at
    // its path before publication. Without a journal nothing records the folder's identity, so the
    // descriptor link must not name our file in whatever folder the path now leads to (PR51 M1 did;
    // the path-based link at 86664d94 failed there). Nothing is linked, our partial stays where it
    // is, and a file planted at our staging name in the new folder is not ours either.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Chosen place");
    fs::create_dir(&folder).unwrap();
    let (file, staging, final_path) = unjournaled_staging(&folder, b"verified bytes");
    let aside = dir.path().join("Moved away");
    fs::rename(&folder, &aside).unwrap();
    fs::create_dir(&folder).unwrap();
    assert_eq!(
        publish_unjournaled(&file, &staging, &final_path)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        0,
        "nothing in the new folder"
    );
    fs::write(&staging, b"planted").unwrap();
    assert_eq!(
        publish_unjournaled(&file, &staging, &final_path)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!final_path.exists(), "never linked into the new folder");
    assert_eq!(fs::read(&staging).unwrap(), b"planted");
    assert_eq!(
        fs::read(aside.join("notes.txt.part")).unwrap(),
        b"verified bytes"
    );
    assert!(!aside.join("notes.txt").exists());
    drop(file);
}

#[test]
fn an_unjournaled_save_never_replaces_a_taken_name_or_leaves_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (file, staging, final_path) = unjournaled_staging(dir.path(), b"verified bytes");
    fs::write(&final_path, b"the user's own file").unwrap();
    assert_eq!(
        publish_unjournaled(&file, &staging, &final_path)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&final_path).unwrap(), b"the user's own file");
    // The two names must be plain names in one folder.
    let other = tempfile::tempdir().unwrap();
    assert_eq!(
        publish_unjournaled(&file, &staging, &other.path().join("notes.txt"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(!other.path().join("notes.txt").exists());
    // A post-link check that cannot be made: Linux, Android and Windows already linked our own
    // object, so the save stands; macOS has nothing else proving the final name ours.
    fs::remove_file(&final_path).unwrap();
    let checked = injected(Effect::Check, io::ErrorKind::PermissionDenied, || {
        publish_unjournaled(&file, &staging, &final_path)
    });
    #[cfg(any(windows, target_os = "linux", target_os = "android"))]
    checked.unwrap();
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    assert!(checked.is_err());
    assert_eq!(fs::read(&final_path).unwrap(), b"verified bytes");
}

#[test]
fn a_native_staging_name_is_removed_only_while_it_holds_our_open_file() {
    // PR51 H1 for writers without a journal: the removal is verified against the in-session number
    // of the handle the writer still holds. Someone else's file put at the name between the check
    // and the delete survives byte-identical (Windows: the swap is refused; Unix: ours was renamed
    // aside first), also when it is read-only.
    for read_only in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (file, staging, _final_path) = unjournaled_staging(dir.path(), b"partial plaintext");
        let id = session_id(&file).unwrap();
        let elsewhere = dir.path().join("moved by someone");
        let moved = swap_before_remove(elsewhere.clone(), b"someone else's file", read_only);
        let removed = remove_if_session(dir.path(), "notes.txt.part", id, &mut rng());
        external::BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(moved.get(), Some(false), "read_only={read_only}");
        assert_eq!(
            removed.unwrap(),
            SessionRemoval::Removed,
            "our file was removed (read_only={read_only})"
        );
        assert!(!elsewhere.exists());
        if staging.exists() {
            assert_eq!(fs::read(&staging).unwrap(), b"someone else's file");
            assert_eq!(
                fs::metadata(&staging).unwrap().permissions().readonly(),
                read_only
            );
            make_writable(&staging);
        }
        drop(file);
    }
    // A name that already holds another object is never deleted.
    let dir = tempfile::tempdir().unwrap();
    let (file, staging, _final_path) = unjournaled_staging(dir.path(), b"partial plaintext");
    let id = session_id(&file).unwrap();
    let ours = dir.path().join("ours elsewhere");
    let moved_away = fs::rename(&staging, &ours).is_ok();
    if moved_away {
        fs::write(&staging, b"someone else's file").unwrap();
        assert_eq!(
            remove_if_session(dir.path(), "notes.txt.part", id, &mut rng()).unwrap(),
            SessionRemoval::NotOurs
        );
        assert_eq!(fs::read(&staging).unwrap(), b"someone else's file");
        assert_eq!(fs::read(&ours).unwrap(), b"partial plaintext");
    }
    // Nothing at the name: nothing to delete.
    assert_eq!(
        remove_if_session(dir.path(), "absent.part", id, &mut rng()).unwrap(),
        SessionRemoval::Missing
    );
    drop(file);
}

#[cfg(unix)]
#[test]
fn unix_never_renames_an_object_that_is_not_ours() {
    // Review M-2: the name is verified before it is renamed aside, so a file that is not ours
    // (here the user's own file at the staging name) is never moved, not even for a moment.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    let partial = staging(&fx.downloads);
    fs::rename(&partial, fx.downloads.join("ours elsewhere")).unwrap();
    fs::write(&partial, b"the user's own file").unwrap();
    let renamed = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = renamed.clone();
    external::AFTER_ASIDE
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move |_: &Path, _: &Path| seen.set(true))));
    let outcome = remove_partial(&journal, false, &mut rng());
    external::AFTER_ASIDE.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(outcome.unwrap(), PartialRemoval::LeftInPlace);
    assert!(!renamed.get(), "nothing was renamed aside");
    assert_eq!(fs::read(&partial).unwrap(), b"the user's own file");
}

#[cfg(unix)]
#[test]
fn unix_a_name_that_cannot_be_put_back_is_never_reported_as_left_in_place() {
    // Review M-2: our file is swapped for someone else's between the check and the rename, and
    // a third file appears at the original name before the put-back. The foreign object stays at
    // the random name, so the outcome must be an error (Unchecked: the save stays tracked), never
    // "left in place".
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    drop(file);
    let ExportState::Writing(journal) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    let ours = fx.downloads.join("ours, moved by someone");
    let moved = ours.clone();
    external::AFTER_ASIDE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |original: &Path, aside: &Path| {
            fs::rename(aside, &moved).unwrap();
            fs::write(aside, b"someone else's file").unwrap();
            fs::write(original, b"a third file").unwrap();
        }))
    });
    let outcome = remove_partial(&journal, false, &mut rng());
    external::AFTER_ASIDE.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(outcome.unwrap(), PartialRemoval::Unchecked);
    assert_eq!(fs::read(staging(&fx.downloads)).unwrap(), b"a third file");
    assert!(
        ours.exists(),
        "our file is wherever it was moved, untouched"
    );
}

#[cfg(unix)]
#[test]
fn unix_never_deletes_automatically_on_a_network_or_fuse_filesystem() {
    // Review M-3: mode bits say who may change a folder only on a local filesystem this kernel
    // enforces. A network or FUSE mount (modelled by an Unknown class) counts as shared.
    let dir = tempfile::tempdir().unwrap();
    let (file, staging, _final_path) = unjournaled_staging(dir.path(), b"partial plaintext");
    let id = session_id(&file).unwrap();
    let removal = injected(Effect::FsClass, io::ErrorKind::Other, || {
        remove_if_session(dir.path(), "notes.txt.part", id, &mut rng())
    });
    assert_eq!(removal.unwrap(), SessionRemoval::SharedFolder);
    assert!(staging.exists());
    assert_eq!(
        remove_if_session(dir.path(), "notes.txt.part", id, &mut rng()).unwrap(),
        SessionRemoval::Removed
    );
    drop(file);
}

#[test]
fn without_proc_no_save_starts_that_could_not_finish() {
    // Review L-7: Linux and Android publish by linking the open descriptor through /proc. Without
    // it (modelled here on every platform) no journal is started, and nothing is left behind.
    let fx = fixture();
    let created = injected(Effect::Descriptor, io::ErrorKind::NotFound, || {
        ExportFile::create(&fx.downloads, "notes.txt", export_plan(), &mut rng())
    });
    assert!(matches!(created, Err(ExportError::Unsupported)));
    assert_eq!(fs::read_dir(&fx.downloads).unwrap().count(), 0);
    // The non-resumable writer's probe refuses too, before any plaintext (Unix only: Windows
    // publishes under a pin instead and never asks).
    #[cfg(unix)]
    {
        let staging = fx.downloads.join("notes.txt.part");
        let file = external::create_leaf(&staging).unwrap();
        let refused = injected(Effect::Descriptor, io::ErrorKind::NotFound, || {
            confirm_staging_links(file, &fx.downloads, "notes.txt.part", &mut rng())
        });
        match refused {
            Err(ExportError::Local { reason, .. }) => assert!(reason.contains("/proc"), "{reason}"),
            other => panic!("expected the /proc refusal, got {other:?}"),
        }
        assert!(!staging.exists(), "the empty reservation is removed");
    }
}

#[cfg(unix)]
#[test]
fn a_native_staging_name_in_a_folder_others_can_modify_is_left_in_place() {
    // PR51 H1: Unix never deletes automatically where other accounts can swap names.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (file, staging, _final_path) = unjournaled_staging(dir.path(), b"partial plaintext");
    let id = session_id(&file).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(
        remove_if_session(dir.path(), "notes.txt.part", id, &mut rng()).unwrap(),
        SessionRemoval::SharedFolder
    );
    assert!(staging.exists(), "left for the user to delete");
    // Combined review M-A: a missing or foreign name there is never reported as shared (which
    // would tell the user to delete it).
    assert_eq!(
        remove_if_session(dir.path(), "absent.part", id, &mut rng()).unwrap(),
        SessionRemoval::Missing
    );
    fs::write(dir.path().join("theirs.part"), b"someone else's file").unwrap();
    assert_eq!(
        remove_if_session(dir.path(), "theirs.part", id, &mut rng()).unwrap(),
        SessionRemoval::NotOurs
    );
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        remove_if_session(dir.path(), "notes.txt.part", id, &mut rng()).unwrap(),
        SessionRemoval::Removed
    );
    drop(file);
}

/// A published save whose staging name is still present (no `finish`), released to Deleting.
fn saved_row_without_finish(fx: &Fixture) -> (PathBuf, ExportJournal) {
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    write_all(&mut file);
    let path = publish(&mut store, &lease, &mut file);
    drop(file);
    let ExportState::Published(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!("a saved row keeps its journal for cleanup");
    };
    (path, saved)
}

#[test]
fn a_saved_file_check_says_what_is_at_the_saved_name_without_changing_anything() {
    // User decision, 23 October: Transfers checks a Saved row's file. Read-only: present and ours,
    // missing, replaced by another file, or unknown when the folder cannot be checked.
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    let listing = || {
        let mut names: Vec<_> = fs::read_dir(&fx.downloads)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let before = listing();
    assert_eq!(saved_presence(&saved), SavedPresence::Ours);
    // Show in folder (Transfers UX) gets the path only while the same check proves it ours.
    assert_eq!(saved_file_path(&saved), Ok(path.clone()));
    assert_eq!(listing(), before, "nothing changed");
    // Replaced by an editor's atomic save (a new file at the saved name).
    let original = fx.downloads.join("original copy");
    fs::rename(&path, &original).unwrap();
    fs::write(&path, b"edited by the user").unwrap();
    assert_eq!(saved_presence(&saved), SavedPresence::Replaced);
    assert_eq!(saved_file_path(&saved), Err(SavedPresence::Replaced));
    assert_eq!(fs::read(&path).unwrap(), b"edited by the user");
    // Missing.
    fs::remove_file(&path).unwrap();
    assert_eq!(saved_presence(&saved), SavedPresence::Missing);
    assert_eq!(saved_file_path(&saved), Err(SavedPresence::Missing));
    assert!(!path.exists(), "nothing was created");
    // The folder moved away: where the file went cannot be told, so it is not called missing.
    fs::rename(&fx.downloads, fx.downloads.with_file_name("Moved")).unwrap();
    assert_eq!(saved_presence(&saved), SavedPresence::Unknown);
    assert_eq!(saved_file_path(&saved), Err(SavedPresence::Unknown));
    // The Transfers row's "Saved to Downloads" compares the recorded folder exactly.
    assert!(saved.is_in_folder(&fx.downloads));
    assert!(!saved.is_in_folder(&fx.downloads.with_file_name("Moved")));
}

#[test]
fn clearing_a_saved_row_whose_final_name_is_gone_keeps_the_only_copy() {
    // PR51 M2: power loss on a filesystem that ignored the folder flush (or the user moving the
    // file) can leave a Published record whose final name is gone while the staging name still
    // holds the file. That staging name is then the only copy: Clear never deletes it, and says
    // where it is. Review M-1: when another file now sits at the final name (an editor's atomic
    // save), the original copy is kept too, but the notice never claims the saved file is gone or
    // asks the user to rename the copy over that other file.
    for replaced in [false, true] {
        let fx = fixture();
        let (path, saved) = saved_row_without_finish(&fx);
        fs::remove_file(&path).unwrap();
        if replaced {
            fs::write(&path, b"a different file").unwrap();
        }
        let outcome = remove_partial(&saved, true, &mut rng()).unwrap();
        let expected = if replaced {
            PartialRemoval::SavedNameReplaced
        } else {
            PartialRemoval::KeptOnlyCopy
        };
        assert_eq!(outcome, expected, "replaced={replaced}");
        assert!(outcome.settled() && !outcome.cleared_staging());
        let notice = outcome.notice(true).unwrap();
        assert!(notice.contains(".mewtual-part"));
        assert_eq!(notice.contains("no longer in its folder"), !replaced);
        assert_eq!(notice.contains("Rename"), !replaced);
        assert_eq!(fs::read(staging(&fx.downloads)).unwrap(), CHUNKS.concat());
        if replaced {
            assert_eq!(fs::read(&path).unwrap(), b"a different file");
        }
    }
}

#[test]
fn clearing_a_saved_row_whose_saved_name_was_replaced_says_nothing_false() {
    // Review M-1: the staging name already went (an earlier Clear, or finish) and the user's
    // editor later replaced the saved file. Nothing of ours is left; the saved row must not be
    // told "The saved file was no longer in its folder."
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    fs::remove_file(staging(&fx.downloads)).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"edited by the user").unwrap();
    let outcome = remove_partial(&saved, true, &mut rng()).unwrap();
    assert_eq!(outcome, PartialRemoval::Removed);
    assert_eq!(outcome.notice(true), None);
    assert_eq!(fs::read(&path).unwrap(), b"edited by the user");
}

#[test]
fn clearing_a_saved_row_whose_names_are_both_gone_never_says_removed() {
    // PR51 M2: neither name is left. A saved row says the saved file is gone (never "Saved" and
    // never "removed"); an unfinished save with nothing left is simply removed.
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    fs::remove_file(&path).unwrap();
    fs::remove_file(staging(&fx.downloads)).unwrap();
    let outcome = remove_partial(&saved, true, &mut rng()).unwrap();
    assert_eq!(outcome, PartialRemoval::SavedFileMissing);
    assert!(outcome.settled());
    assert_eq!(
        outcome.notice(true),
        Some("The saved file was no longer in its folder.")
    );
    assert_eq!(
        remove_partial(&saved, false, &mut rng()).unwrap(),
        PartialRemoval::Removed
    );
}

#[cfg(windows)]
#[test]
fn a_saved_rows_final_name_held_by_another_program_is_not_treated_as_gone() {
    // An indexer or antivirus can hold the just-saved file without sharing for a moment. That
    // proves nothing about the final name, so Clear neither keeps a second name as "the only
    // copy" nor deletes the staging name: it stays tracked (Unchecked) and a retry settles it.
    use std::os::windows::fs::OpenOptionsExt;
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    let holder = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert_eq!(
        remove_partial(&saved, true, &mut rng()).unwrap(),
        PartialRemoval::Unchecked
    );
    assert!(staging(&fx.downloads).exists());
    drop(holder);
    assert_eq!(
        remove_partial(&saved, true, &mut rng()).unwrap(),
        PartialRemoval::KeptSaved
    );
    assert!(!staging(&fx.downloads).exists());
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
}

/// Published-staging review (26 October): Clear checks the saved (final) name, then deletes the
/// staging name. A saved name deleted in between (here at `BEFORE_REMOVE`: after the staging name
/// was opened and verified, on Unix after its rename aside) left that delete removing the last
/// copy. The staging name now goes only while the saved name holds the file at the delete itself,
/// so the copy survives at its own name and the row says it is now the only copy.
#[test]
fn a_saved_name_deleted_before_the_staging_delete_keeps_the_only_copy() {
    // Also when another file took the saved name meanwhile (an editor's atomic save): the
    // original bytes stay under the staging name and the row says the name was replaced.
    for replaced in [false, true] {
        let fx = fixture();
        let (path, saved) = saved_row_without_finish(&fx);
        let doomed = path.clone();
        external::BEFORE_REMOVE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                fs::remove_file(&doomed).unwrap();
                if replaced {
                    fs::write(&doomed, b"a different file").unwrap();
                }
            }))
        });
        let outcome = remove_partial(&saved, true, &mut rng());
        external::BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = None);
        let expected = if replaced {
            PartialRemoval::SavedNameReplaced
        } else {
            PartialRemoval::KeptOnlyCopy
        };
        assert_eq!(outcome.unwrap(), expected, "replaced={replaced}");
        assert_eq!(
            fs::read(staging(&fx.downloads)).unwrap(),
            CHUNKS.concat(),
            "the original copy survives, at its own name (replaced={replaced})"
        );
        if replaced {
            assert_eq!(fs::read(&path).unwrap(), b"a different file");
        } else {
            assert!(!path.exists(), "the saved name was deleted meanwhile");
        }
    }
    // The notice says it is the only copy and where, without assuming the staging name is the
    // saved name plus a suffix (a final name numbered at publication keeps its original staging
    // name).
    let notice = PartialRemoval::KeptOnlyCopy.notice(true).unwrap();
    assert!(notice.contains("only copy") && notice.contains("ends in .mewtual-part"));
    assert!(!notice.contains("saved name with"));
}

/// Review M2 of the published-staging fix: when the pin on the saved name is refused with a
/// sharing violation (here another program holds it with delete access; on a filesystem that
/// checks delete sharing per file our own staging handle would do the same), the saved name is
/// verified without the pin instead of turning every Clear into "could not check" forever.
#[cfg(windows)]
#[test]
fn a_saved_name_whose_pin_is_refused_is_checked_without_it() {
    use std::os::windows::fs::OpenOptionsExt;
    const DELETE: u32 = 0x0001_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    let holder = fs::OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(0x7)
        .open(&path)
        .unwrap();
    let outcome = remove_partial(&saved, true, &mut rng());
    drop(holder);
    assert_eq!(outcome.unwrap(), PartialRemoval::KeptSaved);
    assert!(!staging(&fx.downloads).exists());
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
}

/// Review L1 of the published-staging fix (Unix): a crash between the rename aside and the
/// unlink of an earlier Clear left our file at a "removing" name. The sweep that deletes it is a
/// staging delete too: with the saved name deleted before that unlink, the leftover goes back to
/// the staging name instead of being deleted as the last copy.
#[cfg(unix)]
#[test]
fn a_crash_leftover_is_put_back_when_the_saved_name_goes_before_its_delete() {
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    let aside = fx
        .downloads
        .join("mewtual-removing-00000000000000000000000000000001");
    fs::rename(staging(&fx.downloads), &aside).unwrap();
    let doomed = path.clone();
    external::BEFORE_REMOVE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| fs::remove_file(&doomed).unwrap()))
    });
    let outcome = remove_partial(&saved, true, &mut rng());
    external::BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = None);
    assert!(!path.exists(), "the saved name was deleted meanwhile");
    assert_eq!(outcome.unwrap(), PartialRemoval::KeptOnlyCopy);
    assert!(!aside.exists());
    assert_eq!(fs::read(staging(&fx.downloads)).unwrap(), CHUNKS.concat());
}

/// Review L1 (Unix): the saved name is already gone and the only copy sits at a "removing" name
/// from a crashed Clear. It used to stay hidden there ("could not check" on every Clear); it is
/// put back at the staging name and the row says it is the only copy.
#[cfg(unix)]
#[test]
fn a_hidden_only_copy_of_a_saved_file_is_put_back_at_its_staging_name() {
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    fs::remove_file(&path).unwrap();
    let aside = fx
        .downloads
        .join("mewtual-removing-00000000000000000000000000000002");
    fs::rename(staging(&fx.downloads), &aside).unwrap();
    // Re-review LOW: in a folder other accounts can (or may) modify, here a network or FUSE
    // mount modelled through the filesystem-class seam, nothing is changed automatically: the
    // put-back could fall back to link and unlink there and pick up a swapped-in file. The
    // leftover stays where it is and the row says it could not check.
    let shared = injected(Effect::FsClass, io::ErrorKind::Other, || {
        remove_partial(&saved, true, &mut rng()).unwrap()
    });
    assert_eq!(shared, PartialRemoval::Unchecked);
    assert!(aside.exists() && !staging(&fx.downloads).exists());
    assert_eq!(
        remove_partial(&saved, true, &mut rng()).unwrap(),
        PartialRemoval::KeptOnlyCopy
    );
    assert!(!aside.exists());
    assert_eq!(fs::read(staging(&fx.downloads)).unwrap(), CHUNKS.concat());
}

/// Published-staging review: on Windows the saved name is pinned (delete sharing denied) and
/// verified before the staging delete and held across it, so nothing can delete it in between;
/// a handle on the staging name alone would not stop that, because Windows lets another link of
/// the same file be deleted.
#[cfg(windows)]
#[test]
fn a_saved_name_cannot_be_deleted_while_its_staging_name_goes() {
    let fx = fixture();
    let (path, saved) = saved_row_without_finish(&fx);
    let tried = std::rc::Rc::new(std::cell::Cell::new(None));
    let (target, seen) = (path.clone(), tried.clone());
    external::WHILE_KEPT.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            seen.set(Some(fs::remove_file(&target).is_ok()));
        }))
    });
    let outcome = remove_partial(&saved, true, &mut rng());
    external::WHILE_KEPT.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(
        tried.get(),
        Some(false),
        "deleting the saved name was refused"
    );
    assert_eq!(outcome.unwrap(), PartialRemoval::KeptSaved);
    assert!(!staging(&fx.downloads).exists());
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
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
        remove_partial(&saved, false, &mut rng())
    });
    // PR51 H1: a failed verified delete reports Unchecked (the journal stays, still Removing)
    // rather than an error.
    assert_eq!(
        refused.unwrap(),
        PartialRemoval::Unchecked,
        "removal stays in Removing instead"
    );
    assert!(staging(&fx.downloads).exists());
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    // Where deletion of the name is allowed, it leaves the shared attributes alone.
    assert_eq!(
        remove_partial(&saved, false, &mut rng()).unwrap(),
        PartialRemoval::KeptSaved
    );
    assert!(!staging(&fx.downloads).exists());
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert_eq!(fs::read(&path).unwrap(), CHUNKS.concat());
    // Let the temporary directory clean up.
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn a_fifo_in_place_of_the_saved_folder_fails_resume_and_remove_promptly() {
    // Review U-M6: pause, rename the folder, put a FIFO at its old path, then Resume or Remove.
    // Unix-only and compile-unverified on the Windows development host.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let journal = file.journal().clone();
    drop(file);
    fs::rename(&fx.downloads, fx.downloads.with_file_name("Moved")).unwrap();
    if !external::tests::mkfifo(&fx.downloads) {
        eprintln!("skipping: mkfifo is unavailable");
        return;
    }
    let probed = journal.clone();
    assert!(matches!(
        external::tests::bounded(move || ExportFile::probe(&probed)),
        ExportProbe::Halt(ExportHalt::Unavailable)
    ));
    let ExportState::Writing(saved) = store.cancel_download(id).unwrap().unwrap() else {
        panic!();
    };
    assert_eq!(
        external::tests::bounded(move || remove_partial(&saved, false, &mut rng())).unwrap(),
        PartialRemoval::FolderUnreachable
    );
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
    let outcome = remove_partial(&saved, false, &mut rng()).unwrap();
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
    let outcome = remove_partial(&saved, false, &mut rng()).unwrap();
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
    halt(&mut store, id, &journal, ExportHalt::Changed).unwrap();
    let ExportState::Writing(halted) = state(&store, id) else {
        panic!();
    };
    let intent = store.downloads().unwrap()[0].state.intent;
    store.set_download_paused(id, intent, true).unwrap();
    recover(&mut store, id, &halted).unwrap();
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
fn an_uncertain_link_error_that_made_no_link_retries_instead_of_halting_changed() {
    // Re-review 2 LOW 1: a network timeout in the link call that created no link left the
    // session marked "possibly visible", and the retry then halted Changed (discarding a complete,
    // valid partial with a message saying it was changed or replaced). With one name on our file
    // and a final name that is not ours, nothing else can see these bytes: publish on retry.
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
    assert!(matches!(
        injected(Effect::Link, io::ErrorKind::TimedOut, || file.link()),
        Err(ExportError::Local {
            retryable: true,
            ..
        })
    ));
    assert!(!fx.downloads.join("notes.txt").exists(), "no link was made");
    let Publication::Ready(journal) = file
        .prepare_publication()
        .expect("the partial is still private")
    else {
        panic!("nothing was published yet");
    };
    store
        .checkpoint_download_export(&lease, file.journal(), &journal)
        .unwrap();
    file.checkpointed(journal);
    let path = file.link().unwrap();
    store
        .publish_download_export(&lease, &file.published_journal())
        .unwrap();
    file.finish(&mut rng()).unwrap();
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn an_uncertain_link_error_that_did_link_never_trims_a_final_file_it_cannot_open() {
    // Re-review 3 LOW 1: the link created the final name but reported an error; then something
    // removed our staging name (our file's only name is now the final one), the final file grew
    // and cannot be opened right now. "Final name not ours" must not be inferred from an open
    // error: the session stays "possibly visible" and nothing is trimmed.
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
    assert!(injected(Effect::LinkReport, io::ErrorKind::TimedOut, || file.link()).is_err());
    let path = fx.downloads.join("notes.txt");
    assert!(path.exists(), "the link was made");
    fs::remove_file(fx.downloads.join("notes.txt.mewtual-part")).unwrap();
    file.release_lock_for_test();
    let mut handle = fs::OpenOptions::new().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut handle, b" grown by its owner").unwrap();
    drop(handle);
    let grown = fs::metadata(&path).unwrap().len();
    // The final name cannot be opened for this check (one injected identity failure).
    assert!(matches!(
        injected(Effect::Identity, io::ErrorKind::Other, || file
            .prepare_publication()),
        Err(ExportError::Halted(ExportHalt::Changed))
    ));
    drop(file);
    assert_eq!(fs::metadata(&path).unwrap().len(), grown, "nothing trimmed");
}

#[test]
fn a_long_multibyte_name_saves_with_its_extension_and_numbering_room() {
    // Review U-L3: 120 non-BMP characters made the probe name exceed 255 UTF-16 units (or the
    // staging name exceed 255 bytes on Unix), which looked like a folder without links.
    let fx = fixture();
    let name = format!("{}.txt", "\u{1F600}".repeat(120));
    for expected_number in [0, 1] {
        let file = ExportFile::create(&fx.downloads, &name, export_plan(), &mut rng()).unwrap();
        let journal = file.journal().clone();
        assert!(journal.final_leaf.ends_with(".txt"));
        assert!(external::component_units(&journal.staging) <= 255);
        if expected_number == 1 {
            assert!(
                journal.final_leaf.contains(" (1)"),
                "{}",
                journal.final_leaf
            );
        }
        drop(file);
        // Occupy the final name so the next save is numbered.
        fs::write(fx.downloads.join(&journal.final_leaf), b"taken").unwrap();
    }
}

#[test]
fn a_failed_removal_write_keeps_the_live_session_exclusive() {
    // Review L7: Remove's Deleting write fails on a full vault. The record is still
    // Completed/Writing, so the older session's presence must still refuse a second writer.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let older = started(&mut store, &lease, &fx.downloads);
    let journal = older.journal().clone();
    assert!(
        injected(Effect::RecordWrite, io::ErrorKind::StorageFull, || {
            store.cancel_download(id)
        })
        .is_err()
    );
    assert_eq!(state(&store, id), ExportState::Writing(journal.clone()));
    let lease = reverified(&mut store, id);
    let elsewhere = fx.downloads.with_file_name("Elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let unrelated = ExportFile::create(&elsewhere, "other.txt", export_plan(), &mut rng()).unwrap();
    assert!(
        store
            .resume_download_export(&lease, &journal, unrelated.presence())
            .is_err(),
        "the older session still owns the partial"
    );
    drop(older);
    store
        .resume_download_export(&lease, &journal, unrelated.presence())
        .expect("free once the older session is gone");
}

#[test]
fn a_chat_save_of_a_halted_journal_is_refused_before_any_reverification() {
    // Review L10: only the explicit Resume (intent CAS clears the halt) or Save new copy may
    // continue a halted save; a plain Save must not spend a re-verification first.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let file = started(&mut store, &lease, &fx.downloads);
    let journal = file.journal().clone();
    drop(file);
    halt(&mut store, id, &journal, ExportHalt::Unavailable).unwrap();
    let refused = store.resume_download(id).unwrap_err();
    assert!(
        refused.to_string().contains("Saving this file stopped"),
        "{refused}"
    );
    let again = store.begin_download(vec![plan()], 0, [9; 32]).unwrap_err();
    assert!(
        again.to_string().contains("Saving this file stopped"),
        "{again}"
    );
    let intent = store.downloads().unwrap()[0].state.intent;
    store.set_download_paused(id, intent, false).unwrap();
    store
        .resume_download(id)
        .expect("the explicit Resume cleared the halt");
}

#[test]
fn admission_on_a_full_vault_fails_cleanly_without_faulting_the_store() {
    // Review L6: the new transfer directory itself cannot be created.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let refused = injected(Effect::CreateDir, io::ErrorKind::StorageFull, || {
        store.begin_download(vec![plan()], 0, [9; 32])
    });
    assert!(matches!(&refused, Err(error) if persistence::is_clean_disk_full(error)));
    assert!(store.downloads().unwrap().is_empty(), "nothing admitted");
    let run = store
        .begin_download(vec![plan()], 0, [9; 32])
        .expect("the store is not faulted; a later admission works");
    verify_private(&mut store, run);
}

#[test]
fn an_empty_file_saves_as_one_empty_chunk_across_restart() {
    // Review U-M5: an empty file is one empty chunk. Its plan, its complete checkpoint (1, 0),
    // restart, resume and publication must all work, ending in a zero-byte final file.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let ciphertext = b"empty file ciphertext";
    let empty = KeepPlan {
        cid: Cid::of(b""),
        version: [2; 32],
        manifest: b"opaque empty manifest".to_vec(),
        chunks: vec![(Cid::of(ciphertext), ciphertext.len() as u64)],
    };
    let run = store.begin_download(vec![empty], 0, [9; 32]).unwrap();
    let id = run.id();
    store.put_download(run, ciphertext).unwrap();
    let lease = store.finish_download(run).unwrap();
    let plan = ExportPlan::for_test(&[b""]);
    let mut file =
        ExportFile::create(&fx.downloads, "empty.txt", plan.clone(), &mut rng()).unwrap();
    store
        .begin_download_export(&lease, file.journal(), file.presence())
        .unwrap();
    assert_eq!(file.accept(0, b"").unwrap().done, 1);
    let checkpoint = file.checkpoint().unwrap();
    assert_eq!(
        (checkpoint.written_chunks, checkpoint.written_bytes),
        (1, 0)
    );
    store
        .checkpoint_download_export(&lease, file.journal(), &checkpoint)
        .unwrap();
    drop(file);
    drop(store); // restart with the complete checkpoint
    let mut store = open(&fx.vault);
    assert_eq!(state(&store, id), ExportState::Writing(checkpoint.clone()));
    let run = store.resume_download(id).unwrap();
    store.put_download(run, ciphertext).unwrap();
    let lease = store.finish_download(run).unwrap();
    let ExportReopen::Ready(mut file) = ExportFile::reopen(&checkpoint, plan).unwrap() else {
        panic!("the (1, 0) checkpoint reopens");
    };
    store
        .resume_download_export(&lease, &checkpoint, file.presence())
        .unwrap();
    file.accept(0, b"").unwrap();
    let path = publish(&mut store, &lease, &mut file);
    file.finish(&mut rng()).unwrap();
    assert_eq!(path, fx.downloads.join("empty.txt"));
    assert_eq!(fs::read(&path).unwrap(), b"");
    assert_eq!(view(&store), ExportView::Published);
    // The (1, 0) journal round-trips through the sealed record.
    let index = store.download_index(id).unwrap();
    let record = store.transfers[index].record.clone();
    assert_eq!(record::Record::decode(&record.encode()).unwrap(), record);
}

#[test]
fn only_the_canonical_empty_manifest_may_carry_an_empty_chunk() {
    let chunk = |size: u64| crate::FileRef {
        plaintext_cid: Cid::of(b"p"),
        ciphertext_cid: Cid::of(b"c"),
        wrapped_key: catcoms_crypto::SealedBlob {
            nonce: [0; 24],
            ciphertext: vec![0; 48],
        },
        size,
        mime: String::new(),
    };
    let manifest = |sizes: &[u64]| crate::FileManifest {
        plaintext_cid: Cid::of(b""),
        total_size: sizes.iter().sum(),
        mime: String::new(),
        chunks: sizes.iter().map(|size| chunk(*size)).collect(),
    };
    assert!(ExportPlan::from_manifest(&manifest(&[0])).is_ok());
    assert!(ExportPlan::from_manifest(&manifest(&[5, 0])).is_err());
    assert!(ExportPlan::from_manifest(&manifest(&[0, 0])).is_err());
    assert!(ExportPlan::from_manifest(&manifest(&[])).is_err());
}

#[test]
fn a_stream_that_fails_verification_never_reaches_a_publishable_checkpoint() {
    // Review L4: every chunk was written, but the stream does not hash to the plan's CID. No
    // checkpoint may follow, so the saved journal never claims every chunk and recovery refuses.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let file = started(&mut store, &lease, &fx.downloads);
    let journal = file.journal().clone();
    drop(file);
    let ExportReopen::Ready(mut file) = ExportFile::reopen(
        &journal,
        ExportPlan {
            cid: Cid::of(b"some other file"),
            ..export_plan()
        },
    )
    .unwrap() else {
        panic!();
    };
    write_all(&mut file);
    assert!(matches!(
        file.prepare_publication(),
        Err(ExportError::Storage(StorageError::Malformed))
    ));
    assert!(file.checkpoint().is_err(), "no checkpoint after a mismatch");
    drop(file);
    assert_eq!(state(&store, id), ExportState::Writing(journal.clone()));
    assert!(recover(&mut store, id, &journal).is_err());
}

#[test]
fn preflight_edges_need_a_current_observation_of_this_mount_and_transfer() {
    // Review U-M3, storage half: a preflight observation retires with a re-verification, a
    // removal step or a remount, so a probe released late records nothing.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let (id, lease) = completed(&mut store);
    let mut file = started(&mut store, &lease, &fx.downloads);
    file.accept(0, CHUNKS[0]).unwrap();
    let journal = file.journal().clone();
    drop(file);
    let (stale, _) = store.download_observation(id).unwrap();
    let _ = reverified(&mut store, id); // a newer Resume re-verified meanwhile
    let fence = ExportFence::Observation(&stale);
    assert!(store
        .halt_download_export(fence, &journal, ExportHalt::Unavailable)
        .is_err());
    assert!(store.recover_download_export(fence, &journal).is_err());
    let (before_remount, _) = store.download_observation(id).unwrap();
    drop(store);
    let mut store = open(&fx.vault);
    assert!(!before_remount.is_current());
    assert!(store
        .halt_download_export(
            ExportFence::Observation(&before_remount),
            &journal,
            ExportHalt::Unavailable
        )
        .is_err());
    assert_eq!(state(&store, id), ExportState::Writing(journal.clone()));
    halt(&mut store, id, &journal, ExportHalt::Unavailable).expect("a fresh observation works");
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
    file.finish(&mut rng()).unwrap();
    assert_eq!(fs::read(path).unwrap(), CHUNKS.concat());
}

#[test]
fn a_folder_without_links_is_refused_at_creation_and_halts_at_publication() {
    // FAT and exFAT report identities but cannot link, and every publication (journaled or the
    // non-resumable writer) is a no-replace link: refuse before any plaintext exists. Windows
    // decides with a link probe whose failure kind does not matter; Unix decides with `fstatfs`
    // (see `without_links`). At publication a real Linux `EPERM` (what vfat reports) is a definite
    // refusal like the injected `Unsupported` below (`external::linkat_error`).
    for kind in [io::ErrorKind::Unsupported, io::ErrorKind::PermissionDenied] {
        let fx = fixture();
        assert!(matches!(
            without_links(kind, || {
                ExportFile::create(&fx.downloads, "notes.txt", export_plan(), &mut rng())
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
            ExportFile::create(&fx.downloads, "notes.txt", export_plan(), &mut rng())
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
    recover(&mut store, id, &journal).unwrap();
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
    remove_partial(&saved, false, &mut rng()).unwrap();
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
    // PR51 H1: reported as Unchecked (the caller keeps Removing), not as an error.
    assert!(matches!(
        injected(Effect::Remove, io::ErrorKind::PermissionDenied, || {
            remove_partial(&journal, false, &mut rng())
        }),
        Ok(PartialRemoval::Unchecked)
    ));
    assert!(
        staging(&fx.downloads).exists(),
        "a refused delete leaves the partial at its own name"
    );
    assert!(store.kept_files().error.is_none());
    remove_partial(&journal, false, &mut rng()).unwrap();
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
    remove_partial(&journal, false, &mut rng()).unwrap();
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
        remove_partial(before.journal(), false, &mut rng()).unwrap(),
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
    halt(&mut store, id, &journal, ExportHalt::Unavailable).unwrap();
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
fn every_exempted_pre_rename_step_refuses_cleanly_and_a_failed_cleanup_still_fails_closed() {
    // Review follow-up: the open, write and flush of `next` are the exempted disk-full steps.
    // Each refuses cleanly; a cleanup that cannot remove the torn `next` or flush its removal
    // keeps the fail-closed poison path.
    for effect in [
        Effect::RecordCreate,
        Effect::RecordWrite,
        Effect::RecordSync,
    ] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        write_all(&mut file);
        let checkpoint = file.checkpoint().unwrap();
        let before = state(&store, id);
        let refused = injected(effect, io::ErrorKind::StorageFull, || {
            store.checkpoint_download_export(&lease, file.journal(), &checkpoint)
        });
        assert!(
            matches!(&refused, Err(error) if persistence::is_clean_disk_full(error)),
            "{effect:?}"
        );
        let index = store.download_index(id).unwrap();
        assert!(!store.transfers[index].poisoned, "{effect:?}");
        assert!(!store.transfers[index].path.join("next").exists());
        assert_eq!(state(&store, id), before);
        store
            .checkpoint_download_export(&lease, file.journal(), &checkpoint)
            .expect("a later write succeeds");
    }
    for cleanup in [Effect::RecordDiscard, Effect::RecordDiscardBarrier] {
        let fx = fixture();
        let mut store = open(&fx.vault);
        let (id, lease) = completed(&mut store);
        let mut file = started(&mut store, &lease, &fx.downloads);
        write_all(&mut file);
        let checkpoint = file.checkpoint().unwrap();
        INJECT_ALSO.with(|slot| slot.set(Some((cleanup, io::ErrorKind::PermissionDenied, 1))));
        let refused = injected(Effect::RecordWrite, io::ErrorKind::StorageFull, || {
            store.checkpoint_download_export(&lease, file.journal(), &checkpoint)
        });
        INJECT_ALSO.with(|slot| slot.set(None));
        assert!(
            matches!(&refused, Err(error) if !persistence::is_clean_disk_full(error)),
            "{cleanup:?}"
        );
        // Fail closed: the transfer is poisoned and the store refuses it until a storage check.
        let failed_closed = store
            .download_index(id)
            .map_or(true, |index| store.transfers[index].poisoned);
        assert!(
            failed_closed,
            "an unproven cleanup fails closed ({cleanup:?})"
        );
    }
}

#[test]
fn a_keep_record_write_on_a_full_vault_refuses_cleanly() {
    // The persistence change also covers public Keep transitions.
    let fx = fixture();
    let mut store = open(&fx.vault);
    let run = store.begin_keep_with_listing(plan(), [8; 32]).unwrap();
    store.keep_work(run, None, 0).unwrap();
    let refused = injected(Effect::RecordWrite, io::ErrorKind::StorageFull, || {
        store.put_keep(run, b"first")
    });
    assert!(matches!(&refused, Err(error) if persistence::is_clean_disk_full(error)));
    assert!(store.kept_files().error.is_none());
    assert!(!store.transfers[0].poisoned);
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 0);
    store
        .put_keep(run, b"first")
        .expect("a later write succeeds");
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 1);
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
