//! Succession reaches the native-facing actor worker with real observed MLS tenure evidence.
//! Only the old source/transition are fixtures; new receipts, recovery and pointers must be
//! produced by ordinary Read/Save and idle passes, including after loss of all volatile custody.
use super::actor_save::save;
use super::*;
use crate::studio::StudioVaultLease;
use catcoms_replication::registry::PointerKey;
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::{EpochPhase, InheritedCheckpoint};
use tokio::sync::Mutex;

#[tokio::test]
async fn studio_actor_new_owner_keeps_open_edits_and_rotates_after_restart() {
    successor(false).await;
}

#[tokio::test]
async fn studio_actor_new_owner_recovers_old_owner_closing_source_after_restart() {
    successor(true).await;
}

async fn successor(closing: bool) {
    let mut p = Pair::new().await;
    let logical = target().document(&p.bob.group_id()).unwrap();
    // This is the old owner's eligible history held by its successor. A non-owner cannot
    // fill the epoch alone: production per-author share limits still apply to the fixture.
    p.alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(&mut p.b_store, SERVER, g, d, target())
    });
    let original = p.state().unwrap().projection().unwrap();
    if closing {
        let old = p.alice.sync.with_registry_context(|g, d, _, _| {
            crate::store::studio_owner_decision_fixture(&p.b_store, SERVER, g, d, target())
        });
        let mut b = budget(&mut p.bob, &mut p.b_store);
        p.bob.sync.with_registry_context(|g, d, _, rng| {
            p.b_store
                .seal_studio_epoch(
                    SERVER,
                    g,
                    target(),
                    d,
                    old.receipt().clone(),
                    0,
                    rng,
                    &mut b,
                )
                .unwrap();
        });
        assert_eq!(p.state().unwrap().phase(), EpochPhase::Closing);
    }

    // The desktop does not expose founder removal. Use the existing protocol's staged Remove
    // solely to supply an independently observed transition, then restore the strict default
    // before any Studio actor runs. Never invent a tenure from a receipt or mutate raw MLS.
    let old_owner = p
        .alice
        .sync
        .with_registry_context(|_, d, _, _| d.device_id());
    assert_eq!(p.bob.sync.observed_owner_tenure_start(), None);
    p.bob.sync.set_config(catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..Default::default()
    });
    p.bob.sync.remove(&old_owner).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.bob.sync.set_config(catcoms_sync::SyncConfig::default());
    assert!(p.bob.is_owner());
    let new_owner_id = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    let tenure = p.bob.sync.observed_owner_tenure_start().unwrap();
    assert!(tenure > 0);

    let snapshot = p.bob.snapshot().unwrap();
    p.bob.sync.with_registry_context(|_, _, _, rng| {
        p.b_store.save_server(SERVER, &snapshot, rng).unwrap()
    });
    drop(p.b_store);
    let reopened = open(p.b_root.path());
    let restored = Node::restore(
        &reopened.load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(22))),
        rng(),
        Box::new(p.clock.clone()),
        "new owner after restart",
    )
    .unwrap();
    assert_eq!(restored.sync.observed_owner_tenure_start(), Some(tenure));
    let mut verifier = Node::restore(
        &snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(99))),
        rng(),
        Box::new(p.clock.clone()),
        "read-only verifier",
    )
    .unwrap();
    let store = Arc::new(Mutex::new(Some(reopened)));
    let (actor, mut events, task) = crate::spawn(restored);
    let drain = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            assert!(!matches!(event.event, crate::AppEvent::StudioReceivePaused));
        }
    });
    let before = save(&actor, &store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.epoch, 0);
    assert_eq!(before.projection, original);
    assert_eq!(
        before.phase,
        if closing {
            EpochPhase::Closing
        } else {
            EpochPhase::Open
        }
    );
    let own = title(91, "saved after succession");
    let (source_before, intents_before) = {
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        let source = verifier.sync.with_registry_context(|g, d, _, _| {
            held.load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .unwrap()
        });
        (source, held.load_epoch_intents(SERVER, &logical).unwrap())
    };
    assert!(!source_before
        .contains_exact_operation(new_owner_id, &own)
        .unwrap());
    assert!(!intents_before
        .pending()
        .any(|(id, _)| *id == own.id(&new_owner_id)));
    let saved = save(
        &actor,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: before.epoch_id,
            nonce: own.nonce,
            body: own.body.clone(),
        },
    )
    .await;
    // Inspect the durable result before any idle pass can settle or replay an intent.
    let mut expected = {
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        let source_after = verifier.sync.with_registry_context(|g, d, _, _| {
            held.load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .unwrap()
        });
        let intents_after = held.load_epoch_intents(SERVER, &logical).unwrap();
        assert_eq!(source_after.doc_id(), source_before.doc_id());
        assert_eq!(source_after.phase(), source_before.phase());
        if closing {
            assert_eq!(
                saved.expect_err("a takeover must not reopen the frozen source to edits"),
                format!(
                    "epoch studio: {}",
                    catcoms_replication::ReplError::EpochClosed
                )
            );
            assert_eq!(source_after.op_count(), source_before.op_count());
            assert_eq!(
                source_after.projection().unwrap(),
                source_before.projection().unwrap()
            );
            assert!(!source_after
                .contains_exact_operation(new_owner_id, &own)
                .unwrap());
            assert_eq!(
                intents_after.pending().collect::<Vec<_>>(),
                intents_before.pending().collect::<Vec<_>>(),
                "a rejected Closing Save must leave the intent journal unchanged"
            );
            assert!(!intents_after
                .pending()
                .any(|(id, _)| *id == own.id(&new_owner_id)));
            original.clone()
        } else {
            let saved = saved
                .expect("Open Save must succeed")
                .expect("Open Save must return a view");
            let StudioProjection::Flipnote(art) = &saved.projection else {
                panic!("expected a Flipnote projection")
            };
            let selected = &art.title.as_ref().expect("title must exist").selected;
            assert_eq!(selected.value.as_str(), "saved after succession");
            assert_eq!(selected.source.author, new_owner_id);
            assert_eq!(selected.source.nonce, own.nonce);
            assert_eq!(selected.source.op_id, own.id(&new_owner_id));
            assert!(source_after
                .contains_exact_operation(new_owner_id, &own)
                .unwrap());
            assert_eq!(source_after.op_count(), source_before.op_count() + 1);
            assert_eq!(source_after.projection().unwrap(), saved.projection);
            assert_eq!(
                intents_after.pending().len(),
                intents_before.pending().len() + 1
            );
            let (_, intent) = intents_after
                .pending()
                .find(|(id, _)| **id == own.id(&new_owner_id))
                .expect("the exact Save must be pending before settlement retires it");
            assert_eq!(intent.author, new_owner_id);
            assert_eq!(intent.operation, own);
            saved.projection
        }
    };
    let StudioProjection::Flipnote(art) = &mut expected else {
        panic!()
    };
    art.epoch = 1;
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    let mut installed = false;
    for _ in 0..60 {
        p.clock.advance_ms(1000);
        actor
            .studio_receive_begin()
            .await
            .unwrap()
            .execute(StudioVaultLease::new(
                store.clone().try_lock_owned().unwrap(),
                SERVER,
                (),
            ))
            .await
            .unwrap();
        actor.wait_studio_preparation().await;
        let held = store.lock().await;
        installed = verifier.sync.with_registry_context(|g, d, _, _| {
            let held = held.as_ref().unwrap();
            let source = held
                .load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .unwrap();
            let registry = held
                .load_registry_epoch(SERVER, g, pointer.bucket(), d)
                .unwrap();
            source.epoch() == 1
                && source.phase() == EpochPhase::Open
                && registry
                    .is_some_and(|r| r.projection().unwrap().pointers.get(&pointer) == Some(&1))
        });
        if installed {
            break;
        }
    }
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    assert!(
        installed,
        "ordinary actor idle passes must install the new-owner checkpoint and pointer"
    );

    let guard = store.lock().await;
    let held = guard.as_ref().unwrap();
    let journal = held.load_epoch_owner_receipts(SERVER, &logical).unwrap();
    assert!(
        journal.pending().is_none(),
        "solo availability completes without a peer query"
    );
    let receipt = journal.published().unwrap().clone();
    assert_eq!(receipt.tenure_start_group_epoch, tenure);
    assert_eq!(receipt.inherited, InheritedCheckpoint::EpochZero);
    verifier.sync.with_registry_context(|g, d, _, _| {
        receipt.verify_current_owner(g, tenure).unwrap();
        let state = held
            .load_studio_epoch(SERVER, g, target(), d)
            .unwrap()
            .unwrap();
        assert_eq!(state.projection().unwrap(), expected);
        assert_eq!(state.op_count(), 0);
        let recovery = held.load_epoch_recovery(SERVER, &logical).unwrap();
        if closing {
            let recovered = StudioRecovery::from_snapshot(
                recovery
                    .retained()
                    .next()
                    .expect("frozen source must survive takeover"),
                &logical,
                channel(),
            )
            .unwrap();
            assert_eq!(recovered.projection(), &original);
        } else {
            assert_eq!(
                held.load_epoch_intents(SERVER, &logical)
                    .unwrap()
                    .pending()
                    .len(),
                0,
                "the new receipt covers the own edit accepted after succession"
            );
        }
    });
    drop(guard);
    drop(store.lock().await.take());
    let reopened = open(p.b_root.path());
    let journal = reopened
        .load_epoch_owner_receipts(SERVER, &logical)
        .unwrap();
    assert_eq!(journal.published(), Some(&receipt));
    assert!(journal.pending().is_none());
    verifier.sync.with_registry_context(|g, d, _, _| {
        let state = reopened
            .load_studio_epoch(SERVER, g, target(), d)
            .unwrap()
            .unwrap();
        assert_eq!((state.epoch(), state.phase()), (1, EpochPhase::Open));
        assert_eq!(state.projection().unwrap(), expected);
        let registry = reopened
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.projection().unwrap().pointers.get(&pointer),
            Some(&1)
        );
        if closing {
            assert_eq!(
                reopened
                    .load_epoch_recovery(SERVER, &logical)
                    .unwrap()
                    .retained()
                    .len(),
                1
            );
        }
    });
}
