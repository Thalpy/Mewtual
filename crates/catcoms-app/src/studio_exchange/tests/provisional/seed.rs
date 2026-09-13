use super::*;
use crate::studio_exchange::provisional::{
    ProvisionalStudioSeedAttempt, ProvisionalStudioSeedCompletion,
};
use catcoms_replication::{studio::StudioEpoch, CheckpointSeed};

fn candidate(p: &mut Pair, target: StudioTarget) -> (Receipt, CheckpointSeed) {
    p.alice.sync.with_registry_context(|g, d, _, _| {
        let seed = StudioEpoch::new(g, target, d.device_id())
            .unwrap()
            .projection()
            .unwrap()
            .checkpoint([7; 32])
            .unwrap();
        let receipt = Receipt::sign(
            target.document(&g.group_id()).unwrap(),
            0,
            [7; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        (receipt, seed)
    })
}
async fn seed_response(
    p: &mut Pair,
    pending: ProvisionalStudioSeedAttempt<Net>,
    seed: &CheckpointSeed,
) -> ProvisionalStudioSeedCompletion {
    let service = p
        .alice
        .sync
        .watch_checkpoint_seed(CheckpointTarget::Studio(p.watch.target))
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        loop {
            if let Some(served) = p
                .alice
                .sync
                .serve_registry_seed(&service, |_, _, _, _| {
                    Ok::<_, ()>(Some(seed.bytes().to_vec()))
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

#[tokio::test]
async fn studio_provisional_seed_inspection_never_writes_canonical_state_or_journals() {
    for art in [false, true] {
        let mut p = pages::proven_pair().await;
        let target = if art {
            target()
        } else {
            StudioTarget::Index {
                channel: target().channel(),
            }
        };
        p.watch = p
            .bob
            .watch_studio_epoch(&p.b_store, SERVER, target)
            .unwrap();
        // The existing Read contract returns an empty, unstored epoch-zero view for an absent
        // Index and None for an absent Flipnote. Compare the complete ordinary view across
        // inspection, rather than incorrectly requiring None for both document classes.
        let before = p
            .bob
            .studio_transaction(&mut p.b_store, SERVER, StudioRequest::Read { target })
            .unwrap()
            .map(|v| (v.epoch_id, v.epoch, v.phase, v.projection));
        assert_eq!(before.is_none(), art);
        if let Some((_, epoch, _, StudioProjection::Index(p))) = &before {
            assert_eq!(*epoch, 0);
            assert!(p.objects.is_empty());
        }
        assert_absent(&mut p, target);
        let (receipt, seed) = candidate(&mut p, target);
        let completed = response(&mut p, &receipt).await;
        let hint = p
            .bob
            .complete_provisional_studio_discovery(&p.b_store, SERVER, completed)
            .unwrap()
            .unwrap();
        let pending = p
            .bob
            .prepare_provisional_studio_seed(&p.b_store, SERVER, hint)
            .unwrap();
        let completed = seed_response(&mut p, pending, &seed).await;
        let preparation = p
            .bob
            .complete_provisional_studio_seed(&p.b_store, SERVER, completed)
            .unwrap()
            .unwrap();
        assert_absent(&mut p, target);
        let prepared = tokio::task::spawn_blocking(move || preparation.prepare())
            .await
            .unwrap()
            .unwrap();
        p.bob
            .with_provisional_studio_seed(&p.b_store, SERVER, &prepared, |value| {
                assert_eq!(value.candidate.receipt, &receipt);
                assert_eq!(value.projection.epoch(), 1);
                assert_eq!(value.projection.channel(), target.channel());
            })
            .unwrap();
        assert_absent(&mut p, target);
        let after = p
            .bob
            .studio_transaction(&mut p.b_store, SERVER, StudioRequest::Read { target })
            .unwrap()
            .map(|v| (v.epoch_id, v.epoch, v.phase, v.projection));
        assert_eq!(after, before);
        assert_absent(&mut p, target);
        drop(p.b_store);
        p.b_store = open(p.b_root.path());
        assert_absent(&mut p, target);
        assert!(p
            .bob
            .with_provisional_studio_seed(&p.b_store, SERVER, &prepared, |_| panic!(
                "reopened seed"
            ))
            .is_err());
    }
}

#[tokio::test]
async fn studio_provisional_seed_rechecks_mount_server_channel_watch_before_and_after_parse() {
    use automerge::transaction::Transactable;
    for stage in ["hint", "completed", "preparation", "ready"] {
        for change in ["mount", "server", "channel", "watch"] {
            let mut p = pages::proven_pair().await;
            p.alice.open_channel_index().await.unwrap();
            p.bob.open_channel_index().await.unwrap();
            let channel = p
                .alice
                .create_channel("provisional-seed-test")
                .await
                .unwrap()
                .id;
            p.bob.create_channel("provisional-seed-test").await.unwrap();
            let target = StudioTarget::Flipnote {
                channel: channel.to_be_bytes(),
                object: [7; 16],
            };
            p.watch = p
                .bob
                .watch_studio_epoch(&p.b_store, SERVER, target)
                .unwrap();
            let (receipt, seed) = candidate(&mut p, target);
            let completed = response(&mut p, &receipt).await;
            let hint = p
                .bob
                .complete_provisional_studio_discovery(&p.b_store, SERVER, completed)
                .unwrap()
                .unwrap();
            let (hint, completed, preparation, ready) = if stage == "hint" {
                (Some(hint), None, None, None)
            } else {
                let pending = p
                    .bob
                    .prepare_provisional_studio_seed(&p.b_store, SERVER, hint)
                    .unwrap();
                let completed = seed_response(&mut p, pending, &seed).await;
                if stage == "completed" {
                    (None, Some(completed), None, None)
                } else {
                    let preparation = p
                        .bob
                        .complete_provisional_studio_seed(&p.b_store, SERVER, completed)
                        .unwrap()
                        .unwrap();
                    if stage == "preparation" {
                        (None, None, Some(preparation), None)
                    } else {
                        (None, None, None, Some(preparation.prepare().unwrap()))
                    }
                }
            };
            match change {
                "mount" => {
                    drop(p.b_store);
                    p.b_store = open(p.b_root.path());
                }
                "watch" => {
                    p.bob.unwatch_studio_epoch(&p.watch).unwrap();
                    p.watch = p
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
            if let Some(hint) = hint {
                assert!(
                    p.bob
                        .prepare_provisional_studio_seed(&p.b_store, server, hint)
                        .is_err(),
                    "{stage}/{change}"
                );
            }
            if let Some(completed) = completed {
                assert!(
                    p.bob
                        .complete_provisional_studio_seed(&p.b_store, server, completed)
                        .is_err(),
                    "{stage}/{change}"
                );
            }
            let ready = preparation.map(|p| p.prepare().unwrap()).or(ready);
            if let Some(ready) = ready {
                assert!(p
                    .bob
                    .with_provisional_studio_seed(&p.b_store, server, &ready, |_| panic!(
                        "stale {stage}/{change}"
                    ))
                    .is_err());
            }
            assert_absent(&mut p, target);
        }
    }
}
