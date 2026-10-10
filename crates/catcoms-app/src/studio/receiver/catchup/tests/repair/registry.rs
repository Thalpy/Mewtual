//! The same detached job for a Registry bucket (design 10.3). Every Registry repair path stayed
//! fail-closed until this job could own the bucket's capture, its detached rebuild and the recheck
//! at commit; these pin that it does, on the bucket behind Alice's Index target.
//!
//! The full-load counter is the custody boundary: S1 reads bounded bytes and S3 hands the rebuild
//! to the transaction, so neither may restore the bucket under custody.
use super::*;
use crate::store::registry_full_loads_for_test;
use crate::studio::{StudioControlResponse, StudioFaultScope};
use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp};
use catcoms_replication::registry_epoch::RegistryEpoch;

/// Alice's bucket for the Index target, faulted on two receipts she signed. With `later` they
/// are for a later epoch and adopted without seeds, so a decision owes a replacement that needs
/// `chosen`'s seed; otherwise they seal the current epoch, so a decision is terminal at once.
struct Bucket {
    /// The Studio target whose pointer lives in this bucket; decisions are made for it.
    target: StudioTarget,
    bucket: u8,
    pair: [Receipt; 2],
    chosen: Receipt,
    chosen_seed: Vec<u8>,
    /// The bucket file after the first receipt only: a different, valid version of it. Empty
    /// when the store already held another bucket's file (see `bucket_file`).
    earlier: Vec<u8>,
}

impl Bucket {
    fn new(owed: &mut Owed, later: bool) -> Self {
        let target = owed.target;
        Self::for_target(owed, target, later)
    }

    /// The same fault in the bucket behind any Studio target of Alice's.
    fn for_target(owed: &mut Owed, target: StudioTarget, later: bool) -> Self {
        let bucket = owed.alice.studio_registry_bucket(target).unwrap();
        let first = !owed.root.path().join("servers").exists()
            || std::fs::read_dir(owed.root.path().join("servers"))
                .unwrap()
                .all(|e| {
                    e.unwrap()
                        .path()
                        .extension()
                        .is_none_or(|x| x != "registry-epoch")
                });
        let StudioOwnerTenure::Known(start) = owed.alice.observed_owner_tenure() else {
            panic!("the founder observes its own tenure")
        };
        let mut b = CatchupRuntime::budget(&mut owed.alice, &mut owed.store, SERVER).unwrap();
        let root = owed.root.path().to_path_buf();
        let store = &mut owed.store;
        let (signed, earlier) =
            owed.alice
                .sync
                .with_registry_context(|group, device, clock, rng| {
                    // One signed pointer for the Index target creates the bucket. (Ordinary
                    // maintenance would write it from the Index source, which this fixture faults.)
                    let logical = target.document(&group.group_id()).unwrap();
                    let key = PointerKey::new(logical.doc_type, logical.logical_key).unwrap();
                    let op = RegistryOp::Put { key, epoch: 1 }
                        .domain_op(&group.group_id(), [1; 16])
                        .unwrap();
                    let sealed = RegistryEpoch::new(group, bucket, device.device_id())
                        .unwrap()
                        .edit(device, group, rng, &op)
                        .unwrap();
                    store
                        .with_studio_protocol_budget(SERVER, group, &mut b, |store, budget| {
                            store.ingest_registry_epoch(
                                SERVER, group, bucket, device, &sealed, rng, budget,
                            )
                        })
                        .unwrap();
                    let state = store
                        .load_registry_epoch(SERVER, group, bucket, device)
                        .unwrap()
                        .unwrap();
                    let logical = registry_document(&group.group_id(), bucket).unwrap();
                    let epoch = if later { 10 } else { 0 };
                    let signed = [7u8, 8].map(|salt| {
                        let mut projection = state.projection().unwrap();
                        projection.epoch = epoch;
                        let seed = projection.checkpoint([salt; 32]).unwrap();
                        let receipt = Receipt::sign(
                            logical.clone(),
                            epoch,
                            [salt; 32],
                            seed.change_hash(),
                            start,
                            InheritedCheckpoint::EpochZero,
                            device,
                        )
                        .unwrap();
                        (receipt, seed.bytes().to_vec())
                    });
                    let mut earlier = Vec::new();
                    store
                        .with_studio_protocol_budget(SERVER, group, &mut b, |store, budget| {
                            for (n, (receipt, _)) in signed.iter().enumerate() {
                                if later {
                                    store.adopt_registry_checkpoint(
                                        SERVER, group, bucket, device, receipt, None, start, clock,
                                        rng, budget,
                                    )?;
                                } else {
                                    store.seal_registry_epoch(
                                        SERVER,
                                        group,
                                        bucket,
                                        device,
                                        receipt.clone(),
                                        start,
                                        rng,
                                        budget,
                                    )?;
                                }
                                if n == 0 && first {
                                    earlier = bucket_file(&root);
                                }
                            }
                            Ok(())
                        })
                        .unwrap();
                    (signed, earlier)
                });
        let mut pair = [signed[0].0.clone(), signed[1].0.clone()];
        pair.sort_by_key(Receipt::hash);
        let chosen = pair[0].clone();
        let chosen_seed = signed
            .iter()
            .find(|(receipt, _)| *receipt == chosen)
            .map(|(_, seed)| seed.clone())
            .unwrap();
        let fault = Self {
            target,
            bucket,
            pair,
            chosen,
            chosen_seed,
            earlier,
        };
        assert_eq!(fault.phase(owed), EpochPhase::Fault);
        fault
    }

    fn scope(&self) -> CheckpointTarget {
        CheckpointTarget::Registry(self.bucket)
    }

    fn request(&self) -> StudioRepairRequest {
        StudioRepairRequest {
            receipt_a: self.pair[0].hash(),
            receipt_b: self.pair[1].hash(),
            selected: self.chosen.hash(),
        }
    }

    /// A full load, so never call this inside a window the counter measures.
    fn phase(&self, owed: &mut Owed) -> EpochPhase {
        let (store, bucket) = (&owed.store, self.bucket);
        owed.alice
            .sync
            .with_registry_context(|g, d, _, _| store.load_registry_epoch(SERVER, g, bucket, d))
            .unwrap()
            .unwrap()
            .phase()
    }

    fn decide(&self, runtime: &mut CatchupRuntime, owed: &mut Owed) -> StudioControlResponse {
        runtime
            .repair_registry_fault(
                &mut owed.alice,
                &mut owed.store,
                SERVER,
                self.target,
                self.request(),
            )
            .unwrap()
    }

    /// The owner's decision for this bucket, persisted and applied directly, as before a restart:
    /// with `later` receipts it now owes its replacement.
    fn decide_directly(&self, owed: &mut Owed) {
        let snapshot = owed.snapshot();
        let mut b =
            CatchupRuntime::inventory_budget(&mut owed.alice, &mut owed.store, SERVER).unwrap();
        owed.alice
            .issue_registry_fault_repair(
                &mut owed.store,
                SERVER,
                self.target,
                &snapshot,
                self.request(),
                None,
                &mut b,
            )
            .unwrap();
    }
}

fn bucket_path(root: &std::path::Path) -> std::path::PathBuf {
    std::fs::read_dir(root.join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "registry-epoch"))
        .unwrap()
}

fn bucket_file(root: &std::path::Path) -> Vec<u8> {
    std::fs::read(bucket_path(root)).unwrap()
}

#[tokio::test]
async fn a_bucket_decision_runs_as_a_detached_job_without_a_custody_restore() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, false);
    let (mut runtime, pool) = owed.runtime(4);
    let scope = fault.scope();
    let before = bucket_file(owed.root.path());
    let loads = registry_full_loads_for_test();
    let started = fault.decide(&mut runtime, &mut owed);
    assert!(
        matches!(
            started,
            StudioControlResponse::RepairStarted {
                scope: StudioFaultScope::RegistryBucket(bucket),
                start: StudioRepairStart::Scheduled,
                ..
            } if bucket == fault.bucket
        ),
        "{started:?}"
    );
    assert!(runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 3, "the job owns its slot from S1");
    assert_eq!(
        bucket_file(owed.root.path()),
        before,
        "nothing is decided before S3"
    );

    rebuild(&mut runtime, &owed).await;
    let updated = runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(updated, None);
    assert_eq!(
        registry_full_loads_for_test(),
        loads,
        "S1 and S3 never restored the bucket under custody"
    );
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(StudioRepairOutcome::Repaired))
    );
    assert!(
        runtime.repairs_seen.iter().any(|(seen, _)| *seen == scope),
        "a terminal bucket repair is remembered, so later answers cost no job"
    );
    assert_ne!(bucket_file(owed.root.path()), before);
    assert_eq!(fault.phase(&mut owed), EpochPhase::Closing);
    assert!(!runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn a_stale_bucket_rebuild_writes_nothing_and_backs_off() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, false);
    let (mut runtime, pool) = owed.runtime(4);
    let scope = fault.scope();
    fault.decide(&mut runtime, &mut owed);
    rebuild(&mut runtime, &owed).await;
    // While the rebuild was detached, the bucket on disk became a different valid version.
    std::fs::write(bucket_path(owed.root.path()), &fault.earlier).unwrap();
    let updated = runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(updated, None);
    assert_eq!(
        bucket_file(owed.root.path()),
        fault.earlier,
        "the job wrote nothing"
    );
    assert!(
        runtime.repair_backoff.contains_key(&scope),
        "that bucket retries later"
    );
    assert!(
        matches!(
            runtime.repair_report(scope),
            Some(StudioRepairReport::Failed(ref why))
                if why.contains("changed") && why.contains("decide again")
        ),
        "a stale explicit decision is reported for its person to repeat: {:?}",
        runtime.repair_report(scope)
    );
    assert!(!runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn an_owed_bucket_replacement_is_fetched_then_installed_by_a_job() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, true);
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (fault.scope(), owed.target);
    fault.decide(&mut runtime, &mut owed);
    rebuild(&mut runtime, &owed).await;
    runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AwaitingSeed
        ))
    );
    // The decision owes a replacement: its own selected seed is fetched through a repaired pass
    // made under this device's tenure, and the owner's resume waits instead of reflushing.
    let minted = runtime
        .checkpoint
        .as_ref()
        .expect("the owed seed is fetched");
    assert_eq!(minted.inner.target(), scope);
    assert_eq!(minted.inner.selected_receipt(), &fault.chosen);
    assert!(runtime.checkpoint_sealed && !minted.inner.is_fetched());
    assert!(
        runtime.repair_visits.contains_key(&scope),
        "the started fetch defers this bucket's next visit"
    );
    let repair = minted
        .inner
        .fault_repair()
        .cloned()
        .expect("minted from the repair");
    // The owner's next Registry turn resumes nothing: only the seed is missing, and it is
    // already being fetched.
    let resumed = runtime
        .resume_registry_repair(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            target,
            fault.bucket,
        )
        .unwrap();
    assert!(resumed, "the held decision owns the bucket's turn");
    assert!(runtime.repair_job_target().is_none(), "no rerun of the job");

    // The router hands a fetched seed to a job like this one; S3 installs it through the repair
    // transaction, as the owner's resume would, on the bucket rebuilt detached.
    runtime.checkpoint = None;
    let loads = registry_full_loads_for_test();
    let input = RepairInput::Replace {
        repair: Box::new(repair),
        pair: Box::new(fault.pair.clone()),
        seed: zeroize::Zeroizing::new(fault.chosen_seed.clone()),
    };
    let started = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        input,
    );
    assert_eq!(started.start(), StudioRepairStart::Scheduled);
    rebuild(&mut runtime, &owed).await;
    let updated = runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(updated, None);
    assert_eq!(registry_full_loads_for_test(), loads);
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::Installed
        ))
    );
    assert!(
        !runtime.repair_visits.contains_key(&scope),
        "a terminal bucket outcome ends the backoff"
    );
    assert_eq!(fault.phase(&mut owed), EpochPhase::Open);
    let logical = registry_document(&owed.alice.group_id(), fault.bucket).unwrap();
    assert!(
        owed.store
            .load_epoch_owner_receipts(SERVER, &logical)
            .is_ok(),
        "the owner record was recycled in the same transaction"
    );
    assert!(!runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 4);
}

/// The bucket counterpart of `a_cold_owed_owner_fetches_the_seed_from_its_durable_record`: after
/// a restart, the owner record's B3 flag alone says only the seed is missing.
#[tokio::test]
async fn an_owner_whose_bucket_owes_only_its_seed_fetches_it_without_a_job() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, true);
    // A decision persisted and applied before this runtime existed, as across a restart.
    let snapshot = owed.snapshot();
    let mut b = CatchupRuntime::inventory_budget(&mut owed.alice, &mut owed.store, SERVER).unwrap();
    let (_, outcome, _) = owed
        .alice
        .issue_registry_fault_repair(
            &mut owed.store,
            SERVER,
            owed.target,
            &snapshot,
            fault.request(),
            None,
            &mut b,
        )
        .unwrap();
    assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
    let (mut runtime, pool) = owed.runtime(4);
    let loads = registry_full_loads_for_test();
    let resumed = runtime
        .resume_registry_repair(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            owed.target,
            fault.bucket,
        )
        .unwrap();
    assert!(resumed, "a held decision owns the bucket's turn");
    assert_eq!(registry_full_loads_for_test(), loads, "no bucket restore");
    assert!(
        runtime.repair_job_target().is_none(),
        "no capture and rebuild"
    );
    assert_eq!(pool.available_permits(), 4);
    let minted = runtime
        .checkpoint
        .as_ref()
        .expect("the owed seed is fetched");
    assert_eq!(minted.inner.selected_receipt(), &fault.chosen);
}

/// PR #36 review HIGH-1, for buckets, the counterpart of
/// `an_owner_alone_after_a_crash_between_install_and_recycle_still_recycles`. After a crash
/// between the bucket's install and its record's recycle, the owner restarts alone, with the
/// provider unknown, so the record's B3 flag alone says only the seed is missing. That seed fetch
/// cannot even start with no peer, so the resume runs instead, classifies exactly at S3, finds the
/// install landed and recycles, with no network at all.
#[tokio::test]
async fn an_owner_alone_whose_bucket_seed_cannot_be_fetched_resumes_instead() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, true);
    let (mut runtime, _pool) = owed.runtime(4);
    let (scope, target) = (fault.scope(), owed.target);
    crash_after_bucket_install(&mut owed, &mut runtime, &fault).await;
    // Restarted alone: no peer, and no provider, so only B3 speaks.
    let bob = owed._bob.my_fingerprint();
    owed.alice.remove_member(&bob).await.unwrap();
    assert!(owed.alice.sync.studio_page_peers().is_empty());
    runtime.registry_provider = None;
    runtime.owner_snapshot = Some(owed.snapshot());
    let resumed = runtime
        .resume_registry_repair(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            target,
            fault.bucket,
        )
        .unwrap();
    assert!(resumed, "a held decision owns the bucket's turn");
    assert!(runtime.checkpoint.is_none(), "no peer, so no fetch");
    assert_eq!(
        runtime.repair_job_target(),
        Some(scope),
        "a guessed seed that cannot be fetched resumes"
    );
    commit(&mut runtime, &mut owed).await;
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AlreadyRepaired
        ))
    );
    let logical = registry_document(&owed.alice.group_id(), fault.bucket).unwrap();
    assert!(
        owed.store
            .load_epoch_owner_receipts(SERVER, &logical)
            .is_ok(),
        "the owner record is recycled with no peer at all"
    );
}

#[tokio::test]
async fn an_offered_bucket_repair_this_device_cannot_verify_holds_only_that_repair() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, false);
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (fault.scope(), owed.target);
    let StudioOwnerTenure::Known(start) = owed.alice.observed_owner_tenure() else {
        panic!("the founder observes its own tenure")
    };
    // A current-owner repair naming receipts this bucket has never held.
    let repair = owed
        .alice
        .sync
        .with_registry_context(|group, device, _, _| {
            let logical = registry_document(&group.group_id(), fault.bucket).unwrap();
            ReceiptRepair::sign_in_tenure(
                logical,
                [3; 32],
                [[1; 32], [2; 32]],
                [1; 32],
                1,
                start,
                device,
            )
            .unwrap()
        });
    // The owner never takes a distributed repair from an answer; it resumes its own.
    assert!(!runtime.offer_registry_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        fault.bucket,
        Some(target),
        &repair,
        None,
        false,
    ));
    assert!(runtime.repair_job_target().is_none());

    // Reaching S3 anyway, the evidence is read first: nothing this rebuild holds names it.
    let before = bucket_file(owed.root.path());
    let input = RepairInput::Offered {
        repair: Box::new(repair.clone()),
        offered: None,
    };
    let started = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        input,
    );
    assert_eq!(started.start(), StudioRepairStart::Scheduled);
    rebuild(&mut runtime, &owed).await;
    let updated = runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(updated, None);
    assert_eq!(bucket_file(owed.root.path()), before, "nothing was applied");
    assert!(
        runtime
            .repair_unverifiable
            .contains_key(&(scope, repair.hash())),
        "an unverifiable offer holds that repair"
    );
    assert!(
        !runtime.repair_backoff.contains_key(&scope),
        "only that repair is held, never the bucket"
    );
    assert!(
        matches!(
            runtime.repair_report(scope),
            Some(StudioRepairReport::Failed(ref why)) if why.contains("cannot verify")
        ),
        "{:?}",
        runtime.repair_report(scope)
    );
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn a_claimed_bucket_gets_no_registry_turn() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, false);
    let (mut runtime, _pool) = owed.runtime(4);
    fault.decide(&mut runtime, &mut owed);
    assert!(runtime.repair_claimed(fault.scope()));
    // What this pins is the turn itself: skipped and advanced before any preparation, pointer
    // refresh or owner maintenance could start. (This bucket is faulted, so its file staying
    // unchanged would prove nothing here; the claim is what stops writes to an Open one.)
    let watch = owed
        .alice
        .watch_studio_epoch(&owed.store, SERVER, owed.target)
        .unwrap();
    let watches = VecDeque::from([(watch, 0)]);
    let selection = runtime.registry_selection;
    let worked = runtime
        .work_registry(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    assert!(!worked, "the claimed bucket's turn is skipped, not spent");
    assert!(runtime.registry_target.is_none() && runtime.registry_pass.is_none());
    assert_eq!(runtime.registry_selection, selection.wrapping_add(1));
    assert!(
        runtime.registry_provider.is_none(),
        "not even a preparation was started for it"
    );
}

/// Run S2 and S3 of the job the runtime holds.
async fn commit(runtime: &mut CatchupRuntime, owed: &mut Owed) {
    rebuild(runtime, owed).await;
    runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
}

/// The owner decides the bucket, installs its owed replacement through a job, then crashes before
/// its record's recycle: the bucket keeps the install, and the record still holds the decision
/// with B3 set although the bucket owes nothing.
async fn crash_after_bucket_install(owed: &mut Owed, runtime: &mut CatchupRuntime, fault: &Bucket) {
    let (scope, target) = (fault.scope(), owed.target);
    fault.decide(runtime, owed);
    commit(runtime, owed).await;
    let repair = runtime
        .checkpoint
        .take()
        .and_then(|pass| pass.inner.fault_repair().cloned())
        .expect("the decision owes a replacement and fetches its seed");
    // The records as the crash will leave them: the decision held, B3 set.
    let held_records = owner_records(owed.root.path());
    let input = RepairInput::Replace {
        repair: Box::new(repair.clone()),
        pair: Box::new(fault.pair.clone()),
        seed: zeroize::Zeroizing::new(fault.chosen_seed.clone()),
    };
    runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        input,
    );
    commit(runtime, owed).await;
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::Installed
        ))
    );
    // The crash: the bucket kept its install, the record lost its recycle.
    for (path, bytes) in &held_records {
        std::fs::write(path, bytes).unwrap();
    }
    let (store, bucket) = (&owed.store, fault.bucket);
    assert!(
        owed.alice
            .sync
            .with_registry_context(
                |g, d, _, _| store.held_registry_repair_applied(SERVER, g, bucket, d)
            )
            .unwrap(),
        "the record still says only the seed is missing"
    );
    assert_eq!(fault.phase(owed), EpochPhase::Open);
}

/// Review HIGH-1. The owner's replacement install and its record's recycle are separate writes.
/// After a crash between them, the record still holds the decision with B3 set although the
/// bucket owes nothing. Classifying from B3 alone fetched a seed the router then deferred behind
/// that held decision forever, and nothing else recycles an owner's record. The provider's exact
/// classification resumes instead, and that resume recycles.
#[tokio::test]
async fn an_owner_that_crashed_between_install_and_recycle_resumes_and_recycles() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, true);
    let (mut runtime, _pool) = owed.runtime(4);
    let (scope, target) = (fault.scope(), owed.target);
    crash_after_bucket_install(&mut owed, &mut runtime, &fault).await;
    runtime.owner_snapshot = Some(owed.snapshot());

    // A restarted owner's Registry turn: the provider is warm and current for this bucket.
    let mut provider = owed
        .alice
        .begin_registry_page_provider(&owed.store, SERVER, fault.bucket)
        .unwrap();
    crate::registry_catchup::prepare_test_source(&mut owed.alice, &owed.store, &mut provider)
        .await
        .unwrap();
    runtime.registry_provider = Some(provider);
    let resumed = runtime
        .resume_registry_repair(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            target,
            fault.bucket,
        )
        .unwrap();
    assert!(resumed, "the held decision still owns the bucket's turn");
    assert!(
        runtime.checkpoint.is_none(),
        "no seed is fetched for a bucket that owes nothing"
    );
    assert_eq!(
        runtime.repair_job_target(),
        Some(scope),
        "the exact classification resumes the decision"
    );
    commit(&mut runtime, &mut owed).await;
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AlreadyRepaired
        ))
    );
    let logical = registry_document(&owed.alice.group_id(), fault.bucket).unwrap();
    assert!(
        owed.store
            .load_epoch_owner_receipts(SERVER, &logical)
            .is_ok(),
        "the resume recycled the owner record, so the bucket is ordinary again"
    );
}

/// Review MEDIUM-1, on the S3 failure path: an offered repair that fails holds that repair, not
/// the bucket, so the bucket's own owed seed is still fetched. (The held-outcome half needs a
/// real peer; it takes the same `hold_offer`.)
#[tokio::test]
async fn a_failed_offer_holds_only_that_repair_and_the_owed_seed_is_still_fetched() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, true);
    let (mut runtime, _pool) = owed.runtime(4);
    let (scope, target) = (fault.scope(), owed.target);
    fault.decide(&mut runtime, &mut owed);
    commit(&mut runtime, &mut owed).await;
    let repair = runtime
        .checkpoint
        .take()
        .and_then(|pass| pass.inner.fault_repair().cloned())
        .expect("the decision owes a replacement");
    // This repair, offered back to the owner: its evidence is complete here, so S3 reaches the
    // transaction, which refuses the owner (it resumes, never applies Flow A).
    let input = RepairInput::Offered {
        repair: Box::new(repair.clone()),
        offered: None,
    };
    let started = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        input,
    );
    assert_eq!(started.start(), StudioRepairStart::Scheduled);
    commit(&mut runtime, &mut owed).await;
    assert!(
        matches!(
            runtime.repair_report(scope),
            Some(StudioRepairReport::Failed(ref why)) if why.contains("durable snapshot")
        ),
        "{:?}",
        runtime.repair_report(scope)
    );
    assert!(
        runtime
            .repair_unverifiable
            .contains_key(&(scope, repair.hash())),
        "a failed offer holds that repair"
    );
    assert!(
        !runtime.repair_backoff.contains_key(&scope),
        "an offer never holds the bucket it names"
    );
    // The bucket's own replacement is not held back by it.
    runtime.await_repaired_seed(
        &mut owed.alice,
        &owed.store,
        SERVER,
        scope,
        Some(target),
        &repair,
        &fault.pair,
    );
    assert!(
        runtime.checkpoint.is_some(),
        "the owed seed is still fetched"
    );
}

/// A Registry write drops the retained provider, except while a preparation is queued or
/// running against it: that attachment would find no provider and fail the whole visit.
#[tokio::test]
async fn a_bucket_commit_never_strands_a_queued_registry_preparation() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, false);
    let (mut runtime, _pool) = owed.runtime(4);
    let mut provider = owed
        .alice
        .begin_registry_page_provider(&owed.store, SERVER, fault.bucket)
        .unwrap();
    let permit = runtime.preparation_pool().try_acquire_owned().unwrap();
    let job = owed
        .alice
        .begin_registry_page_preparation_reserved(&owed.store, &mut provider, permit)
        .unwrap()
        .expect("a saved bucket needs preparation");
    runtime.registry_provider = Some(provider);
    runtime.registry_preparation = Some((job, None));

    fault.decide(&mut runtime, &mut owed);
    rebuild(&mut runtime, &owed).await;
    runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(
        runtime.repair_report(fault.scope()),
        Some(StudioRepairReport::Completed(StudioRepairOutcome::Repaired))
    );
    assert!(
        runtime.registry_provider.is_some(),
        "a queued preparation keeps its attachment target"
    );
    // It completes against superseded bytes: refused as stale, not as a visit-wide failure.
    let (job, generation) = runtime.registry_preparation.take().unwrap();
    let StudioBackgroundResult::PreparedRegistry(generation, result) =
        StudioBackgroundJob::<MemNetwork>::PrepareRegistry(job, generation)
            .run(None)
            .await
    else {
        panic!("a Registry preparation completes as one")
    };
    runtime.registry_prepared = Some((generation, result));
    runtime
        .complete_registry_preparation(&mut owed.alice, &mut owed.store)
        .unwrap();
    assert!(
        runtime
            .registry_provider
            .as_ref()
            .is_some_and(|p| !p.has_prepared_source()),
        "the superseded graph was not attached"
    );
}

/// Re-review LOW-3: a bucket whose held decision is backing off spends its Registry turn without
/// preparing anything. Preparing it would be a detached rebuild when cold, and would evict the
/// single warm source, for a turn the decision owns anyway.
#[tokio::test]
async fn a_backing_off_held_bucket_spends_its_turn_without_preparing_anything() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, true);
    fault.decide_directly(&mut owed);
    let (mut runtime, _pool) = owed.runtime(4);
    runtime.hold_repair(fault.scope(), owed.clock.monotonic_ms());
    let watch = owed
        .alice
        .watch_studio_epoch(&owed.store, SERVER, owed.target)
        .unwrap();
    let watches = VecDeque::from([(watch, 0)]);
    let selection = runtime.registry_selection;
    let worked = runtime
        .work_registry(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    assert!(worked, "the held decision's turn is spent");
    assert!(
        runtime.preparation.is_none()
            && runtime.registry_preparation.is_none()
            && runtime.registry_provider.is_none(),
        "nothing was prepared for it"
    );
    assert!(runtime.registry_target.is_none());
    assert_eq!(
        runtime.registry_selection,
        selection.wrapping_add(1),
        "the turn moves on"
    );
}

/// Agent 1's note on the merge of PR #36: Registry maintenance skips a document whose handoff is
/// Prepared, and a record stuck on Hold is never resolved. That skip came before the bucket's
/// held owner decision was resumed, so while the stuck document was the only watched one in its
/// bucket, the decision never resumed. The resume reads only the bucket and its owner record,
/// never the document's source, so it now runs before the skip.
#[tokio::test]
async fn a_prepared_document_never_strands_its_buckets_held_decision() {
    let mut owed = Owed::new(false).await;
    let target = StudioTarget::Flipnote {
        channel: owed.target.channel(),
        object: [23; 16],
    };
    let (store, alice) = (&mut owed.store, &mut owed.alice);
    alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_interrupted_fixture(store, SERVER, g, d, target, 1, true);
    });
    let fault = Bucket::for_target(&mut owed, target, true);
    fault.decide_directly(&mut owed);
    let (mut runtime, _pool) = owed.runtime(4);
    assert!(
        CatchupRuntime::handoff_prepared(&owed.alice, &owed.store, SERVER, target),
        "precondition: the only watched document is Prepared"
    );
    let watch = owed
        .alice
        .watch_studio_epoch(&owed.store, SERVER, target)
        .unwrap();
    let watches = VecDeque::from([(watch, 0)]);
    runtime
        .work_registry(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    let minted = runtime
        .checkpoint
        .as_ref()
        .expect("the held decision resumed despite the Prepared document: its seed is fetched");
    assert_eq!(minted.inner.target(), fault.scope());
    assert!(
        runtime.registry_target.is_none(),
        "the Prepared document's turn is still skipped"
    );
}

/// PR #37 review MEDIUM-1, the two fixtures combined. The owner installed its bucket's replacement
/// and crashed before recycling the record, the provider is cold, and the bucket's only watched
/// document is Prepared, so Registry maintenance never prepares the provider. Bob is a peer but
/// never serves the seed. B3 alone says only the seed is missing, so the first turn fetches it;
/// that fetch comes to nothing. The next eligible turn must not trust the same guess again: it
/// resumes, and S3 finds the install landed and recycles, with no seed ever delivered.
#[tokio::test]
async fn a_prepared_documents_bucket_recycles_a_landed_install_without_the_seed() {
    let mut owed = Owed::new(false).await;
    let target = StudioTarget::Flipnote {
        channel: owed.target.channel(),
        object: [23; 16],
    };
    let (store, alice) = (&mut owed.store, &mut owed.alice);
    alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_interrupted_fixture(store, SERVER, g, d, target, 1, true);
    });
    let fault = Bucket::for_target(&mut owed, target, true);
    let (mut runtime, _pool) = owed.runtime(4);
    let scope = fault.scope();
    crash_after_bucket_install(&mut owed, &mut runtime, &fault).await;
    // Restarted: the provider is cold, and the only watched document is Prepared.
    runtime.registry_provider = None;
    runtime.owner_snapshot = Some(owed.snapshot());
    assert!(CatchupRuntime::handoff_prepared(
        &owed.alice,
        &owed.store,
        SERVER,
        target
    ));
    assert!(
        !owed.alice.sync.studio_page_peers().is_empty(),
        "Bob is a peer"
    );
    let watch = owed
        .alice
        .watch_studio_epoch(&owed.store, SERVER, target)
        .unwrap();
    let watches = VecDeque::from([(watch, 0)]);
    runtime
        .work_registry(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    assert!(
        runtime
            .checkpoint
            .take()
            .is_some_and(|pass| pass.inner.target() == scope),
        "the first turn trusts B3 and fetches the seed"
    );
    assert!(runtime.repair_job_target().is_none());
    // Bob never serves it. Once the bucket's deferral has run out, the next turn resumes.
    owed.clock.advance_ms(60_000);
    runtime
        .work_registry(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    assert!(
        runtime.checkpoint.is_none(),
        "no second fetch on the same guess"
    );
    assert_eq!(
        runtime.repair_job_target(),
        Some(scope),
        "a guess that came to nothing resumes"
    );
    commit(&mut runtime, &mut owed).await;
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AlreadyRepaired
        ))
    );
    let logical = registry_document(&owed.alice.group_id(), fault.bucket).unwrap();
    assert!(
        owed.store
            .load_epoch_owner_receipts(SERVER, &logical)
            .is_ok(),
        "the owner record is recycled, and no seed was ever delivered"
    );
}

/// PR #37 review LOW-1: the bucket arm of S3 memoizes the rebuilt bucket's footprint before
/// building its budget, so the budget's scan reuses it instead of validating the bucket inline,
/// which the receive scan refuses for a cold bucket over its 256 KiB cold-byte limit. This pins
/// the order on a really cold vault without building such a bucket: the commit validates this
/// bucket inline not once.
#[tokio::test]
async fn a_bucket_job_commits_without_validating_its_cold_bucket_inline() {
    let mut owed = Owed::new(false).await;
    let fault = Bucket::new(&mut owed, false);
    let (mut runtime, _pool) = owed.runtime(1);
    let scope = fault.scope();
    fault.decide(&mut runtime, &mut owed);
    rebuild(&mut runtime, &owed).await;
    // Cold for real at S3: the inventory cache forgotten, as a restart leaves it.
    owed.store.forget_warm_studio_state_for_test();
    let key =
        crate::store::registry_inventory_key_for_test(SERVER, &owed.alice.group_id(), fault.bucket);
    let inline = crate::store::inline_registry_validations_for_test(key);
    runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(StudioRepairOutcome::Repaired))
    );
    assert_eq!(
        crate::store::inline_registry_validations_for_test(key),
        inline,
        "S3 validated the cold bucket inline"
    );
}

/// Plan D, fairness, for buckets: a held bucket never delays another bucket's resume. The one
/// shared Registry resume cadence did; now only the held bucket's next visit waits.
#[tokio::test]
async fn a_held_bucket_never_delays_another_buckets_resume() {
    let mut owed = Owed::new(false).await;
    let first = Bucket::new(&mut owed, true);
    // A second document whose pointer lives in another bucket.
    let other = (1u8..=255)
        .map(|n| StudioTarget::Flipnote {
            channel: owed.target.channel(),
            object: [n; 16],
        })
        .find(|t| owed.alice.studio_registry_bucket(*t).unwrap() != first.bucket)
        .expect("another bucket");
    let second = Bucket::for_target(&mut owed, other, true);
    first.decide_directly(&mut owed);
    second.decide_directly(&mut owed);
    let (mut runtime, _pool) = owed.runtime(4);
    runtime.hold_repair(first.scope(), owed.clock.monotonic_ms());

    assert!(
        runtime
            .resume_registry_repair(
                &mut owed.alice,
                &mut owed.store,
                SERVER,
                first.target,
                first.bucket
            )
            .unwrap(),
        "the held bucket's decision owns its turn"
    );
    assert!(
        runtime.checkpoint.is_none(),
        "the held bucket fetches nothing"
    );
    // The other bucket's turn, at the same moment.
    assert!(runtime
        .resume_registry_repair(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            second.target,
            second.bucket
        )
        .unwrap());
    let minted = runtime
        .checkpoint
        .as_ref()
        .expect("the other bucket's owed seed is fetched without waiting");
    assert_eq!(minted.inner.selected_receipt(), &second.chosen);
}
