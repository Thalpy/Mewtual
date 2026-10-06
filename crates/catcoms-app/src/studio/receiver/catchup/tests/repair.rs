//! The detached repair job (design 10.3) and the install router in front of it, on a real founder
//! whose own source is faulted and, after its decision, owes a replacement.
//!
//! Review P1 on `f59eb4ed` is the router half: the owner proving exactly the selected receipt left
//! an unsealed, unfetched proof pass that the router installed seedless, failed and dropped, and
//! the next discovery repeated the cycle forever. The rest is the job: admission before any read,
//! the claim and slot held until the last owner drops, a stale rebuild refused with no write, an
//! authority change abandoning the job, and the visit-model control answers.
use super::repair_job::RepairInput;
use super::*;
use crate::store::{StudioRepairOutcome, StudioRepairRequest};
use crate::studio::{StudioRepairBlocker, StudioRepairReport, StudioRepairStart};
use catcoms_replication::studio::{IndexOp, StudioExpiry, StudioKind, StudioProjection};
use catcoms_replication::{DomainOp, EpochPhase, InheritedCheckpoint, Receipt, ReceiptRepair};
use catcoms_rt::{Clock, MemNetwork};

const SERVER: u64 = 83;

mod registry;

/// Alice founds a server that Bob joins and serves seeds for. Her own Index source adopts two
/// receipts for a later epoch without either seed, so it faults on them. With `decide`, she has
/// also persisted a decision for `chosen` at B1 and applied it at B2, so the source owes a
/// replacement that needs `chosen`'s seed.
struct Owed {
    alice: Server<MemNetwork, ChaCha20Rng>,
    _bob: Server<MemNetwork, ChaCha20Rng>,
    clock: ManualClock,
    store: ServerStore,
    root: tempfile::TempDir,
    target: StudioTarget,
    chosen: Receipt,
    pair: [Receipt; 2],
    chosen_seed: Vec<u8>,
    /// The source file as it was while faulted, before any decision.
    faulted_source: Vec<u8>,
    repair: Option<ReceiptRepair>,
}

impl Owed {
    async fn new(decide: bool) -> Self {
        let hub = Hub::new();
        let clock = ManualClock::new(1_000);
        let alice_peer = PeerId::from_u64(1);
        let mut alice = Server::found(
            hub.join(alice_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(31),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        let invite = alice.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (bob, _) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(32),
                Box::new(ManualClock::new(1_000)),
                "bob",
                alice_peer,
                &invite,
            ),
            alice.sync_once(),
        );
        let mut bob = bob.unwrap();
        // A repaired seed is requested from a proven member, bound through the same
        // authenticated directory catch-up the desktop uses.
        bob.open_channel_index().await.unwrap();
        tokio::select! {
            result = alice.request_channel_index_catchup(bob.local_peer()) => {
                result.unwrap();
            }
            _ = async { loop { bob.sync_once().await.unwrap(); } } => unreachable!(),
        }
        assert_eq!(alice.sync.studio_page_peers(), vec![bob.local_peer()]);
        let root = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(
            root.path(),
            b"owed-repair",
            &mut ChaCha20Rng::seed_from_u64(33),
        )
        .unwrap();
        let target = StudioTarget::Index {
            channel: crate::channel_id("general").to_be_bytes(),
        };
        let StudioOwnerTenure::Known(start) = alice.observed_owner_tenure() else {
            panic!("the founder observes its own tenure")
        };
        let mut b = CatchupRuntime::budget(&mut alice, &mut store, SERVER).unwrap();
        let ([chosen, rival], chosen_seed) =
            alice
                .sync
                .with_registry_context(|group, device, clock, rng| {
                    let logical = target.document(&group.group_id()).unwrap();
                    let op = DomainOp {
                        nonce: [1; 16],
                        doc_type: logical.doc_type,
                        logical_key: logical.logical_key.clone(),
                        body: IndexOp::PutObject {
                            object: [1; 16],
                            kind: StudioKind::Flipnote,
                            title: "moon".into(),
                            created_by: device.device_id(),
                            ts: 100,
                            expiry: StudioExpiry::Never,
                        }
                        .encode()
                        .unwrap(),
                    };
                    let id =
                        catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key);
                    let (_, state) = store
                        .edit_studio_epoch(SERVER, group, target, id, device, op, 100, rng, &mut b)
                        .unwrap();
                    let signed = [10u8, 11].map(|salt| {
                        let mut projection = state.projection().unwrap();
                        let StudioProjection::Index(index) = &mut projection else {
                            panic!("an Index target")
                        };
                        index.epoch = 10;
                        let seed = projection.checkpoint([salt; 32]).unwrap();
                        let receipt = Receipt::sign(
                            logical.clone(),
                            10,
                            [salt; 32],
                            seed.change_hash(),
                            start,
                            InheritedCheckpoint::EpochZero,
                            device,
                        )
                        .unwrap();
                        (receipt, seed.bytes().to_vec())
                    });
                    for (receipt, _) in &signed {
                        store
                            .adopt_studio_checkpoint(
                                SERVER, group, target, device, receipt, None, start, clock, rng,
                                &mut b,
                            )
                            .unwrap();
                    }
                    let [(chosen, chosen_seed), (rival, _)] = signed;
                    ([chosen, rival], chosen_seed)
                });
        let faulted_source = only_source(root.path());
        let mut pair = [chosen.clone(), rival];
        pair.sort_by_key(Receipt::hash);
        let mut owed = Self {
            alice,
            _bob: bob,
            clock,
            store,
            root,
            target,
            chosen,
            pair,
            chosen_seed,
            faulted_source,
            repair: None,
        };
        if decide {
            let snapshot = owed.snapshot();
            let request = owed.request();
            let mut b = CatchupRuntime::budget(&mut owed.alice, &mut owed.store, SERVER).unwrap();
            let (repair, outcome, state) = owed
                .alice
                .issue_studio_fault_repair(
                    &mut owed.store,
                    SERVER,
                    target,
                    &snapshot,
                    request,
                    None,
                    &mut b,
                )
                .unwrap();
            assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
            assert_ne!(state.phase(), EpochPhase::Open);
            owed.alice
                .sync
                .with_registry_context(|g, d, _, _| owed.store.retain_studio_source(g, d, state));
            owed.repair = Some(repair);
        }
        owed
    }

    fn request(&self) -> StudioRepairRequest {
        StudioRepairRequest {
            receipt_a: self.pair[0].hash(),
            receipt_b: self.pair[1].hash(),
            selected: self.chosen.hash(),
        }
    }

    fn snapshot(&mut self) -> ServerOwnerSnapshot {
        self.alice
            .prepare_owner_head_snapshot(&self.store, SERVER)
            .unwrap()
    }

    fn scope(&self) -> CheckpointTarget {
        CheckpointTarget::Studio(self.target)
    }

    fn source(&self) -> Vec<u8> {
        only_source(self.root.path())
    }

    fn phase(&mut self) -> EpochPhase {
        let (store, target) = (&self.store, self.target);
        self.alice
            .sync
            .with_registry_context(|g, d, _, _| store.load_studio_epoch(SERVER, g, target, d))
            .unwrap()
            .unwrap()
            .phase()
    }

    /// A runtime holding Alice's current durable owner snapshot and a private pool.
    fn runtime(&mut self, permits: usize) -> (CatchupRuntime, Arc<tokio::sync::Semaphore>) {
        let snapshot = self.snapshot();
        let mut runtime = CatchupRuntime {
            owner_snapshot: Some(snapshot),
            target: Some(self.target),
            ..Default::default()
        };
        let pool = runtime.inject_overlay_pool_for_test(permits);
        (runtime, pool)
    }

    fn replace(&self) -> RepairInput {
        RepairInput::Replace {
            repair: Box::new(self.repair.clone().expect("decided")),
            pair: Box::new(self.pair.clone()),
            seed: zeroize::Zeroizing::new(self.chosen_seed.clone()),
        }
    }
}

fn only_source(root: &std::path::Path) -> Vec<u8> {
    let mut sources = std::fs::read_dir(root.join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "studio-epoch"))
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 1);
    sources.pop().unwrap()
}

fn source_path(root: &std::path::Path) -> std::path::PathBuf {
    std::fs::read_dir(root.join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "studio-epoch"))
        .unwrap()
}

/// S2 and the completion, exactly as the actor runs them: detach, run the worker, complete.
async fn rebuild(runtime: &mut CatchupRuntime, owed: &Owed) {
    let job = runtime
        .repair_detach::<MemNetwork>()
        .expect("a captured job detaches");
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair rebuild completes as a repair result")
    };
    runtime.repair_complete(completion, owed.clock.monotonic_ms());
}

#[tokio::test]
async fn an_unfetched_pass_for_an_owed_repair_becomes_a_repaired_seed_fetch() {
    let mut owed = Owed::new(true).await;
    let repair = owed.repair.clone().unwrap();
    let source = owed.source();
    let unfetched = |owed: &mut Owed| {
        owed.alice
            .select_repaired_checkpoint(
                &owed.store,
                SERVER,
                CheckpointTarget::Studio(owed.target),
                &repair,
                &owed.chosen,
            )
            .unwrap()
    };
    // The seed extraction S1 uses refuses a pass that has not fetched its seed.
    let pass = unfetched(&mut owed);
    assert!(!pass.inner.is_fetched());
    let refused = owed
        .alice
        .repaired_seed_bytes(&owed.store, SERVER, &pass, &repair)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("no verified seed"), "got: {refused}");

    // The router never installs it or starts a job from it: it becomes a sealed repaired seed
    // fetch for the same receipt. That needs no preparation slot, so a full pool changes nothing.
    let (mut runtime, pool) = owed.runtime(4);
    let _occupied = pool.clone().try_acquire_many_owned(4).unwrap();
    runtime.checkpoint = Some(unfetched(&mut owed));
    runtime.checkpoint_sealed = false;
    let target = owed.target;
    let routed = runtime
        .route_checkpoint_install(&mut owed.alice, &mut owed.store, SERVER, Some(target))
        .unwrap();
    assert_eq!(routed, Some(None), "the router handled the pass");
    assert!(
        runtime.owner_failure.is_none(),
        "{:?}",
        runtime.owner_failure
    );
    let minted = runtime.checkpoint.as_ref().expect("a repaired seed fetch");
    assert!(runtime.checkpoint_sealed && !minted.inner.is_fetched());
    assert_eq!(minted.inner.selected_receipt(), &owed.chosen);
    assert_eq!(minted.inner.fault_repair(), Some(&repair));
    assert!(
        runtime.repair_job_target().is_none(),
        "no job without a seed"
    );
    assert_eq!(owed.source(), source, "nothing was installed");
}

#[tokio::test]
async fn a_repair_job_reserves_a_slot_before_reading_and_waits_flat_when_the_pool_is_full() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(1);
    let scope = owed.scope();
    let target = owed.target;
    let occupied = pool.clone().try_acquire_owned().unwrap();
    let started = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(started.start(), StudioRepairStart::Busy);
    assert!(runtime.repair_job_target().is_none());
    assert!(
        !runtime.repair_claimed(scope),
        "a refused job claims nothing"
    );
    // Nothing was read: a capture would have taken the warm source out of its slot.
    let store = &owed.store;
    assert!(
        owed.alice
            .sync
            .with_registry_context(|g, d, _, _| store.studio_source_is_warm(SERVER, g, target, d)),
        "a refused job read the source"
    );
    // A flat retry: the slot coming free does not admit a job before the retry time.
    drop(occupied);
    let again = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(again.start(), StudioRepairStart::Busy);
    assert_eq!(pool.available_permits(), 1);
    owed.clock.advance_ms(2_001);
    let admitted = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(admitted.start(), StudioRepairStart::Scheduled);
    assert!(runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 0, "the job owns its slot from S1");
    // One job per actor, but asking again for the same work is not a refusal.
    let same = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(same.start(), StudioRepairStart::Scheduled);
}

#[tokio::test]
async fn an_owed_replacement_runs_as_a_detached_job_and_releases_its_slot_and_claim() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    let input = owed.replace();
    let started = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        input,
    );
    assert_eq!(started.start(), StudioRepairStart::Scheduled);
    assert!(runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 3);
    // Ordinary installs and page receive for the claimed target defer to the job.
    let repair = owed.repair.clone().unwrap();
    runtime.checkpoint = Some(
        owed.alice
            .select_repaired_checkpoint(&owed.store, SERVER, scope, &repair, &owed.chosen)
            .unwrap(),
    );
    let routed = runtime
        .route_checkpoint_install(&mut owed.alice, &mut owed.store, SERVER, Some(target))
        .unwrap();
    assert_eq!(routed, Some(None));
    assert!(
        runtime.checkpoint.is_none(),
        "a claimed target installs nothing"
    );

    rebuild(&mut runtime, &owed).await;
    assert!(
        runtime.repair_parked(),
        "the rebuild waits for S3 holding its slot"
    );
    assert_eq!(pool.available_permits(), 3);
    let updated = runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(updated, Some(target));
    assert_eq!(
        owed.phase(),
        EpochPhase::Open,
        "the selected checkpoint is installed"
    );
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::Installed
        ))
    );
    let logical = target.document(&owed.alice.group_id()).unwrap();
    assert!(
        owed.store
            .load_epoch_owner_receipts(SERVER, &logical)
            .is_ok(),
        "the owner record was recycled in the same transaction"
    );
    // S4: the slot and the claim end with the job.
    assert!(!runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn a_rebuild_that_went_stale_during_s2_writes_nothing_and_backs_off() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    let input = owed.replace();
    runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        input,
    );
    rebuild(&mut runtime, &owed).await;
    // While the rebuild was detached, the source on disk became a different valid version.
    std::fs::write(source_path(owed.root.path()), &owed.faulted_source).unwrap();
    let updated = runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert_eq!(updated, None);
    assert_eq!(owed.source(), owed.faulted_source, "the job wrote nothing");
    assert!(
        runtime.repair_backoff.contains_key(&scope),
        "that target retries later"
    );
    assert!(
        matches!(runtime.repair_report(scope), Some(StudioRepairReport::Failed(ref why)) if why.contains("changed")),
        "a stale rebuild is reported, never silently lost: {:?}",
        runtime.repair_report(scope)
    );
    assert!(!runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn a_cancelled_waiter_never_releases_the_slot_or_claim_its_worker_still_owns() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    // `worker` stands for the bundle the still-running blocking closure owns.
    let worker = runtime.repair_detach::<MemNetwork>().unwrap();
    let StudioBackgroundJob::RepairRebuild(token, ..) = &worker else {
        panic!("a repair rebuild")
    };
    runtime.repair_complete(
        super::repair_job::RepairCompletion::Cancelled(*token),
        owed.clock.monotonic_ms(),
    );
    assert!(runtime.repair_job_target().is_none(), "the waiter is gone");
    assert!(
        runtime.repair_claimed(scope),
        "the worker still owns the claim"
    );
    assert_eq!(pool.available_permits(), 3, "and its slot");
    let refused = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(
        refused.start(),
        StudioRepairStart::Busy,
        "no second job on a claimed target"
    );
    // The worker ends by itself; both come back with no release call.
    drop(worker);
    assert!(!runtime.repair_claimed(scope));
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn a_new_mls_epoch_abandons_a_captured_repair_job() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert!(runtime.repair_claimed(scope));
    // Any commit moves the epoch the job captured; removing Bob is one.
    let epoch = owed.alice.epoch();
    let bob = owed._bob.my_fingerprint();
    owed.alice.remove_member(&bob).await.unwrap();
    assert_eq!(owed.alice.epoch(), epoch + 1);
    runtime.repair_check_authority(&mut owed.alice);
    assert!(runtime.repair_job_target().is_none());
    assert!(
        !runtime.repair_claimed(scope),
        "a captured job's ownership goes with it"
    );
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn an_explicit_decision_is_a_scheduled_job_whose_outcome_the_fault_view_reports() {
    let mut owed = Owed::new(false).await;
    let snapshot = owed.snapshot();
    let mut receiver = StudioReceiver::default();
    receiver.catchup.owner_snapshot = Some(snapshot);
    receiver.catchup.inject_overlay_pool_for_test(4);
    let request = owed.request();
    let target = owed.target;
    let control = |action| StudioControlRequest { target, action };
    let decide = |receiver: &mut StudioReceiver, owed: &mut Owed| {
        let (_, _, response) = receiver
            .control(
                &mut owed.alice,
                &mut owed.store,
                SERVER,
                control(StudioControlAction::RepairFault(Box::new(request))),
            )
            .unwrap();
        match response {
            Some(StudioControlResponse::RepairStarted { start, .. }) => start,
            other => panic!("expected RepairStarted, got {other:?}"),
        }
    };
    let read = |receiver: &mut StudioReceiver, owed: &mut Owed| {
        let (_, _, response) = receiver
            .control(
                &mut owed.alice,
                &mut owed.store,
                SERVER,
                control(StudioControlAction::ReadFault),
            )
            .unwrap();
        match response {
            Some(StudioControlResponse::Fault(view)) => view,
            other => panic!("expected Fault, got {other:?}"),
        }
    };
    let source = owed.source();
    assert_eq!(
        decide(&mut receiver, &mut owed),
        StudioRepairStart::Scheduled
    );
    assert_eq!(
        owed.source(),
        source,
        "scheduling decides and writes nothing"
    );
    // The same decision again is not a refusal; the running job blocks any other.
    assert_eq!(
        decide(&mut receiver, &mut owed),
        StudioRepairStart::Scheduled
    );
    let view = read(&mut receiver, &mut owed);
    assert_eq!(view.blocked_by, Some(StudioRepairBlocker::Scheduled));
    assert!(!view.may_decide);
    assert!(view.last_attempt.is_none());

    rebuild(&mut receiver.catchup, &owed).await;
    receiver
        .catchup
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    let view = read(&mut receiver, &mut owed);
    assert_eq!(
        view.last_attempt,
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AwaitingSeed
        ))
    );
    assert_eq!(view.blocked_by, Some(StudioRepairBlocker::HeldRepair));
    assert_ne!(
        owed.source(),
        source,
        "S3 issued, persisted B1 and applied B2"
    );
}

/// M18's runtime half (AG3-DES-043): where no durable claim exists, as for a peer before B2 or an
/// owner before B1, only the live job claim stops an ordinary install racing the job.
#[tokio::test]
async fn a_live_job_claim_defers_installs_where_no_durable_claim_exists() {
    let mut owed = Owed::new(false).await;
    let (mut runtime, _pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    // A pass for the target from a repair that is signed but never persisted: no owner record
    // holds it and the source carries no repair state, so nothing durable claims the target.
    let StudioOwnerTenure::Known(start) = owed.alice.observed_owner_tenure() else {
        panic!("observed tenure")
    };
    let inert = owed.alice.sync.with_registry_context(|_, d, _, _| {
        ReceiptRepair::sign_in_tenure(
            owed.chosen.document.clone(),
            owed.chosen.tenure_id,
            [owed.pair[0].hash(), owed.pair[1].hash()],
            owed.chosen.hash(),
            1,
            start,
            d,
        )
        .unwrap()
    });
    let pass = |owed: &mut Owed| {
        owed.alice
            .select_repaired_checkpoint(&owed.store, SERVER, scope, &inert, &owed.chosen)
            .unwrap()
    };
    runtime.checkpoint = Some(pass(&mut owed));
    let unclaimed = runtime
        .route_checkpoint_install(&mut owed.alice, &mut owed.store, SERVER, Some(target))
        .unwrap();
    assert_eq!(
        unclaimed, None,
        "without a claim the ordinary installer would proceed"
    );
    let request = owed.request();
    let started = runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Decide(request),
    );
    assert_eq!(started.start(), StudioRepairStart::Scheduled);
    runtime.checkpoint = Some(pass(&mut owed));
    let claimed = runtime
        .route_checkpoint_install(&mut owed.alice, &mut owed.store, SERVER, Some(target))
        .unwrap();
    assert_eq!(claimed, Some(None), "the live claim defers the install");
    assert!(runtime.checkpoint.is_none());
}

/// Review HIGH-1 on `d23452c9`: catch-up for a claimed target waits for the job, so the job must
/// never wait for that catch-up. Its S2 detaches past in-flight work and a discovery that is not
/// yet due, and its S3 commits while a checkpoint pass and a discovery plan are still pending.
#[tokio::test]
async fn a_repair_job_is_never_parked_behind_the_catch_up_it_blocks() {
    let mut owed = Owed::new(true).await;
    let snapshot = owed.snapshot();
    let mut receiver = StudioReceiver::default();
    receiver.catchup.owner_snapshot = Some(snapshot);
    receiver.catchup.inject_overlay_pool_for_test(4);
    let (scope, target) = (owed.scope(), owed.target);
    let started = receiver.catchup.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(started.start(), StudioRepairStart::Scheduled);
    // Everything that parks ordinary catch-up: a network attempt out and a discovery not due.
    receiver.catchup.in_flight = true;
    receiver.catchup.discovery_plan = Some(DiscoveryPlan {
        mount: owed.store.registry_mount(),
        server: SERVER,
        peer: PeerId::from_u64(2),
        target: scope,
        fault_report: None,
    });
    receiver.catchup.checkpoint_retry = u64::MAX;
    let job = receiver
        .detach(&mut owed.alice)
        .expect("the repair job detaches past pending catch-up");
    assert_eq!(job.kind_for_test(), "repair-rebuild");
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair result")
    };
    receiver
        .catchup
        .repair_complete(completion, owed.clock.monotonic_ms());
    // S3 too: a pending checkpoint pass and discovery plan do not hold it back.
    let repair = owed.repair.clone().unwrap();
    receiver.catchup.checkpoint = Some(
        owed.alice
            .select_repaired_checkpoint(&owed.store, SERVER, scope, &repair, &owed.chosen)
            .unwrap(),
    );
    let updated = receiver
        .catchup
        .run(&mut owed.alice, &mut owed.store, SERVER, &VecDeque::new())
        .unwrap();
    assert_eq!(updated, Some(target), "S3 committed in the first turn");
    assert!(receiver.catchup.repair_job_target().is_none());
    assert_eq!(
        receiver.catchup.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AwaitingSeed
        ))
    );
}

/// Review H1 on `54c79846`: a paused receiver never reaches the commit visit and nothing wakes it,
/// so a rebuild finishing after the pause must release its slot and claim as it arrives.
#[tokio::test]
async fn a_rebuild_arriving_after_a_pause_releases_its_slot_and_claim() {
    let mut owed = Owed::new(true).await;
    let snapshot = owed.snapshot();
    let mut receiver = StudioReceiver::default();
    receiver.catchup.owner_snapshot = Some(snapshot);
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let (scope, target) = (owed.scope(), owed.target);
    receiver.catchup.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    let job = receiver.detach(&mut owed.alice).expect("S2 detaches");
    // The production pause: it releases nothing that is detached.
    receiver.pause_at_for_test(&owed.alice);
    assert!(
        receiver.catchup.repair_claimed(scope),
        "the worker still owns it"
    );
    let result = job.run(None).await;
    receiver.complete(&mut owed.alice, result);
    assert!(!receiver.catchup.repair_claimed(scope));
    assert_eq!(
        pool.available_permits(),
        4,
        "the parked result released its slot"
    );
    assert!(matches!(
        receiver.catchup.repair_report(scope),
        Some(StudioRepairReport::Failed(ref why)) if why.contains("paused")
    ));
}

/// Review M1 on `54c79846`: once B2 has crossed and only the seed is missing, the owner's resume
/// fetches that seed instead of rerunning a job that would only flush the same source again.
#[tokio::test]
async fn an_owner_owing_only_a_seed_fetches_it_instead_of_rerunning_the_job() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let target = owed.target;
    let watch = owed
        .alice
        .watch_studio_epoch(&owed.store, SERVER, target)
        .unwrap();
    let (store, alice) = (&owed.store, &mut owed.alice);
    let doc_id = alice
        .sync
        .with_registry_context(|g, d, _, _| store.load_studio_epoch(SERVER, g, target, d))
        .unwrap()
        .unwrap()
        .doc_id();
    let watches = VecDeque::from([(watch, doc_id)]);
    let source = owed.source();
    let now = owed.clock.monotonic_ms();
    runtime
        .repair_owner(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    assert!(
        runtime.repair_job_target().is_none(),
        "no job, no rebuild, no flush"
    );
    assert_eq!(pool.available_permits(), 4);
    let minted = runtime
        .checkpoint
        .as_ref()
        .expect("the owed seed is fetched");
    assert_eq!(minted.inner.selected_receipt(), &owed.chosen);
    assert!(
        runtime.repair_next_at >= now + 60_000,
        "and the resume backs off"
    );
    assert_eq!(owed.source(), source);
}

/// Review M2 on `54c79846` and design N16: a newcomer that never observed the owner take office
/// has no authoring tenure, so an offered repair is refused before anything is reserved or read,
/// and nothing is held that could slow another repair.
#[tokio::test]
async fn a_newcomer_refuses_an_offered_repair_before_reserving_anything() {
    let mut owed = Owed::new(true).await;
    let repair = owed.repair.clone().unwrap();
    let target = owed.target;
    let scope = owed.scope();
    let mut bob_store = ServerStore::open(
        owed.root.path().join("bob").as_path(),
        b"bob",
        &mut ChaCha20Rng::seed_from_u64(34),
    )
    .unwrap();
    assert_eq!(
        owed._bob.observed_owner_tenure(),
        StudioOwnerTenure::Unknown
    );
    let mut runtime = CatchupRuntime::default();
    let pool = runtime.inject_overlay_pool_for_test(4);
    let taken = runtime.offer_repair(
        &mut owed._bob,
        &mut bob_store,
        SERVER,
        target,
        &repair,
        Some(&owed.chosen),
        true,
    );
    assert!(!taken, "the pass stays with the router");
    assert!(runtime.repair_job_target().is_none());
    assert_eq!(pool.available_permits(), 4, "no slot was reserved");
    assert!(
        runtime.repair_backoff.is_empty(),
        "nothing was captured or held"
    );
    assert!(!runtime.repair_claimed(scope));
}

/// Review M3 on `54c79846`: every way a scheduled decision ends is reported, and a new decision
/// never inherits an earlier job's report.
#[tokio::test]
async fn an_abandoned_decision_is_reported_and_a_new_one_starts_clean() {
    let mut owed = Owed::new(false).await;
    let snapshot = owed.snapshot();
    let mut receiver = StudioReceiver::default();
    receiver.catchup.owner_snapshot = Some(snapshot);
    receiver.catchup.inject_overlay_pool_for_test(4);
    let request = owed.request();
    let target = owed.target;
    let control = |action| StudioControlRequest { target, action };
    let decide = |receiver: &mut StudioReceiver, owed: &mut Owed| {
        receiver
            .control(
                &mut owed.alice,
                &mut owed.store,
                SERVER,
                control(StudioControlAction::RepairFault(Box::new(request))),
            )
            .map(|(_, _, response)| response)
    };
    let read = |receiver: &mut StudioReceiver, owed: &mut Owed| {
        let (_, _, response) = receiver
            .control(
                &mut owed.alice,
                &mut owed.store,
                SERVER,
                control(StudioControlAction::ReadFault),
            )
            .unwrap();
        match response {
            Some(StudioControlResponse::Fault(view)) => view,
            other => panic!("expected Fault, got {other:?}"),
        }
    };
    assert!(matches!(
        decide(&mut receiver, &mut owed).unwrap(),
        Some(StudioControlResponse::RepairStarted {
            start: StudioRepairStart::Scheduled,
            ..
        })
    ));
    // An MLS commit moves the epoch the job captured; the read abandons it and says so.
    let bob = owed._bob.my_fingerprint();
    owed.alice.remove_member(&bob).await.unwrap();
    let view = read(&mut receiver, &mut owed);
    // Nothing was persisted before S3, so nothing reruns: the report must say decide again.
    assert!(
        matches!(view.last_attempt, Some(StudioRepairReport::Failed(ref why))
            if why.contains("abandoned") && why.contains("decide again") && !why.contains("rerun")),
        "{:?}",
        view.last_attempt
    );
    assert_ne!(view.blocked_by, Some(StudioRepairBlocker::Scheduled));
    // A fresh decision clears that report rather than showing it as its own outcome.
    receiver.catchup.owner_snapshot = Some(owed.snapshot());
    assert!(matches!(
        decide(&mut receiver, &mut owed).unwrap(),
        Some(StudioControlResponse::RepairStarted {
            start: StudioRepairStart::Scheduled,
            ..
        })
    ));
    let view = read(&mut receiver, &mut owed);
    assert!(view.last_attempt.is_none(), "{:?}", view.last_attempt);
    assert_eq!(view.blocked_by, Some(StudioRepairBlocker::Scheduled));
}

/// Re-review MEDIUM-1(b) on `4bc753a6`: a paused receiver runs no job, so a decision made then is
/// refused honestly, reserving nothing, rather than answered `Scheduled` and released unrun.
#[tokio::test]
async fn a_decision_while_paused_is_refused_and_reserves_nothing() {
    let mut owed = Owed::new(false).await;
    let snapshot = owed.snapshot();
    let mut receiver = StudioReceiver::default();
    receiver.catchup.owner_snapshot = Some(snapshot);
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    receiver.pause_at_for_test(&owed.alice);
    let request = owed.request();
    let refused = receiver
        .control(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            StudioControlRequest {
                target: owed.target,
                action: StudioControlAction::RepairFault(Box::new(request)),
            },
        )
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(refused.contains("paused"), "{refused}");
    assert_eq!(pool.available_permits(), 4);
    assert!(!receiver.catchup.repair_claimed(owed.scope()));
}

/// Re-review MEDIUM-1(c) and the M2 per-repair hold: an offered repair this device cannot
/// assemble evidence for is reported, and only that repair is held, never the target.
#[tokio::test]
async fn an_unverifiable_offer_is_reported_and_holds_only_that_repair() {
    let mut owed = Owed::new(false).await;
    let (mut runtime, _pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    let StudioOwnerTenure::Known(start) = owed.alice.observed_owner_tenure() else {
        panic!("observed tenure")
    };
    // A repair naming one receipt this source holds and one it has never seen.
    let mut hashes = [owed.pair[0].hash(), [9; 32]];
    hashes.sort();
    let (chosen, alice) = (&owed.chosen, &mut owed.alice);
    let unknown = alice.sync.with_registry_context(|_, d, _, _| {
        ReceiptRepair::sign_in_tenure(
            chosen.document.clone(),
            chosen.tenure_id,
            hashes,
            hashes[0],
            1,
            start,
            d,
        )
        .unwrap()
    });
    let input = RepairInput::Offered {
        repair: Box::new(unknown.clone()),
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
    runtime
        .repair_commit(&mut owed.alice, &mut owed.store, SERVER)
        .unwrap();
    assert!(
        matches!(runtime.repair_report(scope), Some(StudioRepairReport::Failed(ref why)) if why.contains("cannot verify")),
        "{:?}",
        runtime.repair_report(scope)
    );
    assert!(runtime.repair_backoff.is_empty(), "the target is not held");
    assert!(runtime
        .repair_unverifiable
        .contains_key(&(scope, unknown.hash())));
}

/// Re-review LOW-6: the claim consult sites outside the router. Preparation refuses a claimed
/// target, and a checkpoint pass for it is dropped by `run` before any preparation (LOW-3).
#[tokio::test]
async fn a_claimed_target_is_neither_prepared_nor_left_holding_a_pass() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let (scope, target) = (owed.scope(), owed.target);
    runtime.start_repair(
        &mut owed.alice,
        &mut owed.store,
        SERVER,
        scope,
        Some(target),
        RepairInput::Resume,
    );
    assert_eq!(pool.available_permits(), 3);
    assert!(
        !runtime
            .prepare(&mut owed.alice, &mut owed.store, SERVER, target)
            .unwrap(),
        "a claimed target is not prepared"
    );
    assert!(
        runtime.preparation.is_none(),
        "no rival rebuild was captured"
    );
    assert_eq!(pool.available_permits(), 3, "and no second slot was taken");
    let repair = owed.repair.clone().unwrap();
    runtime.checkpoint = Some(
        owed.alice
            .select_repaired_checkpoint(&owed.store, SERVER, scope, &repair, &owed.chosen)
            .unwrap(),
    );
    runtime.checkpoint_sealed = false;
    runtime
        .run(&mut owed.alice, &mut owed.store, SERVER, &VecDeque::new())
        .unwrap();
    assert!(
        runtime.checkpoint.is_none(),
        "the claimed target's pass was dropped"
    );
}

/// Re-review M3 "cannot start": an explicit decision that cannot even be captured returns why,
/// instead of a retryable "busy" that would fail the same way again.
#[tokio::test]
async fn a_decision_that_cannot_start_says_why() {
    let mut owed = Owed::new(false).await;
    let snapshot = owed.snapshot();
    let mut receiver = StudioReceiver::default();
    receiver.catchup.owner_snapshot = Some(snapshot);
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    // A document on the same channel that this device holds no copy of.
    let absent = StudioTarget::Flipnote {
        channel: owed.target.channel(),
        object: [5; 16],
    };
    let request = owed.request();
    let refused = receiver
        .control(
            &mut owed.alice,
            &mut owed.store,
            SERVER,
            StudioControlRequest {
                target: absent,
                action: StudioControlAction::RepairFault(Box::new(request)),
            },
        )
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(refused.contains("never creates a source"), "{refused}");
    assert_eq!(pool.available_permits(), 4);
}

/// Re-review LOW-2 on `4bc753a6`: with the source cold, the owner still recognises from its
/// durable record (B3) that only the seed is missing, and fetches it without a job.
#[tokio::test]
async fn a_cold_owed_owner_fetches_the_seed_from_its_durable_record() {
    let mut owed = Owed::new(true).await;
    let (mut runtime, pool) = owed.runtime(4);
    let target = owed.target;
    // Evict the warm copy, as capturing any other work would.
    let (store, alice) = (&mut owed.store, &mut owed.alice);
    let _ = alice
        .sync
        .with_registry_context(|g, d, _, _| store.capture_studio_source(SERVER, g, target, d))
        .unwrap();
    let (store, alice) = (&owed.store, &mut owed.alice);
    assert!(!alice
        .sync
        .with_registry_context(|g, d, _, _| store.studio_source_is_warm(SERVER, g, target, d)));
    let watch = owed
        .alice
        .watch_studio_epoch(&owed.store, SERVER, target)
        .unwrap();
    let watches = VecDeque::from([(watch, 0)]);
    runtime
        .repair_owner(&mut owed.alice, &mut owed.store, SERVER, &watches)
        .unwrap();
    assert!(
        runtime.repair_job_target().is_none(),
        "no capture and rebuild"
    );
    assert_eq!(pool.available_permits(), 4);
    let minted = runtime
        .checkpoint
        .as_ref()
        .expect("the owed seed is fetched");
    assert_eq!(minted.inner.selected_receipt(), &owed.chosen);
}
