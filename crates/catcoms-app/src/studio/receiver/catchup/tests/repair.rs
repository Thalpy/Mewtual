//! The install router for a source that owes a repair. Review P1 on `f59eb4ed`: the owner proving
//! exactly the selected receipt left an unsealed, unfetched proof pass; the router installed it
//! seedless, failed, dropped it, and the next discovery repeated the cycle forever.
use super::*;
use crate::store::{StudioRepairOutcome, StudioRepairRequest};
use catcoms_replication::studio::{IndexOp, StudioExpiry, StudioKind, StudioProjection};
use catcoms_replication::{DomainOp, EpochPhase, InheritedCheckpoint, Receipt};

const SERVER: u64 = 83;

#[tokio::test]
async fn a_warm_owed_repair_defers_without_mutation_while_the_shared_pool_is_full() {
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let mut alice = Server::found(
        hub.join(alice_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(31),
        Box::new(ManualClock::new(1_000)),
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
    // A repaired seed is requested from a proven member, bound through the same authenticated
    // directory catch-up the desktop uses.
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
        b"owed-unfetched",
        &mut ChaCha20Rng::seed_from_u64(33),
    )
    .unwrap();
    let target = StudioTarget::Index {
        channel: crate::channel_id("general").to_be_bytes(),
    };
    let StudioOwnerTenure::Known(start) = alice.observed_owner_tenure() else {
        panic!("the founder observes its own tenure")
    };

    // Alice's source adopts two receipts for a later epoch without either seed: Fault, and any
    // decision owes a replacement that needs the selected seed (AwaitingSeed).
    let mut b = CatchupRuntime::budget(&mut alice, &mut store, SERVER).unwrap();
    let [chosen, rival] = alice
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
            let id = catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key);
            let (_, state) = store
                .edit_studio_epoch(SERVER, group, target, id, device, op, 100, rng, &mut b)
                .unwrap();
            let receipts = [10u8, 11].map(|salt| {
                let mut projection = state.projection().unwrap();
                let StudioProjection::Index(index) = &mut projection else {
                    panic!("an Index target")
                };
                index.epoch = 10;
                let seed = projection.checkpoint([salt; 32]).unwrap();
                Receipt::sign(
                    logical.clone(),
                    10,
                    [salt; 32],
                    seed.change_hash(),
                    start,
                    InheritedCheckpoint::EpochZero,
                    device,
                )
                .unwrap()
            });
            for receipt in &receipts {
                store
                    .adopt_studio_checkpoint(
                        SERVER, group, target, device, receipt, None, start, clock, rng, &mut b,
                    )
                    .unwrap();
            }
            receipts
        });
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);
    let snapshot = alice.prepare_owner_head_snapshot(&store, SERVER).unwrap();
    let mut b = CatchupRuntime::budget(&mut alice, &mut store, SERVER).unwrap();
    let (repair, outcome, state) = alice
        .issue_studio_fault_repair(
            &mut store,
            SERVER,
            target,
            &snapshot,
            StudioRepairRequest {
                receipt_a: pair[0].hash(),
                receipt_b: pair[1].hash(),
                selected: chosen.hash(),
            },
            None,
            &mut b,
        )
        .unwrap();
    assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
    assert_ne!(state.phase(), EpochPhase::Open);
    alice
        .sync
        .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
    let source = std::fs::read_dir(root.path().join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "studio-epoch"))
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();

    // A pass for exactly the selected receipt, with no seed yet: the shape a kept proof pass had.
    let unfetched = |alice: &mut Server<_, _>, store: &ServerStore| {
        alice
            .select_repaired_checkpoint(
                store,
                SERVER,
                CheckpointTarget::Studio(target),
                &repair,
                &chosen,
            )
            .unwrap()
    };
    let pass = unfetched(&mut alice, &store);
    assert!(!pass.inner.is_fetched());
    // The repaired installer itself refuses it and writes nothing.
    let mut b = CatchupRuntime::budget(&mut alice, &mut store, SERVER).unwrap();
    let refused = alice
        .install_repaired_studio_seed(
            &mut store,
            SERVER,
            &pass,
            &repair,
            &pair,
            Some(&snapshot),
            &mut b,
        )
        .unwrap_err()
        .to_string();
    assert!(refused.contains("no verified seed"), "got: {refused}");

    // The router never hands it over: it is replaced by a sealed repaired seed fetch for the
    // same selected receipt, with no error surfaced and no write.
    let mut runtime = CatchupRuntime {
        owner_snapshot: Some(snapshot),
        target: Some(target),
        ..Default::default()
    };
    let pool = runtime.inject_overlay_pool_for_test(4);
    let _occupied = pool
        .clone()
        .try_acquire_many_owned(4)
        .expect("occupy every shared preparation slot");
    assert_eq!(pool.available_permits(), 0);
    runtime.checkpoint = Some(unfetched(&mut alice, &store));
    runtime.checkpoint_sealed = false;
    let routed = runtime
        .route_checkpoint_install(&mut alice, &mut store, SERVER, Some(target))
        .unwrap();
    assert_eq!(routed, Some(None), "the router deferred the pass");
    assert!(
        runtime.owner_failure.is_none(),
        "{:?}",
        runtime.owner_failure
    );
    assert!(
        runtime.checkpoint.is_none(),
        "no repair job or ordinary install may survive the fail-closed gate"
    );
    let after = std::fs::read_dir(root.path().join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "studio-epoch"))
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(after, source, "nothing was installed");
}
