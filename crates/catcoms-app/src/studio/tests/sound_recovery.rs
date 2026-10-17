//! Real actor/control recovery: planning alone cannot prove that ordinary Save still enforces
//! held media, source revalidation, exact acknowledgement or durability after a fresh mount.
use super::*;
use crate::store::EpochRecoveryAction;
use catcoms_rt::MemNetwork;
use types::StudioRecovery;

type Node = Server<MemNetwork, ChaCha20Rng>;
fn package() -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    hex::decode(fixture["pixaHex"].as_str().unwrap()).unwrap()
}
fn recipe() -> FlipnoteOp {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-score-ops-v1.json"
    )))
    .unwrap();
    let body = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "set_patch")
        .unwrap()["body"]
        .as_str()
        .unwrap();
    FlipnoteOp::decode(body.as_bytes()).unwrap()
}
fn historical(server: &mut Node, operations: &[FlipnoteOp]) -> StudioProjection {
    server.sync.with_registry_context(|g, d, _, r| {
        let mut state = types::StudioEpoch::new(g, target(), d.device_id()).unwrap();
        for (n, operation) in operations.iter().enumerate() {
            state
                .edit_or_reseal(
                    d,
                    g,
                    r,
                    &domain(target(), [(30 + n) as u8; 16], operation.encode().unwrap()),
                    100,
                )
                .unwrap();
        }
        state.projection().unwrap()
    })
}
fn sound_item(projection: &StudioProjection, operation: &FlipnoteOp) -> StudioRecoveryItem {
    let StudioProjection::Flipnote(art) = projection else {
        panic!()
    };
    let selected = art
        .sound
        .records
        .values()
        .flat_map(|r| std::iter::once(&r.selected).chain(&r.conflicts))
        .find(|v| &v.value == operation)
        .unwrap();
    StudioRecoveryItem::SoundRecord {
        value: selected.source.op_id,
    }
}
fn stage(store: &mut ServerStore, projection: &StudioProjection, salt: u8) -> [u8; 32] {
    let snapshot = StudioRecovery::snapshot(
        projection,
        None,
        RecoveryReason::Excluded,
        [salt; 32],
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    let id = snapshot.id().unwrap();
    store
        .update_epoch_recovery(
            7,
            projection.document(),
            EpochRecoveryAction::Stage(snapshot),
            &ManualClock::new(123),
            &mut rng(),
        )
        .unwrap();
    id
}
async fn control(
    actor: &ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    action: StudioControlAction,
) -> Result<StudioControlResponse, String> {
    actor
        .studio_control_begin(StudioControlRequest {
            target: target(),
            action,
        })
        .await?
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            7,
            (),
        ))
        .await
}
async fn preview(
    actor: &ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    snapshot: [u8; 32],
    item: StudioRecoveryItem,
) -> StudioRecoveryPreview {
    let answer = control(
        actor,
        store,
        StudioControlAction::Preview {
            snapshot,
            item,
            mode: StudioRecoveryMode::Restore,
        },
    )
    .await
    .unwrap();
    let StudioControlResponse::Preview(preview) = answer else {
        panic!()
    };
    preview
}
fn echo(
    preview: &StudioRecoveryPreview,
    item: StudioRecoveryItem,
    nonce: u8,
) -> StudioRecoveryApply {
    StudioRecoveryApply {
        snapshot: preview.snapshot,
        item,
        mode: StudioRecoveryMode::Restore,
        epoch_id: preview.epoch_id,
        expected_projection: preview.fingerprint,
        nonce: [nonce; 16],
        body: preview.plan.body.clone().unwrap(),
    }
}

#[tokio::test]
async fn studio_sound_recovery_actor_save_restart_media_refusal_stale_preview_and_evicted_exact_retry(
) {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new();
    let mut store = ServerStore::open(dir.path(), b"sound-recovery", &mut rng()).unwrap();
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let group = server.group_id();
    server.set_blob_store(store.blob_store(&hex::encode(&group)).unwrap());
    let created = server
        .studio_transaction(&mut store, 7, create())
        .unwrap()
        .unwrap();
    let bytes = package();
    let pix = crate::creative::validate_pixa(&bytes).unwrap().frames()[0].to_vec();
    let pixel = server.publish_pix(&pix).unwrap();
    let insert = FlipnoteOp::InsertFrame {
        frame: [1; 16],
        after: None,
        cid: *crate::Cid::from_hex(&pixel.cid).unwrap().as_bytes(),
        bytes: pix.len() as u64,
    };
    server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::Apply {
                target: target(),
                epoch_id: created.epoch_id,
                nonce: [8; 16],
                body: insert.encode().unwrap(),
            },
        )
        .unwrap();
    server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::CreateScore {
                channel: channel(),
                object: [9; 16],
                nonce: [10; 16],
                title: "soundtrack".into(),
                ts: 123,
            },
        )
        .unwrap();
    let recipe = recipe();
    let FlipnoteOp::SetPatch { patch, .. } = recipe else {
        panic!()
    };
    let sfx = FlipnoteOp::SetSfx {
        sfx: [2; 16],
        frame: [1; 16],
        patch,
        note: 72,
        duration_ms: 375,
    };
    let cid = crate::Cid::of(&bytes);
    let export = FlipnoteOp::SetExport {
        export: [3; 16],
        cid: *cid.as_bytes(),
        bytes: bytes.len() as u64,
        expiry: StudioExpiry::Never,
    };
    let link = FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([9; 16])));
    let bad_package = b"PIXA-not-a-valid-package";
    let bad_cid = crate::Cid::of(bad_package);
    let bad_export = FlipnoteOp::SetExport {
        export: [4; 16],
        cid: *bad_cid.as_bytes(),
        bytes: bad_package.len() as u64,
        expiry: StudioExpiry::Never,
    };
    let score_path = server
        .sync
        .with_registry_context(|g, _, _, _| {
            store.studio_source_path_for_test(
                7,
                g,
                StudioTarget::Score {
                    channel: channel(),
                    object: [9; 16],
                },
            )
        })
        .unwrap();
    let old = historical(
        &mut server,
        &[
            insert,
            recipe.clone(),
            sfx.clone(),
            export.clone(),
            link.clone(),
            bad_export.clone(),
        ],
    );
    let snapshot = stage(&mut store, &old, 1);
    let logical = old.document().clone();
    let store = Arc::new(Mutex::new(Some(store)));
    let (actor, mut events, task) = spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let si = sound_item(&old, &sfx);
    assert_eq!(
        preview(&actor, &store, snapshot, si).await.plan.disposition,
        StudioRecoveryDisposition::MissingTarget
    );
    let ri = sound_item(&old, &recipe);
    let p = preview(&actor, &store, snapshot, ri).await;
    assert_eq!(p.plan.disposition, StudioRecoveryDisposition::Ready);
    control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&p, ri, 50))),
    )
    .await
    .unwrap();
    let stale = preview(&actor, &store, snapshot, si).await;
    run(
        &actor,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: stale.epoch_id,
            nonce: [51; 16],
            body: FlipnoteOp::SetHeader(FlipnoteHeader::Fps(13))
                .encode()
                .unwrap(),
        },
    )
    .await
    .unwrap();
    let before = store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_epoch_intents(7, &logical)
        .unwrap()
        .pending()
        .len();
    assert!(control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&stale, si, 52)))
    )
    .await
    .unwrap_err()
    .contains("stale"));
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(7, &logical)
            .unwrap()
            .pending()
            .len(),
        before
    );
    let p = preview(&actor, &store, snapshot, si).await;
    control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&p, si, 53))),
    )
    .await
    .unwrap();
    let ei = sound_item(&old, &export);
    let p = preview(&actor, &store, snapshot, ei).await;
    let before = store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_epoch_intents(7, &logical)
        .unwrap()
        .pending()
        .len();
    assert!(
        control(
            &actor,
            &store,
            StudioControlAction::Apply(Box::new(echo(&p, ei, 54)))
        )
        .await
        .is_err(),
        "unheld package refuses before intent"
    );
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(7, &logical)
            .unwrap()
            .pending()
            .len(),
        before
    );
    assert_eq!(
        actor.publish_pixa(bytes.clone(), None).await.unwrap().cid,
        cid.to_hex()
    );
    let export_echo = echo(&p, ei, 54);
    control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&p, ei, 54))),
    )
    .await
    .unwrap();
    let li = sound_item(&old, &link);
    let p = preview(&actor, &store, snapshot, li).await;
    // Revalidate the actual independently authenticated linked source after a Ready preview.
    let hidden_score = score_path.with_extension("temporarily-missing");
    std::fs::rename(&score_path, &hidden_score).unwrap();
    assert!(control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&p, li, 55)))
    )
    .await
    .is_err());
    assert_eq!(
        preview(&actor, &store, snapshot, li).await.plan.disposition,
        StudioRecoveryDisposition::MissingTarget
    );
    std::fs::rename(&hidden_score, &score_path).unwrap();
    control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&p, li, 55))),
    )
    .await
    .unwrap();
    let bi = sound_item(&old, &bad_export);
    let bad_preview = preview(&actor, &store, snapshot, bi).await;
    assert_eq!(
        bad_preview.plan.disposition,
        StudioRecoveryDisposition::Ready
    );
    let before_bad = store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_epoch_intents(7, &logical)
        .unwrap()
        .pending()
        .len();
    store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .blob_store(&hex::encode(&group))
        .unwrap()
        .put(bad_package)
        .unwrap();
    assert!(control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(echo(&bad_preview, bi, 56)))
    )
    .await
    .unwrap_err()
    .contains("invalid PIXA"));
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(7, &logical)
            .unwrap()
            .pending()
            .len(),
        before_bad
    );
    let saved = run(&actor, &store, read()).await.unwrap().unwrap();
    let StudioProjection::Flipnote(ref art) = saved.projection else {
        panic!()
    };
    assert_eq!(art.sound.sfx().len(), 1);
    assert_eq!(art.sound.exports().len(), 1);
    assert_eq!(art.sound.patches().len(), 1);
    assert!(store
        .lock()
        .await
        .as_mut()
        .unwrap()
        .creative_pinned_cids()
        .unwrap()
        .for_group(&group)
        .any(|held| held == &cid));
    // Evict the selected version, then simulate external media loss. Exact accepted-own retry
    // must traverse the real receiver and ordinary Save without requiring either obsolete input.
    {
        let mut guard = store.lock().await;
        let store = guard.as_mut().unwrap();
        stage(store, &old, 2);
        stage(store, &old, 3);
        let RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot,
            ..
        } = store
            .load_epoch_recovery(7, &logical)
            .unwrap()
            .eviction_pending()
            .unwrap()
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(oldest_snapshot, snapshot);
        store
            .update_epoch_recovery(
                7,
                &logical,
                EpochRecoveryAction::Acknowledge {
                    oldest_snapshot,
                    staged_snapshot,
                },
                &ManualClock::new(123),
                &mut rng(),
            )
            .unwrap();
    }
    std::fs::remove_file(
        dir.path()
            .join("blobs")
            .join(hex::encode(&group))
            .join(cid.to_hex()),
    )
    .unwrap();
    let answer = control(
        &actor,
        &store,
        StudioControlAction::Apply(Box::new(export_echo)),
    )
    .await
    .unwrap();
    assert!(matches!(
        answer,
        StudioControlResponse::Applied {
            already_saved: true,
            ..
        }
    ));
    assert_eq!(
        run(&actor, &store, read())
            .await
            .unwrap()
            .unwrap()
            .projection,
        saved.projection
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    let vault = store.lock().await.as_ref().unwrap().load_server(7).unwrap();
    drop(store.lock().await.take());
    let mut reopened = ServerStore::open(dir.path(), b"sound-recovery", &mut rng()).unwrap();
    let mut restored = Server::restore(
        &vault,
        hub.join(PeerId::from_u64(1)),
        rng(),
        Box::new(ManualClock::new(999)),
        "alice",
    )
    .unwrap();
    restored.set_blob_store(reopened.blob_store(&hex::encode(&group)).unwrap());
    assert_eq!(
        restored
            .studio_transaction(&mut reopened, 7, read())
            .unwrap()
            .unwrap()
            .projection,
        saved.projection
    );
    assert!(reopened
        .creative_pinned_cids()
        .unwrap()
        .for_group(&group)
        .any(|held| held == &cid));
}

#[tokio::test]
async fn studio_sound_link_recovery_and_fresh_save_reject_wrong_family_channel_and_group_before_intent(
) {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new();
    let mut store = ServerStore::open(dir.path(), b"sound-link", &mut rng()).unwrap();
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let created = server
        .studio_transaction(&mut store, 7, create())
        .unwrap()
        .unwrap();
    server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::Create {
                channel: channel(),
                object: [9; 16],
                nonce: [10; 16],
                title: "not a score".into(),
                ts: 123,
            },
        )
        .unwrap();
    server.open_channel_index().await.unwrap();
    let other = server
        .create_channel("other")
        .await
        .unwrap()
        .id
        .to_be_bytes();
    server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::CreateScore {
                channel: other,
                object: [10; 16],
                nonce: [11; 16],
                title: "different channel".into(),
                ts: 123,
            },
        )
        .unwrap();
    let mut foreign = Server::found(
        hub.join(PeerId::from_u64(2)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "foreign",
    )
    .unwrap();
    foreign
        .studio_transaction(
            &mut store,
            8,
            StudioRequest::CreateScore {
                channel: channel(),
                object: [11; 16],
                nonce: [12; 16],
                title: "different group".into(),
                ts: 123,
            },
        )
        .unwrap();
    let logical = target().document(&server.group_id()).unwrap();
    for (n, id) in [9, 10, 11].into_iter().enumerate() {
        let link = FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([id; 16])));
        let old = historical(&mut server, std::slice::from_ref(&link));
        let snapshot = stage(&mut store, &old, id);
        let answer = server
            .studio_control_transaction(
                &mut store,
                7,
                StudioControlRequest {
                    target: target(),
                    action: StudioControlAction::Preview {
                        snapshot,
                        item: sound_item(&old, &link),
                        mode: StudioRecoveryMode::Restore,
                    },
                },
            )
            .unwrap();
        let StudioControlResponse::Preview(p) = answer else {
            panic!()
        };
        assert_eq!(p.plan.disposition, StudioRecoveryDisposition::MissingTarget);
        assert!(p.plan.source_ops.is_empty());
        let before = store
            .load_epoch_intents(7, &logical)
            .unwrap()
            .pending()
            .len();
        assert!(server
            .studio_transaction(
                &mut store,
                7,
                StudioRequest::Apply {
                    target: target(),
                    epoch_id: created.epoch_id,
                    nonce: [(60 + n) as u8; 16],
                    body: link.encode().unwrap()
                }
            )
            .is_err());
        assert_eq!(
            store
                .load_epoch_intents(7, &logical)
                .unwrap()
                .pending()
                .len(),
            before
        );
    }
}
