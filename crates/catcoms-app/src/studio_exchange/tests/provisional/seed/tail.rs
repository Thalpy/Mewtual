mod runtime;
use super::*;
use crate::studio::{StudioOverlaySaveVisit, StudioReceiver};
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

async fn complete_preview(p: &mut Pair) -> ServerPreparedProvisionalStudioSeed {
    let (seed, tail, _) = ready(p).await;
    let pending = p
        .bob
        .prepare_provisional_studio_tail(&p.b_store, SERVER, seed)
        .unwrap();
    let completed = tail_response(p, pending, tail).await;
    let preparation = p
        .bob
        .complete_provisional_studio_tail(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap();
    let preview = tokio::task::spawn_blocking(move || preparation.prepare())
        .await
        .unwrap()
        .unwrap();
    assert!(preview.tail_complete());
    preview
}

/// The first app-side consumer of an archived awaiting-tenure preview. The fixture obtains a real
/// current seed and complete authenticated tail, then runs all three Flow S stages. The source
/// remains absent and the durable branch records Unconfirmed provenance; neither fact is inferred
/// from a fabricated basis.
#[tokio::test]
async fn complete_preview_can_save_an_unconfirmed_draft_only_through_detached_flow_s() {
    let mut p = pages::proven_pair().await;
    let target = p.watch.target;
    let preview = complete_preview(&mut p).await;

    let mut budget = budget(&mut p.bob, &mut p.b_store);
    let wrong_target = StudioTarget::Index { channel: channel() };
    assert!(p
        .bob
        .prepare_studio_unconfirmed_overlay(
            &mut p.b_store,
            SERVER,
            wrong_target,
            &preview,
            &mut budget,
        )
        .is_err());
    let ticket = p
        .bob
        .prepare_studio_unconfirmed_overlay(&mut p.b_store, SERVER, target, &preview, &mut budget)
        .unwrap();
    let operation = title(10, "local while awaiting tenure");
    let mut receiver = StudioReceiver::default();
    assert!(matches!(
        receiver
            .save_unconfirmed_overlay(
                &mut p.bob,
                &mut p.b_store,
                SERVER,
                target,
                &preview,
                ticket.basis,
                ticket.branch,
                operation.clone(),
                &mut budget,
            )
            .unwrap(),
        StudioOverlaySaveVisit::Scheduled
    ));

    let work = receiver
        .detach(&mut p.bob)
        .expect("the first append must be detached");
    assert_eq!(work.kind_for_test(), "overlay-plan");
    let result = work.run(None).await;
    receiver.complete(&mut p.bob, result);

    let saved = receiver
        .save_unconfirmed_overlay(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            target,
            &preview,
            ticket.basis,
            ticket.branch,
            operation.clone(),
            &mut budget,
        )
        .unwrap();
    let StudioOverlaySaveVisit::Saved(saved) = saved else {
        panic!("the completed detached plan was not committed")
    };
    let catcoms_replication::studio::StudioOverlaySave::Local(draft) = *saved else {
        panic!("new Unconfirmed authoring returned a terminal acknowledgement")
    };
    assert_eq!(draft.basis(), ticket.basis);
    assert_eq!(draft.accepted(), 1);

    // V8: once durable, the exact request is an acknowledgement path. Expiring the preview must
    // not turn it into new authoring or make it require a fresh basis.
    p.clock.advance_ms(60_000);
    assert!(!preview.unconfirmed_is_unexpired());
    let retry = receiver
        .save_unconfirmed_overlay(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            target,
            &preview,
            ticket.basis,
            ticket.branch,
            operation,
            &mut budget,
        )
        .unwrap();
    assert!(matches!(retry, StudioOverlaySaveVisit::Saved(_)));
    assert!(receiver
        .save_unconfirmed_overlay(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            target,
            &preview,
            ticket.basis,
            ticket.branch,
            title(11, "must not save from an expired preview"),
            &mut budget,
        )
        .is_err());

    let (group, author) = p
        .bob
        .sync
        .with_registry_context(|group, device, _, _| (group.group_id(), device.device_id()));
    let capture = p
        .b_store
        .capture_studio_inspection(SERVER, &group, target, author)
        .unwrap();
    let (_, inspected) = capture.rebuild().unwrap();
    assert!(matches!(
        inspected.provenance,
        Some(catcoms_replication::studio::StudioOverlayProvenance::Unconfirmed { .. })
    ));
    assert!(p
        .bob
        .sync
        .with_registry_context(|group, device, _, _| {
            p.b_store
                .capture_studio_source(SERVER, group, target, device)
        })
        .unwrap()
        .is_none());

    receiver
        .observe_for_test(
            &mut p.bob,
            &p.b_store,
            SERVER,
            target,
            preview.unconfirmed_doc_id(),
        )
        .unwrap();
    receiver.handoff_probe_for_test(&mut p.bob, &mut p.b_store, SERVER);
    assert!(
        receiver.handoff_is_quiet_for_test(&p.b_store, target),
        "automatic handoff must memoize Unconfirmed history as quiet"
    );

    // A fresh complete inventory, rather than the in-memory writer counters, must recover both
    // rails from authenticated durable facts. This is the restart/reconciliation authority used
    // by subsequent Save visits.
    let mut refreshed = crate::studio_exchange::tests::budget(&mut p.bob, &mut p.b_store);
    let (branches, bytes) = refreshed.unconfirmed_usage_for_test();
    assert_eq!(branches, 1);
    assert!(bytes > 0);

    // Disposal is the only supported transition that removes a live Unconfirmed branch. It must
    // release the in-memory rails only after the terminal replacement lands, and a fresh scan must
    // derive the same zero usage from the retained terminal record.
    let request = crate::store::StudioOverlayDisposalRequest {
        branch: inspected.branch.unwrap(),
        content: inspected.content.unwrap(),
        accepted: inspected.draft.as_ref().unwrap().accepted(),
        mode: crate::store::StudioDisposalRequestMode::Discard(
            catcoms_replication::studio::StudioDiscardConfirmation::parse(
                catcoms_replication::studio::StudioDiscardConfirmation::TOKEN,
            )
            .unwrap(),
        ),
    };
    p.bob
        .sync
        .with_registry_context(|group, device, clock, rng| {
            let document = target.document(&group.group_id()).unwrap();
            p.b_store.dispose_studio_overlay(
                SERVER,
                &document,
                target,
                group,
                device,
                request,
                clock.now_ms(),
                rng,
                &mut refreshed,
            )
        })
        .unwrap();
    assert_eq!(refreshed.unconfirmed_usage_for_test(), (0, 0));
    let rescanned = crate::studio_exchange::tests::budget(&mut p.bob, &mut p.b_store);
    assert_eq!(rescanned.unconfirmed_usage_for_test(), (0, 0));
}

/// The app rail is intentionally narrower than replication's general overlay bound. Pin the
/// boundary through the real preview and detached scheduler so a future caller cannot bypass the
/// 64-operation policy by invoking another stage directly.
#[tokio::test]
async fn unconfirmed_preview_refuses_a_sixty_fifth_operation_before_scheduling() {
    let mut p = pages::proven_pair().await;
    let target = p.watch.target;
    let preview = complete_preview(&mut p).await;
    let mut budget = budget(&mut p.bob, &mut p.b_store);
    let ticket = p
        .bob
        .prepare_studio_unconfirmed_overlay(&mut p.b_store, SERVER, target, &preview, &mut budget)
        .unwrap();
    let mut receiver = StudioReceiver::default();

    for accepted in 1..=64u8 {
        let operation = title(
            accepted.wrapping_add(80),
            &format!("unconfirmed {accepted}"),
        );
        assert!(matches!(
            receiver
                .save_unconfirmed_overlay(
                    &mut p.bob,
                    &mut p.b_store,
                    SERVER,
                    target,
                    &preview,
                    ticket.basis,
                    ticket.branch,
                    operation.clone(),
                    &mut budget,
                )
                .unwrap(),
            StudioOverlaySaveVisit::Scheduled
        ));
        let work = receiver
            .detach(&mut p.bob)
            .expect("each admitted append must run detached");
        receiver.complete(&mut p.bob, work.run(None).await);
        let saved = receiver
            .save_unconfirmed_overlay(
                &mut p.bob,
                &mut p.b_store,
                SERVER,
                target,
                &preview,
                ticket.basis,
                ticket.branch,
                operation,
                &mut budget,
            )
            .unwrap();
        let StudioOverlaySaveVisit::Saved(saved) = saved else {
            panic!("detached append {accepted} was not committed")
        };
        let catcoms_replication::studio::StudioOverlaySave::Local(draft) = *saved else {
            panic!("new authoring unexpectedly returned a terminal acknowledgement")
        };
        assert_eq!(draft.accepted(), accepted as usize);
    }

    let refused = match receiver.save_unconfirmed_overlay(
        &mut p.bob,
        &mut p.b_store,
        SERVER,
        target,
        &preview,
        ticket.basis,
        ticket.branch,
        title(200, "one operation too many"),
        &mut budget,
    ) {
        Err(error) => error,
        Ok(_) => panic!("a sixty-fifth Unconfirmed operation was admitted"),
    };
    assert!(refused.to_string().contains("64 accepted operations"));
    assert!(
        receiver.detach(&mut p.bob).is_none(),
        "the rejected operation must not consume detached capacity"
    );
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
