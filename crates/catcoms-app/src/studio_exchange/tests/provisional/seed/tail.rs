use super::*;
use crate::studio_exchange::provisional::{
    ProvisionalStudioTailAttempt, ProvisionalStudioTailCompletion,
    ServerPreparedProvisionalStudioSeed,
};
use catcoms_replication::registry_epoch::catchup::{RegistryOpPage, RegistryPageOutcome};
use catcoms_replication::{DomainOp, SealedOp};

async fn ready(
    p: &mut Pair,
) -> (
    ServerPreparedProvisionalStudioSeed,
    SealedOp,
    StudioProjection,
) {
    let target = p.watch.target;
    let (receipt, seed) = candidate(p, target);
    let completed = response(p, &receipt).await;
    let hint = p
        .bob
        .complete_provisional_studio_discovery(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap();
    let pending = p
        .bob
        .prepare_provisional_studio_seed(&p.b_store, SERVER, hint)
        .unwrap();
    let completed = seed_response(p, pending, &seed).await;
    let seed_result = p
        .bob
        .complete_provisional_studio_seed(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap()
        .prepare()
        .unwrap();
    let (op, projection) = p.alice.sync.with_registry_context(|g, d, _, r| {
        let mut source = StudioEpoch::from_checkpoint(
            g,
            target,
            d.device_id(),
            receipt.clone(),
            0,
            seed.bytes(),
        )
        .unwrap();
        let body = match target {
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title("authenticated tail".into()))
                    .encode()
                    .unwrap()
            }
            _ => IndexOp::PutObject {
                object: [7; 16],
                kind: StudioKind::Flipnote,
                title: "authenticated tail".into(),
                created_by: d.device_id(),
                ts: 1,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
        };
        let op = source
            .edit_or_reseal(
                d,
                g,
                r,
                &DomainOp {
                    doc_type: receipt.document.doc_type,
                    logical_key: receipt.document.logical_key.clone(),
                    body,
                    nonce: [9; 16],
                },
                1,
            )
            .unwrap();
        (op, source.projection().unwrap())
    });
    (seed_result, op, projection)
}
async fn tail_response(
    p: &mut Pair,
    pending: ProvisionalStudioTailAttempt<Net>,
    op: SealedOp,
) -> ProvisionalStudioTailCompletion {
    let watch = p
        .alice
        .sync
        .watch_studio(p.watch.target, op.doc_id)
        .unwrap();
    let mut op = Some(op);
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        loop {
            if let Some(served) = p
                .alice
                .sync
                .serve_studio_request(&watch, |_, _, _, _| {
                    Ok::<_, ()>(RegistryPageOutcome::Page(RegistryOpPage {
                        operations: vec![op.take().unwrap()],
                        next: None,
                    }))
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
async fn studio_provisional_tail_remains_volatile_and_cannot_change_ordinary_reads_or_apply() {
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
        let before = p
            .bob
            .studio_transaction(&mut p.b_store, SERVER, StudioRequest::Read { target })
            .unwrap()
            .map(|v| (v.epoch_id, v.epoch, v.phase, v.projection));
        assert_absent(&mut p, target);
        let (seed, op, projection) = ready(&mut p).await;
        let candidate_epoch = op.doc_id;
        let pending = p
            .bob
            .prepare_provisional_studio_tail(&p.b_store, SERVER, seed)
            .unwrap();
        let completed = tail_response(&mut p, pending, op).await;
        let preparation = p
            .bob
            .complete_provisional_studio_tail(&p.b_store, SERVER, completed)
            .unwrap()
            .unwrap();
        assert_absent(&mut p, target);
        let prepared = tokio::task::spawn_blocking(move || preparation.prepare())
            .await
            .unwrap()
            .unwrap();
        assert!(prepared.tail_complete());
        p.bob
            .with_provisional_studio_seed(&p.b_store, SERVER, &prepared, |view| {
                assert_eq!(view.projection, &projection)
            })
            .unwrap();
        let after = p
            .bob
            .studio_transaction(&mut p.b_store, SERVER, StudioRequest::Read { target })
            .unwrap()
            .map(|v| (v.epoch_id, v.epoch, v.phase, v.projection));
        assert_eq!(after, before);
        let body = match target {
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title("not authorized by preview".into()))
                    .encode()
                    .unwrap()
            }
            _ => IndexOp::SetTitle {
                object: [7; 16],
                title: "not authorized by preview".into(),
            }
            .encode()
            .unwrap(),
        };
        let error = p
            .bob
            .studio_transaction(
                &mut p.b_store,
                SERVER,
                StudioRequest::Apply {
                    target,
                    epoch_id: candidate_epoch,
                    nonce: [88; 16],
                    body,
                },
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "epoch studio: edit belongs to a retired epoch"
        );
        assert_absent(&mut p, target);
        drop(p.b_store);
        p.b_store = open(p.b_root.path());
        assert_absent(&mut p, target);
        assert!(p
            .bob
            .with_provisional_studio_seed(&p.b_store, SERVER, &prepared, |_| panic!(
                "reopened preview"
            ))
            .is_err());
    }
}

#[tokio::test]
async fn studio_provisional_tail_rechecks_mount_server_channel_watch_across_detached_work() {
    use automerge::transaction::Transactable;
    for stage in ["seed", "completed", "preparation", "ready"] {
        for change in ["mount", "server", "channel", "watch"] {
            let mut p = pages::proven_pair().await;
            p.alice.open_channel_index().await.unwrap();
            p.bob.open_channel_index().await.unwrap();
            let channel = p
                .alice
                .create_channel("provisional-tail-test")
                .await
                .unwrap()
                .id;
            p.bob.create_channel("provisional-tail-test").await.unwrap();
            let target = StudioTarget::Flipnote {
                channel: channel.to_be_bytes(),
                object: [7; 16],
            };
            p.watch = p
                .bob
                .watch_studio_epoch(&p.b_store, SERVER, target)
                .unwrap();
            let (seed, op, _) = ready(&mut p).await;
            let (seed, completed, preparation, ready) = if stage == "seed" {
                (Some(seed), None, None, None)
            } else {
                let pending = p
                    .bob
                    .prepare_provisional_studio_tail(&p.b_store, SERVER, seed)
                    .unwrap();
                let completed = tail_response(&mut p, pending, op).await;
                if stage == "completed" {
                    (None, Some(completed), None, None)
                } else {
                    let prep = p
                        .bob
                        .complete_provisional_studio_tail(&p.b_store, SERVER, completed)
                        .unwrap()
                        .unwrap();
                    if stage == "preparation" {
                        (None, None, Some(prep), None)
                    } else {
                        (None, None, None, Some(prep.prepare().unwrap()))
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
            if let Some(seed) = seed {
                assert!(
                    p.bob
                        .prepare_provisional_studio_tail(&p.b_store, server, seed)
                        .is_err(),
                    "{stage}/{change}"
                );
            }
            if let Some(completed) = completed {
                assert!(
                    p.bob
                        .complete_provisional_studio_tail(&p.b_store, server, completed)
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
