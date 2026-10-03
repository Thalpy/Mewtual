//! The Server-level repair seam, where live tenure and the durable owner snapshot enter.
use super::*;
use catcoms_mls::MlsDevice;
use catcoms_replication::studio::{IndexOp, StudioExpiry, StudioKind};
use catcoms_replication::{DomainOp, InheritedCheckpoint};
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 7;

fn budget<T: MeshTransport, R: CryptoRngCore>(
    store: &mut ServerStore,
    server: &mut Server<T, R>,
) -> EpochStudioBudget {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    server
        .sync
        .with_registry_context(|g, _, _, _| store.studio_storage_budget(SERVER, g, &inventory))
        .unwrap()
}

fn target() -> StudioTarget {
    StudioTarget::Index {
        channel: crate::channel_id("general").to_be_bytes(),
    }
}

/// A founder and a plain joiner sharing one hub. The joiner holds `Unknown` by construction.
async fn pair() -> (
    Server<catcoms_rt::MemNetwork, ChaCha20Rng>,
    Server<catcoms_rt::MemNetwork, ChaCha20Rng>,
) {
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let mut alice = Server::found(
        hub.join(alice_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1),
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
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        ),
        alice.sync_once(),
    );
    (alice, bob.unwrap())
}

/// Alice's own source, faulted on two receipts she signed in her observed tenure.
fn fault_alice<T: MeshTransport, R: CryptoRngCore>(
    alice: &mut Server<T, R>,
    store: &mut ServerStore,
) -> [Receipt; 2] {
    let StudioOwnerTenure::Known(start) = alice.observed_owner_tenure() else {
        panic!("the founder observes its own tenure")
    };
    let mut b = budget(store, alice);
    let target = target();
    alice.sync.with_registry_context(|group, device, _, rng| {
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
        let receipts = [7u8, 8].map(|close| {
            Receipt::sign(
                logical.clone(),
                state.epoch(),
                [close; 32],
                state
                    .projection()
                    .unwrap()
                    .checkpoint([close; 32])
                    .unwrap()
                    .change_hash(),
                start,
                InheritedCheckpoint::EpochZero,
                device,
            )
            .unwrap()
        });
        for receipt in &receipts {
            store
                .seal_studio_epoch(
                    SERVER,
                    group,
                    target,
                    device,
                    receipt.clone(),
                    start,
                    rng,
                    &mut b,
                )
                .unwrap();
        }
        let mut sorted = receipts;
        sorted.sort_by_key(Receipt::hash);
        sorted
    })
}

/// Agent 2's V5 acceptance at the issuance boundary, on a real joiner rather than a fabricated
/// state. Imported is covered by the seam's own `require` anchor (V7); building a genuine
/// Imported Server needs a migrated v1 snapshot, which this scope does not construct.
#[tokio::test]
async fn issuance_and_application_refuse_an_unobserved_tenure_with_that_message() {
    let (mut alice, mut bob) = pair().await;
    assert_eq!(bob.observed_owner_tenure(), StudioOwnerTenure::Unknown);
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(
        root.path(),
        b"fault-tenure",
        &mut ChaCha20Rng::seed_from_u64(3),
    )
    .unwrap();
    let evidence = fault_alice(&mut alice, &mut store);
    let snapshot = alice.prepare_owner_head_snapshot(&store, SERVER).unwrap();
    let decision = StudioRepairRequest {
        receipt_a: evidence[0].hash(),
        receipt_b: evidence[1].hash(),
        selected: evidence[0].hash(),
    };
    let source = std::fs::read_dir(root.path().join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "studio-epoch"))
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();

    let mut b = budget(&mut store, &mut bob);
    let refused = bob
        .issue_studio_fault_repair(
            &mut store,
            SERVER,
            target(),
            &snapshot,
            decision,
            None,
            &mut b,
        )
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("has not observed the current owner's tenure"),
        "issuance must refuse Unknown as Unknown; got: {refused}"
    );
    // Application is authoring too (design 6.3): it holds rather than adopting a claimed tenure.
    let repair = alice.sync.with_registry_context(|group, device, _, _| {
        ReceiptRepair::sign_in_tenure(
            target().document(&group.group_id()).unwrap(),
            evidence[0].tenure_id,
            [evidence[0].hash(), evidence[1].hash()],
            evidence[0].hash(),
            1,
            evidence[0].tenure_start_group_epoch,
            device,
        )
        .unwrap()
    });
    let mut b = budget(&mut store, &mut bob);
    let refused = bob
        .apply_studio_fault_repair(
            &mut store,
            SERVER,
            target(),
            &repair,
            &evidence,
            None,
            &mut b,
        )
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("has not observed the current owner's tenure"),
        "application must refuse Unknown as Unknown; got: {refused}"
    );
    // Nothing was written by either refusal.
    let after = std::fs::read_dir(root.path().join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "studio-epoch"))
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(after, source);
    // The joiner's view is honest about why it cannot decide.
    let view = bob.read_studio_fault(&mut store, SERVER, target()).unwrap();
    assert_eq!(view.blocked_by, Some(StudioRepairBlocker::NotOwner));
    assert!(!view.may_decide);
    assert!(
        view.candidates.is_some(),
        "a non-owner still sees the evidence"
    );
}

#[tokio::test]
async fn the_owner_decides_its_own_fault_through_the_durable_snapshot_and_exits_fault() {
    let (mut alice, _bob) = pair().await;
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(
        root.path(),
        b"fault-owner",
        &mut ChaCha20Rng::seed_from_u64(4),
    )
    .unwrap();
    let evidence = fault_alice(&mut alice, &mut store);
    let view = alice
        .read_studio_fault(&mut store, SERVER, target())
        .unwrap();
    assert_eq!(view.source.phase, EpochPhase::Fault);
    assert!(view.may_decide, "blocked by {:?}", view.blocked_by);
    let candidates = view.candidates.unwrap();
    assert_eq!(
        [candidates[0].receipt_hash, candidates[1].receipt_hash],
        [evidence[0].hash(), evidence[1].hash()]
    );
    assert!(view.repair.is_none());
    assert_eq!(view.preserved_operations, 1);

    let snapshot = alice.prepare_owner_head_snapshot(&store, SERVER).unwrap();
    let decision = StudioRepairRequest {
        receipt_a: evidence[1].hash(),
        receipt_b: evidence[0].hash(),
        selected: evidence[1].hash(),
    };
    let mut b = budget(&mut store, &mut alice);
    let (repair, outcome, state) = alice
        .issue_studio_fault_repair(
            &mut store,
            SERVER,
            target(),
            &snapshot,
            decision,
            None,
            &mut b,
        )
        .unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Repaired);
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert_eq!(repair.selected_receipt_hash, evidence[1].hash());

    let view = alice
        .read_studio_fault(&mut store, SERVER, target())
        .unwrap();
    assert_eq!(view.source.phase, EpochPhase::Closing);
    assert_eq!(view.blocked_by, Some(StudioRepairBlocker::NoFault));
    let status = view.repair.unwrap();
    assert!(!status.held, "terminal recycling released the owner record");
    assert_eq!(status.repair_hash, repair.hash());
    assert_eq!(status.disposition, Some(RepairDisposition::Transitioned));
}
