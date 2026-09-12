use super::*;
use crate::studio_exchange::provisional::ProvisionalStudioDiscoveryCompletion;
use catcoms_replication::registry::{registry_document, PointerKey};
use catcoms_replication::{InheritedCheckpoint, Receipt};
use catcoms_sync::checkpoint_exchange::CheckpointTarget;
use catcoms_sync::receipt_head::ReceiptHeadSelection;

fn receipt(p: &mut Pair, target: StudioTarget) -> Receipt {
    p.alice.sync.with_registry_context(|g, d, _, _| {
        Receipt::sign(
            target.document(&g.group_id()).unwrap(),
            0,
            [7; 32],
            [8; 32],
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap()
    })
}
async fn response(p: &mut Pair, receipt: &Receipt) -> ProvisionalStudioDiscoveryCompletion {
    let service = p
        .alice
        .sync
        .watch_checkpoint_head(CheckpointTarget::Studio(p.watch.target))
        .unwrap();
    let pending = p
        .bob
        .prepare_provisional_studio_discovery(&p.b_store, SERVER, p.alice.local_peer(), &p.watch)
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        loop {
            if let Some(served) = p
                .alice
                .sync
                .serve_receipt_head(&service, None, |_, _, _, _| {
                    Ok::<_, ()>(ReceiptHeadSelection {
                        receipt: Some(receipt.clone()),
                        prove: false,
                    })
                })
                .unwrap()
            {
                served.unwrap();
                break;
            }
            p.alice.sync_once().await.unwrap();
        }
    });
    completed
}
fn assert_absent(p: &mut Pair, target: StudioTarget) {
    let logical = target.document(&p.bob.group_id()).unwrap();
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    p.bob.sync.with_registry_context(|g, d, _, _| {
        assert!(p
            .b_store
            .load_studio_epoch(SERVER, g, target, d)
            .unwrap()
            .is_none());
        assert!(p
            .b_store
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .is_none());
    });
    for document in [
        logical,
        registry_document(&p.bob.group_id(), pointer.bucket()).unwrap(),
    ] {
        let owner = p
            .b_store
            .load_epoch_owner_receipts(SERVER, &document)
            .unwrap();
        assert!(owner.pending().is_none() && owner.published().is_none());
        assert_eq!(
            p.b_store
                .load_epoch_intents(SERVER, &document)
                .unwrap()
                .pending()
                .len(),
            0
        );
        let recovery = p.b_store.load_epoch_recovery(SERVER, &document).unwrap();
        assert_eq!(recovery.retained().len(), 0);
        assert!(recovery.staged().is_none());
        assert!(recovery.eviction_pending().unwrap().is_none());
    }
}

#[tokio::test]
async fn studio_provisional_discovery_exposes_only_scoped_candidate_metadata() {
    let mut p = pages::proven_pair().await;
    let receipt = receipt(&mut p, target());
    assert_absent(&mut p, target());
    let completed = response(&mut p, &receipt).await;
    let hint = p
        .bob
        .complete_provisional_studio_discovery(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap();
    let provider = p
        .alice
        .sync
        .with_registry_context(|_, d, _, _| d.device_id());
    p.bob
        .with_provisional_studio_hint(&p.b_store, SERVER, &hint, |value| {
            assert_eq!(value.target, target());
            assert_eq!(value.peer, p.alice.local_peer());
            assert_eq!(value.provider, provider);
            assert_eq!(value.receipt, &receipt);
        })
        .unwrap();
    assert_absent(&mut p, target());
    assert!(p
        .bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Read { target: target() }
        )
        .unwrap()
        .is_none());
    assert_absent(&mut p, target());
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    assert_absent(&mut p, target());
    assert!(p
        .bob
        .with_provisional_studio_hint(&p.b_store, SERVER, &hint, |_| panic!(
            "reopened mount callback"
        ))
        .is_err());
}

#[tokio::test]
async fn studio_provisional_discovery_rechecks_mount_server_channel_and_same_key_watch() {
    use automerge::transaction::Transactable;
    for complete_first in [false, true] {
        for change in ["mount", "server", "channel", "watch"] {
            let mut p = pages::proven_pair().await;
            p.alice.open_channel_index().await.unwrap();
            p.bob.open_channel_index().await.unwrap();
            let channel = p.alice.create_channel("provisional-test").await.unwrap().id;
            p.bob.create_channel("provisional-test").await.unwrap();
            let target = StudioTarget::Flipnote {
                channel: channel.to_be_bytes(),
                object: [7; 16],
            };
            p.watch = p
                .bob
                .watch_studio_epoch(&p.b_store, SERVER, target)
                .unwrap();
            let receipt = receipt(&mut p, target);
            assert_absent(&mut p, target);
            let completed = response(&mut p, &receipt).await;
            let (hint, completed) = if complete_first {
                let hint = p
                    .bob
                    .complete_provisional_studio_discovery(&p.b_store, SERVER, completed)
                    .unwrap()
                    .unwrap();
                p.bob
                    .with_provisional_studio_hint(&p.b_store, SERVER, &hint, |value| {
                        assert_eq!(value.receipt, &receipt)
                    })
                    .unwrap();
                (Some(hint), None)
            } else {
                (None, Some(completed))
            };
            match change {
                "mount" => {
                    drop(p.b_store);
                    p.b_store = open(p.b_root.path());
                }
                "watch" => {
                    p.bob.unwatch_studio_epoch(&p.watch).unwrap();
                    let _replacement = p
                        .bob
                        .watch_studio_epoch(&p.b_store, SERVER, target)
                        .unwrap();
                }
                "channel" => {
                    p.bob
                        .sync
                        .post(
                            catcoms_wire::DocType::ChannelIndex,
                            crate::CHANNEL_INDEX_DOC,
                            |d| d.delete(automerge::ROOT, format!("{channel:032x}")),
                        )
                        .await
                        .unwrap();
                }
                _ => {}
            }
            let server = if change == "server" {
                SERVER + 1
            } else {
                SERVER
            };
            assert!(
                p.bob
                    .prepare_provisional_studio_discovery(
                        &p.b_store,
                        server,
                        p.alice.local_peer(),
                        &p.watch
                    )
                    .is_err(),
                "{change}"
            );
            if let Some(completed) = completed {
                assert!(
                    p.bob
                        .complete_provisional_studio_discovery(&p.b_store, server, completed)
                        .is_err(),
                    "{change}"
                );
            }
            if let Some(hint) = hint {
                assert!(
                    p.bob
                        .with_provisional_studio_hint(&p.b_store, server, &hint, |_| panic!(
                            "stale app candidate callback"
                        ))
                        .is_err(),
                    "{change}"
                );
            }
            assert_absent(&mut p, target);
        }
    }
}
