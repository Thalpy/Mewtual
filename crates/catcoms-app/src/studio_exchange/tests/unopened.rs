use super::*;
use crate::studio::StudioReceiver;
use crate::studio_exchange::discovery::ServerCheckpointDiscovery;
use catcoms_sync::checkpoint_exchange::CheckpointTarget;

#[tokio::test]
async fn studio_unopened_cancelled_preparation_cannot_reuse_its_paid_interest() {
    let mut p = super::pages::proven_pair().await;
    p.save(&title(1, "cold source"));
    drop(p.a_store);
    p.a_store = open(p._a_root.path());
    let mut receiver = StudioReceiver::default();
    receiver
        .run(&mut p.alice, &mut p.a_store, SERVER, None)
        .unwrap();
    let attempt = p
        .bob
        .prepare_checkpoint_discovery(
            &p.b_store,
            SERVER,
            p.alice.local_peer(),
            CheckpointTarget::Studio(target()),
        )
        .unwrap();
    let (_completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
        let job = receiver.detach(&mut p.alice).expect("first paid capture");
        let (cancel, signal) = tokio::sync::watch::channel(false);
        cancel.send_replace(true);
        let result = job.run(Some(RequestCancellation::new(signal, None))).await;
        receiver.complete(&mut p.alice, result);
        for _ in 0..4 {
            receiver
                .run(&mut p.alice, &mut p.a_store, SERVER, None)
                .unwrap();
            assert!(
                receiver.detach(&mut p.alice).is_none(),
                "cancelled exact request cannot capture twice"
            );
        }
        assert!(!receiver.take_pause_notice());
        p.clock.advance_ms(10_000);
        receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
    });
}

#[tokio::test]
async fn studio_actors_new_member_after_checkpoint_discovers_installs_and_receives_tail() {
    actor_newcomer(target(), false).await;
}

#[tokio::test]
async fn studio_actors_new_member_after_index_checkpoint_installs_registry_and_index_tail() {
    actor_newcomer(StudioTarget::Index { channel: channel() }, false).await;
}

#[tokio::test]
async fn studio_actors_closing_reopen_after_expired_selection_resumes_without_another_action() {
    actor_newcomer(target(), true).await;
}

#[tokio::test]
async fn studio_receiver_large_registry_cold_provider_and_reopened_receiver_do_not_block_joining() {
    let mut p = super::pages::proven_pair().await;
    let (_, _, id) = super::discovery::prepared_checkpoint(&mut p, target());
    let bucket = prepared_registry(&mut p, target(), true);
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: id,
                nonce: [99; 16],
                body: title(99, "large registry tail").body,
            },
        )
        .unwrap();
    let mut p = reopen_with_large_registry(p, bucket);
    let mut client = StudioReceiver::default();
    let mut provider = StudioReceiver::default();
    provider
        .run(&mut p.alice, &mut p.a_store, SERVER, None)
        .unwrap();
    client
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    let (proof, tick) = tokio::join!(
        p.alice.request_channel_index_catchup(p.bob.local_peer()),
        p.bob.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    client
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    let local_preparation = client
        .detach(&mut p.bob)
        .expect("large local bucket prepares before first page");
    assert!(local_preparation.is_preparation_for_test());
    let remote = p
        .alice
        .prepare_checkpoint_discovery(
            &p.a_store,
            SERVER,
            p.bob.local_peer(),
            CheckpointTarget::Registry(bucket.wrapping_add(1)),
        )
        .unwrap();
    let (_answer, ()) = tokio::join!(remote.fetch(), async {
        p.bob.sync_once().await.unwrap();
        client
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
        // The other bucket must wait without replacing the pending local attachment target.
        client.complete(&mut p.bob, local_preparation.run(None).await);
        client
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
        client
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
    });
    assert!(!client.take_pause_notice());
    // Unlike a synthetic rapid actor loop, await each actual detached preparation. This
    // drives the same receiver methods without advancing Clock through CPU verification.
    for _ in 0..40 {
        p.clock.advance_ms(1000);
        client
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
        if let Some(job) = client.detach(&mut p.bob) {
            let result = if job.is_preparation_for_test() {
                job.run(None).await
            } else {
                let (result, ()) = tokio::join!(job.run(None), async {
                    while !provider.pending(&p.alice) {
                        p.alice.sync_once().await.unwrap();
                    }
                    provider
                        .run(&mut p.alice, &mut p.a_store, SERVER, None)
                        .unwrap();
                    if let Some(preparation) = provider.detach(&mut p.alice) {
                        assert!(preparation.is_preparation_for_test());
                        provider.complete(&mut p.alice, preparation.run(None).await);
                    }
                    provider
                        .run(&mut p.alice, &mut p.a_store, SERVER, None)
                        .unwrap();
                });
                result
            };
            client.complete(&mut p.bob, result);
        }
        if let Some(state) = p.state() {
            if state.doc_id() == id && state.op_count() == 1 {
                break;
            }
        }
    }
    assert!(!client.take_pause_notice() && !provider.take_pause_notice());
    let state = p.state().expect("installed Studio source");
    assert_eq!((state.doc_id(), state.op_count()), (id, 1));
    let StudioProjection::Flipnote(art) = state.projection().unwrap() else {
        panic!()
    };
    assert_eq!(art.title.unwrap().selected.value, "large registry tail");
}
fn reopen_with_large_registry(mut p: Pair, bucket: u8) -> Pair {
    // Give the newcomer a legitimate large Registry checkpoint, then reopen both mounts
    // so neither receiver nor provider can rely on a previously warmed footprint LRU.
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let (receipt, seed) = p.alice.sync.with_registry_context(|g, d, _, _| {
        let state = p
            .a_store
            .load_registry_epoch(SERVER, g, bucket, d)
            .unwrap()
            .unwrap();
        let head = p
            .a_store
            .load_epoch_owner_receipts(
                SERVER,
                &catcoms_replication::registry::registry_document(&g.group_id(), bucket).unwrap(),
            )
            .unwrap();
        let receipt = head.pending().or_else(|| head.published()).unwrap().clone();
        // Rebuild the canonical seed from the selected projection at its predecessor epoch.
        let mut projection = state.projection().unwrap();
        projection.epoch = 0;
        (
            receipt.clone(),
            projection.checkpoint(receipt.close_record_hash).unwrap(),
        )
    });
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            p.b_store
                .with_studio_protocol_budget(SERVER, g, &mut b, |store, budget| {
                    store
                        .adopt_registry_checkpoint(
                            SERVER,
                            g,
                            bucket,
                            d,
                            &receipt,
                            Some(seed.bytes()),
                            0,
                            &p.clock,
                            rng,
                            budget,
                        )
                        .map(|_| ())
                })
        })
        .unwrap();
    drop(p.a_store);
    p.a_store = open(p._a_root.path());
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    assert!(!p
        .b_store
        .registry_receive_source_fits(SERVER, &p.bob.group_id(), bucket)
        .unwrap());

    p
}

async fn actor_newcomer(selected_target: StudioTarget, restart_closing: bool) {
    use crate::studio::{StudioRequest, StudioVaultLease};
    use crate::{spawn, AppEvent};
    use tokio::sync::Mutex;
    let mut p = Pair::new().await;
    let (_, _, id) = super::discovery::prepared_checkpoint(&mut p, selected_target);
    let bucket = prepared_registry(&mut p, selected_target, false);
    let tail = match selected_target {
        StudioTarget::Flipnote { .. } => title(99, "post-checkpoint tail").body,
        StudioTarget::Index { .. } => IndexOp::SetTitle {
            object: [7; 16],
            title: "post-checkpoint tail".into(),
        }
        .encode()
        .unwrap(),
    };
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Apply {
                target: selected_target,
                epoch_id: id,
                nonce: [99; 16],
                body: tail,
            },
        )
        .unwrap();
    // This device joins AFTER rotation, not merely a pre-existing member with an empty vault.
    let invite = p.alice.mint_invite([44; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            Net::new(p.hub.join(PeerId::from_u64(3))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "newcomer",
            p.alice.local_peer(),
            &invite
        ),
        p.alice.sync_once()
    );
    tick.unwrap();
    p.bob = joined.unwrap();
    let (proof, tick) = tokio::join!(
        p.bob.request_channel_index_catchup(p.alice.local_peer()),
        p.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    if restart_closing {
        // The membership addition invalidated the old warm source. The explicit fixture
        // adapter requires local preparation; automatic provider preparation is tested below.
        p.alice.sync.with_registry_context(|g, d, _, _| {
            let state = p
                .a_store
                .load_studio_epoch(SERVER, g, selected_target, d)
                .unwrap()
                .unwrap();
            p.a_store.retain_studio_source(g, d, state);
        });
        let watch = p
            .alice
            .watch_studio_checkpoint(&p.a_store, SERVER, selected_target)
            .unwrap();
        let pass = super::discovery::discover(&mut p, &watch, selected_target).await;
        let mut b = budget(&mut p.bob, &mut p.b_store);
        let (outcome, state) = p
            .bob
            .install_studio_seed_step(&mut p.b_store, SERVER, &pass, &mut b)
            .unwrap();
        assert_eq!(outcome, crate::store::StudioAdoptionOutcome::AwaitingSeed);
        assert_eq!(state.phase(), catcoms_replication::EpochPhase::Closing);
        drop(pass);
        p.alice.unwatch_studio_checkpoint(&watch).unwrap();
        p.clock.advance_ms(61_000);
        let snapshot = p.bob.snapshot().unwrap();
        p.bob = Node::restore(
            &snapshot,
            Net::new(p.hub.join(PeerId::from_u64(3))),
            rng(),
            Box::new(p.clock.clone()),
            "newcomer",
        )
        .unwrap();
        drop(p.b_store);
        p.b_store = open(p.b_root.path());
        // Endpoint knowledge is deliberately not restored as current proof.
        let (proof, tick) = tokio::join!(
            p.bob.request_channel_index_catchup(p.alice.local_peer()),
            p.alice.sync_once()
        );
        proof.unwrap();
        tick.unwrap();
    }
    let verify_snapshot = p.bob.snapshot().unwrap();
    let a_store = Arc::new(Mutex::new(Some(p.a_store)));
    let b_store = Arc::new(Mutex::new(Some(p.b_store)));
    let (a, mut ae, at) = spawn(p.alice);
    let (b, mut be, bt) = spawn(p.bob);
    let ad = tokio::spawn(async move {
        while let Some(e) = ae.recv().await {
            assert!(!matches!(e.event, AppEvent::StudioReceivePaused));
        }
    });
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let bd = tokio::spawn(async move {
        while let Some(e) = be.recv().await {
            assert!(!matches!(e.event, AppEvent::StudioReceivePaused));
            if matches!(e.event, AppEvent::StudioUpdated { .. }) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let initial = super::actor_save::save(
        &b,
        &b_store,
        StudioRequest::Read {
            target: selected_target,
        },
    )
    .await
    .unwrap();
    match selected_target {
        StudioTarget::Flipnote { .. } if restart_closing => assert_eq!(initial.unwrap().epoch, 0),
        StudioTarget::Flipnote { .. } => assert!(initial.is_none()),
        StudioTarget::Index { .. } => assert_eq!(initial.unwrap().epoch, 0),
    };
    // No provider Read and no more newcomer Read/Save. Drive only the existing native-facing
    // worker; neither endpoint can monopolize Server while it waits on the other's response.
    for _ in 0..100 {
        p.clock.advance_ms(1000);
        let step = |actor: crate::ServerActor, store: Arc<Mutex<Option<ServerStore>>>| async move {
            actor
                .studio_receive_begin()
                .await
                .unwrap()
                .execute(StudioVaultLease::new(
                    store.try_lock_owned().unwrap(),
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
        if count.load(Ordering::SeqCst) >= 3 {
            break;
        }
    }
    assert!(
        count.load(Ordering::SeqCst) >= 3,
        "Closing, installation and remote tail each repaint: got {}",
        count.load(Ordering::SeqCst)
    );
    let view = super::actor_save::save(
        &b,
        &b_store,
        StudioRequest::Read {
            target: selected_target,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(view.epoch_id, id);
    match view.projection {
        StudioProjection::Flipnote(art) => {
            assert_eq!(art.title.unwrap().selected.value, "post-checkpoint tail")
        }
        StudioProjection::Index(index) => assert_eq!(
            index.objects[&[7; 16]].title.selected.value,
            "post-checkpoint tail"
        ),
    }
    a.shutdown().await;
    b.shutdown().await;
    at.await.unwrap();
    bt.await.unwrap();
    ad.await.unwrap();
    bd.await.unwrap();
    drop(b_store.lock().await.take());
    let reopened = open(p.b_root.path());
    let mut verifier = Node::restore(
        &verify_snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(91))),
        rng(),
        Box::new(p.clock.clone()),
        "newcomer",
    )
    .unwrap();
    verifier.sync.with_registry_context(|g, d, _, _| {
        let registry = reopened
            .load_registry_epoch(SERVER, g, bucket, d)
            .unwrap()
            .expect("automatic registry checkpoint bootstrap");
        assert_eq!(registry.epoch(), 1);
        assert_eq!(
            reopened
                .load_studio_epoch(SERVER, g, selected_target, d)
                .unwrap()
                .unwrap()
                .doc_id(),
            id
        );
    });
}

fn prepared_registry(p: &mut Pair, target: StudioTarget, large: bool) -> u8 {
    use catcoms_replication::{
        registry::{registry_document, PointerKey, RegistryOp},
        registry_epoch::RegistryEpoch,
        InheritedCheckpoint, Receipt,
    };
    let logical = target.document(&p.alice.group_id()).unwrap();
    let key = PointerKey::new(logical.doc_type, logical.logical_key).unwrap();
    let bucket = key.bucket();
    let (receipt, seed) = p.alice.sync.with_registry_context(|g, d, _, rng| {
        let mut unit = RegistryEpoch::new(g, bucket, d.device_id()).unwrap();
        let logical = registry_document(&g.group_id(), bucket).unwrap();
        unit.edit(
            d,
            g,
            rng,
            &DomainOp {
                doc_type: logical.doc_type,
                logical_key: logical.logical_key.clone(),
                nonce: [41; 16],
                body: RegistryOp::Put { key, epoch: 1 }.encode().unwrap(),
            },
        )
        .unwrap();
        let mut projection = unit.projection().unwrap();
        if large {
            use rand_core::RngCore;
            while projection.pointers.len() < 512 {
                let mut key = vec![0; 192];
                rng.fill_bytes(&mut key);
                let key = PointerKey::new(catcoms_wire::DocType::StudioObject, key).unwrap();
                if key.bucket() == bucket {
                    projection.pointers.insert(key, 1);
                }
            }
        }
        let seed = projection.checkpoint([61; 32]).unwrap();
        let receipt = Receipt::sign(
            logical,
            0,
            [61; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        (receipt, seed)
    });
    let mut b = budget(&mut p.alice, &mut p.a_store);
    p.alice.sync.with_registry_context(|g, d, _, rng| {
        p.a_store
            .with_studio_protocol_budget(SERVER, g, &mut b, |store, budget| {
                store.adopt_registry_checkpoint(
                    SERVER,
                    g,
                    bucket,
                    d,
                    &receipt,
                    Some(seed.bytes()),
                    0,
                    &p.clock,
                    rng,
                    budget,
                )?;
                store.prepare_epoch_owner_receipt(SERVER, receipt, g, 0, rng, budget)?;
                Ok(())
            })
            .unwrap();
    });
    bucket
}

#[tokio::test]
async fn studio_unopened_provider_restart_serves_head_seed_and_pages_without_ui() {
    let mut p = Pair::new().await;
    let (_, _, id) = super::discovery::prepared_checkpoint(&mut p, target());
    // A real provider restart drops all warm sources, receipt/seed registrations and UI watches.
    let snapshot = p.alice.snapshot().unwrap();
    p.alice = Node::restore(
        &snapshot,
        p.wire.clone(),
        rng(),
        Box::new(p.clock.clone()),
        "alice",
    )
    .unwrap();
    drop(p.a_store);
    p.a_store = open(p._a_root.path());
    let peer = p.alice.local_peer();
    let (proof, tick) = tokio::join!(
        p.bob.request_channel_index_catchup(peer),
        p.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let mut receiver = StudioReceiver::default();
    // Same no-request native idle pass used at startup: no Studio Read, watch, head adapter or
    // owner snapshot is explicitly called by this provider fixture.
    receiver
        .run(&mut p.alice, &mut p.a_store, SERVER, None)
        .unwrap();
    let attempt = p
        .bob
        .prepare_checkpoint_discovery(&p.b_store, SERVER, peer, CheckpointTarget::Studio(target()))
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        assert!(
            receiver.pending(&p.alice),
            "zero-watch service must wake native"
        );
        receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
        let preparation = receiver
            .detach(&mut p.alice)
            .expect("cold saved source prepares off actor");
        receiver.complete(&mut p.alice, preparation.run(None).await);
        receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
    });
    let Some(ServerCheckpointDiscovery::Selected(mut pass)) = p
        .bob
        .complete_checkpoint_discovery(&p.b_store, SERVER, completed)
        .unwrap()
    else {
        panic!("current owner proof")
    };
    let fetch = p
        .bob
        .prepare_checkpoint_seed_fetch(&mut pass, peer)
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(fetch.fetch(), async {
        p.alice.sync_once().await.unwrap();
        receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
    });
    assert!(p
        .bob
        .complete_checkpoint_seed_fetch(&mut pass, completed)
        .unwrap());
    let mut budget = budget(&mut p.bob, &mut p.b_store);
    let (_, state) = p
        .bob
        .install_studio_seed_step(&mut p.b_store, SERVER, &pass, &mut budget)
        .unwrap();
    assert_eq!(state.doc_id(), id);
    p.bob
        .sync
        .with_registry_context(|g, d, _, _| p.b_store.retain_studio_source(g, d, state));
    let watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target())
        .unwrap();
    let mut pass = p
        .bob
        .begin_studio_receive(&mut p.b_store, &watch, peer, &mut budget)
        .unwrap();
    let attempt = p
        .bob
        .prepare_studio_receive_step(&mut pass)
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
    });
    assert_eq!(
        p.bob
            .complete_studio_receive_step(&mut pass, completed)
            .unwrap(),
        StudioReceiveState::PageReady
    );
    assert!(!receiver.take_pause_notice());
}
