use super::super::recovery::{invoke_control, ChoiceInput, RecoveryApplyInput};
use super::*;
use catcoms_app::studio::{
    StudioControlAction as Action, StudioRecoveryItem as Item, StudioRecoveryMode as Mode,
};

#[test]
fn native_recovery_choice_and_apply_envelopes_reject_unknown_keys_and_noncanonical_ids() {
    let good = json!({"kind":"frame","id":"01".repeat(16),"value":"02".repeat(32)});
    assert!(serde_json::from_value::<ChoiceInput>(good.clone())
        .unwrap()
        .checked()
        .is_ok());
    for bad in [
        json!({"kind":"frame","id":"AA".repeat(16),"value":"02".repeat(32)}),
        json!({"kind":"frame","id":"01".repeat(16),"value":"02".repeat(31)}),
    ] {
        assert!(serde_json::from_value::<ChoiceInput>(bad)
            .unwrap()
            .checked()
            .is_err());
    }
    let mut extra = good;
    extra["author"] = "03".repeat(32).into();
    assert!(serde_json::from_value::<ChoiceInput>(extra).is_err());
    let base = json!({"snapshot":"01".repeat(32),"choice":{"kind":"title","value":"02".repeat(32)},
        "mode":"copy","epochId":"03".repeat(16),"expectedProjection":"04".repeat(32),
        "nonce":"05".repeat(16),"body":"{}"});
    assert!(serde_json::from_value::<RecoveryApplyInput>(base.clone())
        .unwrap()
        .checked()
        .is_ok());
    let mut bad = base.clone();
    bad["body"] = "x".repeat(64 * 1024 + 1).into();
    assert!(serde_json::from_value::<RecoveryApplyInput>(bad)
        .unwrap()
        .checked()
        .is_err());
    let mut bad = base.clone();
    bad["mode"] = "overwriteEverything".into();
    assert!(serde_json::from_value::<RecoveryApplyInput>(bad)
        .unwrap()
        .checked()
        .is_err());
    let mut bad = base;
    bad["confirmedAuthor"] = "06".repeat(32).into();
    assert!(serde_json::from_value::<RecoveryApplyInput>(bad).is_err());
}

#[tokio::test]
async fn native_recovery_preview_and_copy_use_real_actor_save_and_return_provisional_truth() {
    let (_root, state, actor, task, drain) = fixture().await;
    invoke(&state, 7, create()).await.unwrap();
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    let view = actor
        .studio_begin(StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .execute(authorize(&state, 7, 1, generation).unwrap())
        .await
        .unwrap()
        .unwrap();
    let StudioProjection::Flipnote(art) = &view.projection else {
        panic!()
    };
    let original = art.title.as_ref().unwrap().selected.value.clone();
    let item = Item::Title {
        value: art.title.as_ref().unwrap().selected.source.op_id,
    };
    let group = state.servers.lock().await[&7].group_id.clone();
    let logical = target().document(&group).unwrap();
    let saved = StudioRecovery::snapshot(
        &view.projection,
        None,
        catcoms_app::studio::RecoveryReason::Excluded,
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
    invoke(
        &state,
        7,
        StudioRequest::Apply {
            target: target(),
            epoch_id: view.epoch_id,
            nonce: [11; 16],
            body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("newer".into()))
                .encode()
                .unwrap(),
        },
    )
    .await
    .unwrap();
    let conflict = invoke_control(
        &state,
        7,
        target(),
        Action::Preview {
            snapshot,
            item,
            mode: Mode::Restore,
        },
    )
    .await
    .unwrap();
    assert_eq!(conflict["disposition"], "conflict");
    assert!(conflict["body"].is_null());
    let preview = invoke_control(
        &state,
        7,
        target(),
        Action::Preview {
            snapshot,
            item,
            mode: Mode::Copy,
        },
    )
    .await
    .unwrap();
    assert_eq!(preview["disposition"], "ready");
    let payload = json!({"snapshot":preview["snapshot"],"choice":{"kind":"title","value":hex::encode(art.title.as_ref().unwrap().selected.source.op_id)},
        "mode":"copy","epochId":preview["epochId"],"expectedProjection":preview["expectedProjection"],"nonce":"0a".repeat(16),"body":preview["body"]});
    for retry in [false, true] {
        let parsed = serde_json::from_value::<RecoveryApplyInput>(payload.clone())
            .unwrap()
            .checked()
            .unwrap();
        let answer = invoke_control(&state, 7, target(), Action::Apply(Box::new(parsed)))
            .await
            .unwrap();
        assert_eq!(answer["kind"], "recoveryApplied");
        assert_eq!(answer["alreadySaved"], retry);
        assert_eq!(answer["provisional"], true);
        assert_eq!(
            answer["pointerRestored"], false,
            "content Save does not promise pointer restoration"
        );
    }
    let final_view = invoke(&state, 7, StudioRequest::Read { target: target() })
        .await
        .unwrap();
    assert_eq!(
        final_view.unwrap()["content"]["title"]["selected"]["value"],
        original
    );
    let pointer = invoke_control(&state, 7, target(), Action::RestorePointer)
        .await
        .unwrap();
    assert_eq!(pointer["kind"], "recoveryPointerRestored");
    assert_eq!(pointer["checkpointEpoch"], "0");
    assert_eq!(pointer["provisional"], true);
    // Ordinary Index reads intentionally preserve the warm art graph. Explicit pointer
    // restoration must work without replacing that invariant with a second source cache.
    let index = invoke_control(
        &state,
        7,
        StudioTarget::Index {
            channel: target().channel(),
        },
        Action::RestorePointer,
    )
    .await
    .unwrap();
    assert_eq!(index["kind"], "recoveryPointerRestored");
    assert!(index["object"].is_null());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}
