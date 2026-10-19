//! Actual disk regressions for the new ownership format. They establish process restart and
//! real file/directory API behavior, not physical power-loss or a network custody receipt.
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
fn unbound(path: &Path) -> KeptBlobStore<ChaCha20Rng> {
    KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        path.to_path_buf(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    )
}
fn open(path: &Path) -> KeptBlobStore<ChaCha20Rng> {
    let mut store = unbound(path);
    store.configure_keep_scope(scope()).unwrap();
    store
}
fn plan() -> KeepPlan {
    KeepPlan {
        cid: Cid::of(b"firstsecond"),
        version: [1; 32],
        manifest: b"opaque exact manifest".to_vec(),
        chunks: vec![(Cid::of(b"first"), 5), (Cid::of(b"second"), 6)],
    }
}

#[test]
fn equal_metadata_key_refuses_retention_before_acquiring_lease_and_preserves_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut primary = MemoryBlobStore::new();
    let cid = primary.put(b"ordinary cache").unwrap();
    let mut refused = KeptBlobStore::open(
        Box::new(primary),
        dir.path().to_path_buf(),
        [7; 32],
        [7; 32],
        ChaCha20Rng::seed_from_u64(3),
    );
    assert!(refused.configure_keep_scope(scope()).is_err());
    assert!(refused.begin_keep(plan()).is_err());
    assert_eq!(refused.get(&cid).unwrap().unwrap(), b"ordinary cache");
    assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
    // A legitimate mount still obtains the exclusive lease while the refused handle lives.
    assert!(open(dir.path()).kept_files().error.is_none());
}

#[test]
fn failed_completed_release_retains_a_visible_charged_transfer_without_verified_copy() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store.begin_keep(plan.clone()).unwrap();
    store.put_keep(run, b"first").unwrap();
    store.put_keep(run, b"second").unwrap();
    store.finish_keep(run).unwrap();
    let charged = store.kept_files().allocated_bytes;
    fs::write(
        dir.path()
            .join(TRANSFERS)
            .join(plan.cid.to_hex())
            .join("next"),
        b"blocks a fresh publication",
    )
    .unwrap();
    assert!(store.forget_kept(&plan.cid).is_err());
    let faulted = store.kept_files();
    assert!(faulted.error.is_some());
    assert!(
        faulted.files.is_empty(),
        "failed publication cannot retain a verified-copy projection"
    );
    assert_eq!(
        faulted.transfers.len(),
        1,
        "quota owner remains observable for a blocked cleanup"
    );
    assert_eq!(faulted.transfers[0].phase, KeepPhase::Completed);
    assert_eq!(faulted.allocated_bytes, charged);
}

#[test]
fn legacy_and_resumable_reservations_share_quota_and_failed_deletion_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    // Write a genuine legacy completed manifest. Absent chunks are still fully reserved: repair
    // ownership must not obtain a smaller charge merely because its disk files disappeared.
    let mut legacy = plan();
    legacy.cid = Cid::of(b"legacy large reservation");
    legacy.chunks = (0..100u32)
        .map(|index| (Cid::of(&index.to_be_bytes()), CHUNK_LIMIT_BYTES))
        .collect();
    let path = dir.path().join(legacy.cid.to_hex());
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join(META),
        encode_sealed(&seal(&[7; 32], &legacy.encode(), &mut shared_rng()).unwrap()),
    )
    .unwrap();
    let mut store = open(dir.path());
    assert_eq!(store.kept_files().files.len(), 1);
    let mut partial = plan();
    partial.cid = Cid::of(b"new large reservation");
    partial.chunks = (100..127u32)
        .map(|index| (Cid::of(&index.to_be_bytes()), CHUNK_LIMIT_BYTES))
        .collect();
    let run = store.begin_keep(partial.clone()).unwrap();
    store.pause_keep(run).unwrap();
    let reserved = store.kept_files().allocated_bytes;
    assert!(reserved < KEPT_BYTE_LIMIT);
    let mut extra = plan();
    extra.cid = Cid::of(b"over combined limit");
    extra.chunks = vec![(Cid::of(b"extra full chunk"), CHUNK_LIMIT_BYTES)];
    assert!(
        store.begin_keep(extra.clone()).is_err(),
        "individually valid plans exceed the combined legacy/V2 quota"
    );
    drop(store);
    let mut store = open(dir.path());
    assert_eq!(store.kept_files().allocated_bytes, reserved);
    assert!(store.begin_keep(extra).is_err());
    // A surprising leaf prevents safe cleanup AFTER durable Deleting. Its reservation must
    // remain charged, and even restart must not turn the completed legacy copy into free quota.
    let partial_path = dir.path().join(TRANSFERS).join(partial.cid.to_hex());
    fs::write(partial_path.join("unexpected"), b"do not recursively erase").unwrap();
    assert!(store.forget_kept(&partial.cid).is_err());
    assert!(store.kept_files().allocated_bytes >= reserved);
    assert!(store.begin_keep(plan()).is_err());
    drop(store);
    let mut store = unbound(dir.path());
    assert!(store.configure_keep_scope(scope()).is_err());
    assert!(store.kept_files().error.is_some());
    assert!(store.begin_keep(plan()).is_err());
    assert!(dir
        .path()
        .join(DELETING)
        .join(partial.cid.to_hex())
        .join("unexpected")
        .exists());
}

#[test]
fn partial_keep_survives_scope_bound_restart_and_requires_a_fresh_run() {
    let dir = tempfile::tempdir().unwrap();
    let plan = plan();
    let mut store = open(dir.path());
    let run = store
        .begin_keep_with_listing(plan.clone(), [8; 32])
        .unwrap();
    let work = store.keep_work(run, None, 0).unwrap();
    store.put_keep(run, b"first").unwrap();
    let charge = store.kept_files().allocated_bytes;
    let remaining = store.kept_files().transfers[0].remaining;
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 1);
    assert!(
        !store.has(&Cid::of(b"first")),
        "partial ciphertext is private to this exact run"
    );
    assert_eq!(
        store.get_keep(run, &Cid::of(b"first"), 5).unwrap().unwrap(),
        b"first"
    );
    drop(store);
    let mut store = unbound(dir.path());
    assert!(store.kept_files().transfers.is_empty());
    assert!(!store.has(&Cid::of(b"first")));
    assert!(store.begin_keep(plan.clone()).is_err());
    store.configure_keep_scope(scope()).unwrap();
    assert_eq!(store.kept_files().allocated_bytes, charge);
    assert_eq!(store.kept_files().transfers[0].remaining, remaining);
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 1);
    let next = store.begin_keep(plan.clone()).unwrap();
    assert_ne!(next, run);
    assert!(store.put_keep(run, b"second").is_err());
    assert!(store.validate_keep_work(&work, 0).is_err());
    assert_eq!(
        store
            .get_keep(next, &Cid::of(b"first"), 5)
            .unwrap()
            .unwrap(),
        b"first"
    );
    store.keep_work(next, None, 0).unwrap();
    assert_eq!(
        store.kept_files().transfers[0].remaining.slices,
        remaining.slices - 1
    );
    assert!(
        store.finish_keep(next).is_err(),
        "whole-file verification must cover the resumed prefix again"
    );
    store.put_keep(next, b"first").unwrap();
    store.put_keep(next, b"second").unwrap();
    store.finish_keep(next).unwrap();
    assert!(store.kept_files().files[0].checked);
    assert_eq!(store.kept_files().allocated_bytes, charge);
    store.forget_kept(&plan.cid).unwrap();
    assert!(!store.has(&Cid::of(b"first")));
    assert!(
        dir.path().join(FORMAT).is_file(),
        "release never removes the downgrade barrier"
    );
    assert_eq!(store.kept_files().allocated_bytes, FORMAT_CHARGE);
}

#[test]
fn scope_failure_is_permanent_on_the_handle_and_keeps_primary_cache_usable() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut wrong = scope();
    wrong.device = DeviceId::from_bytes([9; 32]);
    assert!(store.configure_keep_scope(wrong).is_err());
    assert!(store.configure_keep_scope(scope()).is_err());
    assert!(store.begin_keep(plan()).is_err());
    let cid = store.put(b"ordinary cache remains usable").unwrap();
    assert_eq!(
        store.get(&cid).unwrap().unwrap(),
        b"ordinary cache remains usable"
    );
}

#[test]
fn execution_exhaustion_survives_restart_and_retry_is_an_exact_cas_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let plan = plan();
    let mut store = open(dir.path());
    let run = store.begin_keep(plan.clone()).unwrap();
    for slice in 0..120 {
        store
            .keep_work(run, None, slice * KEEP_EXECUTION_SLICE_MS)
            .unwrap();
    }
    assert!(matches!(
        store.keep_work(run, None, 120 * KEEP_EXECUTION_SLICE_MS),
        Err(StorageError::KeepExhausted)
    ));
    store.pause_keep(run).unwrap();
    let identity = store.kept_files().transfers[0].transfer;
    drop(store);
    let mut store = open(dir.path());
    assert!(matches!(
        store.begin_keep(plan.clone()),
        Err(StorageError::KeepExhausted)
    ));
    let retry = KeepRetry {
        transfer: identity,
        expected_grant: 1,
        request: [2; 16],
    };
    let receipt = store.retry_kept(&plan.cid, retry).unwrap();
    assert_eq!(receipt.granted, 2);
    let next = store.begin_keep(plan.clone()).unwrap();
    let work = store.keep_work(next, None, 0).unwrap();
    let spent = store.kept_files().transfers[0].remaining;
    let revision = store.transfers[0].record.revision;
    assert_eq!(
        store.retry_kept(&plan.cid, retry).unwrap(),
        receipt,
        "lost reply recovers metadata only"
    );
    assert_eq!(
        store.kept_files().transfers[0].remaining,
        spent,
        "an exact retry cannot refill consumed capacity"
    );
    assert_eq!(store.transfers[0].record.revision, revision);
    store.validate_keep_work(&work, 1).unwrap();
    assert!(
        store.kept_files().transfers[0].running,
        "receipt recovery never pauses the active run"
    );
    assert!(store
        .retry_kept(
            &plan.cid,
            KeepRetry {
                request: [3; 16],
                ..retry
            }
        )
        .is_err());
    store.pause_keep(next).unwrap();
    drop(store);
    let store = open(dir.path());
    assert_eq!(store.kept_files().transfers[0].remaining, spent);
}

fn shared_rng() -> SharedRng<ChaCha20Rng> {
    SharedRng(std::sync::Arc::new(std::sync::Mutex::new(
        ChaCha20Rng::seed_from_u64(31),
    )))
}
fn transfer(root: &Path) -> Transfer<ChaCha20Rng> {
    let rng = shared_rng();
    let record = record::Record::initial(scope(), plan(), [8; 32], &mut rng.clone()).unwrap();
    Transfer::create(
        root.join(record.plan.cid.to_hex()),
        record,
        [7; 32],
        [8; 32],
        rng,
    )
    .unwrap()
}
fn injected() -> StorageError {
    StorageError::Io("injected persistence boundary failure".into())
}

#[test]
fn pending_debit_restores_file_flush_before_publication_and_rolls_forward_once() {
    use persistence::WritePhase;
    let dir = tempfile::tempdir().unwrap();
    let mut owner = transfer(dir.path());
    let path = owner.path.clone();
    let before = fs::read(path.join("current")).unwrap();
    let mut debit = owner.record.successor().unwrap();
    debit.remaining.requests -= 16;
    debit.remaining.slices -= 1;
    assert!(owner
        .publish_with_hook(debit.clone(), &[8; 32], &mut shared_rng(), |phase, _| {
            if phase == WritePhase::Written {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .is_err());
    assert!(owner.poisoned);
    assert!(owner
        .publish(debit.clone(), &[8; 32], &mut shared_rng())
        .is_err());
    assert_eq!(fs::read(path.join("current")).unwrap(), before);
    drop(owner);
    let refused = Transfer::open_with_barriers(
        path.clone(),
        &scope(),
        [7; 32],
        [8; 32],
        shared_rng(),
        |file| {
            assert_eq!(file.file_name().unwrap(), "next");
            Err(injected())
        },
        |_| panic!("a failed content flush cannot be followed by directory publication"),
    );
    assert!(refused.is_err());
    assert!(path.join("next").exists());
    assert_eq!(fs::read(path.join("current")).unwrap(), before);
    let owner = Transfer::open(path.clone(), &scope(), [7; 32], [8; 32], shared_rng()).unwrap();
    assert_eq!(owner.record, debit);
    assert!(!path.join("next").exists());
    drop(owner);
    assert_eq!(
        Transfer::open(path, &scope(), [7; 32], [8; 32], shared_rng())
            .unwrap()
            .record,
        debit
    );
}

#[test]
fn readable_chunk_after_failed_flush_must_be_flushed_before_resumed_progress() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let run = store.begin_keep(plan()).unwrap();
    assert!(store
        .put_keep_with_barriers(
            run,
            b"first",
            |_| Err(injected()),
            |_| panic!("file flush failed")
        )
        .is_err());
    assert_eq!(
        store.get_keep(run, &Cid::of(b"first"), 5).unwrap().unwrap(),
        b"first"
    );
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 0);
    store.pause_keep(run).unwrap();
    drop(store);
    let mut store = open(dir.path());
    let run = store.begin_keep(plan()).unwrap();
    // This traverses the matching-held-bytes branch. A failure must still prevent progress.
    assert!(store
        .put_keep_with_barriers(
            run,
            b"first",
            |_| Err(injected()),
            |_| panic!("file flush failed")
        )
        .is_err());
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 0);
    assert!(store.finish_keep(run).is_err());
    store.put_keep(run, b"first").unwrap();
    assert_eq!(store.kept_files().transfers[0].verified_chunks, 1);
    store.put_keep(run, b"second").unwrap();
    store.finish_keep(run).unwrap();
    assert!(store.kept_files().files[0].checked);
}

#[test]
fn visible_rename_refuses_effects_until_the_reopen_directory_barrier_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = transfer(dir.path());
    let path = owner.path.clone();
    let mut debit = owner.record.successor().unwrap();
    debit.remaining.slices -= 1;
    assert!(owner
        .publish_with_hook(debit.clone(), &[8; 32], &mut shared_rng(), |phase, _| {
            if phase == persistence::WritePhase::Renamed {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .is_err());
    assert!(owner.poisoned);
    drop(owner);
    let refused = Transfer::open_with_barriers(
        path.clone(),
        &scope(),
        [7; 32],
        [8; 32],
        shared_rng(),
        |file| {
            assert_eq!(file.file_name().unwrap(), "current");
            flush(file)
        },
        |_| Err(injected()),
    );
    assert!(refused.is_err());
    assert!(!path.join("next").exists());
    assert_eq!(
        Transfer::open(path, &scope(), [7; 32], [8; 32], shared_rng())
            .unwrap()
            .record,
        debit
    );
}

#[test]
fn first_publication_failure_at_every_boundary_is_conservative_on_reopen() {
    use persistence::WritePhase;
    for phase in [
        WritePhase::Created,
        WritePhase::Written,
        WritePhase::FileFlushed,
        WritePhase::Renamed,
        WritePhase::DirectoryFlushed,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let record = record::Record::initial(scope(), plan(), [8; 32], &mut shared_rng()).unwrap();
        let path = dir.path().join(record.plan.cid.to_hex());
        let failed = Transfer::create_with_hook(
            path.clone(),
            record.clone(),
            [7; 32],
            [8; 32],
            shared_rng(),
            |seen, _| {
                if seen == phase {
                    Err(injected())
                } else {
                    Ok(())
                }
            },
        );
        assert!(
            failed.is_err(),
            "the caller must receive no usable owner at {phase:?}"
        );
        let reopened = Transfer::open(path, &scope(), [7; 32], [8; 32], shared_rng());
        if phase == WritePhase::Created {
            assert!(
                reopened.is_err(),
                "a partial pending record cannot restore initial allowance"
            );
        } else {
            let reopened = reopened.unwrap();
            assert_eq!(reopened.record, record);
            assert!(!reopened.checked);
        }
    }
}

#[test]
fn invalid_either_record_never_selects_an_older_or_newer_fallback() {
    for corrupt_current in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let owner = transfer(dir.path());
        let path = owner.path.clone();
        let mut deletion = owner.record.successor().unwrap();
        deletion.phase = KeepPhase::Deleting;
        let encoded =
            encode_sealed(&seal(&[8; 32], &deletion.encode(), &mut shared_rng()).unwrap());
        fs::write(path.join("next"), &encoded).unwrap();
        fs::write(
            path.join(if corrupt_current { "current" } else { "next" }),
            b"corrupt record",
        )
        .unwrap();
        let current = fs::read(path.join("current")).unwrap();
        let next = fs::read(path.join("next")).unwrap();
        drop(owner);
        assert!(Transfer::open(path.clone(), &scope(), [7; 32], [8; 32], shared_rng()).is_err());
        assert_eq!(fs::read(path.join("current")).unwrap(), current);
        assert_eq!(fs::read(path.join("next")).unwrap(), next);
    }
}

#[test]
fn wrong_scope_path_or_predecessor_refuses_before_reconciliation_writes() {
    for wrong in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let owner = transfer(dir.path());
        let mut path = owner.path.clone();
        let mut next = owner.record.successor().unwrap();
        next.remaining.slices -= 1;
        if wrong == 0 {
            next.scope.device = DeviceId::from_bytes([9; 32]);
        }
        if wrong == 1 {
            next.predecessor = [9; 32];
        }
        fs::write(
            path.join("next"),
            encode_sealed(&seal(&[8; 32], &next.encode(), &mut shared_rng()).unwrap()),
        )
        .unwrap();
        drop(owner);
        if wrong == 2 {
            let misplaced = dir.path().join(Cid::of(b"different path").to_hex());
            fs::rename(&path, &misplaced).unwrap();
            path = misplaced;
        }
        let current = fs::read(path.join("current")).unwrap();
        let next = fs::read(path.join("next")).unwrap();
        assert!(Transfer::open_with_barriers(
            path.clone(),
            &scope(),
            [7; 32],
            [8; 32],
            shared_rng(),
            |_| panic!("validation must precede flush"),
            |_| panic!("validation must precede publication")
        )
        .is_err());
        assert_eq!(fs::read(path.join("current")).unwrap(), current);
        assert_eq!(fs::read(path.join("next")).unwrap(), next);
    }
}

#[test]
fn maximum_record_and_closed_successor_rules_fit_the_two_record_rail() {
    let mut plan = plan();
    plan.manifest = vec![0; 64 * 1024];
    plan.chunks = (0..128u32)
        .map(|index| (Cid::of(&index.to_be_bytes()), CHUNK_LIMIT_BYTES))
        .collect();
    let mut maximum_scope = scope();
    maximum_scope.group = vec![1; 256];
    let mut record =
        record::Record::initial(maximum_scope, plan, [7; 32], &mut shared_rng()).unwrap();
    record.exhausted = true;
    // This represents a published successor, not a malformed genesis record.
    record.revision = 2;
    record.predecessor = [3; 32];
    let (retried, _) = record
        .retry(KeepRetry {
            transfer: record.transfer,
            expected_grant: 1,
            request: [2; 16],
        })
        .unwrap();
    let encoded = retried.encode();
    assert!(encoded.len() + 40 <= record::RECORD_LIMIT);
    assert_eq!(record::Record::decode(&encoded).unwrap(), retried);
    assert!(
        retried.charge().unwrap() > KEPT_BYTE_LIMIT,
        "codec validity must not bypass actual admission quota"
    );
    let mut trailing = encoded;
    trailing.push(0);
    assert!(record::Record::decode(&trailing).is_err());
    let mut impossible = retried.successor().unwrap();
    impossible.remaining.requests += 1;
    assert!(retried.validate_successor(&impossible).is_err());
    let mut deleting = retried.successor().unwrap();
    deleting.phase = KeepPhase::Deleting;
    retried.validate_successor(&deleting).unwrap();
    let mut resurrected = deleting.successor().unwrap();
    resurrected.phase = KeepPhase::Resumable;
    assert!(deleting.validate_successor(&resurrected).is_err());
    deleting.exhausted = true;
    assert!(deleting
        .retry(KeepRetry {
            transfer: deleting.transfer,
            expected_grant: deleting.grant,
            request: [4; 16]
        })
        .is_err());
}

#[cfg(any(unix, windows))]
#[test]
fn cleanup_refuses_a_directory_link_without_removing_its_target_files() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    let cid = Cid::of(b"must survive");
    fs::write(target.join(cid.to_hex()), b"must survive").unwrap();
    let link = dir.path().join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    {
        // Junction creation is available to an ordinary Windows user, unlike symlink creation
        // on installations without Developer Mode. cmd creates only this fixed temp link;
        // all validation and cleanup use Rust paths and never a cross-shell delete command.
        let output = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(persistence::finish_deleting(&link).is_err());
    assert_eq!(
        fs::read(target.join(cid.to_hex())).unwrap(),
        b"must survive"
    );
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
}

#[test]
fn ordinary_blob_ciphertext_cannot_be_substituted_for_a_retained_intent() {
    let root = tempfile::tempdir().unwrap();
    let primary_dir = root.path().join("primary");
    let kept_dir = root.path().join("kept");
    let primary =
        SealingBlobStore::open(&primary_dir, [7; 32], ChaCha20Rng::seed_from_u64(12)).unwrap();
    let mut store = KeptBlobStore::open(
        Box::new(primary),
        kept_dir.clone(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    store.configure_keep_scope(scope()).unwrap();
    let run = store.begin_keep(plan()).unwrap();
    store.keep_work(run, None, 0).unwrap();
    assert_eq!(store.kept_files().transfers[0].remaining.slices, 119);
    let mut chosen = record::Record::initial(scope(), plan(), [1; 32], &mut shared_rng()).unwrap();
    chosen.transfer = store.transfers[0].record.transfer;
    let encoded = chosen.encode();
    let cached = store.put(&encoded).unwrap(); // The actual ordinary blob encryption oracle.
    fs::copy(
        primary_dir.join(cached.to_hex()),
        kept_dir
            .join(TRANSFERS)
            .join(chosen.plan.cid.to_hex())
            .join("current"),
    )
    .unwrap();
    drop(store);
    let primary =
        SealingBlobStore::open(&primary_dir, [7; 32], ChaCha20Rng::seed_from_u64(13)).unwrap();
    let mut store = KeptBlobStore::open(
        Box::new(primary),
        kept_dir,
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(14),
    );
    assert!(
        store.configure_keep_scope(scope()).is_err(),
        "a valid blob MAC is not a native journal MAC"
    );
    assert!(store.kept_files().error.is_some());
    assert!(store.kept_files().transfers.is_empty());
    assert_eq!(
        store.get(&cached).unwrap().unwrap(),
        encoded,
        "ordinary cache remains usable"
    );
}

#[test]
fn malformed_authenticated_genesis_refuses_before_current_only_barriers() {
    for changed in 0..6 {
        let dir = tempfile::tempdir().unwrap();
        let owner = transfer(dir.path());
        let mut record = owner.record.clone();
        match changed {
            0 => record.predecessor = [1; 32],
            1 => record.phase = KeepPhase::Deleting,
            2 => record.verified[0] = true,
            3 => record.remaining.requests -= 1,
            4 => record.exhausted = true,
            _ => record.revision = 0,
        }
        let path = owner.path.clone();
        drop(owner);
        fs::write(
            path.join("current"),
            encode_sealed(&seal(&[8; 32], &record.encode(), &mut shared_rng()).unwrap()),
        )
        .unwrap();
        assert!(Transfer::open_with_barriers(
            path,
            &scope(),
            [7; 32],
            [8; 32],
            shared_rng(),
            |_| panic!("genesis validation must precede flush"),
            |_| panic!("genesis validation must precede exposure")
        )
        .is_err());
    }
}

#[test]
fn user_pause_survives_restart_preserves_allowance_and_revokes_lower_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store.begin_keep(plan.clone()).unwrap();
    store.put_keep(run, b"first").unwrap();
    let work = store
        .keep_work(
            run,
            Some(KeepNetworkWork {
                bytes: 5,
                paged: true,
            }),
            1_000,
        )
        .unwrap();
    let before = store.kept_files().transfers[0].clone();
    store
        .set_keep_paused(&plan.cid, before.transfer, before.intent, true)
        .unwrap();
    assert!(!work.run_is_live());
    assert!(store.validate_keep_work(&work, 1_001).is_err());
    assert!(store.begin_keep(plan.clone()).is_err());
    assert!(store.automatic_keep_candidate(None).is_none());
    assert_eq!(store.kept_files().transfers[0].remaining, before.remaining);
    drop(store);
    let mut store = open(dir.path());
    let reopened = store.kept_files().transfers[0].clone();
    assert!(reopened.user_paused);
    assert_eq!(reopened.verified_chunks, 1);
    assert_eq!(reopened.remaining, before.remaining);
    assert!(store
        .set_keep_paused(&plan.cid, [0; 32], reopened.intent, false)
        .is_err());
    assert!(store
        .set_keep_paused(&plan.cid, reopened.transfer, reopened.intent + 1, false)
        .is_err());
    assert!(store.automatic_keep_candidate(None).is_none());
    store
        .set_keep_paused(&plan.cid, reopened.transfer, reopened.intent, false)
        .unwrap();
    assert_eq!(store.automatic_keep_candidate(None), Some(plan.cid));
    assert_eq!(store.kept_files().transfers[0].remaining, before.remaining);
    let fresh = store.begin_keep(plan).unwrap();
    assert_eq!(
        store
            .get_keep(fresh, &Cid::of(b"first"), 5)
            .unwrap()
            .unwrap(),
        b"first"
    );
    assert!(!work.run_is_live());
}

#[test]
fn failed_user_pause_cannot_leave_queued_lower_work_alive_or_refund_storage() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store.begin_keep(plan.clone()).unwrap();
    let work = store.keep_work(run, None, 1_000).unwrap();
    let before = store.kept_files();
    fs::write(
        dir.path()
            .join(TRANSFERS)
            .join(plan.cid.to_hex())
            .join("next"),
        b"blocked",
    )
    .unwrap();
    let owner = &before.transfers[0];
    assert!(store
        .set_keep_paused(&plan.cid, owner.transfer, owner.intent, true)
        .is_err());
    assert!(!work.run_is_live());
    assert!(store.automatic_keep_candidate(None).is_none());
    assert!(store.kept_files().error.is_some());
    assert_eq!(store.kept_files().allocated_bytes, before.allocated_bytes);
    drop(store);
    let mut store = unbound(dir.path());
    assert!(store.configure_keep_scope(scope()).is_err());
    assert!(store.automatic_keep_candidate(None).is_none());
}

#[test]
fn older_record_canonical_hash_is_preserved_and_pause_upgrade_reconciles_exact_successor() {
    use record::{Record, RecordEncoding};
    let dir = tempfile::tempdir().unwrap();
    let mut old = Record::initial(scope(), plan(), [8; 32], &mut shared_rng()).unwrap();
    old.encoding = RecordEncoding::V2;
    old.user_paused = true;
    old.intent = 0;
    let bytes = old.encode();
    let decoded = Record::decode(&bytes).unwrap();
    assert_eq!(
        decoded.encode(),
        bytes,
        "read cannot normalize predecessor bytes"
    );
    assert_eq!(decoded.digest(), old.digest());
    let path = dir.path().join(old.plan.cid.to_hex());
    let mut transfer =
        Transfer::create(path.clone(), old.clone(), [7; 32], [8; 32], shared_rng()).unwrap();
    let mut pause = old.successor().unwrap();
    pause.user_paused = true;
    assert_eq!(pause.encoding, RecordEncoding::V3);
    assert_eq!(pause.predecessor, old.digest());
    assert!(transfer
        .publish_with_hook(pause.clone(), &[8; 32], &mut shared_rng(), |phase, _| {
            if phase == persistence::WritePhase::FileFlushed {
                Err(injected())
            } else {
                Ok(())
            }
        })
        .is_err());
    drop(transfer);
    let recovered = Transfer::open_with_barriers(
        path,
        &scope(),
        [7; 32],
        [8; 32],
        shared_rng(),
        flush,
        persistence::directory_barrier,
    )
    .unwrap();
    assert_eq!(recovered.record, pause);
    let mut downgrade = pause.successor().unwrap();
    downgrade.encoding = RecordEncoding::V2;
    downgrade.user_paused = false;
    assert!(pause.validate_successor(&downgrade).is_err());
    let mut malformed = pause.encode();
    let pause_offset = malformed.len() - 10;
    malformed[pause_offset] = 2;
    assert!(Record::decode(&malformed).is_err());
    let mut extra = pause.encode();
    extra.push(0);
    assert!(Record::decode(&extra).is_err());
    let mut old_extra = bytes;
    old_extra.push(0);
    assert!(Record::decode(&old_extra).is_err());
}

#[test]
fn automatic_candidate_rotation_never_resurrects_completed_exhausted_or_deleted_intent() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let first = plan();
    let run = store.begin_keep(first.clone()).unwrap();
    store.pause_keep(run).unwrap();
    let mut second = first.clone();
    second.cid = Cid::of(b"another plaintext file");
    let run = store.begin_keep(second.clone()).unwrap();
    store.pause_keep(run).unwrap();
    let a = store.automatic_keep_candidate(None).unwrap();
    let b = store.automatic_keep_candidate(Some(a)).unwrap();
    assert_ne!(a, b);
    assert_eq!(store.automatic_keep_candidate(Some(b)), Some(a));
    let owner = store
        .kept_files()
        .transfers
        .into_iter()
        .find(|copy| copy.plan.cid == a)
        .unwrap();
    store
        .set_keep_paused(&a, owner.transfer, owner.intent, true)
        .unwrap();
    assert_eq!(store.automatic_keep_candidate(None), Some(b));
    let run = store
        .begin_keep(if b == first.cid { first } else { second })
        .unwrap();
    assert!(
        store.automatic_keep_candidate(None).is_none(),
        "one active native owner"
    );
    store.pause_keep(run).unwrap();
    let index = store
        .transfers
        .iter()
        .position(|transfer| transfer.record.plan.cid == b)
        .unwrap();
    let mut next = store.transfers[index].record.successor().unwrap();
    next.exhausted = true;
    store.transfers[index]
        .publish(next, &store.metadata_key, &mut store.rng)
        .unwrap();
    assert!(store.automatic_keep_candidate(None).is_none());
    let exhausted = store
        .kept_files()
        .transfers
        .into_iter()
        .find(|copy| copy.plan.cid == b)
        .unwrap();
    assert!(store
        .set_keep_paused(&b, exhausted.transfer, exhausted.intent, false)
        .is_err());
    drop(store);
    let mut store = open(dir.path());
    assert!(
        store.automatic_keep_candidate(None).is_none(),
        "restart cannot refill"
    );
    store.forget_kept(&b).unwrap();
    assert!(store.automatic_keep_candidate(None).is_none());
}

#[test]
fn retry_grant_and_exact_receipt_do_not_undo_pause_or_integrity_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let _run = store.begin_keep(plan.clone()).unwrap();
    let before = store.kept_files().transfers[0].clone();
    store
        .set_keep_paused(&plan.cid, before.transfer, before.intent, true)
        .unwrap();
    let mut exhausted = store.transfers[0].record.successor().unwrap();
    exhausted.exhausted = true;
    store.transfers[0]
        .publish(exhausted, &store.metadata_key, &mut store.rng)
        .unwrap();
    let request = KeepRetry {
        transfer: before.transfer,
        expected_grant: before.grant,
        request: [22; 16],
    };
    let receipt = store.retry_kept(&plan.cid, request).unwrap();
    let paused = store.kept_files().transfers[0].clone();
    assert!(paused.user_paused);
    assert_eq!(paused.intent, before.intent + 1);
    assert!(store.automatic_keep_candidate(None).is_none());
    store
        .set_keep_paused(&plan.cid, paused.transfer, paused.intent, false)
        .unwrap();
    let run = store.begin_keep(plan.clone()).unwrap();
    store.block_keep(run).unwrap();
    let stopped = store.kept_files().transfers[0].clone();
    assert!(stopped.blocked);
    assert!(!stopped.user_paused);
    assert!(store.automatic_keep_candidate(None).is_none());
    assert_eq!(store.retry_kept(&plan.cid, request).unwrap(), receipt);
    assert_eq!(
        store.kept_files().transfers[0],
        stopped,
        "metadata receipt cannot clear a later stop"
    );
    drop(store);
    let mut store = open(dir.path());
    assert!(store.automatic_keep_candidate(None).is_none());
    let remaining = store.kept_files().transfers[0].remaining;
    store
        .set_keep_paused(&plan.cid, stopped.transfer, stopped.intent, false)
        .unwrap();
    assert_eq!(store.kept_files().transfers[0].remaining, remaining);
    assert_eq!(store.automatic_keep_candidate(None), Some(plan.cid));
    let _ = run; // An old run remains unusable after both explicit intent changes.
}

#[test]
fn old_writer_pending_v2_debit_progress_and_deletion_recover_before_explicit_upgrade() {
    use record::{Record, RecordEncoding};
    for change in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let mut old = Record::initial(scope(), plan(), [8; 32], &mut shared_rng()).unwrap();
        old.encoding = RecordEncoding::V2;
        old.user_paused = true;
        old.intent = 0;
        let path = dir.path().join(old.plan.cid.to_hex());
        let mut owner =
            Transfer::create(path.clone(), old.clone(), [7; 32], [8; 32], shared_rng()).unwrap();
        let mut next = old.successor().unwrap();
        next.encoding = RecordEncoding::V2;
        next.intent = 0;
        match change {
            0 => {
                next.remaining.requests -= 16;
                next.remaining.slices -= 1;
            }
            1 => {
                owner.store.put(b"first").unwrap();
                next.verified[0] = true;
            }
            _ => next.phase = KeepPhase::Deleting,
        }
        assert!(owner
            .publish_with_hook(next.clone(), &[8; 32], &mut shared_rng(), |phase, _| {
                if phase == persistence::WritePhase::FileFlushed {
                    Err(injected())
                } else {
                    Ok(())
                }
            })
            .is_err());
        drop(owner);
        let mut recovered = Transfer::open_with_barriers(
            path,
            &scope(),
            [7; 32],
            [8; 32],
            shared_rng(),
            flush,
            persistence::directory_barrier,
        )
        .unwrap();
        assert_eq!(recovered.record, next);
        assert_eq!(recovered.record.encode(), next.encode());
        assert!(recovered.record.user_paused);
        let mut upgraded = next.successor().unwrap();
        if upgraded.phase != KeepPhase::Deleting {
            upgraded.user_paused = false;
        }
        recovered
            .publish(upgraded.clone(), &[8; 32], &mut shared_rng())
            .unwrap();
        assert_eq!(recovered.record.encoding, RecordEncoding::V3);
        assert_eq!(recovered.record.intent, 1);
        assert_eq!(recovered.record.remaining, next.remaining);
    }
}

#[test]
fn same_value_explicit_resume_fences_older_pause_without_changing_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let plan = plan();
    let run = store.begin_keep(plan.clone()).unwrap();
    store.pause_keep(run).unwrap();
    let original = store.kept_files().transfers[0].clone();
    store
        .set_keep_paused(&plan.cid, original.transfer, original.intent, false)
        .unwrap();
    let resumed = store.kept_files().transfers[0].clone();
    assert_eq!(resumed.intent, original.intent + 1);
    assert_eq!(resumed.remaining, original.remaining);
    assert!(!resumed.user_paused);
    assert!(store
        .set_keep_paused(&plan.cid, original.transfer, original.intent, true)
        .is_err());
    assert!(!store.kept_files().transfers[0].user_paused);
}
