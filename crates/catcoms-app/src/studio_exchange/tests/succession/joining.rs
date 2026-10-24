//! Historical legacy-group post-succession admission recycles the departed founder's low MLS
//! leaf and changes ownership AGAIN. Held PIX bytes remain fetchable, but a former owner's
//! checkpoint hint must not become current-owner installation authority. Authenticated P2P
//! policy currently refuses that admission; the separate negative case below preserves it.
use super::*;

#[tokio::test]
async fn studio_actor_post_succession_joiner_legacy_fetches_open_pixels_without_owner_confirmation()
{
    newcomer(false, false).await;
}

#[tokio::test]
async fn studio_actor_post_succession_joiner_legacy_fetches_closing_pixels_without_owner_confirmation(
) {
    newcomer(true, false).await;
}

#[tokio::test]
async fn studio_actor_post_succession_joiner_legacy_reads_open_history_provisionally() {
    newcomer(false, true).await;
}

#[tokio::test]
async fn studio_actor_post_succession_joiner_legacy_reads_closing_history_provisionally() {
    newcomer(true, true).await;
}

async fn newcomer(closing: bool, require_preview: bool) {
    let mut p = Pair::new_legacy().await;
    assert_eq!(p.alice.group_mode(), crate::GroupMode::LegacyUnverified);
    assert_eq!(p.bob.group_mode(), crate::GroupMode::LegacyUnverified);
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
    assert_eq!(p.bob.group_mode(), crate::GroupMode::LegacyUnverified);
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
    assert_eq!(provider.group_mode(), crate::GroupMode::LegacyUnverified);
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
    assert_eq!(provider.group_mode(), crate::GroupMode::LegacyUnverified);
    provider.set_blob_store(provider_store.blob_store(&group_key).unwrap());
    let invite = provider.mint_invite([44; 16], u64::MAX, vec![]).unwrap();
    assert!(
        invite.policy.is_none(),
        "legacy admission establishes no P2P pin"
    );
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
    assert_eq!(newcomer.group_mode(), crate::GroupMode::LegacyUnverified);
    let provider_peer = provider.local_peer();
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
    });
    let registry_before = registry_baseline(&mut newcomer, &newcomer_store, &logical);
    assert!(
        registry_before.is_none(),
        "fixture begins without a Registry bucket"
    );
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
    let hint = b.observed_studio_hint_for_test().expect(
        "authenticated Studio Hint must reach completed discovery before checking a preview",
    );
    assert_eq!(
        hint.target,
        catcoms_sync::checkpoint_exchange::CheckpointTarget::Studio(target())
    );
    assert_eq!(hint.peer, provider_peer);
    assert_eq!(hint.provider, new_owner);
    assert_eq!(hint.receipt.as_ref(), Some(&receipt));
    assert!(
        hint.proof_absent,
        "former-owner hint has no current-owner proof"
    );
    {
        let guard = b_store.lock().await;
        let held = guard.as_ref().unwrap();
        assert_unconfirmed(&mut newcomer_verifier, held, &logical, &registry_before);
        assert!(
            !held.blob_store(&group_key).unwrap().has(&cid),
            "metadata discovery must not invent pixel possession"
        );
    }
    let provisional_read = b
        .studio_begin(StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .execute_read(StudioVaultLease::new(
            b_store.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
        .unwrap()
        .map(|read| match read {
            crate::studio::StudioRead::AwaitingTenureReceipt(preview) => {
                assert!(preview.delivery().is_current());
                // Test-only snapshot for assertions after the native handoff has released.
                preview
                    .inspect(|epoch_id, projection| (epoch_id, projection.clone()))
                    .unwrap()
            }
            crate::studio::StudioRead::Document(_) => {
                panic!("a hint must never produce an ordinary view")
            }
        });
    {
        let guard = b_store.lock().await;
        assert_unconfirmed(
            &mut newcomer_verifier,
            guard.as_ref().unwrap(),
            &logical,
            &registry_before,
        );
    }
    if require_preview {
        b.clear_studio_previews();
        let cleared = b
            .studio_begin(StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .execute_read(StudioVaultLease::new(
                b_store.clone().try_lock_owned().unwrap(),
                SERVER,
                (),
            ))
            .await
            .unwrap();
        assert!(
            cleared.is_none(),
            "lock/reset must clear the actual actor preview before the next read"
        );
    }
    let refused = title(100, "a preview cannot authorize this Apply");
    let error = save(
        &b,
        &b_store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: successor_id,
            nonce: refused.nonce,
            body: refused.body,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "epoch studio: edit belongs to a retired epoch");
    {
        let guard = b_store.lock().await;
        assert_unconfirmed(
            &mut newcomer_verifier,
            guard.as_ref().unwrap(),
            &logical,
            &registry_before,
        );
    }
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
    assert_unconfirmed(
        &mut newcomer_verifier,
        &reopened,
        &logical,
        &registry_before,
    );
    // With no provider on this final network, a second fetch can only use persisted bytes.
    let mut offline = Node::restore(
        &newcomer_snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(97))),
        rng(),
        Box::new(clock.clone()),
        "offline reopened newcomer",
    )
    .unwrap();
    assert_eq!(offline.group_mode(), crate::GroupMode::LegacyUnverified);
    offline.set_blob_store(reopened.blob_store(&group_key).unwrap());
    assert_eq!(
        offline
            .request_blob_bounded(&cid, pix.len(), None)
            .await
            .unwrap(),
        Some(pix)
    );
    if require_preview {
        // The actor/native read type explicitly awaits tenure; all authority and byte guards
        // above remain unchanged after the real signed tail has entered the volatile preview.
        let read =
            provisional_read.expect("Gate 4 requires a provisional read of the hinted history");
        assert_eq!(read.0, successor_id);
        assert_eq!(read.1, saved.projection);
        assert_title(&read.1, new_owner, &tail, "pixels after owner succession");
        assert_frame(&read.1, &frame, old_owner, &cid, fetched.len());
    }
}

#[tokio::test]
async fn studio_actor_restored_p2p_successor_preserves_policy_and_refuses_unprovable_admission() {
    let mut p = Pair::new().await;
    let founder = p.alice.device_id();
    let successor = p.bob.device_id();
    let pin = p.bob.sync.group_policy().unwrap().clone();
    assert_eq!(p.alice.group_mode(), crate::GroupMode::PeerToPeer);
    assert_eq!(p.bob.group_mode(), crate::GroupMode::PeerToPeer);
    assert_eq!(pin.issuer(), founder);
    assert_eq!(p.bob.sync.observed_owner_tenure_start(), None);
    p.bob.open_channel(314).await.unwrap();
    p.bob
        .send_message(314, "history survives refused successor admission")
        .await
        .unwrap();
    p.bob.sync.set_config(catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..Default::default()
    });
    p.bob.sync.remove(&founder).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.bob.sync.set_config(catcoms_sync::SyncConfig::default());
    assert!(p.bob.is_owner());
    let tenure = p.bob.sync.observed_owner_tenure_start().unwrap();
    assert!(tenure > 0);
    let bytes = p.bob.snapshot().unwrap();
    p.b_store.save_server(SERVER, &bytes, &mut rng()).unwrap();
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

    let restored_store = open(b_root.path());
    let mut restored = Node::restore(
        &restored_store.load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock),
        "restored authenticated successor",
    )
    .unwrap();
    assert_eq!(restored.group_mode(), crate::GroupMode::PeerToPeer);
    assert_eq!(restored.sync.group_policy(), Some(&pin));
    assert_eq!(restored.sync.observed_owner_tenure_start(), Some(tenure));
    restored.sync.with_registry_context(|group, _, _, _| {
        assert_eq!(group.designated_committer(), Some(successor));
        assert_eq!(group.designated_committer_index(), Some(1));
        assert!(!group.contains_device(&founder));
        pin.verify_pin(group).unwrap();
        assert!(
            matches!(
                pin.verify_current_owner(group),
                Err(catcoms_mls::PolicyError::Unauthorized)
            ),
            "a saved founder pin is not current founder authority after removal"
        );
    });
    let before = restored.snapshot().unwrap();
    let epoch = restored.epoch();
    let version = restored.doc_version(catcoms_wire::DocType::Channel, 314);
    assert_eq!(restored.messages(314).len(), 1);
    assert_eq!(
        restored.messages(314)[0].text,
        "history survives refused successor admission"
    );
    assert!(matches!(
        restored.mint_invite([44; 16], u64::MAX, vec![]),
        Err(AppError::Sync(catcoms_sync::SyncError::Policy(
            catcoms_mls::PolicyError::AdmissionAuthorityUnavailable
        )))
    ));
    assert_eq!(restored.epoch(), epoch);
    assert_eq!(
        restored.doc_version(catcoms_wire::DocType::Channel, 314),
        version
    );
    assert_eq!(restored.sync.group_policy(), Some(&pin));
    assert_eq!(
        restored.snapshot().unwrap(),
        before,
        "refusal cannot mutate the MLS state, retained history or authenticated pin"
    );
}

pub(super) async fn step(actor: &crate::ServerActor, store: &Arc<Mutex<Option<ServerStore>>>) {
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

pub(super) fn drain_events(
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
    registry_before: &RegistryBaseline,
) {
    assert_eq!(
        &registry_baseline(verifier, store, logical),
        registry_before,
        "hint discovery, Read and refused Apply must preserve Registry state"
    );
    verifier.sync.with_registry_context(|g, d, _, _| {
        assert!(
            store
                .load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .is_none(),
            "an unconfirmed hint cannot install an authoritative epoch"
        );
    });
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    let registry =
        catcoms_replication::registry::registry_document(&verifier.group_id(), pointer.bucket())
            .unwrap();
    for document in [logical, &registry] {
        assert_empty_journals(store, document);
    }
}

fn assert_empty_journals(store: &ServerStore, logical: &catcoms_replication::LogicalDocument) {
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
    let recovery = store.load_epoch_recovery(SERVER, logical).unwrap();
    assert_eq!(recovery.retained().len(), 0);
    assert!(recovery.staged().is_none());
    assert!(recovery.eviction_pending().unwrap().is_none());
}

// Compare source identity, authority/admission state AND projection, not only the pointer's
// selected value. In this fixture the captured absence must survive discovery/read/refusal/reopen.
type RegistryBaseline = Option<(
    u128,
    u64,
    EpochPhase,
    usize,
    usize,
    catcoms_replication::registry::RegistryProjection,
)>;

fn registry_baseline(
    verifier: &mut Node,
    store: &ServerStore,
    logical: &catcoms_replication::LogicalDocument,
) -> RegistryBaseline {
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    verifier.sync.with_registry_context(|g, d, _, _| {
        store
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .map(|state| {
                (
                    state.doc_id(),
                    state.epoch(),
                    state.phase(),
                    state.op_count(),
                    state.quarantined_len(),
                    state.projection().unwrap(),
                )
            })
    })
}
