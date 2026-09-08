use super::*;
use crate::registry_head::{ServerOwnerSnapshot, ServerRegistryHeadWatch};

async fn exchange(
    p: &mut Pair,
    watch: &ServerRegistryHeadWatch,
    permit: Option<&ServerOwnerSnapshot>,
) -> catcoms_sync::receipt_head::ReceiptHeadAnswer {
    let (answer, ()) = tokio::join!(
        p.bob
            .request_registry_head(p.alice.local_peer(), p.key.bucket()),
        async {
            p.alice.sync_once().await.unwrap();
            assert!(p
                .alice
                .serve_registry_head_step(&mut p.alice_store, watch, permit, &mut p.alice_budget)
                .unwrap()
                .is_some());
        }
    );
    answer.unwrap().unwrap()
}

#[tokio::test]
async fn registry_head_network_discovers_pending_owner_selection_without_a_concrete_epoch() {
    let mut p = Pair::new().await;
    let (proof, tick) = tokio::join!(
        p.bob
            .sync
            .request_catchup(p.alice.local_peer(), DocType::Wiki, 125),
        p.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let watch = p
        .alice
        .watch_registry_head(&p.alice_store, SERVER, p.key.bucket());
    assert!(exchange(&mut p, &watch, None).await.receipt.is_none());
    let permit = p
        .alice
        .prepare_owner_head_snapshot(&p.alice_store, SERVER)
        .unwrap();
    let snapshot_before = p.alice_store.load_server(SERVER).unwrap();
    let receipt = p.alice.sync.with_registry_context(|_, device, _, _| {
        Receipt::sign(
            p.document.clone(),
            0,
            [7; 32],
            [8; 32],
            0,
            InheritedCheckpoint::EpochZero,
            device,
        )
        .unwrap()
    });
    p.alice
        .sync
        .with_registry_context(|g, _, _, r| {
            p.alice_store.prepare_epoch_owner_receipt(
                SERVER,
                receipt.clone(),
                g,
                0,
                r,
                &mut p.alice_budget,
            )
        })
        .unwrap();
    p.clock.advance_ms(1000);
    let hint = exchange(&mut p, &watch, Some(&permit)).await;
    assert_eq!(hint.receipt, Some(receipt.clone()));
    assert!(hint.proof.is_none());
    p.alice
        .sync
        .with_registry_context(|g, d, _, r| {
            p.alice_store.seal_registry_epoch(
                SERVER,
                g,
                p.key.bucket(),
                d,
                receipt.clone(),
                0,
                r,
                &mut p.alice_budget,
            )
        })
        .unwrap();
    p.clock.advance_ms(1000);
    let answer = exchange(&mut p, &watch, Some(&permit)).await;
    assert_eq!(answer.receipt, Some(receipt.clone()));
    assert!(answer.proof.is_some());
    assert!(answer.repair.is_none());
    assert_eq!(
        *p.alice_store.load_server(SERVER).unwrap(),
        *snapshot_before,
        "remote request never rewrites the whole-server snapshot"
    );
    let journal = p
        .alice_store
        .load_epoch_owner_receipts(SERVER, &p.document)
        .unwrap();
    assert_eq!(journal.pending(), Some(&receipt));
    assert!(journal.published().is_none());
    assert!(
        p.state().is_none(),
        "discovery does not silently install/prune the receiver's epoch"
    );

    // A restored runtime cannot reuse local preparation, even for identical device/group bytes.
    let snapshot = p.alice_store.load_server(SERVER).unwrap();
    p.alice = Server::restore(
        &snapshot,
        p.hub.join(PeerId::from_u64(1)),
        rng(),
        Box::new(p.clock.clone()),
        "alice",
    )
    .unwrap();
    assert!(p
        .alice
        .serve_registry_head_step(
            &mut p.alice_store,
            &watch,
            Some(&permit),
            &mut p.alice_budget
        )
        .is_err());
    let watch = p
        .alice
        .watch_registry_head(&p.alice_store, SERVER, p.key.bucket());
    p.clock.advance_ms(1000);
    assert!(exchange(&mut p, &watch, Some(&permit))
        .await
        .proof
        .is_none());
    let fresh = p
        .alice
        .prepare_owner_head_snapshot(&p.alice_store, SERVER)
        .unwrap();
    p.clock.advance_ms(1000);
    assert!(exchange(&mut p, &watch, Some(&fresh)).await.proof.is_some());
    assert!(p
        .alice
        .serve_registry_head_step(&mut p.bob_store, &watch, Some(&fresh), &mut p.bob_budget)
        .is_err());
    p.alice.unwatch_registry_head(&watch).unwrap();
}
