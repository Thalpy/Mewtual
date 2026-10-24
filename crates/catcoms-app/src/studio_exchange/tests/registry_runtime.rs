use super::*;
use crate::studio::{StudioRequest, StudioVaultLease};
use catcoms_replication::registry::{PointerKey, RegistryOp};
use catcoms_replication::registry_epoch::RegistryEpoch;
use tokio::sync::Mutex;

#[tokio::test]
async fn studio_held_registry_page_is_discarded_after_fault_or_checkpoint_replacement() {
    use crate::registry_catchup::RegistryReceiveState;
    use catcoms_replication::{InheritedCheckpoint, Receipt};
    for fault in [false, true] {
        let mut p = super::pages::proven_pair().await;
        let logical = target().document(&p.alice.group_id()).unwrap();
        let key = PointerKey::new(logical.doc_type, logical.logical_key).unwrap();
        let bucket = key.bucket();
        let (packets, receipt, seed, other) = p.alice.sync.with_registry_context(|g, d, _, rng| {
            let mut unit = RegistryEpoch::new(g, bucket, d.device_id()).unwrap();
            let mut packets = vec![];
            for n in [1, 2] {
                packets.push(
                    unit.edit(
                        d,
                        g,
                        rng,
                        &RegistryOp::Put {
                            key: key.clone(),
                            epoch: n,
                        }
                        .domain_op(&g.group_id(), [n as u8; 16])
                        .unwrap(),
                    )
                    .unwrap(),
                );
            }
            let logical =
                catcoms_replication::registry::registry_document(&g.group_id(), bucket).unwrap();
            let seed = unit.projection().unwrap().checkpoint([81; 32]).unwrap();
            let receipt = Receipt::sign(
                logical.clone(),
                0,
                [81; 32],
                seed.change_hash(),
                0,
                InheritedCheckpoint::EpochZero,
                d,
            )
            .unwrap();
            let other = Receipt::sign(
                logical,
                0,
                [82; 32],
                [82; 32],
                0,
                InheritedCheckpoint::EpochZero,
                d,
            )
            .unwrap();
            (packets, receipt, seed, other)
        });
        for (node, store, packets) in [
            (&mut p.alice, &mut p.a_store, packets.as_slice()),
            (&mut p.bob, &mut p.b_store, &packets[..1]),
        ] {
            let mut b = budget(node, store);
            node.sync.with_registry_context(|g, d, _, rng| {
                store
                    .with_studio_protocol_budget(SERVER, g, &mut b, |store, budget| {
                        for op in packets {
                            store.ingest_registry_epoch(SERVER, g, bucket, d, op, rng, budget)?;
                        }
                        Ok(())
                    })
                    .unwrap();
            });
        }
        let awatch = p
            .alice
            .watch_registry_epoch(&p.a_store, SERVER, bucket)
            .unwrap();
        let bwatch = p
            .bob
            .watch_registry_epoch(&p.b_store, SERVER, bucket)
            .unwrap();
        let mut provider = p
            .alice
            .begin_registry_page_provider(&p.a_store, SERVER, bucket)
            .unwrap();
        crate::registry_catchup::prepare_test_source(&mut p.alice, &p.a_store, &mut provider)
            .await
            .unwrap();
        let mut b = budget(&mut p.bob, &mut p.b_store);
        let group = p.bob.group_id();
        let mut pass = p
            .b_store
            .with_studio_protocol_scope(SERVER, &group, &mut b, |store, budget| {
                p.bob
                    .begin_registry_receive(store, &bwatch, p.alice.local_peer(), budget)
            })
            .unwrap();
        let (fetched, ()) = tokio::join!(p.bob.fetch_registry_receive_step(&mut pass), async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_request_step(&p.a_store, &mut provider, &awatch)
                .unwrap()
                .unwrap();
        });
        assert_eq!(fetched.unwrap(), RegistryReceiveState::PageReady);
        let mut receiver = crate::studio::StudioReceiver::default();
        receiver
            .run(
                &mut p.bob,
                &mut p.b_store,
                SERVER,
                Some(StudioRequest::Read { target: target() }),
            )
            .unwrap();
        receiver.hold_registry_page_for_test(target(), pass);
        let mut b = budget(&mut p.bob, &mut p.b_store);
        p.bob.sync.with_registry_context(|g, d, _, rng| {
            p.b_store
                .with_studio_protocol_budget(SERVER, g, &mut b, |store, budget| {
                    if fault {
                        store.seal_registry_epoch(
                            SERVER,
                            g,
                            bucket,
                            d,
                            receipt.clone(),
                            0,
                            rng,
                            budget,
                        )?;
                        store.seal_registry_epoch(SERVER, g, bucket, d, other, 0, rng, budget)?;
                    } else {
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
                    }
                    Ok(())
                })
                .unwrap();
        });
        for _ in 0..3 {
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap();
            if !receiver.has_registry_page_for_test() {
                break;
            }
            if let Some(job) = receiver.detach(&mut p.bob) {
                assert!(job.is_preparation_for_test());
                receiver.complete(&mut p.bob, job.run(None).await);
            }
        }
        assert!(!receiver.has_registry_page_for_test());
        assert!(!receiver.take_pause_notice());
        p.bob.sync.with_registry_context(|g, d, _, _| {
            let state = p
                .b_store
                .load_registry_epoch(SERVER, g, bucket, d)
                .unwrap()
                .unwrap();
            assert_eq!(
                state.op_count(),
                if fault { 1 } else { 0 },
                "old page must not persist"
            );
            assert_eq!(
                state.phase(),
                if fault {
                    catcoms_replication::EpochPhase::Fault
                } else {
                    catcoms_replication::EpochPhase::Open
                }
            );
        });
    }
}

#[tokio::test]
async fn studio_faulted_registry_bucket_does_not_pause_healthy_studio_receive() {
    let mut p = Pair::new().await;
    let initial = title(1, "before fault");
    p.save(&initial);
    p.send(initial).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap().unwrap();
    let logical = target().document(&p.alice.group_id()).unwrap();
    let key = PointerKey::new(logical.doc_type, logical.logical_key).unwrap();
    let bucket = key.bucket();
    let receipts = p.alice.sync.with_registry_context(|g, d, _, _| {
        let logical =
            catcoms_replication::registry::registry_document(&g.group_id(), bucket).unwrap();
        [71, 72].map(|n| {
            catcoms_replication::Receipt::sign(
                logical.clone(),
                0,
                [n; 32],
                [n; 32],
                0,
                catcoms_replication::InheritedCheckpoint::EpochZero,
                d,
            )
            .unwrap()
        })
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob.sync.with_registry_context(|g, d, _, rng| {
        p.b_store
            .with_studio_protocol_budget(SERVER, g, &mut b, |store, budget| {
                let mut unit = RegistryEpoch::new(g, bucket, d.device_id()).unwrap();
                let op = RegistryOp::Put {
                    key: key.clone(),
                    epoch: 0,
                }
                .domain_op(&g.group_id(), [50; 16])
                .unwrap();
                let packet = unit.edit(d, g, rng, &op).unwrap();
                store.ingest_registry_epoch(SERVER, g, bucket, d, &packet, rng, budget)?;
                for receipt in receipts {
                    store.seal_registry_epoch(SERVER, g, bucket, d, receipt, 0, rng, budget)?;
                }
                Ok(())
            })
            .unwrap();
    });
    let mut receiver = crate::studio::StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    // Exercise due maintenance (including actual detached preparation) before receiving the
    // next edit. The saved Fault is legitimate accounting, not a global storage error.
    for _ in 0..8 {
        p.clock.advance_ms(5_000);
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
        if let Some(job) = receiver.detach(&mut p.bob) {
            assert!(job.is_preparation_for_test());
            receiver.complete(&mut p.bob, job.run(None).await);
        }
        assert!(!receiver.take_pause_notice());
    }
    let later = title(2, "after fault");
    p.save(&later);
    p.send(later).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let (_, updated) = receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(updated, Some(target()));
    assert!(!receiver.take_pause_notice());
    p.bob.sync.with_registry_context(|g, d, _, _| {
        assert_eq!(
            p.b_store
                .load_registry_epoch(SERVER, g, bucket, d)
                .unwrap()
                .unwrap()
                .phase(),
            catcoms_replication::EpochPhase::Fault
        );
    });
}

#[tokio::test]
async fn studio_actors_publish_real_registry_pointers_and_receive_unsettled_tail() {
    registry_tail(false).await;
}

#[tokio::test]
async fn studio_actors_large_registry_tail_keeps_inventory_warm_between_pages() {
    registry_tail(true).await;
}

async fn registry_tail(large: bool) {
    let mut p = super::pages::proven_pair().await;
    let peer = p.bob.local_peer();
    let (proof, tick) = tokio::join!(
        p.alice.request_channel_index_catchup(peer),
        p.bob.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    p.save(&title(1, "shared source"));
    p.send(title(1, "shared source")).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap().unwrap();
    let logical = target().document(&p.alice.group_id()).unwrap();
    let key = PointerKey::new(logical.doc_type, logical.logical_key).unwrap();
    let bucket = key.bucket();
    // A second pointer exists only in Alice's Registry log. Bob cannot manufacture it by
    // refreshing the opened Flipnote; observing it proves actual authenticated tail ingestion.
    let other = (1u128..10_000)
        .map(|n| {
            PointerKey::new(
                catcoms_wire::DocType::StudioObject,
                n.to_be_bytes().to_vec(),
            )
            .unwrap()
        })
        .find(|k| k.bucket() == bucket && *k != key)
        .unwrap();
    let mut budget = budget(&mut p.alice, &mut p.a_store);
    p.alice.sync.with_registry_context(|g, d, _, rng| {
        let mut unit = RegistryEpoch::new(g, bucket, d.device_id()).unwrap();
        for n in 0..if large { 3 } else { 1 } {
            let op = RegistryOp::Put {
                key: other.clone(),
                epoch: 5 + n,
            }
            .domain_op(&g.group_id(), [41 + n as u8; 16])
            .unwrap();
            // Valid signed, byte-heavy changes force separate pages and a >256 KiB wrapper.
            // Build against the actually accepted previous change, never an orphan test DAG.
            let mut next =
                RegistryEpoch::restore(&unit.snapshot().unwrap(), g, bucket, d.device_id())
                    .unwrap();
            let packet = next.edit(d, g, rng, &op).unwrap();
            let packet = if large {
                let signed = packet
                    .open(&g.channel_secret(d, packet.doc_type, packet.doc_id).unwrap())
                    .unwrap();
                let mut change = automerge::Change::from_bytes(signed.delta)
                    .unwrap()
                    .decode();
                change.message = Some("x".repeat(160_000));
                let change = automerge::Change::from(change);
                let signed = catcoms_replication::SignedOp::sign_domain(
                    d,
                    catcoms_wire::DocType::DocRegistry,
                    unit.doc_id(),
                    change.raw_bytes().to_vec(),
                    &op,
                )
                .unwrap();
                catcoms_replication::SealedOp::seal(&signed, g, d, rng).unwrap()
            } else {
                packet
            };
            unit.ingest(&packet, g, d).unwrap();
            p.a_store
                .with_studio_protocol_budget(SERVER, g, &mut budget, |store, budget| {
                    store.ingest_registry_epoch(SERVER, g, bucket, d, &packet, rng, budget)
                })
                .unwrap();
        }
    });
    let mut verifier = Node::restore(
        &p.bob.snapshot().unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(99))),
        rng(),
        Box::new(p.clock.clone()),
        "read-only verifier",
    )
    .unwrap();
    let a_store = Arc::new(Mutex::new(Some(p.a_store)));
    let b_store = Arc::new(Mutex::new(Some(p.b_store)));
    let (a, ae, at) = crate::spawn(p.alice);
    let (b, be, bt) = crate::spawn(p.bob);
    let drain = |mut events: tokio::sync::mpsc::Receiver<crate::TracedEvent>| async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, crate::AppEvent::StudioReceivePaused));
        }
    };
    let ad = tokio::spawn(drain(ae));
    let bd = tokio::spawn(drain(be));
    super::actor_save::save(&a, &a_store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    super::actor_save::save(&b, &b_store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    let mut done = false;
    for _ in 0..120 {
        p.clock.advance_ms(1000);
        let step = |actor: crate::ServerActor, store: Arc<Mutex<Option<ServerStore>>>| async move {
            actor
                .studio_receive_begin()
                .await
                .unwrap()
                .execute(StudioVaultLease::new(
                    store.try_lock_owned().expect("detached I/O releases vault"),
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
        let held = b_store.lock().await;
        done = verifier.sync.with_registry_context(|g, d, _, _| {
            let state = held
                .as_ref()
                .unwrap()
                .load_registry_epoch(SERVER, g, bucket, d)
                .unwrap();
            state.is_some_and(|s| {
                let projection = s.projection().unwrap();
                projection.pointers.get(&key) == Some(&0)
                    && projection.pointers.get(&other) == Some(&if large { 7 } else { 5 })
            })
        });
        if done {
            break;
        }
    }
    a.shutdown().await;
    b.shutdown().await;
    at.await.unwrap();
    bt.await.unwrap();
    ad.await.unwrap();
    bd.await.unwrap();
    assert!(
        done,
        "local derived pointer plus remote-only Registry tail must persist"
    );
    drop(b_store.lock().await.take());
    let reopened = open(p.b_root.path());
    verifier.sync.with_registry_context(|g, d, _, _| {
        let state = reopened
            .load_registry_epoch(SERVER, g, bucket, d)
            .unwrap()
            .unwrap();
        assert_eq!(
            state.projection().unwrap().pointers.get(&other),
            Some(&if large { 7 } else { 5 })
        );
        assert_eq!(
            reopened
                .load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .unwrap()
                .epoch(),
            0,
            "pointer hints cannot install or jump a Studio epoch"
        );
    });
}
