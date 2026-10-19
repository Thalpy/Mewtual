use super::*;
use crate::MemoryBlobStore;
use catcoms_crypto::DeviceId;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

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
fn complete(store: &mut KeptBlobStore<ChaCha20Rng>, run: DownloadRun) -> DownloadExportLease {
    store.put_download(run, b"first").unwrap();
    store.put_download(run, b"second").unwrap();
    store.finish_download(run).unwrap()
}

#[test]
fn completed_download_is_private_in_all_public_source_paths_and_revokes_export_on_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
    let lease = complete(&mut store, run);
    assert!(lease.is_current());
    assert!(store.kept_files().files.is_empty() && store.kept_files().transfers.is_empty());
    assert!(store.automatic_keep_candidate(None).is_none());
    assert!(store.kept_saved_plans(&plan.cid).is_none());
    assert!(
        store.kept_files().allocated_bytes > 800_000,
        "private custody and both journals are charged"
    );
    for (cid, bound) in &plan.chunks {
        assert!(store.get(cid).unwrap().is_none());
        assert!(store.get_bounded(cid, *bound as usize).unwrap().is_none());
        assert!(!store.has(cid));
        assert!(
            store
                .prepare_read_bounded(cid, *bound as usize)
                .unwrap()
                .run()
                .unwrap()
                .is_none(),
            "actual source-worker preparation cannot see completed private downloads"
        );
    }
    assert!(store.cids().is_empty());
    assert_eq!(
        store
            .read_download_export(&lease, &plan.chunks[0].0, 5)
            .unwrap(),
        b"first"
    );
    store.cancel_download(run.id()).unwrap();
    assert!(!lease.is_current());
    assert!(store
        .read_download_export(&lease, &plan.chunks[0].0, 5)
        .is_err());
    assert!(store.downloads().unwrap().is_empty());
}

#[test]
fn same_cid_keep_and_download_have_independent_active_cleanup_and_custody_in_both_orders() {
    for cancel_download_first in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plan = plan();
        let keep = store.begin_keep(plan.clone()).unwrap();
        store.put_keep(keep, b"first").unwrap();
        store.put_keep(keep, b"second").unwrap();
        store.finish_keep(keep).unwrap();
        let run = store
            .begin_download(vec![plan.clone()], 0, [9; 32])
            .unwrap();
        assert!(
            store.forget_kept(&plan.cid).is_err(),
            "same CID cannot erase another active owner"
        );
        assert!(store.download_run_current(run, &plan.cid, &plan.version));
        let lease = complete(&mut store, run);
        let keep = store.begin_keep(plan.clone()).unwrap();
        let work = store.keep_work(keep, None, 100).unwrap();
        assert!(store.cancel_download(run.id()).is_err());
        assert!(
            work.run_is_live(),
            "failed Download cancellation cannot pause active Keep"
        );
        assert!(store.keep_run_current(keep, &plan.cid, &plan.version));
        store.pause_keep(keep).unwrap();
        if cancel_download_first {
            store.cancel_download(run.id()).unwrap();
            assert!(store.has(&plan.chunks[0].0));
            assert!(!lease.is_current());
            store.forget_kept(&plan.cid).unwrap();
        } else {
            store.forget_kept(&plan.cid).unwrap();
            assert!(!store.has(&plan.chunks[0].0));
            assert!(lease.is_current());
            assert_eq!(
                store
                    .read_download_export(&lease, &plan.chunks[0].0, 5)
                    .unwrap(),
                b"first"
            );
            store.cancel_download(run.id()).unwrap();
        }
        assert!(store.downloads().unwrap().is_empty());
        assert!(store.kept_files().transfers.is_empty());
    }
}

#[test]
fn restart_preserves_download_spend_and_partial_chunks_without_export_permission() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
    let work = store
        .download_work(
            run,
            Some(KeepNetworkWork {
                bytes: 5,
                paged: true,
            }),
            100,
        )
        .unwrap();
    store.put_download(run, b"first").unwrap();
    let before = store.downloads().unwrap()[0].clone();
    drop(store);
    assert!(!work.run_is_live());
    let mut store = open(dir.path());
    let after = store.downloads().unwrap()[0].clone();
    assert_eq!(after.state.remaining, before.state.remaining);
    assert_eq!(after.state.grant, before.state.grant);
    assert_eq!(after.state.intent, before.state.intent);
    assert!(!after.state.running && !after.checked);
    assert_eq!(after.state.verified_chunks, 1);
    assert!(store.download_export(run.id()).is_err());
    assert!(store.automatic_keep_candidate(None).is_none());
    let resumed = store.resume_download(run.id()).unwrap();
    assert_ne!(resumed, run);
    assert_eq!(
        store
            .get_download(resumed, &plan.chunks[0].0, 5)
            .unwrap()
            .unwrap(),
        b"first"
    );
    let lease = complete(&mut store, resumed);
    assert!(lease.is_current());
    drop(store);
    assert!(!lease.is_current());
    let store = open(dir.path());
    assert!(
        store.download_export(run.id()).is_err(),
        "restart requires whole-file revalidation and fresh native scope"
    );
}

#[test]
fn download_record_purpose_and_directory_identity_are_immutable_and_legacy_bytes_stay_canonical() {
    let mut rng = ChaCha20Rng::seed_from_u64(7);
    let plan = plan();
    let record =
        record::Record::initial_download(scope(), vec![plan.clone()], 0, [9; 32], &mut rng)
            .unwrap();
    let encoded = record.encode();
    let decoded = record::Record::decode(&encoded).unwrap();
    assert_eq!(decoded, record);
    assert!(!decoded.public_retained());
    assert!(Owner::parse(&decoded.owner().name()).is_some());
    assert!(Owner::parse(&decoded.owner().name().to_uppercase()).is_none());
    let mut next = record.successor().unwrap();
    assert!(record.validate_successor(&next).is_ok());
    next.encoding = record::RecordEncoding::V4;
    assert!(record.validate_successor(&next).is_err());
    let legacy = record::Record::initial(scope(), plan, [9; 32], &mut rng).unwrap();
    assert_eq!(
        record::Record::decode(&legacy.encode()).unwrap().encode(),
        legacy.encode()
    );
    assert!(legacy.public_retained());
}

#[test]
fn export_leases_are_bound_to_both_the_issuing_mount_and_transfer_generation() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut first = open(a.path());
    let mut second = open(b.path());
    let plan = plan();
    let a_run = first
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
    let b_run = second
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
    assert_eq!(
        a_run.id(),
        b_run.id(),
        "test RNG deliberately reproduces saved identifiers"
    );
    let a_lease = complete(&mut first, a_run);
    let b_lease = complete(&mut second, b_run);
    assert!(second
        .read_download_export(&a_lease, &plan.chunks[0].0, 5)
        .is_err());
    // Test the transfer binding independently of the mount binding. Public callers cannot
    // construct this hybrid, but neither half may become optional in a future refactor.
    let mut hybrid = a_lease.clone();
    hybrid.mount = b_lease.mount.clone();
    assert!(second
        .read_download_export(&hybrid, &plan.chunks[0].0, 5)
        .is_err());
    assert_eq!(
        second
            .read_download_export(&b_lease, &plan.chunks[0].0, 5)
            .unwrap(),
        b"first"
    );
}

#[test]
fn export_refuses_oversized_corrupt_leaf_at_the_saved_bound_before_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
    let lease = complete(&mut store, run);
    let index = store.download_index(run.id()).unwrap();
    let large = store.transfers[index].store.put(&[42; 1024]).unwrap();
    let leaf = store.transfers[index].path.clone();
    fs::copy(
        leaf.join(large.to_hex()),
        leaf.join(plan.chunks[0].0.to_hex()),
    )
    .unwrap();
    assert!(
        matches!(
            store.read_download_export(&lease, &plan.chunks[0].0, 2048),
            Err(StorageError::BlobSizeLimit)
        ),
        "the saved five-byte bound must stop the read before AEAD/CID checks"
    );
}

#[test]
fn completed_download_pause_resume_retires_old_export_and_preserves_intent_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
    let old = complete(&mut store, run);
    let before = store.downloads().unwrap().remove(0);
    store
        .set_download_paused(run.id(), before.state.intent, true)
        .unwrap();
    assert!(!old.is_current());
    assert!(store.download_export(run.id()).is_err());
    assert!(store
        .set_download_paused(run.id(), before.state.intent, false)
        .is_err());
    drop(store);
    let mut store = open(dir.path());
    let paused = store.downloads().unwrap().remove(0);
    assert!(paused.state.user_paused);
    assert_eq!(paused.state.remaining, before.state.remaining);
    assert!(store.resume_download(run.id()).is_err());
    store
        .set_download_paused(run.id(), paused.state.intent, false)
        .unwrap();
    let resumed = store.resume_download(run.id()).unwrap();
    let new = complete(&mut store, resumed);
    assert!(new.is_current() && !old.is_current());
}

#[test]
fn download_exhaustion_survives_restart_and_exact_retry_replay_cannot_disturb_active_keep() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store
        .begin_download(vec![plan.clone()], 0, [9; 32])
        .unwrap();
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
    let mut store = open(dir.path());
    assert!(matches!(
        store.begin_download(vec![plan.clone()], 0, [9; 32]),
        Err(StorageError::KeepExhausted)
    ));
    let retry = KeepRetry {
        transfer: *run.id().as_bytes(),
        expected_grant: 1,
        request: [2; 16],
    };
    let receipt = store.retry_download(run.id(), retry).unwrap();
    assert_eq!(receipt.granted, 2);
    let resumed = store.resume_download(run.id()).unwrap();
    store.download_work(resumed, None, 0).unwrap();
    store.pause_download(resumed).unwrap();
    let spent = store.downloads().unwrap().remove(0).state.remaining;
    let keep = store.begin_keep(plan.clone()).unwrap();
    let work = store.keep_work(keep, None, 0).unwrap();
    assert_eq!(store.retry_download(run.id(), retry).unwrap(), receipt);
    assert!(work.run_is_live() && store.keep_run_current(keep, &plan.cid, &plan.version));
    assert_eq!(store.downloads().unwrap().remove(0).state.remaining, spent);
    assert!(store
        .retry_download(
            run.id(),
            KeepRetry {
                request: [3; 16],
                ..retry
            }
        )
        .is_err());
}

#[test]
fn downloads_and_keeps_share_the_same_record_quota_even_when_private_records_are_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    for index in 0..KEPT_FILE_LIMIT {
        let plan = KeepPlan {
            cid: Cid::of(&[index as u8]),
            ..plan()
        };
        if index % 2 == 0 {
            let run = store.begin_download(vec![plan], 0, [9; 32]).unwrap();
            store.pause_download(run).unwrap();
        } else {
            let run = store.begin_keep(plan).unwrap();
            store.pause_keep(run).unwrap();
        }
    }
    assert_eq!(store.downloads().unwrap().len(), KEPT_FILE_LIMIT / 2);
    assert_eq!(store.kept_files().transfers.len(), KEPT_FILE_LIMIT / 2);
    assert!(store.begin_download(vec![plan()], 0, [9; 32]).is_err());
    assert!(store.begin_keep(plan()).is_err());
}

#[test]
fn old_export_cleanup_cannot_delete_a_later_run_or_verified_export_of_the_same_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let run = store.begin_download(vec![plan()], 0, [9; 32]).unwrap();
    let old = complete(&mut store, run);
    let resumed = store.resume_download(run.id()).unwrap();
    assert!(store.finish_download_export(&old).is_err());
    assert!(store.download_run_current(resumed, &plan().cid, &plan().version));
    let fresh = complete(&mut store, resumed);
    assert!(store.finish_download_export(&old).is_err());
    assert!(fresh.is_current());
    store.finish_download_export(&fresh).unwrap();
    assert!(!fresh.is_current());
    assert!(store.downloads().unwrap().is_empty());
}

#[test]
fn private_owner_and_scope_are_validated_before_pending_journal_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let run = store.begin_download(vec![plan()], 0, [9; 32]).unwrap();
    store.pause_download(run).unwrap();
    let index = store.download_index(run.id()).unwrap();
    let transfer = &mut store.transfers[index];
    let path = transfer.path.clone();
    let mut next = transfer.record.successor().unwrap();
    next.remaining.requests -= 16;
    assert!(transfer
        .publish_with_hook(next, &[8; 32], &mut store.rng, |phase, _| {
            if phase == persistence::WritePhase::Written {
                Err(StorageError::Io("injected crash".into()))
            } else {
                Ok(())
            }
        })
        .is_err());
    let current = fs::read(path.join("current")).unwrap();
    let pending = fs::read(path.join("next")).unwrap();
    drop(store);
    assert!(
        Cid::from_hex(path.file_name().unwrap().to_str().unwrap()).is_none(),
        "canonical Download paths are refused by the legacy CID-only namespace"
    );
    let wrong = path.with_file_name(plan().cid.to_hex());
    fs::rename(&path, &wrong).unwrap();
    let mut store = KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        dir.path().to_owned(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    assert!(store.configure_keep_scope(scope()).is_err());
    assert_eq!(fs::read(wrong.join("current")).unwrap(), current);
    assert_eq!(fs::read(wrong.join("next")).unwrap(), pending);
    drop(store);
    fs::rename(&wrong, &path).unwrap();
    let mut store = KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        dir.path().to_owned(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    let mut foreign = scope();
    foreign.device = DeviceId::from_bytes([99; 32]);
    assert!(store.configure_keep_scope(foreign).is_err());
    assert_eq!(fs::read(path.join("current")).unwrap(), current);
    assert_eq!(fs::read(path.join("next")).unwrap(), pending);
    drop(store);
    let store = open(dir.path());
    let recovered = store.downloads().unwrap().remove(0);
    assert_eq!(recovered.state.remaining.requests, 131_072 - 16);
    assert!(!path.join("next").exists());
}
