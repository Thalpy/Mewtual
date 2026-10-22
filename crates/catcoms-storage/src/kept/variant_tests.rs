//! Whole-candidate storage ownership regressions. Opaque manifests deliberately stay opaque
//! here: app tests additionally pin equivalence, exact decryption and whole-plaintext hashing.
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
fn rng() -> SharedRng<ChaCha20Rng> {
    SharedRng(std::sync::Arc::new(std::sync::Mutex::new(
        ChaCha20Rng::seed_from_u64(8),
    )))
}
fn open(path: &Path) -> KeptBlobStore<ChaCha20Rng> {
    let mut store = KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        path.to_path_buf(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    store.configure_keep_scope(scope()).unwrap();
    store
}
fn plans() -> Vec<KeepPlan> {
    (1..=2)
        .map(|index| KeepPlan {
            cid: Cid::of(b"whole plaintext"),
            version: [index; 32],
            manifest: vec![index; 10],
            chunks: vec![(Cid::of(&[index; 3]), 3), (Cid::of(&[index; 4]), 4)],
        })
        .collect()
}
#[test]
fn fallback_retires_old_lower_preserves_budget_and_reopens_selected_private_progress() {
    let dir = tempfile::tempdir().unwrap();
    let plans = plans();
    let cid = plans[0].cid;
    let mut store = open(dir.path());
    let old = store
        .begin_keep_variants(plans.clone(), 0, [9; 32])
        .unwrap();
    let lower = store
        .keep_work(
            old,
            Some(KeepNetworkWork {
                bytes: 3,
                paged: true,
            }),
            100,
        )
        .unwrap();
    store.put_keep(old, &[1; 3]).unwrap();
    let before = store.kept_files();
    let (new, selected) = store
        .fallback_keep(old, &plans[0].version, &[plans[0].version], true)
        .unwrap()
        .unwrap();
    assert_eq!(selected, plans[1]);
    assert!(!lower.run_is_live());
    assert!(store.validate_keep_work(&lower, 100).is_err());
    assert!(store.put_keep(old, &[1; 4]).is_err());
    assert!(store.get_keep(new, &plans[0].chunks[0].0, 10).is_err());
    assert!(!store.has(&plans[0].chunks[0].0));
    assert!(store.cids().is_empty());
    store.put_keep(new, &[2; 3]).unwrap();
    store.pause_keep(new).unwrap();
    let after = store.kept_files();
    assert_eq!(after.allocated_bytes, before.allocated_bytes);
    assert_eq!(after.transfers[0].transfer, before.transfers[0].transfer);
    assert_eq!(after.transfers[0].remaining, before.transfers[0].remaining);
    assert_eq!(after.transfers[0].grant, before.transfers[0].grant);
    assert_eq!(after.transfers[0].intent, before.transfers[0].intent);
    drop(store);
    let mut store = open(dir.path());
    let reopened = store.kept_files();
    assert_eq!(reopened, after);
    assert_eq!(reopened.transfers[0].plan, plans[1]);
    let run = store.begin_keep(plans[1].clone()).unwrap();
    assert_eq!(
        store.get_keep(run, &plans[1].chunks[0].0, 10).unwrap(),
        Some(vec![2; 3])
    );
    // V4 remembers the old bad verdict and cannot cycle back to it after restart.
    assert!(store
        .fallback_keep(run, &plans[1].version, &[plans[1].version], false)
        .unwrap()
        .is_none());
    assert_eq!(store.kept_files().transfers[0].plan.cid, cid);
}
#[test]
fn unfinished_alternative_never_becomes_public_or_replaces_completed_winner() {
    let dir = tempfile::tempdir().unwrap();
    let plans = plans();
    let mut store = open(dir.path());
    let run = store
        .begin_keep_variants(plans.clone(), 0, [9; 32])
        .unwrap();
    store.put_keep(run, &[1; 3]).unwrap();
    store.put_keep(run, &[1; 4]).unwrap();
    store.finish_keep(run).unwrap();
    let charge = store.kept_files().allocated_bytes;
    let run = store.begin_keep(plans[0].clone()).unwrap();
    let old_checked = store.kept_files().files[0].checked;
    let (next, _) = store
        .fallback_keep(run, &plans[0].version, &[plans[0].version], false)
        .unwrap()
        .unwrap();
    store.put_keep(next, &[2; 3]).unwrap();
    assert_eq!(store.kept_files().files[0].plan, plans[0]);
    assert_eq!(store.kept_files().files[0].checked, old_checked);
    assert!(store.has(&plans[0].chunks[0].0));
    assert!(!store.has(&plans[1].chunks[0].0));
    assert_eq!(store.cids().len(), 2);
    store.abort_keep(next).unwrap();
    assert_eq!(store.kept_files().files[0].plan, plans[0]);
    assert_eq!(store.kept_files().allocated_bytes, charge);
    drop(store);
    let mut store = open(dir.path());
    assert_eq!(store.kept_files().files[0].plan, plans[0]);
    assert!(!store.kept_files().files[0].checked);
    let run = store.begin_keep(plans[1].clone()).unwrap();
    // Already durable bits do not bypass per-run verification of every chunk.
    assert!(store.finish_keep(run).is_err());
    store.put_keep(run, &[2; 3]).unwrap();
    store.put_keep(run, &[2; 4]).unwrap();
    store.finish_keep(run).unwrap();
    assert_eq!(store.kept_files().files[0].plan, plans[1]);
    assert!(!store.has(&plans[0].chunks[0].0));
    assert!(store.has(&plans[1].chunks[0].0));
    assert!(!store.cids().contains(&plans[0].chunks[0].0));
    store.forget_kept(&plans[0].cid).unwrap();
    assert!(store.kept_files().files.is_empty());
    assert!(store.kept_files().transfers.is_empty());
}
#[test]
fn providerless_eligibility_requires_one_whole_nonbad_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let plans = plans();
    let cid = plans[0].cid;
    let mut store = open(dir.path());
    let run = store
        .begin_keep_variants(plans.clone(), 0, [9; 32])
        .unwrap();
    store.put_keep(run, &[1; 3]).unwrap();
    let (run, _) = store
        .fallback_keep(run, &plans[0].version, &[plans[0].version], false)
        .unwrap()
        .unwrap();
    store.put_keep(run, &[2; 4]).unwrap();
    store.pause_keep(run).unwrap();
    assert!(
        !store.keep_plausibly_local(&cid),
        "mixed positions cannot admit a whole-variant run"
    );
    store.put(&[2; 3]).unwrap();
    assert!(store.keep_plausibly_local(&cid));
    let run = store.begin_keep(plans[1].clone()).unwrap();
    let (run, _) = store
        .fallback_keep(run, &plans[1].version, &[plans[1].version], true)
        .unwrap()
        .unwrap();
    store.pause_keep(run).unwrap();
    assert!(
        !store.keep_plausibly_local(&cid),
        "a bad exact reference is not an eligible local candidate"
    );
}
#[test]
fn closed_candidate_set_quota_and_old_record_slots_are_not_reinterpreted() {
    let plans = plans();
    let mut random = rng();
    let old = record::Record::initial(scope(), plans[0].clone(), [9; 32], &mut random).unwrap();
    let old_bytes = old.encode();
    let old_charge = old.charge().unwrap();
    assert_eq!(old_charge, 3 + 40 + 4 + 40 + 2 * 80 * 1024);
    assert_eq!(
        record::Record::decode(&old_bytes).unwrap().encode(),
        old_bytes
    );
    let large: Vec<_> = (1..=4u8)
        .map(|index| KeepPlan {
            cid: plans[0].cid,
            version: [index; 32],
            manifest: vec![index; 64 * 1024],
            chunks: (0..128u16)
                .map(|chunk| {
                    let mut bytes = chunk.to_be_bytes().to_vec();
                    bytes.push(index);
                    (Cid::of(&bytes), CHUNK_LIMIT_BYTES)
                })
                .collect(),
        })
        .collect();
    let record =
        record::Record::initial_variants(scope(), large.clone(), 3, [9; 32], &mut random).unwrap();
    assert!(record.encode().len() + 40 < 360 * 1024);
    assert_eq!(record::Record::decode(&record.encode()).unwrap(), record);
    assert_eq!(
        record.charge().unwrap(),
        4 * 128 * (CHUNK_LIMIT_BYTES + 40) + 2 * 400 * 1024
    );
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    assert!(store.begin_keep_variants(large, 0, [9; 32]).is_err());
    assert!(store.kept_files().transfers.is_empty());
    let mut invalid = plans.clone();
    invalid.reverse();
    assert!(store.begin_keep_variants(invalid, 0, [9; 32]).is_err());
    let mut invalid = plans.clone();
    invalid[1].cid = Cid::of(b"another whole file");
    assert!(store.begin_keep_variants(invalid, 0, [9; 32]).is_err());
    let mut invalid = plans.clone();
    invalid.extend(plans.clone());
    invalid.push(plans[0].clone());
    assert!(store.begin_keep_variants(invalid, 0, [9; 32]).is_err());
    let run = store.begin_keep(plans[0].clone()).unwrap();
    store.pause_keep(run).unwrap();
    assert!(
        store.begin_keep_variants(plans, 0, [9; 32]).is_err(),
        "no implicit enrichment of old exact ownership"
    );
}
#[test]
fn failed_selection_publication_retires_authority_and_keeps_entire_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let plans = plans();
    let mut store = open(dir.path());
    let run = store
        .begin_keep_variants(plans.clone(), 0, [9; 32])
        .unwrap();
    let work = store.keep_work(run, None, 100).unwrap();
    let charged = store.kept_files().allocated_bytes;
    fs::write(
        dir.path()
            .join(TRANSFERS)
            .join(plans[0].cid.to_hex())
            .join("next"),
        b"incomplete publication",
    )
    .unwrap();
    assert!(store
        .fallback_keep(run, &plans[0].version, &[plans[0].version], false)
        .is_err());
    assert!(!work.run_is_live());
    assert!(!store.keep_run_current(run, &plans[0].cid, &plans[0].version));
    assert_eq!(store.kept_files().allocated_bytes, charged);
    assert!(store.kept_files().error.is_some());
    drop(store);
    let mut store = KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        dir.path().to_path_buf(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(6),
    );
    assert!(store.configure_keep_scope(scope()).is_err());
    assert!(store.kept_files().files.is_empty());
}

#[test]
fn valid_pending_v4_selection_and_commit_reconcile_once_without_resetting_budget() {
    for commit in [false, true] {
        for failure in [
            persistence::WritePhase::FileFlushed,
            persistence::WritePhase::Renamed,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let plans = plans();
            let mut original =
                record::Record::initial_variants(scope(), plans.clone(), 0, [9; 32], &mut rng())
                    .unwrap();
            original.remaining.requests -= 16;
            original.remaining.slices -= 1;
            // Genesis must precede a debit. Publish the same debit through the real successor
            // rules, then stop the following selection/commit at its actual disk boundary.
            let genesis =
                record::Record::initial_variants(scope(), plans.clone(), 0, [9; 32], &mut rng())
                    .unwrap();
            let path = dir.path().join(plans[0].cid.to_hex());
            let mut transfer =
                Transfer::create(path.clone(), genesis, [7; 32], [8; 32], rng()).unwrap();
            let mut debit = transfer.record.successor().unwrap();
            debit.remaining = original.remaining;
            transfer.publish(debit, &[8; 32], &mut rng()).unwrap();
            let next = if commit {
                for (index, bytes) in [vec![1; 3], vec![1; 4]].iter().enumerate() {
                    transfer.store.put(bytes).unwrap();
                    let mut progress = transfer.record.successor().unwrap();
                    progress.note_verified(index);
                    transfer.publish(progress, &[8; 32], &mut rng()).unwrap();
                }
                let mut next = transfer.record.successor().unwrap();
                next.commit_selected();
                next
            } else {
                transfer
                    .record
                    .fallback(&plans[0].version, &[plans[0].version], true)
                    .unwrap()
                    .0
                    .unwrap()
            };
            let allowance = transfer.record.remaining;
            assert!(transfer
                .publish_with_hook(next.clone(), &[8; 32], &mut rng(), |phase, _| {
                    if phase == failure {
                        Err(StorageError::Io("injected selection boundary".into()))
                    } else {
                        Ok(())
                    }
                })
                .is_err());
            drop(transfer);
            let recovered =
                Transfer::open(path.clone(), &scope(), [7; 32], [8; 32], rng()).unwrap();
            assert_eq!(recovered.record, next);
            assert_eq!(recovered.record.remaining, allowance);
            assert!(!recovered.checked);
            assert!(!path.join("next").exists());
            assert_eq!(recovered.record.committed_plan().is_some(), commit);
            drop(recovered);
            assert_eq!(
                Transfer::open(path, &scope(), [7; 32], [8; 32], rng())
                    .unwrap()
                    .record,
                next
            );
        }
    }
}

#[test]
fn all_512_saved_candidate_leaves_reopen_serve_only_the_winner_and_release_boundedly() {
    let dir = tempfile::tempdir().unwrap();
    let candidates: Vec<_> = (1..=4u8)
        .map(|index| KeepPlan {
            cid: Cid::of(b"one whole file"),
            version: [index; 32],
            manifest: vec![index],
            chunks: (0..128u8)
                .map(|chunk| (Cid::of(&[index, chunk]), 2))
                .collect(),
        })
        .collect();
    let mut store = open(dir.path());
    let mut run = store
        .begin_keep_variants(candidates.clone(), 0, [9; 32])
        .unwrap();
    let mut tried = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        tried.push(candidate.version);
        for chunk in 0..128u8 {
            store.put_keep(run, &[index as u8 + 1, chunk]).unwrap();
        }
        if index != 3 {
            run = store
                .fallback_keep(run, &candidate.version, &tried, false)
                .unwrap()
                .unwrap()
                .0;
        }
    }
    store.finish_keep(run).unwrap();
    let charge = store.kept_files().allocated_bytes;
    assert_eq!(
        fs::read_dir(dir.path().join(TRANSFERS).join(candidates[0].cid.to_hex()))
            .unwrap()
            .count(),
        513
    );
    assert_eq!(store.cids().len(), 128);
    drop(store);
    let mut store = open(dir.path());
    assert_eq!(store.kept_files().allocated_bytes, charge);
    assert_eq!(store.cids().len(), 128);
    assert!(store.get(&candidates[0].chunks[0].0).unwrap().is_none());
    assert_eq!(
        store.get(&candidates[3].chunks[0].0).unwrap(),
        Some(vec![4, 0])
    );
    store.forget_kept(&candidates[0].cid).unwrap();
    assert!(store.kept_files().transfers.is_empty());
    assert!(store.cids().is_empty());
    assert_eq!(fs::read_dir(dir.path().join(DELETING)).unwrap().count(), 0);
    drop(store);
    assert!(open(dir.path()).kept_files().transfers.is_empty());
}

#[test]
fn variant_exhaustion_pause_bad_verdicts_and_exact_retry_receipt_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let plans = plans();
    let cid = plans[0].cid;
    let mut store = open(dir.path());
    let run = store
        .begin_keep_variants(plans.clone(), 0, [9; 32])
        .unwrap();
    let (run, _) = store
        .fallback_keep(run, &plans[0].version, &[plans[0].version], true)
        .unwrap()
        .unwrap();
    store.put_keep(run, &[2; 3]).unwrap();
    store.pause_keep(run).unwrap();
    let intent = store.kept_files().transfers[0].clone();
    store
        .set_keep_paused(&cid, intent.transfer, intent.intent, true)
        .unwrap();
    let mut next = store.transfers[0].record.successor().unwrap();
    next.remaining = KeepAllowance {
        requests: 0,
        pages: 0,
        bytes: 0,
        slices: 0,
    };
    next.exhausted = true;
    store.transfers[0]
        .publish(next, &[8; 32], &mut rng())
        .unwrap();
    let exhausted = store.kept_files();
    drop(store);
    let mut store = open(dir.path());
    assert_eq!(store.kept_files(), exhausted);
    assert!(store.automatic_keep_candidate(None).is_none());
    assert!(store.begin_keep(plans[1].clone()).is_err());
    let request = KeepRetry {
        transfer: intent.transfer,
        expected_grant: intent.grant,
        request: [11; 16],
    };
    let receipt = store.retry_kept(&cid, request).unwrap();
    assert_eq!(receipt.granted, intent.grant + 1);
    let renewed = store.kept_files().transfers[0].clone();
    assert!(renewed.user_paused);
    assert_eq!(renewed.plan, plans[1]);
    assert_eq!(renewed.verified_chunks, 1);
    assert_eq!(renewed.remaining, record::KeepAllowance::initial(&plans[1]));
    store
        .set_keep_paused(&cid, renewed.transfer, renewed.intent, false)
        .unwrap();
    let run = store.begin_keep(plans[1].clone()).unwrap();
    store
        .keep_work(
            run,
            Some(KeepNetworkWork {
                bytes: 3,
                paged: true,
            }),
            100,
        )
        .unwrap();
    assert!(
        store
            .fallback_keep(run, &plans[1].version, &[plans[1].version], false)
            .unwrap()
            .is_none(),
        "Retry did not clear candidate A's authenticated bad verdict"
    );
    store.pause_keep(run).unwrap();
    let spent = store.kept_files();
    assert_eq!(store.retry_kept(&cid, request).unwrap(), receipt);
    assert_eq!(store.kept_files(), spent);
    drop(store);
    let mut store = open(dir.path());
    assert_eq!(store.retry_kept(&cid, request).unwrap(), receipt);
    assert_eq!(store.kept_files(), spent);
}

#[test]
fn owned_reads_expose_only_the_committed_winner_and_retire_on_promotion_release_and_scope_loss() {
    let dir = tempfile::tempdir().unwrap();
    let candidates = plans();
    let mut store = open(dir.path());
    let run = store
        .begin_keep_variants(candidates.clone(), 0, [9; 32])
        .unwrap();
    store.put_keep(run, &[1; 3]).unwrap();
    assert!(store
        .prepare_read_bounded(&candidates[0].chunks[0].0, 3)
        .unwrap()
        .run()
        .unwrap()
        .is_none());
    store.put_keep(run, &[1; 4]).unwrap();
    store.finish_keep(run).unwrap();
    let winner = store
        .prepare_read_bounded(&candidates[0].chunks[0].0, 3)
        .unwrap()
        .run()
        .unwrap()
        .unwrap();
    let waiting_winner = store
        .prepare_read_bounded(&candidates[0].chunks[0].0, 3)
        .unwrap();
    let run = store.begin_keep(candidates[0].clone()).unwrap();
    let (run, _) = store
        .fallback_keep(run, &candidates[0].version, &[candidates[0].version], false)
        .unwrap()
        .unwrap();
    store.put_keep(run, &[2; 3]).unwrap();
    assert!(
        winner.is_current(),
        "working selection/debit does not replace the committed winner"
    );
    assert!(store
        .prepare_read_bounded(&candidates[1].chunks[0].0, 3)
        .unwrap()
        .run()
        .unwrap()
        .is_none());
    store.put_keep(run, &[2; 4]).unwrap();
    store.finish_keep(run).unwrap();
    assert!(!winner.is_current());
    assert!(waiting_winner.run().unwrap().is_none());
    let winner = store
        .prepare_read_bounded(&candidates[1].chunks[0].0, 3)
        .unwrap()
        .run()
        .unwrap()
        .unwrap();
    assert!(winner.is_current());
    store.forget_kept(&candidates[0].cid).unwrap();
    assert!(!winner.is_current());
    let run = store.begin_keep(candidates[1].clone()).unwrap();
    store.put_keep(run, &[2; 3]).unwrap();
    store.put_keep(run, &[2; 4]).unwrap();
    store.finish_keep(run).unwrap();
    let winner = store
        .prepare_read_bounded(&candidates[1].chunks[0].0, 3)
        .unwrap()
        .run()
        .unwrap()
        .unwrap();
    store.disable_retention();
    assert!(!winner.is_current());
}

#[test]
fn owned_read_uses_healthy_committed_fallback_and_poisoned_publication_retires_its_result() {
    let dir = tempfile::tempdir().unwrap();
    let candidates = plans();
    let cache = dir.path().join("cache");
    let mut primary =
        SealingBlobStore::open(&cache, [5; 32], ChaCha20Rng::seed_from_u64(1)).unwrap();
    let chunk = primary.put(&[1; 3]).unwrap();
    let mut store = KeptBlobStore::open(
        Box::new(primary),
        dir.path().join("kept"),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(2),
    );
    store.configure_keep_scope(scope()).unwrap();
    let run = store
        .begin_keep_variants(candidates.clone(), 0, [9; 32])
        .unwrap();
    store.put_keep(run, &[1; 3]).unwrap();
    store.put_keep(run, &[1; 4]).unwrap();
    store.finish_keep(run).unwrap();
    fs::write(cache.join(chunk.to_hex()), b"bad primary seal").unwrap();
    let result = store
        .prepare_read_bounded(&chunk, 3)
        .unwrap()
        .run()
        .unwrap()
        .unwrap();
    assert_eq!(result.bytes(), &[1; 3]);
    assert!(result.is_current());
    let run = store.begin_keep(candidates[0].clone()).unwrap();
    fs::write(
        dir.path()
            .join("kept")
            .join(TRANSFERS)
            .join(candidates[0].cid.to_hex())
            .join("next"),
        b"blocks publication",
    )
    .unwrap();
    assert!(store.keep_work(run, None, 100).is_err());
    assert!(
        !result.is_current(),
        "a poisoned winner cannot sign a delayed public response"
    );
    assert!(
        store
            .prepare_read_bounded(&chunk, 3)
            .unwrap()
            .run()
            .is_err(),
        "corrupt primary never substitutes for poisoned retention"
    );
}
