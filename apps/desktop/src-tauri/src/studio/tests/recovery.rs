use super::*;
use catcoms_app::studio::{
    RecoveryReason, RecoveryTransition, StudioControlAction as Action,
    StudioControlResponse as Response, StudioRecoveryListing, StudioRecoverySummary,
    StudioSettlementSource,
};

#[tokio::test]
async fn native_recovery_export_conversion_holds_fences_and_suppresses_a_late_lock() {
    for generation_change in [false, true] {
        let (_root, state, actor, task, drain) = fixture().await;
        invoke(&state, 7, create()).await.unwrap();
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let ready = actor
            .studio_begin(StudioRequest::Read { target: target() })
            .await
            .unwrap();
        let view = ready
            .execute(authorize(&state, 7, 1, generation).unwrap())
            .await
            .unwrap()
            .unwrap();
        let group = state.servers.lock().await[&7].group_id.clone();
        let logical = target().document(&group).unwrap();
        let saved = StudioRecovery::snapshot(
            &view.projection,
            None,
            RecoveryReason::Excluded,
            [1; 32],
            &std::collections::BTreeMap::new(),
        )
        .unwrap();
        let snapshot = saved.id().unwrap();
        state
            .store
            .lock()
            .await
            .as_mut()
            .unwrap()
            .update_epoch_recovery(
                7,
                &logical,
                catcoms_app::store::EpochRecoveryAction::Stage(saved),
                &ManualClock::new(100),
                &mut rng(),
            )
            .unwrap();
        let converted = std::cell::Cell::new(false);
        let result = invoke_custody(
            &state,
            7,
            InvokeRequest::Control(catcoms_app::studio::StudioControlRequest {
                target: target(),
                action: Action::Export { snapshot },
            }),
            |response| {
                // Deterministic conversion barrier: lock completion and server replacement cannot
                // cross these exact guards while private bytes are being prepared for IPC.
                assert!(state.ui_session_commit.try_lock().is_err());
                assert!(state.servers.try_lock().is_err());
                let InvokeResponse::Control(response) = response else {
                    panic!()
                };
                let value = super::super::recovery::response_value(response)?;
                assert_eq!(value["format"], "p1-recovery-v1");
                converted.set(true);
                if generation_change {
                    state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
                } else {
                    state.session_lock_requested.store(true, Ordering::Release);
                }
                Ok(value)
            },
        )
        .await;
        assert!(converted.get());
        assert!(
            result.is_err(),
            "converted export must not escape a now-stale session"
        );
        state.session_lock_requested.store(false, Ordering::Release);
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    }
}

#[tokio::test]
async fn native_recovery_custody_is_shared_with_save_and_lock_guards() {
    let (_root, state, actor, task, drain) = fixture().await;
    let before = super::super::recovery::invoke_control(&state, 7, target(), Action::List)
        .await
        .unwrap();
    assert!(before["source"].is_null());
    assert_eq!(before["versions"], json!([]));
    invoke(&state, 7, create()).await.unwrap();
    let after = super::super::recovery::invoke_control(&state, 7, target(), Action::List)
        .await
        .unwrap();
    assert_eq!(after["source"]["phase"], "open");
    assert_eq!(after["source"]["provisional"], true);
    assert_eq!(after["pendingIntents"], 1);
    assert!(super::super::recovery::invoke_control(
        &state,
        7,
        target(),
        Action::Export { snapshot: [1; 32] }
    )
    .await
    .is_err());
    // Busy must not deadlock the actor: a queued control holds no vault guard.
    let guard = persist_lock_for(&state, 7).lock_owned().await;
    assert!(
        super::super::recovery::invoke_control(&state, 7, target(), Action::List)
            .await
            .is_err()
    );
    drop(guard);
    state.session_lock_requested.store(true, Ordering::Release);
    assert!(
        super::super::recovery::invoke_control(&state, 7, target(), Action::List)
            .await
            .is_err()
    );
    state.session_lock_requested.store(false, Ordering::Release);
    assert!(
        super::super::recovery::invoke_control(&state, 7, target(), Action::List)
            .await
            .is_ok()
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[test]
fn native_recovery_metadata_is_lossless_and_export_is_not_animation() {
    let v = super::super::recovery::response_value(Response::List(StudioRecoveryListing {
        target: target(),
        source: Some(StudioSettlementSource {
            epoch_id: u128::MAX,
            epoch: u64::MAX,
            phase: EpochPhase::Fault,
        }),
        versions: vec![StudioRecoverySummary {
            id: [8; 32],
            epoch: u64::MAX,
            reason: RecoveryReason::Rewound,
            staged: true,
            encoded_bytes: 10,
        }],
        eviction_pending: Some(RecoveryTransition::EvictionPending {
            oldest_snapshot: [7; 32],
            staged_snapshot: [8; 32],
            deadline_ms: u64::MAX,
        }),
        pending_intents: 2,
    }))
    .unwrap();
    assert_eq!(v["source"]["epochId"], "f".repeat(32));
    assert_eq!(v["source"]["epoch"], u64::MAX.to_string());
    assert_eq!(v["source"]["phase"], "fault");
    assert_eq!(v["evictionPending"]["deadlineMs"], u64::MAX.to_string());
    assert_eq!(v["versions"][0]["reason"], "rewound");
    let exported = super::super::recovery::response_value(Response::Export {
        snapshot: [8; 32],
        bytes: vec![1, 2, 3],
    })
    .unwrap();
    assert_eq!(exported["format"], "p1-recovery-v1");
    assert_eq!(exported["bytesB64"], "AQID");
    assert_eq!(exported["bytes"], 3);
    assert_eq!(
        super::super::recovery::hash(&"ab".repeat(32)).unwrap(),
        [0xab; 32]
    );
    for value in [
        "ab".repeat(31),
        "AB".repeat(32),
        "gg".repeat(32),
        "ab".repeat(33),
    ] {
        assert!(super::super::recovery::hash(&value).is_err());
    }
}
