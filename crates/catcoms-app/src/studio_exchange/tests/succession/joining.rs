//! A genuine post-succession join recycles the departed founder's low MLS leaf and changes
//! ownership AGAIN. Held PIX bytes remain fetchable, but a former owner's checkpoint hint must
//! not become current-owner installation authority. The ignored acceptance cases expose the
//! missing provisional metadata read without relaxing that boundary.
use super::*;

#[tokio::test]
async fn studio_actor_post_succession_joiner_fetches_open_source_pixels_without_owner_confirmation()
{
    newcomer(false, false).await;
}

#[tokio::test]
async fn studio_actor_post_succession_joiner_fetches_closing_source_pixels_without_owner_confirmation(
) {
    newcomer(true, false).await;
}

#[tokio::test]
#[ignore = "Gate 4: provisional reads of former-owner checkpoint hints are not implemented"]
async fn studio_actor_post_succession_joiner_reads_open_history_provisionally() {
    newcomer(false, true).await;
}

#[tokio::test]
#[ignore = "Gate 4: provisional reads of former-owner checkpoint hints are not implemented"]
async fn studio_actor_post_succession_joiner_reads_closing_history_provisionally() {
    newcomer(true, true).await;
}

async fn newcomer(closing: bool, require_preview: bool) {
    let mut p = Pair::new().await;
    let logical = target().document(&p.bob.group_id()).unwrap();
    let group_key = hex::encode(p.bob.group_id());
    let old_owner = p
        .alice
        .sync
        .with_registry_context(|_, d, _, _| d.device_id());
    let new_owner = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    p.alice
        .set_blob_store(p.a_store.blob_store(&group_key).unwrap());
    p.bob
        .set_blob_store(p.b_store.blob_store(&group_key).unwrap());
    let published = p.alice.publish_pix(&pix).unwrap();
    assert_eq!(published.bytes, pix.len());
    let cid = crate::Cid::from_hex(&published.cid).unwrap();
    let frame = domain(
        target(),
        FlipnoteOp::InsertFrame {
            frame: [4; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pix.len() as u64,
        }
        .encode()
        .unwrap(),
        81,
    );
    p.save(&frame);
    p.send(frame.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert_eq!(p.receive().unwrap().unwrap().admission, Admission::Accepted);
    assert!(!p.b_store.blob_store(&group_key).unwrap().has(&cid));
    let (fetched, tick) = tokio::join!(
        p.bob.request_blob_bounded(&cid, pix.len(), None),
        p.alice.sync_once()
    );
    tick.unwrap();
    assert_eq!(fetched.unwrap(), Some(pix.clone()));
    assert!(p.b_store.blob_store(&group_key).unwrap().has(&cid));

    // Only eligible old-owner history and the optional interrupted old seal are prepared.
    // The actual frame was saved, sent and fetched through the existing adapters above.
    p.alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(&mut p.b_store, SERVER, g, d, target())
    });
    let original = source(&mut p, target()).projection().unwrap();
    if closing {
        let old = p.alice.sync.with_registry_context(|g, d, _, _| {
            crate::store::studio_owner_decision_fixture(&p.b_store, SERVER, g, d, target(), None)
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
    }
    assert_eq!(
        source(&mut p, target()).phase(),
        if closing {
            EpochPhase::Closing
        } else {
            EpochPhase::Open
        }
    );
    // Same observed-transition fixture as the reviewed matrix; strict policy is restored
    // before Studio runs. This is not a desktop founder-transfer feature.
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
    let tenure = p.bob.sync.observed_owner_tenure_start().unwrap();
    assert!(tenure > 0);
    let snapshot = p.bob.snapshot().unwrap();
    p.bob.sync.with_registry_context(|_, _, _, rng| {
        p.b_store.save_server(SERVER, &snapshot, rng).unwrap()
    });
    let Pair {
        b_root,
        b_store,
        clock,
        alice,
        bob,
        ..
    } = p;
    drop(alice);
    drop(bob);
    drop(b_store);
    // Both restarts use a fresh network. No departed owner's endpoint can answer a query.
    let restored_store = open(b_root.path());
    let mut provider = Node::restore(
        &restored_store.load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "successor",
    )
    .unwrap();
    provider.set_blob_store(restored_store.blob_store(&group_key).unwrap());
    let mut verifier = Node::restore(
        &snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(99))),
        rng(),
        Box::new(clock.clone()),
        "read-only successor verifier",
    )
    .unwrap();
    let store = Arc::new(Mutex::new(Some(restored_store)));
    let (actor, events, task) = crate::spawn(provider);
    let drained = drain_events(events);
    let before = save(&actor, &store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.epoch, 0);
    assert_eq!(before.projection, original);
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    let mut installed = false;
    for _ in 0..60 {
        clock.advance_ms(1000);
        step(&actor, &store).await;
        actor.wait_studio_preparation().await;
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        installed = verifier.sync.with_registry_context(|g, d, _, _| {
            let state = held
                .load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .unwrap();
            state.epoch() == 1
                && state.phase() == EpochPhase::Open
                && held
                    .load_registry_epoch(SERVER, g, pointer.bucket(), d)
                    .unwrap()
                    .is_some_and(|r| r.projection().unwrap().pointers.get(&pointer) == Some(&1))
        });
        if installed {
            break;
        }
    }
    assert!(
        installed,
        "the successor must issue/install its checkpoint and pointer before joining"
    );
    let receipt = {
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        let journal = held.load_epoch_owner_receipts(SERVER, &logical).unwrap();
        assert!(journal.pending().is_none());
        let receipt = journal.published().unwrap().clone();
        assert_eq!(receipt.closed_epoch, 0);
        assert_eq!(receipt.tenure_start_group_epoch, tenure);
        assert_eq!(receipt.inherited, InheritedCheckpoint::EpochZero);
        verifier.sync.with_registry_context(|g, _, _, _| {
            receipt.verify_current_owner(g, tenure).unwrap();
        });
        if closing {
            let recovery = held.load_epoch_recovery(SERVER, &logical).unwrap();
            let recovered = StudioRecovery::from_snapshot(
                recovery.retained().next().expect("frozen source retained"),
                &logical,
                channel(),
            )
            .unwrap();
            assert_eq!(recovered.projection(), &original);
        }
        receipt
    };
    let successor_id = catcoms_replication::epoch::epoch_id(
        logical.doc_type,
        &logical.logical_key,
        1,
        &receipt.close_record_hash,
    );
    // A real current-tail edit must reach the newcomer as well as the receipt-bound baseline.
    let tail = title(99, "pixels after owner succession");
    let saved = save(
        &actor,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: successor_id,
            nonce: tail.nonce,
            body: tail.body.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_title(
        &saved.projection,
        new_owner,
        &tail,
        "pixels after owner succession",
    );
    assert_frame(&saved.projection, &frame, old_owner, &cid, pix.len());
    let provider_snapshot = actor.snapshot().await.unwrap();
    actor.shutdown().await;
    task.await.unwrap();
    drained.await.unwrap();
    {
        let mut guard = store.lock().await;
        let held = guard.as_mut().unwrap();
        verifier.sync.with_registry_context(|_, _, _, rng| {
            held.save_server(SERVER, &provider_snapshot, rng).unwrap()
        });
        drop(guard.take());
    }

    // Drop every actor/cache, reopen the sealed vault, then admit an independent device.
    let provider_store = open(b_root.path());
    assert!(provider_store.blob_store(&group_key).unwrap().has(&cid));
    verifier.sync.with_registry_context(|g, d, _, _| {
        let state = provider_store
            .load_studio_epoch(SERVER, g, target(), d)
            .unwrap()
            .unwrap();
        assert_eq!(
            (state.doc_id(), state.epoch(), state.phase()),
            (successor_id, 1, EpochPhase::Open)
        );
        assert_eq!(state.projection().unwrap(), saved.projection);
        assert_eq!(state.op_count(), 1);
        assert!(state.contains_exact_operation(new_owner, &tail).unwrap());
        let registry = provider_store
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.projection().unwrap().pointers.get(&pointer),
            Some(&1)
        );
    });
    assert_eq!(
        provider_store
            .load_epoch_owner_receipts(SERVER, &logical)
            .unwrap()
            .published(),
        Some(&receipt)
    );
    let hub = Hub::new();
    let mut provider = Node::restore(
        &provider_store.load_server(SERVER).unwrap(),
        Net::new(hub.join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "restarted successor",
    )
    .unwrap();
    assert_eq!(provider.sync.observed_owner_tenure_start(), Some(tenure));
    provider.set_blob_store(provider_store.blob_store(&group_key).unwrap());
    let invite = provider.mint_invite([44; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            Net::new(hub.join(PeerId::from_u64(3))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "post-succession newcomer",
            provider.local_peer(),
            &invite
        ),
        provider.sync_once()
    );
    tick.unwrap();
    let mut newcomer = joined.unwrap();
    // Adding into Alice's recycled leaf changes Bob -> newcomer. A Welcome is not an
    // independently witnessed tenure transition, even when it makes its recipient owner.
    assert!(newcomer.is_owner());
    assert!(!provider.is_owner());
    assert_eq!(newcomer.sync.observed_owner_tenure_start(), None);
    assert!(provider.sync.observed_owner_tenure_start().unwrap() > tenure);
    let newcomer_id = newcomer
        .sync
        .with_registry_context(|_, d, _, _| d.device_id());
    assert_ne!(newcomer_id, old_owner);
    assert_ne!(newcomer_id, new_owner);
    let (proof, tick) = tokio::join!(
        newcomer.request_channel_index_catchup(provider.local_peer()),
        provider.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let newcomer_snapshot = newcomer.snapshot().unwrap();
    let root = tempfile::tempdir().unwrap();
    let newcomer_store = open(root.path());
    newcomer.sync.with_registry_context(|g, d, _, _| {
        assert!(newcomer_store
            .load_studio_epoch(SERVER, g, target(), d)
            .unwrap()
            .is_none());
        assert!(newcomer_store
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .is_none());
    });
    newcomer.set_blob_store(newcomer_store.blob_store(&group_key).unwrap());
    assert!(!newcomer_store.blob_store(&group_key).unwrap().has(&cid));
    let mut newcomer_verifier = Node::restore(
        &newcomer_snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(98))),
        rng(),
        Box::new(clock.clone()),
        "read-only newcomer verifier",
    )
    .unwrap();
    let a_store = Arc::new(Mutex::new(Some(provider_store)));
    let b_store = Arc::new(Mutex::new(Some(newcomer_store)));
    let (a, ae, at) = crate::spawn(provider);
    let (b, be, bt) = crate::spawn(newcomer);
    let ad = drain_events(ae);
    let bd = drain_events(be);
    assert!(save(&b, &b_store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .is_none());
    // No provider Read, no repeated newcomer reads and no direct receiver/install helper.
    // The worker may learn a former-owner hint, but has no current-owner proof for it.
    for _ in 0..100 {
        clock.advance_ms(1000);
        tokio::join!(step(&a, &a_store), step(&b, &b_store));
        tokio::join!(a.wait_studio_preparation(), b.wait_studio_preparation());
    }
    {
        let guard = b_store.lock().await;
        let held = guard.as_ref().unwrap();
        assert_unconfirmed(&mut newcomer_verifier, held, &logical);
        assert!(
            !held.blob_store(&group_key).unwrap().has(&cid),
            "metadata discovery must not invent pixel possession"
        );
    }
    let provisional_read = save(&b, &b_store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    // The CID is supplied by this fixture, not discovered by the newcomer. Availability is
    // useful evidence, but does not claim that the current app can display this Flipnote.
    let fetched = b
        .request_blob_bounded(cid, pix.len(), None)
        .await
        .unwrap()
        .expect("restarted successor must serve held PIX bytes");
    assert_eq!(fetched.len(), pix.len());
    assert_eq!(fetched, pix);
    crate::creative::validate_pix(&fetched).unwrap();
    assert!(
        a.files().await.is_empty() && b.files().await.is_empty(),
        "PIX availability does not require a fileshare listing"
    );
    a.shutdown().await;
    b.shutdown().await;
    at.await.unwrap();
    bt.await.unwrap();
    ad.await.unwrap();
    bd.await.unwrap();
    drop(b_store.lock().await.take());
    let reopened = open(root.path());
    assert_unconfirmed(&mut newcomer_verifier, &reopened, &logical);
    // With no provider on this final network, a second fetch can only use persisted bytes.
    let mut offline = Node::restore(
        &newcomer_snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(97))),
        rng(),
        Box::new(clock.clone()),
        "offline reopened newcomer",
    )
    .unwrap();
    offline.set_blob_store(reopened.blob_store(&group_key).unwrap());
    assert_eq!(
        offline
            .request_blob_bounded(&cid, pix.len(), None)
            .await
            .unwrap(),
        Some(pix)
    );
    if require_preview {
        // This is an opt-in, currently failing gate-acceptance oracle. Even once a preview is
        // implemented, the assertions above still prohibit promoting a hint into authority.
        let read =
            provisional_read.expect("Gate 4 requires a provisional read of the hinted history");
        assert_eq!(read.epoch_id, successor_id);
        assert_eq!(read.projection, saved.projection);
        assert_title(
            &read.projection,
            new_owner,
            &tail,
            "pixels after owner succession",
        );
        assert_frame(&read.projection, &frame, old_owner, &cid, fetched.len());
    }
}

async fn step(actor: &crate::ServerActor, store: &Arc<Mutex<Option<ServerStore>>>) {
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
}

fn drain_events(
    mut events: tokio::sync::mpsc::Receiver<crate::TracedEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            assert!(!matches!(event.event, crate::AppEvent::StudioReceivePaused));
        }
    })
}

fn assert_frame(
    projection: &StudioProjection,
    frame: &DomainOp,
    author: catcoms_crypto::DeviceId,
    cid: &crate::Cid,
    bytes: usize,
) {
    let StudioProjection::Flipnote(art) = projection else {
        panic!("expected Flipnote")
    };
    assert_eq!(art.timeline, vec![[4; 16]]);
    assert_eq!(art.frames.len(), 1);
    assert_eq!(art.declared_frame_bytes, bytes as u64);
    let entry = &art.frames[&[4; 16]];
    assert!(entry.insertions[0].value.checkpoint);
    let pixels = &entry.pixels.selected;
    assert_eq!(pixels.value.cid, *cid.as_bytes());
    assert_eq!(pixels.value.bytes, bytes as u64);
    assert_eq!(pixels.source.author, author);
    assert_eq!(pixels.source.nonce, frame.nonce);
    assert_eq!(pixels.source.op_id, frame.id(&author));
}

fn assert_unconfirmed(
    verifier: &mut Node,
    store: &ServerStore,
    logical: &catcoms_replication::LogicalDocument,
) {
    verifier.sync.with_registry_context(|g, d, _, _| {
        assert!(
            store
                .load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .is_none(),
            "an unconfirmed hint cannot install an authoritative epoch"
        );
    });
    let journal = store.load_epoch_owner_receipts(SERVER, logical).unwrap();
    assert!(journal.pending().is_none() && journal.published().is_none());
    assert_eq!(
        store
            .load_epoch_intents(SERVER, logical)
            .unwrap()
            .pending()
            .len(),
        0
    );
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, logical)
            .unwrap()
            .retained()
            .len(),
        0
    );
}
