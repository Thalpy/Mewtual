use super::*;
use crate::studio::{StudioVaultLease, StudioView};
use crate::{spawn, ServerActor};
use tokio::sync::{oneshot, Mutex};

fn create() -> StudioRequest {
    StudioRequest::Create {
        channel: channel(),
        object: [7; 16],
        nonce: [1; 16],
        title: "shared moon".into(),
        ts: 1000,
    }
}
pub(super) async fn save(
    actor: &ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    request: StudioRequest,
) -> Result<Option<StudioView>, String> {
    actor
        .studio_begin(request)
        .await?
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
}

#[tokio::test]
async fn studio_actor_save_automatically_sends_both_create_records_and_real_frame() {
    let Pair {
        _a_root,
        b_root: _b_root,
        mut alice,
        mut bob,
        a_store,
        mut b_store,
        wire,
        watch,
        ..
    } = Pair::new().await;
    let index = StudioTarget::Index { channel: channel() };
    let index_watch = bob.watch_studio_epoch(&b_store, SERVER, index).unwrap();
    bob.flush_studio_subscriptions().await.unwrap();
    alice.set_blob_store(a_store.blob_store(&hex::encode(alice.group_id())).unwrap());
    let group = alice.group_id();
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let created = save(&actor, &store, create()).await.unwrap().unwrap();
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 2);
    let mut b = budget(&mut bob, &mut b_store);
    for selected in [&watch, &index_watch] {
        bob.sync_once().await.unwrap();
        assert_eq!(
            bob.receive_studio_step(&mut b_store, selected, &mut b)
                .unwrap()
                .unwrap()
                .admission,
            Admission::Accepted
        );
    }
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    let blob = actor.publish_pix(pix.clone(), None).await.unwrap();
    let body = FlipnoteOp::InsertFrame {
        frame: [4; 16],
        after: None,
        cid: *crate::Cid::from_hex(&blob.cid).unwrap().as_bytes(),
        bytes: pix.len() as u64,
    }
    .encode()
    .unwrap();
    let request = || StudioRequest::Apply {
        target: target(),
        epoch_id: created.epoch_id,
        nonce: [2; 16],
        body: body.clone(),
    };
    let saved = save(&actor, &store, request()).await.unwrap().unwrap();
    bob.sync_once().await.unwrap();
    let received = bob
        .receive_studio_step(&mut b_store, &watch, &mut b)
        .unwrap()
        .unwrap();
    assert_eq!(received.admission, Admission::Accepted);
    assert_eq!(received.state.projection().unwrap(), saved.projection);
    // Read is not a retry trigger. An exact Apply retry does send, but retires no local intent.
    save(&actor, &store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 3);
    save(&actor, &store, request()).await.unwrap();
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 4);
    bob.sync_once().await.unwrap();
    assert_eq!(
        bob.receive_studio_step(&mut b_store, &watch, &mut b)
            .unwrap()
            .unwrap()
            .admission,
        Admission::Duplicate
    );
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(SERVER, &target().document(&group).unwrap(),)
            .unwrap()
            .pending()
            .len(),
        2
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[derive(Debug)]
struct Released(Arc<AtomicBool>);
impl Drop for Released {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn studio_actor_save_cancellation_during_publication_keeps_save_and_releases_custody() {
    let Pair {
        _a_root,
        b_root: _b_root,
        alice,
        a_store,
        wire,
        ..
    } = Pair::new().await;
    let group = alice.group_id();
    wire.pause.store(true, Ordering::SeqCst);
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let ready = actor.studio_begin(create()).await.unwrap();
    let (cancel, signal) = tokio::sync::watch::channel(false);
    let released = Arc::new(AtomicBool::new(false));
    let lease = StudioVaultLease::new(store.clone().try_lock_owned().unwrap(), SERVER, ())
        .with_cancellation(RequestCancellation::new(
            signal,
            Some(Arc::new(Released(released.clone()))),
        ));
    let (result, ()) = tokio::join!(ready.execute(lease), async {
        wire.started.notified().await;
        assert!(
            store.try_lock().is_err(),
            "same source custody extends through send"
        );
        assert!(!released.load(Ordering::SeqCst));
        cancel.send_replace(true);
    });
    assert!(
        result.unwrap().is_some(),
        "cancellation does not undo durable Save"
    );
    assert_eq!(
        wire.attempts.load(Ordering::SeqCst),
        1,
        "second Create packet is suppressed"
    );
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(
        store
            .try_lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .load_epoch_intents(SERVER, &target().document(&group).unwrap(),)
            .unwrap()
            .pending()
            .len(),
        1
    );
    assert!(
        save(&actor, &store, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .is_some()
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_actor_create_uses_one_deadline_for_both_packets() {
    let Pair {
        _a_root,
        b_root: _b_root,
        alice,
        a_store,
        wire,
        clock,
        ..
    } = Pair::new().await;
    wire.controlled.store(true, Ordering::SeqCst);
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (result, ()) = tokio::join!(save(&actor, &store, create()), async {
        wire.started.notified().await;
        clock.advance_ms(1_500);
        wire.release.add_permits(1);
        wire.started.notified().await;
        clock.advance_ms(500);
        // If the second packet minted another two seconds, this test would never complete.
    });
    assert!(result.unwrap().is_some());
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 2);
    assert!(store.try_lock().is_ok());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_actor_precancelled_lease_saves_and_sends_nothing() {
    let Pair {
        _a_root,
        b_root: _b_root,
        alice,
        a_store,
        wire,
        ..
    } = Pair::new().await;
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let ready = actor.studio_begin(create()).await.unwrap();
    let (_cancel, signal) = tokio::sync::watch::channel(true);
    let lease = StudioVaultLease::new(store.clone().try_lock_owned().unwrap(), SERVER, ())
        .with_cancellation(RequestCancellation::new(signal, None));
    assert!(ready.execute(lease).await.is_err());
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 0);
    assert!(store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(SERVER)
        .is_err());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_actor_partial_create_keeps_object_but_sends_no_packets() {
    let Pair {
        _a_root,
        b_root: _b_root,
        mut alice,
        mut a_store,
        wire,
        ..
    } = Pair::new().await;
    alice
        .studio_transaction(
            &mut a_store,
            SERVER,
            StudioRequest::Create {
                channel: channel(),
                object: [8; 16],
                nonce: [8; 16],
                title: "prior".into(),
                ts: 1000,
            },
        )
        .unwrap();
    let index = StudioTarget::Index { channel: channel() };
    let mut b = budget(&mut alice, &mut a_store);
    // A genuine owner receipt closes just the index. The new object's own header can still
    // save, but the subsequent index write must fail. No automatic close production is claimed.
    alice.sync.with_registry_context(|g, d, _, r| {
        let held = a_store
            .load_studio_epoch(SERVER, g, index, d)
            .unwrap()
            .unwrap();
        let receipt = catcoms_replication::Receipt::sign(
            index.document(&g.group_id()).unwrap(),
            0,
            [9; 32],
            held.projection()
                .unwrap()
                .checkpoint([9; 32])
                .unwrap()
                .change_hash(),
            0,
            catcoms_replication::InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        a_store
            .seal_studio_epoch(SERVER, g, index, d, receipt, 0, r, &mut b)
            .unwrap();
    });
    let group = alice.group_id();
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    assert!(save(&actor, &store, create()).await.is_err());
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 0);
    assert!(
        save(&actor, &store, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(SERVER, &target().document(&group).unwrap(),)
            .unwrap()
            .pending()
            .len(),
        1
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_actor_save_without_subscribers_is_still_saved_and_retryable() {
    let Pair {
        _a_root,
        b_root: _b_root,
        alice,
        mut bob,
        a_store,
        wire,
        watch,
        ..
    } = Pair::new().await;
    bob.unwatch_studio_epoch(&watch).unwrap();
    bob.flush_studio_subscriptions().await.unwrap();
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let first = save(&actor, &store, create()).await.unwrap().unwrap();
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 2);
    let retry = save(&actor, &store, create()).await.unwrap().unwrap();
    assert_eq!(retry.projection, first.projection);
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 4);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_save_cancelled_after_commit_does_not_publish() {
    let Pair {
        _a_root,
        b_root: _b_root,
        mut alice,
        mut a_store,
        wire,
        ..
    } = Pair::new().await;
    let saved = alice
        .studio_transaction_with_publication(&mut a_store, SERVER, create())
        .unwrap();
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (_cancel, signal) = tokio::sync::watch::channel(true);
    let mut lease = StudioVaultLease::new(store.clone().try_lock_owned().unwrap(), SERVER, ())
        .with_cancellation(RequestCancellation::new(signal, None));
    let (reply, _receive) = oneshot::channel();
    let mut reply = crate::studio::StudioReply::Document(reply);
    assert!(alice
        .publish_studio_save(&mut lease, &mut reply, saved)
        .await
        .is_some());
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 0);
    drop(lease);
    assert!(alice
        .studio_transaction(
            store.lock().await.as_mut().unwrap(),
            SERVER,
            StudioRequest::Read { target: target() }
        )
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn studio_actor_publication_waiter_drop_and_actor_abort_preserve_saved_state() {
    for abort_actor in [false, true] {
        let Pair {
            _a_root,
            b_root: _b_root,
            alice,
            a_store,
            wire,
            ..
        } = Pair::new().await;
        wire.pause.store(true, Ordering::SeqCst);
        let group = alice.group_id();
        let store = Arc::new(Mutex::new(Some(a_store)));
        let (actor, mut events, task) = spawn(alice);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let ready = actor.studio_begin(create()).await.unwrap();
        let (_cancel, signal) = tokio::sync::watch::channel(false);
        let released = Arc::new(AtomicBool::new(false));
        let lease = StudioVaultLease::new(store.clone().try_lock_owned().unwrap(), SERVER, ())
            .with_cancellation(RequestCancellation::new(
                signal,
                Some(Arc::new(Released(released.clone()))),
            ));
        let waiter = tokio::spawn(ready.execute(lease));
        wire.started.notified().await;
        assert!(store.try_lock().is_err());
        assert!(!released.load(Ordering::SeqCst));
        if abort_actor {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(waiter.await.unwrap().is_err());
        } else {
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
            // Barrier proving the actor observed receiver closure and dropped its send future.
            assert_eq!(actor.member_count().await, 2);
            actor.shutdown().await;
            task.await.unwrap();
        }
        drain.await.unwrap();
        assert_eq!(wire.attempts.load(Ordering::SeqCst), 1);
        assert!(released.load(Ordering::SeqCst));
        let mut held = store.try_lock().unwrap();
        let disk = held.as_mut().unwrap();
        assert_eq!(
            disk.load_epoch_intents(SERVER, &target().document(&group).unwrap())
                .unwrap()
                .pending()
                .len(),
            1
        );
        let snapshot = disk.load_server(SERVER).unwrap();
        let mut restored = Server::restore(
            &snapshot,
            Net::new(Hub::new().join(PeerId::from_u64(1))),
            rng(),
            Box::new(ManualClock::new(5000)),
            "alice",
        )
        .unwrap();
        assert!(restored
            .studio_transaction(disk, SERVER, StudioRequest::Read { target: target() })
            .unwrap()
            .is_some());
    }
}
