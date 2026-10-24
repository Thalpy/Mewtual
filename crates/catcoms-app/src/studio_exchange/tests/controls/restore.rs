use super::*;
use crate::studio::{
    StudioRecoveryApply, StudioRecoveryDisposition as D, StudioRecoveryItem as I,
    StudioRecoveryMode as M, StudioRecoveryPreview,
};

fn apply(preview: &StudioRecoveryPreview, item: I, nonce: u8) -> Action {
    Action::Apply(Box::new(StudioRecoveryApply {
        snapshot: preview.snapshot,
        item,
        mode: M::Copy,
        epoch_id: preview.epoch_id,
        expected_projection: preview.fingerprint,
        nonce: [nonce; 16],
        body: preview.plan.body.clone().unwrap(),
    }))
}

#[tokio::test]
async fn studio_recovery_pointer_retries_directly_after_vault_restart_without_a_prior_read() {
    let mut p = Pair::new().await;
    p.save(&title(1, "saved object"));
    for _ in 0..2 {
        drop(p.a_store);
        p.a_store = open(p._a_root.path());
        let Response::PointerRestored { epoch, .. } = p
            .alice
            .studio_control_transaction(
                &mut p.a_store,
                SERVER,
                StudioControlRequest {
                    target: target(),
                    action: Action::RestorePointer,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(epoch, 0);
    }
}

#[tokio::test]
async fn studio_recovery_actor_restores_real_pixels_as_the_restorer_after_missing_blob_refusal() {
    let mut p = Pair::new().await;
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    p.alice.set_blob_store(
        p.a_store
            .blob_store(&hex::encode(p.alice.group_id()))
            .unwrap(),
    );
    let published = p.alice.publish_pix(&pix).unwrap();
    let cid = *crate::Cid::from_hex(&published.cid).unwrap().as_bytes();
    p.save(&domain(
        target(),
        FlipnoteOp::InsertFrame {
            frame: [4; 16],
            after: None,
            cid,
            bytes: pix.len() as u64,
        }
        .encode()
        .unwrap(),
        1,
    ));
    let source = p
        .alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Read { target: target() },
        )
        .unwrap()
        .unwrap();
    let StudioProjection::Flipnote(art) = &source.projection else {
        panic!()
    };
    let original = art.frames[&[4; 16]].pixels.selected.source.author;
    let item = I::Frame {
        id: [4; 16],
        value: art.frames[&[4; 16]].pixels.selected.source.op_id,
    };
    let logical = target().document(&p.bob.group_id()).unwrap();
    let saved = snapshot(&source.projection, 1);
    let snapshot = saved.id().unwrap();
    p.b_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(saved),
            &p.clock,
            &mut rng(),
        )
        .unwrap();
    p.bob.set_blob_store(
        p.b_store
            .blob_store(&hex::encode(p.bob.group_id()))
            .unwrap(),
    );
    let restorer = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    assert_ne!(original, restorer);
    let store = Arc::new(Mutex::new(Some(p.b_store)));
    let (actor, mut events, task) = crate::spawn(p.bob);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let Response::Preview(preview) = invoke(
        &actor,
        &store,
        Action::Preview {
            snapshot,
            item,
            mode: M::Restore,
        },
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert_eq!(preview.plan.original_author, Some(original));
    let edit = || {
        let Action::Apply(mut edit) = apply(&preview, item, 9) else {
            panic!()
        };
        edit.mode = M::Restore;
        Action::Apply(edit)
    };
    assert!(invoke(&actor, &store, edit())
        .await
        .unwrap_err()
        .contains("publish the frame PIX"));
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        0
    );
    let local = actor.publish_pix(pix, None).await.unwrap();
    assert_eq!(local.cid, published.cid);
    assert!(matches!(
        invoke(&actor, &store, edit()).await.unwrap(),
        Response::Applied {
            already_saved: false,
            ..
        }
    ));
    let view =
        super::super::actor_save::save(&actor, &store, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .unwrap();
    let StudioProjection::Flipnote(art) = view.projection else {
        panic!()
    };
    assert_eq!(art.frames[&[4; 16]].pixels.selected.value.cid, cid);
    assert_eq!(art.frames[&[4; 16]].pixels.selected.source.author, restorer);
    assert!(matches!(
        invoke(&actor, &store, Action::RestorePointer)
            .await
            .unwrap(),
        Response::PointerRestored { epoch: 0, .. }
    ));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_recovery_actor_copy_checks_preview_and_retries_after_new_edit_and_snapshot_eviction(
) {
    let mut p = Pair::new().await;
    p.save(&title(1, "old version"));
    let view = p
        .alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Read { target: target() },
        )
        .unwrap()
        .unwrap();
    let StudioProjection::Flipnote(art) = &view.projection else {
        panic!()
    };
    let item = I::Title {
        value: art.title.as_ref().unwrap().selected.source.op_id,
    };
    let logical = target().document(&p.alice.group_id()).unwrap();
    let saved = snapshot(&view.projection, 1);
    let id = saved.id().unwrap();
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(saved),
            &p.clock,
            &mut rng(),
        )
        .unwrap();
    p.save(&title(2, "current version"));
    let store = Arc::new(Mutex::new(Some(p.a_store)));
    let (actor, mut events, task) = crate::spawn(p.alice);
    let changes = Arc::new(AtomicUsize::new(0));
    let observed = changes.clone();
    let drain = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if matches!(event.event, crate::AppEvent::StudioUpdated { .. }) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let Response::Preview(restore) = invoke(
        &actor,
        &store,
        Action::Preview {
            snapshot: id,
            item,
            mode: M::Restore,
        },
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert_eq!(restore.plan.disposition, D::Conflict);
    let Response::Preview(old) = invoke(
        &actor,
        &store,
        Action::Preview {
            snapshot: id,
            item,
            mode: M::Copy,
        },
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert_eq!(old.plan.disposition, D::Ready);
    super::super::actor_save::save(
        &actor,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: old.epoch_id,
            nonce: [3; 16],
            body: title(3, "newer than preview").body,
        },
    )
    .await
    .unwrap();
    assert!(invoke(&actor, &store, apply(&old, item, 4))
        .await
        .unwrap_err()
        .contains("stale"));
    let Response::Preview(fresh) = invoke(
        &actor,
        &store,
        Action::Preview {
            snapshot: id,
            item,
            mode: M::Copy,
        },
    )
    .await
    .unwrap() else {
        panic!()
    };
    let Action::Apply(mut wrong) = apply(&fresh, item, 4) else {
        panic!()
    };
    wrong.body = title(4, "not the selected historical value").body;
    assert!(invoke(&actor, &store, Action::Apply(wrong)).await.is_err());
    assert!(matches!(
        invoke(&actor, &store, apply(&fresh, item, 4))
            .await
            .unwrap(),
        Response::Applied {
            already_saved: false,
            ..
        }
    ));
    let copied =
        super::super::actor_save::save(&actor, &store, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .unwrap();
    let StudioProjection::Flipnote(copied) = copied.projection else {
        panic!()
    };
    assert_eq!(copied.title.unwrap().selected.value, "old version");
    super::super::actor_save::save(
        &actor,
        &store,
        StudioRequest::Apply {
            target: target(),
            epoch_id: fresh.epoch_id,
            nonce: [5; 16],
            body: title(5, "later edit").body,
        },
    )
    .await
    .unwrap();
    // Fill both slots and explicitly acknowledge the third so the selected snapshot disappears.
    let mut newest = [0; 32];
    {
        let mut guard = store.lock().await;
        let vault = guard.as_mut().unwrap();
        for n in [2, 3] {
            let saved = snapshot(&view.projection, n);
            newest = saved.id().unwrap();
            vault
                .update_epoch_recovery(
                    SERVER,
                    &logical,
                    crate::store::EpochRecoveryAction::Stage(saved),
                    &p.clock,
                    &mut rng(),
                )
                .unwrap();
        }
    }
    invoke(
        &actor,
        &store,
        Action::Acknowledge {
            oldest_snapshot: id,
            staged_snapshot: newest,
        },
    )
    .await
    .unwrap();
    assert!(invoke(&actor, &store, Action::Read { snapshot: id })
        .await
        .is_err());
    assert!(matches!(
        invoke(&actor, &store, apply(&fresh, item, 4))
            .await
            .unwrap(),
        Response::Applied {
            already_saved: true,
            ..
        }
    ));
    let after =
        super::super::actor_save::save(&actor, &store, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .unwrap();
    let StudioProjection::Flipnote(after) = after.projection else {
        panic!()
    };
    assert_eq!(
        after.title.unwrap().selected.value,
        "later edit",
        "exact retry never reapplies over a later write"
    );
    let Action::Apply(mut changed) = apply(&fresh, item, 4) else {
        panic!()
    };
    changed.body = title(4, "same nonce different body").body;
    assert!(invoke(&actor, &store, Action::Apply(changed))
        .await
        .is_err());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    assert!(changes.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn studio_recovery_new_historical_tombstone_invalidates_unchanged_projection_preview() {
    let mut p = Pair::new().await;
    let logical = target().document(&p.alice.group_id()).unwrap();
    let (old, deleted) = p.alice.sync.with_registry_context(|g, d, _, _| {
        let mut state =
            catcoms_replication::studio::StudioEpoch::new(g, target(), d.device_id()).unwrap();
        state
            .edit_or_reseal(
                d,
                g,
                &mut rng(),
                &domain(
                    target(),
                    FlipnoteOp::InsertFrame {
                        frame: [6; 16],
                        after: None,
                        cid: [6; 32],
                        bytes: 10,
                    }
                    .encode()
                    .unwrap(),
                    1,
                ),
                1,
            )
            .unwrap();
        let old = state.projection().unwrap();
        state
            .edit_or_reseal(
                d,
                g,
                &mut rng(),
                &domain(
                    target(),
                    FlipnoteOp::RemoveFrame { frame: [6; 16] }.encode().unwrap(),
                    2,
                ),
                2,
            )
            .unwrap();
        (old, state.projection().unwrap())
    });
    let StudioProjection::Flipnote(art) = &old else {
        panic!()
    };
    let item = I::Frame {
        id: [6; 16],
        value: art.frames[&[6; 16]].pixels.selected.source.op_id,
    };
    let saved = snapshot(&old, 1);
    let id = saved.id().unwrap();
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(saved),
            &p.clock,
            &mut rng(),
        )
        .unwrap();
    let Response::Preview(before) = p
        .alice
        .studio_control_transaction(
            &mut p.a_store,
            SERVER,
            StudioControlRequest {
                target: target(),
                action: Action::Preview {
                    snapshot: id,
                    item,
                    mode: M::Copy,
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(before.plan.disposition, D::Ready);
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(snapshot(&deleted, 2)),
            &p.clock,
            &mut rng(),
        )
        .unwrap();
    let Response::Preview(after) = p
        .alice
        .studio_control_transaction(
            &mut p.a_store,
            SERVER,
            StudioControlRequest {
                target: target(),
                action: Action::Preview {
                    snapshot: id,
                    item,
                    mode: M::Copy,
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(before.fingerprint, after.fingerprint);
    assert_eq!(after.plan.disposition, D::Deleted);
    let Action::Apply(edit) = apply(&before, item, 9) else {
        panic!()
    };
    assert!(p
        .alice
        .prepare_studio_recovery_apply(&mut p.a_store, SERVER, target(), *edit)
        .unwrap_err()
        .to_string()
        .contains("no longer Ready"));
    assert_eq!(
        p.a_store
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        0
    );
}

#[tokio::test]
async fn studio_recovery_pending_intent_alone_cannot_bypass_a_stale_preview() {
    use crate::store::epoch_budget::{EpochStorageBudget, StorageScope};
    use crate::store::EpochIntentBudget;
    let mut p = Pair::new().await;
    p.save(&title(1, "historical"));
    let old = p
        .alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Read { target: target() },
        )
        .unwrap()
        .unwrap();
    let StudioProjection::Flipnote(art) = &old.projection else {
        panic!()
    };
    let item = I::Title {
        value: art.title.as_ref().unwrap().selected.source.op_id,
    };
    let logical = target().document(&p.alice.group_id()).unwrap();
    let saved = snapshot(&old.projection, 1);
    let snapshot = saved.id().unwrap();
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(saved),
            &p.clock,
            &mut rng(),
        )
        .unwrap();
    p.save(&title(2, "current"));
    let Response::Preview(preview) = p
        .alice
        .studio_control_transaction(
            &mut p.a_store,
            SERVER,
            StudioControlRequest {
                target: target(),
                action: Action::Preview {
                    snapshot,
                    item,
                    mode: M::Copy,
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let Action::Apply(edit) = apply(&preview, item, 7) else {
        panic!()
    };
    // Exact failure boundary: intent was sealed but no corresponding operation reached source.
    let mut scan = p.a_store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    let mut bytes = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &p.alice.group_id()).unwrap(),
        inventory
            .records_for_server(SERVER, &p.alice.group_id())
            .unwrap(),
    )
    .unwrap();
    let mut intents = EpochIntentBudget::from_inventory(&inventory).unwrap();
    p.alice
        .sync
        .with_registry_context(|g, d, _, r| {
            p.a_store.prepare_epoch_intent(
                SERVER,
                &logical,
                domain(target(), edit.body.clone(), 7),
                d,
                g,
                r,
                &mut bytes,
                &mut intents,
            )
        })
        .unwrap();
    p.save(&title(3, "newer edit after uncertain Save"));
    assert!(p
        .alice
        .prepare_studio_recovery_apply(&mut p.a_store, SERVER, target(), *edit)
        .unwrap_err()
        .to_string()
        .contains("stale"));
    assert_eq!(
        p.a_store
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        4,
        "held intent survives refusal"
    );
}

#[tokio::test]
async fn studio_recovery_index_object_checks_actual_existence_and_same_channel() {
    for condition in [0, 1, 2] {
        let mut p = Pair::new().await;
        let index = StudioTarget::Index { channel: channel() };
        let historical = p.alice.sync.with_registry_context(|g, d, _, _| {
            let mut state =
                catcoms_replication::studio::StudioEpoch::new(g, index, d.device_id()).unwrap();
            state
                .edit_or_reseal(
                    d,
                    g,
                    &mut rng(),
                    &domain(
                        index,
                        IndexOp::PutObject {
                            object: [7; 16],
                            kind: StudioKind::Flipnote,
                            title: "old indexed object".into(),
                            created_by: d.device_id(),
                            ts: 1,
                            expiry: StudioExpiry::Never,
                        }
                        .encode()
                        .unwrap(),
                        1,
                    ),
                    1,
                )
                .unwrap();
            state.projection().unwrap()
        });
        let logical = index.document(&p.alice.group_id()).unwrap();
        let saved = snapshot(&historical, 1);
        let snapshot = saved.id().unwrap();
        p.a_store
            .update_epoch_recovery(
                SERVER,
                &logical,
                crate::store::EpochRecoveryAction::Stage(saved),
                &p.clock,
                &mut rng(),
            )
            .unwrap();
        if condition == 1 {
            p.save(&title(1, "actual object"));
        }
        if condition == 2 {
            p.alice.open_channel_index().await.unwrap();
            let other = p
                .alice
                .create_channel("other")
                .await
                .unwrap()
                .id
                .to_be_bytes();
            p.alice
                .studio_transaction(
                    &mut p.a_store,
                    SERVER,
                    StudioRequest::Apply {
                        target: StudioTarget::Flipnote {
                            channel: other,
                            object: [7; 16],
                        },
                        epoch_id: catcoms_replication::epoch_zero_id(
                            catcoms_wire::DocType::StudioObject,
                            &[7; 16],
                        ),
                        nonce: [1; 16],
                        body: title(1, "wrong channel").body,
                    },
                )
                .unwrap();
        }
        let result = p.alice.studio_control_transaction(
            &mut p.a_store,
            SERVER,
            StudioControlRequest {
                target: index,
                action: Action::Preview {
                    snapshot,
                    item: I::Object { id: [7; 16] },
                    mode: M::Restore,
                },
            },
        );
        if condition == 2 {
            assert!(result.is_err());
        } else {
            let Response::Preview(preview) = result.unwrap() else {
                panic!()
            };
            assert_eq!(
                preview.plan.disposition,
                if condition == 0 {
                    D::MissingTarget
                } else {
                    D::Ready
                }
            );
        }
    }
}
