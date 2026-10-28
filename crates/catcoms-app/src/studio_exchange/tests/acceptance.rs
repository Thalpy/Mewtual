//! Gate 7 composes the production actor/vault seams instead of installing a hand-made receipt
//! or recovery snapshot. The memory transport is deliberate: socket coverage is separate.
use super::*;
use crate::studio::{
    StudioControlAction as Action, StudioControlRequest, StudioControlResponse as Response,
    StudioRecoveryApply, StudioRecoveryDisposition as Disposition, StudioRecoveryItem as Item,
    StudioRecoveryMode as Mode, StudioVaultLease,
};
use catcoms_replication::studio::{ScoreCell, ScoreOp, StudioPatch};
use catcoms_replication::EpochPhase;
use tokio::sync::Mutex;

/// Independently authored semantic expectations. Peer agreement alone cannot detect a compact
/// seed which lost the same live content at both endpoints; provenance may legitimately change.
struct Composition {
    pixel: crate::Cid,
    pixel_bytes: u64,
    recipe: StudioPatch,
    package: crate::Cid,
    package_bytes: u64,
}
impl Composition {
    fn assert_preserved(&self, projection: &StudioProjection, fps: u8) {
        let StudioProjection::Flipnote(art) = projection else {
            panic!("expected art")
        };
        assert_eq!(art.timeline, vec![[4; 16]]);
        assert_eq!(
            art.frames[&[4; 16]].pixels.selected.value.cid,
            *self.pixel.as_bytes()
        );
        assert_eq!(
            art.frames[&[4; 16]].pixels.selected.value.bytes,
            self.pixel_bytes
        );
        assert_eq!(art.fps.as_ref().map_or(12, |r| r.selected.value), fps);
        assert_eq!(
            art.sound.records["h/score"].selected.value,
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([8; 16])))
        );
        assert_eq!(
            art.sound.patches()[&self.recipe.id()].value,
            FlipnoteOp::SetPatch {
                patch: self.recipe.id(),
                descriptor: self.recipe.clone()
            }
        );
        assert_eq!(art.sound.sfx().len(), 1);
        assert_eq!(
            art.sound.sfx()[&[6; 16]].value,
            FlipnoteOp::SetSfx {
                sfx: [6; 16],
                frame: [4; 16],
                patch: self.recipe.id(),
                note: 72,
                duration_ms: 375,
            }
        );
        assert_eq!(art.sound.exports().len(), 1);
        assert_eq!(
            art.sound.exports()[&[7; 16]].value,
            FlipnoteOp::SetExport {
                export: [7; 16],
                cid: *self.package.as_bytes(),
                bytes: self.package_bytes,
                expiry: StudioExpiry::Never,
            }
        );
        assert!(!art.sound.sfx().contains_key(&[3; 16]));
        assert!(!art.sound.exports().contains_key(&[5; 16]));
    }
}

async fn control(
    actor: &crate::ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    action: Action,
) -> Response {
    actor
        .studio_control_begin(StudioControlRequest {
            target: target(),
            action,
        })
        .await
        .unwrap()
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
        .unwrap()
}

async fn idle(actor: &crate::ServerActor, store: &Arc<Mutex<Option<ServerStore>>>) {
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
}

fn own_pools() {
    // Each actor models a separate desktop process. Unrelated tests must not consume its
    // detached reconstruction permits and turn this bounded lifecycle into a timing lottery.
    crate::actor::studio_pools_for_next_spawn(
        Arc::new(tokio::sync::Semaphore::new(4)),
        Arc::new(tokio::sync::Semaphore::new(3)),
    );
}

#[tokio::test]
async fn gate7_create_sound_score_restart_rotate_recover_export_through_actor_custody() {
    let mut p = Pair::new().await;
    let group = p.bob.group_id();
    let logical = target().document(&group).unwrap();
    let pointer = catcoms_replication::registry::PointerKey::new(
        logical.doc_type,
        logical.logical_key.clone(),
    )
    .unwrap();
    let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    let pixa = hex::decode(vectors["pixaHex"].as_str().unwrap()).unwrap();
    let pix = crate::creative::validate_pixa(&pixa).unwrap().frames()[0].to_vec();
    let descriptor = StudioPatch::new(&vectors["patch"]).unwrap();
    let patch = descriptor.id();
    let score = StudioTarget::Score {
        channel: channel(),
        object: [8; 16],
    };
    p.alice.set_blob_store(
        p.a_store
            .blob_store(&hex::encode(p.alice.group_id()))
            .unwrap(),
    );
    own_pools();
    let (a, mut events, task) = crate::spawn(p.alice);
    let drain = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, crate::AppEvent::StudioReceivePaused));
        }
    });
    let store = Arc::new(Mutex::new(Some(p.a_store)));
    let created = actor_save::save(
        &a,
        &store,
        StudioRequest::Create {
            channel: channel(),
            object: [7; 16],
            nonce: [71; 16],
            title: "Gate 7".into(),
            ts: 1000,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let apply = |nonce, op: FlipnoteOp| StudioRequest::Apply {
        target: target(),
        epoch_id: created.epoch_id,
        nonce: [nonce; 16],
        body: op.encode().unwrap(),
    };
    // A reference is not custody. Both refusals precede durable intent admission.
    let export = FlipnoteOp::SetExport {
        export: [5; 16],
        cid: [99; 32],
        bytes: pixa.len() as u64,
        expiry: StudioExpiry::Never,
    };
    assert!(actor_save::save(&a, &store, apply(72, export))
        .await
        .is_err());
    assert!(actor_save::save(
        &a,
        &store,
        apply(
            73,
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([8; 16])))
        )
    )
    .await
    .is_err());
    let score_created = actor_save::save(
        &a,
        &store,
        StudioRequest::CreateScore {
            channel: channel(),
            object: [8; 16],
            nonce: [74; 16],
            title: "Linked score".into(),
            ts: 1000,
        },
    )
    .await
    .unwrap()
    .unwrap();
    for (nonce, op) in [
        (
            75,
            ScoreOp::SetPatch {
                patch,
                descriptor: descriptor.clone(),
            },
        ),
        (
            76,
            ScoreOp::SetCell {
                step: 0,
                row: 0,
                value: ScoreCell::Note {
                    patch,
                    len: Some(1),
                    volume: None,
                },
            },
        ),
    ] {
        actor_save::save(
            &a,
            &store,
            StudioRequest::Apply {
                target: score,
                epoch_id: score_created.epoch_id,
                nonce: [nonce; 16],
                body: op.encode().unwrap(),
            },
        )
        .await
        .unwrap();
    }
    let pixel_cid =
        crate::Cid::from_hex(&a.publish_pix(pix.clone(), None).await.unwrap().cid).unwrap();
    let package_cid =
        crate::Cid::from_hex(&a.publish_pixa(pixa.clone(), None).await.unwrap().cid).unwrap();
    let expected = Composition {
        pixel: pixel_cid,
        pixel_bytes: pix.len() as u64,
        recipe: descriptor.clone(),
        package: package_cid,
        package_bytes: pixa.len() as u64,
    };
    let sfx = FlipnoteOp::SetSfx {
        sfx: [3; 16],
        frame: [4; 16],
        patch,
        note: 60,
        duration_ms: 120,
    };
    let export = FlipnoteOp::SetExport {
        export: [5; 16],
        cid: *package_cid.as_bytes(),
        bytes: pixa.len() as u64,
        expiry: StudioExpiry::Never,
    };
    for (nonce, op) in [
        (
            77,
            FlipnoteOp::InsertFrame {
                frame: [4; 16],
                after: None,
                cid: *pixel_cid.as_bytes(),
                bytes: pix.len() as u64,
            },
        ),
        (78, FlipnoteOp::SetPatch { patch, descriptor }),
        (79, sfx.clone()),
        (
            80,
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([8; 16]))),
        ),
        (81, export.clone()),
        (
            88,
            FlipnoteOp::SetSfx {
                sfx: [6; 16],
                frame: [4; 16],
                patch,
                note: 72,
                duration_ms: 375,
            },
        ),
        (
            89,
            FlipnoteOp::SetExport {
                export: [7; 16],
                cid: *package_cid.as_bytes(),
                bytes: pixa.len() as u64,
                expiry: StudioExpiry::Never,
            },
        ),
    ] {
        actor_save::save(&a, &store, apply(nonce, op))
            .await
            .unwrap();
    }
    let saved = actor_save::save(&a, &store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    let score_saved = actor_save::save(&a, &store, StudioRequest::Read { target: score })
        .await
        .unwrap()
        .unwrap();
    // Restart both actor and sealed vault. A cold read must reconstruct the same typed family,
    // complete sound records and media references; no live graph or publication task survives.
    a.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    drop(store.lock().await.take());
    let mut reopened = open(p._a_root.path());
    let snapshot = reopened.load_server(SERVER).unwrap();
    let mut node = Node::restore(
        &snapshot,
        p.wire.clone(),
        rng(),
        Box::new(p.clock.clone()),
        "alice",
    )
    .unwrap();
    node.set_blob_store(reopened.blob_store(&hex::encode(node.group_id())).unwrap());
    for (selected, expected) in [(target(), &saved), (score, &score_saved)] {
        let read = node
            .studio_transaction(
                &mut reopened,
                SERVER,
                StudioRequest::Read { target: selected },
            )
            .unwrap()
            .unwrap();
        assert_eq!(read.projection, expected.projection);
    }
    assert!(
        node.studio_transaction(
            &mut reopened,
            SERVER,
            StudioRequest::Read {
                target: StudioTarget::Flipnote {
                    channel: channel(),
                    object: [8; 16]
                },
            }
        )
        .is_err(),
        "a persisted Score cannot be opened as art"
    );
    // Removed sound/export slots are omitted from the next compact seed. The production
    // rotation must therefore retain their full historical evidence BEFORE replacing the source.
    for (nonce, op) in [
        (82, FlipnoteOp::RemoveSfx { sfx: [3; 16] }),
        (83, FlipnoteOp::RemoveExport { export: [5; 16] }),
    ] {
        node.studio_transaction(&mut reopened, SERVER, apply(nonce, op))
            .unwrap();
    }
    // Only the eligibility workload is prepared. These are valid signed causal title changes;
    // neither a receipt, checkpoint nor recovery snapshot is injected by this test.
    node.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(&mut reopened, SERVER, g, d, target())
    });
    *store.lock().await = Some(reopened);
    own_pools();
    let (a, mut events, task) = crate::spawn(node);
    let drain = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, crate::AppEvent::StudioReceivePaused));
        }
    });
    actor_save::save(&a, &store, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    let mut rotated = None;
    for _ in 0..80 {
        p.clock.advance_ms(1000);
        idle(&a, &store).await;
        let guard = store.lock().await;
        let (current, published) = p.bob.sync.with_registry_context(|g, d, _, _| {
            let held = guard.as_ref().unwrap();
            let current = held
                .load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .unwrap();
            let registry = held
                .load_registry_epoch(SERVER, g, pointer.bucket(), d)
                .unwrap();
            let journal = held.load_epoch_owner_receipts(SERVER, &logical).unwrap();
            let published = registry
                .is_some_and(|r| r.projection().unwrap().pointers.get(&pointer) == Some(&1))
                && journal.pending().is_none()
                && journal.published().is_some();
            (current, published)
        });
        if current.epoch() == 1 && current.phase() == EpochPhase::Open && published {
            rotated = Some(current);
            break;
        }
    }
    let rotated =
        rotated.expect("idle rotation must install AND publish the Registry pointer/owner journal");
    assert_ne!(rotated.doc_id(), created.epoch_id);
    expected.assert_preserved(&rotated.projection().unwrap(), 12);
    // This cold member has no installed Studio source. Learn the ACTUAL runtime-issued seed
    // and its retained tail through both actors; prepared receipts would miss publication gaps.
    p.bob
        .request_channel_index_catchup(p.wire.local_peer())
        .await
        .unwrap();
    p.bob
        .set_blob_store(p.b_store.blob_store(&hex::encode(&group)).unwrap());
    let mut verifier = Node::restore(
        &p.bob.snapshot().unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(99))),
        rng(),
        Box::new(p.clock.clone()),
        "read-only verifier",
    )
    .unwrap();
    let b_store = Arc::new(Mutex::new(Some(p.b_store)));
    own_pools();
    let (b, mut events, b_task) = crate::spawn(p.bob);
    let b_drain = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, crate::AppEvent::StudioReceivePaused));
        }
    });
    for selected in [target(), score] {
        actor_save::save(&b, &b_store, StudioRequest::Read { target: selected })
            .await
            .unwrap();
    }
    let mut caught_up = false;
    for _ in 0..120 {
        p.clock.advance_ms(1000);
        tokio::join!(idle(&a, &store), idle(&b, &b_store));
        let guard = b_store.lock().await;
        caught_up = verifier.sync.with_registry_context(|g, d, _, _| {
            let held = guard.as_ref().unwrap();
            held.load_studio_epoch(SERVER, g, target(), d)
                .unwrap()
                .is_some_and(|s| {
                    s.epoch() == 1
                        && s.phase() == EpochPhase::Open
                        && s.projection().unwrap() == rotated.projection().unwrap()
                })
                && held
                    .load_studio_epoch(SERVER, g, score, d)
                    .unwrap()
                    .is_some_and(|s| s.projection().unwrap() == score_saved.projection)
        });
        if caught_up {
            break;
        }
    }
    assert!(
        caught_up,
        "cold member must learn runtime-issued rotated art and linked Score"
    );
    let bob_view = actor_save::save(&b, &b_store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    expected.assert_preserved(&bob_view.projection, 12);
    assert_eq!(
        b.request_blob_bounded(pixel_cid, pix.len(), None)
            .await
            .unwrap(),
        Some(pix.clone())
    );
    assert_eq!(
        b.request_blob_bounded(package_cid, pixa.len(), None)
            .await
            .unwrap(),
        Some(pixa.clone())
    );
    b.shutdown().await;
    b_task.await.unwrap();
    b_drain.await.unwrap();
    let Response::List(listing) = control(&a, &store, Action::List).await else {
        panic!()
    };
    assert!(
        !listing.versions.is_empty(),
        "rotation must retain omitted sound evidence"
    );
    let historical = listing.versions[0].id;
    let Response::Version(version) = control(
        &a,
        &store,
        Action::Read {
            snapshot: historical,
        },
    )
    .await
    else {
        panic!()
    };
    let StudioProjection::Flipnote(art) = version.projection else {
        panic!()
    };
    let source = |op: &FlipnoteOp| {
        art.sound
            .records
            .values()
            .flat_map(|r| std::iter::once(&r.selected).chain(&r.conflicts))
            .find(|v| &v.value == op)
            .expect("full historical alternative retained")
            .source
            .op_id
    };
    // Retained negative evidence still fences deleted ids, even after compaction omitted the
    // live slot. Explicit Copy is not permission to resurrect them from this recovery version.
    for op in [&sfx, &export] {
        let item = Item::SoundRecord { value: source(op) };
        let Response::Preview(preview) = control(
            &a,
            &store,
            Action::Preview {
                snapshot: historical,
                item,
                mode: Mode::Copy,
            },
        )
        .await
        else {
            panic!()
        };
        assert_eq!(preview.plan.disposition, Disposition::Deleted);
        assert!(preview.plan.body.is_none());
    }
    // Recover an actual historical recipe through the same preview/ordinary-Save path. A
    // recipe can be explicitly re-added; SFX/export ids have stricter deletion semantics above.
    let recipe = art
        .sound
        .records
        .values()
        .map(|r| &r.selected.value)
        .find(|op| matches!(op, FlipnoteOp::SetPatch { .. }))
        .unwrap()
        .clone();
    actor_save::save(
        &a,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: rotated.doc_id(),
            nonce: [84; 16],
            body: FlipnoteOp::RemovePatch { patch }.encode().unwrap(),
        },
    )
    .await
    .unwrap();
    let item = Item::SoundRecord {
        value: source(&recipe),
    };
    let Response::Preview(preview) = control(
        &a,
        &store,
        Action::Preview {
            snapshot: historical,
            item,
            mode: Mode::Copy,
        },
    )
    .await
    else {
        panic!()
    };
    assert_eq!(preview.plan.disposition, Disposition::Ready);
    assert_eq!(
        preview.plan.body.as_ref().unwrap(),
        &recipe.encode().unwrap()
    );
    actor_save::save(
        &a,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: rotated.doc_id(),
            nonce: [86; 16],
            body: FlipnoteOp::SetHeader(FlipnoteHeader::Fps(24))
                .encode()
                .unwrap(),
        },
    )
    .await
    .unwrap();
    let stale = a
        .studio_control_begin(StudioControlRequest {
            target: target(),
            action: Action::Apply(Box::new(StudioRecoveryApply {
                snapshot: historical,
                item,
                mode: Mode::Copy,
                epoch_id: preview.epoch_id,
                expected_projection: preview.fingerprint,
                nonce: [87; 16],
                body: preview.plan.body.clone().unwrap(),
            })),
        })
        .await
        .unwrap()
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await;
    assert!(
        stale.is_err(),
        "a newer header invalidates the whole recovery preview"
    );
    // Re-preview after the known refusal; an ambiguous Apply would retain its exact identity.
    let Response::Preview(preview) = control(
        &a,
        &store,
        Action::Preview {
            snapshot: historical,
            item,
            mode: Mode::Copy,
        },
    )
    .await
    else {
        panic!()
    };
    let request = || {
        Action::Apply(Box::new(StudioRecoveryApply {
            snapshot: historical,
            item,
            mode: Mode::Copy,
            epoch_id: preview.epoch_id,
            expected_projection: preview.fingerprint,
            nonce: [85; 16],
            body: preview.plan.body.clone().unwrap(),
        }))
    };
    for already in [false, true] {
        assert!(matches!(control(&a, &store, request()).await,
                Response::Applied { already_saved, .. } if already_saved == already));
    }
    let Response::Export { bytes, .. } = control(
        &a,
        &store,
        Action::Export {
            snapshot: historical,
        },
    )
    .await
    else {
        panic!()
    };
    assert!(
        !bytes.starts_with(b"PIXA"),
        "historical backup is not playable media"
    );
    let backup = catcoms_replication::RecoverySnapshot::decode(&bytes).unwrap();
    assert_eq!(backup.id().unwrap(), historical);
    let decoded =
        catcoms_replication::studio::StudioRecovery::from_snapshot(&backup, &logical, channel())
            .unwrap();
    assert_eq!(
        decoded.projection(),
        &StudioProjection::Flipnote(art.clone())
    );
    assert_eq!(
        a.request_blob_bounded(pixel_cid, pix.len(), None)
            .await
            .unwrap(),
        Some(pix)
    );
    assert_eq!(
        a.request_blob_bounded(package_cid, pixa.len(), None)
            .await
            .unwrap(),
        Some(pixa)
    );
    let final_view = actor_save::save(&a, &store, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    let StudioProjection::Flipnote(art) = &final_view.projection else {
        panic!()
    };
    expected.assert_preserved(&final_view.projection, 24);
    assert_eq!(art.sound.patches().len(), 1);
    assert!(art.sound_readiness().unwrap().missing_patches.is_empty());
    a.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    // Recoveries and pins must survive a second vault open too, after the exact retries above.
    drop(store.lock().await.take());
    let mut held = open(p._a_root.path());
    let mut restored = Node::restore(
        &held.load_server(SERVER).unwrap(),
        p.wire,
        rng(),
        Box::new(p.clock),
        "alice",
    )
    .unwrap();
    let reopened = restored
        .studio_transaction(&mut held, SERVER, StudioRequest::Read { target: target() })
        .unwrap()
        .unwrap();
    assert_eq!(
        reopened.projection, final_view.projection,
        "the recovered recipe and newer FPS survive a second cold restart"
    );
    expected.assert_preserved(&reopened.projection, 24);
    assert!(held
        .creative_pinned_cids()
        .unwrap()
        .for_group(&group)
        .any(|cid| cid == &package_cid));
}
