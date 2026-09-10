use super::*;
use crate::studio::{
    StudioControlAction as Action, StudioControlRequest, StudioControlResponse as Response,
    StudioVaultLease,
};
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::{RecoveryReason, RecoverySnapshot};
use std::collections::BTreeMap;
use tokio::sync::Mutex;
mod restore;
mod settlement;

async fn invoke(
    actor: &crate::ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    action: Action,
) -> Result<Response, String> {
    actor
        .studio_control_begin(StudioControlRequest {
            target: target(),
            action,
        })
        .await?
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
}

fn snapshot(p: &StudioProjection, n: u8) -> RecoverySnapshot {
    StudioRecovery::snapshot(p, None, RecoveryReason::Excluded, [n; 32], &BTreeMap::new()).unwrap()
}

#[tokio::test]
async fn studio_control_actor_lists_reads_exports_and_acknowledges_exact_warning_without_publication(
) {
    let mut p = Pair::new().await;
    let logical = target().document(&p.alice.group_id()).unwrap();
    let mut ids = vec![];
    for n in 1..=3 {
        let view = p
            .alice
            .studio_transaction(
                &mut p.a_store,
                SERVER,
                StudioRequest::Apply {
                    target: target(),
                    epoch_id: catcoms_replication::epoch_zero_id(
                        logical.doc_type,
                        &logical.logical_key,
                    ),
                    nonce: [n; 16],
                    body: title(n, &format!("version {n}")).body,
                },
            )
            .unwrap()
            .unwrap();
        let saved = snapshot(&view.projection, n);
        ids.push(saved.id().unwrap());
        p.a_store
            .update_epoch_recovery(
                SERVER,
                &logical,
                crate::store::EpochRecoveryAction::Stage(saved),
                &ManualClock::new(100),
                &mut rng(),
            )
            .unwrap();
    }
    let store = Arc::new(Mutex::new(Some(p.a_store)));
    let (actor, mut events, task) = crate::spawn(p.alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let Response::List(before) = invoke(&actor, &store, Action::List).await.unwrap() else {
        panic!()
    };
    assert_eq!(before.versions.len(), 3);
    assert!(before.eviction_pending.is_some());
    assert_eq!(before.pending_intents, 3);
    for id in &ids {
        let Response::Version(version) = invoke(&actor, &store, Action::Read { snapshot: *id })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(version.summary.id, *id);
        let Response::Export { snapshot, bytes } =
            invoke(&actor, &store, Action::Export { snapshot: *id })
                .await
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(snapshot, *id);
        let decoded = RecoverySnapshot::decode(&bytes).unwrap();
        assert_eq!(decoded.id().unwrap(), *id);
        assert_eq!(
            StudioRecovery::from_snapshot(&decoded, &logical, channel())
                .unwrap()
                .projection(),
            &version.projection
        );
    }
    assert!(invoke(&actor, &store, Action::Read { snapshot: [99; 32] })
        .await
        .is_err());
    assert!(invoke(
        &actor,
        &store,
        Action::Acknowledge {
            oldest_snapshot: ids[1],
            staged_snapshot: ids[2]
        }
    )
    .await
    .is_err());
    for _ in 0..2 {
        let Response::Acknowledged(after) = invoke(
            &actor,
            &store,
            Action::Acknowledge {
                oldest_snapshot: ids[0],
                staged_snapshot: ids[2],
            },
        )
        .await
        .unwrap() else {
            panic!()
        };
        assert_eq!(after.versions.len(), 2);
        assert!(after.eviction_pending.is_none());
        assert_eq!(
            after.pending_intents, 3,
            "warning ack does not retire intents"
        );
        assert_eq!(
            after.source.unwrap().epoch,
            0,
            "warning ack does not install a checkpoint"
        );
    }
    assert_eq!(p.wire.attempts.load(Ordering::SeqCst), 0);
    // Both reply variants still work in the same actor; no fake historical StudioView.
    assert!(
        super::actor_save::save(&actor, &store, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .is_some()
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    let guard = store.lock().await;
    assert_eq!(
        guard
            .as_ref()
            .unwrap()
            .load_epoch_recovery(SERVER, &logical)
            .unwrap()
            .retained()
            .len(),
        2
    );
}

#[tokio::test]
async fn studio_control_rejects_wrong_channel_and_validates_unselected_staged_content() {
    let mut p = Pair::new().await;
    let logical = target().document(&p.alice.group_id()).unwrap();
    let mut source = p.alice.sync.with_registry_context(|g, d, _, _| {
        catcoms_replication::studio::StudioEpoch::new(g, target(), d.device_id()).unwrap()
    });
    let saved = snapshot(&source.projection().unwrap(), 1);
    let id = saved.id().unwrap();
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(saved),
            &ManualClock::new(10),
            &mut rng(),
        )
        .unwrap();
    // Same object but another actual existing channel cannot read its historical content.
    p.alice.open_channel_index().await.unwrap();
    let channel = p
        .alice
        .create_channel("elsewhere")
        .await
        .unwrap()
        .id
        .to_be_bytes();
    let wrong = StudioTarget::Flipnote {
        channel,
        object: [7; 16],
    };
    assert!(p
        .alice
        .studio_control_transaction(
            &mut p.a_store,
            SERVER,
            StudioControlRequest {
                target: wrong,
                action: Action::Read { snapshot: id }
            }
        )
        .is_err());
    p.alice.sync.with_registry_context(|g, d, _, rng| {
        source
            .edit_or_reseal(d, g, rng, &title(2, "later"), 2)
            .unwrap();
    });
    let second = snapshot(&source.projection().unwrap(), 2);
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(second),
            &ManualClock::new(10),
            &mut rng(),
        )
        .unwrap();
    let mut bad = snapshot(&source.projection().unwrap(), 3);
    // Valid generic envelope, invalid typed payload, specifically in the unselected third slot.
    bad.projection = vec![255];
    let bad_id = bad.id().unwrap();
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(bad),
            &ManualClock::new(10),
            &mut rng(),
        )
        .unwrap();
    for action in [
        Action::List,
        Action::Read { snapshot: id },
        Action::Export { snapshot: id },
        Action::Acknowledge {
            oldest_snapshot: id,
            staged_snapshot: bad_id,
        },
    ] {
        assert!(p
            .alice
            .studio_control_transaction(
                &mut p.a_store,
                SERVER,
                StudioControlRequest {
                    target: target(),
                    action
                }
            )
            .is_err());
    }
}

#[tokio::test]
async fn studio_control_historical_reads_remain_available_in_actual_closing_and_fault() {
    use catcoms_replication::{InheritedCheckpoint, Receipt};
    let mut p = Pair::new().await;
    p.save(&title(1, "recoverable content"));
    let logical = target().document(&p.alice.group_id()).unwrap();
    let view = p
        .alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Read { target: target() },
        )
        .unwrap()
        .unwrap();
    let saved = snapshot(&view.projection, 1);
    let id = saved.id().unwrap();
    p.a_store
        .update_epoch_recovery(
            SERVER,
            &logical,
            crate::store::EpochRecoveryAction::Stage(saved),
            &ManualClock::new(10),
            &mut rng(),
        )
        .unwrap();
    for n in [1, 2] {
        let mut b = budget(&mut p.alice, &mut p.a_store);
        p.alice.sync.with_registry_context(|g, d, _, rng| {
            let receipt = Receipt::sign(
                logical.clone(),
                0,
                [n; 32],
                [n; 32],
                0,
                InheritedCheckpoint::EpochZero,
                d,
            )
            .unwrap();
            p.a_store
                .seal_studio_epoch(SERVER, g, target(), d, receipt, 0, rng, &mut b)
                .unwrap();
        });
        let Response::List(list) = p
            .alice
            .studio_control_transaction(
                &mut p.a_store,
                SERVER,
                StudioControlRequest {
                    target: target(),
                    action: Action::List,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            list.source.unwrap().phase,
            if n == 1 {
                catcoms_replication::EpochPhase::Closing
            } else {
                catcoms_replication::EpochPhase::Fault
            }
        );
        for action in [
            Action::Read { snapshot: id },
            Action::Export { snapshot: id },
        ] {
            p.alice
                .studio_control_transaction(
                    &mut p.a_store,
                    SERVER,
                    StudioControlRequest {
                        target: target(),
                        action,
                    },
                )
                .unwrap();
        }
    }
}
