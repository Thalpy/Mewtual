use super::actor_save::save;
use super::*;
use crate::studio::StudioVaultLease;
use crate::{spawn, AppEvent};
use tokio::sync::Mutex;

#[tokio::test]
async fn studio_receiver_local_pass_starts_under_successive_page_service_requests() {
    let mut p = super::pages::proven_pair().await;
    let id = p.save(&title(1, "source"));
    p.send(title(1, "source")).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap();
    let peer = p.bob.local_peer();
    let (proof, tick) = tokio::join!(
        p.alice
            .sync
            .request_catchup(peer, catcoms_wire::DocType::Wiki, 43),
        p.bob.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let mut receiver = crate::studio::StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    for n in 0..2 {
        let request = p
            .alice
            .sync
            .prepare_studio_page(
                &watch.inner,
                peer,
                catcoms_sync::registry_catchup::StudioPageQuery {
                    target: target(),
                    doc_id: id,
                    heads: &[],
                    seed: None,
                    cursor: None,
                },
            )
            .unwrap();
        let (completed, ()) = tokio::join!(request.fetch(), async {
            p.bob.sync_once().await.unwrap();
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap();
            if n == 1 {
                let attempt = receiver
                    .detach(&mut p.bob)
                    .expect("bounded service turns must start local client");
                // This queued/unpolled job owns no Server. The second request can still be
                // answered while the local page attempt waits for native scheduling.
                receiver
                    .run(&mut p.bob, &mut p.b_store, SERVER, None)
                    .unwrap();
                drop(attempt);
            }
        });
        assert!(p
            .alice
            .sync
            .complete_studio_page(completed)
            .unwrap()
            .is_some());
        p.clock.advance_ms(1000);
    }
}

#[tokio::test]
async fn studio_receiver_membership_change_after_page_retries_without_explicit_access() {
    let mut p = super::pages::proven_pair().await;
    p.save(&title(1, "missed across membership change"));
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
    let mut receiver = crate::studio::StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    let job = receiver.detach(&mut p.bob).expect("initial page attempt");
    let (result, ()) = tokio::join!(job.run(None), async {
        p.alice.sync_once().await.unwrap();
        p.alice
            .serve_studio_request_step(&mut p.a_store, &mut provider, &watch)
            .unwrap()
            .unwrap();
    });
    receiver.complete(&mut p.bob, result);
    // A real, group-bound membership commit occurs after response authentication but before
    // the next save. Its old ciphertext must neither advance the cursor nor pause disk work.
    p.bob.subscribe_control().await.unwrap();
    let invite = p.alice.mint_invite([91; 16], u64::MAX, vec![]).unwrap();
    let (third, tick) = tokio::join!(
        Server::join(
            Net::new(p.hub.join(PeerId::from_u64(3))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "third",
            p.alice.local_peer(),
            &invite
        ),
        p.alice.sync_once()
    );
    third.unwrap();
    tick.unwrap();
    while p.bob.sync.with_registry_context(|g, _, _, _| g.epoch())
        != p.alice.sync.with_registry_context(|g, _, _, _| g.epoch())
    {
        p.bob.sync_once().await.unwrap();
    }
    assert_eq!(
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap()
            .1,
        None
    );
    assert!(!receiver.take_pause_notice());
    assert!(p.state().is_none());
    // This test drives the provider adapter directly; mirror the runtime's detached source
    // preparation after MLS invalidated its old warm binding, without an explicit Read.
    let capture = p
        .alice
        .sync
        .with_registry_context(|g, d, _, _| p.a_store.capture_studio_source(SERVER, g, target(), d))
        .unwrap()
        .unwrap();
    let prepared = tokio::task::spawn_blocking(move || capture.rebuild())
        .await
        .unwrap()
        .unwrap();
    assert!(p
        .alice
        .sync
        .with_registry_context(|g, d, _, _| p
            .a_store
            .install_prepared_studio_source(g, d, prepared))
        .unwrap());
    p.clock.advance_ms(5_000);
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    let job = receiver
        .detach(&mut p.bob)
        .expect("fresh page without Read");
    let (result, ()) = tokio::join!(job.run(None), async {
        p.alice.sync_once().await.unwrap();
        p.alice
            .serve_studio_request_step(&mut p.a_store, &mut provider, &watch)
            .unwrap()
            .unwrap();
    });
    receiver.complete(&mut p.bob, result);
    assert_eq!(
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap()
            .1,
        Some(target())
    );
    assert!(!receiver.take_pause_notice());
    assert_eq!(p.state().unwrap().op_count(), 1);
}

#[tokio::test]
async fn studio_actors_simultaneously_catch_up_missed_independent_edits_without_more_saves() {
    let mut p = Pair::new().await;
    // Establish transport-bound membership through the existing authenticated exchange.
    let peer = p.alice.local_peer();
    let (proof, tick) = tokio::join!(
        p.bob
            .sync
            .request_catchup(peer, catcoms_wire::DocType::Wiki, 43),
        p.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let peer = p.bob.local_peer();
    let (proof, tick) = tokio::join!(
        p.alice
            .sync
            .request_catchup(peer, catcoms_wire::DocType::Wiki, 43),
        p.bob.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let id = p.save(&title(21, "missed Alice title"));
    let body = FlipnoteOp::SetHeader(FlipnoteHeader::Fps(12))
        .encode()
        .unwrap();
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: id,
                nonce: [22; 16],
                body,
            },
        )
        .unwrap();
    let a_store = Arc::new(Mutex::new(Some(p.a_store)));
    let b_store = Arc::new(Mutex::new(Some(p.b_store)));
    let (a, mut ae, at) = spawn(p.alice);
    let (b, mut be, bt) = spawn(p.bob);
    let (updates, mut received) = tokio::sync::mpsc::unbounded_channel();
    let tx = updates.clone();
    let ad = tokio::spawn(async move {
        while let Some(e) = ae.recv().await {
            match e.event {
                AppEvent::StudioUpdated { .. } => {
                    tx.send(0).unwrap();
                }
                AppEvent::StudioReceivePaused => panic!("healthy catchup paused"),
                _ => {}
            }
        }
    });
    let bd = tokio::spawn(async move {
        while let Some(e) = be.recv().await {
            match e.event {
                AppEvent::StudioUpdated { .. } => {
                    updates.send(1).unwrap();
                }
                AppEvent::StudioReceivePaused => panic!("healthy catchup paused"),
                _ => {}
            }
        }
    });
    save(&a, &a_store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    save(&b, &b_store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    let mut seen = [false; 2];
    // Drive the actual native-facing Ready/lease seam, with both actors attempting each round.
    // No further Read/Save or test-only queue drain can break a mutual network wait.
    for _ in 0..40 {
        p.clock.advance_ms(1000);
        let step = |actor: crate::ServerActor, store: Arc<Mutex<Option<ServerStore>>>| async move {
            let ready = actor.studio_receive_begin().await.unwrap();
            ready
                .execute(StudioVaultLease::new(
                    store
                        .try_lock_owned()
                        .expect("network wait must release vault"),
                    SERVER,
                    (),
                ))
                .await
                .unwrap();
        };
        tokio::join!(
            step(a.clone(), a_store.clone()),
            step(b.clone(), b_store.clone())
        );
        while let Ok(peer) = received.try_recv() {
            seen[peer] = true;
        }
        if seen == [true, true] {
            break;
        }
    }
    assert_eq!(
        seen,
        [true, true],
        "both directions must catch up through detached network work"
    );
    let av = save(&a, &a_store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    let bv = save(&b, &b_store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(av.projection, bv.projection);
    a.shutdown().await;
    b.shutdown().await;
    at.await.unwrap();
    bt.await.unwrap();
    ad.await.unwrap();
    bd.await.unwrap();
}

#[tokio::test]
async fn studio_page_service_progresses_while_gossip_remains_queued() {
    let mut p = Pair::new().await;
    p.save(&title(1, "base"));
    p.send(title(1, "base")).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap();
    let id = p.state().unwrap().doc_id();
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: id,
                nonce: [90; 16],
                body: FlipnoteOp::SetHeader(FlipnoteHeader::Fps(12))
                    .encode()
                    .unwrap(),
            },
        )
        .unwrap();
    let mut receiver = crate::studio::StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    let peer = p.bob.local_peer();
    let (proof, tick) = tokio::join!(
        p.alice
            .sync
            .request_catchup(peer, catcoms_wire::DocType::Wiki, 43),
        p.bob.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    for n in 2..=7 {
        let op = title(n, &format!("gossip {n}"));
        p.save(&op);
        p.send(op).await.unwrap();
        p.bob.sync_once().await.unwrap();
    }
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    let request = p
        .alice
        .sync
        .prepare_studio_page(
            &watch.inner,
            peer,
            catcoms_sync::registry_catchup::StudioPageQuery {
                target: target(),
                doc_id: id,
                heads: &[],
                seed: None,
                cursor: None,
            },
        )
        .unwrap();
    let (completed, ()) = tokio::join!(request.fetch(), async {
        p.bob.sync_once().await.unwrap(); // authenticated page request joins existing gossip work
        assert_eq!(
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap()
                .1,
            Some(target())
        );
        assert_eq!(
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap()
                .1,
            None
        );
        assert!(
            receiver.pending(&p.bob),
            "page service cannot require emptying the gossip queue"
        );
    });
    let Some(catcoms_replication::studio::catchup::StudioPageOutcome::Page(page)) =
        p.alice.sync.complete_studio_page(completed).unwrap()
    else {
        panic!("served page")
    };
    let mut b = budget(&mut p.alice, &mut p.a_store);
    let admitted = p
        .alice
        .sync
        .with_registry_context(|g, d, _, rng| {
            p.a_store
                .ingest_studio_page(SERVER, g, target(), id, d, &page.operations, rng, &mut b)
        })
        .unwrap();
    assert_eq!(
        admitted.accepted, 1,
        "Bob's missing edit arrived while Bob was still receiving gossip"
    );
}
