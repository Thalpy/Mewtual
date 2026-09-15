use super::*;
use crate::studio::{PreviewHarness, StudioPreview};
use catcoms_rt::Clock;

#[tokio::test]
async fn studio_preview_index_read_fallback_preserves_installed_source_priority() {
    let mut p = pages::proven_pair().await;
    let target = StudioTarget::Index { channel: channel() };
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    let mut runtime = PreviewHarness::default();
    let expected = drive(&mut p, &mut runtime).await;
    let watch = ServerStudioWatch {
        inner: p.watch.inner.copy_binding(),
        mount: p.watch.mount.clone(),
        server: SERVER,
        target,
    };
    let logical = target.document(&p.bob.group_id()).unwrap();
    let epoch_id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let mut receiver = runtime.into_receiver(vec![(watch, epoch_id)]);
    let (mut saved, _) = receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    assert_eq!(
        saved.view.as_ref().unwrap().epoch,
        0,
        "ordinary empty Index remains unstored"
    );
    let mut preview = saved
        .preview
        .take()
        .expect("synthetic empty Index must not mask preview");
    let handoff = preview.begin_delivery();
    preview
        .inspect(|_, projection| assert_eq!(projection, &expected))
        .unwrap();
    drop((preview, handoff, saved));
    assert_absent(&mut p, target);
    let owner = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    let body = IndexOp::PutObject {
        object: [8; 16],
        kind: StudioKind::Flipnote,
        title: "stored locally".into(),
        created_by: owner,
        ts: 1,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap();
    let ordinary = p
        .bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target,
                epoch_id,
                nonce: [33; 16],
                body,
            },
        )
        .unwrap()
        .unwrap();
    let (saved, _) = receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    assert!(
        saved.preview.is_none(),
        "an installed source always wins over an unconfirmed candidate"
    );
    assert_eq!(saved.view.unwrap().projection, ordinary.projection);
}

#[tokio::test]
async fn studio_preview_cancelled_blocking_worker_retains_parser_and_seed_capacity() {
    let mut p = pages::proven_pair().await;
    let (seed, op, _) = ready(&mut p).await;
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
    let runtime = PreviewHarness::default();
    let (job, entered, release) = runtime.tail_preparation(preparation).pause_for_test();
    let waiter = tokio::spawn(job.run());
    entered.await.unwrap();
    waiter.abort();
    assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
    let a = p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .unwrap();
    let b = p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .unwrap();
    assert!(p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .is_err());
    let pool = runtime.parser_pool();
    let authoritative = pool.clone().try_acquire_many_owned(3).unwrap();
    assert!(
        pool.clone().try_acquire_owned().is_err(),
        "cancelled waiter cannot refund the running worker"
    );
    // Expire the candidate while the actual worker still owns its memory. Expiry revokes use,
    // not accounting; even after it, no fourth preview fits until the worker actually exits.
    p.clock.advance_ms(60_000);
    assert!(p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .is_err());
    drop(release); // also releases on panic; no blocking worker can be stranded
    let finished = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        pool.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(p.bob.sync.reserve_provisional_checkpoint_capacity().is_ok());
    drop((a, b, authoritative, finished));
}

async fn drive(p: &mut Pair, runtime: &mut PreviewHarness) -> StudioProjection {
    p.clock.advance_ms(1000);
    let (parsed, op, projection) = ready(p).await;
    drop(parsed);
    let (receipt, seed) = candidate(p, p.watch.target);
    let head = p
        .alice
        .sync
        .watch_checkpoint_head(CheckpointTarget::Studio(p.watch.target))
        .unwrap();
    let seed_watch = p
        .alice
        .sync
        .watch_checkpoint_seed(CheckpointTarget::Studio(p.watch.target))
        .unwrap();
    let tail_watch = p
        .alice
        .sync
        .watch_studio(p.watch.target, op.doc_id)
        .unwrap();
    runtime.queue(&p.watch, p.alice.local_peer(), p.clock.monotonic_ms());
    // Duplicate triggers cannot add work or reset this candidate's position.
    runtime.queue(&p.watch, p.alice.local_peer(), p.clock.monotonic_ms());
    for stage in 0..5 {
        p.clock.advance_ms(1000);
        runtime.step(&mut p.bob, &p.b_store, SERVER);
        let job = runtime.take_job().expect("head, seed, parse, tail, parse");
        let result = if job.is_preparation() {
            assert!(stage == 2 || stage == 4);
            job.run().await
        } else {
            let (result, ()) = tokio::join!(job.run(), async {
                loop {
                    let served = match stage {
                        0 => p
                            .alice
                            .sync
                            .serve_receipt_head(&head, None, |_, _, _, _| {
                                Ok::<_, ()>(ReceiptHeadSelection {
                                    receipt: Some(receipt.clone()),
                                    prove: false,
                                })
                            })
                            .unwrap()
                            .map(|served| served.unwrap())
                            .map(|_| ()),
                        1 => p
                            .alice
                            .sync
                            .serve_registry_seed(&seed_watch, |_, _, _, _| {
                                Ok::<_, ()>(Some(seed.bytes().to_vec()))
                            })
                            .unwrap()
                            .map(|served| served.unwrap())
                            .map(|_| ()),
                        3 => p
                            .alice
                            .sync
                            .serve_studio_request(&tail_watch, |_, _, _, _| {
                                Ok::<_, ()>(RegistryPageOutcome::Page(RegistryOpPage {
                                    operations: vec![op.clone()],
                                    next: None,
                                }))
                            })
                            .unwrap()
                            .map(|served| served.unwrap())
                            .map(|_| ()),
                        _ => panic!("unexpected network stage"),
                    };
                    if served.is_some() {
                        break;
                    }
                    p.alice.sync_once().await.unwrap();
                }
            });
            result
        };
        runtime.complete(result);
    }
    runtime.step(&mut p.bob, &p.b_store, SERVER);
    assert!(
        runtime.take_job().is_none(),
        "a duplicate must not refill a ready preview"
    );
    projection
}

#[tokio::test]
async fn studio_preview_runtime_three_ready_slots_include_late_delivery_and_release_authoritative_hints(
) {
    let mut p = pages::proven_pair().await;
    let mut runtime = PreviewHarness::default();
    let targets = [
        StudioTarget::Index { channel: channel() },
        target(),
        StudioTarget::Flipnote {
            channel: channel(),
            object: [8; 16],
        },
    ];
    for (i, target) in targets.into_iter().enumerate() {
        p.watch = p
            .bob
            .watch_studio_epoch(&p.b_store, SERVER, target)
            .unwrap();
        let expected = drive(&mut p, &mut runtime).await;
        assert_eq!(runtime.ready(), i + 1);
        assert_eq!(
            runtime.parser_pool().available_permits(),
            4,
            "ready previews must release parser permits"
        );
        let mut preview = runtime.read(&p.bob, &p.b_store, SERVER, target).unwrap();
        let handoff = preview.begin_delivery();
        preview
            .inspect(|_, projection| assert_eq!(projection, &expected))
            .unwrap();
        drop(preview);
        handoff.finish(p.bob.runtime_clock(), None).await;
        assert_absent(&mut p, target);
    }
    assert!(p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .is_err());
    let mut preview = runtime
        .read(&p.bob, &p.b_store, SERVER, targets[0])
        .unwrap();
    let handoff = preview.begin_delivery();
    let delivery = preview.delivery();
    drop(preview);
    drop(handoff);
    assert!(!delivery.is_current());
    // Same-key replacement invalidates the cache, but cannot refund a native custodian.
    let _replacement = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, targets[0])
        .unwrap();
    runtime.step(&mut p.bob, &p.b_store, SERVER);
    assert_eq!(runtime.ready(), 2);
    assert!(runtime
        .read(&p.bob, &p.b_store, SERVER, targets[0])
        .is_none());
    assert!(p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .is_err());

    // The fourth slot can perform real authoritative discovery and release an authenticated
    // Hint even while all three provisional slots have ready/native owners.
    let target = StudioTarget::Flipnote {
        channel: channel(),
        object: [9; 16],
    };
    let (receipt, _) = candidate(&mut p, target);
    let watch = p
        .alice
        .sync
        .watch_checkpoint_head(CheckpointTarget::Studio(target))
        .unwrap();
    let attempt = p
        .bob
        .prepare_checkpoint_discovery(
            &p.b_store,
            SERVER,
            p.alice.local_peer(),
            CheckpointTarget::Studio(target),
        )
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        loop {
            if let Some(served) = p
                .alice
                .sync
                .serve_receipt_head(&watch, None, |_, _, _, _| {
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
    let hint = p
        .bob
        .complete_checkpoint_discovery(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap();
    assert!(matches!(
        hint,
        crate::studio_exchange::discovery::ServerCheckpointDiscovery::Hint(_)
    ));
    // A second authoritative request fits while the raw Hint is still held.
    let second = p
        .bob
        .prepare_checkpoint_discovery(
            &p.b_store,
            SERVER,
            p.alice.local_peer(),
            CheckpointTarget::Studio(target),
        )
        .unwrap();
    assert!(p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .is_err());
    drop(second);
    assert!(!delivery.is_current());
    assert!(p
        .bob
        .sync
        .reserve_provisional_checkpoint_capacity()
        .is_err());
    drop(delivery);
    assert!(p.bob.sync.reserve_provisional_checkpoint_capacity().is_ok());
}

#[tokio::test]
async fn studio_preview_native_handoff_cancellation_expiry_and_last_copy_bound_delivery() {
    for cause in ["cancel", "deadline", "seed-expiry", "last-copy"] {
        let mut p = pages::proven_pair().await;
        let mut runtime = PreviewHarness::default();
        drive(&mut p, &mut runtime).await;
        let mut preview: StudioPreview =
            runtime.read(&p.bob, &p.b_store, SERVER, target()).unwrap();
        let handoff = preview.begin_delivery();
        let delivery = preview.delivery();
        let last = delivery.clone();
        drop(preview);
        let (cancel, signal) = tokio::sync::watch::channel(false);
        let mut finish = Box::pin(handoff.finish(
            p.bob.runtime_clock(),
            Some(RequestCancellation::new(signal, None)),
        ));
        let waker = Waker::noop();
        assert!(finish
            .as_mut()
            .poll(&mut Context::from_waker(waker))
            .is_pending());
        match cause {
            "cancel" => {
                cancel.send(true).unwrap();
            }
            "deadline" => {
                p.clock.advance_ms(5_000);
            }
            "seed-expiry" => {
                p.clock.advance_ms(60_000);
                assert!(!delivery.is_current());
            }
            _ => {
                drop(delivery);
                assert!(finish
                    .as_mut()
                    .poll(&mut Context::from_waker(waker))
                    .is_pending());
                drop(last);
                finish.await;
                continue;
            }
        }
        finish.await;
        assert!(!delivery.is_current());
        assert!(!last.is_current());
    }
}
