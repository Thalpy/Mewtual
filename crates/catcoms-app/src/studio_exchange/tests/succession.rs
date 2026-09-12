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
    successor(target(), false, false).await;
}

#[tokio::test]
async fn studio_actor_new_owner_recovers_old_owner_closing_source_after_restart() {
    successor(target(), false, true).await;
}

#[tokio::test]
async fn studio_actor_new_owner_rotates_open_index_after_restart() {
    successor(StudioTarget::Index { channel: channel() }, false, false).await;
}

#[tokio::test]
async fn studio_actor_new_owner_recovers_closing_index_after_restart() {
    successor(StudioTarget::Index { channel: channel() }, false, true).await;
}

#[tokio::test]
async fn studio_actor_new_owner_inherits_open_flipnote_checkpoint_after_restart() {
    successor(target(), true, false).await;
}

#[tokio::test]
async fn studio_actor_new_owner_inherits_closing_flipnote_checkpoint_after_restart() {
    successor(target(), true, true).await;
}

#[tokio::test]
async fn studio_actor_new_owner_inherits_open_index_checkpoint_after_restart() {
    successor(StudioTarget::Index { channel: channel() }, true, false).await;
}

#[tokio::test]
async fn studio_actor_new_owner_inherits_closing_index_checkpoint_after_restart() {
    successor(StudioTarget::Index { channel: channel() }, true, true).await;
}

async fn successor(selected_target: StudioTarget, checkpoint: bool, closing: bool) {
    let mut p = Pair::new().await;
    let logical = selected_target.document(&p.bob.group_id()).unwrap();
    // Only the PREVIOUS owner's checkpoint is a fixture. The new tenure must derive its
    // inherited close and seed from this installed opening, even if a later old close seals it.
    let opening = if checkpoint {
        let (receipt, seed, _) = super::discovery::prepared_checkpoint(&mut p, selected_target);
        let mut b = budget(&mut p.bob, &mut p.b_store);
        p.bob.sync.with_registry_context(|g, d, _, rng| {
            let (outcome, _) = p
                .b_store
                .adopt_studio_checkpoint(
                    SERVER,
                    g,
                    selected_target,
                    d,
                    &receipt,
                    Some(seed.bytes()),
                    0,
                    &p.clock,
                    rng,
                    &mut b,
                )
                .unwrap();
            assert_eq!(outcome, crate::store::StudioAdoptionOutcome::Installed);
        });
        Some(receipt)
    } else {
        None
    };
    let initial_epoch = u64::from(checkpoint);
    let successor_epoch = initial_epoch + 1;
    let inherited = opening
        .as_ref()
        .map_or(InheritedCheckpoint::EpochZero, |receipt| {
            InheritedCheckpoint::Checkpoint {
                epoch: initial_epoch,
                close_record_hash: receipt.close_record_hash,
                seed_change_hash: receipt.seed_change_hash,
            }
        });
    // This is the old owner's eligible history held by its successor. A non-owner cannot
    // fill the epoch alone: production per-author share limits still apply to the fixture.
    p.alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(&mut p.b_store, SERVER, g, d, selected_target)
    });
    let original = source(&mut p, selected_target).projection().unwrap();
    if closing {
        let old = p.alice.sync.with_registry_context(|g, d, _, _| {
            crate::store::studio_owner_decision_fixture(
                &p.b_store,
                SERVER,
                g,
                d,
                selected_target,
                opening.as_ref(),
            )
        });
        let mut b = budget(&mut p.bob, &mut p.b_store);
        p.bob.sync.with_registry_context(|g, d, _, rng| {
            p.b_store
                .seal_studio_epoch(
                    SERVER,
                    g,
                    selected_target,
                    d,
                    old.receipt().clone(),
                    0,
                    rng,
                    &mut b,
                )
                .unwrap();
        });
        assert_eq!(source(&mut p, selected_target).phase(), EpochPhase::Closing);
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
    let before = save(
        &actor,
        &store,
        StudioRequest::Read {
            target: selected_target,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(before.epoch, initial_epoch);
    assert_eq!(before.projection, original);
    assert_eq!(
        before.phase,
        if closing {
            EpochPhase::Closing
        } else {
            EpochPhase::Open
        }
    );
    let own = edit(selected_target, 91, "saved after succession");
    let (source_before, intents_before) = {
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        let source = verifier.sync.with_registry_context(|g, d, _, _| {
            held.load_studio_epoch(SERVER, g, selected_target, d)
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
            target: selected_target,
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
            held.load_studio_epoch(SERVER, g, selected_target, d)
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
            assert_title(
                &saved.projection,
                new_owner_id,
                &own,
                "saved after succession",
            );
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
    match &mut expected {
        StudioProjection::Flipnote(art) => art.epoch = successor_epoch,
        StudioProjection::Index(index) => index.epoch = successor_epoch,
    }
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
                .load_studio_epoch(SERVER, g, selected_target, d)
                .unwrap()
                .unwrap();
            let registry = held
                .load_registry_epoch(SERVER, g, pointer.bucket(), d)
                .unwrap();
            source.epoch() == successor_epoch
                && source.phase() == EpochPhase::Open
                && registry.is_some_and(|r| {
                    r.projection().unwrap().pointers.get(&pointer) == Some(&successor_epoch)
                })
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
    assert_eq!(receipt.closed_epoch, initial_epoch);
    assert_eq!(receipt.inherited, inherited);
    let successor_id = catcoms_replication::epoch::epoch_id(
        logical.doc_type,
        &logical.logical_key,
        successor_epoch,
        &receipt.close_record_hash,
    );
    verifier.sync.with_registry_context(|g, d, _, _| {
        receipt.verify_current_owner(g, tenure).unwrap();
        let state = held
            .load_studio_epoch(SERVER, g, selected_target, d)
            .unwrap()
            .unwrap();
        assert_eq!(state.doc_id(), successor_id);
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
            .load_studio_epoch(SERVER, g, selected_target, d)
            .unwrap()
            .unwrap();
        assert_eq!(
            (state.epoch(), state.phase()),
            (successor_epoch, EpochPhase::Open)
        );
        assert_eq!(state.projection().unwrap(), expected);
        let registry = reopened
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.projection().unwrap().pointers.get(&pointer),
            Some(&successor_epoch)
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
    // Restore a NEW actor from the durable MLS snapshot and reopen the installed successor.
    // A read-only vault check cannot prove its local author binding or the subsequent Save.
    let restored = Node::restore(
        &reopened.load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(23))),
        rng(),
        Box::new(p.clock.clone()),
        "new owner editing after checkpoint restart",
    )
    .unwrap();
    assert_eq!(restored.sync.observed_owner_tenure_start(), Some(tenure));
    let store = Arc::new(Mutex::new(Some(reopened)));
    let (actor, mut events, task) = crate::spawn(restored);
    let drain = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            assert!(!matches!(event.event, crate::AppEvent::StudioReceivePaused));
        }
    });
    let read = save(
        &actor,
        &store,
        StudioRequest::Read {
            target: selected_target,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        (read.epoch_id, read.epoch, read.phase),
        (successor_id, successor_epoch, EpochPhase::Open)
    );
    assert_eq!(read.projection, expected);
    let after_restart = edit(
        selected_target,
        92,
        "saved through restored successor actor",
    );
    let saved = save(
        &actor,
        &store,
        StudioRequest::Apply {
            target: selected_target,
            epoch_id: read.epoch_id,
            nonce: after_restart.nonce,
            body: after_restart.body.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_title(
        &saved.projection,
        new_owner_id,
        &after_restart,
        "saved through restored successor actor",
    );
    assert_eq!(
        (saved.epoch_id, saved.epoch, saved.phase),
        (successor_id, successor_epoch, EpochPhase::Open)
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    drop(store.lock().await.take());
    let reopened = open(p.b_root.path());
    verifier.sync.with_registry_context(|g, d, _, _| {
        let state = reopened
            .load_studio_epoch(SERVER, g, selected_target, d)
            .unwrap()
            .unwrap();
        assert_eq!(
            (state.doc_id(), state.epoch(), state.phase()),
            (successor_id, successor_epoch, EpochPhase::Open)
        );
        assert_eq!(state.projection().unwrap(), saved.projection);
        assert_eq!(state.op_count(), 1);
        assert!(state
            .contains_exact_operation(new_owner_id, &after_restart)
            .unwrap());
        let intents = reopened.load_epoch_intents(SERVER, &logical).unwrap();
        assert_eq!(
            intents.pending().len(),
            1,
            "a new Open edit remains provisional after restart"
        );
        let (id, pending) = intents.pending().next().unwrap();
        assert_eq!(*id, after_restart.id(&new_owner_id));
        assert_eq!(pending.author, new_owner_id);
        assert_eq!(pending.operation, after_restart);
        let journal = reopened
            .load_epoch_owner_receipts(SERVER, &logical)
            .unwrap();
        assert_eq!(
            journal.published(),
            Some(&receipt),
            "Save must not invent another receipt"
        );
        assert!(journal.pending().is_none());
        assert_eq!(
            reopened
                .load_registry_epoch(SERVER, g, pointer.bucket(), d)
                .unwrap()
                .unwrap()
                .projection()
                .unwrap()
                .pointers
                .get(&pointer),
            Some(&successor_epoch)
        );
    });
}

fn source(p: &mut Pair, target: StudioTarget) -> EpochStudioState {
    p.bob.sync.with_registry_context(|g, d, _, _| {
        p.b_store
            .load_studio_epoch(SERVER, g, target, d)
            .unwrap()
            .unwrap()
    })
}

fn edit(target: StudioTarget, nonce: u8, text: &str) -> DomainOp {
    let body = match target {
        StudioTarget::Flipnote { .. } => title(nonce, text).body,
        StudioTarget::Index { .. } => IndexOp::SetTitle {
            object: [7; 16],
            title: text.into(),
        }
        .encode()
        .unwrap(),
    };
    domain(target, body, nonce)
}

fn assert_title(
    projection: &StudioProjection,
    author: catcoms_crypto::DeviceId,
    operation: &DomainOp,
    text: &str,
) {
    let (value, found_author, nonce, op_id) = match projection {
        StudioProjection::Flipnote(art) => {
            let selected = &art.title.as_ref().expect("title must exist").selected;
            (
                selected.value.as_str(),
                selected.source.author,
                selected.source.nonce,
                selected.source.op_id,
            )
        }
        StudioProjection::Index(index) => {
            let selected = &index
                .objects
                .get(&[7; 16])
                .expect("indexed object must exist")
                .title
                .selected;
            (
                selected.value.as_str(),
                selected.source.author,
                selected.source.nonce,
                selected.source.op_id,
            )
        }
    };
    assert_eq!(value, text);
    assert_eq!(found_author, author);
    assert_eq!(nonce, operation.nonce);
    assert_eq!(op_id, operation.id(&author));
}
